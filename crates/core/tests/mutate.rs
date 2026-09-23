//! E1-T6, the `remove_node` half. The cascade is the point: a removed node
//! must not leave an edge pointing at an id that is gone, which is precisely
//! what `validate` rejects at the write boundary.

use extboard_core::{Canvas, Edge, MutationError, Node, NodeKind, validate};

fn node(id: &str) -> String {
    format!(r#"{{"id":"{id}","type":"text","text":"x","x":0,"y":0,"width":10,"height":10}}"#)
}

fn edge(id: &str, from: &str, to: &str) -> String {
    format!(r#"{{"id":"{id}","fromNode":"{from}","toNode":"{to}"}}"#)
}

fn canvas(nodes: &[String], edges: &[String]) -> Canvas {
    serde_json::from_str(&format!(
        r#"{{"nodes":[{}],"edges":[{}]}}"#,
        nodes.join(","),
        edges.join(",")
    ))
    .unwrap()
}

/// hub has three edges: one out, one in, one to itself.
fn hub() -> Canvas {
    canvas(
        &[
            node("hub"),
            node("a"),
            node("b"),
            node("far"),
            node("lonely"),
        ],
        &[
            edge("out", "hub", "a"),
            edge("in", "b", "hub"),
            edge("loop", "hub", "hub"),
            edge("untouched", "a", "far"),
        ],
    )
}

#[test]
fn removing_a_node_with_three_edges_leaves_a_valid_document() {
    let mut c = hub();
    assert!(validate(&c).is_ok(), "fixture was not valid to begin with");

    c.remove_node("hub").unwrap();

    assert!(validate(&c).is_ok(), "{:?}", validate(&c).unwrap_err());
    assert!(!c.nodes.iter().any(|n| n.id == "hub"));
    assert_eq!(c.nodes.len(), 4);
}

#[test]
fn only_the_attached_edges_go() {
    let mut c = hub();
    c.remove_node("hub").unwrap();

    let ids: Vec<&str> = c.edges.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["untouched"], "cascade took the wrong edges");
}

#[test]
fn an_unknown_node_is_an_error_not_a_no_op() {
    let mut c = hub();
    let before = c.clone();

    assert_eq!(
        c.remove_node("ghost"),
        Err(MutationError::NoSuchNode("ghost".into()))
    );
    assert_eq!(c, before, "a failed mutation must not change the document");
}

#[test]
fn removing_an_unattached_node_leaves_every_edge() {
    let mut c = hub();
    c.remove_node("lonely").unwrap();

    assert_eq!(c.edges.len(), 4, "cascade fired for a node with no edges");
    assert!(validate(&c).is_ok());
}

#[test]
fn a_node_is_removed_by_its_incoming_edge_too() {
    // `far` is only ever a `toNode`, so this covers the to-side of the cascade
    let mut c = hub();
    c.remove_node("far").unwrap();

    let ids: Vec<&str> = c.edges.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["out", "in", "loop"]);
    assert!(validate(&c).is_ok());
}

#[test]
fn move_node_actually_moves_it() {
    let mut c = hub();
    c.move_node("a", 99, -77).unwrap();

    let moved = c.nodes.iter().find(|n| n.id == "a").unwrap();
    assert_eq!((moved.x, moved.y), (99, -77));
}

#[test]
fn move_node_keeps_the_array_order() {
    // array position is z-order, so a move must not raise or lower the node
    let mut c = hub();
    let before: Vec<String> = c.nodes.iter().map(|n| n.id.clone()).collect();

    c.move_node("a", 5, 5).unwrap();

    let after: Vec<String> = c.nodes.iter().map(|n| n.id.clone()).collect();
    assert_eq!(before, after, "the move reordered the nodes");
}

#[test]
fn move_node_changes_nothing_else() {
    let mut c = hub();
    let before = c.clone();
    c.move_node("a", 5, 5).unwrap();

    for (a, b) in c.nodes.iter().zip(&before.nodes) {
        if a.id == "a" {
            assert_eq!(
                Node {
                    x: b.x,
                    y: b.y,
                    ..a.clone()
                },
                *b
            );
        } else {
            assert_eq!(a, b);
        }
    }
    assert_eq!(c.edges, before.edges);
}

#[test]
fn moving_an_unknown_node_is_an_error() {
    let mut c = hub();
    let before = c.clone();

    assert_eq!(
        c.move_node("ghost", 1, 1),
        Err(MutationError::NoSuchNode("ghost".into()))
    );
    assert_eq!(c, before);
}

