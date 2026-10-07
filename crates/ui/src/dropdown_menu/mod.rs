use bevy::clipboard::Clipboard;
use bevy::diagnostic::FrameCount;
use bevy::input::InputSystems;
use bevy::prelude::*;

use crate::client::Document;
use crate::edit::{duplicated, in_group, ungrouped};
use crate::node::{NodeId, NodeRect};
use crate::select::{bounds, cursor_world, pick, pressed_a_panel};

const WIDTH: f32 = 180.0;
const PAD: f32 = 5.0;
const ROW: f32 = 13.0;
// What a row and the padding around them stand to be, for keeping the menu
// inside the window; an estimate, because layout has not run yet.
const ROW_HEIGHT: f32 = 24.0;
const FRAME_HEIGHT: f32 = 10.0;

const BG: Color = Color::srgb(0.12, 0.13, 0.16);
const FG: Color = Color::srgb(0.86, 0.88, 0.92);
const HOVER: Color = Color::srgb(0.2, 0.22, 0.27);
const EDGE: Color = Color::srgb(0.25, 0.27, 0.32);

pub struct MenuPlugin;

#[derive(Component)]
struct ContextMenu;

// `swallow` consumes the press, so the closing it implies has to be carried on
// the entity rather than read back off an input it has already cleared.
#[derive(Component)]
struct Closing;

// The row, and the node and board point it was opened on. Copied onto every row
// rather than looked up through the parent: this is a few rows.
#[derive(Component)]
struct Item {
    action: Action,
    node: String,
    at: Vec2,
}

#[derive(Clone, Copy)]
enum Action {
    Copy,
    Duplicate,
    CopyId,
    Ungroup,
    NewSpace,
}

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        // Before every system that reads the same press, canvas and editors
        // alike, so a frame is either a menu frame or a canvas one.
        app.add_systems(
            PreUpdate,
            swallow.after(InputSystems).before(crate::text::ReadPress),
        )
        .add_systems(
            Update,
            // Only `open` waits for the document, and only to keep a row — whose
            // observer writes it — from existing before it does. Closing waits
            // for nothing: a menu that cannot be dismissed is a stuck window.
            (
                open.run_if(resource_exists::<Document>.and_then(not(pressed_a_panel))),
                dismiss,
            )
                .chain(),
        );
    }
}

// A left press while the menu is up belongs to the menu: it either activates a
// row or puts the menu away, and either way the canvas under it must not also
// act on it. The rows still fire, because picking reads the mouse events rather
// than this resource.
fn swallow(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    menus: Query<Entity, With<ContextMenu>>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
) {
    if menus.is_empty() {
        return;
    }
    if !buttons.just_pressed(MouseButton::Left) && !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    buttons.clear_just_pressed(MouseButton::Left);
    for entity in &menus {
        commands.entity(entity).insert(Closing);
    }
}

impl Action {
    fn label(self) -> &'static str {
        match self {
            Action::Copy => "copy",
            Action::Duplicate => "duplicate",
            Action::CopyId => "copy node id",
            Action::Ungroup => "remove from group",
            Action::NewSpace => "new space",
        }
    }
}

// Right-click a node for its menu, or the empty board for one that only makes a
// space. Only the left button moves or selects anything, so nothing else reads this.
fn open(
    mut commands: Commands,
    buttons: Res<ButtonInput<MouseButton>>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    nodes: Query<(&NodeId, &Transform, &NodeRect)>,
    existing: Query<Entity, With<ContextMenu>>,
    document: Res<Document>,
) {
    if !buttons.just_pressed(MouseButton::Right) {
        return;
    }
    let Some(at) = window.cursor_position() else {
        return;
    };
    let Some(world) = cursor_world(&window, *camera) else {
        return;
    };
    let node = under(world, &nodes);
    // A second right-click moves the menu rather than stacking another one, and
    // takes one already marked for closing with it.
    for entity in &existing {
        commands.entity(entity).despawn();
    }

    let actions = match &node {
        Some(node) if in_group(&document.0, node) => {
            vec![
                Action::Copy,
                Action::Duplicate,
                Action::CopyId,
                Action::Ungroup,
            ]
        }
        Some(_) => vec![Action::Copy, Action::Duplicate, Action::CopyId],
        None => vec![Action::NewSpace],
    };
    let node = node.unwrap_or_default();
    // Clamped, or a node near the edge opens its menu off the window.
    let height = FRAME_HEIGHT + ROW_HEIGHT * actions.len() as f32;
    let left = at.x.min((window.width() - WIDTH).max(0.0));
    let top = at.y.min((window.height() - height).max(0.0));
    commands.spawn(menu(left, top)).with_children(|parent| {
        for action in actions {
            parent
                .spawn(row(action, &node, world))
                .observe(activate)
                .observe(highlight)
                .observe(unhighlight);
        }
    });
}

