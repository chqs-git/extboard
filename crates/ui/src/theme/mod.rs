use bevy::asset::{AssetPath, embedded_asset, embedded_path};
use bevy::color::Hsva;
use bevy::input_focus::{AutoFocus, FocusCause, InputFocus};
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use bevy::text::{EditableText, Font, FontCx, FontSmoothing, TextCursorStyle, TextEdit};
use bevy::ui_widgets::{ControlOrientation, ScrollArea, Scrollbar, ScrollbarThumb};
use extboard_core::{
    DEFAULT_FONT, FONT_ROLES, Fonts, MAX_COLORS, MIN_COLORS, PRESETS, PRIMARY_TEXT, Theme as Block,
    is_font,
};

use crate::client::Document;
use crate::edit::command;

// A board with no theme at all has to draw as something, and this is the first
// preset's background: the one colour that exists before the document loads.
pub const DARK: Color = Color::srgb(0.102, 0.106, 0.149);

// A pixel font is a few hundred glyphs of flat, hinting-free outlines, so the
// file is tiny; a text face with kerning and a full Unicode range never is. The
// weight is the classifier, which is why no space has to declare one.
const PIXEL_BYTES: usize = 75 * 1024;

const LEFT: f32 = 8.0;
const TOP: f32 = 30.0;
const WIDTH: f32 = 236.0;
const PAD: f32 = 8.0;
const ROW: f32 = 18.0;
const SWATCH: f32 = 14.0;
const LABEL_SIZE: f32 = 11.0;
// Seven rows of themes shows every preset; both lists scroll past their cap.
const THEMES_H: f32 = 7.0 * (ROW + 2.0);
const COLORS_H: f32 = 9.0 * (ROW + 2.0);
// As many fonts as have been dropped on the board, so this one scrolls too.
const FONTS_H: f32 = 7.0 * (ROW + 2.0);
const SCROLLBAR: f32 = 6.0;
const THUMB_MIN: f32 = 16.0;
// The naming window, which is the only thing this panel puts over the board.
const PROMPT: Vec2 = Vec2::new(260.0, 62.0);
const PLANE_H: f32 = 108.0;
const STRIP_H: f32 = 12.0;
const KNOB: f32 = 10.0;
const KNOB_EDGE: f32 = 2.0;
const PANEL_BG: Color = Color::srgb(0.08, 0.09, 0.11);
const FIELD_BG: Color = Color::srgb(0.06, 0.07, 0.09);
const ROW_ON: Color = Color::srgb(0.20, 0.24, 0.33);
const FG: Color = Color::srgb(0.86, 0.88, 0.92);
const LABEL: Color = Color::srgb(0.5, 0.55, 0.62);
// An edit the library does not have yet. The same amber the selection ring uses.
const UNSAVED: Color = Color::srgb(0.95, 0.75, 0.30);
const RAIL_BG: Color = Color::srgb(0.12, 0.13, 0.16);
const THUMB: Color = Color::srgb(0.32, 0.35, 0.42);

pub struct ThemePlugin;

// The space's palette as the board draws it, and the themes a save may write
// to. The panel reads both from here rather than reaching for the document.
#[derive(Resource, Default)]
pub struct Theme {
    pub live: Block,
    saved: Vec<Block>,
    pub fonts: Fonts,
    loaded: Vec<Loaded>,
}

// One font of the library, resolved.
struct Loaded {
    // What the canvas calls it: a path in the spaces dir, or a family name.
    name: String,
    // The file's weight, once it has landed. A family name has no file, so it
    // has no weight and is never classified.
    bytes: Option<usize>,
    // Held for as long as the entry is, because dropping the last handle
    // unloads the asset -- and bevy clears the whole font collection when a
    // font asset goes.
    file: Option<Handle<Font>>,
    // The embedded font until the file lands, the family it registers after.
    source: FontSource,
}

// What the name in the box points at, which is what a save is allowed to do.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Named {
    // A shipped palette. It cannot be written to, so an edit to one is a copy.
    Preset,
    Custom,
    New,
}

#[derive(Resource, Default)]
struct Open(bool);

// The naming window, open with the name it is offering. A new theme needs a
// name before it can exist, and this is where one is typed.
#[derive(Resource, Default)]
struct Naming(Option<String>);

#[derive(Component)]
struct NamePrompt;

#[derive(Component)]
struct NewName;

// Which slot has its picker open, and the hue its plane is drawn for. The hue
// is held here rather than read back off the colour because black and grey have
// no hue left to read: without it, dragging into the corner loses the one the
// plane was showing.
#[derive(Resource, Default)]
struct Picking(Option<Picked>);

// Which text has its font list open. The two never open together: the panel
// grows downwards and one tall thing below the sections is enough.
#[derive(Resource, Default)]
struct Targeting(Option<usize>);

#[derive(Clone, Copy)]
struct Picked {
    slot: usize,
    // Turns, as the shader takes it.
    hue: f32,
}

// The panel is respawned when its shape changes -- a slot added, a theme saved,
// a picker opened, another theme selected -- and left alone otherwise, so typing
// a hex code does not pull the caret out from under itself. Which is why neither
// button's state is in here: both are always drawn, and `sync` is what lights them.
#[derive(Component, PartialEq, Eq)]
pub(crate) struct ThemePanel {
    name: String,
    slots: usize,
    themes: usize,
    picking: Option<usize>,
    // Both lists whole: a text row shows the name it points at, so any edit
    // respawns the panel -- and nothing in this section is typed into.
    fonts: Fonts,
    targeting: Option<usize>,
    // The previews are drawn in the fonts themselves, so a file that has just
    // landed and been named is a reason to spawn the panel again.
    named: usize,
}

