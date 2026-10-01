use bevy::input_focus::{FocusCause, InputFocus};
use bevy::prelude::*;
use bevy::text::{EditableText, TextCursorStyle, TextEdit};
use bevy::ui::widget::TextScroll;
use extboard_core::Canvas;
use rhai::{AST, Engine, FnPtr};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};

use crate::client::Document;
use crate::edit::command;
use crate::node::{NodeId, NodeRect};
use crate::select::{bounds, cursor_world, pick};
use crate::text::{Editing, Target, wrap};

// A handler runs inside a Bevy system on the frame loop, so `while true {}`
// would freeze the window. The budget turns that hang into an error message.
const MAX_OPS: u64 = 200_000;
const MAX_CALLS: usize = 32;
// A press that travelled this far is a drag, and the node went with it.
const SLOP_PX: f32 = 4.0;
pub const DARK: Color = Color::srgb(0.07, 0.07, 0.09);
const LIGHT: Color = Color::srgb(0.91, 0.91, 0.94);

// The sidebar. Wide enough for a handler without wrapping, and a fixed strip
// down the right edge so the hit test is a single comparison.
pub const WIDTH: f32 = 500.0;
const PAD: f32 = 10.0;
const CODE_SIZE: f32 = 11.0;
const LABEL_SIZE: f32 = 12.0;
const PANEL_BG: Color = Color::srgb(0.08, 0.09, 0.11);
const PANEL_FG: Color = Color::srgb(0.86, 0.88, 0.92);
const LABEL: Color = Color::srgb(0.5, 0.55, 0.62);
const KEYWORD: Color = Color::srgb(0.78, 0.6, 1.0);
const STRING: Color = Color::srgb(0.62, 0.84, 0.6);
const NUMBER: Color = Color::srgb(0.95, 0.72, 0.45);
const COMMENT: Color = Color::srgb(0.45, 0.49, 0.55);
const ERROR: Color = Color::srgb(0.9, 0.35, 0.35);
const WARNING: Color = Color::srgb(0.9, 0.72, 0.4);
const TAB_ON: Color = Color::srgb(0.17, 0.19, 0.23);

// E6-T4's single script, and the name it reads as now.
const LEGACY: &str = "script";
const MAIN: &str = "main";

pub struct ScriptPlugin;

// The whole of what a handler can reach. Each one is a write the API would
// have accepted from you anyway, which is the entire security story: there is
// no privileged tier behind this list to hand someone else's script.
enum Effect {
    Theme(String),
    Move(String, i64, i64),
    Resize(String, i64, i64),
    // `None` clears it, back to the colour the node's kind gets.
    Color(String, Option<String>),
    Text(String, String),
}

// Host functions are closures owned by the engine and the document is a
// resource the running system holds, so the two meet here rather than through
// a borrow that cannot exist.
#[derive(Default)]
struct Pending {
    handlers: HashMap<String, Vec<FnPtr>>,
    effects: Vec<Effect>,
}

// Open or closed. Focus is `Editing`'s, so every system that already stands
// down for a text node stands down for the sidebar too.
// Which panel is up, which script it is showing and which of its three views.
// Focus is `Editing`'s, and it carries the name, so a blur always writes back the
// script it was typed into.
#[derive(Resource, Default)]
pub struct Sidebar {
    pub open: bool,
    pub selected: String,
    pub view: View,
}

#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub enum View {
    #[default]
    Editor,
    // The scripts, vertically, and where a new one is made.
    List,
    // The board API, in the buffer's place.
    Help,
}

impl Sidebar {
    // Whether a press in the panel is going to land a caret. The list and the
    // help view have no buffer, so nothing there wants the keyboard.
    pub fn takes_caret(&self) -> bool {
        self.open && self.view == View::Editor
    }
}

#[derive(Component)]
struct SidebarPanel;

// The editable buffer, and the coloured text under it.
#[derive(Component)]
pub struct ScriptBuffer;

#[derive(Component)]
struct Highlight;

// A row in the script list, and the line under the buffer that reports on the
// script being shown.
#[derive(Component)]
struct ScriptRow(String);

#[derive(Component)]
struct StatusLine;

// One script, as compiled. The `ast` is kept beside the handlers because an
// `FnPtr` is only callable against the AST it was taken from.
struct Compiled {
    ast: AST,
    handlers: HashMap<String, Vec<FnPtr>>,
    // What last compiled, so an unchanged script is not rebuilt every frame.
    source: String,
    // The script is not doing what it says: it would not compile, or it threw.
    error: Option<String>,
    // Ids no node answers to. A warning, not an error: `on_click` on a deleted
    // node compiles fine and silently does nothing, which is worse.
    dangling: Vec<String>,
}

#[derive(Resource)]
struct Script {
    engine: Engine,
    pending: Arc<Mutex<Pending>>,
    // Each script compiles on its own, so one that will not compile disables
    // itself and nothing else. Sorted, so the tab row never reshuffles.
    scripts: BTreeMap<String, Compiled>,
}

impl Plugin for ScriptPlugin {
    fn build(&self, app: &mut App) {
        // Every condition stands on its own: a set's condition does not stop
        // the ones inside it from being evaluated, and a `Res<Document>` in
        // one of those is a panic on the frames before the first load.
        app.insert_resource(Script::new())
            .init_resource::<Sidebar>()
            .add_systems(
                PreUpdate,
                swallow
                    .after(bevy::input::InputSystems)
                    .before(crate::text::ReadPress)
                    .run_if(resource_exists::<Document>),
            )
            .add_systems(
                Update,
                (
                    (recompile, repaint)
                        .chain()
                        .run_if(resource_exists_and_changed::<Document>),
                    (
                        toggle_sidebar,
                        draw_sidebar,
                        hold_focus,
                        reseed,
                        draw_highlight,
                        draw_status,
                    )
                        .chain()
                        // `Editing` is `TextPlugin`'s, and `and_then` is lazy:
                        // none of these may ask for it before it exists.
                        .run_if(resource_exists::<Document>.and_then(resource_exists::<Editing>)),
                    // Typing is not clicking, and an edit session owns the mouse.
                    click.run_if(resource_exists::<Document>.and_then(not(crate::text::editing))),
                )
                    .chain(),
            )
            // Before Layout, which is where a `UiTransform` is read.
            .add_systems(
                PostUpdate,
                track_highlight.before(bevy::ui::UiSystems::Layout),
            );
    }
}

impl Script {
    fn new() -> Self {
        let pending = Arc::<Mutex<Pending>>::default();
        Self {
            engine: engine(&pending),
            pending,
            scripts: BTreeMap::new(),
        }
    }

    // `true` when the document holds a script this has not compiled as it now
    // stands. Checked before the document is borrowed mutably, or a top-level
    // `set_theme` would mark it changed every frame and recompile forever.
    fn stale(&self, wanted: &[(String, String)]) -> bool {
        wanted.len() != self.scripts.len()
            || wanted.iter().any(|(name, source)| {
                self.scripts
                    .get(name)
                    .is_none_or(|compiled| &compiled.source != source)
            })
    }

    fn load_all(&mut self, wanted: &[(String, String)], canvas: &mut Canvas) {
        self.scripts
            .retain(|name, _| wanted.iter().any(|(at, _)| at == name));
        for (name, source) in wanted {
            if self
                .scripts
                .get(name)
                .is_some_and(|compiled| &compiled.source == source)
            {
                continue;
            }
            self.load(name, source, canvas);
        }
    }

