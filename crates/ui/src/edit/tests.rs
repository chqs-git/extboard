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
            sides: None,
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
    moved(&mut canvas, "n", center, 1.0);
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
        1.0,
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

const NODE: Rect = Rect {
    min: Vec2::new(0.0, 0.0),
    max: Vec2::new(200.0, 100.0),
};

#[test]
fn an_anchor_sits_on_the_middle_of_its_side() {
    // World +y is up, so canvas Top is the rect's high edge.
    assert_eq!(anchor(NODE, Side::Top), Vec2::new(100.0, 100.0));
    assert_eq!(anchor(NODE, Side::Bottom), Vec2::new(100.0, 0.0));
    assert_eq!(anchor(NODE, Side::Left), Vec2::new(0.0, 50.0));
    assert_eq!(anchor(NODE, Side::Right), Vec2::new(200.0, 50.0));
}

#[test]
fn the_live_anchor_is_the_nearest_one_within_reach() {
    let far = Rect::from_corners(Vec2::new(600.0, 0.0), Vec2::new(800.0, 100.0));
    let a = Entity::from_raw_u32(1).unwrap();
    let b = Entity::from_raw_u32(2).unwrap();
    let nodes = || [(a, NODE), (b, far)].into_iter();

    // Outside the node entirely, but still within reach of its right anchor.
    let got = nearest_anchor(nodes(), Vec2::new(220.0, 52.0), 28.0);
    assert_eq!(
        got.map(|(entity, side, _)| (entity, side)),
        Some((a, Side::Right))
    );

    // The middle of a wide node is out of reach of all four.
    assert_eq!(nearest_anchor(nodes(), NODE.center(), 28.0), None);

    // Between the two, the nearer anchor wins.
    let got = nearest_anchor(nodes(), Vec2::new(580.0, 50.0), 28.0);
    assert_eq!(
        got.map(|(entity, side, _)| (entity, side)),
        Some((b, Side::Left))
    );
}

#[test]
fn a_drop_by_a_corner_lands_on_the_edge_it_is_closest_to() {
    // Two units in from the right, ten down from the top: the right edge wins.
    assert_eq!(nearest_side(NODE, Vec2::new(198.0, 90.0)), Side::Right);
    assert_eq!(nearest_side(NODE, Vec2::new(190.0, 98.0)), Side::Top);
    assert_eq!(nearest_side(NODE, Vec2::new(4.0, 50.0)), Side::Left);
    assert_eq!(nearest_side(NODE, Vec2::new(100.0, 3.0)), Side::Bottom);
}

#[test]
fn an_edge_records_both_sides_and_never_joins_a_node_to_itself() {
    let mut canvas = canvas();
    let other = created(&mut canvas, 1, Rect::from_center_size(Vec2::ZERO, NEW_SIZE));

    assert!(!connected(
        &mut canvas,
        2,
        ("n", Side::Right),
        ("n", Side::Left)
    ));
    assert!(canvas.edges.is_empty());

    assert!(connected(
        &mut canvas,
        2,
        ("n", Side::Right),
        (&other, Side::Left)
    ));
    let edge = &canvas.edges[0];
    assert_eq!(
        (edge.from_side, edge.to_side),
        (Some(Side::Right), Some(Side::Left))
    );
    assert_eq!((&edge.from_node, &edge.to_node), (&"n".to_owned(), &other));
}

// The bug this splits apart: anything within the hint's reach of a side used to
// start a new edge, so the tip of a selected edge sitting there was ungrabbable.
#[test]
fn only_the_circle_itself_grips_and_the_hint_reaches_much_further() {
    let entity = Entity::from_raw_u32(1).unwrap();
    let nodes = || [(entity, NODE)].into_iter();
    let near = Vec2::new(220.0, 52.0);

    assert!(nearest_anchor(nodes(), near, Reach::Grip.px()).is_none());
    assert!(nearest_anchor(nodes(), near, Reach::Hover.px()).is_some());
    // On the circle, which is what a press has to mean now.
    let on = anchor(NODE, Side::Right) + Vec2::new(2.0, 2.0);
    assert!(nearest_anchor(nodes(), on, Reach::Grip.px()).is_some());
}

// Both gestures start from the same point, so the circle is the smaller target
// and `grab` asks for it first: what is left over is the arrowhead around it.
#[test]
fn an_edge_end_is_held_by_the_arrowhead_and_the_nearer_end_wins() {
    let mut canvas = canvas();
    let other = created(&mut canvas, 1, Rect::from_center_size(Vec2::ZERO, NEW_SIZE));
    assert!(connected(
        &mut canvas,
        2,
        ("n", Side::Right),
        (&other, Side::Left)
    ));
    let edge = canvas.edges[0].id.clone();
    let (_, from, to) = segments(&canvas).next().unwrap();

    assert!(TIP_PX > Reach::Grip.px());
    assert_eq!(
        tip_under(&canvas, from, TIP_PX),
        Some((edge.clone(), Tip::From))
    );
    assert_eq!(tip_under(&canvas, to, TIP_PX), Some((edge, Tip::To)));
    // Out along the shaft, past either head.
    assert_eq!(tip_under(&canvas, from.midpoint(to), TIP_PX), None);
}

