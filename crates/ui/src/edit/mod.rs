use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use bevy::window::{CursorIcon, SystemCursorIcon};
use extboard_core::{Canvas, Edge as CanvasEdge, Node as CanvasNode, NodeKind, Side, fresh_id};

use crate::client::Document;
use crate::node::{NodeId, NodeRect, to_canvas};
use crate::scene::{EdgeId, draw_arrow, segments};
use crate::select::{Selected, bounds, cursor_world, pick};

pub const MIN_SIZE: f32 = 40.0;
const NEW_SIZE: Vec2 = Vec2::new(120.0, 120.0);
// Two presses inside both of these are one double-click. Screen pixels, so the
// slop is the hand's, not the zoom's.
const DOUBLE_SECS: f32 = 0.4;
const DOUBLE_PX: f32 = 6.0;
// Screen pixels, so the grip is the same target at every zoom.
const GRIP_PX: f32 = 12.0;
// The grip on the end of an edge: what is aimed at is the arrowhead, which is
// wider than the anchor circle sitting under its tip.
pub const TIP_PX: f32 = 16.0;
// The side anchor an edge starts from, in screen pixels so it is the same
// target at every zoom.
const ANCHOR_PX: f32 = 5.0;
const ANCHOR: Color = Color::WHITE;
// Screen pixels of reach around an anchor, by what is being asked of it.
// Showing the circle is a hint and can be generous; landing a drop wider still;
// but starting a drag has to mean the circle itself, or everything else near a
// node's side, the tip of a selected edge included, is unreachable.
const REACH_GRIP: f32 = ANCHOR_PX + 3.0;
const REACH_HOVER: f32 = 56.0;
const REACH_DRAG: f32 = 224.0;

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
    Edge {
        from: Entity,
        side: Side,
    },
    // Which end of which edge is being dragged onto a new target.
    Redirect {
        edge: String,
        tip: Tip,
    },
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tip {
    From,
    To,
}

impl Plugin for EditPlugin {
    fn build(&self, app: &mut App) {
        // An edit session owns the keyboard and the pointer: backspace deletes a
        // character, not the node. Only `release` and `cursor` stay, so a drag
        // that was under way when it began still gets cleaned up.
        app.add_systems(
            Update,
            (
                (
                    grab,
                    apply.run_if(resource_exists::<Document>),
                    anchors,
                    connect.run_if(resource_exists::<Document>),
                )
                    .chain()
                    .run_if(not(crate::text::editing)),
                release,
                (create, delete)
                    .run_if(resource_exists::<Document>.and_then(not(crate::text::editing))),
                cursor,
            )
                .chain(),
        );
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "a system's arguments are its query"
)]
fn grab(
    mut commands: Commands,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform, &Projection), With<Camera2d>>,
    nodes: Query<(Entity, &Transform, &NodeRect)>,
    document: Option<Res<Document>>,
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

    // An edge ends on the very anchor that starts a new one, so the circle keeps
    // the pixel it is drawn on and the arrowhead around it takes the rest: one
    // gesture per visible target, neither of them needing a selection first.
    if let Some((from, side, _)) = anchor_under(rects(&nodes), projection, world, Reach::Grip) {
        commands.insert_resource(Drag::Edge { from, side });
        return;
    }

    // Before the resize band, which claims the same boundary.
    if let Some(document) = document.as_deref()
        && let Some((edge, tip)) = tip_under(&document.0, world, TIP_PX * ortho.scale)
    {
        commands.insert_resource(Drag::Redirect { edge, tip });
        return;
    }

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
        // Nothing to write until the drop lands on a node.
        Drag::Edge { .. } | Drag::Redirect { .. } => {}
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
    mut editing: ResMut<crate::text::Editing>,
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
    let id = created(
        &mut document.0,
        frames.0,
        Rect::from_center_size(world, NEW_SIZE),
    );
    // A node made by double-clicking opens for typing, so the gesture is one
    // move: double-click, type.
    editing.0 = Some(crate::text::Target::Node(id));
}

fn delete(
    keys: Res<ButtonInput<KeyCode>>,
    selected: Query<&NodeId, With<Selected>>,
    edges: Query<&EdgeId, With<Selected>>,
    mut document: ResMut<Document>,
) {
    // Mac's delete key is Backspace.
    if (selected.is_empty() && edges.is_empty())
        || !keys.any_just_pressed([KeyCode::Delete, KeyCode::Backspace])
    {
        return;
    }
    let canvas = &mut document.0;
    for id in &selected {
        // Cascades the node's edges; a stale selection entity is not an error.
        let _ = canvas.remove_node(&id.0);
    }
    // After the nodes: a cascade may already have taken this edge with it.
    for id in &edges {
        let _ = canvas.remove_edge(&id.0);
    }
}