    // Compile, then run the top level for its `on_click` calls. The handlers are
    // swapped only on success, so a half-typed script leaves the ones that were
    // working live — and leaves every other script untouched either way.
    fn load(&mut self, name: &str, source: &str, canvas: &mut Canvas) {
        let mut compiled = self.scripts.remove(name).unwrap_or_else(Compiled::empty);
        compiled.source = source.to_owned();
        lock(&self.pending).handlers.clear();

        match self.engine.compile(source) {
            Err(e) => compiled.error = Some(fault(e)),
            Ok(ast) => {
                let ran = self.engine.run_ast(&ast).map_err(|e| thrown(&e));
                let handlers = std::mem::take(&mut lock(&self.pending).handlers);
                let applied = self.apply(canvas);
                match ran.and(applied) {
                    Ok(()) => {
                        compiled.handlers = handlers;
                        compiled.ast = ast;
                        compiled.error = None;
                    }
                    // A top level that threw half way leaves the old handlers:
                    // what it did register is not what it says it registers.
                    Err(e) => compiled.error = Some(e),
                }
            }
        }
        self.scripts.insert(name.to_owned(), compiled);
    }

    // `on_click("n7")` after `n7` is deleted compiles and does nothing, so the
    // sidebar has to say so. Recomputed from the live canvas, never cached.
    fn lint(&mut self, canvas: &Canvas) {
        for compiled in self.scripts.values_mut() {
            compiled.dangling = compiled
                .handlers
                .keys()
                .filter(|id| !canvas.nodes.iter().any(|node| &&node.id == id))
                .cloned()
                .collect();
            compiled.dangling.sort();
        }
    }

    // Script by script, so each one's effects and each one's failure land on the
    // script that caused them.
    fn fire(&mut self, node: &str, canvas: &mut Canvas) {
        for name in self.scripts.keys().cloned().collect::<Vec<_>>() {
            let mut failed = None;
            {
                let Some(compiled) = self.scripts.get(&name) else {
                    continue;
                };
                let Some(handlers) = compiled.handlers.get(node) else {
                    continue;
                };
                for handler in handlers {
                    if let Err(e) = handler.call::<()>(&self.engine, &compiled.ast, ()) {
                        failed = Some(thrown(&e));
                    }
                }
            }
            // One frame, one document change, so five mutations are one Ctrl-Z.
            let applied = self.apply(canvas);
            if let Some(compiled) = self.scripts.get_mut(&name) {
                compiled.error = failed.map_or(applied, Err).err();
            }
        }
    }

    fn handles(&self, node: &str) -> bool {
        self.scripts
            .values()
            .any(|compiled| compiled.handlers.contains_key(node))
    }

    fn apply(&self, canvas: &mut Canvas) -> Result<(), String> {
        let mut failed = None;
        for effect in std::mem::take(&mut lock(&self.pending).effects) {
            let outcome = match effect {
                Effect::Theme(name) => {
                    set_theme(canvas, &name);
                    Ok(())
                }
                Effect::Move(id, x, y) => canvas.move_node(&id, x, y),
                Effect::Resize(id, width, height) => canvas.resize_node(&id, width, height),
                Effect::Color(id, color) => canvas.set_color(&id, color.as_deref()),
                Effect::Text(id, text) => canvas.set_text(&id, text),
            };
            if let Err(e) = outcome {
                failed = Some(fault(e));
            }
        }
        failed.map_or(Ok(()), Err)
    }
}

impl Compiled {
    fn empty() -> Self {
        Self {
            ast: AST::empty(),
            handlers: HashMap::new(),
            source: String::new(),
            error: None,
            dangling: Vec::new(),
        }
    }

    // What the sidebar says about this script, and in what colour.
    fn status(&self) -> Option<(String, Color)> {
        if let Some(error) = &self.error {
            return Some((error.clone(), ERROR));
        }
        if self.dangling.is_empty() {
            return None;
        }
        Some((
            format!("no such node: {}", self.dangling.join(", ")),
            WARNING,
        ))
    }
}

fn engine(pending: &Arc<Mutex<Pending>>) -> Engine {
    let mut engine = Engine::new();
    engine
        .set_max_operations(MAX_OPS)
        .set_max_call_levels(MAX_CALLS)
        // Nothing needs it, and a script that cannot build code at runtime is
        // a script whose reach you can read off the registrations below.
        .disable_symbol("eval");

    let cell = pending.clone();
    engine.register_fn("on_click", move |node: &str, handler: FnPtr| {
        lock(&cell)
            .handlers
            .entry(node.to_owned())
            .or_default()
            .push(handler);
    });

    let cell = pending.clone();
    engine.register_fn("set_theme", move |name: &str| {
        lock(&cell).effects.push(Effect::Theme(name.to_owned()));
    });

    let cell = pending.clone();
    engine.register_fn("move_node", move |node: &str, x: i64, y: i64| {
        lock(&cell)
            .effects
            .push(Effect::Move(node.to_owned(), x, y));
    });

    let cell = pending.clone();
    engine.register_fn("resize_node", move |node: &str, width: i64, height: i64| {
        lock(&cell)
            .effects
            .push(Effect::Resize(node.to_owned(), width, height));
    });

    // `""` clears the colour. Anything that is not a preset or `#rrggbb` is
    // refused by the write rather than drawn as nothing, so a typo says so.
    let cell = pending.clone();
    engine.register_fn("set_color", move |node: &str, color: &str| {
        let color = (!color.is_empty()).then(|| color.to_owned());
        lock(&cell)
            .effects
            .push(Effect::Color(node.to_owned(), color));
    });

    // Only a text node has text to set; core refuses the rest, which is an error
    // on the script rather than a write that went nowhere.
    let cell = pending.clone();
    engine.register_fn("set_text", move |node: &str, text: &str| {
        lock(&cell)
            .effects
            .push(Effect::Text(node.to_owned(), text.to_owned()));
    });

    engine
}

// Top-level keys of the `.canvas` survive Obsidian (E0-T2), so the scripts and
// the theme they set are both ordinary parts of the document.
//
// `serde_json`'s map is sorted, which is what keeps the tab row stable.
pub fn scripts(canvas: &Canvas) -> Vec<(String, String)> {
    let extboard = canvas.extra.get("extboard");
    if let Some(map) = extboard
        .and_then(|extboard| extboard.get("scripts"))
        .and_then(Value::as_object)
    {
        return map
            .iter()
            .filter_map(|(name, source)| Some((name.clone(), source.as_str()?.to_owned())))
            .collect();
    }
    // E6-T4 wrote a single script under `extboard.script`. It reads as `main`,
    // and the first write moves it into `scripts` for good.
    extboard
        .and_then(|extboard| extboard.get(LEGACY))
        .and_then(Value::as_str)
        .map(|source| vec![(MAIN.to_owned(), source.to_owned())])
        .unwrap_or_default()
}

pub fn script(canvas: &Canvas, name: &str) -> String {
    scripts(canvas)
        .into_iter()
        .find_map(|(at, source)| (at == name).then_some(source))
        .unwrap_or_default()
}

