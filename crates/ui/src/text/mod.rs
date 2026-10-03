use bevy::input::InputSystems;
use bevy::prelude::*;
use bevy::text::{FontStyle, FontWeight};
use bevy::transform::TransformSystems;
use bevy::ui::UiSystems;
use extboard_core::{Node as CanvasNode, NodeKind};
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use bevy::camera::CameraUpdateSystems;

use crate::client::Document;

mod edit_text;
mod panel;

pub use edit_text::{Editing, Target, editing};
use edit_text::{toggle, track_label};
use panel::{outline_panels, spawn_label_editor, spawn_panels, track_panels};

// global mk settings
const PADDING: f32 = 12.0;
const ROW_GAP: f32 = 8.0;

const BODY: f32 = 15.0;
const CODE: f32 = 13.0;
// A group's name, bigger and dimmer than body text: it labels a container.
const GROUP_SIZE: f32 = 16.0;
const GROUP_LABEL: Color = Color::srgb(0.7, 0.74, 0.8);

const FG: Color = Color::srgb(0.88, 0.9, 0.93);
const LINK: Color = Color::srgb(0.44, 0.62, 1.0);
const MONO: Color = Color::srgb(0.7, 0.85, 0.72);
const CODE_BG: Color = Color::srgb(0.06, 0.07, 0.09);
const RULE: Color = Color::srgb(0.25, 0.27, 0.32);

// The frame's press, as the editors read it. Anything that means to consume a
// click before they see it orders itself before this.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReadPress;

pub struct TextPlugin;

impl Plugin for TextPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Editing>()
            // Before every Update system that reads the same click, so a frame
            // is either an editing frame or a canvas one, never half of each.
            .add_systems(
                PreUpdate,
                toggle
                    .in_set(ReadPress)
                    .after(InputSystems)
                    .run_if(resource_exists::<Document>),
            )
            .add_systems(
                Update,
                // `and_then` is lazy, so the changed checks never run before the
                // document lands: entering edit mode has to rebuild a panel too.
                (spawn_panels, spawn_label_editor).chain().run_if(
                    resource_exists::<Document>.and_then(
                        resource_changed::<Document>.or_else(resource_changed::<Editing>),
                    ),
                ),
            )
            // After propagation and CameraUpdateSystems, or the camera transform and
            // projection this reads are a frame stale. Before Layout: it writes `Node`.
            .add_systems(
                PostUpdate,
                (
                    // The document arrives a fetch later than the first frame.
                    track_panels.run_if(resource_exists::<Document>),
                    track_label,
                    outline_panels,
                )
                    .after(TransformSystems::Propagate)
                    .after(CameraUpdateSystems)
                    .before(UiSystems::Layout),
            );
    }
}

pub(super) fn markdown(node: &CanvasNode) -> Option<&str> {
    match &node.kind {
        NodeKind::Text { text } => Some(text),
        _ => None,
    }
}

#[allow(clippy::type_complexity, reason = "a system's arguments are its query")]
#[derive(Debug, PartialEq)]
pub(super) enum Block {
    // Paragraph, heading or list item.
    Line(Vec<Span>),
    Code(String),
    Table { cols: usize, cells: Vec<Cell> },
}

#[derive(Debug, Default, PartialEq)]
pub(super) struct Span {
    text: String,
    size: f32,
    bold: bool,
    italic: bool,
    mono: bool,
    link: bool,
}

#[derive(Debug, PartialEq)]
pub(super) struct Cell {
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

pub(super) fn blocks(md: &str) -> Vec<Block> {
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

pub(super) fn spawn_blocks(blocks: &[Block], parent: &mut ChildSpawnerCommands) {
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
                        wrap(),
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
                                    wrap(),
                                ));
                        }
                    });
            }
        }
    }
}

fn text_bundle(span: &Span) -> (Text, TextFont, TextColor, TextLayout) {
    (
        Text::new(span.text.clone()),
        font(span),
        TextColor(color(span)),
        wrap(),
    )
}

// Words stay whole where they can, and break where they cannot: a URL or a long
// identifier is otherwise drawn straight out of the node and clipped.
pub(super) fn wrap() -> TextLayout {
    TextLayout {
        linebreak: LineBreak::WordOrCharacter,
        ..default()
    }
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
mod tests;
