use bevy::input_focus::{AutoFocus, InputFocus};
use bevy::prelude::*;
use bevy::text::{EditableText, FontSize, TextCursorStyle, TextEdit, TextLayoutInfo};
use extboard_core::{Canvas, NodeKind};

use crate::camera::world_to_screen;
use crate::client::Document;
use crate::edit::{command, double_click};
use crate::node::{NodeId, NodeRect};
use crate::scene::{EdgeId, LABEL_SIZE, label_box, nearest};
use crate::select::{EDGE_PX, bounds, cursor_world, pick};
use crate::theme::Face;

use super::{BODY, markdown};

// How far from the midpoint a click still counts as being in the label, and the
// width the one line it holds is centred in.
const LABEL_BOX: Vec2 = Vec2::new(180.0, 28.0);

// bevy's own default, and what the line the editor lays out will measure.
pub(super) const LINE_HEIGHT: f32 = 1.2;

// The label is drawn in the world, so the zoom is the size it reads on screen.
// Whole pixels: the glyph atlas keeps one set per size, and a ramp through the
// fractions between 13 and 14 buys nothing anybody can see.
pub(super) fn label_size(zoom: f32) -> f32 {
    (LABEL_SIZE * zoom).round().max(1.0)
}

// The editor's box: one line tall, so it lands over the label rather than near
// it, in a width the line is centred in.
fn label_half(size: f32) -> Vec2 {
    Vec2::new(LABEL_BOX.x, size * LINE_HEIGHT) / 2.0
}

#[derive(Clone, PartialEq, Debug)]
pub enum Target {
    Node(String),
    Edge(String),
    // One of the board's scripts, by name, in the sidebar. A target rather than a
    // mode of its own, so everything that already stands down for a text node —
    // the canvas undo stack included — stands down for it unchanged. It carries
    // the name so a blur always writes back the script that was typed into.
    Script(String),
}

#[derive(Resource, Default)]
pub struct Editing(pub Option<Target>);

#[derive(Component)]
pub(super) struct Editor;

// Half the editor's own box, so `track_label` can centre it on the midpoint
// without measuring a layout that is a frame behind.
#[derive(Component)]
pub(super) struct LabelBox(Vec2);

pub fn editing(editing: Res<Editing>) -> bool {
    editing.0.is_some()
}

impl Editing {
    pub(super) fn node(&self) -> Option<&str> {
        match &self.0 {
            Some(Target::Node(id)) => Some(id),
            _ => None,
        }
    }

    // `scene` reads it too: the drawn label stands down for the editor.
    pub fn edge(&self) -> Option<&str> {
        match &self.0 {
            Some(Target::Edge(id)) => Some(id),
            _ => None,
        }
    }
}

// The label typed where it is drawn: the drawn label's own size, so the text
// does not change under the caret, and no well or padding to read as a form
// over the edge. `track_label` keeps all of it on the camera.
pub(super) fn label_editor(label: &str, face: &Face, zoom: f32) -> impl Bundle {
    let size = label_size(zoom);
    let half = label_half(size);
    (
        Editor,
        LabelBox(half),
        Node {
            position_type: PositionType::Absolute,
            width: px(half.x * 2.0),
            height: px(half.y * 2.0),
            ..default()
        },
        EditableText::new(label),
        // Centred on the midpoint the label sits on, and unwrapped, so it grows
        // out of that point in both directions the way the drawn label does.
        TextLayout {
            justify: bevy::text::Justify::Center,
            linebreak: LineBreak::NoWrap,
        },
        TextFont {
            font: face.source.clone(),
            font_smoothing: face.smoothing,
            ..TextFont::from_font_size(size)
        },
        TextColor(face.ink),
        TextCursorStyle {
            color: face.ink,
            ..default()
        },
        AutoFocus,
    )
}

// The box rides the edge's midpoint, which scene.rs keeps current every frame,
// and the camera's zoom: a label the board scales away from is the same thing
// card 106 fixed for a node's text.
pub(super) fn track_label(
    editing: Res<Editing>,
    camera: Single<(&Camera, &GlobalTransform, &Projection), With<Camera2d>>,
    edges: Query<(&EdgeId, &Transform)>,
    mut boxes: Query<(&mut LabelBox, &mut Node, &mut TextFont)>,
) {
    let (camera, cam_global, projection) = *camera;
    let Projection::Orthographic(ortho) = projection else {
        return;
    };
    let size = label_size(1.0 / ortho.scale);
    let Some(screen) = editing
        .edge()
        .and_then(|id| edges.iter().find(|(edge, _)| edge.0 == id))
        .and_then(|(_, transform)| {
            world_to_screen(camera, cam_global, transform.translation.truncate())
        })
    else {
        return;
    };
    let half = label_half(size);
    for (mut box_, mut node, mut font) in &mut boxes {
        if box_.0 != half {
            box_.0 = half;
            (node.width, node.height) = (px(half.x * 2.0), px(half.y * 2.0));
            font.font_size = FontSize::Px(size);
        }
        let want = (px(screen.x - half.x), px(screen.y - half.y));
        if (node.left, node.top) != want {
            (node.left, node.top) = want;
        }
    }
}

