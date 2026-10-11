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

// The higher of its two ends in the board's order: an edge is drawn just over
// the front one, so it runs across what is behind it and under what is ahead.
pub(super) fn rank(canvas: &Canvas, edge: &extboard_core::Edge) -> Option<usize> {
    let index = |id: &str| canvas.nodes.iter().position(|node| node.id == id);
    Some(index(&edge.from_node)?.max(index(&edge.to_node)?))
}

// Where the edge sits among the node depths, for a click to weigh it against
// the node under it.
pub fn edge_depth(canvas: &Canvas, id: &str) -> Option<f32> {
    let edge = canvas.edges.iter().find(|edge| edge.id == id)?;
    Some(crate::node::depth(rank(canvas, edge)?, canvas.nodes.len()))
}

// Whether any of the segment lies in the rect: clipped against each slab in turn.
pub fn crosses(rect: Rect, a: Vec2, b: Vec2) -> bool {
    let span = b - a;
    let (mut low, mut high) = (0.0_f32, 1.0_f32);
    for axis in 0..2 {
        let (min, max, from, step) = (rect.min[axis], rect.max[axis], a[axis], span[axis]);
        if step.abs() < f32::EPSILON {
            if from < min || from > max {
                return false;
            }
            continue;
        }
        let (t0, t1) = ((min - from) / step, (max - from) / step);
        low = low.max(t0.min(t1));
        high = high.min(t0.max(t1));
        if low > high {
            return false;
        }
    }
    true
}
