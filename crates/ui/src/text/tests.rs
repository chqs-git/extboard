use super::edit_text::written_back;
use super::panel::centre_scale_offset;
use super::*;
use extboard_core::{ACCENT, Canvas, Edge};

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
            sides: None,
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
        color: None,
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

// Every run on a node comes out of the palette: a link and a code span take the
// two slots that are not the body's, and none of the three is a constant.
#[test]
fn a_link_and_a_code_span_are_the_themes_rather_than_fixed_colours() {
    let theme = Theme::from_colors(&["#000000", "#111111", "#222222", "#333333", "#444444"]);
    let span = |mono, link| Span {
        text: "x".to_owned(),
        size: BODY,
        bold: false,
        italic: false,
        mono,
        link,
        space: None,
    };
    let face = theme.face(None, None);
    let code = theme.code_face();
    let body = color(&span(false, false), &theme, &face, &code);
    let link = color(&span(false, true), &theme, &face, &code);
    let mono = color(&span(true, false), &theme, &face, &code);
    let ink = |theme: &Theme| {
        color(
            &Span {
                space: Some("trip".to_owned()),
                ..span(false, true)
            },
            theme,
            &theme.face(None, None),
            &theme.code_face(),
        )
    };
    let full = Theme::from_colors(&[
        "#000000", "#111111", "#222222", "#333333", "#444444", "#555555", "#666666", "#777777",
    ]);
    let door = ink(&full);
    assert_eq!(door, full.color(extboard_core::ACCENT_B));
    assert_ne!(
        door,
        full.color(extboard_core::ACCENT),
        "the next accent, not the plain one"
    );
    assert_ne!(
        door,
        color(&span(false, true), &full, &face, &code),
        "a door has to read differently from a web link"
    );

    assert_eq!(ink(&theme), theme.color(extboard_core::ACCENT));
    assert_ne!(ink(&theme), theme.color(extboard_core::BACKGROUND));
    assert_eq!(body, theme.color(TEXT));
    assert_eq!(link, theme.color(PRIMARY));
    assert_eq!(mono, theme.color(ACCENT));
    assert_eq!([body, link, mono].map(|c| c.alpha()), [1.0; 3]);

    // The node's stroke takes the body text with it, and leaves the two runs
    // that are not body text where they are.
    let painted = theme.face(None, Some("1"));
    assert_eq!(
        color(&span(false, false), &theme, &painted, &code),
        theme.color(PRIMARY)
    );
    assert_eq!(color(&span(false, true), &theme, &painted, &code), link);
    assert_eq!(color(&span(true, false), &theme, &painted, &code), mono);
}

// bevy clips an editor to its content box as laid out, before the panel's scale
// reaches it, so any stretch at all is glyphs cut off at the left and the top.
#[test]
fn an_editing_session_is_never_stretched() {
    for zoom in [0.25, 1.0, 1.19, 1.2, 2.4, 3.6, 5.0, 8.0] {
        let scale = zoom / editing_raster(zoom);
        assert!(scale <= 1.0, "zoom {zoom}: scaled by {scale}");
    }
    // The slack a drawn panel is allowed is the stretch that cut the text.
    assert!(1.2 / raster_tier(1.2) > 1.0);
}

// The tier only trades layout size for transform scale: the glyphs are
// rasterized bigger, and the box they land in is the one zoom alone would give.
#[test]
fn a_raster_tier_never_undersamples_or_resizes_the_content() {
    let size = Vec2::new(740.0, 460.0);
    for zoom in [0.25, 1.0, 1.1, 2.0, 2.6, 8.0] {
        let tier = raster_tier(zoom);
        assert!(
            tier * RASTER_SLACK >= zoom.min(RASTER_MAX) && tier <= RASTER_MAX,
            "zoom {zoom}: tier {tier}"
        );
        let scale = zoom / tier;
        let on_screen = size * tier * scale;
        assert!(
            (on_screen - size * zoom).abs().max_element() < 1e-3,
            "{on_screen}"
        );
        let corner = content_top_left(size * tier, scale);
        assert!(corner.abs().max_element() < 1e-3, "zoom {zoom}: {corner}");
    }
}

#[test]
fn a_markdown_link_to_a_space_carries_it_and_a_web_link_does_not() {
    let got = blocks("see [trip](/s/trip) and [bevy](https://bevy.org)");
    let spans = line(&got, 0);
    let doors: Vec<_> = spans
        .iter()
        .map(|span| (span.text.as_str(), span.link, span.space.as_deref()))
        .collect();
    assert_eq!(
        doors,
        [
            ("see ", false, None),
            ("trip", true, Some("trip")),
            (" and ", false, None),
            ("bevy", true, None),
        ]
    );
}

#[test]
fn a_door_is_indexed_by_its_span_position_in_the_line() {
    let spans = blocks("see [trip](/s/trip) and [kitchen](/s/kitchen-sink)");
    assert_eq!(
        doors(line(&spans, 0)),
        [(1, "trip".to_owned()), (3, "kitchen-sink".to_owned()),]
    );
    assert!(doors(line(&blocks("just prose"), 0)).is_empty());
}