// Marked for closing in `PreUpdate` and despawned here, a schedule later: a row's
// own observer runs while picking dispatches the press, so by now it has had its
// click. Clicking an option and clicking away are the same gesture to this.
fn dismiss(mut commands: Commands, closing: Query<Entity, With<Closing>>) {
    for entity in &closing {
        commands.entity(entity).despawn();
    }
}

fn activate(
    press: On<Pointer<Press>>,
    items: Query<&Item>,
    frames: Res<FrameCount>,
    mut document: ResMut<Document>,
    mut clipboard: ResMut<Clipboard>,
    space: Res<crate::client::Space>,
    mut naming: ResMut<crate::spaces::Naming>,
) {
    let Ok(item) = items.get(press.entity) else {
        return;
    };
    match item.action {
        // The id alone: the quotes around it are the script's, and an id is worth
        // having on the clipboard for more than one purpose.
        Action::CopyId => copy(&mut clipboard, item.node.clone()),
        // The node as the file holds it, which is what makes it paste-able into a
        // `.canvas` or another board.
        Action::Copy => {
            let Some(node) = document.0.nodes.iter().find(|node| node.id == item.node) else {
                return;
            };
            match serde_json::to_string_pretty(node) {
                Ok(json) => copy(&mut clipboard, json),
                Err(e) => warn!("copy: {e}"),
            }
        }
        Action::Duplicate => {
            duplicated(&mut document.0, frames.0, &[&item.node]);
        }
        Action::Ungroup => {
            ungrouped(&mut document.0, &item.node);
        }
        Action::NewSpace => {
            if let Some(parent) = space.id() {
                naming.0 = Some(crate::spaces::Draft {
                    parent: parent.to_owned(),
                    world: item.at,
                    screen: press.pointer_location.position,
                });
            }
        }
    }
}

// A clipboard can be absent or held by someone else, and a copy that quietly did
// nothing is worse than one that says so.
fn copy(clipboard: &mut Clipboard, text: String) {
    if let Err(e) = clipboard.set_text(text) {
        warn!("clipboard: {e}");
    }
}

fn highlight(over: On<Pointer<Over>>, mut rows: Query<&mut BackgroundColor, With<Item>>) {
    if let Ok(mut color) = rows.get_mut(over.entity) {
        color.0 = HOVER;
    }
}

fn unhighlight(out: On<Pointer<Out>>, mut rows: Query<&mut BackgroundColor, With<Item>>) {
    if let Ok(mut color) = rows.get_mut(out.entity) {
        color.0 = Color::NONE;
    }
}

fn menu(left: f32, top: f32) -> impl Bundle {
    (
        ContextMenu,
        // Over the node panels, which are UI too and respawn on every change.
        GlobalZIndex(3),
        Node {
            position_type: PositionType::Absolute,
            left: px(left),
            top: px(top),
            width: px(WIDTH),
            padding: UiRect::all(px(PAD)),
            border: UiRect::all(px(1.0)),
            border_radius: BorderRadius::all(px(6.0)),
            flex_direction: FlexDirection::Column,
            ..default()
        },
        BackgroundColor(BG),
        BorderColor::all(EDGE),
    )
}

fn row(action: Action, node: &str, at: Vec2) -> impl Bundle {
    (
        Item {
            action,
            node: node.to_owned(),
            at,
        },
        Node {
            padding: UiRect::axes(px(7.0), px(4.0)),
            border_radius: BorderRadius::all(px(4.0)),
            ..default()
        },
        BackgroundColor(Color::NONE),
        Pickable::default(),
        children![(
            Text::new(action.label()),
            TextFont::from_font_size(ROW),
            TextColor(FG),
        )],
    )
}

fn under(world: Vec2, nodes: &Query<(&NodeId, &Transform, &NodeRect)>) -> Option<String> {
    pick(
        nodes
            .iter()
            .map(|(id, transform, rect)| (&id.0, bounds(transform, rect), transform.translation.z)),
        world,
    )
    .cloned()
}

#[cfg(test)]
mod tests;
