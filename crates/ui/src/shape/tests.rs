use bevy::prelude::*;
use extboard_core::{CIRCLE_SIDES, MID_STROKE, MIN_SIDES, Node as CanvasNode, NodeKind, style};

use super::panel::{Row, Target, shown, slot_value};
use super::{Palette, PolygonMaterial, RECT_SIDES, Sides, paint};

#[test]
fn the_count_runs_from_the_triangle_to_the_circle() {
    assert_eq!(Sides::from_notch(3.0), Sides::Ngon(MIN_SIDES));
    assert_eq!(Sides::from_notch(f32::from(CIRCLE_SIDES)), Sides::Circle);
    assert_eq!(Sides::from_notch(-4.0), Sides::Ngon(MIN_SIDES));
    assert_eq!(Sides::from_notch(900.0), Sides::Circle);

    for sides in [Sides::Ngon(3), Sides::Ngon(9), Sides::Circle] {
        assert_eq!(Sides::from_notch(sides.notch()), sides);
        assert_eq!(Sides::of(sides.key()), sides);
    }
}

#[test]
fn the_rectangle_and_four_sides_are_the_same_node() {
    assert_eq!(Sides::of(None), Sides::Ngon(RECT_SIDES));
    assert_eq!(Sides::of(Some(RECT_SIDES)), Sides::Ngon(RECT_SIDES));
    assert_eq!(Sides::Ngon(RECT_SIDES).key(), None);
}

#[test]
fn the_box_takes_a_count_or_a_word_for_the_circle() {
    assert_eq!(Sides::parse("7"), Some(Sides::Ngon(7)));
    assert_eq!(Sides::parse(" 7 "), Some(Sides::Ngon(7)));
    assert_eq!(Sides::parse("10"), Some(Sides::Circle));
    assert_eq!(Sides::parse("1000000"), Some(Sides::Circle));
    assert_eq!(Sides::parse("2"), Some(Sides::Ngon(MIN_SIDES)));
    for word in ["circle", "Circle", "c", "o", "\u{221e}"] {
        assert_eq!(Sides::parse(word), Some(Sides::Circle), "{word}");
    }
    assert_eq!(Sides::parse("cir"), None);
    assert_eq!(Sides::parse(""), None);
    assert_eq!(Sides::parse("-3"), None);
}

fn node(id: &str, sides: Option<u8>) -> CanvasNode {
    CanvasNode {
        id: id.to_owned(),
        x: 0,
        y: 0,
        width: 10,
        height: 10,
        color: None,
        sides,
        kind: NodeKind::Text {
            text: String::new(),
        },
        extra: serde_json::Map::new(),
    }
}

#[test]
fn the_polygon_shader_compiles() {
    crate::wgsl::compiles(include_str!("polygon.wgsl"));
}

#[test]
fn nodes_that_look_alike_share_one_material() {
    let mut palette = Palette::default();
    let mut materials = Assets::<PolygonMaterial>::default();
    let theme = crate::theme::Theme::default();
    let plain = paint(&mut palette, &mut materials, &theme, &node("a", None));
    assert_eq!(
        paint(
            &mut palette,
            &mut materials,
            &theme,
            &node("b", Some(RECT_SIDES))
        ),
        plain
    );
    assert_eq!(
        paint(&mut palette, &mut materials, &theme, &node("c", None)),
        plain
    );
    assert_ne!(
        paint(&mut palette, &mut materials, &theme, &node("d", Some(6))),
        plain
    );
    assert_eq!(materials.len(), 2);
}

#[test]
fn the_stroke_weight_is_part_of_a_nodes_look() {
    let mut palette = Palette::default();
    let mut materials = Assets::<PolygonMaterial>::default();
    let theme = crate::theme::Theme::default();
    let plain = node("a", None);
    let mut heavy = node("b", None);
    style::set_stroke_width(&mut heavy.extra, 3);

    let thin = paint(&mut palette, &mut materials, &theme, &plain);
    assert_ne!(paint(&mut palette, &mut materials, &theme, &heavy), thin);
    // Back to the middle weight is back to the material it started on.
    style::set_stroke_width(&mut heavy.extra, MID_STROKE);
    assert_eq!(paint(&mut palette, &mut materials, &theme, &heavy), thin);
}

// An edge has one colour, and nothing to fill with it.
#[test]
fn an_edge_follows_the_boards_text_until_it_names_a_colour() {
    let theme =
        crate::theme::Theme::from_colors(&["#000000", "#111111", "#222222", "#333333", "#444444"]);
    let mut edge = extboard_core::Edge {
        id: "e".to_owned(),
        from_node: "a".to_owned(),
        from_side: None,
        from_end: None,
        to_node: "b".to_owned(),
        to_side: None,
        to_end: None,
        label: None,
        color: None,
        extra: default(),
    };
    assert_eq!(
        crate::scene::edge_color(&theme, &edge),
        theme.color(extboard_core::TEXT)
    );
    edge.color = Some("3".to_owned());
    assert_eq!(crate::scene::edge_color(&theme, &edge), theme.color(3));
}

