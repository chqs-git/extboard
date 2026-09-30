use bevy::camera::CameraUpdateSystems;
use bevy::input::InputSystems;
use bevy::input_focus::AutoFocus;
use bevy::prelude::*;
use bevy::text::{EditableText, FontStyle, FontWeight, TextCursorStyle};
use bevy::transform::TransformSystems;
use bevy::ui::UiSystems;
use extboard_core::{Canvas, Node as CanvasNode, NodeKind};
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::collections::HashMap;

use crate::camera::world_to_screen;
use crate::client::Document;
use crate::edit::double_click;
use crate::node::{NodeId, NodeRect};
use crate::select::{bounds, cursor_world, pick};

// global mk settings
const PADDING: f32 = 12.0;
const ROW_GAP: f32 = 8.0;

const BODY: f32 = 15.0;
const CODE: f32 = 13.0;

const FG: Color = Color::srgb(0.88, 0.9, 0.93);
const LINK: Color = Color::srgb(0.44, 0.62, 1.0);
const MONO: Color = Color::srgb(0.7, 0.85, 0.72);
const CODE_BG: Color = Color::srgb(0.06, 0.07, 0.09);
const RULE: Color = Color::srgb(0.25, 0.27, 0.32);

pub struct TextPlugin;

// Two nodes because bevy clips to the *laid-out* box and never to the
// transformed one: the clip box takes the zoom in layout, the content in scale.
// Both hold the node id rather than a position: a drag moves the node entity,
// and the panel has to go with it.
#[derive(Component)]
struct ClipBox(String);

#[derive(Component)]
struct Content(String);

#[derive(Resource, Default)]
pub struct Editing(pub Option<String>);

#[derive(Component)]
struct Editor;

pub fn editing(editing: Res<Editing>) -> bool {
    editing.0.is_some()
}

impl Plugin for TextPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Editing>()
            // Before every Update system that reads the same click, so a frame
            // is either an editing frame or a canvas one, never half of each.
            .add_systems(
                PreUpdate,
                toggle
                    .after(InputSystems)
                    .run_if(resource_exists::<Document>),
            )
            .add_systems(
                Update,
                // `and_then` is lazy, so the changed checks never run before the
                // document lands: entering edit mode has to rebuild a panel too.
                spawn_panels.run_if(
                    resource_exists::<Document>.and_then(
                        resource_changed::<Document>.or_else(resource_changed::<Editing>),
                    ),
                ),
            )
            // After propagation and CameraUpdateSystems, or the camera transform and
            // projection this reads are a frame stale. Before Layout: it writes `Node`.
            .add_systems(
                PostUpdate,
                track_panels
                    .after(TransformSystems::Propagate)
                    .after(CameraUpdateSystems)
                    .before(UiSystems::Layout),
            );
    }
}

fn markdown(node: &CanvasNode) -> Option<&str> {
    match &node.kind {
        NodeKind::Text { text } => Some(text),
        _ => None,
    }
}

fn spawn_panels(
    mut commands: Commands,
    document: Res<Document>,
    editing: Res<Editing>,
    existing: Query<Entity, With<ClipBox>>,
) {
    // rebuilds every panel
    for entity in &existing {
        commands.entity(entity).despawn();
    }

    for node in &document.0.nodes {
        let Some(md) = markdown(node) else {
            continue;
        };
        let size = Vec2::new(node.width as f32, node.height as f32);
        let open = editing.0.as_deref() == Some(node.id.as_str());
        commands
            .spawn((
                ClipBox(node.id.clone()),
                Node {
                    position_type: PositionType::Absolute,
                    width: px(size.x),
                    height: px(size.y),
                    overflow: Overflow::clip(),
                    ..default()
                },
            ))
            .with_children(|parent| {
                parent
                    .spawn((
                        Content(node.id.clone()),
                        // Absolute: a flex child would be shrunk to fit below 100%.
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(0.0),
                            top: px(0.0),
                            width: px(size.x),
                            height: px(size.y),
                            padding: UiRect::all(px(PADDING)),
                            flex_direction: FlexDirection::Column,
                            row_gap: px(ROW_GAP),
                            ..default()
                        },
                    ))
                    .with_children(|parent| {
                        if open {
                            parent.spawn(editor(md));
                        } else {
                            spawn_blocks(&blocks(md), parent);
                        }
                    });
            });
    }
}

