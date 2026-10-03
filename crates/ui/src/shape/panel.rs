use bevy::input_focus::InputFocus;
use bevy::prelude::*;
use bevy::text::{EditableText, TextCursorStyle, TextEdit};
use bevy::ui_widgets::{
    Slider, SliderRange, SliderStep, SliderThumb, SliderValue, TrackClick, ValueChange,
};
use extboard_core::{CIRCLE_SIDES, MIN_SIDES};

use crate::client::Document;
use crate::node::NodeId;
use crate::select::Selected;

use super::{Sides, set_sides};

// A fixed rect, which is also the whole hit test, as the sidebar does it.
const PANEL: Vec2 = Vec2::new(300.0, 44.0);
const BOTTOM: f32 = 24.0;
const TRACK: f32 = 150.0;
const RAIL: f32 = 4.0;
const THUMB: f32 = 14.0;
const FIELD: f32 = 52.0;
const LABEL_SIZE: f32 = 10.0;
const PANEL_BG: Color = Color::srgb(0.08, 0.09, 0.11);
const LABEL: Color = Color::srgb(0.5, 0.55, 0.62);
const FG: Color = Color::srgb(0.86, 0.88, 0.92);
const RAIL_BG: Color = Color::srgb(0.2, 0.22, 0.26);
const KNOB: Color = Color::srgb(0.95, 0.75, 0.30);
const FIELD_BG: Color = Color::srgb(0.06, 0.07, 0.09);

#[derive(Component)]
pub(crate) struct SidesPanel {
    node: String,
}

#[derive(Component)]
pub(super) struct SidesSlider;

#[derive(Component)]
pub(crate) struct SidesField;

// Not respawned while it holds the same node: that would drop the slider's
// grip on the pointer mid-drag.
#[allow(
    clippy::too_many_arguments,
    reason = "a system's arguments are its query"
)]
pub(super) fn sync(
    mut commands: Commands,
    document: Res<Document>,
    focus: Res<InputFocus>,
    selected: Query<&NodeId, With<Selected>>,
    panels: Query<(Entity, &SidesPanel)>,
    sliders: Query<(Entity, &SliderValue), With<SidesSlider>>,
    thumbs: Query<(Entity, &Node), With<SliderThumb>>,
    mut fields: Query<(Entity, &mut EditableText), With<SidesField>>,
) {
    let want = selected.single().ok().and_then(|id| {
        document
            .0
            .nodes
            .iter()
            .find(|node| node.id == id.0)
            .map(|node| (node.id.clone(), Sides::of(node.sides)))
    });
    let open = panels.single().ok();
    match (want, open) {
        (None, Some((entity, _))) => commands.entity(entity).despawn(),
        (Some((node, sides)), None) => spawn(&mut commands, node, sides),
        (Some((node, sides)), Some((entity, panel))) if panel.node != node => {
            commands.entity(entity).despawn();
            spawn(&mut commands, node, sides);
        }
        (Some((_, sides)), Some(_)) => {
            // The controls follow the document, not their own history. Every
            // write re-runs layout, so write only what moved.
            for (entity, value) in &sliders {
                if value.0 != sides.notch() {
                    commands.entity(entity).insert(SliderValue(sides.notch()));
                }
            }
            for (entity, node) in &thumbs {
                let want = thumb_at(sides);
                if node.left != want.left {
                    commands.entity(entity).insert(want);
                }
            }
            for (entity, mut field) in &mut fields {
                let label = sides.label();
                // Never while it is being typed into: the caret is mid-number.
                if focus.get() != Some(entity) && field.value().to_string() != label {
                    set_label(&mut field, &label);
                }
            }
        }
        (None, None) => {}
    }
}

