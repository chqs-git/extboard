use bevy::clipboard::{Clipboard, ClipboardRead};
use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use extboard_core::{Canvas, Node as CanvasNode, NodeKind, fresh_id};

use crate::client::Document;
use crate::node::{NodeId, NodeKind as Kind, NodeRect, to_world};
use crate::scene::EdgeId;
use crate::select::{Selected, bounds, cursor_world};

use super::drop::Uploads;
use super::{GRID, added, moved, with_contents};

// Room around the selection a new group gets, so its contents are not flush
// against its own outline.
const GROUP_PAD: f32 = 24.0;

#[cfg(not(target_arch = "wasm32"))]
const PASTED: &str = "pasted.png";

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

#[derive(Resource, Default)]
pub(super) struct Pasting(Option<(ClipboardRead, Vec2)>);

pub(super) fn copy(
    keys: Res<ButtonInput<KeyCode>>,
    selected: Query<&NodeId, With<Selected>>,
    document: Res<Document>,
    mut clipboard: ResMut<Clipboard>,
) {
    if !command(&keys) || !keys.just_pressed(KeyCode::KeyC) || selected.is_empty() {
        return;
    }
    let ids: Vec<&str> = selected.iter().map(|id| id.0.as_str()).collect();
    match copied(&document.0, &ids) {
        Ok(json) => {
            if let Err(e) = clipboard.set_text(json) {
                warn!("clipboard: {e}");
            }
        }
        Err(e) => warn!("copy: {e}"),
    }
}

pub fn copied(canvas: &Canvas, ids: &[&str]) -> Result<String, serde_json::Error> {
    let nodes: Vec<&CanvasNode> = canvas
        .nodes
        .iter()
        .filter(|node| ids.contains(&node.id.as_str()))
        .collect();
    serde_json::to_string_pretty(&nodes)
}

pub(super) fn paste(
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    uploads: Res<Uploads>,
    mut clipboard: ResMut<Clipboard>,
    mut pasting: ResMut<Pasting>,
) {
    if !command(&keys) || !keys.just_pressed(KeyCode::KeyV) {
        return;
    }
    if took_an_image(&mut clipboard, &uploads, window.cursor_position()) {
        return;
    }
    let Some(world) = cursor_world(&window, *camera) else {
        return;
    };
    pasting.0 = Some((clipboard.fetch_text(), world));
}

#[cfg(not(target_arch = "wasm32"))]
fn took_an_image(clipboard: &mut Clipboard, uploads: &Uploads, screen: Option<Vec2>) -> bool {
    let Ok(image) = clipboard.fetch_image() else {
        return false;
    };
    match png_bytes(image) {
        Ok(bytes) => super::drop::upload(uploads, PASTED, bytes, screen),
        Err(e) => warn!("paste: {e}"),
    }
    true
}

// The tab reads a pasted image off its own `paste` event instead: arboard, which
// is what `fetch_image` reads, is not built for wasm. See drop.rs.
#[cfg(target_arch = "wasm32")]
fn took_an_image(_: &mut Clipboard, _: &Uploads, _: Option<Vec2>) -> bool {
    false
}

#[cfg(not(target_arch = "wasm32"))]
fn png_bytes(image: Image) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    image
        .try_into_dynamic()
        .map_err(|e| e.to_string())?
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

pub(super) fn pasted_nodes(
    frames: Res<FrameCount>,
    mut pasting: ResMut<Pasting>,
    mut document: ResMut<Document>,
) {
    let Some((read, world)) = pasting.0.as_mut() else {
        return;
    };
    let (text, world) = (read.poll_result(), *world);
    let Some(text) = text else {
        return;
    };
    pasting.0 = None;
    match text {
        Ok(text) => {
            if pasted(&mut document.0, frames.0, &text, world).is_empty() {
                info!("paste: the clipboard holds no canvas nodes");
            }
        }
        Err(e) => warn!("paste: {e}"),
    }
}

