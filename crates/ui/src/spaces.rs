use bevy::prelude::*;
use std::sync::{Arc, Mutex, PoisonError};

use crate::client::{BASE_URL, Notice, Space, space_url};
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

#[derive(Component, Clone)]
enum Hit {
    Open(String),
    New,
    Back,
}

impl Plugin for SpacesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Spaces>()
            .init_resource::<Pending>()
            .add_systems(
                Update,
                (
                    // Also on frame one, when `Space` is inserted: whichever
                    // way the app opened, the list behind it is current.
                    refresh.run_if(resource_changed::<Space>),
                    receive,
                    draw.run_if(resource_changed::<Spaces>.or_else(resource_changed::<Space>)),
                )
                    .chain(),
            );
    }
}

fn refresh(pending: Res<Pending>) {
    let inbox = pending.0.clone();
    let request = ehttp::Request::get(format!("{BASE_URL}/api/spaces"));
    ehttp::fetch(request, move |result| {
        lock(&inbox).push(listed(result));
    });
}

fn listed(result: ehttp::Result<ehttp::Response>) -> Reply {
    match result {
        Err(e) => Reply::Failed(format!("extd unreachable at {BASE_URL}: {e}")),
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
        Err(e) => Reply::Failed(format!("extd unreachable at {BASE_URL}: {e}")),
        Ok(response) if response.status == 201 => Reply::Created(id),
        Ok(response) if response.status == 409 => Reply::Failed(format!("{id} already exists")),
        Ok(response) => Reply::Failed(format!("extd refused a new space: {}", response.status)),
    }
}

// The first `untitled` nobody has taken. Naming is the rename this has not got.
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
    existing: Query<Entity, With<Chrome>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }

    // A board on screen gets the way back to the list, and nothing more.
    if space.0.is_some() {
        commands.spawn(chip()).with_children(|parent| {
            button(parent, Hit::Back, "\u{2190} spaces");
        });
        return;
    }

    commands.spawn(screen()).with_children(|parent| {
        parent.spawn((
            Text::new("spaces"),
            TextFont::from_font_size(20.0),
            TextColor(FG),
        ));
        parent.spawn(grid()).with_children(|parent| {
            for id in &spaces.0 {
                tile(parent, Hit::Open(id.clone()), id);
            }
            tile(parent, Hit::New, "+ new space");
        });
    });
}

fn pick(
    press: On<Pointer<Press>>,
    hits: Query<&Hit>,
    pending: Res<Pending>,
    spaces: Res<Spaces>,
    mut space: ResMut<Space>,
) {
    match hits.get(press.entity) {
        Ok(Hit::Open(id)) => space.0 = Some(id.clone()),
        Ok(Hit::Back) => space.0 = None,
        Ok(Hit::New) => create(&pending, untitled(&spaces.0)),
        Err(_) => {}
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

fn tile(parent: &mut ChildSpawnerCommands, hit: Hit, label: &str) {
    let new = matches!(hit, Hit::New);
    parent
        .spawn((
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
            children![(
                Text::new(label.to_owned()),
                TextFont::from_font_size(13.0),
                TextColor(if new { LABEL } else { FG }),
            )],
        ))
        .observe(pick)
        .observe(highlight)
        .observe(unhighlight);
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
    }
}
