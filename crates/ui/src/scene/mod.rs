use bevy::prelude::*;
use bevy::transform::TransformSystems;
use extboard_core::{End, TEXT, interpolate, stroke_scale, style};
use std::collections::HashMap;

use crate::camera::world_to_screen;
use crate::client::Document;
use crate::select::{OUTLINE, Selected};
use crate::text::Raster;
use crate::theme::Theme;

mod arrow;
mod geometry;
mod line;

pub use arrow::draw_arrow;
use arrow::{HEAD_HALF_WIDTH, LABEL_PAD, box_reach};
pub use geometry::{crosses, edge_depth, nearest, segments};
use geometry::{edge_ends, rank, resolved_edges};
use line::{LineMaterial, LinePaint};

pub const LABEL_SIZE: f32 = 12.0;
// The shaft at the middle stroke weight, in screen pixels.
const SHAFT: f32 = 3.0;
const RIM_PX: f32 = 2.0;

pub struct ScenePlugin;

#[derive(Component)]
pub struct EdgeId(pub String);

#[derive(Component, Default)]
pub struct LabelSize(pub Vec2);

#[derive(Component)]
struct EdgeLine(String);

#[derive(Component)]
struct EdgeLabel(String);

impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        line::register(app);
        app.insert_gizmo_config(
            DefaultGizmoConfigGroup,
            GizmoConfig {
                line: GizmoLineConfig {
                    width: SHAFT,
                    ..default()
                },
                ..default()
            },
        )
        .add_systems(
            Update,
            (
                spawn_edges
                    .in_set(crate::node::Respawn)
                    // Ahead of the node respawn, which spends the `Reselect` both read.
                    .before(crate::node::spawn_nodes)
                    .run_if(
                        resource_exists::<Document>.and_then(
                            resource_changed::<Document>
                                .or_else(resource_changed::<crate::vars::Vars>)
                                .or_else(resource_changed::<Raster>)
                                .or_else(resource_changed::<Theme>),
                        ),
                    ),
                // Every frame: a drag moves nodes without waking change detection.
                place_edges.run_if(resource_exists::<Document>),
            )
                .chain(),
        )
        .add_systems(
            PostUpdate,
            (track_lines, track_labels)
                .run_if(resource_exists::<Document>)
                .after(TransformSystems::Propagate)
                .after(bevy::camera::CameraUpdateSystems)
                .before(bevy::ui::UiSystems::Layout),
        );
    }
}

// A label's box in world space: the text is centred on the edge's midpoint.
// It counts as the edge for a click, because on a steep edge the text sticks out
// far past the few pixels of reach around the shaft.
pub fn label_box(at: &Transform, label: &LabelSize) -> Rect {
    Rect::from_center_size(at.translation.truncate(), label.0)
}

// The spec's colour is the edge's line, and an edge that names none follows the
// board's text: a line is read, not filled.
pub fn edge_color(theme: &Theme, edge: &extboard_core::Edge) -> Color {
    theme.paint(edge.color.as_deref(), TEXT)
}

#[allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    reason = "a system's arguments are its query"
)]
fn spawn_edges(
    mut commands: Commands,
    document: Res<Document>,
    vars: Res<crate::vars::Vars>,
    theme: Res<Theme>,
    raster: Res<Raster>,
    reselect: Option<Res<crate::node::Reselect>>,
    mut materials: ResMut<Assets<LineMaterial>>,
    existing: Query<(Entity, &EdgeId, Has<Selected>)>,
    drawn: Query<Entity, Or<(With<EdgeLine>, With<EdgeLabel>)>>,
) {
    // A reselect names nodes only, so it leaves no edge selected.
    let fresh = reselect.is_some_and(|reselect| reselect.0.is_some());
    let mut selected: Vec<&str> = Vec::new();
    for (entity, id, picked) in &existing {
        if picked && !fresh {
            selected.push(id.0.as_str());
        }
        commands.entity(entity).despawn();
    }
    for entity in &drawn {
        commands.entity(entity).despawn();
    }

    for (edge, from, to) in resolved_edges(&document.0) {
        let (a, b) = edge_ends(from, edge.from_side, to, edge.to_side);
        let Some(rank) = rank(&document.0, edge) else {
            continue;
        };
        let z = ZIndex(crate::node::stack(rank).0 + 1);
        let mut spawned = commands.spawn((
            EdgeId(edge.id.clone()),
            Transform::from_translation(a.midpoint(b).extend(0.0)),
        ));
        if selected.contains(&edge.id.as_str()) {
            spawned.insert(Selected);
        }
        if edge.label.is_some() {
            spawned.insert(LabelSize::default());
        }
        commands.spawn((
            EdgeLine(edge.id.clone()),
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            MaterialNode(materials.add(LineMaterial {
                paint: LinePaint::default(),
            })),
            z,
            Pickable::IGNORE,
        ));
        if let Some(label) = &edge.label {
            let face = theme.face(None, style::text_color(&edge.extra));
            commands.spawn((
                EdgeLabel(edge.id.clone()),
                Text::new(interpolate(label, &vars.0).into_owned()),
                TextFont {
                    font: face.source,
                    font_smoothing: face.smoothing,
                    ..TextFont::from_font_size(LABEL_SIZE * raster.0)
                },
                TextColor(face.ink),
                TextLayout::no_wrap(),
                Node {
                    position_type: PositionType::Absolute,
                    ..default()
                },
                z,
                Pickable::IGNORE,
            ));
        }
    }
}

