use super::edit_text::written_back;
use super::panel::{back_to_front, centre_scale_offset, fill};
use super::*;
use extboard_core::{Canvas, Edge};

// The scaled content's corner, relative to the clip box's. Non-zero clips.
fn content_top_left(size: Vec2, zoom: f32) -> Vec2 {
    let center = size / 2.0 + centre_scale_offset(size, zoom);
    center - size * zoom / 2.0
}

fn canvas(text: &str) -> Canvas {
    Canvas {
        nodes: vec![CanvasNode {
            id: "n".to_owned(),
            x: 0,
            y: 0,
            width: 200,
            height: 100,
            color: None,
            kind: NodeKind::Text {
                text: text.to_owned(),
            },
            extra: default(),
        }],
        edges: Vec::new(),
        extra: default(),
    }
}

fn node(id: &str) -> Target {
    Target::Node(id.to_owned())
}

#[test]
fn only_a_changed_buffer_is_written_back() {
    let mut got = canvas("before");
    assert!(written_back(&mut got, &node("n"), "after"));
    assert_eq!(markdown(&got.nodes[0]), Some("after"));

    // Opened and closed without typing: the document must not be woken.
    assert!(!written_back(&mut got, &node("n"), "after"));
    // And a node that is gone, or was never text, is not an error.
    assert!(!written_back(&mut got, &node("gone"), "after"));
}

#[test]
fn an_emptied_label_is_dropped_rather_than_written_blank() {
    let mut got = canvas("");
    got.edges.push(Edge {
        id: "e1".to_owned(),
        from_node: "n".to_owned(),
        from_side: None,
        from_end: None,
        to_node: "n".to_owned(),
        to_side: None,
        to_end: None,
        label: None,
        extra: default(),
    });
    let edge = Target::Edge("e1".to_owned());

    assert!(written_back(&mut got, &edge, "depends on"));
    assert_eq!(got.edges[0].label.as_deref(), Some("depends on"));
    assert!(!written_back(&mut got, &edge, "depends on"));

    // Cleared in the editor: the key goes, it does not become "".
    assert!(written_back(&mut got, &edge, ""));
    assert_eq!(got.edges[0].label, None);
    assert!(!written_back(
        &mut got,
        &Target::Edge("gone".to_owned()),
        "x"
    ));
}

#[test]
fn content_fills_its_clip_box_at_every_zoom() {
    let size = Vec2::new(740.0, 460.0);
    for zoom in [0.25, 0.5, 1.0, 2.0, 8.0] {
        let corner = content_top_left(size, zoom);
        assert!(corner.abs().max_element() < 1e-3, "zoom {zoom}: {corner}");
    }
}

fn line(blocks: &[Block], index: usize) -> &[Span] {
    match &blocks[index] {
        Block::Line(spans) => spans,
        other => panic!("block {index} is {other:?}, not a line"),
    }
}

#[test]
fn inline_runs_keep_their_marks() {
    let got = blocks("plain *em* **strong** `code()` [link](https://bevy.org)");
    let spans = line(&got, 0);
    let marks: Vec<_> = spans
        .iter()
        .map(|s| (s.text.as_str(), s.bold, s.italic, s.mono, s.link))
        .collect();
    assert_eq!(
        marks,
        [
            ("plain ", false, false, false, false),
            ("em", false, true, false, false),
            (" ", false, false, false, false),
            ("strong", true, false, false, false),
            (" ", false, false, false, false),
            ("code()", false, false, true, false),
            (" ", false, false, false, false),
            ("link", false, false, false, true),
        ]
    );
}

#[test]
fn a_heading_is_bold_and_bigger_than_the_body() {
    let got = blocks("# Title\n\nbody\n");
    let title = &line(&got, 0)[0];
    assert!(title.bold && title.size > BODY, "{title:?}");
    assert_eq!(line(&got, 1)[0].size, BODY);
}

#[test]
fn nested_emphasis_closes_inside_out() {
    let got = blocks("**bold *both* tail**");
    let spans = line(&got, 0);
    assert!(spans.iter().all(|s| s.bold), "{spans:?}");
    assert_eq!(
        spans
            .iter()
            .filter(|s| s.italic)
            .map(|s| &s.text)
            .collect::<Vec<_>>(),
        ["both"]
    );
}

#[test]
fn a_table_becomes_a_grid_of_cells_in_row_order() {
    let got = blocks("| crate | verdict |\n|---|---|\n| bevy | parley |\n| pulldown | tables |\n");
    let Block::Table { cols, cells } = &got[0] else {
        panic!("{got:?}");
    };
    assert_eq!(*cols, 2);
    assert_eq!(cells.len(), 6, "2 columns x 3 rows");
    assert_eq!(
        cells
            .iter()
            .map(|c| (c.text.as_str(), c.head))
            .collect::<Vec<_>>(),
        [
            ("crate", true),
            ("verdict", true),
            ("bevy", false),
            ("parley", false),
            ("pulldown", false),
            ("tables", false),
        ]
    );
}

#[test]
fn a_code_block_keeps_its_newlines_and_drops_the_trailing_one() {
    let got = blocks("```rust\nfn main() {\n    ok();\n}\n```\n");
    assert_eq!(got, [Block::Code("fn main() {\n    ok();\n}".to_owned())]);
}

#[test]
fn list_items_are_one_line_each_with_a_bullet() {
    let got = blocks("- first\n- second **bold**\n");
    assert_eq!(got.len(), 2);
    assert_eq!(line(&got, 0)[0].text, "- ");
    assert_eq!(line(&got, 1).last().unwrap().text, "bold");
}

fn sized(id: &str, width: i64, kind: NodeKind) -> CanvasNode {
    CanvasNode {
        id: id.to_owned(),
        x: 0,
        y: 0,
        width,
        height: width,
        color: None,
        kind,
        extra: default(),
    }
}

// The bug this fixes: a panel is UI, and UI is drawn after the whole 2D world,
// so nothing but another panel can ever cover one.
#[test]
fn panels_are_ordered_back_to_front_by_area() {
    let text = || NodeKind::Text {
        text: String::new(),
    };
    let nodes = [
        sized("small", 100, text()),
        sized("group", 900, NodeKind::Group { label: None }),
        sized("medium", 300, text()),
    ];

    let order: Vec<&str> = back_to_front(&nodes)
        .iter()
        .map(|node| node.id.as_str())
        .collect();
    assert_eq!(order, ["group", "medium", "small"]);
}

#[test]
fn only_a_group_lets_what_is_inside_it_show_through() {
    let group = fill(&sized("g", 100, NodeKind::Group { label: None }));
    let text = fill(&sized(
        "t",
        100,
        NodeKind::Text {
            text: String::new(),
        },
    ));
    assert!(group.alpha() < 1.0, "a group has to be see-through");
    assert_eq!(text.alpha(), 1.0, "everything else has to cover");
}
