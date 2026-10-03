use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use extboard_core::{Canvas, NodeKind, fresh_id};

use crate::client::Document;
use crate::node::{NodeId, NodeKind as Kind, NodeRect};
use crate::scene::EdgeId;
use crate::select::{Selected, bounds};

use super::{GRID, added, moved, with_contents};

// Room around the selection a new group gets, so its contents are not flush
// against its own outline.
const GROUP_PAD: f32 = 24.0;

pub fn command(keys: &ButtonInput<KeyCode>) -> bool {
    keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ])
}

pub(super) fn delete(
    keys: Res<ButtonInput<KeyCode>>,
    selected: Query<&NodeId, With<Selected>>,
    edges: Query<&EdgeId, With<Selected>>,
    mut document: ResMut<Document>,
) {
    // Mac's delete key is Backspace.
    if (selected.is_empty() && edges.is_empty())
        || !keys.any_just_pressed([KeyCode::Delete, KeyCode::Backspace])
    {
        return;
    }
    let canvas = &mut document.0;
    for id in &selected {
        // Cascades the node's edges; a stale selection entity is not an error.
        let _ = canvas.remove_node(&id.0);
    }
    // After the nodes: a cascade may already have taken this edge with it.
    for id in &edges {
        let _ = canvas.remove_edge(&id.0);
    }
}

// Arrow keys move the selection a pixel at a time, a grid step with shift.
#[allow(
    clippy::too_many_arguments,
    reason = "a system's arguments are its query"
)]
pub(super) fn nudge(
    keys: Res<ButtonInput<KeyCode>>,
    nodes: Query<(Entity, &Transform, &NodeRect)>,
    kinds: Query<&Kind>,
    ids: Query<&NodeId>,
    selected: Query<Entity, With<Selected>>,
    mut document: ResMut<Document>,
) {
    let Some(step) = step(&keys) else {
        return;
    };
    // Bypassed like a drag: respawning every node would drop the selection, and
    // a nudge is meant to be repeatable.
    let canvas = &mut document.bypass_change_detection().0;
    for entity in with_contents(selected.iter().collect(), &nodes, &kinds) {
        if let (Ok(id), Ok((_, transform, _))) = (ids.get(entity), nodes.get(entity)) {
            moved(canvas, &id.0, transform.translation.truncate() + step, 1.0);
        }
    }
}

// World space, so up is +y and the flip into canvas coordinates is `moved`'s.
fn step(keys: &ButtonInput<KeyCode>) -> Option<Vec2> {
    let far = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let step = if far { GRID } else { 1.0 };
    let arrow = match () {
        _ if keys.just_pressed(KeyCode::ArrowLeft) => -Vec2::X,
        _ if keys.just_pressed(KeyCode::ArrowRight) => Vec2::X,
        _ if keys.just_pressed(KeyCode::ArrowUp) => Vec2::Y,
        _ if keys.just_pressed(KeyCode::ArrowDown) => -Vec2::Y,
        _ => return None,
    };
    Some(arrow * step)
}

pub(super) fn duplicate(
    keys: Res<ButtonInput<KeyCode>>,
    frames: Res<FrameCount>,
    selected: Query<&NodeId, With<Selected>>,
    mut document: ResMut<Document>,
) {
    if !command(&keys) || !keys.just_pressed(KeyCode::KeyD) || selected.is_empty() {
        return;
    }
    let ids: Vec<&str> = selected.iter().map(|id| id.0.as_str()).collect();
    duplicated(&mut document.0, frames.0, &ids);
}

// Offset by a grid step, so the copy is visibly its own node rather than exactly
// over the original. Edges between the copies are not carried over.
pub fn duplicated(canvas: &mut Canvas, seed: u32, ids: &[&str]) -> Vec<String> {
    let mut copies: Vec<_> = canvas
        .nodes
        .iter()
        .filter(|node| ids.contains(&node.id.as_str()))
        .cloned()
        .collect();

    copies
        .iter_mut()
        .enumerate()
        .map(|(index, node)| {
            // One id at a time: `fresh_id` reads the canvas, so each copy has to
            // be in it before the next is named.
            node.id = fresh_id(canvas, &(seed.wrapping_add(index as u32)).to_le_bytes());
            (node.x, node.y) = (node.x + GRID as i64, node.y + GRID as i64);
            canvas
                .add_node(node.clone())
                .expect("fresh_id never collides");
            node.id.clone()
        })
        .collect()
}