fn place_edges(document: Res<Document>, mut edges: Query<(&EdgeId, &mut Transform)>) {
    let placed: HashMap<&str, Vec3> = segments(&document.0)
        .map(|(id, a, b)| (id, a.midpoint(b).extend(0.0)))
        .collect();
    for (id, mut transform) in &mut edges {
        let Some(&want) = placed.get(id.0.as_str()) else {
            continue;
        };
        if transform.translation != want {
            transform.translation = want;
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    reason = "a system's arguments are its query"
)]
fn track_lines(
    camera: Single<(&Camera, &GlobalTransform, &Projection), With<Camera2d>>,
    document: Res<Document>,
    theme: Res<Theme>,
    edges: Query<(&EdgeId, Has<Selected>, Option<&LabelSize>)>,
    mut lines: Query<(
        &EdgeLine,
        &mut Node,
        &mut UiTransform,
        &mut Visibility,
        &MaterialNode<LineMaterial>,
    )>,
    mut materials: ResMut<Assets<LineMaterial>>,
) {
    let (camera, cam_global, projection) = *camera;
    let Projection::Orthographic(ortho) = projection else {
        return;
    };
    let zoom = 1.0 / ortho.scale;
    let state: HashMap<&str, (bool, Option<Vec2>)> = edges
        .iter()
        .map(|(id, picked, label)| (id.0.as_str(), (picked, label.map(|label| label.0))))
        .collect();

    for (edge, from, to) in resolved_edges(&document.0) {
        let Some((mut node, mut transform, mut visibility, material)) = lines
            .iter_mut()
            .find(|(line, ..)| line.0 == edge.id)
            .map(|(_, node, transform, visibility, material)| {
                (node, transform, visibility, material)
            })
        else {
            continue;
        };
        let (a, b) = edge_ends(from, edge.from_side, to, edge.to_side);
        let (Some(sa), Some(sb)) = (
            world_to_screen(camera, cam_global, a),
            world_to_screen(camera, cam_global, b),
        ) else {
            continue;
        };
        let length = a.distance(b);
        let want = if length < f32::EPSILON {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        visibility.set_if_neq(want);
        if want == Visibility::Hidden {
            continue;
        }

        let (picked, label) = state.get(edge.id.as_str()).copied().unwrap_or_default();
        let shaft = SHAFT * stroke_scale(&edge.extra);
        let (width, height) = (
            length * zoom,
            (2.0 * HEAD_HALF_WIDTH * zoom).max(shaft) + 2.0 * RIM_PX,
        );
        let middle = sa.midpoint(sb);
        let placed = (
            px(middle.x - width / 2.0),
            px(middle.y - height / 2.0),
            px(width),
            px(height),
        );
        // Any write re-runs layout, so write only what moved.
        if (node.left, node.top, node.width, node.height) != placed {
            (node.left, node.top, node.width, node.height) = placed;
        }
        let turn = UiTransform::from_rotation(Rot2::radians((sb - sa).to_angle()));
        if *transform != turn {
            *transform = turn;
        }

        let color = if picked {
            OUTLINE
        } else {
            edge_color(&theme, edge)
        };
        let paint = LinePaint {
            color: Vec4::from_array(color.to_linear().to_f32_array()),
            length,
            shaft,
            hole: label.map_or(0.0, |size| {
                box_reach(size / 2.0 + Vec2::splat(LABEL_PAD), (b - a) / length)
            }),
            dash: style::dash(&edge.extra).unwrap_or(extboard_core::SOLID) as f32,
            heads: Vec2::new(
                f32::from(edge.from_end.unwrap_or(End::None) == End::Arrow),
                f32::from(edge.to_end.unwrap_or(End::Arrow) == End::Arrow),
            ),
            pad: Vec2::ZERO,
        };
        if materials
            .get(&material.0)
            .is_some_and(|line| line.paint != paint)
            && let Some(mut line) = materials.get_mut(&material.0)
        {
            line.paint = paint;
        }
    }
}

#[allow(clippy::type_complexity, reason = "a system's arguments are its query")]
fn track_labels(
    camera: Single<(&Camera, &GlobalTransform, &Projection), With<Camera2d>>,
    editing: Res<crate::text::Editing>,
    raster: Res<Raster>,
    mut edges: Query<(&EdgeId, &Transform, &mut LabelSize)>,
    mut labels: Query<(
        &EdgeLabel,
        &mut Node,
        &mut UiTransform,
        &mut Visibility,
        &ComputedNode,
    )>,
) {
    let (camera, cam_global, projection) = *camera;
    let Projection::Orthographic(ortho) = projection else {
        return;
    };
    let zoom = 1.0 / ortho.scale;
    for (label, mut node, mut transform, mut visibility, computed) in &mut labels {
        let Some((_, at, mut size)) = edges.iter_mut().find(|(id, ..)| id.0 == label.0) else {
            continue;
        };
        // The editor draws the label for as long as it is open; two of them over
        // one midpoint is what read as a form.
        visibility.set_if_neq(if editing.edge() == Some(label.0.as_str()) {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        });
        let measured = computed.size() * computed.inverse_scale_factor() / raster.0;
        if size.0 != measured {
            size.0 = measured;
        }
        let Some(screen) = world_to_screen(camera, cam_global, at.translation.truncate()) else {
            continue;
        };
        if (node.left, node.top) != (px(screen.x), px(screen.y)) {
            (node.left, node.top) = (px(screen.x), px(screen.y));
        }
        let want = UiTransform {
            translation: Val2::percent(-50.0, -50.0),
            scale: Vec2::splat(zoom / raster.0),
            ..UiTransform::IDENTITY
        };
        if *transform != want {
            *transform = want;
        }
    }
}

#[cfg(test)]
mod tests;
