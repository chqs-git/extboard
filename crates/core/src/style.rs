use serde_json::{Map, Value};

use crate::theme::{FONT_ROLES, PRIMARY_TEXT};

// How a node or an edge is drawn, past the spec's own `color`, which is the
// body it fills: the colour of its outline, the colour of its text, how thick
// the outline is, and which of the space's three texts that text is set in.
// None of the four keys is the spec's, so all four sit in the extras -- which
// is also what lets one set of accessors serve a node and an edge.

// Three weights, as a pen has. They are multiples of whatever width the thing
// being drawn already used, so the middle one is no change at all.
pub const STROKES: [f32; 3] = [0.5, 1.0, 2.0];
pub const MID_STROKE: u8 = 2;

pub fn stroke_width(extra: &Map<String, Value>) -> Option<u8> {
    weighed(u8::try_from(extra.get("strokeWidth")?.as_u64()?).ok()?)
}

// The middle weight is written as no key: a node that has never been styled and
// one styled back to the default are the same node.
pub fn set_stroke_width(extra: &mut Map<String, Value>, weight: u8) {
    match weighed(weight).filter(|weight| *weight != MID_STROKE) {
        Some(weight) => extra.insert("strokeWidth".to_owned(), Value::from(weight)),
        None => extra.remove("strokeWidth"),
    };
}

pub fn stroke_scale(extra: &Map<String, Value>) -> f32 {
    STROKES[usize::from(stroke_width(extra).unwrap_or(MID_STROKE)) - 1]
}

fn weighed(weight: u8) -> Option<u8> {
    (1..=STROKES.len() as u8)
        .contains(&weight)
        .then_some(weight)
}

// A slot in the space's palette or a `#rrggbb`, the two forms the spec's own
// colour field takes: one palette per board, and these are part of it. The
// outline is the rim of a node and the line of an edge; the text is a node's
// own words and an edge's label.
pub fn outline_color(extra: &Map<String, Value>) -> Option<&str> {
    color(extra, "outlineColor")
}

pub fn set_outline_color(extra: &mut Map<String, Value>, color: Option<&str>) {
    set_color(extra, "outlineColor", color);
}

// `textColor`, never `text`: that key is a text node's own content.
pub fn text_color(extra: &Map<String, Value>) -> Option<&str> {
    color(extra, "textColor")
}

pub fn set_text_color(extra: &mut Map<String, Value>, color: Option<&str>) {
    set_color(extra, "textColor", color);
}

fn color<'a>(extra: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    extra.get(key)?.as_str().filter(|color| !color.is_empty())
}

fn set_color(extra: &mut Map<String, Value>, key: &str, color: Option<&str>) {
    match color.filter(|color| !color.is_empty()) {
        Some(color) => extra.insert(key.to_owned(), Value::from(color)),
        None => extra.remove(key),
    };
}

// Which of the space's texts, by the name the theme block gives it. A name the
// roles do not have reads as unset, which is the primary text.
pub fn font_role(extra: &Map<String, Value>) -> Option<usize> {
    let name = extra.get("font")?.as_str()?;
    FONT_ROLES.iter().position(|role| *role == name)
}

// The primary text is written as no key: every node's text was that before one
// could name another.
pub fn set_font_role(extra: &mut Map<String, Value>, role: Option<usize>) {
    match role
        .filter(|role| *role != PRIMARY_TEXT)
        .and_then(|role| FONT_ROLES.get(role))
    {
        Some(name) => extra.insert("font".to_owned(), Value::from(*name)),
        None => extra.remove("font"),
    };
}

// How a line sits in the node: left, centred, or right. Left is written as no
// key, since every text node drew that way before this existed.
pub const ALIGNS: [&str; 3] = ["left", "center", "right"];
pub const LEFT_ALIGN: usize = 0;

pub fn align(extra: &Map<String, Value>) -> Option<usize> {
    let name = extra.get("textAlign")?.as_str()?;
    ALIGNS.iter().position(|align| *align == name)
}