#[derive(Component)]
struct ThemeRow(String);

// A text and its font list: the row opens the list, a row in the list is the
// font that text takes, and the `-` beside one drops it from the space.
#[derive(Component)]
struct TextRow(usize);

#[derive(Component)]
struct Candidate(String);

#[derive(Component)]
struct DropFont(String);

#[derive(Component)]
struct Swatch(usize);

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Field {
    Color(usize),
    New,
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Dial {
    Plane,
    Hue,
}

#[derive(Component)]
struct Knob(Dial);

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct DialMaterial {
    #[uniform(0)]
    dial: DialPaint,
}

#[derive(ShaderType, Clone, Debug)]
struct DialPaint {
    hue: f32,
    strip: f32,
}

impl UiMaterial for DialMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("picker.wgsl")).with_source("embedded"),
        )
    }
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Button {
    Add,
    Drop,
    Save,
    Clone,
}

#[derive(Component)]
struct ButtonLabel(Button);

impl Plugin for ThemePlugin {
    fn build(&self, app: &mut App) {
        // The dials are a shader, and loading one needs a renderer to load it
        // into. Headless -- a test -- gets the panel's logic and no picture.
        app.init_asset::<DialMaterial>();
        if app.get_sub_app(RenderApp).is_some() {
            embedded_asset!(app, "picker.wgsl");
            app.add_plugins(UiMaterialPlugin::<DialMaterial>::default());
        }
        app.init_resource::<Theme>()
            .init_resource::<Open>()
            .init_resource::<Naming>()
            .init_resource::<Picking>()
            .init_resource::<Targeting>()
            .add_systems(
                Update,
                (
                    (resolve, repaint)
                        .chain()
                        .run_if(resource_exists_and_changed::<Document>),
                    // Not with `resolve`: a file lands frames after the
                    // document that asked for it. `FontCx` is `TextPlugin`'s,
                    // so a headless panel has no fonts to name.
                    name_fonts.run_if(resource_exists::<FontCx>),
                    (toggle, shortcuts, sync, prompt, typed)
                        .chain()
                        .run_if(resource_exists::<Document>),
                )
                    .chain(),
            );
    }
}

impl Theme {
    // Every slot answers a colour, so an unreadable palette still draws.
    pub fn color(&self, index: usize) -> Color {
        self.live.color(index).and_then(hex).unwrap_or(DARK)
    }

    #[cfg(test)]
    pub fn from_colors(colors: &[&str]) -> Self {
        Self {
            live: Block {
                name: "test".to_owned(),
                colors: colors.iter().map(|hex| (*hex).to_owned()).collect(),
            },
            ..default()
        }
    }

    // What a text draws in. The embedded font answers for `DEFAULT_FONT`, for
    // an unset text, and for a name nothing has loaded.
    pub fn text_font(&self, role: usize) -> FontSource {
        self.fonts
            .font(role)
            .map(|name| self.source(name))
            .unwrap_or_default()
    }

    // Every text on the board is rasterized this way, and the atlas is sampled
    // to match: `None` is a nearest sampler as well as a hard edge. A pixel font
    // wants both, and it is classified rather than declared: see `PIXEL_BYTES`.
    pub fn smoothing(&self) -> FontSmoothing {
        let pixel = self
            .fonts
            .font(PRIMARY_TEXT)
            .and_then(|name| self.entry(name))
            .and_then(|loaded| loaded.bytes)
            .is_some_and(|bytes| bytes < PIXEL_BYTES);
        match pixel {
            true => FontSmoothing::None,
            false => FontSmoothing::AntiAliased,
        }
    }

    fn source(&self, name: &str) -> FontSource {
        self.entry(name)
            .map(|loaded| loaded.source.clone())
            .unwrap_or_default()
    }

    fn entry(&self, name: &str) -> Option<&Loaded> {
        self.loaded.iter().find(|loaded| loaded.name == name)
    }

    // A file still on its way, which `name_fonts` is waiting to name.
    fn pending(&self) -> bool {
        self.loaded.iter().any(Loaded::pending)
    }

    // How many of the library's fonts have a font behind them, which is what
    // tells the panel a preview has something new to draw with.
    fn resolved(&self) -> usize {
        self.loaded
            .iter()
            .filter(|loaded| !loaded.pending())
            .count()
    }

    fn rows(&self) -> Vec<Block> {
        PRESETS
            .iter()
            .map(|(name, _)| Block::preset(name))
            .chain(self.saved.iter().cloned())
            .collect()
    }

    fn of(&self, name: &str) -> Option<Block> {
        Block::preset_named(name)
            .or_else(|| self.saved.iter().find(|saved| saved.name == name).cloned())
    }

    fn named(&self) -> Named {
        match () {
            _ if Block::preset_colors(&self.live.name).is_some() => Named::Preset,
            _ if self.saved.iter().any(|saved| saved.name == self.live.name) => Named::Custom,
            _ => Named::New,
        }
    }

    // Against the palettes rather than through `of`: read on every open frame.
    fn dirty(&self) -> bool {
        match Block::preset_colors(&self.live.name) {
            Some(colors) => self.live.colors != *colors,
            None => self
                .saved
                .iter()
                .find(|saved| saved.name == self.live.name)
                .is_none_or(|saved| saved.colors != self.live.colors),
        }
    }
}

// `#rrggbb`, the one form a theme slot is written in.
pub fn hex(color: &str) -> Option<Color> {
    let digits = color.strip_prefix('#').filter(|hex| hex.len() == 6)?;
    let packed = u32::from_str_radix(digits, 16).ok()?;
    Some(Color::srgb_u8(
        (packed >> 16) as u8,
        (packed >> 8) as u8,
        packed as u8,
    ))
}