// A box that takes no newline has no use for enter, and escape means "done"
// everywhere else in the editor: both let go of the caret. Without this a panel
// field keeps the keyboard until something else is clicked, and the canvas is
// deaf for as long as it does — the sides box and the theme boxes both.
pub(super) fn release_field(
    keys: Res<ButtonInput<KeyCode>>,
    fields: Query<&EditableText>,
    mut focus: ResMut<InputFocus>,
) {
    let Some(entity) = focus.get() else {
        return;
    };
    if !keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter, KeyCode::Escape]) {
        return;
    }
    if fields.get(entity).is_ok_and(|field| !field.allow_newlines) {
        focus.clear();
    }
}

// Source while editing, rendered at rest: the buffer is the node's raw markdown,
// and the styled span tree is never edited. `live` is the buffer of a session
// this respawn interrupts, which the document does not hold yet.
pub(super) fn editor(
    md: &str,
    face: &Face,
    raster: f32,
    live: Option<&EditableText>,
) -> impl Bundle {
    let buffer = match live {
        Some(live) => live.clone(),
        None => EditableText {
            allow_newlines: true,
            // The node's own height, not a line count.
            visible_lines: None,
            ..EditableText::new(md)
        },
    };

    (
        Editor,
        // Fills the content box; the clip box above it does the clipping.
        Node {
            width: percent(100.0),
            height: percent(100.0),
            ..default()
        },
        buffer,
        TextLayout {
            linebreak: LineBreak::WordOrCharacter,
            ..default()
        },
        TextFont {
            font: face.source.clone(),
            font_smoothing: face.smoothing,
            ..TextFont::from_font_size(BODY * raster)
        },
        TextColor(face.ink),
        // The default caret is slate, which on a coloured node rect is invisible.
        TextCursorStyle {
            color: face.ink,
            ..default()
        },
        AutoFocus,
    )
}

// Double-click a text node to edit it; escape or a click away ends the session
// and writes the buffer back.
#[allow(
    clippy::too_many_arguments,
    reason = "a system's arguments are its query"
)]
pub(super) fn toggle(
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform, &Projection), With<Camera2d>>,
    nodes: Query<(&NodeId, &Transform, &NodeRect)>,
    edges: Query<(&EdgeId, &Transform, Option<&TextLayoutInfo>)>,
    editors: Query<&EditableText, With<Editor>>,
    buffers: Query<&EditableText, With<crate::script::ScriptBuffer>>,
    sidebar: Res<crate::script::Sidebar>,
    mut document: ResMut<Document>,
    mut editing: ResMut<Editing>,
    mut last: Local<Option<(f32, Vec2)>>,
) {
    let (camera, cam_global, projection) = *camera;
    let world = cursor_world(&window, (camera, cam_global));
    let Projection::Orthographic(ortho) = projection else {
        return;
    };

    if let Some(target) = editing.0.clone() {
        // Space+left is the camera's pan grab, the same press the opening path
        // stands down for: panning a board is not a click away from the caret.
        let away = buttons.just_pressed(MouseButton::Left)
            && !keys.pressed(KeyCode::Space)
            && !holds(
                &target,
                &nodes,
                &edges,
                (camera, cam_global),
                &window,
                world,
            );
        // Enter finishes a one-line field, and a label is one; node text and a
        // script take newlines, so there it is a newline and nothing more.
        let entered = matches!(target, Target::Edge(_))
            && keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter]);
        if !away && !entered && !keys.just_pressed(KeyCode::Escape) {
            return;
        }
        // The sidebar's buffer is not a node's editor, and both can be alive at
        // once: the panel outlives a blur.
        let typed = match target {
            Target::Script(_) => buffers.single(),
            _ => editors.single(),
        };
        // One document mutation per session, and only if something was typed:
        // waking the document respawns every node and panel.
        if let Ok(editor) = typed
            && written_back(
                &mut document.bypass_change_detection().0,
                &target,
                &editor.value().to_string(),
            )
        {
            document.set_changed();
        }
        editing.0 = None;
        return;
    }

    if !buttons.just_pressed(MouseButton::Left) || keys.pressed(KeyCode::Space) {
        return;
    }
    // One click, because the panel is already open and asking to be typed in.
    // Here rather than in `script.rs`: this runs before anything reads the
    // press, so the click lands in the panel and not on the canvas under it.
    if sidebar.takes_caret() && crate::script::in_sidebar(&window, window.cursor_position()) {
        editing.0 = Some(Target::Script(sidebar.selected.clone()));
        return;
    }
    let (Some(world), Some(screen)) = (world, window.cursor_position()) else {
        return;
    };
    if !double_click(&mut last, time.elapsed_secs(), screen) {
        return;
    }
    editing.0 = opened(&document.0, &nodes, &edges, world, EDGE_PX * ortho.scale);
}

