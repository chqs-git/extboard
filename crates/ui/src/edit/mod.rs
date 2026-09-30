use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use bevy::window::{CursorIcon, SystemCursorIcon};
use extboard_core::{Canvas, Node as CanvasNode, NodeKind, fresh_id};

use crate::client::Document;
use crate::node::{NodeId, NodeRect, to_canvas};
use crate::select::{Selected, bounds, cursor_world, pick};

pub const MIN_SIZE: f32 = 40.0;
const NEW_SIZE: Vec2 = Vec2::new(120.0, 120.0);
// Two presses inside both of these are one double-click. Screen pixels, so the
// slop is the hand's, not the zoom's.
const DOUBLE_SECS: f32 = 0.4;
const DOUBLE_PX: f32 = 6.0;
// Screen pixels, so the grip is the same target at every zoom.
const GRIP_PX: f32 = 12.0;

pub struct EditPlugin;

#[derive(Resource)]
enum Drag {
    Move {
        grab: Vec2,
        start: Vec<(Entity, Vec2)>,
    },
    Resize {
        entity: Entity,
        handle: IVec2,
        start: Rect,
    },
}

impl Plugin for EditPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                grab,
                apply.run_if(resource_exists::<Document>),
                release,
                (create, delete).run_if(resource_exists::<Document>),
                cursor,
            )
                .chain(),
        );
    }
}

fn grab(
    mut commands: Commands,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform, &Projection), With<Camera2d>>,
    nodes: Query<(Entity, &Transform, &NodeRect)>,
    selected: Query<Entity, With<Selected>>,
) {
    let (camera, cam_global, projection) = *camera;
    if !buttons.just_pressed(MouseButton::Left) || keys.pressed(KeyCode::Space) {
        return;
    }
    let Some(world) = cursor_world(&window, (camera, cam_global)) else {
        return;
    };
    let Projection::Orthographic(ortho) = projection else {
        return;
    };

    if let Ok(entity) = selected.single()
        && let Ok((_, transform, rect)) = nodes.get(entity)
    {
        let start = bounds(transform, rect);
        if let Some(handle) = handle_at(start, world, GRIP_PX * ortho.scale) {
            commands.insert_resource(Drag::Resize {
                entity,
                handle,
                start,
            });
            return;
        }
    }

    let Some(picked) = pick(
        nodes.iter().map(|(entity, transform, rect)| {
            (entity, bounds(transform, rect), transform.translation.z)
        }),
        world,
    ) else {
        return;
    };

    // Mirrors select.rs: a press on an unselected node takes the selection with
    // it, so the drag set is that node alone.
    let moving: Vec<Entity> = if selected.contains(picked) {
        selected.iter().collect()
    } else {
        vec![picked]
    };
    let start = moving
        .into_iter()
        .filter_map(|entity| {
            let (_, transform, _) = nodes.get(entity).ok()?;
            Some((entity, transform.translation.truncate()))
        })
        .collect();
    commands.insert_resource(Drag::Move { grab: world, start });
}

fn apply(
    drag: Option<Res<Drag>>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    nodes: Query<&NodeId>,
    mut document: ResMut<Document>,
) {
    let Some(drag) = drag else {
        return;
    };
    let Some(world) = cursor_world(&window, *camera) else {
        return;
    };

    // The entities are already where the document says; waking change detection
    // would respawn every node and panel mid-gesture, and lose the selection.
    let canvas = &mut document.bypass_change_detection().0;

    match &*drag {
        Drag::Move { grab, start } => {
            for &(entity, from) in start {
                if let Ok(id) = nodes.get(entity) {
                    moved(canvas, &id.0, from + world - *grab);
                }
            }
        }
        Drag::Resize {
            entity,
            handle,
            start,
        } => {
            if let Ok(id) = nodes.get(*entity) {
                sized(canvas, &id.0, resized(*start, *handle, world, MIN_SIZE));
            }
        }
    }
}

fn node_mut<'a>(canvas: &'a mut Canvas, id: &str) -> Option<&'a mut CanvasNode> {
    canvas.nodes.iter_mut().find(|node| node.id == id)
}

fn moved(canvas: &mut Canvas, id: &str, center: Vec2) {
    let Some(node) = node_mut(canvas, id) else {
        return;
    };
    let size = Vec2::new(node.width as f32, node.height as f32);
    let top_left = to_canvas(center, size);
    (node.x, node.y) = (top_left.x.round() as i64, top_left.y.round() as i64);
}

fn sized(canvas: &mut Canvas, id: &str, rect: Rect) {
    let Some(node) = node_mut(canvas, id) else {
        return;
    };
    let top_left = to_canvas(rect.center(), rect.size());
    (node.x, node.y) = (top_left.x.round() as i64, top_left.y.round() as i64);
    (node.width, node.height) = (rect.width().round() as i64, rect.height().round() as i64);
}