pub fn double_click(last: &mut Option<(f32, Vec2)>, now: f32, at: Vec2) -> bool {
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

// The one anchor within reach shows where an edge can start; a live drag shows
// where it would land.
fn anchors(
    mut gizmos: Gizmos,
    drag: Option<Res<Drag>>,
    document: Option<Res<Document>>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform, &Projection), With<Camera2d>>,
    nodes: Query<(Entity, &Transform, &NodeRect)>,
) {
    let (camera, cam_global, projection) = *camera;
    let (Some(world), Projection::Orthographic(ortho)) =
        (cursor_world(&window, (camera, cam_global)), projection)
    else {
        return;
    };
    // A move or resize is already under way; anchors would only be clutter.
    let drawing = matches!(
        drag.as_deref(),
        Some(Drag::Edge { .. } | Drag::Redirect { .. })
    );
    if drag.is_some() && !drawing {
        return;
    }
    // The preview carries its arrowhead: a drag should look like the edge it is
    // about to become, pointing wherever the head will end up.
    match drag.as_deref() {
        Some(Drag::Edge { from, side }) => {
            if let Ok((_, transform, rect)) = nodes.get(*from) {
                let start = anchor(bounds(transform, rect), *side);
                draw_arrow(&mut gizmos, start, world, (false, true), None, ANCHOR);
            }
        }
        // The end staying put is the one the line is drawn from, and dragging the
        // tail leaves the head on it.
        Some(Drag::Redirect { edge, tip }) => {
            if let Some(document) = document.as_deref()
                && let Some((_, a, b)) = segments(&document.0).find(|(id, _, _)| *id == edge)
            {
                let (start, end) = if *tip == Tip::From {
                    (world, b)
                } else {
                    (a, world)
                };
                draw_arrow(&mut gizmos, start, end, (false, true), None, ANCHOR);
            }
        }
        _ => {}
    }

    let reach = if drawing { Reach::Drag } else { Reach::Hover };
    if let Some((_, _, at)) = anchor_under(rects(&nodes), projection, world, reach) {
        gizmos.circle_2d(at, ANCHOR_PX * ortho.scale, ANCHOR);
    }
}

pub fn rects<'a>(
    nodes: &'a Query<(Entity, &Transform, &NodeRect)>,
) -> impl Iterator<Item = (Entity, Rect)> + 'a {
    nodes
        .iter()
        .map(|(entity, transform, rect)| (entity, bounds(transform, rect)))
}

#[derive(Clone, Copy)]
pub enum Reach {
    Grip,
    Hover,
    Drag,
}

impl Reach {
    fn px(self) -> f32 {
        match self {
            Reach::Grip => REACH_GRIP,
            Reach::Hover => REACH_HOVER,
            Reach::Drag => REACH_DRAG,
        }
    }
}

pub fn anchor_under(
    nodes: impl Iterator<Item = (Entity, Rect)>,
    projection: &Projection,
    point: Vec2,
    reach: Reach,
) -> Option<(Entity, Side, Vec2)> {
    let Projection::Orthographic(ortho) = projection else {
        return None;
    };
    nearest_anchor(nodes, point, reach.px() * ortho.scale)
}

fn nearest_anchor(
    nodes: impl Iterator<Item = (Entity, Rect)>,
    point: Vec2,
    reach: f32,
) -> Option<(Entity, Side, Vec2)> {
    nodes
        .flat_map(|(entity, rect)| SIDES.map(|side| (entity, side, anchor(rect, side))))
        .filter(|(_, _, at)| at.distance(point) <= reach)
        .min_by(|a, b| a.2.distance(point).total_cmp(&b.2.distance(point)))
}

