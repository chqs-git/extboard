use bevy::input_focus::InputFocus;
use bevy::prelude::*;
use bevy::text::EditableText;
use bevy::text::TextCursorStyle;
use bevy::ui_widgets::{
    Slider, SliderRange, SliderStep, SliderThumb, SliderValue, TrackClick, ValueChange,
};
use extboard_core::{
    CIRCLE_SIDES, Canvas, FONT_ROLES, MID_STROKE, MIN_SIDES, PRIMARY, PRIMARY_TEXT, SECONDARY,
    STROKES, TEXT, Theme as Block, stroke_width, style,
};
use serde_json::{Map, Value};

use crate::client::Document;
use crate::node::NodeId;
use crate::scene::EdgeId;
use crate::select::{PanelRoot, Selected};
use crate::theme::{
    FG, FIELD_BG, LABEL, LABEL_SIZE, PAD, PANEL_BG, ROW, TOP, Theme, code, editable, field_box,
    heading, list, row, row_bg, set_text, tag,
};

use super::{Sides, set_sides};

// The left edge, under the hud line. The theme panel has the right one.
const LEFT: f32 = 8.0;
const WIDTH: f32 = 244.0;
const TRACK: f32 = 100.0;
const RAIL: f32 = 4.0;
const THUMB: f32 = 14.0;
const FIELD: f32 = 40.0;
// The label column, so every row's control starts at the same place.
const GUTTER: f32 = 62.0;
// Seven rows of a dropdown; a palette longer than that scrolls.
const LIST_H: f32 = 7.0 * (ROW + 2.0);
// The stroke buttons draw their own weight as a bar this wide.
const BAR: f32 = 16.0;
const BAR_BASE: f32 = 3.0;
// The chip's own swatch, smaller than a list's.
const DOT: f32 = 10.0;
const RAIL_BG: Color = Color::srgb(0.2, 0.22, 0.26);
const KNOB: Color = Color::srgb(0.95, 0.75, 0.30);

// What the panel is configuring. One node or one edge: a sweep of several has
// nothing to show a single value for.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum Target {
    Node(String),
    Edge(String),
}

// The rows that open a list, which is both the chip's component and what
// `Opened` holds. One list at a time: it unfolds below the rows, and two of
// them there is a panel nobody can aim at.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Row {
    Outline,
    Text,
    Background,
    Font,
}

#[derive(Resource, Default)]
pub(super) struct Opened(pub(super) Option<Row>);

// A colour row's state: what the thing says, and what that comes out as. The
// code is in here so a palette edit moves the chips without the node or the
// edge saying anything new.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Paint {
    pub(super) names: Option<String>,
    pub(super) code: String,
}

// The panel is respawned when any of this changes and left alone otherwise, so
// the sides slider keeps its grip on the pointer mid-drag. The sides are the
// one thing not in here: they are dragged, and they are written in place.
#[derive(Component, PartialEq, Eq)]
pub(crate) struct ConfigPanel {
    target: Target,
    pub(super) outline: Paint,
    pub(super) text: Paint,
    // An edge has no body to fill, so it has no background row.
    pub(super) background: Option<Paint>,
    pub(super) weight: Option<u8>,
    pub(super) font: Option<usize>,
    pub(super) slots: usize,
    pub(super) open: Option<Row>,
}

#[derive(Component)]
pub(super) struct SidesSlider;

#[derive(Component)]
pub(crate) struct SidesField;

#[derive(Component)]
struct Weight(u8);

// A row of an open list: a palette slot, or none of them.
#[derive(Component)]
struct Slot(Option<usize>);

#[derive(Component)]
struct Role(usize);

pub(super) struct Shown {
    pub(super) panel: ConfigPanel,
    pub(super) sides: Option<Sides>,
}

