//! E1-T3. The rev is the HTTP `ETag`, so what it must track is *content*:
//! equal documents hash equal however they were built or formatted, and any
//! real edit moves the hash.

use extboard_core::{Canvas, Edge, Node, NodeKind, Side, fresh_id, rev};
use serde_json::Map;
use std::collections::HashSet;

const SIMPLE: &str = include_str!("fixtures/simple.canvas");

fn simple() -> Canvas {
    serde_json::from_str(SIMPLE).unwrap()
}

#[test]
fn same_canvas_same_rev() {
    assert_eq!(rev(&simple()), rev(&simple()));
    // and it survives its own round trip, which is what the 409 path relies on
    let reparsed: Canvas = serde_json::from_str(&simple().to_pretty_string()).unwrap();
    assert_eq!(rev(&simple()), rev(&reparsed));
}

#[test]
fn moving_a_node_changes_the_rev() {
    let before = simple();
    let mut after = before.clone();
    after.nodes[0].x += 1;
    assert_ne!(rev(&before), rev(&after), "a moved node must move the rev");
}

#[test]
fn identical_documents_built_differently_have_equal_revs() {
    // hand-built, never went near the fixture text
    let built = Canvas {
        nodes: vec![
            Node {
                id: "a559e50dc6d68554".into(),
                x: 180,
                y: 0,
                width: 250,
                height: 60,
                color: Some("5".into()),
                kind: NodeKind::Text {
                    text: "Blue box".into(),
                },
                extra: Map::new(),
            },
            Node {
                id: "3b4fe2054591f700".into(),
                x: -420,
                y: 0,
                width: 250,
                height: 60,
                color: Some("1".into()),
                kind: NodeKind::Text {
                    text: "Red box".into(),
                },
                extra: Map::new(),
            },
        ],
        edges: vec![Edge {
            id: "b3a735ca54e8bb28".into(),
            from_node: "3b4fe2054591f700".into(),
            from_side: Some(Side::Right),
            from_end: None,
            to_node: "a559e50dc6d68554".into(),
            to_side: Some(Side::Left),
            to_end: None,
            label: None,
            extra: Map::new(),
        }],
        extra: Map::new(),
    };

    assert_eq!(
        rev(&built),
        rev(&simple()),
        "construction history leaked into the rev"
    );
}

#[test]
fn reformatted_input_keeps_the_rev() {
    // the property the ETag actually rests on: Obsidian reflowing whitespace
    // and reordering keys is not a content change.
    let mangled = r#"{"edges":[{"toSide":"left","fromNode":"3b4fe2054591f700",
        "id":"b3a735ca54e8bb28","toNode":"a559e50dc6d68554","fromSide":"right"}],
      "nodes":[{"text":"Blue box","type":"text","color":"5","id":"a559e50dc6d68554",
                "x":180,"y":0,"width":250,"height":60},
               {"y":0,"x":-420,"id":"3b4fe2054591f700","type":"text",
                "text":"Red box","width":250,"height":60,"color":"1"}]}"#;

    let reordered: Canvas = serde_json::from_str(mangled).unwrap();
    assert_eq!(rev(&reordered), rev(&simple()), "formatting moved the rev");
}
#[test]
fn the_canonical_form_is_pinned() {
    // the rev is the ETag, so the canonical form is part of the wire contract:
    // switching to the pretty form, or changing REV_LEN, silently invalidates
    // every cached ETag and 409s every in-flight PUT. if this test fails, that
    // is what happened -- it is a release-coordination event, not a re-baseline.
    assert_eq!(rev(&simple()), "6535237b11b258b5");
}

#[test]
fn a_thousand_ids_are_distinct_and_miss_the_existing_nodes() {
    let canvas = simple();
    let existing: HashSet<&str> = canvas.nodes.iter().map(|n| n.id.as_str()).collect();

    let ids: HashSet<String> = (0u32..1000)
        .map(|seed| fresh_id(&canvas, &seed.to_le_bytes()))
        .collect();

    assert_eq!(ids.len(), 1000, "seeds collided");
    assert!(
        ids.iter().all(|id| !existing.contains(id.as_str())),
        "collided with a node already in the document"
    );
    assert!(
        ids.iter()
            .all(|id| id.len() == 16 && id.chars().all(|c| c.is_ascii_hexdigit())),
        "not Obsidian's id shape"
    );
}

#[test]
fn a_taken_id_forces_the_counter_forward() {
    // the natural path never collides, so the retry loop is only reachable by
    // planting the id fresh_id is about to produce.
    let mut canvas = simple();
    let first = fresh_id(&canvas, b"seed");

    canvas.nodes.push(Node {
        id: first.clone(),
        x: 0,
        y: 0,
        width: 10,
        height: 10,
        color: None,
        kind: NodeKind::Text {
            text: String::new(),
        },
        extra: Map::new(),
    });

    assert_ne!(
        fresh_id(&canvas, b"seed"),
        first,
        "retry loop did not advance"
    );

    // edges share the check, because fresh_id is the only id source we have
    let mut canvas = simple();
    let edge_id = fresh_id(&canvas, b"edge");
    canvas.edges[0].id = edge_id.clone();
    assert_ne!(
        fresh_id(&canvas, b"edge"),
        edge_id,
        "edge ids are not checked"
    );
}

#[test]
fn the_same_seed_and_document_give_the_same_id() {
    // the documented ceiling, pinned so it is a decision rather than a surprise
    let canvas = simple();
    assert_eq!(fresh_id(&canvas, b"seed"), fresh_id(&canvas, b"seed"));
}