fn resolve(document: Res<Document>, assets: Res<AssetServer>, mut theme: ResMut<Theme>) {
    let (live, saved) = (document.0.theme(), document.0.themes());
    if (&theme.live, &theme.saved) != (&live, &saved) {
        (theme.live, theme.saved) = (live, saved);
    }
    let fonts = document.0.fonts();
    if theme.fonts != fonts {
        theme.loaded = fonts
            .library
            .iter()
            .map(|name| load(name, &assets))
            .collect();
        theme.fonts = fonts;
    }
}

// A file in the spaces dir is loaded as an asset; anything else is the name of
// a family the system may have.
fn load(name: &str, assets: &AssetServer) -> Loaded {
    match name {
        file if is_font(file) => Loaded {
            name: name.to_owned(),
            bytes: None,
            file: Some(assets.load(file.to_owned())),
            // The embedded font until it lands: a handle for an asset that has
            // not arrived draws nothing at all.
            source: FontSource::default(),
        },
        family => Loaded {
            name: name.to_owned(),
            bytes: None,
            file: None,
            source: match family {
                "" | DEFAULT_FONT => FontSource::default(),
                family => FontSource::from(family),
            },
        },
    }
}

impl Loaded {
    fn pending(&self) -> bool {
        self.file.is_some() && !matches!(self.source, FontSource::Family(_))
    }
}

// The board is given the family a font file registers, never the handle to it.
// A `Handle` resolves through the asset's alias, and a reload of the file --
// which the dev build's watcher fires for the `.ttf` a drop has just written
// into the spaces dir -- replaces the asset with one whose alias is empty.
// Bevy never re-registers an id it has already seen, so that text falls back to
// the default font for good. A family name has nothing to go stale.
fn name_fonts(assets: Res<Assets<Font>>, mut font_cx: ResMut<FontCx>, mut theme: ResMut<Theme>) {
    if !theme.pending() {
        return;
    }
    let named: Vec<(usize, String, usize)> = theme
        .loaded
        .iter()
        .enumerate()
        .filter(|(_, loaded)| loaded.pending())
        .filter_map(|(slot, loaded)| {
            let font = assets.get(loaded.file.as_ref()?.id())?;
            Some((slot, family_of(&mut font_cx, font)?, font.data.len()))
        })
        .collect();
    // Only once something has a name: this runs every frame until it does, and
    // waking the resource respawns every panel on the board.
    for (slot, family, bytes) in named {
        theme.loaded[slot].source = FontSource::from(family.as_str());
        theme.loaded[slot].bytes = Some(bytes);
    }
}

// Registering a blob the collection already holds answers the same family, so
// this both learns the name and puts back a registration a reload lost.
fn family_of(font_cx: &mut FontCx, font: &Font) -> Option<String> {
    let (family, _) = font_cx
        .collection
        .register_fonts(font.data.clone(), None)
        .into_iter()
        .next()?;
    font_cx.collection.family_name(family).map(str::to_owned)
}

fn repaint(theme: Res<Theme>, mut clear: ResMut<ClearColor>) {
    let want = theme.color(extboard_core::BACKGROUND);
    if clear.0 != want {
        clear.0 = want;
    }
}

fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    mut open: ResMut<Open>,
    mut picking: ResMut<Picking>,
    mut targeting: ResMut<Targeting>,
) {
    if command(&keys) && keys.just_pressed(KeyCode::KeyT) {
        open.0 = !open.0;
        picking.0 = None;
        targeting.0 = None;
    }
}

// Ctrl-S is the save button. Undo is not here: a theme edit is an ordinary
// document change, so the canvas stack in undo.rs already steps back through it.
fn shortcuts(
    keys: Res<ButtonInput<KeyCode>>,
    open: Res<Open>,
    theme: Res<Theme>,
    naming: ResMut<Naming>,
    document: ResMut<Document>,
) {
    if open.0 && command(&keys) && keys.just_pressed(KeyCode::KeyS) {
        keep(&theme, naming, document);
    }
}

// An ordinary document change, so the save debounces and undo picks it up.
fn write(document: &mut ResMut<Document>, theme: &Block) {
    if &document.0.stored_theme() != theme {
        document.0.set_theme(theme);
    }
}

