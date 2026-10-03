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

#[test]
fn the_polygon_shader_compiles() {
    crate::wgsl::compiles(include_str!("polygon.wgsl"));
}

#[test]
fn nodes_that_look_alike_share_one_material() {
    let mut palette = Palette::default();
    let mut materials = Assets::<PolygonMaterial>::default();
    let theme = crate::theme::Theme::default();
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