// `true` when something differed, so only a real edit wakes the document.
pub fn set_script(canvas: &mut Canvas, name: &str, source: &str) -> bool {
    let same = scripts(canvas)
        .iter()
        .any(|(at, had)| at == name && had == source);
    // A legacy key still in the file is a pending change even when the text
    // matches, because writing is what migrates it.
    let legacy = canvas
        .extra
        .get("extboard")
        .is_some_and(|extboard| extboard.get(LEGACY).is_some());
    if same && !legacy {
        return false;
    }
    scripts_mut(canvas).insert(name.to_owned(), Value::String(source.to_owned()));
    true
}

// The scripts block, made if absent and migrated off `extboard.script` on the
// way. Everything that writes a script goes through here.
fn scripts_mut(canvas: &mut Canvas) -> &mut Map<String, Value> {
    let extboard = canvas
        .extra
        .entry("extboard")
        .or_insert_with(|| Value::Object(Map::new()));
    if !extboard.is_object() {
        *extboard = Value::Object(Map::new());
    }
    let extboard = extboard.as_object_mut().expect("an object either way");
    let legacy = extboard.remove(LEGACY);

    let scripts = extboard
        .entry("scripts")
        .or_insert_with(|| Value::Object(Map::new()));
    if !scripts.is_object() {
        *scripts = Value::Object(Map::new());
    }
    let scripts = scripts.as_object_mut().expect("an object either way");
    if let Some(source @ Value::String(_)) = legacy {
        scripts.entry(MAIN).or_insert(source);
    }
    scripts
}

// A name no script in the document is using, for the tab row's `+`.
pub fn fresh_name(canvas: &Canvas) -> String {
    let taken = scripts(canvas);
    (1..)
        .map(|n| format!("script-{n}"))
        .find(|name| !taken.iter().any(|(at, _)| at == name))
        .expect("the integers run out after the names do")
}

// The document the canvas undo stack sees. The scripts have their own history,
// so a canvas step must neither record them nor carry old ones back (E6-T5).
pub fn without_script(canvas: &Canvas) -> Canvas {
    let mut bare = canvas.clone();
    if let Some(extboard) = bare
        .extra
        .get_mut("extboard")
        .and_then(Value::as_object_mut)
    {
        extboard.remove("scripts");
        extboard.remove(LEGACY);
        // An `extboard` that held only scripts is not an empty `extboard`, or two
        // documents that differ by nothing would hash differently.
        if extboard.is_empty() {
            bare.extra.remove("extboard");
        }
    }
    bare
}

// The scripts lifted off the live document onto a restored one: a step back
// moves the canvas and leaves the sidebar's text where it was.
pub fn carry_scripts(from: &Canvas, into: &mut Canvas) {
    let keep = scripts(from);
    if keep.is_empty() && scripts(into).is_empty() {
        return;
    }
    let scripts = scripts_mut(into);
    scripts.clear();
    for (name, source) in keep {
        scripts.insert(name, Value::String(source));
    }
}

// The panel is a strip down the right edge, which is the whole hit test.
pub fn in_sidebar(window: &Window, at: Option<Vec2>) -> bool {
    at.is_some_and(|at| at.x >= window.width() - WIDTH)
}

fn theme_name(canvas: &Canvas) -> Option<&str> {
    canvas.extra.get("theme")?.get("name")?.as_str()
}

fn set_theme(canvas: &mut Canvas, name: &str) {
    let theme = canvas
        .extra
        .entry("theme")
        .or_insert_with(|| Value::Object(Map::new()));
    if !theme.is_object() {
        *theme = Value::Object(Map::new());
    }
    theme["name"] = Value::String(name.to_owned());
}

fn fault(e: impl std::fmt::Display) -> String {
    format!("script: {e}")
}

// A parse error prints its own position; a runtime one hangs it off the error
// instead, so a throw would otherwise say only what went wrong and not where.
fn thrown(e: &rhai::EvalAltResult) -> String {
    let at = e.position();
    if at.is_none() {
        return fault(e);
    }
    fault(format!("{e} ({at})"))
}

// Script errors stay on the script (the sidebar says which one and why), so
// nothing here reaches for the canvas notice any more.
fn recompile(mut script: ResMut<Script>, mut document: ResMut<Document>) {
    let wanted = scripts(&document.0);
    if script.stale(&wanted) {
        script.load_all(&wanted, &mut document.0);
    }
    // Reading through the `ResMut` does not mark it changed; writing would.
    script.lint(&document.0);
}

fn repaint(document: Res<Document>, mut clear: ResMut<ClearColor>) {
    let want = match theme_name(&document.0) {
        Some("light") => LIGHT,
        _ => DARK,
    };
    if clear.0 != want {
        clear.0 = want;
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "a system's arguments are its query"
)]
fn click(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    nodes: Query<(&NodeId, &Transform, &NodeRect)>,
    mut script: ResMut<Script>,
    mut document: ResMut<Document>,
    mut pressed: Local<Option<(String, Vec2)>>,
) {
    let at = window.cursor_position();
    // Space+left is the camera's pan grab, not a click on anything.
    if buttons.just_pressed(MouseButton::Left) && !keys.pressed(KeyCode::Space) {
        *pressed = under(&window, *camera, &nodes).zip(at);
        return;
    }
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    let Some((node, from)) = pressed.take() else {
        return;
    };
    if at.is_none_or(|at| at.distance(from) > SLOP_PX) {
        return;
    }
    // Checked before the write, so an unscripted click is not a document change.
    if !script.handles(&node) {
        return;
    }
    script.fire(&node, &mut document.0);
}

fn under(
    window: &Window,
    camera: (&Camera, &GlobalTransform),
    nodes: &Query<(&NodeId, &Transform, &NodeRect)>,
) -> Option<String> {
    let world = cursor_world(window, camera)?;
    pick(
        nodes
            .iter()
            .map(|(id, transform, rect)| (&id.0, bounds(transform, rect), transform.translation.z)),
        world,
    )
    .cloned()
}

fn lock(pending: &Arc<Mutex<Pending>>) -> std::sync::MutexGuard<'_, Pending> {
    pending.lock().unwrap_or_else(PoisonError::into_inner)
}

fn toggle_sidebar(
    keys: Res<ButtonInput<KeyCode>>,
    document: Res<Document>,
    mut sidebar: ResMut<Sidebar>,
) {
    // A node's own editor owns the keyboard: ctrl-E there is a keystroke, and in
    // ours escape is what ends the session before the panel can be put away.
    if !command(&keys) || !keys.just_pressed(KeyCode::KeyE) {
        return;
    }
    sidebar.open = !sidebar.open;
    if !sidebar.open {
        return;
    }
    sidebar.view = View::Editor;
    // Whatever it was last showing, if the document still has it.
    let scripts = scripts(&document.0);
    if !scripts.iter().any(|(name, _)| name == &sidebar.selected) {
        sidebar.selected = scripts
            .first()
            .map_or_else(|| MAIN.to_owned(), |(name, _)| name.clone());
    }
}

// A press in the panel belongs to the panel. In the editor view `text::toggle`
// takes the caret, and the canvas stands down because something is being edited;
// the list and the help view have no caret to take, so the press is consumed
// here instead. The rows still fire: picking reads the mouse events, not this.
fn swallow(
    sidebar: Res<Sidebar>,
    window: Single<&Window>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
) {
    if sidebar.open && sidebar.view != View::Editor && in_sidebar(&window, window.cursor_position())
    {
        buttons.clear_just_pressed(MouseButton::Left);
    }
}

