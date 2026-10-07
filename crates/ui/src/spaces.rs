use bevy::diagnostic::FrameCount;
use bevy::input_focus::{AutoFocus, InputFocus};
use bevy::prelude::*;
use bevy::text::{EditableText, TextCursorStyle};
use extboard_core::{Canvas, NodeKind, space_path};
use std::sync::{Arc, Mutex, PoisonError};

use crate::client::{Document, Notice, Space, base_url, space_url};
use crate::select::PanelRoot;
use crate::theme::DARK;

// Over every panel and over the selection band at 1000: this is a screen, not
// a panel on the board.
const Z: i32 = 2_000;
const TILE: Vec2 = Vec2::new(180.0, 96.0);
const GAP: f32 = 12.0;
const PAD: f32 = 24.0;
// Clear of the theme panel below it, which starts at 30.
const CHIP_TOP: f32 = 8.0;

const TILE_BG: Color = Color::srgb(0.16, 0.17, 0.21);
const HOVER: Color = Color::srgb(0.22, 0.24, 0.3);
const EDGE: Color = Color::srgb(0.25, 0.27, 0.32);
const FG: Color = Color::srgb(0.86, 0.88, 0.92);
const LABEL: Color = Color::srgb(0.45, 0.5, 0.55);

pub struct SpacesPlugin;

#[derive(Debug)]
enum Reply {
    Listed(Vec<String>),
    Created(String),
    Inner {
        parent: String,
        name: String,
        id: String,
        at: Vec2,
    },
    Path(String, Vec<String>),
    Renamed {
        from: String,
        to: String,
    },
    Failed(String),
}

type Inbox = Arc<Mutex<Vec<Reply>>>;

// The ids `GET /api/spaces` last returned.
#[derive(Resource, Default)]
struct Spaces(Vec<String>);

// Separate from `Spaces` so draining it is not a change to the list: the screen
// is rebuilt when the list changes, and rebuilding it every frame would take
// the hover state and the click with it.
#[derive(Resource, Default)]
pub struct Pending(Inbox);

// The open space and the ones it was made inside, root first.
#[derive(Resource, Default)]
struct Trail(Vec<String>);

#[derive(Component)]
struct Chrome;

// The tile a right-click opened the menu on, and where on screen.
#[derive(Resource, Default)]
struct Menu(Option<(String, Vec2)>);

// The space whose tile is a text field for now.
#[derive(Resource, Default)]
struct Renaming(Option<String>);

#[derive(Component)]
struct RenameField;

// A space about to be made inside `parent`: its name is being typed into a field
// at `screen`, and its door lands at `world`.
#[derive(Resource, Default)]
pub struct Naming(pub Option<Draft>);

pub struct Draft {
    pub parent: String,
    pub world: Vec2,
    pub screen: Vec2,
}

#[derive(Component)]
pub struct NameField;

#[derive(Component, Clone)]
enum Hit {
    Open(String),
    New,
    // A breadcrumb, which is not a tile: a right-click on it renames nothing.
    Crumb(String),
    Back,
    Rename(String),
}

impl Plugin for SpacesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Spaces>()
            .init_resource::<Pending>()
            .init_resource::<Trail>()
            .init_resource::<Menu>()
            .init_resource::<Renaming>()
            .init_resource::<Naming>()
            .add_systems(
                Update,
                (
                    // Also on frame one, when `Space` is inserted: whichever
                    // way the app opened, the list behind it is current.
                    refresh.run_if(resource_changed::<Space>),
                    receive,
                    commit,
                    name,
                    draw.run_if(
                        resource_changed::<Spaces>
                            .or_else(resource_changed::<Space>)
                            .or_else(resource_changed::<Trail>)
                            .or_else(resource_changed::<Menu>)
                            .or_else(resource_changed::<Renaming>)
                            .or_else(resource_changed::<Naming>),
                    ),
                )
                    .chain(),
            );
    }
}