// Source while editing, rendered at rest: the buffer is the node's raw markdown,
// and the styled span tree is never edited.
fn editor(md: &str) -> impl Bundle {
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
fn toggle(
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
fn written_back(canvas: &mut Canvas, id: &str, text: &str) -> bool {
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

fn track_panels(
    camera: Single<(&Camera, &GlobalTransform, &Projection), With<Camera2d>>,
    nodes: Query<(&NodeId, &Transform, &NodeRect)>,
    mut clip_boxes: Query<(&ClipBox, &mut Node)>,
    mut contents: Query<(&Content, &mut UiTransform)>,
) {
    let (camera, cam_global, projection) = *camera;
    let Projection::Orthographic(ortho) = projection else {
        return;
    };
    let zoom = 1.0 / ortho.scale;
    let placed: HashMap<&str, (Vec2, Vec2)> = nodes
        .iter()
        .map(|(id, transform, rect)| {
            (
                id.0.as_str(),
                (transform.translation.truncate(), rect.size()),
            )
        })
        .collect();

    for (clip_box, mut node) in &mut clip_boxes {
        let Some(&(center, size)) = placed.get(clip_box.0.as_str()) else {
            continue;
        };
        let Some(screen) = world_to_screen(camera, cam_global, center) else {
            continue;
        };
        let scaled = size * zoom;
        let want = (
            px(screen.x - scaled.x / 2.0),
            px(screen.y - scaled.y / 2.0),
            px(scaled.x),
            px(scaled.y),
        );
        // Any write re-runs layout, so write only what moved.
        if (node.left, node.top, node.width, node.height) != want {
            (node.left, node.top, node.width, node.height) = want;
        }
    }

    for (content, mut transform) in &mut contents {
        let Some(&(_, size)) = placed.get(content.0.as_str()) else {
            continue;
        };
        let offset = centre_scale_offset(size, zoom);
        let want = UiTransform {
            scale: Vec2::splat(zoom),
            translation: Val2::px(offset.x, offset.y),
            ..UiTransform::IDENTITY
        };
        if *transform != want {
            *transform = want;
        }
    }
}

// A scale is about the centre, so put the grown box back on the clip box corner
fn centre_scale_offset(size: Vec2, zoom: f32) -> Vec2 {
    size * (zoom - 1.0) / 2.0
}

#[derive(Debug, PartialEq)]
enum Block {
    // Paragraph, heading or list item.
    Line(Vec<Span>),
    Code(String),
    Table { cols: usize, cells: Vec<Cell> },
}

#[derive(Debug, Default, PartialEq)]
struct Span {
    text: String,
    size: f32,
    bold: bool,
    italic: bool,
    mono: bool,
    link: bool,
}

#[derive(Debug, PartialEq)]
struct Cell {
    text: String,
    head: bool,
}

#[derive(Default)]
struct Marks {
    size: f32,
    bold: usize,
    italic: usize,
    mono: usize,
    link: usize,
}

impl Marks {
    fn span(&self, text: impl Into<String>) -> Span {
        Span {
            text: text.into(),
            size: if self.mono > 0 { CODE } else { self.size },
            bold: self.bold > 0,
            italic: self.italic > 0,
            mono: self.mono > 0,
            link: self.link > 0,
        }
    }
}

fn blocks(md: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut spans: Vec<Span> = Vec::new();
    let mut marks = Marks {
        size: BODY,
        ..default()
    };
    let mut cells: Vec<Cell> = Vec::new();
    let mut cols = 0usize;
    let mut in_head = false;
    let mut in_table = false;

    for event in Parser::new_ext(md, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                marks.size = heading_size(level);
                marks.bold += 1;
            }
            Event::End(TagEnd::Heading(_)) => {
                flush(&mut out, &mut spans);
                marks.size = BODY;
                marks.bold -= 1;
            }
            Event::Start(Tag::Strong) => marks.bold += 1,
            Event::End(TagEnd::Strong) => marks.bold -= 1,
            Event::Start(Tag::Emphasis) => marks.italic += 1,
            Event::End(TagEnd::Emphasis) => marks.italic -= 1,
            Event::Start(Tag::Link { .. }) => marks.link += 1,
            Event::End(TagEnd::Link) => marks.link -= 1,

            Event::Start(Tag::Item) => spans.push(marks.span("- ")),
            Event::End(TagEnd::Item) => flush(&mut out, &mut spans),

            Event::Start(Tag::CodeBlock(_)) => marks.mono += 1,
            Event::End(TagEnd::CodeBlock) => {
                marks.mono -= 1;
                let code: String = spans.drain(..).map(|span| span.text).collect();
                out.push(Block::Code(code.trim_end().to_owned()));
            }

            Event::Start(Tag::Table(aligns)) => {
                in_table = true;
                cols = aligns.len();
            }
            Event::Start(Tag::TableHead) => in_head = true,
            Event::End(TagEnd::TableHead) => in_head = false,
            Event::End(TagEnd::TableCell) => cells.push(Cell {
                text: spans.drain(..).map(|span| span.text).collect(),
                head: in_head,
            }),
            Event::End(TagEnd::Table) => {
                in_table = false;
                out.push(Block::Table {
                    cols,
                    cells: std::mem::take(&mut cells),
                });
            }

            Event::Text(text) => spans.push(marks.span(text.into_string())),
            Event::Code(text) => {
                marks.mono += 1;
                spans.push(marks.span(text.into_string()));
                marks.mono -= 1;
            }
            // A paragraph inside a cell is the cell
            Event::End(TagEnd::Paragraph) if !in_table => flush(&mut out, &mut spans),
            Event::SoftBreak => spans.push(marks.span(" ")),
            Event::HardBreak => flush(&mut out, &mut spans),
            _ => {}
        }
    }

    flush(&mut out, &mut spans);
    out
}

fn flush(out: &mut Vec<Block>, spans: &mut Vec<Span>) {
    if !spans.is_empty() {
        out.push(Block::Line(std::mem::take(spans)));
    }
}

fn heading_size(level: HeadingLevel) -> f32 {
    match level {
        HeadingLevel::H1 => 26.0,
        HeadingLevel::H2 => 21.0,
        _ => 17.0,
    }
}

fn spawn_blocks(blocks: &[Block], parent: &mut ChildSpawnerCommands) {
    for block in blocks {
        match block {
            Block::Line(spans) => {
                let Some((first, rest)) = spans.split_first() else {
                    continue;
                };
                parent.spawn(text_bundle(first)).with_children(|parent| {
                    for span in rest {
                        parent.spawn((
                            TextSpan::new(span.text.clone()),
                            font(span),
                            TextColor(color(span)),
                        ));
                    }
                });
            }
            Block::Code(code) => {
                parent
                    .spawn((
                        Node {
                            padding: UiRect::all(px(8.0)),
                            border_radius: BorderRadius::all(px(4.0)),
                            ..default()
                        },
                        BackgroundColor(CODE_BG),
                    ))
                    .with_child((
                        Text::new(code.clone()),
                        TextFont::from_font_size(CODE),
                        TextColor(MONO),
                    ));
            }
            Block::Table { cols, cells } => {
                if *cols == 0 {
                    continue;
                }
                parent
                    .spawn(Node {
                        display: Display::Grid,
                        // Equal fractions: an `auto` table overflows the node.
                        grid_template_columns: RepeatedGridTrack::flex(*cols as u16, 1.0),
                        ..default()
                    })
                    .with_children(|parent| {
                        for cell in cells {
                            parent
                                .spawn((
                                    Node {
                                        padding: UiRect::axes(px(8.0), px(4.0)),
                                        border: UiRect::all(px(1.0)),
                                        ..default()
                                    },
                                    BorderColor::all(RULE),
                                ))
                                .with_child((
                                    Text::new(cell.text.clone()),
                                    font(&Span {
                                        size: CODE,
                                        bold: cell.head,
                                        ..default()
                                    }),
                                    TextColor(FG),
                                ));
                        }
                    });
            }
        }
    }
}

fn text_bundle(span: &Span) -> (Text, TextFont, TextColor) {
    (
        Text::new(span.text.clone()),
        font(span),
        TextColor(color(span)),
    )
}

fn font(span: &Span) -> TextFont {
    TextFont {
        // The embedded default font is FiraMono, so `mono` is carried by
        // colour, not family. A generic FontSource renders nothing here.
        font: default(),
        weight: if span.bold {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        },
        style: if span.italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        },
        ..TextFont::from_font_size(span.size)
    }
}

