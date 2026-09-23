//! E1-T5. The write boundary: a document whose edges point at ids that do not
//! exist never reaches disk. That is what an LLM produces when it invents an
//! id; a human dragging an edge in the GUI cannot.

use extboard_core::{Canvas, ValidationError, validate};

fn canvas(json: &str) -> Canvas {
    serde_json::from_str(json).unwrap()
}

fn node(id: &str) -> String {
    format!(r#"{{"id":"{id}","type":"text","text":"x","x":0,"y":0,"width":10,"height":10}}"#)
}

fn edge(id: &str, from: &str, to: &str) -> String {
    format!(r#"{{"id":"{id}","fromNode":"{from}","toNode":"{to}"}}"#)
}

fn errors(nodes: &[String], edges: &[String]) -> Vec<ValidationError> {
    let json = format!(
        r#"{{"nodes":[{}],"edges":[{}]}}"#,
        nodes.join(","),
        edges.join(",")
    );
    validate(&canvas(&json)).unwrap_err()
}

#[test]
fn the_fixtures_are_valid() {
    for src in [
        include_str!("fixtures/simple.canvas"),
        include_str!("fixtures/unknown-keys.canvas"),
        include_str!("fixtures/kitchen-sink.canvas"),
    ] {
        assert!(validate(&canvas(src)).is_ok());
    }
}

#[test]
fn two_dangling_edges_report_both() {
    let found = errors(
        &[node("a"), node("b")],
        &[edge("e1", "a", "ghost"), edge("e2", "phantom", "b")],
    );

    assert_eq!(
        found,
        vec![
            ValidationError::DanglingEdge {
                edge: "e1".into(),
                node: "ghost".into()
            },
            ValidationError::DanglingEdge {
                edge: "e2".into(),
                node: "phantom".into()
            },
        ]
    );

    // the ticket's "done when": the offending ids are in the message, so a
    // caller can report them without taking the enum apart
    let rendered = found[0].to_string();
    assert!(
        rendered.contains("e1") && rendered.contains("ghost"),
        "{rendered}"
    );
}

#[test]
fn an_edge_dangling_at_both_ends_reports_each_end() {
    let found = errors(&[node("a")], &[edge("e1", "nope", "also-nope")]);
    assert_eq!(found.len(), 2, "{found:?}");
}

#[test]
fn duplicate_ids_are_caught() {
    assert_eq!(
        errors(&[node("a"), node("a")], &[]),
        vec![ValidationError::DuplicateNodeId("a".into())]
    );
    assert_eq!(
        errors(
            &[node("a"), node("b")],
            &[edge("e1", "a", "b"), edge("e1", "b", "a")]
        ),
        vec![ValidationError::DuplicateEdgeId("e1".into())]
    );
}

#[test]
fn every_problem_is_collected_not_just_the_first() {
    // one of each, all in one document
    let found = errors(
        &[node("a"), node("a")],
        &[edge("e1", "a", "ghost"), edge("e1", "a", "a")],
    );

    assert_eq!(found.len(), 3, "{found:?}");
    assert!(matches!(found[0], ValidationError::DuplicateNodeId(_)));
    assert!(matches!(found[1], ValidationError::DanglingEdge { .. }));
    assert!(matches!(found[2], ValidationError::DuplicateEdgeId(_)));
}

#[test]
fn a_duplicated_node_still_resolves_its_edges() {
    // the duplicate is reported once, but the id exists, so edges pointing at
    // it are not also flagged dangling
    let found = errors(&[node("a"), node("a")], &[edge("e1", "a", "a")]);
    assert_eq!(found, vec![ValidationError::DuplicateNodeId("a".into())]);
}

#[test]
fn a_bad_side_never_reaches_validate() {
    // `BadSide` is not a ValidationError variant: `Side` is a typed enum, so
    // an invalid side is rejected at parse time and no Canvas carrying one can
    // be constructed. The guarantee holds, one layer earlier.
    let bad =
        r#"{"nodes":[],"edges":[{"id":"e1","fromNode":"a","toNode":"b","fromSide":"sideways"}]}"#;
    let err = serde_json::from_str::<Canvas>(bad).unwrap_err();
    assert!(err.to_string().contains("sideways"), "{err}");
}