#[allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    reason = "a system's arguments are its query"
)]
pub(super) fn sync(
    mut commands: Commands,
    document: Res<Document>,
    theme: Res<Theme>,
    opened: Res<Opened>,
    focus: Res<InputFocus>,
    nodes: Query<&NodeId, With<Selected>>,
    edges: Query<&EdgeId, With<Selected>>,
    panels: Query<(Entity, &ConfigPanel)>,
    sliders: Query<(Entity, &SliderValue), With<SidesSlider>>,
    thumbs: Query<(Entity, &Node), With<SliderThumb>>,
    mut fields: Query<(Entity, &mut EditableText), With<SidesField>>,
) {
    let want =
        selection(&nodes, &edges).and_then(|target| shown(&document.0, &theme, opened.0, target));
    let open = panels.single().ok();
    match (want, open) {
        (None, Some((entity, _))) => commands.entity(entity).despawn(),
        (Some(want), None) => spawn(&mut commands, &theme, want),
        (Some(want), Some((entity, panel))) if *panel != want.panel => {
            commands.entity(entity).despawn();
            spawn(&mut commands, &theme, want);
        }
        (Some(want), Some(_)) => {
            let Some(sides) = want.sides else {
                return;
            };
            // The controls follow the document, not their own history. Every
            // write re-runs layout, so write only what moved.
            for (entity, value) in &sliders {
                if value.0 != sides.notch() {
                    commands.entity(entity).insert(SliderValue(sides.notch()));
                }
            }
            for (entity, node) in &thumbs {
                let at = thumb_at(sides);
                if node.left != at.left {
                    commands.entity(entity).insert(at);
                }
            }
            for (entity, mut field) in &mut fields {
                let label = sides.label();
                // Never while it is being typed into: the caret is mid-number.
                if focus.get() != Some(entity) && field.value().to_string() != label {
                    set_text(&mut field, &label);
                }
            }
        }
        (None, None) => {}
    }
}

// One node or one edge, and nothing when both or several are picked up.
fn selection(
    nodes: &Query<&NodeId, With<Selected>>,
    edges: &Query<&EdgeId, With<Selected>>,
) -> Option<Target> {
    match (nodes.iter().count(), edges.iter().count()) {
        (1, 0) => Some(Target::Node(nodes.single().ok()?.0.clone())),
        (0, 1) => Some(Target::Edge(edges.single().ok()?.0.clone())),
        _ => None,
    }
}

// `None` for a selection the document has since lost: the entity outlives the
// write that removed it by a frame.
pub(super) fn shown(
    canvas: &Canvas,
    theme: &Theme,
    open: Option<Row>,
    target: Target,
) -> Option<Shown> {
    let painted = |color: Option<&str>, fallback| Paint {
        names: color.map(str::to_owned),
        code: code(theme.paint(color, fallback)),
    };
    let (extra, outline, background, sides) = match &target {
        Target::Node(id) => {
            let node = canvas.nodes.iter().find(|node| node.id == *id)?;
            (
                &node.extra,
                painted(style::outline_color(&node.extra), PRIMARY),
                Some(painted(node.color.as_deref(), SECONDARY)),
                Some(Sides::of(node.sides)),
            )
        }
        // The spec's colour is the edge's outline: a line has nothing to fill.
        Target::Edge(id) => {
            let edge = canvas.edges.iter().find(|edge| edge.id == *id)?;
            (
                &edge.extra,
                painted(edge.color.as_deref(), SECONDARY),
                None,
                None,
            )
        }
    };
    Some(Shown {
        panel: ConfigPanel {
            outline,
            text: painted(style::text_color(extra), TEXT),
            background,
            weight: stroke_width(extra),
            font: style::font_role(extra),
            slots: theme.live.colors.len(),
            // An edge has no body and no text of its own to set, so neither
            // list can be open over it -- not even one the node before it left
            // open.
            open: open.filter(|row| {
                !matches!(target, Target::Edge(_)) || matches!(row, Row::Outline | Row::Text)
            }),
            target,
        },
        sides,
    })
}