pub(super) fn group(
    keys: Res<ButtonInput<KeyCode>>,
    frames: Res<FrameCount>,
    selected: Query<(&Transform, &NodeRect), With<Selected>>,
    mut document: ResMut<Document>,
) {
    if !command(&keys) || !keys.just_pressed(KeyCode::KeyG) || selected.is_empty() {
        return;
    }
    let rects: Vec<Rect> = selected
        .iter()
        .map(|(transform, rect)| bounds(transform, rect))
        .collect();
    grouped(&mut document.0, frames.0, &rects);
}

// The selection's bounds with room to breathe. It goes in unnamed: the label is
// the spec's, and there is nowhere in the GUI to type one yet.
pub(super) fn grouped(canvas: &mut Canvas, seed: u32, rects: &[Rect]) -> Option<String> {
    let bounds = rects
        .iter()
        .copied()
        .reduce(|a, b| a.union(b))?
        .inflate(GROUP_PAD);
    let id = added(canvas, seed, bounds, NodeKind::Group { label: None });
    // A group is a container: it belongs behind what it holds, and `added`
    // appends, which is the front.
    canvas.nodes.rotate_right(1);
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::created;

    fn canvas() -> Canvas {
        serde_json::from_str(r#"{"nodes":[],"edges":[]}"#).expect("fixture")
    }

    fn rect(x: f32, y: f32) -> Rect {
        Rect::from_center_size(Vec2::new(x, y), Vec2::splat(100.0))
    }

    #[test]
    fn a_copy_is_a_new_id_offset_from_the_original() {
        let mut canvas = canvas();
        let first = created(&mut canvas, 1, rect(0.0, 0.0));
        let second = created(&mut canvas, 2, rect(400.0, 0.0));
        let before = canvas.nodes.clone();

        let copies = duplicated(&mut canvas, 3, &[&first, &second]);
        assert_eq!(copies.len(), 2);
        assert_eq!(canvas.nodes.len(), 4);
        // The end of the array is the front (PLAN.md §8): a copy lands over the
        // node it came from, never under it.
        let order: Vec<&str> = canvas.nodes.iter().map(|node| node.id.as_str()).collect();
        assert_eq!(order[2..], copies[..]);
        // Distinct from each other and from what they were copied from.
        assert_ne!(copies[0], copies[1]);
        assert!(!copies.contains(&first) && !copies.contains(&second));

        for (original, copy) in before.iter().zip(&copies) {
            let copy = canvas.nodes.iter().find(|node| &node.id == copy).unwrap();
            assert_eq!(
                (copy.x - original.x, copy.y - original.y),
                (GRID as i64, GRID as i64)
            );
            assert_eq!((copy.width, copy.height), (original.width, original.height));
        }
    }

    #[test]
    fn a_group_wraps_the_whole_selection_with_room_around_it() {
        let mut canvas = canvas();
        let rects = [rect(0.0, 0.0), rect(400.0, -200.0)];
        let id = grouped(&mut canvas, 1, &rects).expect("a group");

        let group = canvas.nodes.iter().find(|node| node.id == id).unwrap();
        assert!(matches!(group.kind, NodeKind::Group { label: None }));
        // Canvas y runs down, so the world rect's high y is the group's top.
        let bounds = Rect::from_corners(
            Vec2::new(group.x as f32, -group.y as f32),
            Vec2::new(
                (group.x + group.width) as f32,
                -(group.y + group.height) as f32,
            ),
        );
        for rect in rects {
            assert!(
                bounds.contains(rect.min) && bounds.contains(rect.max),
                "{bounds:?}"
            );
        }
        assert!(bounds.width() > rects[0].union(rects[1]).width());
    }

    #[test]
    fn grouping_nothing_makes_nothing() {
        let mut canvas = canvas();
        assert!(grouped(&mut canvas, 1, &[]).is_none());
        assert!(canvas.nodes.is_empty());
    }

    fn held(keys: &[KeyCode]) -> ButtonInput<KeyCode> {
        let mut input = ButtonInput::default();
        for key in keys {
            input.press(*key);
        }
        input
    }

    #[test]
    fn shift_nudges_a_whole_grid_step_and_a_bare_arrow_a_pixel() {
        assert_eq!(step(&held(&[])), None);
        assert_eq!(step(&held(&[KeyCode::ArrowRight])), Some(Vec2::X));
        assert_eq!(
            step(&held(&[KeyCode::ArrowRight, KeyCode::ShiftLeft])),
            Some(Vec2::X * GRID)
        );
        // World +y is up; `moved` is what flips it into the canvas.
        assert_eq!(step(&held(&[KeyCode::ArrowUp])), Some(Vec2::Y));
        assert_eq!(step(&held(&[KeyCode::ArrowDown])), Some(-Vec2::Y));
    }
}