// A node under the cursor opens its markdown; failing that, an edge near it, or
// one whose label the cursor is in, opens its label. Only a text node has
// markdown to edit, and a node without it — a group around the edge, an image
// under it — does not swallow the click.
fn opened(
    canvas: &Canvas,
    nodes: &Query<(&NodeId, &Transform, &NodeRect)>,
    edges: &Query<(&EdgeId, &Transform, Option<&TextLayoutInfo>)>,
    world: Vec2,
    reach: f32,
) -> Option<Target> {
    let node = pick(
        nodes
            .iter()
            .map(|(id, transform, rect)| (&id.0, bounds(transform, rect), transform.translation.z)),
        world,
    );
    if let Some(id) = node.filter(|id| {
        canvas
            .nodes
            .iter()
            .any(|node| &node.id == *id && markdown(node).is_some())
    }) {
        return Some(Target::Node(id.clone()));
    }
    nearest(canvas, world, reach)
        .map(str::to_owned)
        .or_else(|| {
            edges.iter().find_map(|(edge, at, label)| {
                label
                    .filter(|label| label_box(at, label).contains(world))
                    .map(|_| edge.0.clone())
            })
        })
        .map(Target::Edge)
}

// Whether the press is still inside what is being edited: the node's rect, or
// the label box, which is screen-sized and so measured in screen pixels.
fn holds(
    target: &Target,
    nodes: &Query<(&NodeId, &Transform, &NodeRect)>,
    edges: &Query<(&EdgeId, &Transform, Option<&TextLayoutInfo>)>,
    camera: (&Camera, &GlobalTransform),
    window: &Window,
    world: Option<Vec2>,
) -> bool {
    match target {
        Target::Node(id) => {
            let Some(world) = world else {
                return false;
            };
            node_rect(nodes, id).is_some_and(|rect| rect.contains(world))
        }
        Target::Edge(id) => {
            let Some((cursor, at)) = window.cursor_position().zip(
                edges
                    .iter()
                    .find(|(edge, _, _)| edge.0 == *id)
                    .and_then(|(_, transform, _)| {
                        world_to_screen(camera.0, camera.1, transform.translation.truncate())
                    }),
            ) else {
                return false;
            };
            ((cursor - at).abs() * 2.0).cmple(LABEL_BOX).all()
        }
        Target::Script(_) => crate::script::in_sidebar(window, window.cursor_position()),
    }
}

fn node_rect(nodes: &Query<(&NodeId, &Transform, &NodeRect)>, id: &str) -> Option<Rect> {
    nodes
        .iter()
        .find(|(node, _, _)| node.0 == id)
        .map(|(_, transform, rect)| bounds(transform, rect))
}

// `true` when the buffer differed, so only a real edit wakes the document. An
// emptied label is dropped rather than written: the file keeps no empty strings.
pub(super) fn written_back(canvas: &mut Canvas, target: &Target, text: &str) -> bool {
    match target {
        Target::Node(id) => {
            let Some(node) = canvas.nodes.iter_mut().find(|node| node.id == *id) else {
                return false;
            };
            match &mut node.kind {
                NodeKind::Text { text: buffer } if buffer != text => {
                    *buffer = text.to_owned();
                    true
                }
                _ => false,
            }
        }
        Target::Edge(id) => {
            let Some(edge) = canvas.edges.iter_mut().find(|edge| edge.id == *id) else {
                return false;
            };
            let want = (!text.is_empty()).then(|| text.to_owned());
            if edge.label == want {
                return false;
            }
            edge.label = want;
            true
        }
        Target::Script(name) => crate::script::set_script(canvas, name, text),
    }
}

// bevy binds select-all to Cmd alone on a Mac, so Ctrl-A there did nothing.
pub(super) fn select_all(
    keys: Res<ButtonInput<KeyCode>>,
    focus: Res<InputFocus>,
    mut fields: Query<&mut EditableText>,
) {
    if !command(&keys) || !keys.just_pressed(KeyCode::KeyA) {
        return;
    }
    if let Some(mut field) = focus.get().and_then(|entity| fields.get_mut(entity).ok()) {
        field.queue_edit(TextEdit::SelectAll);
    }
}