pub(crate) fn write_fonts(document: &mut ResMut<Document>, fonts: &Fonts) {
    if &document.0.fonts() != fonts {
        document.0.set_fonts(fonts);
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "a system's arguments are its query"
)]
fn sync(
    mut commands: Commands,
    open: Res<Open>,
    theme: Res<Theme>,
    picking: Res<Picking>,
    targeting: Res<Targeting>,
    focus: Res<InputFocus>,
    panels: Query<(Entity, &ThemePanel)>,
    mut swatches: Query<(&Swatch, &mut BackgroundColor)>,
    mut rows: Query<(&ThemeRow, &mut BackgroundColor), Without<Swatch>>,
    mut fields: Query<(Entity, &Field, &mut EditableText)>,
    mut labels: Query<(&ButtonLabel, &mut Text, &mut TextColor)>,
    mut knobs: Query<(&Knob, &mut Node)>,
    dials: Query<(&Dial, &MaterialNode<DialMaterial>)>,
    mut materials: ResMut<Assets<DialMaterial>>,
) {
    // Keyed on the slot and not on the hue: a hue drag may not respawn the panel
    // it is being dragged in.
    let want = ThemePanel {
        name: theme.live.name.clone(),
        slots: theme.live.colors.len(),
        themes: theme.saved.len(),
        picking: picking.0.map(|open| open.slot),
        fonts: theme.fonts.clone(),
        targeting: targeting.0,
        named: theme.resolved(),
    };
    match (open.0, panels.single().ok()) {
        (false, Some((entity, _))) => commands.entity(entity).despawn(),
        (true, None) => spawn(&mut commands, &theme, want, &picking, &mut materials),
        (true, Some((entity, panel))) if *panel != want => {
            commands.entity(entity).despawn();
            spawn(&mut commands, &theme, want, &picking, &mut materials);
        }
        (true, Some(_)) => {
            for (swatch, mut background) in &mut swatches {
                let want = theme.color(swatch.0);
                if background.0 != want {
                    background.0 = want;
                }
            }
            for (row, mut background) in &mut rows {
                let want = if row.0 == theme.live.name {
                    ROW_ON
                } else {
                    Color::NONE
                };
                if background.0 != want {
                    background.0 = want;
                }
            }
            // Never while a field is being typed into: the caret is mid-code.
            for (entity, which, mut field) in &mut fields {
                if let Some(shown) = shown(&theme, *which)
                    && focus.get() != Some(entity)
                    && field.value() != shown
                {
                    set_text(&mut field, shown);
                }
            }
            for (label, mut text, mut color) in &mut labels {
                let (want, fg) = legend(&theme, label.0);
                if text.0 != want {
                    text.0 = want.to_owned();
                }
                if color.0 != fg {
                    color.0 = fg;
                }
            }
            if let Some(open) = picking.0 {
                let hsva = Hsva::from(theme.color(open.slot));
                for (knob, mut node) in &mut knobs {
                    let want = match knob.0 {
                        Dial::Plane => Vec2::new(hsva.saturation, 1.0 - hsva.value),
                        Dial::Hue => Vec2::new(open.hue, 0.5),
                    };
                    let want = knob_at(want);
                    if (node.left, node.top) != (want.left, want.top) {
                        (node.left, node.top) = (want.left, want.top);
                    }
                }
                // The plane is drawn for the hue the strip holds, so a hue drag
                // repaints it without respawning anything.
                for (dial, material) in &dials {
                    if *dial == Dial::Plane
                        && let Some(mut asset) = materials.get_mut(&material.0)
                        && asset.dial.hue != open.hue
                    {
                        asset.dial.hue = open.hue;
                    }
                }
            }
        }
        (false, None) => {}
    }
}

fn spawn(
    commands: &mut Commands,
    theme: &Theme,
    panel: ThemePanel,
    picking: &Picking,
    materials: &mut Assets<DialMaterial>,
) {
    let block = theme.live.clone();
    let open = picking.0;
    let targeting = panel.targeting;
    commands
        .spawn((
            panel,
            // Over the node panels, which order themselves from 0 up, and under
            // the script sidebar at 2.
            GlobalZIndex(3),
            Node {
                position_type: PositionType::Absolute,
                left: px(LEFT),
                top: px(TOP),
                width: px(WIDTH),
                padding: UiRect::all(px(PAD)),
                row_gap: px(4.0),
                flex_direction: FlexDirection::Column,
                border_radius: BorderRadius::all(px(8.0)),
                ..default()
            },
            BackgroundColor(PANEL_BG),
        ))
        .with_children(|parent| {
            parent.spawn((heading("theme"), TextColor(LABEL)));
            parent.spawn((field_box(percent(100.0)), tag(&block.name, FG)));

            list(parent, THEMES_H, |parent| {
                for saved in theme.rows() {
                    let on = saved.name == block.name;
                    parent
                        .spawn((
                            ThemeRow(saved.name.clone()),
                            row(),
                            BackgroundColor(row_bg(on)),
                            children![tag(&saved.name, FG)],
                        ))
                        .observe(select_theme);
                }
            });

            parent.spawn((heading("colors"), TextColor(LABEL)));
            list(parent, COLORS_H, |parent| {
                for slot in 0..block.colors.len() {
                    color_row(parent, &block, slot);
                }
            });
            buttons(parent, theme);

            parent.spawn((heading("text"), TextColor(LABEL)));
            for role in 0..FONT_ROLES.len() {
                text_row(parent, theme, role, targeting);
            }

            // Below both lists rather than inside the one that scrolls: a dial
            // half off the top of a scroll box is a dial you cannot aim at.
            if let Some(open) = open.filter(|open| open.slot < block.colors.len()) {
                parent.spawn((
                    heading(&Block::role(open.slot)),
                    TextColor(theme.color(open.slot)),
                ));
                let color = hex(block.color(open.slot).unwrap_or_default()).unwrap_or(DARK);
                picker(parent, materials, color, open.hue);
            }
            if let Some(role) = targeting {
                candidates(parent, theme, role);
            }
        });
}