// The panel is respawned when it opens, when the shown script changes and when
// the view does, and left alone otherwise: the buffer outlives a blur, so a
// remote write cannot take a paragraph with it.
#[allow(
    clippy::too_many_arguments,
    reason = "a system's arguments are its query"
)]
fn draw_sidebar(
    mut commands: Commands,
    registry: Res<Script>,
    sidebar: Res<Sidebar>,
    buffers: Query<&EditableText, With<ScriptBuffer>>,
    existing: Query<Entity, With<SidebarPanel>>,
    mut document: ResMut<Document>,
    mut editing: ResMut<Editing>,
    mut shown: Local<Option<(String, View)>>,
) {
    let want = sidebar
        .open
        .then(|| (sidebar.selected.clone(), sidebar.view));
    if *shown == want {
        return;
    }
    // Leaving the buffer saves it, whether for another script, another view or
    // no panel at all — the same write a blur does, under the name that was
    // being typed into.
    if let (Some((was, View::Editor)), Ok(buffer)) = (shown.as_ref(), buffers.single())
        && set_script(
            &mut document.bypass_change_detection().0,
            was,
            &buffer.value().to_string(),
        )
    {
        document.set_changed();
    }
    *shown = want.clone();

    // Leaving the editor view ends the session. A node's own editor is not ours
    // to end, so only a script session is cleared here.
    match &want {
        Some((name, View::Editor)) => editing.0 = Some(Target::Script(name.clone())),
        _ if matches!(editing.0, Some(Target::Script(_))) => editing.0 = None,
        _ => {}
    }

    for entity in &existing {
        commands.entity(entity).despawn();
    }
    let Some((selected, view)) = want else {
        return;
    };

    let names: Vec<String> = scripts(&document.0)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    let broken = script_errors(&registry, &names);
    let source = script(&document.0, &selected);

    commands.spawn(panel()).with_children(|parent| {
        topbar(&selected, broken.contains(&selected), parent);
        match view {
            View::Editor => editor_body(&source, parent),
            View::List => list_body(&names, &selected, &broken, parent),
            View::Help => help_body(parent),
        }
    });
}

fn script_errors(script: &Script, names: &[String]) -> Vec<String> {
    names
        .iter()
        .filter(|name| {
            script
                .scripts
                .get(*name)
                .is_some_and(|compiled| compiled.status().is_some())
        })
        .cloned()
        .collect()
}

// The shown script's own verdict, under its own buffer. Text rather than a
// respawn, so a compile on every blur does not rebuild the panel.
fn draw_status(
    registry: Res<Script>,
    sidebar: Res<Sidebar>,
    mut lines: Query<(&mut Text, &mut TextColor), With<StatusLine>>,
) {
    let Ok((mut text, mut color)) = lines.single_mut() else {
        return;
    };
    let (message, want) = registry
        .scripts
        .get(&sidebar.selected)
        .and_then(Compiled::status)
        .unwrap_or_else(|| (String::new(), LABEL));
    if text.0 != message {
        text.0 = message;
    }
    if color.0 != want {
        color.0 = want;
    }
}

// Keyboard focus has to be taken off the buffer by hand: escape collapses the
// selection and nothing else, so a blurred panel would otherwise go on
// swallowing the keys the canvas has just started acting on again.
fn hold_focus(
    editing: Res<Editing>,
    buffers: Query<Entity, With<ScriptBuffer>>,
    mut focus: ResMut<InputFocus>,
) {
    let Ok(entity) = buffers.single() else {
        return;
    };
    match matches!(editing.0, Some(Target::Script(_))) {
        true if focus.get() != Some(entity) => focus.set(entity, FocusCause::Pressed),
        false if focus.get() == Some(entity) => focus.clear(),
        _ => {}
    }
}

// An outside write lands in the buffer, never over a caret that is in it.
fn reseed(
    document: Res<Document>,
    sidebar: Res<Sidebar>,
    editing: Res<Editing>,
    mut buffers: Query<&mut EditableText, With<ScriptBuffer>>,
) {
    if !document.is_changed() || matches!(editing.0, Some(Target::Script(_))) {
        return;
    }
    let source = script(&document.0, &sidebar.selected);
    for mut text in &mut buffers {
        if text.value().to_string() != source {
            text.editor_mut().set_text(&source);
        }
    }
}

// The overlay is the only thing that renders the script, so it is rebuilt
// whenever the buffer differs from what it is showing \u{2014} a keystroke at a time.
// Keyed by entity as well as text, or an empty script would rebuild every frame.
fn draw_highlight(
    mut commands: Commands,
    buffers: Query<&EditableText, With<ScriptBuffer>>,
    overlays: Query<Entity, With<Highlight>>,
    mut drawn: Local<Option<(Entity, String)>>,
) {
    let (Ok(buffer), Ok(entity)) = (buffers.single(), overlays.single()) else {
        // Nothing is open, so the next panel starts from a clean slate.
        *drawn = None;
        return;
    };
    let source = buffer.value().to_string();
    if drawn
        .as_ref()
        .is_some_and(|(at, shown)| *at == entity && *shown == source)
    {
        return;
    }
    *drawn = Some((entity, source.clone()));

    commands.entity(entity).despawn_related::<Children>();
    commands.entity(entity).with_children(|parent| {
        // `EditableText` has no placeholder, so the empty panel says what it is
        // here. It is under the caret and gone on the first keystroke, which is
        // what a placeholder is.
        if source.is_empty() {
            parent.spawn((
                TextSpan::new("on_click(\"id\", || set_theme(\"dark\"));".to_owned()),
                TextFont::from_font_size(CODE_SIZE),
                TextColor(COMMENT),
            ));
            return;
        }
        for (kind, text) in highlight(&source) {
            parent.spawn((
                TextSpan::new(text),
                TextFont::from_font_size(CODE_SIZE),
                TextColor(tint(kind)),
            ));
        }
    });
}

// The buffer scrolls itself to keep the caret in view, and `TextScroll` is the
// offset it was drawn at: the overlay has to take the same one or the colour
// slides off the code. A frame behind, which no eye catches on a keystroke.
fn track_highlight(
    buffers: Query<&TextScroll, With<ScriptBuffer>>,
    mut overlays: Query<&mut UiTransform, With<Highlight>>,
) {
    let Ok(scroll) = buffers.single() else {
        return;
    };
    for mut transform in &mut overlays {
        let want = Val2::px(-scroll.0.x, -scroll.0.y);
        if transform.translation != want {
            transform.translation = want;
        }
    }
}

// `+` adds an empty script and shows it. It reaches the file on the first save,
// the same as any other edit to it.
fn panel() -> impl Bundle {
    (
        SidebarPanel,
        // Node panels are UI too and respawn on every document change.
        GlobalZIndex(2),
        Node {
            position_type: PositionType::Absolute,
            right: px(0.0),
            top: px(0.0),
            width: px(WIDTH),
            height: percent(100.0),
            padding: UiRect::all(px(PAD)),
            row_gap: px(PAD),
            flex_direction: FlexDirection::Column,
            overflow: Overflow::clip(),
            ..default()
        },
        BackgroundColor(PANEL_BG),
    )
}

