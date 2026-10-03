use bevy::input_focus::{AutoFocus, InputFocus};
use bevy::prelude::*;
use bevy::text::{EditableText, TextCursorStyle, TextEdit};
use extboard_core::{BACKGROUND, Canvas, NodeKind, TEXT};

use crate::camera::world_to_screen;
use crate::client::Document;
use crate::edit::double_click;
use crate::node::{NodeId, NodeRect};
use crate::scene::{EdgeId, nearest};
use crate::select::{EDGE_PX, bounds, cursor_world, pick};
use crate::theme::Theme;

use super::{BODY, WELL, markdown};

// The label editor is a fixed screen-sized box on the edge's midpoint: a label
// is one short line, and it stays legible at every zoom.
const LABEL_BOX: Vec2 = Vec2::new(180.0, 28.0);

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

#[derive(Component)]
pub(super) struct LabelBox;

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

    pub(super) fn edge(&self) -> Option<&str> {
        match &self.0 {
            Some(Target::Edge(id)) => Some(id),
            _ => None,
        }
    }
}

// One line on a solid ground, over the label it is replacing.
pub(super) fn label_editor(label: &str, theme: &Theme) -> impl Bundle {
    let fg = theme.color(TEXT);
    (
        Editor,
        LabelBox,
        Node {
            position_type: PositionType::Absolute,
            width: px(LABEL_BOX.x),
            padding: UiRect::all(px(4.0)),
            ..default()
        },
        BackgroundColor(theme.color(BACKGROUND).with_alpha(WELL)),
        EditableText::new(label),
        TextFont::from_font_size(BODY),
        TextColor(fg),
        TextCursorStyle {
            color: fg,
            ..default()
        },
        AutoFocus,
    )
}

// The box rides the edge's midpoint, which scene.rs keeps current every frame.
pub(super) fn track_label(
    editing: Res<Editing>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    edges: Query<(&EdgeId, &Transform)>,
    mut boxes: Query<&mut Node, With<LabelBox>>,
) {
    let (camera, cam_global) = *camera;
    let Some(screen) = editing
        .edge()
        .and_then(|id| edges.iter().find(|(edge, _)| edge.0 == id))
        .and_then(|(_, transform)| {
            world_to_screen(camera, cam_global, transform.translation.truncate())
        })
    else {
        return;
    };
    let want = (
        px(screen.x - LABEL_BOX.x / 2.0),
        px(screen.y - LABEL_BOX.y / 2.0),
    );
    for mut node in &mut boxes {
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
// and the styled span tree is never edited.
pub(super) fn editor(md: &str, fg: Color) -> impl Bundle {
    let mut buffer = EditableText {
        allow_newlines: true,
        // The node's own height, not a line count.
        visible_lines: None,
        ..EditableText::new(md)
    };
    // Open at the top. `new` leaves the caret at the end, and the editor scrolls
    // to keep it in view, so a source taller than its node opens mid-text with
    // the caret's own width scrolled off the left.
    buffer.queue_edit(TextEdit::TextStart(false));

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
        TextFont::from_font_size(BODY),
        TextColor(fg),
        // The default caret is slate, which on a coloured node rect is invisible.
        TextCursorStyle {
            color: fg,
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
    edges: Query<(&EdgeId, &Transform)>,
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
        let away = buttons.just_pressed(MouseButton::Left)
            && !holds(
                &target,
                &nodes,
                &edges,
                (camera, cam_global),
                &window,
                world,
            );
        if !away && !keys.just_pressed(KeyCode::Escape) {
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
    editing.0 = opened(&document.0, &nodes, world, EDGE_PX * ortho.scale);
}

// A node under the cursor opens its markdown; failing that, an edge under it
// opens its label. Only a text node has markdown to edit.
fn opened(
    canvas: &Canvas,
    nodes: &Query<(&NodeId, &Transform, &NodeRect)>,
    world: Vec2,
    reach: f32,
) -> Option<Target> {
    let node = pick(
        nodes
            .iter()
            .map(|(id, transform, rect)| (&id.0, bounds(transform, rect), transform.translation.z)),
        world,
    );
    if let Some(id) = node {
        return canvas
            .nodes
            .iter()
            .any(|node| &node.id == id && markdown(node).is_some())
            .then(|| Target::Node(id.clone()));
    }
    nearest(canvas, world, reach).map(|id| Target::Edge(id.to_owned()))
}

// Whether the press is still inside what is being edited: the node's rect, or
// the label box, which is screen-sized and so measured in screen pixels.
fn holds(
    target: &Target,
    nodes: &Query<(&NodeId, &Transform, &NodeRect)>,
    edges: &Query<(&EdgeId, &Transform)>,
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
                    .find(|(edge, _)| edge.0 == *id)
                    .and_then(|(_, transform)| {
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