// `ScrollArea` is what the wheel reaches and `Scrollbar` what the pointer drags.
fn list(
    parent: &mut ChildSpawnerCommands,
    height: f32,
    rows: impl FnOnce(&mut ChildSpawnerCommands),
) {
    parent
        .spawn(Node {
            max_height: px(height),
            column_gap: px(2.0),
            ..default()
        })
        .with_children(|parent| {
            let scrolling = parent
                .spawn((
                    ScrollArea,
                    Node {
                        flex_grow: 1.0,
                        max_height: px(height),
                        flex_direction: FlexDirection::Column,
                        row_gap: px(2.0),
                        overflow: Overflow::scroll_y(),
                        ..default()
                    },
                ))
                .with_children(rows)
                .id();
            parent.spawn((
                Scrollbar::new(scrolling, ControlOrientation::Vertical, THUMB_MIN),
                Node {
                    width: px(SCROLLBAR),
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                BackgroundColor(RAIL_BG),
                // The thumb carries no `Node`: the widget lays it out itself.
                children![(
                    ScrollbarThumb {
                        border_radius: BorderRadius::MAX,
                        ..default()
                    },
                    BackgroundColor(THUMB),
                )],
            ));
        });
}

fn buttons(parent: &mut ChildSpawnerCommands, theme: &Theme) {
    for pair in [[Button::Add, Button::Drop], [Button::Save, Button::Clone]] {
        parent
            .spawn(Node {
                column_gap: px(4.0),
                ..default()
            })
            .with_children(|parent| {
                for which in pair {
                    let (label, color) = legend(theme, which);
                    let mut button = parent.spawn((
                        which,
                        row(),
                        children![(
                            ButtonLabel(which),
                            Text::new(label.to_owned()),
                            TextFont::from_font_size(LABEL_SIZE),
                            TextColor(color),
                        )],
                    ));
                    match which {
                        Button::Add => button.observe(add_color),
                        Button::Drop => button.observe(drop_color),
                        Button::Save => button.observe(save_theme),
                        Button::Clone => button.observe(clone_theme),
                    };
                }
            });
    }
}

// A save says what it would do: the label is the only warning before the name
// in the box changes.
fn legend(theme: &Theme, which: Button) -> (&'static str, Color) {
    let lit = |on: bool| if on { FG } else { LABEL };
    match which {
        Button::Add => ("+ colour", lit(theme.live.colors.len() < MAX_COLORS)),
        Button::Drop => ("\u{2212} colour", lit(theme.live.colors.len() > MIN_COLORS)),
        Button::Clone => ("clone", lit(theme.named() == Named::Custom)),
        // Amber rather than lit: the colour is the whole notice that the board
        // is drawing something the library does not have.
        Button::Save if theme.dirty() => ("save", UNSAVED),
        Button::Save => ("save", LABEL),
    }
}

fn color_row(parent: &mut ChildSpawnerCommands, theme: &Block, slot: usize) {
    let shown = theme.color(slot).unwrap_or_default().to_owned();
    parent
        .spawn(Node {
            height: px(ROW),
            align_items: AlignItems::Center,
            column_gap: px(6.0),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|parent| {
            parent
                .spawn((
                    Swatch(slot),
                    Node {
                        width: px(SWATCH),
                        height: px(SWATCH),
                        border_radius: BorderRadius::all(px(3.0)),
                        ..default()
                    },
                    BackgroundColor(hex(&shown).unwrap_or(DARK)),
                ))
                .observe(open_picker);
            parent.spawn((
                Node {
                    flex_grow: 1.0,
                    overflow: Overflow::clip(),
                    ..default()
                },
                children![tag(&Block::role(slot), LABEL)],
            ));
            parent.spawn((
                Field::Color(slot),
                field_box(px(68.0)),
                editable(&shown),
                TextFont::from_font_size(LABEL_SIZE),
                TextColor(FG),
                TextCursorStyle {
                    color: FG,
                    ..default()
                },
            ));
        });
}

// The three texts. Pressing one unfolds its font list below the sections, in
// the same place and for the same reason the colour picker opens there.
fn text_row(parent: &mut ChildSpawnerCommands, theme: &Theme, role: usize, open: Option<usize>) {
    parent
        .spawn((
            TextRow(role),
            row(),
            BackgroundColor(row_bg(open == Some(role))),
        ))
        .with_children(|parent| {
            parent.spawn((
                Node {
                    flex_grow: 1.0,
                    overflow: Overflow::clip(),
                    ..default()
                },
                children![tag(FONT_ROLES[role], LABEL)],
            ));
            parent.spawn(font_tag(theme, theme.fonts.font(role)));
        })
        .observe(open_targets);
}

// What one text may draw in: the embedded font, the space's own, and -- for the
// two that nothing reads yet -- nothing. The primary text has no `none`,
// because every node's text follows it.
fn candidates(parent: &mut ChildSpawnerCommands, theme: &Theme, role: usize) {
    let chosen = theme.fonts.font(role);
    parent.spawn((heading(FONT_ROLES[role]), TextColor(FG)));
    list(parent, FONTS_H, |parent| {
        let library = theme.fonts.library.iter().map(String::as_str);
        for name in std::iter::once(DEFAULT_FONT).chain(library) {
            // The `-` is a sibling of the row and not a child of it: a press
            // bubbles to its parent, and dropping a font is not choosing it.
            parent
                .spawn(Node {
                    column_gap: px(4.0),
                    flex_shrink: 0.0,
                    ..default()
                })
                .with_children(|parent| {
                    parent
                        .spawn((
                            Candidate(name.to_owned()),
                            Node {
                                flex_grow: 1.0,
                                overflow: Overflow::clip(),
                                ..row()
                            },
                            BackgroundColor(row_bg(chosen == Some(name))),
                            children![font_tag(theme, Some(name))],
                        ))
                        .observe(pick_font);
                    if name != DEFAULT_FONT {
                        parent
                            .spawn((
                                DropFont(name.to_owned()),
                                Node {
                                    padding: UiRect::horizontal(px(5.0)),
                                    align_items: AlignItems::Center,
                                    ..default()
                                },
                                children![tag("\u{2212}", LABEL)],
                            ))
                            .observe(remove_font);
                    }
                });
        }
        if role != PRIMARY_TEXT {
            parent
                .spawn((
                    Candidate(String::new()),
                    row(),
                    BackgroundColor(row_bg(chosen.is_none())),
                    children![font_tag(theme, None)],
                ))
                .observe(pick_font);
        }
    });
}

fn row_bg(on: bool) -> Color {
    if on { ROW_ON } else { Color::NONE }
}

// Drawn in the font it names, which is the only preview a font needs.
fn font_tag(theme: &Theme, name: Option<&str>) -> impl Bundle {
    let (shown, color) = match name {
        Some(name) => (font_label(name), FG),
        None => ("none", LABEL),
    };
    (
        Text::new(shown.to_owned()),
        TextFont {
            font: name.map(|name| theme.source(name)).unwrap_or_default(),
            ..TextFont::from_font_size(LABEL_SIZE)
        },
        TextColor(color),
    )
}

// The library holds a path and the row is 236px wide, so a row shows the file
// and not the folder it is in. A family name has neither and comes through.
fn font_label(name: &str) -> &str {
    let file = name.rsplit('/').next().unwrap_or(name);
    file.rsplit_once('.').map_or(file, |(stem, _)| stem)
}

// One list at a time: the picker and this both open below the sections.
fn open_targets(
    press: On<Pointer<Press>>,
    rows: Query<&TextRow>,
    mut targeting: ResMut<Targeting>,
    mut picking: ResMut<Picking>,
) {
    let Ok(row) = rows.get(press.entity) else {
        return;
    };
    targeting.0 = match targeting.0 {
        Some(open) if open == row.0 => None,
        _ => Some(row.0),
    };
    picking.0 = None;
}

fn pick_font(
    press: On<Pointer<Press>>,
    rows: Query<&Candidate>,
    theme: Res<Theme>,
    mut targeting: ResMut<Targeting>,
    mut document: ResMut<Document>,
) {
    let (Ok(row), Some(role)) = (rows.get(press.entity), targeting.0) else {
        return;
    };
    let mut fonts = theme.fonts.clone();
    fonts.set(role, &row.0);
    write_fonts(&mut document, &fonts);
    targeting.0 = None;
}

// The file stays in the spaces dir: nothing here deletes from it, and a space
// that drops a font keeps it a drop away.
fn remove_font(
    press: On<Pointer<Press>>,
    rows: Query<&DropFont>,
    theme: Res<Theme>,
    mut document: ResMut<Document>,
) {
    let Ok(row) = rows.get(press.entity) else {
        return;
    };
    let mut fonts = theme.fonts.clone();
    fonts.remove(&row.0);
    write_fonts(&mut document, &fonts);
}

// `MaterialNode`s, so the gradient is the GPU's rather than a grid of entities.
fn picker(
    parent: &mut ChildSpawnerCommands,
    materials: &mut Assets<DialMaterial>,
    color: Color,
    hue: f32,
) {
    let hsva = Hsva::from(color);
    let at = Vec2::new(hsva.saturation, 1.0 - hsva.value);
    dial(parent, materials, Dial::Plane, (hue, PLANE_H), at);
    dial(
        parent,
        materials,
        Dial::Hue,
        (hue, STRIP_H),
        Vec2::new(hue, 0.5),
    );
}

fn dial(
    parent: &mut ChildSpawnerCommands,
    materials: &mut Assets<DialMaterial>,
    which: Dial,
    (hue, height): (f32, f32),
    at: Vec2,
) {
    parent
        .spawn((
            which,
            Node {
                width: percent(100.0),
                height: px(height),
                flex_shrink: 0.0,
                border_radius: BorderRadius::all(px(3.0)),
                ..default()
            },
            MaterialNode(materials.add(DialMaterial {
                dial: DialPaint {
                    hue,
                    strip: f32::from(which == Dial::Hue),
                },
            })),
            // A child, so it draws over the material and positions in its box.
            children![(
                Knob(which),
                knob_at(at),
                BorderColor::all(Color::WHITE),
                BackgroundColor(Color::NONE),
                // Or the ring eats the press meant for the colour under it.
                Pickable::IGNORE,
            )],
        ))
        .observe(dial_pressed)
        .observe(dial_dragged);
}

fn knob_at(at: Vec2) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: percent(at.x * 100.0),
        top: percent(at.y * 100.0),
        margin: UiRect::all(px(-KNOB / 2.0)),
        width: px(KNOB),
        height: px(KNOB),
        border: UiRect::all(px(KNOB_EDGE)),
        border_radius: BorderRadius::MAX,
        ..default()
    }
}

