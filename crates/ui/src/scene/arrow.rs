use bevy::prelude::*;

const HEAD_LEN: f32 = 18.0;
const HEAD_HALF_WIDTH: f32 = 8.0;
// Clear space around a label where the shaft would otherwise run through it.
const LABEL_PAD: f32 = 6.0;

pub fn draw_arrow(
    gizmos: &mut Gizmos,
    a: Vec2,
    b: Vec2,
    heads: (bool, bool),
    hole: Option<Vec2>,
    color: Color,
) {
    // Stop the shaft at each head's base: a line through an outlined triangle
    // does not read as an arrow.
    let dir = (b - a).normalize_or_zero();
    let shaft_a = if heads.0 { a + dir * HEAD_LEN } else { a };
    let shaft_b = if heads.1 { b - dir * HEAD_LEN } else { b };

    match hole {
        Some(size) => {
            let middle = a.midpoint(b);
            let gap = dir * box_reach(size / 2.0 + Vec2::splat(LABEL_PAD), dir);
            // A label wider than its edge leaves no shaft to draw, only heads.
            for (start, end) in [(shaft_a, middle - gap), (middle + gap, shaft_b)] {
                if (end - start).dot(dir) > 0.0 {
                    gizmos.line_2d(start, end, color);
                }
            }
        }
        None => gizmos.line_2d(shaft_a, shaft_b, color),
    }

    if heads.1 {
        arrow_head(gizmos, b, dir, color);
    }
    if heads.0 {
        arrow_head(gizmos, a, -dir, color);
    }
}

// How far the edge of a box with these half-extents is from its centre, along
// `dir`: the shaft stops there rather than a fixed distance out, so a wide label
// and a tall one both get the room they need.
pub(super) fn box_reach(half: Vec2, dir: Vec2) -> f32 {
    let axis = |half: f32, d: f32| {
        if d.abs() > f32::EPSILON {
            (half / d).abs()
        } else {
            f32::INFINITY
        }
    };
    axis(half.x, dir.x).min(axis(half.y, dir.y))
}

// Closed triangle, tip at `tip`, pointing along `dir`. Gizmos have no fill, so
// this is an outline; at the default 2px stroke it reads as solid.
fn arrow_head(gizmos: &mut Gizmos, tip: Vec2, dir: Vec2, color: Color) {
    let base = tip - dir * HEAD_LEN;
    let side = dir.perp() * HEAD_HALF_WIDTH;
    gizmos.linestrip_2d([tip, base + side, base - side, tip], color);
}