// The space a whole node leads to: a link node, or a text node that is nothing
// but one link to a space, the door `door_text` writes.
pub fn linked(canvas: &Canvas, node: &str) -> Option<String> {
    let node = canvas.nodes.iter().find(|candidate| candidate.id == node)?;
    match &node.kind {
        NodeKind::Link { url } => space_path(url).map(str::to_owned),
        NodeKind::Text { text } => only_link(text),
        _ => None,
    }
}

fn only_link(text: &str) -> Option<String> {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};
    let (mut dest, mut links, mut inside) = (None, 0, false);
    for event in Parser::new(text) {
        match event {
            Event::Start(Tag::Link { dest_url, .. }) => {
                (dest, inside) = (Some(dest_url), true);
                links += 1;
            }
            Event::End(TagEnd::Link) => inside = false,
            Event::Start(Tag::Paragraph) | Event::End(TagEnd::Paragraph) => {}
            _ if inside => {}
            _ => return None,
        }
    }
    (links == 1).then(|| space_path(&dest?).map(str::to_owned))?
}

fn refresh(pending: Res<Pending>, space: Res<Space>) {
    let inbox = pending.0.clone();
    let request = ehttp::Request::get(format!("{}/api/spaces", base_url()));
    ehttp::fetch(request, move |result| {
        lock(&inbox).push(listed(result));
    });
    if let Some(id) = space.id() {
        let (inbox, id) = (pending.0.clone(), id.to_owned());
        let request = ehttp::Request::get(format!("{}/path", space_url(&id)));
        ehttp::fetch(request, move |result| {
            let path = result
                .ok()
                .filter(|response| response.ok)
                .and_then(|response| serde_json::from_slice(&response.bytes).ok());
            // No path is still a breadcrumb: the space alone.
            lock(&inbox).push(Reply::Path(id.clone(), path.unwrap_or_else(|| vec![id])));
        });
    }
}

fn listed(result: ehttp::Result<ehttp::Response>) -> Reply {
    match result {
        Err(e) => Reply::Failed(format!("extd unreachable at {}: {e}", base_url())),
        Ok(response) if !response.ok => Reply::Failed(format!("extd returned {}", response.status)),
        Ok(response) => match serde_json::from_slice(&response.bytes) {
            Ok(ids) => Reply::Listed(ids),
            Err(e) => Reply::Failed(format!("bad space list from extd: {e}")),
        },
    }
}

// The server names a new space: the list does not show the inner ones, so
// only it knows which `untitled` is free.
fn create(pending: &Pending) {
    let inbox = pending.0.clone();
    let request = ehttp::Request::post(format!("{}/api/spaces", base_url()), Vec::new());
    ehttp::fetch(request, move |result| {
        lock(&inbox).push(created(result).map_or_else(Reply::Failed, Reply::Created));
    });
}

// A space called `id` inside `parent`, and a door to it on `parent` at `at`.
// The server makes the id, `<parent>_<name>`, and answers with it.
fn create_inner(pending: &Pending, parent: String, name: String, at: Vec2) {
    let inbox = pending.0.clone();
    let request = ehttp::Request::post(space_url(&name), parent.clone().into());
    ehttp::fetch(request, move |result| {
        lock(&inbox).push(made_inner(parent, name, at, result));
    });
}

fn made_inner(
    parent: String,
    name: String,
    at: Vec2,
    result: ehttp::Result<ehttp::Response>,
) -> Reply {
    let refused = match &result {
        Ok(response) if response.status == 409 => Some(format!("{name} already exists here")),
        Ok(response) if response.status == 400 => Some(format!("{name} is not a usable name")),
        _ => None,
    };
    match (refused, created(result)) {
        (Some(message), _) | (None, Err(message)) => Reply::Failed(message),
        (None, Ok(id)) => Reply::Inner {
            parent,
            name,
            id,
            at,
        },
    }
}

