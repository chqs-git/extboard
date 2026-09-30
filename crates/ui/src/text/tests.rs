use super::edit_text::written_back;
use super::*;
use extboard_core::Canvas;

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

#[test]
fn only_a_changed_buffer_is_written_back() {
    let mut got = canvas("before");
    assert!(written_back(&mut got, "n", "after"));
    assert_eq!(markdown(&got.nodes[0]), Some("after"));

    // Opened and closed without typing: the document must not be woken.
    assert!(!written_back(&mut got, "n", "after"));
    // And a node that is gone, or was never text, is not an error.
    assert!(!written_back(&mut got, "gone", "after"));
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
