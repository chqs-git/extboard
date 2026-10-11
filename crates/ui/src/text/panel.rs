use bevy::prelude::*;
use bevy::text::EditableText;
use extboard_core::{
    ACCENT, ACCENT_B, Node as CanvasNode, NodeKind, Vars, interpolate, is_image, sides_inset,
    space_path, style,
};
use std::collections::HashMap;

use crate::camera::world_to_screen;
use crate::client::{Document, asset_path};
use crate::node::{NodeId, NodeRect};
use crate::select::{OUTLINE, Selected};
use crate::shape::{Palette, PolygonMaterial, paint};
use crate::theme::{Face, Theme};

use super::edit_text::{Editing, Editor, LabelBox, editor, label_editor};
use super::{GROUP_SIZE, PADDING, ROW_GAP, Raster, blocks, justify, markdown, spawn_blocks, wrap};

const OUTLINE_PX: f32 = 2.0;

// A filling image is laid out in the content box and drawn through the panel's
// clip, and the two round to physical pixels independently: the seam between
// them shows the node's own colour, and it flickers as either rounds under a
// pan or a zoom. Overshooting the clip by a pixel on every side means the clip,
// not the image, is what the edge of a node is.
const BLEED_PX: f32 = 1.0;

// UI rather than a mesh: the UI pass runs after the whole 2D world, so nothing
// in the world can occlude a panel.
#[derive(Component)]
pub(super) struct Panel(String);

// The scaled child. Two entities because bevy clips to the *laid-out* box and
// never to the transformed one: the panel takes the zoom in layout, this takes
// it in scale.
#[derive(Component)]
pub(super) struct Content(String);

// Content drawn past the edge of its box, to be cut off by the panel's clip.
#[derive(Component)]
pub(super) struct Bleed;

#[allow(
    clippy::too_many_arguments,
    reason = "a system's arguments are its query"
)]
pub(super) fn spawn_panels(
    mut commands: Commands,
    document: Res<Document>,
    editing: Res<Editing>,
    theme: Res<Theme>,
    raster: Res<Raster>,
    assets: Res<AssetServer>,
    mut palette: ResMut<Palette>,
    mut materials: ResMut<Assets<PolygonMaterial>>,
    existing: Query<Entity, With<Panel>>,
    live: Query<&EditableText, (With<Editor>, Without<LabelBox>)>,
) {
    // A panel is rebuilt from the document, and the open editor's buffer is the
    // one thing in one the document does not hold yet.
    let carried = live.single().ok().cloned();
    for entity in &existing {
        commands.entity(entity).despawn();
    }

    let raster = raster.0;
    // Array order is already back to front, so the index is the rank.
    for (rank, node) in document.0.nodes.iter().enumerate() {
        let size = Vec2::new(node.width as f32, node.height as f32);
        // The content is laid out `raster` times too big and scaled back down by
        // the same factor, so its glyphs are rasterized at that resolution.
        let box_size = size * raster;
        commands
            .spawn((
                Panel(node.id.clone()),
                Node {
                    position_type: PositionType::Absolute,
                    width: px(size.x),
                    height: px(size.y),
                    // A group's label hangs above its box, outside the clip.
                    overflow: if is_group(node) {
                        Overflow::visible()
                    } else {
                        Overflow::clip()
                    },
                    ..default()
                },
                MaterialNode(paint(&mut palette, &mut materials, &theme, node)),
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
                            width: px(box_size.x),
                            height: px(box_size.y),
                            padding: content_padding(Some(node), size, raster),
                            flex_direction: FlexDirection::Column,
                            row_gap: px(ROW_GAP * raster),
                            ..default()
                        },
                    ))
                    .with_children(|parent| {
                        inside(
                            node,
                            &editing,
                            &theme,
                            &assets,
                            raster,
                            document.0.vars(),
                            carried.as_ref(),
                            parent,
                        )
                    });
            });
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "a system's arguments are its query"
)]
fn inside(
    node: &CanvasNode,
    editing: &Editing,
    theme: &Theme,
    assets: &AssetServer,
    raster: f32,
    vars: &Vars,
    carried: Option<&EditableText>,
    parent: &mut ChildSpawnerCommands,
) {
    let open = editing.node() == Some(node.id.as_str());
    // The text the node names and the colour it paints it, which is what its
    // editor draws in too.
    let face = theme.face(
        style::font_role(&node.extra),
        style::text_color(&node.extra),
    );
    match (&node.kind, markdown(node)) {
        (_, Some(md)) if open => {
            parent.spawn(editor(md, &face, raster, carried));
        }
        (_, Some(md)) => spawn_blocks(
            &node.id,
            &blocks(md, vars),
            theme,
            &face,
            &theme.code_face(),
            raster,
            justify(&node.extra),
            parent,
        ),
        // The file is a path in the spaces dir, which is the asset root.
        (NodeKind::File { file, .. }, _) if is_image(file) => {
            parent.spawn((
                Bleed,
                ImageNode {
                    image: assets.load(asset_path(file)),
                    // The node box is the frame; resize it to crop.
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
                // Sized by its insets alone, which `track_panels` sets to a
                // pixel outside the box on every side.
                Node {
                    position_type: PositionType::Absolute,
                    ..default()
                },
            ));
        }
        (NodeKind::Group { label: Some(label) }, _) => {
            // Sat on the box's top edge, over any panel, under every overlay.
            parent.spawn((
                heading(&interpolate(label, vars), &face, face.ink, raster),
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0.0),
                    bottom: percent(100.0),
                    ..default()
                },
                GlobalZIndex(0),
                ZIndex(i32::MAX),
            ));
        }
        (NodeKind::Link { url }, _) => {
            if let Some(space) = space_path(url) {
                parent.spawn(heading(space, &face, theme.slot(ACCENT_B, ACCENT), raster));
            }
        }
        _ => {}
    }
}