fn code(color: Color) -> String {
    let rgb = color.to_srgba();
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(rgb.red),
        channel(rgb.green),
        channel(rgb.blue)
    )
}

fn heading(text: &str) -> impl Bundle {
    (
        Text::new(text.to_owned()),
        TextFont::from_font_size(LABEL_SIZE),
    )
}

fn tag(text: &str, color: Color) -> impl Bundle {
    (
        Text::new(text.to_owned()),
        TextFont::from_font_size(LABEL_SIZE),
        TextColor(color),
    )
}

fn row() -> Node {
    Node {
        height: px(ROW),
        padding: UiRect::horizontal(px(4.0)),
        align_items: AlignItems::Center,
        border_radius: BorderRadius::all(px(3.0)),
        flex_shrink: 0.0,
        ..default()
    }
}

fn field_box(width: Val) -> impl Bundle {
    (
        Node {
            width,
            padding: UiRect::axes(px(5.0), px(2.0)),
            border_radius: BorderRadius::all(px(3.0)),
            ..default()
        },
        BackgroundColor(FIELD_BG),
    )
}

fn editable(value: &str) -> EditableText {
    EditableText {
        allow_newlines: false,
        ..EditableText::new(value)
    }
}

// The editor holds its own font size, and bevy pushes `TextFont` into it only
// when that component changes: replacing the editor leaves it at 100px.
fn set_text(field: &mut EditableText, value: &str) {
    field.editor_mut().set_text(value);
    field.queue_edit(TextEdit::TextEnd(false));
}