#[test]
fn add_node_appends_and_stays_valid() {
    let mut c = hub();
    let new: Node = serde_json::from_str(&node("fresh")).unwrap();

    c.add_node(new).unwrap();

    assert_eq!(
        c.nodes.last().unwrap().id,
        "fresh",
        "new node must be on top"
    );
    assert!(validate(&c).is_ok());
}

#[test]
fn add_node_refuses_a_duplicate_id() {
    let mut c = hub();
    let clash: Node = serde_json::from_str(&node("hub")).unwrap();
    let before = c.clone();

    assert_eq!(
        c.add_node(clash),
        Err(MutationError::DuplicateNodeId("hub".into()))
    );
    assert_eq!(c, before);
    // the point of refusing: pushing it would have made the document invalid
    assert!(validate(&c).is_ok());
}

#[test]
fn resize_node_sets_both_dimensions() {
    let mut c = hub();
    c.resize_node("a", 300, 120).unwrap();

    let n = c.nodes.iter().find(|n| n.id == "a").unwrap();
    assert_eq!((n.width, n.height), (300, 120));
}

#[test]
fn resize_node_refuses_a_non_positive_size() {
    let mut c = hub();
    let before = c.clone();

    for (w, h) in [(0, 10), (10, 0), (-5, 10), (10, -5)] {
        assert!(c.resize_node("a", w, h).is_err(), "{w}x{h} was accepted");
    }
    assert_eq!(c, before, "a refused resize must not change the document");

    // and the lookup still happens for a good size
    assert_eq!(
        c.resize_node("ghost", 10, 10),
        Err(MutationError::NoSuchNode("ghost".into()))
    );
}

#[test]
fn set_text_replaces_the_body() {
    let mut c = hub();
    c.set_text("a", "# heading\n\nbody".into()).unwrap();

    let n = c.nodes.iter().find(|n| n.id == "a").unwrap();
    assert!(matches!(&n.kind, NodeKind::Text { text } if text == "# heading\n\nbody"));
}

#[test]
fn set_text_refuses_a_non_text_node() {
    let mut c: Canvas = serde_json::from_str(
        r#"{"nodes":[{"id":"g","type":"group","label":"box","x":0,"y":0,"width":10,"height":10}],"edges":[]}"#,
    )
    .unwrap();
    let before = c.clone();

    assert_eq!(
        c.set_text("g", "nope".into()),
        Err(MutationError::NotATextNode("g".into()))
    );
    assert_eq!(
        c, before,
        "a refused set_text must not change the node kind"
    );
}

#[test]
fn add_edge_appends_and_stays_valid() {
    let mut c = hub();
    let e: Edge = serde_json::from_str(&edge("new", "a", "b")).unwrap();

    c.add_edge(e).unwrap();

    assert_eq!(c.edges.last().unwrap().id, "new");
    assert!(validate(&c).is_ok());
}

#[test]
fn add_edge_refuses_an_endpoint_that_does_not_exist() {
    let mut c = hub();
    let before = c.clone();

    // this is the dangling edge validate exists to reject -- add_edge must
    // never be the thing that creates one
    for (from, to, missing) in [("a", "ghost", "ghost"), ("ghost", "a", "ghost")] {
        let e: Edge = serde_json::from_str(&edge("new", from, to)).unwrap();
        assert_eq!(
            c.add_edge(e),
            Err(MutationError::NoSuchNode(missing.into()))
        );
    }

    assert_eq!(c, before);
    assert!(validate(&c).is_ok());
}

#[test]
fn add_edge_refuses_a_duplicate_id() {
    let mut c = hub();
    let clash: Edge = serde_json::from_str(&edge("out", "a", "b")).unwrap();
    let before = c.clone();

    assert_eq!(
        c.add_edge(clash),
        Err(MutationError::DuplicateEdgeId("out".into()))
    );
    assert_eq!(c, before);
}

#[test]
fn remove_edge_takes_only_that_edge() {
    let mut c = hub();
    c.remove_edge("out").unwrap();

    let ids: Vec<&str> = c.edges.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["in", "loop", "untouched"]);
    assert_eq!(c.nodes.len(), 5, "removing an edge must not touch nodes");
    assert!(validate(&c).is_ok());
}

#[test]
fn removing_an_unknown_edge_is_an_error() {
    let mut c = hub();
    let before = c.clone();

    assert_eq!(
        c.remove_edge("ghost"),
        Err(MutationError::NoSuchEdge("ghost".into()))
    );
    assert_eq!(c, before);
}
