//! Where an edge meets a node, in canvas coordinates (+y down). Shared because
//! the Bevy scene and the phone view have to land an edge in the same place.

use crate::{CIRCLE_SIDES, MIN_SIDES, Node, Side};

/// The side of `from` that naturally points at `to`, for an edge that did not
/// say. Normalised by the pair's half-extents: two tall nodes side by side
/// connect left-to-right even when their centres are further apart vertically.
pub fn side_facing(from: &Node, to: &Node) -> Side {
    let dx = ((to.x + to.width / 2) - (from.x + from.width / 2)) as f32;
    let dy = ((to.y + to.height / 2) - (from.y + from.height / 2)) as f32;

    let hx = (from.width + to.width) as f32 / 2.0;
    let hy = (from.height + to.height) as f32 / 2.0;

    if (dx / hx).abs() >= (dy / hy).abs() {
        if dx >= 0.0 { Side::Right } else { Side::Left }
    } else if dy >= 0.0 {
        Side::Bottom
    } else {
        Side::Top
    }
}

/// The midpoint of one side of a node's rect.
pub fn side_anchor(node: &Node, side: Side) -> (f32, f32) {
    let (x, y) = (node.x as f32, node.y as f32);
    let (w, h) = (node.width as f32, node.height as f32);
    match side {
        Side::Top => (x + w / 2.0, y),
        Side::Bottom => (x + w / 2.0, y + h),
        Side::Left => (x, y + h / 2.0),
        Side::Right => (x + w, y + h / 2.0),
    }
}

pub fn sides_polygon(sides: u8) -> Vec<(f32, f32)> {
    let sides = sides.clamp(MIN_SIDES, CIRCLE_SIDES);
    let turn = std::f32::consts::TAU / f32::from(sides);
    // Even sides get a half-step turn, so a square sits flat.
    let start = std::f32::consts::FRAC_PI_2
        + if sides.is_multiple_of(2) {
            turn / 2.0
        } else {
            0.0
        };

    let corners: Vec<(f32, f32)> = (0..sides)
        .map(|i| {
            let (sin, cos) = (start + turn * f32::from(i)).sin_cos();
            (cos, -sin)
        })
        .collect();

    // End to end rather than scaled about the centre: a triangle is not
    // symmetric, and scaling alone leaves its base hanging inside the box.
    let fit = |axis: fn(&(f32, f32)) -> f32| {
        let low = corners.iter().map(axis).fold(f32::MAX, f32::min);
        let high = corners.iter().map(axis).fold(f32::MIN, f32::max);
        move |value: f32| (value - low) / (high - low) - 0.5
    };
    let (across, down) = (fit(|c| c.0), fit(|c| c.1));

    corners.iter().map(|c| (across(c.0), down(c.1))).collect()
}

// The fraction of the box given up on each side, so content clears the sloped
// edges. Zero for four sides, which is the box.
pub fn sides_inset(sides: Option<u8>) -> f32 {
    let Some(sides) = sides else {
        return 0.0;
    };
    let polygon = sides_polygon(sides);
    let mut half = 0.5f32;
    for (a, b) in polygon.iter().zip(polygon.iter().cycle().skip(1)) {
        let normal = (b.1 - a.1, a.0 - b.0);
        let reach = normal.0 * a.0 + normal.1 * a.1;
        let (normal, reach) = if reach < 0.0 {
            ((-normal.0, -normal.1), -reach)
        } else {
            (normal, reach)
        };
        let corner = normal.0.abs() + normal.1.abs();
        if corner > 0.0 {
            half = half.min(reach / corner);
        }
    }
    (0.5 - half).max(0.0)
}