fn spawn(commands: &mut Commands, node: String, sides: Sides) {
    commands
        .spawn((
            SidesPanel { node },
            GlobalZIndex(3),
            Node {
                position_type: PositionType::Absolute,
                left: percent(50.0),
                bottom: px(BOTTOM),
                margin: UiRect::left(px(-PANEL.x / 2.0)),
                width: px(PANEL.x),
                height: px(PANEL.y),
                padding: UiRect::horizontal(px(12.0)),
                column_gap: px(10.0),
                align_items: AlignItems::Center,
                border_radius: BorderRadius::all(px(8.0)),
                ..default()
            },
            BackgroundColor(PANEL_BG),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("sides"),
                TextFont::from_font_size(LABEL_SIZE),
                TextColor(LABEL),
            ));
            parent
                .spawn((
                    SidesSlider,
                    Slider {
                        track_click: TrackClick::Snap,
                        ..default()
                    },
                    SliderValue(sides.notch()),
                    range(),
                    SliderStep(1.0),
                    Node {
                        width: px(TRACK),
                        height: px(THUMB),
                        align_items: AlignItems::Center,
                        ..default()
                    },
                ))
                .with_children(|parent| {
                    parent.spawn((
                        Node {
                            width: percent(100.0),
                            height: px(RAIL),
                            border_radius: BorderRadius::MAX,
                            ..default()
                        },
                        BackgroundColor(RAIL_BG),
                    ));
                    parent.spawn((SliderThumb, thumb_at(sides), BackgroundColor(KNOB)));
                })
                .observe(dragged);
            parent.spawn((
                SidesField,
                Node {
                    width: px(FIELD),
                    padding: UiRect::axes(px(6.0), px(3.0)),
                    border_radius: BorderRadius::all(px(4.0)),
                    ..default()
                },
                BackgroundColor(FIELD_BG),
                text_box(&sides.label()),
                TextFont::from_font_size(LABEL_SIZE),
                TextColor(FG),
                TextCursorStyle {
                    color: FG,
                    ..default()
                },
            ));
        });
}

fn range() -> SliderRange {
    SliderRange::new(f32::from(MIN_SIDES), f32::from(CIRCLE_SIDES))
}

fn text_box(label: &str) -> EditableText {
    EditableText {
        allow_newlines: false,
        ..EditableText::new(label)
    }
}

// The editor holds its own font size, and bevy pushes `TextFont` into it only
// when that component changes: replacing the editor leaves it at 100px.
fn set_label(field: &mut EditableText, label: &str) {
    field.editor_mut().set_text(label);
    field.queue_edit(TextEdit::TextEnd(false));
}

fn thumb_at(sides: Sides) -> Node {
    let along = range().thumb_position(sides.notch());
    Node {
        position_type: PositionType::Absolute,
        left: px(along * (TRACK - THUMB)),
        width: px(THUMB),
        height: px(THUMB),
        border_radius: BorderRadius::MAX,
        ..default()
    }
}

// The drag moves the document and nothing else: `sync` writes the controls, and
// a second writer lands on the box mid-keystroke and takes the caret with it.
fn dragged(
    change: On<ValueChange<f32>>,
    panels: Query<&SidesPanel>,
    mut document: ResMut<Document>,
) {
    if let Ok(panel) = panels.single() {
        set_sides(&mut document, &panel.node, Sides::from_notch(change.value));
    }
}

pub(super) fn typed(
    fields: Query<&EditableText, (With<SidesField>, Changed<EditableText>)>,
    panels: Query<&SidesPanel>,
    mut document: ResMut<Document>,
) {
    let (Ok(field), Ok(panel)) = (fields.single(), panels.single()) else {
        return;
    };
    let Some(sides) = Sides::parse(&field.value().to_string()) else {
        return;
    };
    set_sides(&mut document, &panel.node, sides);
}

pub(super) fn holds(window: &Window, at: Vec2) -> bool {
    let left = (window.width() - PANEL.x) / 2.0;
    at.x >= left && at.x <= left + PANEL.x && at.y >= window.height() - BOTTOM - PANEL.y
}