pub fn pasted(canvas: &mut Canvas, seed: u32, json: &str, at: Vec2) -> Vec<String> {
    let mut nodes = match parsed(json) {
        Some(nodes) if !nodes.is_empty() => nodes,
        _ => return Vec::new(),
    };
    let cursor = to_world(at);
    let min = nodes.iter().fold(Vec2::MAX, |min, node| {
        min.min(Vec2::new(node.x as f32, node.y as f32))
    });
    let max = nodes.iter().fold(Vec2::MIN, |max, node| {
        max.max(Vec2::new(
            (node.x + node.width) as f32,
            (node.y + node.height) as f32,
        ))
    });
    let delta = cursor - (min + max) / 2.0;

    nodes
        .iter_mut()
        .enumerate()
        .map(|(index, node)| {
            node.id = fresh_id(canvas, &(seed.wrapping_add(index as u32)).to_le_bytes());
            node.x += delta.x.round() as i64;
            node.y += delta.y.round() as i64;
            canvas
                .add_node(node.clone())
                .expect("fresh_id never collides");
            node.id.clone()
        })
        .collect()
}

fn parsed(json: &str) -> Option<Vec<CanvasNode>> {
    serde_json::from_str::<Vec<CanvasNode>>(json)
        .or_else(|_| serde_json::from_str::<CanvasNode>(json).map(|node| vec![node]))
        .ok()
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
    fn a_paste_lands_centred_on_the_cursor_under_fresh_ids() {
        let mut canvas = canvas();
        let first = created(&mut canvas, 1, rect(0.0, 0.0));
        let second = created(&mut canvas, 2, rect(400.0, 0.0));
        let json = copied(&canvas, &[&first, &second]).expect("nodes serialize");

        let at = Vec2::new(1000.0, -300.0);
        let ids = pasted(&mut canvas, 3, &json, at);
        assert_eq!(ids.len(), 2);
        assert_eq!(canvas.nodes.len(), 4);
        assert!(!ids.contains(&first) && !ids.contains(&second));

        let box_of = |ids: &[String]| {
            ids.iter().fold((Vec2::MAX, Vec2::MIN), |(min, max), id| {
                let node = canvas.nodes.iter().find(|node| &node.id == id).unwrap();
                (
                    min.min(Vec2::new(node.x as f32, node.y as f32)),
                    max.max(Vec2::new(
                        (node.x + node.width) as f32,
                        (node.y + node.height) as f32,
                    )),
                )
            })
        };
        let (min, max) = box_of(&ids);
        assert_eq!((min + max) / 2.0, Vec2::new(1000.0, 300.0));
        let (was_min, was_max) = box_of(&[first, second]);
        assert_eq!(max - min, was_max - was_min);
    }

    #[test]
    fn a_paste_takes_one_node_or_a_whole_array_and_ignores_anything_else() {
        let mut canvas = canvas();
        let one = created(&mut canvas, 1, rect(0.0, 0.0));
        let object =
            serde_json::to_string(canvas.nodes.iter().find(|node| node.id == one).unwrap())
                .expect("a node serializes");
        assert_eq!(pasted(&mut canvas, 2, &object, Vec2::ZERO).len(), 1);

        for junk in ["", "not json", "[]", "{}", "[1,2,3]", r#""a string""#] {
            assert!(
                pasted(&mut canvas, 3, junk, Vec2::ZERO).is_empty(),
                "{junk:?} pasted something"
            );
        }
        assert_eq!(canvas.nodes.len(), 2, "and nothing was added either");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_clipboard_image_is_encoded_as_a_png_and_a_bad_one_says_so() {
        use bevy::asset::RenderAssetUsages;
        use bevy::image::Image;
        use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

        let size = Extent3d {
            width: 2,
            height: 2,
            depth_or_array_layers: 1,
        };
        let rgba = Image::new(
            size,
            TextureDimension::D2,
            vec![255; 2 * 2 * 4],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        let png = png_bytes(rgba).expect("rgba encodes");
        assert_eq!(
            &png[..8],
            b"\x89PNG\r\n\x1a\n",
            "not a png: {:?}",
            &png[..8]
        );
        assert!(extboard_core::is_image(PASTED));

        let float = Image::new(
            size,
            TextureDimension::D2,
            vec![0; 2 * 2 * 16],
            TextureFormat::Rgba32Float,
            RenderAssetUsages::default(),
        );
        assert!(png_bytes(float).is_err());
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
