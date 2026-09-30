use bevy::input_focus::AutoFocus;
use bevy::prelude::*;
use bevy::text::{EditableText, TextCursorStyle};
use extboard_core::{Canvas, NodeKind};

use crate::client::Document;
use crate::edit::double_click;
use crate::node::{NodeId, NodeRect};
use crate::select::{bounds, cursor_world, pick};

use super::{BODY, FG, markdown};

#[derive(Resource, Default)]
pub struct Editing(pub Option<String>);

#[derive(Component)]
pub(super) struct Editor;

pub fn editing(editing: Res<Editing>) -> bool {
    editing.0.is_some()
}

// Source while editing, rendered at rest: the buffer is the node's raw markdown,
// and the styled span tree is never edited.
pub(super) fn editor(md: &str) -> impl Bundle {
    (
        Editor,
        // Fills the content box; the clip box above it does the clipping.
        Node {
            width: percent(100.0),
            height: percent(100.0),
            ..default()
        },
        EditableText {
            allow_newlines: true,
            // The node's own height, not a line count.
            visible_lines: None,
            ..EditableText::new(md)
        },
        TextLayout {
            linebreak: LineBreak::WordOrCharacter,
            ..default()
        },
        TextFont::from_font_size(BODY),
        TextColor(FG),
        // The default caret is slate, which on a coloured node rect is invisible.
        TextCursorStyle {
            color: FG,
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
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    nodes: Query<(&NodeId, &Transform, &NodeRect)>,
    editors: Query<&EditableText, With<Editor>>,
    mut document: ResMut<Document>,
    mut editing: ResMut<Editing>,
    mut last: Local<Option<(f32, Vec2)>>,
) {
    let world = cursor_world(&window, *camera);

    if let Some(id) = editing.0.clone() {
        let away = buttons.just_pressed(MouseButton::Left)
            && !world.is_some_and(|world| {
                node_rect(&nodes, &id).is_some_and(|rect| rect.contains(world))
            });
        if !away && !keys.just_pressed(KeyCode::Escape) {
            return;
        }
        // One document mutation per session, and only if something was typed:
        // waking the document respawns every node and panel.
        if let Ok(editor) = editors.single()
            && written_back(
                &mut document.bypass_change_detection().0,
                &id,
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
    let (Some(world), Some(screen)) = (world, window.cursor_position()) else {
        return;
    };
    if !double_click(&mut last, time.elapsed_secs(), screen) {
        return;
    }
    let Some(id) = pick(
        nodes
            .iter()
            .map(|(id, transform, rect)| (&id.0, bounds(transform, rect), transform.translation.z)),
        world,
    ) else {
        return;
    };
    // Only a text node has markdown to edit.
    if document
        .0
        .nodes
        .iter()
        .any(|node| &node.id == id && markdown(node).is_some())
    {
        editing.0 = Some(id.clone());
    }
}

fn node_rect(nodes: &Query<(&NodeId, &Transform, &NodeRect)>, id: &str) -> Option<Rect> {
    nodes
        .iter()
        .find(|(node, _, _)| node.0 == id)
        .map(|(_, transform, rect)| bounds(transform, rect))
}

// `true` when the buffer differed, so only a real edit wakes the document.
pub(super) fn written_back(canvas: &mut Canvas, id: &str, text: &str) -> bool {
    let Some(node) = canvas.nodes.iter_mut().find(|node| node.id == id) else {
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