// No dimming: node rects are coloured, and a dimmed run on one is unreadable.
fn color(span: &Span) -> Color {
    if span.link {
        LINK
    } else if span.mono {
        MONO
    } else {
        FG
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The scaled content's corner, relative to the clip box's. Non-zero clips.
    fn content_top_left(size: Vec2, zoom: f32) -> Vec2 {
        let center = size / 2.0 + centre_scale_offset(size, zoom);
        center - size * zoom / 2.0
    }

    fn canvas(text: &str) -> Canvas {
        Canvas {
            nodes: vec![CanvasNode {
                id: "n".to_owned(),
                x: 0,
                y: 0,
                width: 200,
                height: 100,
                color: None,
                kind: NodeKind::Text {
                    text: text.to_owned(),
                },
                extra: default(),
            }],
            edges: Vec::new(),
            extra: default(),
        }
    }

    #[test]
    fn only_a_changed_buffer_is_written_back() {
        let mut got = canvas("before");
        assert!(written_back(&mut got, "n", "after"));
        assert_eq!(markdown(&got.nodes[0]), Some("after"));

        // Opened and closed without typing: the document must not be woken.
        assert!(!written_back(&mut got, "n", "after"));
        // And a node that is gone, or was never text, is not an error.
        assert!(!written_back(&mut got, "gone", "after"));
    }

    #[test]
    fn content_fills_its_clip_box_at_every_zoom() {
        let size = Vec2::new(740.0, 460.0);
        for zoom in [0.25, 0.5, 1.0, 2.0, 8.0] {
            let corner = content_top_left(size, zoom);
            assert!(corner.abs().max_element() < 1e-3, "zoom {zoom}: {corner}");
        }
    }

    fn line(blocks: &[Block], index: usize) -> &[Span] {
        match &blocks[index] {
            Block::Line(spans) => spans,
            other => panic!("block {index} is {other:?}, not a line"),
        }
    }

    #[test]
    fn inline_runs_keep_their_marks() {
        let got = blocks("plain *em* **strong** `code()` [link](https://bevy.org)");
        let spans = line(&got, 0);
        let marks: Vec<_> = spans
            .iter()
            .map(|s| (s.text.as_str(), s.bold, s.italic, s.mono, s.link))
            .collect();
        assert_eq!(
            marks,
            [
                ("plain ", false, false, false, false),
                ("em", false, true, false, false),
                (" ", false, false, false, false),
                ("strong", true, false, false, false),
                (" ", false, false, false, false),
                ("code()", false, false, true, false),
                (" ", false, false, false, false),
                ("link", false, false, false, true),
            ]
        );
    }

    #[test]
    fn a_heading_is_bold_and_bigger_than_the_body() {
        let got = blocks("# Title\n\nbody\n");
        let title = &line(&got, 0)[0];
        assert!(title.bold && title.size > BODY, "{title:?}");
        assert_eq!(line(&got, 1)[0].size, BODY);
    }

    #[test]
    fn nested_emphasis_closes_inside_out() {
        let got = blocks("**bold *both* tail**");
        let spans = line(&got, 0);
        assert!(spans.iter().all(|s| s.bold), "{spans:?}");
        assert_eq!(
            spans
                .iter()
                .filter(|s| s.italic)
                .map(|s| &s.text)
                .collect::<Vec<_>>(),
            ["both"]
        );
    }

    #[test]
    fn a_table_becomes_a_grid_of_cells_in_row_order() {
        let got =
            blocks("| crate | verdict |\n|---|---|\n| bevy | parley |\n| pulldown | tables |\n");
        let Block::Table { cols, cells } = &got[0] else {
            panic!("{got:?}");
        };
        assert_eq!(*cols, 2);
        assert_eq!(cells.len(), 6, "2 columns x 3 rows");
        assert_eq!(
            cells
                .iter()
                .map(|c| (c.text.as_str(), c.head))
                .collect::<Vec<_>>(),
            [
                ("crate", true),
                ("verdict", true),
                ("bevy", false),
                ("parley", false),
                ("pulldown", false),
                ("tables", false),
            ]
        );
    }

    #[test]
    fn a_code_block_keeps_its_newlines_and_drops_the_trailing_one() {
        let got = blocks("```rust\nfn main() {\n    ok();\n}\n```\n");
        assert_eq!(got, [Block::Code("fn main() {\n    ok();\n}".to_owned())]);
    }

    #[test]
    fn list_items_are_one_line_each_with_a_bullet() {
        let got = blocks("- first\n- second **bold**\n");
        assert_eq!(got.len(), 2);
        assert_eq!(line(&got, 0)[0].text, "- ");
        assert_eq!(line(&got, 1).last().unwrap().text, "bold");
    }
}