// The door to a new inner space: its name, linked. Text rather than a link node,
// so it is edited like any other.
fn door_text(name: &str, id: &str) -> String {
    let escape = |text: &str, special: &[char]| {
        text.chars().fold(String::new(), |mut out, c| {
            if special.contains(&c) || c == '\\' {
                out.push('\\');
            }
            out.push(c);
            out
        })
    };
    format!(
        "[{}](</s/{}>)",
        escape(name, &['[', ']']),
        escape(id, &['<', '>'])
    )
}

fn created(result: ehttp::Result<ehttp::Response>) -> Result<String, String> {
    match result {
        Err(e) => Err(format!("extd unreachable at {}: {e}", base_url())),
        Ok(response) if response.status == 201 => String::from_utf8(response.bytes)
            .map_err(|_| "extd named the new space in something other than UTF-8".to_owned()),
        Ok(response) => Err(format!("extd refused a new space: {}", response.status)),
    }
}

fn rename(pending: &Pending, from: String, to: String) {
    let inbox = pending.0.clone();
    let request = ehttp::Request::post(format!("{}/rename", space_url(&from)), to.clone().into());
    ehttp::fetch(request, move |result| {
        lock(&inbox).push(renamed(from, to, result));
    });
}

fn renamed(from: String, to: String, result: ehttp::Result<ehttp::Response>) -> Reply {
    match result {
        Err(e) => Reply::Failed(format!("extd unreachable at {}: {e}", base_url())),
        Ok(response) if response.ok => Reply::Renamed { from, to },
        Ok(response) if response.status == 409 => Reply::Failed(format!("{to} already exists")),
        Ok(response) if response.status == 400 => {
            Reply::Failed(format!("{to} is not a usable name"))
        }
        Ok(response) => Reply::Failed(format!("extd refused the rename: {}", response.status)),
    }
}

// Enter renames, escape leaves the name as it was.
fn commit(
    keys: Res<ButtonInput<KeyCode>>,
    fields: Query<&EditableText, With<RenameField>>,
    pending: Res<Pending>,
    mut renaming: ResMut<Renaming>,
) {
    let Some(from) = renaming.0.clone() else {
        return;
    };
    if keys.just_pressed(KeyCode::Escape) {
        renaming.0 = None;
        return;
    }
    if !keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter]) {
        return;
    }
    let Ok(field) = fields.single() else {
        return;
    };
    let to = field.value().to_string().trim().to_owned();
    renaming.0 = None;
    if !to.is_empty() && to != from {
        rename(&pending, from, to);
    }
}

// Enter makes the space, escape or a press anywhere else drops it.
fn name(
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    fields: Query<&EditableText, With<NameField>>,
    frames: Res<FrameCount>,
    panel: Res<crate::select::PanelPress>,
    pending: Res<Pending>,
    mut naming: ResMut<Naming>,
) {
    if naming.0.is_none() {
        return;
    }
    // The field sits in a panel, so a press on it is not a press away.
    let pressed_away = buttons.get_just_pressed().next().is_some()
        && !crate::select::pressed_a_panel(frames, panel);
    if keys.just_pressed(KeyCode::Escape) || pressed_away {
        naming.0 = None;
        return;
    }
    if !keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter]) {
        return;
    }
    let Ok(field) = fields.single() else {
        return;
    };
    let id = field.value().to_string().trim().to_owned();
    let Some(draft) = naming.0.take() else {
        return;
    };
    if !id.is_empty() {
        create_inner(&pending, draft.parent, id, draft.world);
    }
}

// While the name field has the keyboard, backspace is a character, not a node.
pub fn typing(focus: Res<InputFocus>, fields: Query<(), With<NameField>>) -> bool {
    focus.get().is_some_and(|entity| fields.contains(entity))
}

