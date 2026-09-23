//! E1-T2. The byte-identical gate the ticket asks for is not reachable against
//! Obsidian output: Obsidian's key order depends on its *input* order, so there
//! is no single declaration order for our structs to match (PLAN.md §"What this
//! changes in Epic 1"). These are the two properties that are both achievable
//! and load-bearing:
//!
//! - **Our writer is idempotent.** Formatting settled output changes nothing.
//!   That is what lets `rev` hash a re-serialization instead of the file bytes,
//!   so an Obsidian save that only reflows whitespace is not a content change.
//! - **Obsidian input loses nothing.** Semantic, not textual, because the text
//!   is Obsidian's to choose and ours to normalise.

use extboard_core::{Canvas, Node};
use serde_json::Value;

const FIXTURES: [(&str, &str); 3] = [
    ("simple", include_str!("fixtures/simple.canvas")),
    ("unknown-keys", include_str!("fixtures/unknown-keys.canvas")),
    ("kitchen-sink", include_str!("fixtures/kitchen-sink.canvas")),
];

fn parse(name: &str, src: &str) -> Canvas {
    serde_json::from_str(src).unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
fn writer_is_idempotent() {
    for (name, src) in FIXTURES {
        let once = parse(name, src).to_pretty_string();
        let twice = parse(name, &once).to_pretty_string();
        assert_eq!(once, twice, "{name}: second fmt changed bytes");
    }
}

#[test]
fn obsidian_input_loses_nothing() {
    for (name, src) in FIXTURES {
        // Against the *original* Value, not just against ourselves: this is the
        // direction that catches a key we silently dropped on the way in.
        let original: Value = serde_json::from_str(src).unwrap();
        let back = serde_json::to_value(parse(name, src)).unwrap();
        assert_eq!(original, back, "{name} lost or invented a field");
    }
}

#[test]
fn serialised_text_has_no_duplicate_keys() {
    // Value-space equality hides this: two `"type"` keys in the JSON text
    // collapse into one when parsed back. Obsidian reads the text.
    for (name, src) in FIXTURES {
        let text = serde_json::to_string(&parse(name, src)).unwrap();
        for node in text.split("{\"id\"") {
            assert!(
                node.matches("\"type\":").count() <= 1,
                "{name}: duplicate type key"
            );
        }
    }
}

#[test]
fn moving_one_node_touches_nothing_else() {
    let (name, src) = FIXTURES[2]; // kitchen-sink: every variant, plus an edge
    let before = parse(name, src);

    let mut after = before.clone();
    let moved = 1;
    after.nodes[moved].x += 40;
    after.nodes[moved].y -= 25;

    let after = parse(name, &after.to_pretty_string());

    assert_eq!(after.nodes.len(), before.nodes.len(), "node count changed");
    assert_eq!(after.edges, before.edges, "edges changed");
    assert_eq!(after.extra, before.extra, "top-level keys changed");

    for (i, (a, b)) in after.nodes.iter().zip(&before.nodes).enumerate() {
        if i == moved {
            assert_eq!((a.x, a.y), (b.x + 40, b.y - 25), "the move did not land");
            // Everything about the moved node except its position is untouched.
            assert_eq!(
                Node {
                    x: b.x,
                    y: b.y,
                    ..a.clone()
                },
                *b,
                "move altered the node"
            );
        } else {
            assert_eq!(a, b, "node {i} changed and should not have");
        }
    }
}
