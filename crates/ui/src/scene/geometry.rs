use bevy::prelude::*;
use extboard_core::{Canvas, Node, Side};

use crate::node::to_world;

pub fn segments(canvas: &Canvas) -> impl Iterator<Item = (&str, Vec2, Vec2)> {
    resolved_edges(canvas).map(|(edge, from, to)| {
        let (a, b) = edge_ends(from, edge.from_side, to, edge.to_side);
        (edge.id.as_str(), a, b)
    })
}

pub fn nearest(canvas: &Canvas, point: Vec2, reach: f32) -> Option<&str> {
    segments(canvas)
        .map(|(id, a, b)| (id, distance_to_segment(point, a, b)))
        .filter(|(_, distance)| *distance <= reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

// The perpendicular distance, clamped to the segment so that the region past an
// end belongs to that end rather than to the infinite line.
fn distance_to_segment(point: Vec2, a: Vec2, b: Vec2) -> f32 {
    let span = b - a;
    let length_squared = span.length_squared();
    if length_squared == 0.0 {
        return point.distance(a);
    }
    let t = ((point - a).dot(span) / length_squared).clamp(0.0, 1.0);
    point.distance(a + span * t)
}

// Each edge with the nodes it joins. extd validates both ends exist, but a
// hand-edited file between fetches can still dangle: skip, do not panic.
pub(super) fn resolved_edges(
    canvas: &Canvas,
) -> impl Iterator<Item = (&extboard_core::Edge, &Node, &Node)> {
    let find = |id: &str| canvas.nodes.iter().find(|node| node.id == id);
    canvas
        .edges
        .iter()
        .filter_map(move |edge| Some((edge, find(&edge.from_node)?, find(&edge.to_node)?)))
}

// Canvas coordinates from core, flipped into Bevy's +y-up world.
pub(super) fn edge_ends(
    from: &Node,
    from_side: Option<Side>,
    to: &Node,
    to_side: Option<Side>,
) -> (Vec2, Vec2) {
    let (a, b) = extboard_core::edge_ends(from, from_side, to, to_side);
    (to_world(Vec2::from(a)), to_world(Vec2::from(b)))
}