// Landing on a node makes the edge, or moves the end being dragged; anywhere
// else drops the gesture.
#[allow(
    clippy::too_many_arguments,
    reason = "a system's arguments are its query"
)]
fn connect(
    buttons: Res<ButtonInput<MouseButton>>,
    frames: Res<FrameCount>,
    drag: Option<Res<Drag>>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform, &Projection), With<Camera2d>>,
    nodes: Query<(Entity, &Transform, &NodeRect)>,
    ids: Query<&NodeId>,
    mut document: ResMut<Document>,
) {
    let (camera, cam_global, projection) = *camera;
    let Some(drag @ (Drag::Edge { .. } | Drag::Redirect { .. })) = drag.as_deref() else {
        return;
    };
    if buttons.pressed(MouseButton::Left) {
        return;
    }
    let Some(world) = cursor_world(&window, (camera, cam_global)) else {
        return;
    };
    let Some((target, to_side)) = dropped_on(&nodes, projection, world) else {
        return;
    };
    let Ok(end) = ids.get(target) else {
        return;
    };

    match drag {
        Drag::Edge { from, side } => {
            if let Ok(start) = ids.get(*from) {
                connected(
                    &mut document.0,
                    frames.0,
                    (&start.0, *side),
                    (&end.0, to_side),
                );
            }
        }
        Drag::Redirect { edge, tip } => {
            redirected(&mut document.0, edge, *tip, (&end.0, to_side));
        }
        _ => {}
    }
}

// Inside a node, or on the anchor that is showing just off one of its sides.
fn dropped_on(
    nodes: &Query<(Entity, &Transform, &NodeRect)>,
    projection: &Projection,
    world: Vec2,
) -> Option<(Entity, Side)> {
    pick(
        nodes.iter().map(|(entity, transform, rect)| {
            (entity, bounds(transform, rect), transform.translation.z)
        }),
        world,
    )
    .and_then(|entity| {
        let (_, transform, rect) = nodes.get(entity).ok()?;
        Some((entity, nearest_side(bounds(transform, rect), world)))
    })
    .or_else(|| {
        anchor_under(rects(nodes), projection, world, Reach::Drag)
            .map(|(entity, side, _)| (entity, side))
    })
}

pub fn tip_under(canvas: &Canvas, point: Vec2, reach: f32) -> Option<(String, Tip)> {
    segments(canvas)
        .flat_map(|(id, a, b)| [(id, Tip::From, a), (id, Tip::To, b)])
        .map(|(id, tip, at)| (id, tip, at.distance(point)))
        .filter(|(_, _, distance)| *distance <= reach)
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(id, tip, _)| (id.to_owned(), tip))
}

// `false` when the move was refused: an edge with both ends on one node is not
// a link, same as one drawn that way.
fn redirected(canvas: &mut Canvas, id: &str, tip: Tip, to: (&str, Side)) -> bool {
    let Some(edge) = canvas.edges.iter_mut().find(|edge| edge.id == id) else {
        return false;
    };
    let other = if tip == Tip::From {
        &edge.to_node
    } else {
        &edge.from_node
    };
    if other == to.0 {
        return false;
    }
    match tip {
        Tip::From => (edge.from_node, edge.from_side) = (to.0.to_owned(), Some(to.1)),
        Tip::To => (edge.to_node, edge.to_side) = (to.0.to_owned(), Some(to.1)),
    }
    true
}

const SIDES: [Side; 4] = [Side::Top, Side::Bottom, Side::Left, Side::Right];

// World space, so canvas Top is the rect's high y.
fn anchor(rect: Rect, side: Side) -> Vec2 {
    let center = rect.center();
    match side {
        Side::Top => Vec2::new(center.x, rect.max.y),
        Side::Bottom => Vec2::new(center.x, rect.min.y),
        Side::Left => Vec2::new(rect.min.x, center.y),
        Side::Right => Vec2::new(rect.max.x, center.y),
    }
}

// The side the drop is nearest, measured straight out to each edge rather than
// to its midpoint: a drop by a corner belongs to the edge it is closest to.
fn nearest_side(rect: Rect, point: Vec2) -> Side {
    [
        (Side::Left, point.x - rect.min.x),
        (Side::Right, rect.max.x - point.x),
        (Side::Bottom, point.y - rect.min.y),
        (Side::Top, rect.max.y - point.y),
    ]
    .into_iter()
    .min_by(|a, b| a.1.total_cmp(&b.1))
    .map(|(side, _)| side)
    .expect("four sides")
}

// `false` when the edge was refused: a node joined to itself is not a link.
fn connected(canvas: &mut Canvas, seed: u32, from: (&str, Side), to: (&str, Side)) -> bool {
    if from.0 == to.0 {
        return false;
    }
    let edge = CanvasEdge {
        id: fresh_id(canvas, &seed.to_le_bytes()),
        from_node: from.0.to_owned(),
        from_side: Some(from.1),
        from_end: None,
        to_node: to.0.to_owned(),
        to_side: Some(to.1),
        to_end: None,
        label: None,
        extra: default(),
    };
    canvas.add_edge(edge).is_ok()
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
        Some(Drag::Edge { .. } | Drag::Redirect { .. }) => SystemCursorIcon::Crosshair,
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
