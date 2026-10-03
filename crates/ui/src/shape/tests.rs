use bevy::prelude::*;
use extboard_core::{CIRCLE_SIDES, MIN_SIDES, Node as CanvasNode, NodeKind};

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

// A mistake in the shader is a blank board rather than a build error. Bevy's
// preprocessor is not here, so the import becomes the struct it would bring.
#[test]
fn the_polygon_shader_compiles() {
    const VERTEX_OUTPUT: &str = "struct UiVertexOutput {
        @location(0) uv: vec2<f32>,
        @location(1) border_widths: vec4<f32>,
        @location(2) border_radius: vec4<f32>,
        @location(3) @interpolate(flat) size: vec2<f32>,
        @builtin(position) position: vec4<f32>,
    };";

    let mut source = String::from(VERTEX_OUTPUT);
    let mut skipping = false;
    for line in include_str!("polygon.wgsl").lines() {
        match line.trim_start() {
            directive if directive.starts_with("#ifdef") => skipping = true,
            directive if directive.starts_with("#endif") => skipping = false,
            directive if directive.starts_with('#') => {}
            _ if skipping => {}
            code => {
                source.push_str(code);
                source.push('\n');
            }
        }
    }

    let module = naga::front::wgsl::parse_str(&source)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
}

#[test]
fn nodes_that_look_alike_share_one_material() {
    let mut palette = Palette::default();
    let mut materials = Assets::<PolygonMaterial>::default();
    let node = |id: &str, sides| CanvasNode {
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
    };

    let plain = paint(&mut palette, &mut materials, &node("a", None));
    assert_eq!(
        paint(&mut palette, &mut materials, &node("b", Some(RECT_SIDES))),
        plain
    );
    assert_eq!(paint(&mut palette, &mut materials, &node("c", None)), plain);
    assert_ne!(
        paint(&mut palette, &mut materials, &node("d", Some(6))),
        plain
    );
    assert_eq!(materials.len(), 2);
}
