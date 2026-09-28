use bevy::prelude::*;
use extboard_core::{Canvas, End, Node, Side};

use crate::client::Document;
use crate::node::to_world;

const EDGE_COLOR: Color = Color::srgb(0.45, 0.5, 0.55);
const HEAD_LEN: f32 = 18.0;
const HEAD_HALF_WIDTH: f32 = 8.0;
const LABEL_SIZE: f32 = 12.0;

pub struct ScenePlugin;

// Gizmos cannot draw text, so labels are entities, respawned with the document.
#[derive(Component)]
struct EdgeLabel;

impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup);
        app.add_systems(
            Update,
            (
                draw_edges.run_if(resource_exists::<Document>),
                spawn_edge_labels.run_if(resource_exists_and_changed::<Document>),
            ),
        );
    }
}

fn setup(mut store: ResMut<GizmoConfigStore>) {
    store.config_mut::<DefaultGizmoConfigGroup>().0.line.width = 3.0;
}

fn draw_edges(mut gizmos: Gizmos, document: Res<Document>) {
    for (edge, from, to) in resolved_edges(&document.0) {
        let (a, b) = edge_ends(from, edge.from_side, to, edge.to_side);

        let from_arrow = edge.from_end.unwrap_or(End::None) == End::Arrow;
        let to_arrow = edge.to_end.unwrap_or(End::Arrow) == End::Arrow;

        // Stop the shaft at each head's base: a line through an outlined
        // triangle does not read as an arrow.
        let dir = (b - a).normalize_or_zero();
        let shaft_a = if from_arrow { a + dir * HEAD_LEN } else { a };
        let shaft_b = if to_arrow { b - dir * HEAD_LEN } else { b };
        gizmos.line_2d(shaft_a, shaft_b, EDGE_COLOR);

        if to_arrow {
            arrow_head(&mut gizmos, b, dir);
        }
        if from_arrow {
            arrow_head(&mut gizmos, a, -dir);
        }
    }
}

// Closed triangle, tip at `tip`, pointing along `dir`. Gizmos have no fill, so
// this is an outline; at the default 2px stroke it reads as solid.
fn arrow_head(gizmos: &mut Gizmos, tip: Vec2, dir: Vec2) {
    let base = tip - dir * HEAD_LEN;
    let side = dir.perp() * HEAD_HALF_WIDTH;
    gizmos.linestrip_2d([tip, base + side, base - side, tip], EDGE_COLOR);
}

fn spawn_edge_labels(
    mut commands: Commands,
    document: Res<Document>,
    existing: Query<Entity, With<EdgeLabel>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }

    for (edge, from, to) in resolved_edges(&document.0) {
        let Some(label) = &edge.label else {
            continue;
        };
        let (a, b) = edge_ends(from, edge.from_side, to, edge.to_side);
        commands.spawn((
            EdgeLabel,
            Text2d::new(label.clone()),
            TextFont::from_font_size(LABEL_SIZE),
            // z=1: above the node rects, which sit at 0.
            Transform::from_translation(a.midpoint(b).extend(1.0)),
        ));
    }
}

// Each edge with the nodes it joins. extd validates both ends exist, but a
// hand-edited file between fetches can still dangle: skip, do not panic.
fn resolved_edges(canvas: &Canvas) -> impl Iterator<Item = (&extboard_core::Edge, &Node, &Node)> {
    // ponytail: linear scan per end. Index by id when a board is big enough
    // to notice.
    let find = |id: &str| canvas.nodes.iter().find(|node| node.id == id);
    canvas
        .edges
        .iter()
        .filter_map(move |edge| Some((edge, find(&edge.from_node)?, find(&edge.to_node)?)))
}

// Canvas coordinates from core, flipped into Bevy's +y-up world.
fn edge_ends(
    from: &Node,
    from_side: Option<Side>,
    to: &Node,
    to_side: Option<Side>,
) -> (Vec2, Vec2) {
    let (a, b) = extboard_core::edge_ends(from, from_side, to, to_side);
    (to_world(Vec2::from(a)), to_world(Vec2::from(b)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use extboard_core::NodeKind;

    // The flip is all this module adds; the sides themselves are core's.
    #[test]
    fn edge_ends_flip_canvas_y_into_world_y() {
        let node = |x: i64| Node {
            id: "n".to_owned(),
            x,
            y: 0,
            width: 100,
            height: 50,
            color: None,
            kind: NodeKind::Text {
                text: String::new(),
            },
            extra: Default::default(),
        };

        let (from, to) = edge_ends(&node(0), None, &node(300), None);
        assert_eq!(from, Vec2::new(100.0, -25.0));
        assert_eq!(to, Vec2::new(300.0, -25.0));
    }
}