fn select_theme(
    press: On<Pointer<Press>>,
    rows: Query<&ThemeRow>,
    theme: Res<Theme>,
    mut document: ResMut<Document>,
) {
    if let Ok(row) = rows.get(press.entity)
        && let Some(picked) = theme.of(&row.0)
    {
        write(&mut document, &picked);
    }
}

fn save_theme(
    _press: On<Pointer<Press>>,
    theme: Res<Theme>,
    naming: ResMut<Naming>,
    document: ResMut<Document>,
) {
    keep(&theme, naming, document);
}

// A preset cannot be written to, so saving an edit to one creates a theme --
// and a new theme needs a name, which the window asks for. A theme of your own
// is written straight back into, with nothing to ask.
fn keep(theme: &Theme, mut naming: ResMut<Naming>, mut document: ResMut<Document>) {
    if !theme.dirty() {
        return;
    }
    let block = theme.live.clone();
    match theme.named() {
        Named::Custom => {
            if document.0.save_theme(&block) {
                write(&mut document, &block);
            }
        }
        Named::Preset | Named::New => {
            naming.0 = Some(document.0.fresh_theme_name(&block.name));
        }
    }
}

// Only a theme of your own: a preset is copied by editing it and saving.
fn clone_theme(
    _press: On<Pointer<Press>>,
    theme: Res<Theme>,
    mut naming: ResMut<Naming>,
    document: Res<Document>,
) {
    if theme.named() == Named::Custom {
        naming.0 = Some(document.0.fresh_theme_name(&theme.live.name));
    }
}

// Enter is the name, escape is no theme. Its box owns the keyboard while up.
#[allow(
    clippy::too_many_arguments,
    reason = "a system's arguments are its query"
)]
fn prompt(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    theme: Res<Theme>,
    mut naming: ResMut<Naming>,
    mut focus: ResMut<InputFocus>,
    prompts: Query<Entity, With<NamePrompt>>,
    boxes: Query<(Entity, &EditableText), With<NewName>>,
    mut document: ResMut<Document>,
) {
    match (naming.0.clone(), prompts.single().ok()) {
        (None, Some(entity)) => {
            commands.entity(entity).despawn();
            focus.clear();
        }
        (Some(offered), None) => spawn_prompt(&mut commands, &offered),
        (Some(_), Some(_)) => {
            if keys.just_pressed(KeyCode::Escape) {
                naming.0 = None;
                return;
            }
            if !keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter]) {
                return;
            }
            // Trimmed, because a name of spaces is no name; `save_theme` is what
            // refuses an empty one, and the window stays up until it takes one.
            let typed = boxes.single().ok();
            let block = Block {
                name: typed
                    .map(|(_, field)| field.value().to_string())
                    .unwrap_or_default()
                    .trim()
                    .to_owned(),
                colors: theme.live.colors.clone(),
            };
            if document.0.save_theme(&block) {
                write(&mut document, &block);
                naming.0 = None;
            } else if let Some((entity, _)) = typed {
                // Enter let go of the caret on its way here, so a refused name
                // has to take it back or the window cannot be typed into.
                focus.set(entity, FocusCause::Pressed);
            }
        }
        (None, None) => {}
    }
}

fn spawn_prompt(commands: &mut Commands, offered: &str) {
    commands.spawn((
        NamePrompt,
        GlobalZIndex(4),
        Node {
            position_type: PositionType::Absolute,
            left: percent(50.0),
            top: percent(30.0),
            margin: UiRect::left(px(-PROMPT.x / 2.0)),
            width: px(PROMPT.x),
            height: px(PROMPT.y),
            padding: UiRect::all(px(PAD)),
            row_gap: px(6.0),
            flex_direction: FlexDirection::Column,
            border_radius: BorderRadius::all(px(8.0)),
            ..default()
        },
        BackgroundColor(PANEL_BG),
        children![
            (
                Text::new("name it \u{2014} enter saves, escape cancels".to_owned()),
                TextFont::from_font_size(LABEL_SIZE),
                TextColor(LABEL),
            ),
            (
                Field::New,
                NewName,
                field_box(percent(100.0)),
                editable(offered),
                TextFont::from_font_size(LABEL_SIZE),
                TextColor(FG),
                TextCursorStyle {
                    color: FG,
                    ..default()
                },
                AutoFocus,
            ),
        ],
    ));
}

// The hue is read off the colour here and nowhere else: once the picker is
// open it is the plane's, not the colour's.
fn open_picker(
    press: On<Pointer<Press>>,
    swatches: Query<&Swatch>,
    theme: Res<Theme>,
    mut picking: ResMut<Picking>,
    mut targeting: ResMut<Targeting>,
) {
    let Ok(swatch) = swatches.get(press.entity) else {
        return;
    };
    targeting.0 = None;
    picking.0 = match picking.0 {
        Some(open) if open.slot == swatch.0 => None,
        _ => Some(Picked {
            slot: swatch.0,
            hue: Hsva::from(theme.color(swatch.0)).hue / 360.0,
        }),
    };
}