#[allow(
    clippy::too_many_arguments,
    reason = "a system's arguments are its query"
)]
fn create(
    time: Res<Time>,
    frames: Res<FrameCount>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    nodes: Query<(Entity, &Transform, &NodeRect)>,
    mut document: ResMut<Document>,
    mut last: Local<Option<(f32, Vec2)>>,
) {
    if !buttons.just_pressed(MouseButton::Left) || keys.pressed(KeyCode::Space) {
        return;
    }
    let Some(screen) = window.cursor_position() else {
        return;
    };
    if !double_click(&mut last, time.elapsed_secs(), screen) {
        return;
    }
    let Some(world) = cursor_world(&window, *camera) else {
        return;
    };
    // Empty canvas only: a double-click on a node is E5-T5's way into edit mode.
    let occupied = pick(
        nodes.iter().map(|(entity, transform, rect)| {
            (entity, bounds(transform, rect), transform.translation.z)
        }),
        world,
    );
    if occupied.is_some() {
        return;
    }
    created(
        &mut document.0,
        frames.0,
        Rect::from_center_size(world, NEW_SIZE),
    );
}

fn delete(
    keys: Res<ButtonInput<KeyCode>>,
    selected: Query<&NodeId, With<Selected>>,
    mut document: ResMut<Document>,
) {
    // Mac's delete key is Backspace.
    if selected.is_empty() || !keys.any_just_pressed([KeyCode::Delete, KeyCode::Backspace]) {
        return;
    }
    let canvas = &mut document.0;
    for id in &selected {
        // Cascades the node's edges; a stale selection entity is not an error.
        let _ = canvas.remove_node(&id.0);
    }
}

fn double_click(last: &mut Option<(f32, Vec2)>, now: f32, at: Vec2) -> bool {
    let again = last
        .is_some_and(|(then, there)| now - then <= DOUBLE_SECS && there.distance(at) <= DOUBLE_PX);
    // The pair is spent, so a third press opens a new one instead of firing again.
    *last = (!again).then_some((now, at));
    again
}

pub fn created(canvas: &mut Canvas, seed: u32, rect: Rect) -> String {
    let top_left = to_canvas(rect.center(), rect.size());
    let id = fresh_id(canvas, &seed.to_le_bytes());
    canvas
        .add_node(CanvasNode {
            id: id.clone(),
            x: top_left.x.round() as i64,
            y: top_left.y.round() as i64,
            width: rect.width().round() as i64,
            height: rect.height().round() as i64,
            color: None,
            kind: NodeKind::Text {
                text: String::new(),
            },
            extra: default(),
        })
        .expect("fresh_id never collides");
    id
}

fn release(
    mut commands: Commands,
    buttons: Res<ButtonInput<MouseButton>>,
    drag: Option<Res<Drag>>,
) {
    if drag.is_some() && !buttons.pressed(MouseButton::Left) {
        commands.remove_resource::<Drag>();
    }
}

fn cursor(
    mut commands: Commands,
    drag: Option<Res<Drag>>,
    window: Single<(Entity, &Window, Option<&CursorIcon>)>,
    camera: Single<(&Camera, &GlobalTransform, &Projection), With<Camera2d>>,
    nodes: Query<(&Transform, &NodeRect)>,
    selected: Query<Entity, With<Selected>>,
) {
    let (entity, window, current) = *window;
    let (camera, cam_global, projection) = *camera;

    let want = match drag.as_deref() {
        Some(Drag::Resize { handle, .. }) => resize_cursor(*handle),
        Some(Drag::Move { .. }) => SystemCursorIcon::Grabbing,
        None => hovered_handle(window, (camera, cam_global), projection, &nodes, &selected)
            .map_or(SystemCursorIcon::Default, resize_cursor),
    };

    let want = CursorIcon::System(want);
    if current != Some(&want) {
        commands.entity(entity).insert(want);
    }
}

fn hovered_handle(
    window: &Window,
    camera: (&Camera, &GlobalTransform),
    projection: &Projection,
    nodes: &Query<(&Transform, &NodeRect)>,
    selected: &Query<Entity, With<Selected>>,
) -> Option<IVec2> {
    let Projection::Orthographic(ortho) = projection else {
        return None;
    };
    let (transform, rect) = nodes.get(selected.single().ok()?).ok()?;
    handle_at(
        bounds(transform, rect),
        cursor_world(window, camera)?,
        GRIP_PX * ortho.scale,
    )
}

fn resize_cursor(handle: IVec2) -> SystemCursorIcon {
    match (handle.x, handle.y) {
        (0, _) => SystemCursorIcon::NsResize,
        (_, 0) => SystemCursorIcon::EwResize,
        (x, y) if x == y => SystemCursorIcon::NeswResize,
        _ => SystemCursorIcon::NwseResize,
    }
}

fn handle_at(rect: Rect, point: Vec2, band: f32) -> Option<IVec2> {
    if !rect.contains(point) {
        return None;
    }
    let band = band.min(rect.size().min_element() / 3.0);
    let edge = |v: f32, min: f32, max: f32| match v {
        _ if v - min <= band => -1,
        _ if max - v <= band => 1,
        _ => 0,
    };
    let handle = IVec2::new(
        edge(point.x, rect.min.x, rect.max.x),
        edge(point.y, rect.min.y, rect.max.y),
    );
    (handle != IVec2::ZERO).then_some(handle)
}

fn resized(start: Rect, handle: IVec2, point: Vec2, min: f32) -> Rect {
    let mut rect = start;
    if handle.x < 0 {
        rect.min.x = point.x.min(start.max.x - min);
    } else if handle.x > 0 {
        rect.max.x = point.x.max(start.min.x + min);
    }
    if handle.y < 0 {
        rect.min.y = point.y.min(start.max.y - min);
    } else if handle.y > 0 {
        rect.max.y = point.y.max(start.min.y + min);
    }
    rect
}

#[cfg(test)]
mod tests;