/// Both ends of an edge, each side inferred when the edge left it out.
pub fn edge_ends(
    from: &Node,
    from_side: Option<Side>,
    to: &Node,
    to_side: Option<Side>,
) -> ((f32, f32), (f32, f32)) {
    (
        side_anchor(from, from_side.unwrap_or_else(|| side_facing(from, to))),
        side_anchor(to, to_side.unwrap_or_else(|| side_facing(to, from))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NodeKind;

    fn node(x: i64, y: i64, width: i64, height: i64) -> Node {
        Node {
            id: "n".to_owned(),
            x,
            y,
            width,
            height,
            color: None,
            sides: None,
            kind: NodeKind::Text {
                text: String::new(),
            },
            extra: Default::default(),
        }
    }

    #[test]
    fn faces_the_other_node() {
        let left = node(0, 0, 100, 100);
        let right = node(300, 0, 100, 100);
        assert_eq!(side_facing(&left, &right), Side::Right);
        assert_eq!(side_facing(&right, &left), Side::Left);

        // Canvas +y is down, so the node with the larger y is *below*.
        let above = node(0, 0, 100, 100);
        let below = node(0, 300, 100, 100);
        assert_eq!(side_facing(&above, &below), Side::Bottom);
        assert_eq!(side_facing(&below, &above), Side::Top);
    }

    #[test]
    fn tall_nodes_still_connect_sideways() {
        // dy (150) is larger than dx (100), so a raw |dx| vs |dy| test says
        // vertical. These two overlap for their whole height; the natural
        // connection is left-to-right.
        let a = node(0, 0, 40, 400);
        let b = node(100, 150, 40, 400);
        assert_eq!(side_facing(&a, &b), Side::Right);
        assert_eq!(side_facing(&b, &a), Side::Left);
    }

    #[test]
    fn anchors_sit_on_the_rect() {
        let n = node(0, 0, 100, 50);
        assert_eq!(side_anchor(&n, Side::Right), (100.0, 25.0));
        assert_eq!(side_anchor(&n, Side::Left), (0.0, 25.0));
        assert_eq!(side_anchor(&n, Side::Top), (50.0, 0.0));
        assert_eq!(side_anchor(&n, Side::Bottom), (50.0, 50.0));
    }

    #[test]
    fn a_specified_side_wins_over_the_inferred_one() {
        let a = node(0, 0, 100, 100);
        let b = node(300, 0, 100, 100);
        let (from, to) = edge_ends(&a, Some(Side::Top), &b, None);
        assert_eq!(from, side_anchor(&a, Side::Top));
        assert_eq!(to, side_anchor(&b, Side::Left));
    }

    #[test]
    fn a_polygon_fills_the_node_box() {
        let square = sides_polygon(4);
        for corner in [(0.5, 0.5), (0.5, -0.5), (-0.5, 0.5), (-0.5, -0.5)] {
            assert!(
                square
                    .iter()
                    .any(|v| (v.0 - corner.0).abs() < 1e-5 && (v.1 - corner.1).abs() < 1e-5),
                "{corner:?} is not a vertex of {square:?}"
            );
        }
        let triangle = sides_polygon(3);
        assert!((triangle[0].1 + 0.5).abs() < 1e-5, "{triangle:?}");
        for sides in MIN_SIDES..=CIRCLE_SIDES {
            let polygon = sides_polygon(sides);
            for axis in [|v: &(f32, f32)| v.0, |v: &(f32, f32)| v.1] {
                let low = polygon.iter().map(axis).fold(f32::MAX, f32::min);
                let high = polygon.iter().map(axis).fold(f32::MIN, f32::max);
                assert!(
                    (low + 0.5).abs() < 1e-5 && (high - 0.5).abs() < 1e-5,
                    "{sides} sides reach {low} to {high}: {polygon:?}"
                );
            }
        }
    }

    #[test]
    fn the_content_inset_is_what_the_sloped_edges_cost() {
        assert_eq!(sides_inset(None), 0.0);
        assert!(sides_inset(Some(4)) < 1e-5, "a square is the whole box");
        let insets: Vec<f32> = (MIN_SIDES..=CIRCLE_SIDES)
            .map(|n| sides_inset(Some(n)))
            .collect();
        assert!(insets.iter().all(|inset| (0.0..0.5).contains(inset)));
        assert_eq!(
            insets.iter().copied().fold(f32::MIN, f32::max).to_bits(),
            insets[0].to_bits(),
            "{insets:?}"
        );
    }
}