// `\u{2190}` to the list, the name in the middle, `?` for the API and `\u{d7}` to put the panel
// away. The name is a button too: it is the way back to the script itself.
fn topbar(selected: &str, broken: bool, parent: &mut ChildSpawnerCommands) {
    parent
        .spawn(Node {
            width: percent(100.0),
            align_items: AlignItems::Center,
            column_gap: px(4.0),
            ..default()
        })
        .with_children(|parent| {
            parent.spawn(button("\u{2190}", LABEL)).observe(go_list);
            // Grows to take the slack, so the name sits in the middle of it.
            parent
                .spawn(Node {
                    flex_grow: 1.0,
                    justify_content: JustifyContent::Center,
                    overflow: Overflow::clip(),
                    ..default()
                })
                .with_children(|parent| {
                    let label = if broken {
                        format!("{selected} \u{26a0}")
                    } else {
                        selected.to_owned()
                    };
                    parent
                        .spawn(button(&label, if broken { ERROR } else { PANEL_FG }))
                        .observe(go_editor);
                });
            parent.spawn(button("?", LABEL)).observe(go_help);
            parent.spawn(button("\u{d7}", LABEL)).observe(close_panel);
        });
}

// Both halves of the body fill the same box, so they lay out as one and the caret
// lands where the colour is. The overlay goes first, under the caret and the
// selection the buffer draws over it.
fn editor_body(source: &str, parent: &mut ChildSpawnerCommands) {
    parent
        .spawn(Node {
            width: percent(100.0),
            flex_grow: 1.0,
            overflow: Overflow::clip(),
            ..default()
        })
        .with_children(|parent| {
            parent.spawn(overlay());
            parent.spawn(buffer(source));
        });
    parent.spawn(status_line());
}

// Every script in the document, and the one row that makes another.
fn list_body(
    names: &[String],
    selected: &str,
    broken: &[String],
    parent: &mut ChildSpawnerCommands,
) {
    parent
        .spawn(Node {
            width: percent(100.0),
            flex_grow: 1.0,
            flex_direction: FlexDirection::Column,
            row_gap: px(2.0),
            overflow: Overflow::clip(),
            ..default()
        })
        .with_children(|parent| {
            for name in names {
                let label = if broken.contains(name) {
                    format!("{name} \u{26a0}")
                } else {
                    name.clone()
                };
                let fg = match () {
                    _ if broken.contains(name) => ERROR,
                    _ if name == selected => PANEL_FG,
                    _ => LABEL,
                };
                parent
                    .spawn((
                        ScriptRow(name.clone()),
                        row(),
                        BackgroundColor(if name == selected {
                            TAB_ON
                        } else {
                            Color::NONE
                        }),
                        children![(
                            Text::new(label),
                            TextFont::from_font_size(CODE_SIZE),
                            TextColor(fg),
                        )],
                    ))
                    .observe(pick_script);
            }
            parent
                .spawn((
                    row(),
                    children![(
                        Text::new("+ new script"),
                        TextFont::from_font_size(CODE_SIZE),
                        TextColor(STRING),
                    )],
                ))
                .observe(new_script);
        });
}

// Everything a handler can reach, which is the whole of it: there is no tier
// behind this list to hand someone else's script.
const API: [(&str, &str); 6] = [
    (
        "on_click(id, || ...)",
        "run the body when that node is clicked",
    ),
    ("set_theme(name)", "\"dark\" or \"light\""),
    ("move_node(id, x, y)", "top-left, in canvas coordinates"),
    ("resize_node(id, w, h)", "both have to be above zero"),
    (
        "set_color(id, color)",
        "\"1\"-\"6\", \"#rrggbb\", \"\" clears it",
    ),
    (
        "set_text(id, markdown)",
        "the node's own text, if it is a text node",
    ),
];

const API_NOTE: &str = "Right-click a node for its id. A handler reaches the \
                        document and nothing else: no network, no files. It runs \
                        on the frame loop under an operation budget, so a runaway \
                        loop is an error here rather than a frozen window.";

fn help_body(parent: &mut ChildSpawnerCommands) {
    parent
        .spawn(Node {
            width: percent(100.0),
            flex_grow: 1.0,
            flex_direction: FlexDirection::Column,
            row_gap: px(8.0),
            overflow: Overflow::clip(),
            ..default()
        })
        .with_children(|parent| {
            for (signature, means) in API {
                parent
                    .spawn(Node {
                        flex_direction: FlexDirection::Column,
                        ..default()
                    })
                    .with_children(|parent| {
                        parent.spawn((
                            Text::new(signature),
                            TextFont::from_font_size(CODE_SIZE),
                            TextColor(STRING),
                            wrap(),
                        ));
                        parent.spawn((
                            Text::new(means),
                            TextFont::from_font_size(LABEL_SIZE),
                            TextColor(LABEL),
                            wrap(),
                        ));
                    });
            }
            parent.spawn((
                Text::new(API_NOTE),
                TextFont::from_font_size(LABEL_SIZE),
                TextColor(COMMENT),
                wrap(),
            ));
        });
}

fn button(label: &str, fg: Color) -> impl Bundle {
    (
        row(),
        children![(
            Text::new(label.to_owned()),
            TextFont::from_font_size(LABEL_SIZE),
            TextColor(fg),
        )],
    )
}

// No `BackgroundColor`: a row that wants one adds it, and two in one bundle is a
// panic rather than a warning.
fn row() -> impl Bundle {
    (
        Node {
            padding: UiRect::axes(px(7.0), px(3.0)),
            border_radius: BorderRadius::all(px(4.0)),
            ..default()
        },
        // The panel is a click target, so it must not be transparent to picking.
        Pickable::default(),
    )
}

fn go_list(_press: On<Pointer<Press>>, mut sidebar: ResMut<Sidebar>) {
    sidebar.view = View::List;
}

fn go_editor(_press: On<Pointer<Press>>, mut sidebar: ResMut<Sidebar>) {
    sidebar.view = View::Editor;
}

fn go_help(_press: On<Pointer<Press>>, mut sidebar: ResMut<Sidebar>) {
    sidebar.view = View::Help;
}

fn close_panel(_press: On<Pointer<Press>>, mut sidebar: ResMut<Sidebar>) {
    sidebar.open = false;
}

fn pick_script(press: On<Pointer<Press>>, rows: Query<&ScriptRow>, mut sidebar: ResMut<Sidebar>) {
    if let Ok(row) = rows.get(press.entity) {
        sidebar.selected = row.0.clone();
        sidebar.view = View::Editor;
    }
}

// A new script is made to be typed into, so it opens. It reaches the file on the
// first save, the same as any other edit to it.
fn new_script(
    _press: On<Pointer<Press>>,
    mut document: ResMut<Document>,
    mut sidebar: ResMut<Sidebar>,
) {
    let name = fresh_name(&document.0);
    set_script(&mut document.0, &name, "");
    sidebar.selected = name;
    sidebar.view = View::Editor;
}

// Empty until something is wrong with the shown script; `draw_status` fills it.
fn status_line() -> impl Bundle {
    (
        StatusLine,
        Text::new(String::new()),
        TextFont::from_font_size(LABEL_SIZE),
        TextColor(LABEL),
        wrap(),
    )
}

// The colour. An empty `Text` with the spans as children, because that is the
// one way a bevy text block carries more than one colour. Its font and wrap have
// to match the buffer's exactly, `LineHeight` included \u{2014} both take the same
// default, and the glyphs part company the moment one of them does not.
fn overlay() -> impl Bundle {
    (
        Highlight,
        Node {
            position_type: PositionType::Absolute,
            left: px(0.0),
            top: px(0.0),
            width: percent(100.0),
            height: percent(100.0),
            ..default()
        },
        Text::new(String::new()),
        TextFont::from_font_size(CODE_SIZE),
        TextColor(PANEL_FG),
        wrap(),
    )
}