fn receive(
    pending: Res<Pending>,
    frames: Res<FrameCount>,
    mut spaces: ResMut<Spaces>,
    mut space: ResMut<Space>,
    mut trail: ResMut<Trail>,
    mut notice: ResMut<Notice>,
    mut document: Option<ResMut<Document>>,
) {
    let batch = std::mem::take(&mut *lock(&pending.0));
    for reply in batch {
        match reply {
            Reply::Listed(ids) => {
                spaces.0 = ids;
                // A list that arrives clears the one that did not.
                if notice.0.is_some() {
                    notice.0 = None;
                }
            }
            // Made, so opened: the list it is missing from is refreshed by the
            // switch itself.
            Reply::Created(id) => space.0 = Some(id),
            // Left behind if the board changed meanwhile: the space is made,
            // and a door on some other board would be a surprise.
            Reply::Inner {
                parent,
                name,
                id,
                at,
            } => {
                if let Some(document) = document.as_mut().filter(|_| space.id() == Some(&parent)) {
                    let rect = Rect::from_center_size(at, crate::edit::NEW_SIZE);
                    let text = door_text(&name, &id);
                    crate::edit::added(&mut document.0, frames.0, rect, NodeKind::Text { text });
                }
            }
            // A late reply for a space already left is dropped.
            Reply::Path(id, path) => {
                if space.id() == Some(&id) {
                    trail.0 = path;
                }
            }
            Reply::Renamed { from, to } => {
                spaces.0.retain(|id| *id != from);
                spaces.0.push(to);
                spaces.0.sort();
            }
            Reply::Failed(message) => {
                error!("{message}");
                notice.0 = Some(message);
            }
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "a system's arguments are its query"
)]
fn draw(
    mut commands: Commands,
    space: Res<Space>,
    spaces: Res<Spaces>,
    trail: Res<Trail>,
    naming: Res<Naming>,
    menu: Res<Menu>,
    renaming: Res<Renaming>,
    existing: Query<Entity, With<Chrome>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }

    // A board on screen gets its name and the way back to the list.
    if let Some(id) = space.id() {
        // Until the path for this space lands, it is the space alone.
        let path = match trail.0.last() {
            Some(last) if last == id => trail.0.clone(),
            _ => vec![id.to_owned()],
        };
        let last = path.len() - 1;
        commands.spawn(title()).with_children(|parent| {
            for (n, crumb) in crumbs(path.len()).into_iter().enumerate() {
                if n > 0 {
                    parent.spawn(label(">", LABEL));
                }
                match crumb {
                    Some(at) if at == last => {
                        parent.spawn(label(shown(&path, at), FG));
                    }
                    Some(at) => button(parent, Hit::Crumb(path[at].clone()), shown(&path, at)),
                    None => {
                        parent.spawn(label("...", LABEL));
                    }
                }
            }
        });
        commands.spawn(chip()).with_children(|parent| {
            button(parent, Hit::Back, "\u{2190} spaces");
        });
        if let Some(draft) = naming.0.as_ref().filter(|draft| draft.parent == id) {
            commands.spawn(menu_at(draft.screen)).with_child((
                NameField,
                Node {
                    width: px(TILE.x),
                    padding: UiRect::axes(px(6.0), px(4.0)),
                    ..default()
                },
                EditableText::new(""),
                TextFont::from_font_size(13.0),
                TextColor(FG),
                TextCursorStyle {
                    color: FG,
                    ..default()
                },
                AutoFocus,
            ));
        }
        return;
    }

    if let Some((id, at)) = &menu.0 {
        commands.spawn(menu_at(*at)).with_children(|parent| {
            button(parent, Hit::Rename(id.clone()), "rename");
        });
    }

    commands
        .spawn(screen())
        .observe(dismiss)
        .with_children(|parent| {
            parent.spawn((
                Text::new("spaces"),
                TextFont::from_font_size(20.0),
                TextColor(FG),
            ));
            parent.spawn(grid()).with_children(|parent| {
                for id in &spaces.0 {
                    let editing = renaming.0.as_ref() == Some(id);
                    tile(parent, Hit::Open(id.clone()), id, editing);
                }
                tile(parent, Hit::New, "+ new space", false);
            });
        });
}