pub fn set_align(extra: &mut Map<String, Value>, align: Option<usize>) {
    match align
        .filter(|align| *align != LEFT_ALIGN)
        .and_then(|align| ALIGNS.get(align))
    {
        Some(name) => extra.insert("textAlign".to_owned(), Value::from(*name)),
        None => extra.remove("textAlign"),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(key: &str, value: Value) -> Map<String, Value> {
        Map::from_iter([(key.to_owned(), value)])
    }

    #[test]
    fn a_weight_outside_the_three_is_no_weight() {
        assert_eq!(stroke_width(&Map::new()), None);
        assert_eq!(stroke_width(&with("strokeWidth", Value::from(1))), Some(1));
        assert_eq!(stroke_width(&with("strokeWidth", Value::from(3))), Some(3));
        assert_eq!(stroke_width(&with("strokeWidth", Value::from(0))), None);
        assert_eq!(stroke_width(&with("strokeWidth", Value::from(9))), None);
        // A hand-written file can put anything under the key.
        assert_eq!(
            stroke_width(&with("strokeWidth", Value::from("thick"))),
            None
        );
        assert_eq!(stroke_width(&with("strokeWidth", Value::from(-1))), None);
        // Unset and the middle weight both draw at the default width.
        assert_eq!(stroke_scale(&Map::new()), 1.0);
        assert_eq!(stroke_scale(&with("strokeWidth", Value::from(9))), 1.0);
        assert!(stroke_scale(&with("strokeWidth", Value::from(1))) < 1.0);
        assert!(stroke_scale(&with("strokeWidth", Value::from(3))) > 1.0);
    }

    #[test]
    fn a_text_is_named_by_its_role_and_the_primary_one_is_no_key() {
        let mut extra = Map::new();
        set_font_role(&mut extra, Some(1));
        assert_eq!(extra.get("font").and_then(Value::as_str), Some("secondary"));
        assert_eq!(font_role(&extra), Some(1));

        set_font_role(&mut extra, Some(PRIMARY_TEXT));
        assert!(extra.is_empty(), "{extra:?}");
        // A role the space does not have, and one a hand-edited file invented.
        set_font_role(&mut extra, Some(9));
        assert!(extra.is_empty(), "{extra:?}");
        assert_eq!(font_role(&with("font", Value::from("quaternary"))), None);
        assert_eq!(font_role(&with("font", Value::from(2))), None);
    }

    #[test]
    fn a_default_colour_and_weight_leave_no_key_behind() {
        let mut extra = Map::new();
        set_outline_color(&mut extra, Some("3"));
        set_text_color(&mut extra, Some("#1a2b3c"));
        set_stroke_width(&mut extra, 3);
        assert_eq!(outline_color(&extra), Some("3"));
        assert_eq!(text_color(&extra), Some("#1a2b3c"));
        assert_eq!(stroke_width(&extra), Some(3));

        set_outline_color(&mut extra, None);
        set_text_color(&mut extra, None);
        set_stroke_width(&mut extra, MID_STROKE);
        assert!(extra.is_empty(), "{extra:?}");

        // Junk is a clear rather than a key nothing can read.
        set_outline_color(&mut extra, Some(""));
        set_text_color(&mut extra, Some(""));
        set_stroke_width(&mut extra, 42);
        assert!(extra.is_empty(), "{extra:?}");
    }

    // A text node's content lives under `text`, so a colour must not.
    #[test]
    fn a_text_colour_never_touches_a_nodes_own_text() {
        let mut extra = Map::from_iter([("text".to_owned(), Value::from("# hello"))]);
        set_text_color(&mut extra, Some("5"));
        assert_eq!(extra.get("text").and_then(Value::as_str), Some("# hello"));
        assert_eq!(text_color(&extra), Some("5"));
    }

    #[test]
    fn left_is_no_key_and_the_other_two_round_trip() {
        let mut extra = Map::new();
        assert_eq!(align(&extra), None);

        set_align(&mut extra, Some(2));
        assert_eq!(
            extra.get("textAlign").and_then(Value::as_str),
            Some("right")
        );
        assert_eq!(align(&extra), Some(2));

        set_align(&mut extra, Some(LEFT_ALIGN));
        assert!(extra.is_empty(), "{extra:?}");
        assert_eq!(align(&with("textAlign", Value::from("sideways"))), None);
    }
}