// The buffer is the caret and the keys; its own glyphs are transparent, because
// `EditableText` is uniform-styled and the colour comes from the overlay under
// it. `TextCursorStyle` is a separate component, so the caret still shows.
fn buffer(script: &str) -> impl Bundle {
    let mut text = EditableText {
        allow_newlines: true,
        // The panel's own height, not a line count.
        visible_lines: None,
        ..EditableText::new(script)
    };
    // `new` leaves the caret at the end and the editor scrolls to keep it in
    // view, so a long script would otherwise open mid-text.
    text.queue_edit(TextEdit::TextStart(false));

    (
        ScriptBuffer,
        Node {
            position_type: PositionType::Absolute,
            left: px(0.0),
            top: px(0.0),
            width: percent(100.0),
            height: percent(100.0),
            ..default()
        },
        text,
        wrap(),
        TextFont::from_font_size(CODE_SIZE),
        TextColor(Color::NONE),
        // The default caret is slate, which on this panel is invisible.
        TextCursorStyle {
            color: PANEL_FG,
            ..default()
        },
    )
}

fn tint(kind: Tok) -> Color {
    match kind {
        Tok::Keyword => KEYWORD,
        Tok::Str => STRING,
        Tok::Num => NUMBER,
        Tok::Comment => COMMENT,
        Tok::Plain => PANEL_FG,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Tok {
    Plain,
    Keyword,
    Str,
    Num,
    Comment,
}

const KEYWORDS: [&str; 19] = [
    "break", "catch", "const", "continue", "do", "else", "false", "fn", "for", "if", "in", "let",
    "loop", "return", "switch", "throw", "true", "try", "while",
];

// Enough of Rhai to read a handler: what is a string, what is a comment, and
// which words are the language's. Runs over `char`s rather than bytes, so a
// comment in Portuguese does not slice a character in half.
fn highlight(script: &str) -> Vec<(Tok, String)> {
    let chars: Vec<char> = script.chars().collect();
    let mut runs: Vec<(Tok, String)> = Vec::new();
    let mut at = 0;

    while at < chars.len() {
        let (kind, end) = match chars[at] {
            '/' if chars.get(at + 1) == Some(&'/') => {
                (Tok::Comment, run(&chars, at, |c| c != '\n'))
            }
            '/' if chars.get(at + 1) == Some(&'*') => (Tok::Comment, block_end(&chars, at)),
            quote @ ('"' | '\'') => (Tok::Str, string_end(&chars, at, quote)),
            c if c.is_ascii_digit() => (
                Tok::Num,
                run(&chars, at, |c| c.is_ascii_alphanumeric() || c == '.'),
            ),
            c if c.is_alphabetic() || c == '_' => {
                let end = run(&chars, at, |c| c.is_alphanumeric() || c == '_');
                let word: String = chars[at..end].iter().collect();
                let kind = if KEYWORDS.contains(&word.as_str()) {
                    Tok::Keyword
                } else {
                    Tok::Plain
                };
                (kind, end)
            }
            _ => (Tok::Plain, at + 1),
        };

        let text: String = chars[at..end].iter().collect();
        // Merged while they match, or the span tree is as long as the script:
        // every identifier and every bracket would be an entity of its own.
        match runs.last_mut() {
            Some((last, buffer)) if *last == kind && kind == Tok::Plain => buffer.push_str(&text),
            _ => runs.push((kind, text)),
        }
        at = end;
    }
    runs
}

// From one past `at`, so the opening character is always part of the run.
fn run(chars: &[char], at: usize, mut keep: impl FnMut(char) -> bool) -> usize {
    let mut end = at + 1;
    while end < chars.len() && keep(chars[end]) {
        end += 1;
    }
    end
}

// Past the closing quote, or to the end: a half-typed string colours to the end
// of the panel rather than not at all.
fn string_end(chars: &[char], at: usize, quote: char) -> usize {
    let mut end = at + 1;
    while end < chars.len() {
        match chars[end] {
            '\\' => end = (end + 2).min(chars.len()),
            c if c == quote => return end + 1,
            _ => end += 1,
        }
    }
    chars.len()
}

fn block_end(chars: &[char], at: usize) -> usize {
    let mut end = at + 2;
    while end + 1 < chars.len() {
        if chars[end] == '*' && chars[end + 1] == '/' {
            return end + 2;
        }
        end += 1;
    }
    chars.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas() -> Canvas {
        serde_json::from_str(
            r#"{"nodes":[
                 {"id":"n7","type":"text","x":0,"y":0,"width":100,"height":50,"text":"hi"},
                 {"id":"g1","type":"group","x":0,"y":0,"width":200,"height":200}
               ],"edges":[]}"#,
        )
        .expect("fixture")
    }

    // One named script, compiled the way `recompile` compiles it.
    fn scripted(source: &str) -> (Script, Canvas) {
        let mut canvas = canvas();
        set_script(&mut canvas, MAIN, source);
        let mut script = Script::new();
        script.load_all(&scripts(&canvas), &mut canvas);
        script.lint(&canvas);
        (script, canvas)
    }

    fn error(script: &Script, name: &str) -> Option<String> {
        script.scripts.get(name).and_then(|c| c.error.clone())
    }

    // A set's condition does not stop the conditions inside it from running,
    // so a `Res<Document>` in one of them panicked on every frame before the
    // first load. Nothing here may run, and nothing may ask for what is absent.
    #[test]
    fn the_frames_before_the_first_load_are_quiet() {
        let mut app = App::new();
        app.add_plugins(ScriptPlugin);
        app.world_mut().run_schedule(Update);
    }

    // Closing the panel, or leaving the editor view, has to end the session: a
    // `Target::Script` nobody can reach gates the canvas keyboard forever.
    #[test]
    fn leaving_the_editor_ends_the_script_session() {
        let mut app = App::new();
        app.add_plugins(ScriptPlugin)
            .init_resource::<Editing>()
            .init_resource::<InputFocus>()
            .init_resource::<ClearColor>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .insert_resource(Document(canvas()));

        let mut open = |view, open| {
            let mut sidebar = app.world_mut().resource_mut::<Sidebar>();
            (sidebar.open, sidebar.view) = (open, view);
            app.world_mut().run_schedule(Update);
            app.world().resource::<Editing>().0.clone()
        };

        assert!(matches!(open(View::Editor, true), Some(Target::Script(_)),));
        // The back button and the help button both leave the buffer behind.
        assert_eq!(open(View::List, true), None);
        assert!(matches!(open(View::Editor, true), Some(Target::Script(_))));
        assert_eq!(open(View::Help, true), None);
        // And the exit button.
        assert!(matches!(open(View::Editor, true), Some(Target::Script(_))));
        assert_eq!(open(View::Editor, false), None);
    }

    // Only the editor view wants the keyboard, which is what lets `swallow` eat a
    // press in the other two without stealing one the caret needed.
    #[test]
    fn only_the_editor_view_takes_the_caret() {
        let mut sidebar = Sidebar::default();
        assert!(!sidebar.takes_caret(), "a closed panel takes nothing");
        sidebar.open = true;
        assert!(sidebar.takes_caret());
        for view in [View::List, View::Help] {
            sidebar.view = view;
            assert!(!sidebar.takes_caret(), "{view:?}");
        }
    }

    // Every host function a handler has, and each one an ordinary document write.
    #[test]
    fn a_handler_moves_resizes_and_colours_a_node() {
        let (mut script, mut canvas) = scripted(
            // `r##` because the markdown heading below contains `"#`, which would
            // otherwise close an `r#` string.
            r##"on_click("n7", || {
                 move_node("n7", 40, 50);
                 resize_node("n7", 300, 120);
                 set_color("n7", "4");
                 set_text("n7", "# written by a handler");
               });"##,
        );
        script.fire("n7", &mut canvas);
        assert_eq!(error(&script, MAIN), None);

        let node = &canvas.nodes[0];
        assert_eq!((node.x, node.y), (40, 50));
        assert_eq!((node.width, node.height), (300, 120));
        assert_eq!(node.color.as_deref(), Some("4"));
        assert_eq!(crate::text::markdown(node), Some("# written by a handler"));

        // `""` is how a script puts the colour back to the node's own.
        script.load(
            MAIN,
            r#"on_click("n7", || set_color("n7", ""));"#,
            &mut canvas,
        );
        script.fire("n7", &mut canvas);
        assert_eq!(canvas.nodes[0].color, None);
    }

    // A write core refuses is an error on the script, not a silent no-op: the
    // whole reason `set_color` validates rather than storing what it is given.
    #[test]
    fn a_refused_write_lands_on_the_script_that_asked_for_it() {
        for (source, expected) in [
            (r#"on_click("n7", || set_color("n7", "banana"));"#, "colour"),
            (
                r#"on_click("n7", || resize_node("n7", 0, 10));"#,
                "usable size",
            ),
            (r#"on_click("n7", || move_node("gone", 1, 1));"#, "no node"),
            // The one refusal that is about the node's kind, not its values.
            (
                r#"on_click("n7", || set_text("g1", "x"));"#,
                "not a text node",
            ),
        ] {
            let (mut script, mut canvas) = scripted(source);
            script.fire("n7", &mut canvas);
            let e = error(&script, MAIN).unwrap_or_default();
            assert!(e.contains(expected), "{source} gave {e:?}");
        }
    }

    // The ticket's done-when: the click's effect is an ordinary document write,
    // which is the only reason every other client sees it.
    #[test]
    fn a_handler_writes_the_document_and_nothing_else_does() {
        let (mut script, mut canvas) =
            scripted(r#"on_click("n7", || { set_theme("dark"); move_node("n7", 40, 50); });"#);
        assert_eq!(theme_name(&canvas), None, "registration is not a click");

        script.fire("n7", &mut canvas);
        assert_eq!(error(&script, MAIN), None);
        assert_eq!(theme_name(&canvas), Some("dark"));
        assert_eq!((canvas.nodes[0].x, canvas.nodes[0].y), (40, 50));

        // A node nobody registered is not an error, it is nothing.
        script.fire("n1", &mut canvas);
        assert_eq!(error(&script, MAIN), None);
    }

    // The other half of the done-when: an error message, not a frozen window.
    #[test]
    fn a_runaway_handler_runs_out_of_budget() {
        let (mut script, mut canvas) = scripted(r#"on_click("n7", || { while true { } });"#);
        script.fire("n7", &mut canvas);
        let e = error(&script, MAIN).expect("the budget bites");
        assert!(e.to_lowercase().contains("operations"), "{e}");
    }

    #[test]
    fn a_broken_script_leaves_the_working_handlers_live() {
        let (mut script, mut canvas) = scripted(r#"on_click("n7", || set_theme("dark"));"#);
        script.load(MAIN, "on_click(\"n7\", || {", &mut canvas);
        assert!(error(&script, MAIN).is_some(), "unclosed block");

        script.fire("n7", &mut canvas);
        assert_eq!(theme_name(&canvas), Some("dark"), "the old handler runs");
    }

    // The ticket's done-when: an invalid script says which line, and a handler
    // that throws says so rather than failing silently on the click.
    #[test]
    fn an_error_says_which_line_it_happened_on() {
        let (script, _) = scripted("on_click(\"n7\", || {\n  set_theme(\"dark\");\n");
        let e = error(&script, MAIN).expect("unclosed block");
        assert!(e.contains("line"), "{e}");

        let (mut script, mut canvas) = scripted("on_click(\"n7\", || {\n  throw \"boom\";\n});");
        script.fire("n7", &mut canvas);
        let e = error(&script, MAIN).expect("the handler threw");
        assert!(e.contains("boom") && e.contains("line 2"), "{e}");
    }

    // The point of naming them: your typo in one script is not everyone else's
    // handlers gone. This is the state the kitchen-sink board was actually in.
    #[test]
    fn a_broken_script_does_not_disable_the_others() {
        let mut canvas = canvas();
        // The real typo: a string that never closes.
        set_script(
            &mut canvas,
            "broken",
            r#"on_click("n7 || set_theme("dark"));"#,
        );
        set_script(
            &mut canvas,
            "fine",
            r#"on_click("n7", || move_node("n7", 9, 9));"#,
        );

        let mut script = Script::new();
        script.load_all(&scripts(&canvas), &mut canvas);
        assert!(error(&script, "broken").is_some());
        assert_eq!(error(&script, "fine"), None);

        script.fire("n7", &mut canvas);
        assert_eq!((canvas.nodes[0].x, canvas.nodes[0].y), (9, 9));
    }

    // `on_click` on a node that is gone compiles and silently does nothing, so
    // it is reported as a warning against the script that asked for it.
    #[test]
    fn a_handler_on_a_missing_node_is_a_warning_not_an_error() {
        let (script, _) = scripted(r#"on_click("gone", || set_theme("dark"));"#);
        assert_eq!(error(&script, MAIN), None, "it compiles");
        let (message, color) = script.scripts[MAIN].status().expect("a warning");
        assert!(message.contains("gone"), "{message}");
        assert_eq!(color, WARNING);

        // And a live id says nothing at all.
        let (script, _) = scripted(r#"on_click("n7", || set_theme("dark"));"#);
        assert!(script.scripts[MAIN].status().is_none());
    }

    // A script that stands as it was compiled is not recompiled: `recompile`
    // borrows the document mutably only when this says to, and a top-level
    // `set_theme` would otherwise mark it changed forever.
    #[test]
    fn an_unchanged_script_is_not_recompiled() {
        let (script, canvas) = scripted(r#"set_theme("dark");"#);
        assert!(!script.stale(&scripts(&canvas)));

        let mut edited = canvas.clone();
        set_script(&mut edited, MAIN, r#"set_theme("light");"#);
        assert!(script.stale(&scripts(&edited)));
        // And a second script nobody has compiled yet.
        let mut added = canvas.clone();
        set_script(&mut added, "other", "");
        assert!(script.stale(&scripts(&added)));
    }

    // Everything stock Rhai has for reaching outside the document, gone.
    #[test]
    fn a_handler_cannot_reach_past_the_document() {
        let mut canvas = canvas();
        let mut script = Script::new();
        for reach in [r#"import "std" as s;"#, r#"eval("1+1")"#] {
            script.load(MAIN, reach, &mut canvas);
            assert!(error(&script, MAIN).is_some(), "{reach}");
        }
    }

    #[test]
    fn a_theme_write_keeps_the_rest_of_the_theme() {
        let mut canvas = canvas();
        canvas.extra.insert(
            "theme".to_owned(),
            serde_json::json!({"name": "studio", "colors": ["#4b62f0"]}),
        );
        set_theme(&mut canvas, "dark");
        assert_eq!(theme_name(&canvas), Some("dark"));
        assert_eq!(canvas.extra["theme"]["colors"][0], "#4b62f0");

        // A `theme` that is not an object is not something to merge into.
        canvas.extra.insert("theme".to_owned(), Value::Bool(true));
        set_theme(&mut canvas, "dark");
        assert_eq!(theme_name(&canvas), Some("dark"));
    }

    #[test]
    fn scripts_are_named_under_a_top_level_key() {
        let mut canvas = canvas();
        assert!(scripts(&canvas).is_empty());
        assert_eq!(script(&canvas, MAIN), "");

        assert!(set_script(
            &mut canvas,
            "themes",
            "on_click(\"n7\", || {});"
        ));
        assert_eq!(script(&canvas, "themes"), "on_click(\"n7\", || {});");
        assert!(!set_script(
            &mut canvas,
            "themes",
            "on_click(\"n7\", || {});"
        ));
        assert_eq!(
            canvas.extra["extboard"]["scripts"]["themes"],
            "on_click(\"n7\", || {});"
        );

        // Sorted, so the tab row does not reshuffle between frames.
        set_script(&mut canvas, "nav", "");
        let names: Vec<String> = scripts(&canvas).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["nav", "themes"]);

        // A sibling key under `extboard` is not the scripts' to clobber.
        canvas.extra["extboard"]["theme"] = Value::String("studio".to_owned());
        set_script(&mut canvas, "themes", "// gone");
        assert_eq!(canvas.extra["extboard"]["theme"], "studio");
    }

    // E6-T4 wrote one script under `extboard.script`; boards in the wild have
    // one. It reads as `main` and the first write moves it for good.
    #[test]
    fn the_single_script_of_e6_t4_becomes_main() {
        let mut canvas = canvas();
        canvas.extra.insert(
            "extboard".to_owned(),
            serde_json::json!({"script": "on_click(\"n7\", || {});"}),
        );
        assert_eq!(
            scripts(&canvas),
            [(MAIN.to_owned(), "on_click(\"n7\", || {});".to_owned())]
        );

        // Nothing typed, but the legacy key is still a pending migration.
        assert!(set_script(&mut canvas, MAIN, "on_click(\"n7\", || {});"));
        assert!(canvas.extra["extboard"].get("script").is_none());
        assert_eq!(
            canvas.extra["extboard"]["scripts"][MAIN],
            "on_click(\"n7\", || {});"
        );
        // And now it is migrated, so there is nothing left to write.
        assert!(!set_script(&mut canvas, MAIN, "on_click(\"n7\", || {});"));
    }

    #[test]
    fn a_new_tab_takes_a_name_nobody_is_using() {
        let mut canvas = canvas();
        assert_eq!(fresh_name(&canvas), "script-1");
        set_script(&mut canvas, "script-1", "");
        assert_eq!(fresh_name(&canvas), "script-2");
    }

    // What the canvas undo stack is allowed to see. Two documents that differ
    // only by their scripts have to be the same step, which means the stripped
    // form has to be identical \u{2014} `extboard` and all.
    #[test]
    fn stripping_the_scripts_leaves_no_trace_of_them() {
        let bare = serde_json::to_string(&canvas()).expect("json");
        for mut with in [canvas(), canvas()] {
            set_script(&mut with, "themes", "on_click(\"n7\", || {});");
            assert_eq!(
                serde_json::to_string(&without_script(&with)).expect("json"),
                bare
            );
        }

        // The legacy key counts too, or the migration would read as a step.
        let mut legacy = canvas();
        legacy
            .extra
            .insert("extboard".to_owned(), serde_json::json!({"script": "x"}));
        assert_eq!(
            serde_json::to_string(&without_script(&legacy)).expect("json"),
            bare
        );

        // Not the whole key, though: the rest of `extboard` is the document's.
        let mut shared = canvas();
        set_script(&mut shared, "themes", "x");
        shared.extra["extboard"]["theme"] = Value::String("studio".to_owned());
        let kept = without_script(&shared);
        assert_eq!(kept.extra["extboard"]["theme"], "studio");
        assert!(kept.extra["extboard"].get("scripts").is_none());
    }

    // A canvas step moves the canvas and leaves every script where it was.
    #[test]
    fn carrying_the_scripts_replaces_whatever_the_step_brought_back() {
        let mut live = canvas();
        set_script(&mut live, "themes", "live");
        set_script(&mut live, "nav", "live too");

        let mut restored = canvas();
        set_script(&mut restored, "themes", "stale");
        set_script(&mut restored, "deleted-since", "stale");

        carry_scripts(&live, &mut restored);
        assert_eq!(scripts(&restored), scripts(&live));

        // And no scripts either side leaves no empty block behind.
        let mut plain = canvas();
        carry_scripts(&canvas(), &mut plain);
        assert!(plain.extra.is_empty());
    }

    #[test]
    fn the_sidebar_is_the_right_hand_strip() {
        let mut window = Window::default();
        window.resolution.set(1000.0, 800.0);
        assert!(in_sidebar(&window, Some(Vec2::new(999.0, 400.0))));
        assert!(!in_sidebar(
            &window,
            Some(Vec2::new(1000.0 - WIDTH - 1.0, 400.0))
        ));
        // Off the window entirely is not a click in the panel.
        assert!(!in_sidebar(&window, None));
    }

    #[test]
    fn highlighting_knows_strings_comments_numbers_and_keywords() {
        assert_eq!(
            highlight("let x = 7;"),
            [
                (Tok::Keyword, "let".to_owned()),
                (Tok::Plain, " x = ".to_owned()),
                (Tok::Num, "7".to_owned()),
                (Tok::Plain, ";".to_owned()),
            ]
        );

        // A `//` runs to the newline and no further, and the line after it is code.
        let runs = highlight("// nota\nlet x;");
        assert_eq!(runs[0], (Tok::Comment, "// nota".to_owned()));
        assert_eq!(runs[2], (Tok::Keyword, "let".to_owned()));

        // `on_click` is not a keyword; its argument is a string, brace and all.
        let runs = highlight(r#"on_click("n{7}", || {})"#);
        assert!(
            runs.contains(&(Tok::Str, "\"n{7}\"".to_owned())),
            "{runs:?}"
        );
        assert!(!runs.iter().any(|(kind, _)| *kind == Tok::Keyword));
    }

    // Nothing in the panel may panic on a script that is mid-keystroke, and a
    // half-typed string or comment colours to the end rather than to nothing.
    #[test]
    fn highlighting_survives_a_half_typed_script() {
        for script in [
            "\"unterminated",
            "/* unterminated",
            "'",
            "\"\\",
            "// trailing backslash \\",
            "0x",
            "x\u{e7}\u{e3}o /* acentos */ \"cora\u{e7}\u{e3}o",
            // The board's own typo, which has to colour without panicking.
            r#"on_click("n7 || set_theme("dark"));"#,
        ] {
            let runs = highlight(script);
            let round: String = runs.iter().map(|(_, text)| text.as_str()).collect();
            assert_eq!(round, script, "{script:?}");
        }
    }
}