#[allow(
    clippy::too_many_arguments,
    reason = "a system's arguments are its query"
)]
fn pick(
    mut press: On<Pointer<Press>>,
    hits: Query<&Hit>,
    pending: Res<Pending>,
    mut space: ResMut<Space>,
    mut menu: ResMut<Menu>,
    mut renaming: ResMut<Renaming>,
) {
    // The screen behind the tile reads a press as a click away.
    press.propagate(false);
    let Ok(hit) = hits.get(press.entity) else {
        return;
    };
    if press.button == PointerButton::Secondary {
        if let Hit::Open(id) = hit {
            menu.0 = Some((id.clone(), press.pointer_location.position));
        }
        return;
    }
    // Conditional, or a press inside the field redraws it out from under the caret.
    if menu.0.is_some() {
        menu.0 = None;
    }
    match hit {
        Hit::Open(id) if renaming.0.as_ref() == Some(id) => {}
        Hit::Open(id) => space.0 = Some(id.clone()),
        Hit::Back => space.0 = None,
        Hit::New => create(&pending),
        Hit::Crumb(id) => space.0 = Some(id.clone()),
        Hit::Rename(id) => renaming.0 = Some(id.clone()),
    }
}

fn dismiss(_: On<Pointer<Press>>, mut menu: ResMut<Menu>, mut renaming: ResMut<Renaming>) {
    if menu.0.is_some() {
        menu.0 = None;
    }
    if renaming.0.is_some() {
        renaming.0 = None;
    }
}

fn highlight(over: On<Pointer<Over>>, mut hits: Query<&mut BackgroundColor, With<Hit>>) {
    if let Ok(mut color) = hits.get_mut(over.entity) {
        color.0 = HOVER;
    }
}

fn unhighlight(out: On<Pointer<Out>>, mut hits: Query<&mut BackgroundColor, With<Hit>>) {
    if let Ok(mut color) = hits.get_mut(out.entity) {
        color.0 = TILE_BG;
    }
}

// Opaque and the full window: the board behind it is empty while this is up,
// but the script sidebar and the notice line are not.
fn screen() -> impl Bundle {
    (
        Chrome,
        // A press in here is this screen's; the board stands down for it.
        PanelRoot,
        GlobalZIndex(Z),
        Node {
            position_type: PositionType::Absolute,
            left: px(0.0),
            top: px(0.0),
            width: percent(100.0),
            height: percent(100.0),
            padding: UiRect::all(px(PAD)),
            row_gap: px(PAD),
            flex_direction: FlexDirection::Column,
            ..default()
        },
        BackgroundColor(DARK),
        Pickable::default(),
    )
}

fn grid() -> impl Bundle {
    Node {
        width: percent(100.0),
        flex_wrap: FlexWrap::Wrap,
        align_content: AlignContent::FlexStart,
        row_gap: px(GAP),
        column_gap: px(GAP),
        ..default()
    }
}

fn chip() -> impl Bundle {
    (
        Chrome,
        PanelRoot,
        GlobalZIndex(Z),
        Node {
            position_type: PositionType::Absolute,
            right: px(8.0),
            top: px(CHIP_TOP),
            ..default()
        },
    )
}

// Like a folder path, as indices into it: root, `...` for whatever is between
// past three, then the last two. `None` is the `...`.
fn crumbs(len: usize) -> Vec<Option<usize>> {
    match len {
        ..=3 => (0..len).map(Some).collect(),
        _ => vec![Some(0), None, Some(len - 2), Some(len - 1)],
    }
}

// A space by the name it was given: its id less the `<parent>_` the server put
// in front of it. A parent renamed since leaves the whole id.
fn shown(path: &[String], at: usize) -> &str {
    let id = &path[at];
    at.checked_sub(1)
        .and_then(|up| id.strip_prefix(path[up].as_str())?.strip_prefix('_'))
        .unwrap_or(id)
}