fn heading(text: &str, face: &Face, ink: Color, raster: f32) -> impl Bundle {
    (
        Text::new(text.to_owned()),
        TextFont {
            font: face.source.clone(),
            font_smoothing: face.smoothing,
            ..TextFont::from_font_size(GROUP_SIZE * raster)
        },
        TextColor(ink),
        wrap(bevy::text::Justify::Left),
    )
}

fn is_group(node: &CanvasNode) -> bool {
    matches!(node.kind, NodeKind::Group { .. })
}

// Whether the node's content is drawn edge to edge rather than inside a margin.
fn fills(node: &CanvasNode) -> bool {
    matches!(&node.kind, NodeKind::File { file, .. } if is_image(file))
}

// An image is the node's whole face, so it gets no padding to sit inside.
fn content_padding(node: Option<&CanvasNode>, size: Vec2, raster: f32) -> UiRect {
    if node.is_some_and(fills) {
        return UiRect::ZERO;
    }
    let inset = sides_inset(node.and_then(|node| node.sides));
    UiRect::axes(
        px((PADDING + inset * size.x) * raster),
        px((PADDING + inset * size.y) * raster),
    )
}

// Outside any panel: a screen-space box over the edge's midpoint, not a node's
// body. It despawns here too, so closing a session takes the editor off screen.
pub(super) fn spawn_label_editor(
    mut commands: Commands,
    document: Res<Document>,
    theme: Res<Theme>,
    editing: Res<Editing>,
    camera: Single<&Projection, With<Camera2d>>,
    existing: Query<Entity, With<LabelBox>>,
) {
    // One box per session: this runs again on every change a panel reacts to.
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    let Some(id) = editing.edge() else {
        return;
    };
    let Projection::Orthographic(ortho) = *camera else {
        return;
    };
    let edge = document.0.edges.iter().find(|edge| edge.id == id);
    let face = theme.face(None, edge.and_then(|edge| style::text_color(&edge.extra)));
    let label = edge.and_then(|edge| edge.label.as_deref());
    commands.spawn(label_editor(
        label.unwrap_or_default(),
        &face,
        1.0 / ortho.scale,
    ));
}

#[allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    reason = "a system's arguments are its query"
)]
pub(super) fn track_panels(
    camera: Single<(&Camera, &GlobalTransform, &Projection), With<Camera2d>>,
    document: Res<Document>,
    nodes: Query<(&NodeId, &Transform, &NodeRect)>,
    theme: Res<Theme>,
    raster: Res<Raster>,
    mut palette: ResMut<Palette>,
    mut materials: ResMut<Assets<PolygonMaterial>>,
    mut panels: Query<(&Panel, &mut Node, &mut MaterialNode<PolygonMaterial>)>,
    mut contents: Query<(&Content, &mut UiTransform, &mut Node), Without<Panel>>,
    mut bleeding: Query<&mut Node, (With<Bleed>, Without<Content>, Without<Panel>)>,
) {
    let (camera, cam_global, projection) = *camera;
    let Projection::Orthographic(ortho) = projection else {
        return;
    };
    let zoom = 1.0 / ortho.scale;
    let current: HashMap<&str, &CanvasNode> = document
        .0
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect();
    let placed: HashMap<&str, (Vec2, Vec2)> = nodes
        .iter()
        .map(|(id, transform, rect)| {
            (
                id.0.as_str(),
                (transform.translation.truncate(), rect.size()),
            )
        })
        .collect();

    for (panel, mut node, mut material) in &mut panels {
        let Some(&(center, size)) = placed.get(panel.0.as_str()) else {
            continue;
        };
        // Here rather than at spawn: the slider and a resize drag both change
        // what a panel draws without waking the document.
        if let Some(&node) = current.get(panel.0.as_str()) {
            let want = paint(&mut palette, &mut materials, &theme, node);
            if material.0 != want {
                material.0 = want;
            }
        }
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

    let bleed = px(-bleed(raster.0, zoom));
    for mut node in &mut bleeding {
        if node.left != bleed {
            (node.left, node.top, node.right, node.bottom) = (bleed, bleed, bleed, bleed);
        }
    }

    for (content, mut transform, mut node) in &mut contents {
        let Some(&(_, size)) = placed.get(content.0.as_str()) else {
            continue;
        };
        let want = content_padding(current.get(content.0.as_str()).copied(), size, raster.0);
        if node.padding != want {
            node.padding = want;
        }
        let box_size = size * raster.0;
        if (node.width, node.height) != (px(box_size.x), px(box_size.y)) {
            (node.width, node.height) = (px(box_size.x), px(box_size.y));
        }
        // The box is already `raster` times too big, so it needs that much less
        // scale to reach the camera's zoom.
        let scale = zoom / raster.0;
        let offset = centre_scale_offset(size * raster.0, scale);
        let want = UiTransform {
            scale: Vec2::splat(scale),
            translation: Val2::px(offset.x, offset.y),
            ..UiTransform::IDENTITY
        };
        if *transform != want {
            *transform = want;
        }
    }
}

// In content pixels, which the panel's own scale shrinks on the way to the
// screen: the overshoot is a pixel there however far out the board is.
pub(super) fn bleed(raster: f32, zoom: f32) -> f32 {
    BLEED_PX * raster / zoom
}

// A gizmo would be drawn in the world, under the panel it means to ring.
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
