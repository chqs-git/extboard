use super::*;

const START: Rect = Rect {
    min: Vec2::new(0.0, 0.0),
    max: Vec2::new(200.0, 100.0),
};

#[test]
fn resizing_from_a_corner_pins_the_opposite_one() {
    // Top-left in world space is (min.x, max.y): both of those move.
    let got = resized(START, IVec2::new(-1, 1), Vec2::new(-40.0, 160.0), MIN_SIZE);
    assert_eq!(got.min, Vec2::new(-40.0, 0.0));
    assert_eq!(got.max, Vec2::new(200.0, 160.0));
}

#[test]
fn an_edge_handle_moves_one_side_only() {
    let got = resized(START, IVec2::new(1, 0), Vec2::new(300.0, 999.0), MIN_SIZE);
    assert_eq!(got.min, START.min);
    assert_eq!(got.max, Vec2::new(300.0, 100.0));
}

#[test]
fn the_minimum_holds_and_never_crosses_the_far_edge() {
    // Dragging the right edge far past the left one.
    let got = resized(START, IVec2::new(1, 0), Vec2::new(-500.0, 0.0), MIN_SIZE);
    assert_eq!(got.width(), MIN_SIZE);
    assert_eq!(got.min.x, START.min.x);
    // And the same from the other side.
    let got = resized(START, IVec2::new(-1, 0), Vec2::new(500.0, 0.0), MIN_SIZE);
    assert_eq!(got.width(), MIN_SIZE);
    assert_eq!(got.max.x, START.max.x);
}

fn canvas() -> Canvas {
    Canvas {
        nodes: vec![CanvasNode {
            id: "n".to_owned(),
            x: 100,
            y: 40,
            width: 200,
            height: 100,
            color: None,
            kind: extboard_core::NodeKind::Text {
                text: String::new(),
            },
            extra: Default::default(),
        }],
        edges: Vec::new(),
        extra: Default::default(),
    }
}

// Canvas +y is down, so dragging up in the world lowers y.
#[test]
fn a_move_writes_the_canvas_top_left_with_y_flipped() {
    let mut canvas = canvas();
    let center = Vec2::new(200.0, -90.0) + Vec2::new(30.0, 25.0);
    moved(&mut canvas, "n", center);
    let node = &canvas.nodes[0];
    assert_eq!((node.x, node.y), (130, 15));
    assert_eq!((node.width, node.height), (200, 100));
}

#[test]
fn a_resize_from_the_top_left_leaves_the_bottom_right_alone() {
    let mut canvas = canvas();
    let start = Rect::from_center_size(Vec2::new(200.0, -90.0), Vec2::new(200.0, 100.0));
    sized(
        &mut canvas,
        "n",
        resized(start, IVec2::new(-1, 1), Vec2::new(60.0, -20.0), MIN_SIZE),
    );
    let node = &canvas.nodes[0];
    assert_eq!((node.x, node.y), (60, 20));
    // Bottom-right was (300, 140) in canvas coordinates; it still is.
    assert_eq!((node.x + node.width, node.y + node.height), (300, 140));
}

#[test]
fn every_edge_and_corner_answers_inside_its_own_band() {
    let band = 10.0;
    // Corners first: both bands overlap there, and both axes must report.
    for corner in [
        IVec2::new(-1, -1),
        IVec2::new(1, -1),
        IVec2::new(-1, 1),
        IVec2::new(1, 1),
    ] {
        let point = START.center() + (START.half_size() - Vec2::splat(1.0)) * corner.as_vec2();
        assert_eq!(handle_at(START, point, band), Some(corner), "{corner}");
    }
    // Mid-edge, the length of the edge away from any corner.
    assert_eq!(
        handle_at(START, Vec2::new(100.0, 2.0), band),
        Some(IVec2::new(0, -1))
    );
    assert_eq!(
        handle_at(START, Vec2::new(198.0, 50.0), band),
        Some(IVec2::new(1, 0))
    );
    // The middle moves, and outside is nobody's.
    assert_eq!(handle_at(START, START.center(), band), None);
    assert_eq!(handle_at(START, Vec2::new(-20.0, 50.0), band), None);
}