fn title() -> impl Bundle {
    (
        Chrome,
        PanelRoot,
        GlobalZIndex(Z),
        Node {
            position_type: PositionType::Absolute,
            left: px(8.0),
            top: px(CHIP_TOP),
            column_gap: px(6.0),
            align_items: AlignItems::Center,
            ..default()
        },
    )
}

fn label(text: &str, color: Color) -> impl Bundle {
    (
        Text::new(text.to_owned()),
        TextFont::from_font_size(14.0),
        TextColor(color),
    )
}

fn menu_at(at: Vec2) -> impl Bundle {
    (
        Chrome,
        PanelRoot,
        GlobalZIndex(Z + 1),
        Node {
            position_type: PositionType::Absolute,
            left: px(at.x),
            top: px(at.y),
            padding: UiRect::all(px(4.0)),
            border: UiRect::all(px(1.0)),
            border_radius: BorderRadius::all(px(6.0)),
            ..default()
        },
        BackgroundColor(DARK),
        BorderColor::all(EDGE),
    )
}

fn tile(parent: &mut ChildSpawnerCommands, hit: Hit, label: &str, editing: bool) {
    let new = matches!(hit, Hit::New);
    let mut tile = parent.spawn((
        hit,
        Node {
            width: px(TILE.x),
            height: px(TILE.y),
            padding: UiRect::all(px(10.0)),
            border: UiRect::all(px(1.0)),
            border_radius: BorderRadius::all(px(8.0)),
            align_items: AlignItems::FlexEnd,
            ..default()
        },
        BackgroundColor(TILE_BG),
        BorderColor::all(EDGE),
        Pickable::default(),
    ));
    if editing {
        tile.with_child((
            RenameField,
            Node {
                width: percent(100.0),
                ..default()
            },
            EditableText::new(label),
            TextFont::from_font_size(13.0),
            TextColor(FG),
            TextCursorStyle {
                color: FG,
                ..default()
            },
            AutoFocus,
        ));
    } else {
        tile.with_child((
            Text::new(label.to_owned()),
            TextFont::from_font_size(13.0),
            TextColor(if new { LABEL } else { FG }),
        ));
    }
    tile.observe(pick).observe(highlight).observe(unhighlight);
}

fn button(parent: &mut ChildSpawnerCommands, hit: Hit, label: &str) {
    parent
        .spawn((
            hit,
            Node {
                padding: UiRect::axes(px(8.0), px(4.0)),
                border_radius: BorderRadius::all(px(6.0)),
                ..default()
            },
            BackgroundColor(TILE_BG),
            Pickable::default(),
            children![(
                Text::new(label.to_owned()),
                TextFont::from_font_size(12.0),
                TextColor(FG),
            )],
        ))
        .observe(pick)
        .observe(highlight)
        .observe(unhighlight);
}