#[test]
fn a_nodes_three_colours_paint_its_rim_its_text_and_its_body() {
    let theme =
        crate::theme::Theme::from_colors(&["#000000", "#111111", "#222222", "#333333", "#444444"]);
    let mut styled = node("a", None);
    styled.color = Some("2".to_owned());
    style::set_outline_color(&mut styled.extra, Some("3"));
    style::set_text_color(&mut styled.extra, Some("1"));

    assert_eq!(crate::node::node_color(&theme, &styled), theme.color(2));
    assert_eq!(crate::node::outline_color(&theme, &styled), theme.color(3));
    assert_eq!(
        theme.face(None, style::text_color(&styled.extra)).ink,
        theme.color(1)
    );

    // Saying nothing leaves every one of the three on the role it had.
    let plain = node("b", None);
    assert_eq!(
        crate::node::node_color(&theme, &plain),
        theme.color(extboard_core::SECONDARY)
    );
    assert_eq!(
        crate::node::outline_color(&theme, &plain),
        theme.color(extboard_core::PRIMARY)
    );
    assert_eq!(
        theme.face(None, style::text_color(&plain.extra)).ink,
        theme.color(extboard_core::TEXT)
    );
}

#[test]
fn a_slot_goes_in_as_an_index_while_the_spec_can_spell_one() {
    let theme = crate::theme::Theme::from_colors(&[
        "#000000", "#111111", "#222222", "#333333", "#444444", "#555555", "#666666", "#777777",
    ]);
    // Inside the six, the index: a retheme then moves the node with it.
    assert_eq!(slot_value(&theme, 1), "1");
    assert_eq!(slot_value(&theme, 6), "6");
    // Outside them -- the background and the extra accents -- the colour itself.
    assert_eq!(slot_value(&theme, 0), "#000000");
    assert_eq!(slot_value(&theme, 7), "#777777");
}

#[test]
fn the_panel_reads_a_node_or_an_edge_and_nothing_that_has_gone() {
    let mut canvas = extboard_core::Canvas {
        nodes: vec![node("a", Some(6))],
        edges: vec![],
        extra: serde_json::Map::new(),
    };
    canvas.nodes[0].color = Some("#1a2b3c".to_owned());
    style::set_outline_color(&mut canvas.nodes[0].extra, Some("3"));
    style::set_text_color(&mut canvas.nodes[0].extra, Some("1"));
    style::set_stroke_width(&mut canvas.nodes[0].extra, 1);
    style::set_font_role(&mut canvas.nodes[0].extra, Some(1));
    canvas.edges.push(extboard_core::Edge {
        id: "e".to_owned(),
        from_node: "a".to_owned(),
        from_side: None,
        from_end: None,
        to_node: "a".to_owned(),
        to_side: None,
        to_end: None,
        label: None,
        color: Some("2".to_owned()),
        extra: serde_json::Map::new(),
    });
    let theme = crate::theme::Theme::from_colors(&["#000000", "#111111", "#222222"]);

    let picked = shown(&canvas, &theme, None, Target::Node("a".to_owned())).unwrap();
    assert_eq!(picked.sides, Some(Sides::Ngon(6)));
    assert_eq!(picked.panel.weight, Some(1));
    assert_eq!(picked.panel.font, Some(1));
    assert_eq!(picked.panel.slots, 3);
    assert_eq!(picked.panel.outline.names.as_deref(), Some("3"));
    // A short palette wraps, which is what the chip has to show.
    assert_eq!(picked.panel.outline.code, "#000000");
    assert_eq!(picked.panel.text.names.as_deref(), Some("1"));
    assert_eq!(picked.panel.text.code, "#111111");
    let fill = picked.panel.background.expect("a node has a body");
    assert_eq!(fill.names.as_deref(), Some("#1a2b3c"));
    assert_eq!(fill.code, "#1a2b3c");

    // An edge has no sides to draw a slider for and no body to fill: the spec's
    // colour is its outline.
    let picked = shown(&canvas, &theme, None, Target::Edge("e".to_owned())).unwrap();
    assert_eq!(picked.sides, None);
    assert_eq!(picked.panel.weight, None);
    assert!(picked.panel.background.is_none());
    assert_eq!(picked.panel.outline.code, "#222222");

    // And a list it has no row for is not left open under it.
    for row in [Row::Background, Row::Font] {
        let picked = shown(&canvas, &theme, Some(row), Target::Edge("e".to_owned())).unwrap();
        assert_eq!(picked.panel.open, None, "{row:?}");
    }
    for row in [Row::Outline, Row::Text] {
        let picked = shown(&canvas, &theme, Some(row), Target::Edge("e".to_owned())).unwrap();
        assert_eq!(picked.panel.open, Some(row), "{row:?}");
    }

    // A selection the document has lost closes the panel rather than drawing one.
    assert!(shown(&canvas, &theme, None, Target::Node("gone".to_owned())).is_none());
    assert!(shown(&canvas, &theme, None, Target::Edge("gone".to_owned())).is_none());
}

// Bevy validates a system's queries against each other at init, which is a
// running app rather than a compile: two boxes of the same component in one
// system is a panic on the first frame, not a build error.
#[test]
fn the_panels_queries_stay_disjoint() {
    let mut app = App::new();
    // Nothing these systems read exists here, so every parameter fails
    // validation. Init is what is under test, and it runs first.
    app.set_error_handler(bevy::ecs::error::ignore)
        .add_systems(Update, (super::panel::sync, super::panel::typed));
    app.world_mut().run_schedule(Update);
}