#[test]
fn a_redirect_moves_one_end_and_leaves_the_other_where_it_was() {
    let mut canvas = canvas();
    let second = created(&mut canvas, 1, Rect::from_center_size(Vec2::ZERO, NEW_SIZE));
    let third = created(&mut canvas, 2, Rect::from_center_size(Vec2::X, NEW_SIZE));
    assert!(connected(
        &mut canvas,
        3,
        ("n", Side::Right),
        (&second, Side::Left)
    ));
    let edge = canvas.edges[0].id.clone();

    assert!(redirected(&mut canvas, &edge, Tip::To, (&third, Side::Top)));
    let moved = &canvas.edges[0];
    assert_eq!((&moved.to_node, moved.to_side), (&third, Some(Side::Top)));
    assert_eq!(
        (&moved.from_node, moved.from_side),
        (&"n".to_owned(), Some(Side::Right))
    );

    // The other way round, and the far end is still the one just landed on.
    assert!(redirected(
        &mut canvas,
        &edge,
        Tip::From,
        (&second, Side::Bottom)
    ));
    let moved = &canvas.edges[0];
    assert_eq!(
        (&moved.from_node, moved.from_side),
        (&second, Some(Side::Bottom))
    );
    assert_eq!(&moved.to_node, &third);
}

#[test]
fn a_redirect_onto_the_other_end_is_refused_and_an_unknown_edge_is_not_a_panic() {
    let mut canvas = canvas();
    let other = created(&mut canvas, 1, Rect::from_center_size(Vec2::ZERO, NEW_SIZE));
    assert!(connected(
        &mut canvas,
        2,
        ("n", Side::Right),
        (&other, Side::Left)
    ));
    let edge = canvas.edges[0].id.clone();

    assert!(!redirected(&mut canvas, &edge, Tip::To, ("n", Side::Top)));
    assert_eq!(&canvas.edges[0].to_node, &other);
    assert!(!redirected(
        &mut canvas,
        "gone",
        Tip::To,
        (&other, Side::Top)
    ));
}

#[test]
fn a_group_carries_what_sits_wholly_inside_it() {
    let group = (Entity::from_raw_u32(1).unwrap(), NODE);
    let inside = (
        Entity::from_raw_u32(2).unwrap(),
        Rect::from_corners(Vec2::new(20.0, 20.0), Vec2::new(80.0, 80.0)),
    );
    // Over the border, so it belongs to what is outside the group.
    let straddling = (
        Entity::from_raw_u32(3).unwrap(),
        Rect::from_corners(Vec2::new(180.0, 20.0), Vec2::new(260.0, 80.0)),
    );
    let outside = (
        Entity::from_raw_u32(4).unwrap(),
        Rect::from_corners(Vec2::new(400.0, 0.0), Vec2::new(500.0, 100.0)),
    );

    let got = contained([group, inside, straddling, outside].into_iter(), group);
    assert_eq!(got, [inside.0]);
}

#[test]
fn a_snap_lands_on_the_grid_and_a_grid_of_one_only_rounds() {
    assert_eq!(
        snapped(Vec2::new(103.0, -47.0), GRID),
        Vec2::new(100.0, -50.0)
    );
    assert_eq!(snapped(Vec2::new(105.0, 0.0), GRID), Vec2::new(110.0, 0.0));
    // What the document needs regardless: it holds integers.
    assert_eq!(
        snapped(Vec2::new(103.4, -47.6), 1.0),
        Vec2::new(103.0, -48.0)
    );
}

#[test]
fn a_snapped_drag_puts_both_corners_on_the_grid() {
    let mut canvas = canvas();
    let start = Rect::from_center_size(Vec2::new(203.0, -87.0), Vec2::new(204.0, 96.0));
    sized(&mut canvas, "n", start, GRID);
    let node = &canvas.nodes[0];
    for edge in [node.x, node.y, node.x + node.width, node.y + node.height] {
        assert_eq!(edge % GRID as i64, 0, "{edge} is off the grid");
    }
}

#[test]
fn a_locked_move_keeps_only_the_axis_it_went_furthest_along() {
    let mostly_across = Vec2::new(90.0, 12.0);
    assert_eq!(travel(mostly_across, true), Vec2::new(90.0, 0.0));
    assert_eq!(travel(mostly_across, false), mostly_across);

    let mostly_up = Vec2::new(-8.0, 60.0);
    assert_eq!(travel(mostly_up, true), Vec2::new(0.0, 60.0));
    // A diagonal of exactly equal parts has to pick one, and does not wobble.
    assert_eq!(travel(Vec2::splat(20.0), true), Vec2::new(20.0, 0.0));
}
