use bevy::prelude::*;
use extboard_core::{Node as CanvasNode, NodeKind};
use std::collections::HashMap;

use crate::camera::world_to_screen;
use crate::client::Document;
use crate::node::{NodeId, NodeRect, depth, node_color};
use crate::select::{OUTLINE, Selected};

use super::edit_text::{Editing, editor, label_editor};
use super::{GROUP_LABEL, GROUP_SIZE, PADDING, ROW_GAP, blocks, markdown, spawn_blocks, wrap};

const OUTLINE_PX: f32 = 2.0;
// A group is a container: what sits inside it, edges included, shows through.
const GROUP_ALPHA: f32 = 0.45;

// The node's body: its fill, its clip box and whatever is drawn inside it. UI
// rather than a mesh, because the text is UI and the UI pass runs after the
// whole 2D world, so nothing in the world can ever occlude a panel.
#[derive(Component)]
pub(super) struct Panel(String);

// The scaled child. Two entities because bevy clips to the *laid-out* box and
// never to the transformed one: the panel takes the zoom in layout, this takes
// it in scale.
#[derive(Component)]
pub(super) struct Content(String);

pub(super) fn spawn_panels(
    mut commands: Commands,
    document: Res<Document>,
    editing: Res<Editing>,
    existing: Query<Entity, With<Panel>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }

    for (rank, node) in back_to_front(&document.0.nodes).iter().enumerate() {
        let size = Vec2::new(node.width as f32, node.height as f32);
        commands
            .spawn((
                Panel(node.id.clone()),
                Node {
                    position_type: PositionType::Absolute,
                    width: px(size.x),
                    height: px(size.y),
                    overflow: Overflow::clip(),
                    ..default()
                },
                BackgroundColor(fill(node)),
                // Hidden rather than absent: toggling the component would move
                // the panel between tables on every selection change.
                Outline::new(px(OUTLINE_PX), px(0.0), Color::NONE),
                ZIndex(rank as i32),
            ))
            .with_children(|parent| {
                parent
                    .spawn((
                        Content(node.id.clone()),
                        // Absolute: a flex child would be shrunk to fit below 100%.
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(0.0),
                            top: px(0.0),
                            width: px(size.x),
                            height: px(size.y),
                            padding: UiRect::all(px(PADDING)),
                            flex_direction: FlexDirection::Column,
                            row_gap: px(ROW_GAP),
                            ..default()
                        },
                    ))
                    .with_children(|parent| inside(node, &editing, parent));
            });
    }
}

// A node inside a group has to draw over it, and two overlapping nodes have to
// pick a winner: the same area-based depth the node entities carry, as a `ZIndex`
// rank. Spawn order would do it too, but not visibly.
pub(super) fn back_to_front(nodes: &[CanvasNode]) -> Vec<&CanvasNode> {
    let mut order: Vec<&CanvasNode> = nodes.iter().collect();
    order.sort_by(|a, b| depth(a).total_cmp(&depth(b)));
    order
}

// Source while editing, rendered at rest, and a group is only ever its name.
fn inside(node: &CanvasNode, editing: &Editing, parent: &mut ChildSpawnerCommands) {
    let open = editing.node() == Some(node.id.as_str());
    match (&node.kind, markdown(node)) {
        (_, Some(md)) if open => {
            parent.spawn(editor(md));
        }
        (_, Some(md)) => spawn_blocks(&blocks(md), parent),
        (NodeKind::Group { label: Some(label) }, _) => {
            parent.spawn((
                Text::new(label.clone()),
                TextFont::from_font_size(GROUP_SIZE),
                TextColor(GROUP_LABEL),
                wrap(),
            ));
        }
        _ => {}
    }
}

// A group's fill is translucent so its contents read as inside it; everything
// else is opaque, which is the whole point of drawing the body here.
pub(super) fn fill(node: &CanvasNode) -> Color {
    let color = node_color(node);
    match node.kind {
        NodeKind::Group { .. } => color.with_alpha(GROUP_ALPHA),
        _ => color,
    }
}

// The label editor is spawned outside any panel: it is a fixed screen-sized box
// on an edge, not part of a node's body.
pub(super) fn spawn_label_editor(
    mut commands: Commands,
    document: Res<Document>,
    editing: Res<Editing>,
) {
    if let Some(id) = editing.edge() {
        let label = document
            .0
            .edges
            .iter()
            .find(|edge| edge.id == id)
            .and_then(|edge| edge.label.as_deref());
        commands.spawn(label_editor(label.unwrap_or_default()));
    }
}

pub(super) fn track_panels(
    camera: Single<(&Camera, &GlobalTransform, &Projection), With<Camera2d>>,
    nodes: Query<(&NodeId, &Transform, &NodeRect)>,
    mut panels: Query<(&Panel, &mut Node)>,
    mut contents: Query<(&Content, &mut UiTransform)>,
) {
    let (camera, cam_global, projection) = *camera;
    let Projection::Orthographic(ortho) = projection else {
        return;
    };
    let zoom = 1.0 / ortho.scale;
    let placed: HashMap<&str, (Vec2, Vec2)> = nodes
        .iter()
        .map(|(id, transform, rect)| {
            (
                id.0.as_str(),
                (transform.translation.truncate(), rect.size()),
            )
        })
        .collect();

    for (panel, mut node) in &mut panels {
        let Some(&(center, size)) = placed.get(panel.0.as_str()) else {
            continue;
        };
        let Some(screen) = world_to_screen(camera, cam_global, center) else {
            continue;
        };
        let scaled = size * zoom;
        let want = (
            px(screen.x - scaled.x / 2.0),
            px(screen.y - scaled.y / 2.0),
            px(scaled.x),
            px(scaled.y),
        );
        // Any write re-runs layout, so write only what moved.
        if (node.left, node.top, node.width, node.height) != want {
            (node.left, node.top, node.width, node.height) = want;
        }
    }

    for (content, mut transform) in &mut contents {
        let Some(&(_, size)) = placed.get(content.0.as_str()) else {
            continue;
        };
        let offset = centre_scale_offset(size, zoom);
        let want = UiTransform {
            scale: Vec2::splat(zoom),
            translation: Val2::px(offset.x, offset.y),
            ..UiTransform::IDENTITY
        };
        if *transform != want {
            *transform = want;
        }
    }
}

// The selection ring. A gizmo would be drawn in the world, which is to say under
// every panel, including the one it is meant to be ringing.
pub(super) fn outline_panels(
    selected: Query<&NodeId, With<Selected>>,
    mut panels: Query<(&Panel, &mut Outline)>,
) {
    for (panel, mut outline) in &mut panels {
        let want = if selected.iter().any(|id| id.0 == panel.0) {
            OUTLINE
        } else {
            Color::NONE
        };
        if outline.color != want {
            outline.color = want;
        }
    }
}

// A scale is about the centre, so put the grown box back on the panel's corner
pub(super) fn centre_scale_offset(size: Vec2, zoom: f32) -> Vec2 {
    size * (zoom - 1.0) / 2.0
}
