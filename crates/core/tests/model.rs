//! The ticket's "done when": every fixture parses. The round trip is the half
//! that actually proves the no-data-loss claim — parsing alone is happy to
//! drop a key it never looked at.

use extboard_core::{Canvas, NodeKind};

const FIXTURES: [(&str, &str); 3] = [
    ("simple", include_str!("fixtures/simple.canvas")),
    ("unknown-keys", include_str!("fixtures/unknown-keys.canvas")),
    ("kitchen-sink", include_str!("fixtures/kitchen-sink.canvas")),
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
