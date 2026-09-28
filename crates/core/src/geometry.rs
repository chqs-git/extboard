//! Where an edge meets a node, in canvas coordinates (+y down). Shared because
//! the Bevy scene and the phone view have to land an edge in the same place.

use crate::{Node, Side};

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
}
