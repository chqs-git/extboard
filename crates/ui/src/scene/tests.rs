use super::arrow::box_reach;
use super::geometry::edge_ends;
use super::*;
use extboard_core::{Canvas, Edge, Node, NodeKind};

fn node(id: &str, x: i64) -> Node {
    Node {
        id: id.to_owned(),
        x,
        y: 0,
        width: 100,
        height: 50,
        color: None,
        sides: None,
        kind: NodeKind::Text {
            text: String::new(),
        },
        extra: Default::default(),
    }
}

fn canvas() -> Canvas {
    let edge = |id: &str, from: &str, to: &str| Edge {
        id: id.to_owned(),
        from_node: from.to_owned(),
        from_side: None,
        from_end: None,
        to_node: to.to_owned(),
        to_side: None,
        to_end: None,
        label: None,
        color: None,
        extra: Default::default(),
    };
    Canvas {
        nodes: vec![node("a", 0), node("b", 300), node("c", 900)],
        edges: vec![edge("e1", "a", "b"), edge("e2", "b", "c")],
        extra: Default::default(),
    }
}

// The flip is all this module adds; the sides themselves are core's.
#[test]
fn edge_ends_flip_canvas_y_into_world_y() {
    let (from, to) = edge_ends(&node("n", 0), None, &node("n", 300), None);
    assert_eq!(from, Vec2::new(100.0, -25.0));
    assert_eq!(to, Vec2::new(300.0, -25.0));
}

// What the label rides on: the segment is read from the document every
// frame, so moving a node moves the midpoint with it.
#[test]
fn a_segments_midpoint_moves_with_its_nodes() {
    let mut canvas = canvas();
    let midpoint = |canvas: &Canvas| {
        let (_, a, b) = segments(canvas).next().unwrap();
        a.midpoint(b)
    };
    let before = midpoint(&canvas);
    canvas.nodes[1].y += 400;
    assert_ne!(midpoint(&canvas), before);
}

#[test]
fn a_dangling_edge_is_skipped_rather_than_drawn() {
    let mut canvas = canvas();
    canvas.nodes.retain(|node| node.id != "c");
    assert_eq!(
        segments(&canvas).map(|(id, _, _)| id).collect::<Vec<_>>(),
        ["e1"]
    );
}

#[test]
fn the_shaft_stops_at_the_side_of_the_label_it_meets() {
    let half = Vec2::new(30.0, 8.0);
    // Straight across: the wide side, so half the width.
    assert_eq!(box_reach(half, Vec2::X), 30.0);
    // Straight up: the short side.
    assert_eq!(box_reach(half, Vec2::Y), 8.0);
    // Diagonally, whichever side the ray leaves through first.
    assert!(box_reach(half, Vec2::splat(0.5).normalize()) < 30.0);
    // A degenerate direction has no side to meet.
    assert_eq!(box_reach(half, Vec2::ZERO), f32::INFINITY);
}

#[test]
fn nearest_takes_the_closest_edge_within_reach_and_nothing_outside_it() {
    let canvas = canvas();
    // e1 runs from (100, -25) to (300, -25), e2 from (400, -25) to (900, -25).
    assert_eq!(nearest(&canvas, Vec2::new(200.0, -21.0), 8.0), Some("e1"));
    assert_eq!(nearest(&canvas, Vec2::new(500.0, -25.0), 8.0), Some("e2"));
    // Off the end of e1, not just off the infinite line it sits on.
    assert_eq!(nearest(&canvas, Vec2::new(350.0, -25.0), 8.0), None);
    assert_eq!(nearest(&canvas, Vec2::new(200.0, -200.0), 8.0), None);
}

// A label hangs off the shaft, so on an edge it lies broad side across it the
// text sits tens of pixels from the line `nearest` measures. Its own box is what
// has to catch a click that landed on the text.
#[test]
fn a_label_reaches_where_the_shaft_does_not() {
    let mut canvas = canvas();
    let mut below = node("b", 0);
    below.y = 300;
    canvas.nodes = vec![node("a", 0), below];
    canvas.edges.truncate(1);

    let (_, a, b) = segments(&canvas).find(|(id, _, _)| *id == "e1").unwrap();
    let midpoint = a.midpoint(b);
    let at = Transform::from_translation(midpoint.extend(0.0));
    let box_ = label_box(&at, &LabelSize(Vec2::new(90.0, 14.0)));

    // The far end of the text: past the shaft's reach, inside the label.
    let on_the_text = midpoint + Vec2::new(40.0, 0.0);
    assert_eq!(nearest(&canvas, on_the_text, 8.0), None);
    assert!(box_.contains(on_the_text));
    // And no wider than the text it measured.
    assert!(!box_.contains(midpoint + Vec2::new(46.0, 0.0)));
}

#[test]
fn a_band_catches_an_edge_it_crosses_even_with_both_ends_outside() {
    let band = Rect::from_corners(Vec2::new(-10.0, -10.0), Vec2::new(10.0, 10.0));
    assert!(crosses(band, Vec2::new(-50.0, 0.0), Vec2::new(50.0, 0.0)));
    assert!(crosses(band, Vec2::new(0.0, 0.0), Vec2::new(50.0, 50.0)));
    assert!(!crosses(
        band,
        Vec2::new(-50.0, 20.0),
        Vec2::new(50.0, 20.0)
    ));
    assert!(!crosses(band, Vec2::new(0.0, 30.0), Vec2::new(30.0, 0.0)));
}

#[test]
fn an_edge_stacks_over_the_front_one_of_its_ends() {
    let canvas = canvas();
    assert_eq!(edge_depth(&canvas, "e1"), Some(crate::node::depth(1, 3)));
    assert_eq!(edge_depth(&canvas, "e2"), Some(crate::node::depth(2, 3)));
}