fn spawn(commands: &mut Commands, theme: &Theme, want: Shown) {
    let Shown { panel, sides } = want;
    let node = matches!(panel.target, Target::Node(_));
    let (outline, text) = (panel.outline.clone(), panel.text.clone());
    let background = panel.background.clone();
    let (weight, font, open) = (panel.weight, panel.font, panel.open);
    commands
        .spawn((
            panel,
            // A press anywhere in here is this panel's, and the board stands
            // down for it: see `select::PanelPress`.
            PanelRoot,
            // Over the node panels, which order themselves from 0 up, and under
            // the script sidebar at 2.
            GlobalZIndex(3),
            Node {
                position_type: PositionType::Absolute,
                left: px(LEFT),
                top: px(TOP),
                width: px(WIDTH),
                padding: UiRect::all(px(PAD)),
                row_gap: px(4.0),
                flex_direction: FlexDirection::Column,
                border_radius: BorderRadius::all(px(8.0)),
                ..default()
            },
            BackgroundColor(PANEL_BG),
        ))
        .with_children(|parent| {
            color_chip(parent, Row::Outline, &outline);
            color_chip(parent, Row::Text, &text);
            if let Some(background) = &background {
                color_chip(parent, Row::Background, background);
            }
            let pen = weight.unwrap_or(MID_STROKE);
            labelled(parent, "width", |parent| {
                for weight in 1..=STROKES.len() as u8 {
                    weight_button(parent, weight, weight == pen);
                }
            });
            if node {
                font_chip(parent, theme, font.unwrap_or(PRIMARY_TEXT));
            }
            if let Some(sides) = sides {
                sides_row(parent, sides);
            }
            // Below the rows rather than over them: a list that covers the row
            // it belongs to is a list you cannot see your choice in.
            if let Some(row) = open {
                candidates(parent, theme, row, (&outline, &text, &background, font));
            }
        });
}

// A label in the gutter and the row's controls beside it.
fn labelled(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    controls: impl FnOnce(&mut ChildSpawnerCommands),
) {
    parent
        .spawn(Node {
            height: px(ROW),
            align_items: AlignItems::Center,
            column_gap: px(6.0),
            ..default()
        })
        .with_children(|parent| {
            parent.spawn((
                Node {
                    width: px(GUTTER),
                    ..default()
                },
                children![tag(label, LABEL)],
            ));
            controls(parent);
        });
}

// The chip says what the row draws in: the colour, and the palette's own word
// for it. Pressing it unfolds the palette.
fn color_chip(parent: &mut ChildSpawnerCommands, which: Row, paint: &Paint) {
    labelled(parent, which.label(), |parent| {
        parent
            .spawn((
                which,
                Node {
                    flex_grow: 1.0,
                    height: px(ROW),
                    align_items: AlignItems::Center,
                    column_gap: px(6.0),
                    padding: UiRect::horizontal(px(5.0)),
                    overflow: Overflow::clip(),
                    ..default()
                },
                BackgroundColor(FIELD_BG),
                children![dot(paint.code.as_str()), tag(&name_of(paint), FG)],
            ))
            .observe(open_list);
    });
}

// The three texts, by name, each drawn in the font that text holds.
fn font_chip(parent: &mut ChildSpawnerCommands, theme: &Theme, role: usize) {
    labelled(parent, Row::Font.label(), |parent| {
        parent
            .spawn((
                Row::Font,
                Node {
                    flex_grow: 1.0,
                    height: px(ROW),
                    align_items: AlignItems::Center,
                    column_gap: px(6.0),
                    padding: UiRect::horizontal(px(5.0)),
                    overflow: Overflow::clip(),
                    ..default()
                },
                BackgroundColor(FIELD_BG),
                children![role_tag(theme, role)],
            ))
            .observe(open_list);
    });
}

fn sides_row(parent: &mut ChildSpawnerCommands, sides: Sides) {
    labelled(parent, "sides", |parent| {
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
            field_box(px(FIELD)),
            editable(&sides.label()),
            TextFont::from_font_size(LABEL_SIZE),
            TextColor(FG),
            TextCursorStyle {
                color: FG,
                ..default()
            },
        ));
    });
}