#[test]
fn the_band_never_swallows_the_whole_node() {
    let tiny = Rect::from_center_size(Vec2::ZERO, Vec2::splat(MIN_SIZE));
    assert_eq!(handle_at(tiny, tiny.center(), 200.0), None);
}

#[test]
fn each_handle_points_the_way_its_edge_moves() {
    use SystemCursorIcon::*;
    assert!(matches!(resize_cursor(IVec2::new(0, 1)), NsResize));
    assert!(matches!(resize_cursor(IVec2::new(-1, 0)), EwResize));
    // World +y is up, so (-1, 1) is the top-left corner.
    assert!(matches!(resize_cursor(IVec2::new(-1, 1)), NwseResize));
    assert!(matches!(resize_cursor(IVec2::new(1, -1)), NwseResize));
    assert!(matches!(resize_cursor(IVec2::new(1, 1)), NeswResize));
    assert!(matches!(resize_cursor(IVec2::new(-1, -1)), NeswResize));
}

#[test]
fn a_third_press_does_not_ride_the_second_ones_pair() {
    let mut last = None;
    assert!(!double_click(&mut last, 0.0, Vec2::ZERO));
    assert!(double_click(&mut last, 0.1, Vec2::new(2.0, 0.0)));
    assert!(!double_click(&mut last, 0.2, Vec2::ZERO));
}

#[test]
fn a_slow_or_distant_second_press_is_two_clicks() {
    let mut last = None;
    double_click(&mut last, 0.0, Vec2::ZERO);
    assert!(!double_click(&mut last, 1.0, Vec2::ZERO));

    let mut last = None;
    double_click(&mut last, 0.0, Vec2::ZERO);
    assert!(!double_click(&mut last, 0.1, Vec2::new(40.0, 0.0)));
}

#[test]
fn a_new_node_is_centred_on_the_cursor_and_keeps_its_own_id() {
    let mut canvas = canvas();
    let id = created(
        &mut canvas,
        7,
        Rect::from_center_size(Vec2::new(200.0, -90.0), NEW_SIZE),
    );
    assert_ne!(id, "n");
    let node = canvas.nodes.iter().find(|n| n.id == id).unwrap();
    let half = NEW_SIZE / 2.0;
    assert_eq!(
        (node.width, node.height),
        (NEW_SIZE.x as i64, NEW_SIZE.y as i64)
    );
    // Canvas +y is down: the top-left is above and left of the world centre.
    assert_eq!((node.x, node.y), (200 - half.x as i64, 90 - half.y as i64));
    assert!(matches!(&node.kind, NodeKind::Text { text } if text.is_empty()));
}

#[test]
fn a_band_drawn_node_is_exactly_the_band() {
    let mut canvas = canvas();
    let rect = Rect::from_corners(Vec2::new(-20.0, -300.0), Vec2::new(130.0, -80.0));
    let id = created(&mut canvas, 3, rect);
    let node = canvas.nodes.iter().find(|n| n.id == id).unwrap();
    assert_eq!((node.width, node.height), (150, 220));
    // World top-left is (min.x, max.y); canvas y is that flipped.
    assert_eq!((node.x, node.y), (-20, 80));
}

#[test]
fn deleting_a_node_takes_its_edges_with_it() {
    let mut canvas = canvas();
    let other = created(&mut canvas, 1, Rect::from_center_size(Vec2::ZERO, NEW_SIZE));
    canvas
        .add_edge(extboard_core::Edge {
            id: "e".to_owned(),
            from_node: "n".to_owned(),
            from_side: None,
            from_end: None,
            to_node: other,
            to_side: None,
            to_end: None,
            label: None,
            extra: Default::default(),
        })
        .unwrap();

    canvas.remove_node("n").unwrap();
    assert_eq!(canvas.nodes.len(), 1);
    assert!(canvas.edges.is_empty());
}
