use bevy::input_focus::AutoFocus;
use bevy::prelude::*;
use bevy::text::{EditableText, TextCursorStyle};
use extboard_core::{Canvas, NodeKind, space_path};
use std::sync::{Arc, Mutex, PoisonError};

use crate::client::{Notice, Space, base_url, space_url};
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
    Renamed { from: String, to: String },
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
struct Pending(Inbox);

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

#[derive(Component, Clone)]
enum Hit {
    Open(String),
    New,
    Back,
    Rename(String),
}

impl Plugin for SpacesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Spaces>()
            .init_resource::<Pending>()
            .init_resource::<Menu>()
            .init_resource::<Renaming>()
            .add_systems(
                Update,
                (
                    // Also on frame one, when `Space` is inserted: whichever
                    // way the app opened, the list behind it is current.
                    refresh.run_if(resource_changed::<Space>),
                    receive,
                    commit,
                    draw.run_if(
                        resource_changed::<Spaces>
                            .or_else(resource_changed::<Space>)
                            .or_else(resource_changed::<Menu>)
                            .or_else(resource_changed::<Renaming>),
                    ),
                )
                    .chain(),
            );
    }
}

pub fn linked(canvas: &Canvas, node: &str) -> Option<String> {
    let node = canvas.nodes.iter().find(|candidate| candidate.id == node)?;
    match &node.kind {
        NodeKind::Link { url } => space_path(url).map(str::to_owned),
        _ => None,
    }
}

fn refresh(pending: Res<Pending>) {
    let inbox = pending.0.clone();
    let request = ehttp::Request::get(format!("{}/api/spaces", base_url()));
    ehttp::fetch(request, move |result| {
        lock(&inbox).push(listed(result));
    });
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

fn create(pending: &Pending, id: String) {
    let inbox = pending.0.clone();
    let request = ehttp::Request::post(space_url(&id), Vec::new());
    ehttp::fetch(request, move |result| {
        lock(&inbox).push(created(id, result));
    });
}

fn created(id: String, result: ehttp::Result<ehttp::Response>) -> Reply {
    match result {
        Err(e) => Reply::Failed(format!("extd unreachable at {}: {e}", base_url())),
        Ok(response) if response.status == 201 => Reply::Created(id),
        Ok(response) if response.status == 409 => Reply::Failed(format!("{id} already exists")),
        Ok(response) => Reply::Failed(format!("extd refused a new space: {}", response.status)),
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

// The first `untitled` nobody has taken.
fn untitled(ids: &[String]) -> String {
    let mut name = "untitled".to_owned();
    let mut n = 1;
    while ids.contains(&name) {
        n += 1;
        name = format!("untitled-{n}");
    }
    name
}

fn receive(
    pending: Res<Pending>,
    mut spaces: ResMut<Spaces>,
    mut space: ResMut<Space>,
    mut notice: ResMut<Notice>,
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

fn draw(
    mut commands: Commands,
    space: Res<Space>,
    spaces: Res<Spaces>,
    menu: Res<Menu>,
    renaming: Res<Renaming>,
    existing: Query<Entity, With<Chrome>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }

    // A board on screen gets its name and the way back to the list.
    if let Some(id) = space.id() {
        commands.spawn(title(id));
        commands.spawn(chip()).with_children(|parent| {
            button(parent, Hit::Back, "\u{2190} spaces");
        });
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
    spaces: Res<Spaces>,
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
        Hit::New => create(&pending, untitled(&spaces.0)),
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

fn title(id: &str) -> impl Bundle {
    (
        Chrome,
        GlobalZIndex(Z),
        Text::new(id.to_owned()),
        TextFont::from_font_size(14.0),
        TextColor(FG),
        Node {
            position_type: PositionType::Absolute,
            left: px(8.0),
            top: px(CHIP_TOP),
            ..default()
        },
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
        assert_eq!(linked(&canvas("/s/trip"), "gone"), None);
    }

    // A new space cannot land on one that exists: the POST would 409, and the
    // name on the tile would be somebody else's board.
    #[test]
    fn a_new_space_takes_the_first_free_name() {
        assert_eq!(untitled(&[]), "untitled");
        assert_eq!(untitled(&["untitled".to_owned()]), "untitled-2");
        assert_eq!(
            untitled(&["untitled".to_owned(), "untitled-2".to_owned()]),
            "untitled-3"
        );
        // Order is the server's, and a gap is still a free name.
        assert_eq!(
            untitled(&["untitled-2".to_owned(), "trip".to_owned()]),
            "untitled"
        );
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
        // The id is the client's, not the response's: a 201 carries no body.
        assert!(matches!(
            created("fresh".to_owned(), response(201, "")),
            Reply::Created(id) if id == "fresh"
        ));
        assert!(matches!(
            created("fresh".to_owned(), response(409, "")),
            Reply::Failed(_)
        ));
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