// The button draws the weight it stands for: a bar of that thickness is a
// clearer label than the number is.
fn weight_button(parent: &mut ChildSpawnerCommands, weight: u8, on: bool) {
    parent
        .spawn((
            Weight(weight),
            Node {
                width: px(30.0),
                height: px(ROW),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                border_radius: BorderRadius::all(px(3.0)),
                ..default()
            },
            BackgroundColor(row_bg(on)),
            children![(
                Node {
                    width: px(BAR),
                    height: px(BAR_BASE * STROKES[usize::from(weight) - 1]),
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                BackgroundColor(if on { FG } else { LABEL }),
                // Or the bar eats the press meant for the button under it.
                Pickable::IGNORE,
            )],
        ))
        .observe(pick_weight);
}

// The open list: the palette by role, or the three texts.
fn candidates(
    parent: &mut ChildSpawnerCommands,
    theme: &Theme,
    which: Row,
    (outline, text, background, font): (&Paint, &Paint, &Option<Paint>, Option<usize>),
) {
    parent.spawn((heading(which.label()), TextColor(LABEL)));
    list(parent, LIST_H, |parent| match which {
        Row::Font => {
            let chosen = font.unwrap_or(PRIMARY_TEXT);
            for role in 0..FONT_ROLES.len() {
                parent
                    .spawn((
                        Role(role),
                        Node {
                            column_gap: px(6.0),
                            ..row()
                        },
                        BackgroundColor(row_bg(role == chosen)),
                        children![role_tag(theme, role)],
                    ))
                    .observe(pick_role);
            }
        }
        which => {
            let paint = match which {
                Row::Text => text,
                Row::Background => background.as_ref().unwrap_or(outline),
                _ => outline,
            };
            // The palette's own colour first, which is what no key at all draws.
            for slot in std::iter::once(None).chain((0..theme.live.colors.len()).map(Some)) {
                let on = match slot {
                    Some(slot) => paint.names.as_deref() == Some(slot_value(theme, slot).as_str()),
                    None => paint.names.is_none(),
                };
                let label = slot.map_or_else(|| "theme".to_owned(), Block::role);
                let swatch = slot.map_or(paint.code.clone(), |slot| code(theme.color(slot)));
                parent
                    .spawn((
                        Slot(slot),
                        Node {
                            column_gap: px(6.0),
                            ..row()
                        },
                        BackgroundColor(row_bg(on)),
                        children![dot(&swatch), tag(&label, FG)],
                    ))
                    .observe(pick_slot);
            }
        }
    });
}

fn dot(code: &str) -> impl Bundle {
    (
        Node {
            width: px(DOT),
            height: px(DOT),
            flex_shrink: 0.0,
            border_radius: BorderRadius::all(px(2.0)),
            ..default()
        },
        BackgroundColor(crate::theme::hex(code).unwrap_or(Color::NONE)),
        Pickable::IGNORE,
    )
}

// The role's name, drawn in the font that role holds: the preview and the label
// are the same words.
fn role_tag(theme: &Theme, role: usize) -> impl Bundle {
    (
        Text::new(FONT_ROLES[role].to_owned()),
        TextFont {
            font: theme.text_font(role),
            font_smoothing: theme.smoothing(),
            ..TextFont::from_font_size(LABEL_SIZE)
        },
        TextColor(FG),
        Pickable::IGNORE,
    )
}

impl Row {
    fn label(self) -> &'static str {
        match self {
            Self::Outline => "outline",
            Self::Text => "text",
            Self::Background => "background",
            Self::Font => "font",
        }
    }
}

// What the chip reads: the role a slot spells, a literal colour as itself, and
// the palette's own word for a row naming nothing.
fn name_of(paint: &Paint) -> String {
    match paint.names.as_deref() {
        None => "theme".to_owned(),
        Some(color) => match color.parse::<usize>() {
            Ok(slot) => Block::role(slot),
            Err(_) => color.to_owned(),
        },
    }
}

// The spec spells six slots and no more, so a colour past them goes in as the
// hex it is: inside the six, the index is what makes a retheme move the node.
pub(super) fn slot_value(theme: &Theme, slot: usize) -> String {
    match slot {
        1..=6 => slot.to_string(),
        _ => theme
            .live
            .color(slot)
            .map(str::to_owned)
            .unwrap_or_default(),
    }
}

fn range() -> SliderRange {
    SliderRange::new(f32::from(MIN_SIDES), f32::from(CIRCLE_SIDES))
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
    panels: Query<&ConfigPanel>,
    mut document: ResMut<Document>,
) {
    if let Ok(ConfigPanel {
        target: Target::Node(id),
        ..
    }) = panels.single()
    {
        set_sides(&mut document, id, Sides::from_notch(change.value));
    }
}

