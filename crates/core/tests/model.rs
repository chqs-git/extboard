use extboard_core::{CIRCLE_SIDES, Canvas, MIN_SIDES, NodeKind, space_path, style};

const FIXTURES: [(&str, &str); 4] = [
    ("simple", include_str!("fixtures/simple.canvas")),
    ("unknown-keys", include_str!("fixtures/unknown-keys.canvas")),
    ("kitchen-sink", include_str!("fixtures/kitchen-sink.canvas")),
    ("shapes", include_str!("fixtures/shapes.canvas")),
];

#[test]
fn unknown_keys_land_in_extra_and_the_type_tag_does_not() {
    let src = FIXTURES[1].1;
    let canvas: Canvas = serde_json::from_str(src).unwrap();

    // Top-level keys we do not model.
    assert!(canvas.extra.contains_key("theme"));
    assert!(canvas.extra.contains_key("extboard"));

    // Per-node and per-edge extensions.
    let shaped = canvas
        .nodes
        .iter()
        .find(|n| n.extra.contains_key("extboardShape"));
    assert!(shaped.is_some(), "node extension dropped");
    assert!(canvas.edges[0].extra.contains_key("extboardWeight"));

    // The flattened enum must claim `type` before the catch-all sees it,
    // otherwise serialising writes it twice.
    for node in &canvas.nodes {
        assert!(
            !node.extra.contains_key("type"),
            "type tag leaked into extra"
        );
        assert!(
            !node.extra.contains_key("text"),
            "variant payload leaked into extra"
        );
    }
}

#[test]
fn every_node_variant_parses() {
    let canvas: Canvas = serde_json::from_str(FIXTURES[2].1).unwrap();
    let kinds: Vec<_> = canvas
        .nodes
        .iter()
        .map(|n| match &n.kind {
            NodeKind::Text { .. } => "text",
            NodeKind::File { .. } => "file",
            NodeKind::Link { .. } => "link",
            NodeKind::Group { .. } => "group",
        })
        .collect();
    for want in ["text", "file", "link", "group"] {
        assert!(kinds.contains(&want), "{want} node did not parse");
    }
}

#[test]
fn coordinates_stay_integers() {
    let canvas: Canvas = serde_json::from_str(FIXTURES[0].1).unwrap();
    let back = serde_json::to_string(&canvas).unwrap();
    assert!(
        back.contains("\"x\":180"),
        "coordinate became a float: {back}"
    );
}

#[test]
fn empty_canvas_parses() {
    let canvas: Canvas = serde_json::from_str("{}").unwrap();
    assert!(canvas.nodes.is_empty() && canvas.edges.is_empty());
}

#[test]
fn sides_ride_on_any_node_and_the_type_is_untouched() {
    let canvas: Canvas = serde_json::from_str(FIXTURES[3].1).unwrap();
    let shapes: Vec<_> = canvas
        .nodes
        .iter()
        .map(|n| (n.sides, matches!(n.kind, NodeKind::Text { .. })))
        .collect();
    assert_eq!(
        shapes,
        vec![
            (Some(6), true),
            (Some(CIRCLE_SIDES), true),
            (Some(MIN_SIDES), false),
            (None, true),
        ]
    );
}

#[test]
fn sides_are_clamped_on_the_way_in() {
    let canvas: Canvas = serde_json::from_str(
        r#"{"nodes":[
            {"id":"a","x":0,"y":0,"width":10,"height":10,"type":"text","text":"","sides":200},
            {"id":"b","x":0,"y":0,"width":10,"height":10,"type":"text","text":"","sides":2},
            {"id":"c","x":0,"y":0,"width":10,"height":10,"type":"text","text":"","sides":9},
            {"id":"d","x":0,"y":0,"width":10,"height":10,"type":"text","text":""}
        ]}"#,
    )
    .unwrap();
    let sides: Vec<_> = canvas.nodes.iter().map(|n| n.sides).collect();
    assert_eq!(
        sides,
        vec![Some(CIRCLE_SIDES), None, Some(CIRCLE_SIDES - 1), None]
    );
}

// The configure panel's keys: the spec's colour on an edge, and the three of
// ours that ride in the extras.
#[test]
fn a_style_survives_the_file() {
    let src = r#"{"nodes":[
            {"id":"a","x":0,"y":0,"width":10,"height":10,"type":"text","text":"",
             "outlineColor":"3","textColor":"5","strokeWidth":3,"font":"secondary"}
        ],"edges":[
            {"id":"e","fromNode":"a","toNode":"a","color":"4","strokeWidth":1}
        ]}"#;
    let canvas: Canvas = serde_json::from_str(src).unwrap();
    let node = &canvas.nodes[0].extra;
    assert_eq!(style::outline_color(node), Some("3"));
    assert_eq!(style::text_color(node), Some("5"));
    assert_eq!(style::stroke_width(node), Some(3));
    assert_eq!(style::font_role(node), Some(1));
    assert_eq!(canvas.edges[0].color.as_deref(), Some("4"));
    assert_eq!(style::stroke_width(&canvas.edges[0].extra), Some(1));

    let back: Canvas = serde_json::from_str(&canvas.to_pretty_string()).unwrap();
    assert_eq!(back, canvas);
}

#[test]
fn a_space_path_is_the_first_segment_after_s() {
    assert_eq!(space_path("/s/kitchen-sink"), Some("kitchen-sink"));
    assert_eq!(space_path("/s/lisbon-trip/"), Some("lisbon-trip"));
    assert_eq!(space_path("/"), None);
    assert_eq!(space_path("/s/"), None);
    assert_eq!(space_path("/v/kitchen-sink"), None);
    assert_eq!(space_path("https://example.com/s/trip"), None);
}
