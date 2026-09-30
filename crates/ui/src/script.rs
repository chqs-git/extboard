use bevy::prelude::*;
use extboard_core::Canvas;
use rhai::{AST, Engine, FnPtr};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use crate::client::{Document, Notice};
use crate::node::{NodeId, NodeRect};
use crate::select::{bounds, cursor_world, pick};

// A handler runs inside a Bevy system on the frame loop, so `while true {}`
// would freeze the window. The budget turns that hang into an error message.
const MAX_OPS: u64 = 200_000;
const MAX_CALLS: usize = 32;
// A press that travelled this far is a drag, and the node went with it.
const SLOP_PX: f32 = 4.0;
pub const DARK: Color = Color::srgb(0.07, 0.07, 0.09);
const LIGHT: Color = Color::srgb(0.91, 0.91, 0.94);

pub struct ScriptPlugin;

// The whole of what a handler can reach. Each one is a write the API would
// have accepted from you anyway, which is the entire security story: there is
// no privileged tier behind this list to hand someone else's script.
enum Effect {
    Theme(String),
    Move(String, i64, i64),
}

// Host functions are closures owned by the engine and the document is a
// resource the running system holds, so the two meet here rather than through
// a borrow that cannot exist.
#[derive(Default)]
struct Pending {
    handlers: HashMap<String, Vec<FnPtr>>,
    effects: Vec<Effect>,
}

#[derive(Resource)]
struct Script {
    engine: Engine,
    ast: AST,
    handlers: HashMap<String, Vec<FnPtr>>,
    pending: Arc<Mutex<Pending>>,
    source: String,
}

impl Plugin for ScriptPlugin {
    fn build(&self, app: &mut App) {
        // Every condition stands on its own: a set's condition does not stop
        // the ones inside it from being evaluated, and a `Res<Document>` in
        // one of those is a panic on the frames before the first load.
        app.insert_resource(Script::new()).add_systems(
            Update,
            (
                (recompile, repaint)
                    .chain()
                    .run_if(resource_exists_and_changed::<Document>),
                // Typing is not clicking, and an edit session owns the mouse.
                click.run_if(resource_exists::<Document>.and_then(not(crate::text::editing))),
            )
                .chain(),
        );
    }
}

impl Script {
    fn new() -> Self {
        let pending = Arc::<Mutex<Pending>>::default();
        Self {
            engine: engine(&pending),
            ast: AST::empty(),
            handlers: HashMap::new(),
            pending,
            source: String::new(),
        }
    }

    // Compile, then run the top level for its `on_click` calls. The registry
    // is swapped only on success, so a half-typed script leaves the handlers
    // that were working live.
    fn load(&mut self, source: &str, canvas: &mut Canvas) -> Result<(), String> {
        self.source = source.to_owned();
        lock(&self.pending).handlers.clear();

        let ast = self.engine.compile(source).map_err(fault)?;
        let ran = self.engine.run_ast(&ast).map_err(fault);
        let applied = self.apply(canvas);
        ran?;

        self.handlers = std::mem::take(&mut lock(&self.pending).handlers);
        self.ast = ast;
        applied
    }

    fn fire(&self, node: &str, canvas: &mut Canvas) -> Result<(), String> {
        let mut failed = None;
        for handler in self.handlers.get(node).into_iter().flatten() {
            if let Err(e) = handler.call::<()>(&self.engine, &self.ast, ()) {
                failed = Some(fault(e));
            }
        }
        // One frame, one document change, so five mutations are one Ctrl-Z.
        let applied = self.apply(canvas);
        failed.map_or(applied, Err)
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
            };
            if let Err(e) = outcome {
                failed = Some(fault(e));
            }
        }
        failed.map_or(Ok(()), Err)
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

    engine
}

// Top-level keys of the `.canvas` survive Obsidian (E0-T2), so the script and
// the theme it sets are both ordinary parts of the document.
fn source(canvas: &Canvas) -> String {
    canvas
        .extra
        .get("extboard")
        .and_then(|extboard| extboard.get("script"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
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

fn recompile(
    mut script: ResMut<Script>,
    mut document: ResMut<Document>,
    mut notice: ResMut<Notice>,
) {
    let source = source(&document.0);
    if source == script.source {
        return;
    }
    if let Err(e) = script.load(&source, &mut document.0) {
        error!("{e}");
        notice.0 = Some(e);
    }
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
    script: Res<Script>,
    mut document: ResMut<Document>,
    mut notice: ResMut<Notice>,
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
    if !script.handlers.contains_key(&node) {
        return;
    }
    if let Err(e) = script.fire(&node, &mut document.0) {
        error!("{e}");
        notice.0 = Some(e);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas() -> Canvas {
        serde_json::from_str(
            r#"{"nodes":[
                 {"id":"n7","type":"text","x":0,"y":0,"width":100,"height":50,"text":"hi"}
               ],"edges":[]}"#,
        )
        .expect("fixture")
    }

    fn scripted(source: &str) -> (Script, Canvas) {
        let mut canvas = canvas();
        let mut script = Script::new();
        script.load(source, &mut canvas).expect("compiles");
        (script, canvas)
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

    // The ticket's done-when: the click's effect is an ordinary document write,
    // which is the only reason every other client sees it.
    #[test]
    fn a_handler_writes_the_document_and_nothing_else_does() {
        let (script, mut canvas) =
            scripted(r#"on_click("n7", || { set_theme("dark"); move_node("n7", 40, 50); });"#);
        assert_eq!(theme_name(&canvas), None, "registration is not a click");

        script.fire("n7", &mut canvas).expect("the handler runs");
        assert_eq!(theme_name(&canvas), Some("dark"));
        assert_eq!((canvas.nodes[0].x, canvas.nodes[0].y), (40, 50));

        // A node nobody registered is not an error, it is nothing.
        script
            .fire("n1", &mut canvas)
            .expect("no handler, no write");
    }

    // The other half of the done-when: an error message, not a frozen window.
    #[test]
    fn a_runaway_handler_runs_out_of_budget() {
        let (script, mut canvas) = scripted(r#"on_click("n7", || { while true { } });"#);
        let e = script
            .fire("n7", &mut canvas)
            .expect_err("the budget bites");
        assert!(e.to_lowercase().contains("operations"), "{e}");
    }

    #[test]
    fn a_broken_script_leaves_the_working_handlers_live() {
        let (mut script, mut canvas) = scripted(r#"on_click("n7", || set_theme("dark"));"#);
        script
            .load("on_click(\"n7\", || {", &mut canvas)
            .expect_err("unclosed block");

        script
            .fire("n7", &mut canvas)
            .expect("the old handler runs");
        assert_eq!(theme_name(&canvas), Some("dark"));
    }

    // Everything stock Rhai has for reaching outside the document, gone.
    #[test]
    fn a_handler_cannot_reach_past_the_document() {
        let mut canvas = canvas();
        let mut script = Script::new();
        for reach in [r#"import "std" as s;"#, r#"eval("1+1")"#] {
            assert!(script.load(reach, &mut canvas).is_err(), "{reach}");
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
    fn the_script_is_a_top_level_key_and_missing_is_empty() {
        let mut canvas = canvas();
        assert_eq!(source(&canvas), "");
        canvas.extra.insert(
            "extboard".to_owned(),
            serde_json::json!({"script": "on_click(\"n7\", || {});"}),
        );
        assert_eq!(source(&canvas), "on_click(\"n7\", || {});");
    }
}