fn open_list(press: On<Pointer<Press>>, chips: Query<&Row>, mut opened: ResMut<Opened>) {
    let Ok(row) = chips.get(press.entity) else {
        return;
    };
    opened.0 = match opened.0 {
        Some(open) if open == *row => None,
        _ => Some(*row),
    };
}

fn pick_weight(
    press: On<Pointer<Press>>,
    buttons: Query<&Weight>,
    panels: Query<&ConfigPanel>,
    mut document: ResMut<Document>,
) {
    let (Ok(button), Ok(panel)) = (buttons.get(press.entity), panels.single()) else {
        return;
    };
    if let Some(extra) = extras(&mut document.0, &panel.target) {
        style::set_stroke_width(extra, button.0);
    }
}

fn pick_slot(
    press: On<Pointer<Press>>,
    rows: Query<&Slot>,
    panels: Query<&ConfigPanel>,
    theme: Res<Theme>,
    mut opened: ResMut<Opened>,
    mut document: ResMut<Document>,
) {
    let (Ok(slot), Ok(panel), Some(row)) = (rows.get(press.entity), panels.single(), opened.0)
    else {
        return;
    };
    let value = slot.0.map(|slot| slot_value(&theme, slot));
    set_color(&mut document, &panel.target, row, value.as_deref());
    opened.0 = None;
}

fn pick_role(
    press: On<Pointer<Press>>,
    rows: Query<&Role>,
    panels: Query<&ConfigPanel>,
    mut opened: ResMut<Opened>,
    mut document: ResMut<Document>,
) {
    let (Ok(role), Ok(panel)) = (rows.get(press.entity), panels.single()) else {
        return;
    };
    if let Some(extra) = extras(&mut document.0, &panel.target) {
        style::set_font_role(extra, Some(role.0));
    }
    opened.0 = None;
}

// Only the box the caret is in: a box that has just been spawned full of the
// value it is showing counts as changed, and writing that back is not an edit.
#[allow(clippy::type_complexity, reason = "a system's arguments are its query")]
pub(super) fn typed(
    focus: Res<InputFocus>,
    panels: Query<&ConfigPanel>,
    fields: Query<(Entity, &EditableText), (With<SidesField>, Changed<EditableText>)>,
    mut document: ResMut<Document>,
) {
    let (Ok(panel), Ok((entity, field))) = (panels.single(), fields.single()) else {
        return;
    };
    if let Target::Node(id) = &panel.target
        && focus.get() == Some(entity)
        && let Some(sides) = Sides::parse(&field.value().to_string())
    {
        set_sides(&mut document, id, sides);
    }
}

// A node's outline and text colours are ours and its background is the spec's
// `color`; an edge has no background, so the spec's colour is its outline.
fn set_color(
    document: &mut ResMut<Document>,
    target: &Target,
    row: Row,
    color: Option<&str>,
) -> Option<()> {
    let canvas = &mut document.0;
    match (row, target) {
        (Row::Background, Target::Node(id)) => {
            let node = canvas.nodes.iter_mut().find(|node| node.id == *id)?;
            node.color = color.map(str::to_owned);
        }
        (Row::Outline, Target::Edge(id)) => {
            let edge = canvas.edges.iter_mut().find(|edge| edge.id == *id)?;
            edge.color = color.map(str::to_owned);
        }
        (Row::Outline, _) => style::set_outline_color(extras(canvas, target)?, color),
        (Row::Text, _) => style::set_text_color(extras(canvas, target)?, color),
        (Row::Background, _) | (Row::Font, _) => {}
    }
    Some(())
}

// Our keys, on a node and on an edge alike, so one lookup serves all four.
fn extras<'a>(canvas: &'a mut Canvas, target: &Target) -> Option<&'a mut Map<String, Value>> {
    match target {
        Target::Node(id) => canvas
            .nodes
            .iter_mut()
            .find(|node| node.id == *id)
            .map(|node| &mut node.extra),
        Target::Edge(id) => canvas
            .edges
            .iter_mut()
            .find(|edge| edge.id == *id)
            .map(|edge| &mut edge.extra),
    }
}