fn lock(inbox: &Inbox) -> std::sync::MutexGuard<'_, Vec<Reply>> {
    inbox.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas(url: &str) -> Canvas {
        serde_json::from_str(&format!(
            r#"{{"nodes":[
                 {{"id":"n1","x":0,"y":0,"width":10,"height":10,"type":"link","url":"{url}"}},
                 {{"id":"n2","x":0,"y":0,"width":10,"height":10,"type":"text","text":"/s/x"}}
               ],"edges":[]}}"#
        ))
        .expect("a canvas")
    }

    #[test]
    fn only_a_link_node_pointing_at_a_space_is_a_door() {
        assert_eq!(linked(&canvas("/s/trip"), "n1").as_deref(), Some("trip"));
        assert_eq!(linked(&canvas("/s/trip/"), "n1").as_deref(), Some("trip"));
        for url in ["https://example.com", "/s/", "/v/trip", "trip", ""] {
            assert_eq!(linked(&canvas(url), "n1"), None, "{url}");
        }
        assert_eq!(linked(&canvas("/s/trip"), "n2"), None);
        // A text node that is only a door is one, whole; one with words around it is not.
        let text = |text: &str| {
            let mut canvas = canvas("");
            canvas.nodes[1].kind = NodeKind::Text {
                text: text.to_owned(),
            };
            canvas
        };
        assert_eq!(
            linked(&text(&door_text("notes", "a_notes")), "n2").as_deref(),
            Some("a_notes")
        );
        assert_eq!(
            linked(&text("[notes](</s/a b>)\n"), "n2").as_deref(),
            Some("a b")
        );
        for other in [
            "see [notes](/s/a)",
            "[a](/s/a) [b](/s/b)",
            "[web](https://x.y)",
            "plain",
        ] {
            assert_eq!(linked(&text(other), "n2"), None, "{other}");
        }
        assert_eq!(linked(&canvas("/s/trip"), "gone"), None);
    }

    #[test]
    fn a_deep_path_keeps_its_root_and_its_last_two() {
        assert_eq!(crumbs(3), [Some(0), Some(1), Some(2)]);
        assert_eq!(crumbs(5), [Some(0), None, Some(3), Some(4)]);
        assert_eq!(crumbs(1), [Some(0)]);
    }

    #[test]
    fn a_crumb_shows_its_own_name_not_its_parents() {
        let path = ["home", "home_notes", "home_notes_a_b", "kitchen"].map(String::from);
        let names: Vec<_> = (0..path.len()).map(|at| shown(&path, at)).collect();
        assert_eq!(names, ["home", "notes", "a_b", "kitchen"]);
    }

    #[test]
    fn a_door_is_its_name_linked_and_both_survive_markdown() {
        assert_eq!(door_text("notes", "home_notes"), "[notes](</s/home_notes>)");
        assert_eq!(door_text("a [b]", "x_a [b]"), r"[a \[b\]](</s/x_a [b]>)");
        // What the board reads back out of it is the id, whole.
        for (name, id) in [
            ("notes", "home_notes"),
            ("a [b]", "x_a [b]"),
            ("<odd>", "x_<odd>"),
        ] {
            let text = door_text(name, id);
            let dest = pulldown_cmark::Parser::new(&text).find_map(|event| match event {
                pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link { dest_url, .. }) => {
                    Some(dest_url)
                }
                _ => None,
            });
            assert_eq!(space_path(&dest.expect("a link")), Some(id), "{name}");
        }
    }

    #[test]
    fn a_list_that_is_not_a_list_is_a_notice_not_a_panic() {
        let response = |status: u16, body: &str| {
            Ok(ehttp::Response {
                url: String::new(),
                ok: (200..300).contains(&status),
                status,
                status_text: String::new(),
                headers: ehttp::Headers::new(&[]),
                bytes: body.as_bytes().to_vec(),
            })
        };
        assert!(matches!(
            listed(response(200, r#"["a","b"]"#)),
            Reply::Listed(ids) if ids == ["a", "b"]
        ));
        assert!(matches!(listed(response(200, "<html>")), Reply::Failed(_)));
        assert!(matches!(listed(response(500, "")), Reply::Failed(_)));
        assert!(matches!(
            listed(Err("connection refused".to_owned())),
            Reply::Failed(_)
        ));
        // The server names the space, in the body.
        assert_eq!(
            created(response(201, "untitled-2")),
            Ok("untitled-2".to_owned())
        );
        assert!(created(response(404, "")).is_err());
        assert!(matches!(
            renamed("a".to_owned(), "b".to_owned(), response(204, "")),
            Reply::Renamed { from, to } if from == "a" && to == "b"
        ));
        assert!(matches!(
            renamed("a".to_owned(), "b".to_owned(), response(409, "")),
            Reply::Failed(message) if message == "b already exists"
        ));
    }
}