// `Press` as well as `Drag`: a single click on a dial has to land a colour, and
// bevy only starts reporting drags once the pointer has moved.
fn dial_pressed(
    press: On<Pointer<Press>>,
    dials: Query<(&Dial, &ComputedNode, &UiGlobalTransform)>,
    theme: ResMut<Theme>,
    picking: ResMut<Picking>,
    document: ResMut<Document>,
) {
    dialled(
        (press.entity, press.pointer_location.position),
        &dials,
        theme,
        picking,
        document,
    );
}

fn dial_dragged(
    drag: On<Pointer<Drag>>,
    dials: Query<(&Dial, &ComputedNode, &UiGlobalTransform)>,
    theme: ResMut<Theme>,
    picking: ResMut<Picking>,
    document: ResMut<Document>,
) {
    dialled(
        (drag.entity, drag.pointer_location.position),
        &dials,
        theme,
        picking,
        document,
    );
}

// Each dial writes its own two channels and leaves the third, which is what
// keeps dragging one from undoing the other.
fn dialled(
    (entity, at): (Entity, Vec2),
    dials: &Query<(&Dial, &ComputedNode, &UiGlobalTransform)>,
    mut theme: ResMut<Theme>,
    mut picking: ResMut<Picking>,
    mut document: ResMut<Document>,
) {
    let (Ok((dial, node, transform)), Some(open)) = (dials.get(entity), picking.0) else {
        return;
    };
    // The pointer is in logical pixels and a laid-out node in physical ones.
    let Some(local) = node.normalize_point(*transform, at / node.inverse_scale_factor()) else {
        return;
    };
    let uv = (local + Vec2::splat(0.5)).clamp(Vec2::ZERO, Vec2::ONE);

    let current = Hsva::from(theme.color(open.slot));
    let hsva = match dial {
        Dial::Hue => Hsva {
            hue: uv.x * 360.0,
            ..current
        },
        Dial::Plane => Hsva::new(open.hue * 360.0, uv.x, 1.0 - uv.y, 1.0),
    };
    if *dial == Dial::Hue {
        picking.0 = Some(Picked { hue: uv.x, ..open });
    }

    let mut block = theme.live.clone();
    if let Some(color) = block.colors.get_mut(open.slot) {
        *color = code(Color::from(hsva));
        // Bypassed like the sides slider: waking the document respawns every
        // node, on every frame of the drag. `Theme` is what the board draws
        // from, so it is written here instead -- and sync.rs and undo.rs poll
        // `rev`, so the edit still saves and still undoes.
        document.bypass_change_detection().0.set_theme(&block);
        theme.live = block;
    }
}

fn add_color(_press: On<Pointer<Press>>, theme: Res<Theme>, mut document: ResMut<Document>) {
    if let Some(block) = resized(&theme.live, 1) {
        write(&mut document, &block);
    }
}

fn drop_color(_press: On<Pointer<Press>>, theme: Res<Theme>, mut document: ResMut<Document>) {
    if let Some(block) = resized(&theme.live, -1) {
        write(&mut document, &block);
    }
}

// One slot either way, `None` at either end. A new slot repeats the last
// colour: a swatch you cannot see is a slot you cannot find.
fn resized(theme: &Block, by: i8) -> Option<Block> {
    let mut colors = theme.colors.clone();
    match by {
        1 if colors.len() < MAX_COLORS => colors.push(colors.last().cloned().unwrap_or_default()),
        -1 if colors.len() > MIN_COLORS => {
            colors.pop();
        }
        _ => return None,
    }
    Some(Block {
        colors,
        ..theme.clone()
    })
}

fn typed(
    theme: Res<Theme>,
    mut fields: Query<(&Field, &mut EditableText), Changed<EditableText>>,
    mut document: ResMut<Document>,
) {
    let mut block = theme.live.clone();
    for (which, mut field) in &mut fields {
        let typed = field.value().to_string();
        match which {
            Field::New => {}
            Field::Color(slot) => {
                // The box holds a colour and nothing else, so anything a colour
                // cannot contain never lands in it.
                let code = clamped(&typed);
                if code != typed {
                    set_text(&mut field, &code);
                }
                if let Some(color) = block.colors.get_mut(*slot)
                    && hex(&code).is_some()
                {
                    *color = code;
                }
            }
        }
    }
    // A field that reads what the theme already says is not an edit: the panel
    // spawns its boxes full, and opening it may not write the file.
    if block != theme.live {
        write(&mut document, &block);
    }
}

// `#` and six hex digits, no more. The hash is the box's own and never leaves.
fn clamped(typed: &str) -> String {
    let mut code = String::with_capacity(7);
    code.push('#');
    code.extend(typed.chars().filter(char::is_ascii_hexdigit).take(6));
    code
}

// What a field reads when nobody is typing into it. The naming box has no such
// source: what is typed there is its whole state until enter or escape.
fn shown(theme: &Theme, which: Field) -> Option<&str> {
    match which {
        Field::Color(slot) => Some(theme.live.color(slot).unwrap_or_default()),
        Field::New => None,
    }
}

// A press that reaches the canvas clears the selection, and one on the panel is
// not. The cursor is logical where a `ComputedNode` is physical.
pub fn over_panel(
    window: Single<&Window>,
    panels: Query<(&ComputedNode, &UiGlobalTransform), With<ThemePanel>>,
) -> bool {
    let Ok((panel, transform)) = panels.single() else {
        return false;
    };
    let Some(at) = window.cursor_position() else {
        return false;
    };
    panel.contains_point(*transform, at / panel.inverse_scale_factor())
}

pub fn typing(focus: Res<InputFocus>, fields: Query<(), With<Field>>) -> bool {
    focus.get().is_some_and(|entity| fields.contains(entity))
}

#[cfg(test)]
mod tests;
