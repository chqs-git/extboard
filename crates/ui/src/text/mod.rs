use bevy::input::InputSystems;
use bevy::prelude::*;
use bevy::text::{FontStyle, FontWeight, Justify};
use bevy::transform::TransformSystems;
use bevy::ui::UiSystems;
use extboard_core::{
    ACCENT, ACCENT_B, BACKGROUND, Node as CanvasNode, NodeKind, PRIMARY, TEXT, Vars, interpolate,
    space_path,
};
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use bevy::camera::CameraUpdateSystems;

use crate::client::Document;
use crate::theme::{Face, Theme};

mod edit_text;
mod link;
mod panel;

pub use edit_text::{Editing, Target, editing};
use edit_text::{release_field, toggle, track_label};
use link::Doors;
pub use link::Links;
use panel::{outline_panels, spawn_label_editor, spawn_panels, track_panels};

const PADDING: f32 = 12.0;
const ROW_GAP: f32 = 8.0;

const BODY: f32 = 15.0;
const CODE: f32 = 13.0;
const GROUP_SIZE: f32 = 16.0;

// A code block's well: the board behind it, so it reads as a hole in the node.
const WELL: f32 = 0.5;

// Three because the fourth costs another atlas to buy a difference nobody sees.
const RASTER_MAX: f32 = 3.0;

// The stretch a tier is allowed to carry before the next one takes over: a fifth
// over native still reads as sharp, and it keeps the cheap tier over more zoom.
const RASTER_SLACK: f32 = 1.2;

// Glyphs are rasterized at the size they are laid out at, and a panel is then
// stretched to the camera's zoom — so a zoomed-in panel is a blown-up bitmap.
// Laying it out at this multiple instead re-rasterizes it that much sharper;
// bevy's atlas keeps one set of glyphs per size, so a tier is paid for once.
#[derive(Resource, PartialEq, Debug)]
pub struct Raster(pub f32);

impl Default for Raster {
    fn default() -> Self {
        Self(1.0)
    }
}

// Ceil, not round: short of the zoom by more than the slack is a visible stretch.
fn raster_tier(zoom: f32) -> f32 {
    (zoom / RASTER_SLACK).ceil().clamp(1.0, RASTER_MAX)
}

// An editor is clipped to its own content box, which bevy measures before the
// panel's scale reaches it: a tier's slack then shaves the left and top off the
// glyphs, so a session is laid out at the zoom it opens on and scaled by one.
fn editing_raster(zoom: f32) -> f32 {
    zoom.max(1.0)
}

// Frozen while editing: a tier change respawns the panel, and a respawn would
// reload the editor from the document and drop what has been typed into it.
fn track_raster(
    camera: Single<&Projection, With<Camera2d>>,
    editing: Res<Editing>,
    mut raster: ResMut<Raster>,
) {
    let Projection::Orthographic(ortho) = *camera else {
        return;
    };
    let zoom = 1.0 / ortho.scale;
    if editing.0.is_some() {
        if editing.is_changed() {
            raster.set_if_neq(Raster(editing_raster(zoom)));
        }
        return;
    }
    raster.set_if_neq(Raster(raster_tier(zoom)));
}

// Anything consuming a click before the editors see it orders itself before this.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReadPress;

pub struct TextPlugin;

impl Plugin for TextPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Editing>()
            .init_resource::<Raster>()
            // Before every Update system that reads the same click, so a frame
            // is either an editing frame or a canvas one, never half of each.
            .add_systems(
                PreUpdate,
                (release_field, toggle)
                    .chain()
                    .in_set(ReadPress)
                    .after(InputSystems)
                    .run_if(resource_exists::<Document>),
            )
            .add_systems(
                Update,
                (
                    track_raster,
                    // `and_then` is lazy, so the changed checks never run before
                    // the document lands: entering edit mode has to rebuild a
                    // panel too.
                    (spawn_panels, spawn_label_editor).chain().run_if(
                        resource_exists::<Document>.and_then(
                            resource_changed::<Document>
                                .or_else(resource_changed::<Editing>)
                                .or_else(resource_changed::<Raster>)
                                .or_else(resource_changed::<crate::theme::Theme>),
                        ),
                    ),
                )
                    .chain(),
            )
            // After propagation, or the camera it reads is a frame stale. Before
            // Layout: it writes `Node`.
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
    space: Option<String>,
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
    space: Option<String>,
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
            space: self.space.clone(),
        }
    }
}

pub(super) fn blocks(md: &str, vars: &Vars) -> Vec<Block> {
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
            Event::Start(Tag::Link { ref dest_url, .. }) => {
                marks.link += 1;
                marks.space = space_path(dest_url).map(str::to_owned);
            }
            Event::End(TagEnd::Link) => {
                marks.link -= 1;
                marks.space = None;
            }

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

            // A fence's body arrives as `Text` too, so `{{x}}` in one is left
            // alone by the same guard that picks the code font.
            Event::Text(text) if marks.mono > 0 => spans.push(marks.span(text.into_string())),
            Event::Text(text) => spans.push(marks.span(interpolate(&text, vars).into_owned())),
            Event::Code(text) => {
                marks.mono += 1;
                spans.push(marks.span(text.into_string()));
                marks.mono -= 1;
            }
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

#[allow(
    clippy::too_many_arguments,
    reason = "two faces, and each is three fields"
)]
pub(super) fn spawn_blocks(
    node: &str,
    blocks: &[Block],
    theme: &Theme,
    face: &Face,
    code_face: &Face,
    raster: f32,
    justify: Justify,
    parent: &mut ChildSpawnerCommands,
) {
    for block in blocks {
        match block {
            Block::Line(spans) => {
                let Some((first, rest)) = spans.split_first() else {
                    continue;
                };
                let mut line =
                    parent.spawn(text_bundle(first, theme, face, code_face, raster, justify));
                // The root text is span zero and each child the next.
                if let Some(doors) = Doors::new(node, doors(spans)) {
                    line.insert(doors);
                }
                line.with_children(|parent| {
                    for span in rest {
                        parent.spawn((
                            TextSpan::new(span.text.clone()),
                            font(span, face, code_face, raster),
                            TextColor(color(span, theme, face, code_face)),
                        ));
                    }
                });
            }
            Block::Code(code) => {
                parent
                    .spawn((
                        Node {
                            padding: UiRect::all(px(8.0 * raster)),
                            border_radius: BorderRadius::all(px(4.0 * raster)),
                            ..default()
                        },
                        BackgroundColor(theme.color(BACKGROUND).with_alpha(WELL)),
                    ))
                    .with_child((
                        Text::new(code.clone()),
                        TextFont {
                            font: code_face.source.clone(),
                            font_smoothing: code_face.smoothing,
                            ..TextFont::from_font_size(CODE)
                        },
                        TextColor(code_face.ink),
                        wrap(Justify::Left),
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
                                        padding: UiRect::axes(px(8.0 * raster), px(4.0 * raster)),
                                        border: UiRect::all(px(1.0 * raster)),
                                        ..default()
                                    },
                                    BorderColor::all(theme.color(PRIMARY)),
                                ))
                                .with_child((
                                    Text::new(cell.text.clone()),
                                    font(
                                        &Span {
                                            size: CODE,
                                            bold: cell.head,
                                            ..default()
                                        },
                                        face,
                                        code_face,
                                        raster,
                                    ),
                                    TextColor(theme.color(TEXT)),
                                    wrap(Justify::Left),
                                ));
                        }
                    });
            }
        }
    }
}

fn doors(spans: &[Span]) -> Vec<(usize, String)> {
    spans
        .iter()
        .enumerate()
        .filter_map(|(at, span)| Some((at, span.space.clone()?)))
        .collect()
}

fn text_bundle(
    span: &Span,
    theme: &Theme,
    face: &Face,
    code_face: &Face,
    raster: f32,
    justify: Justify,
) -> (Text, TextFont, TextColor, TextLayout) {
    (
        Text::new(span.text.clone()),
        font(span, face, code_face, raster),
        TextColor(color(span, theme, face, code_face)),
        wrap(justify),
    )
}

// A URL or a long identifier is otherwise drawn out of the node and clipped.
pub(super) fn wrap(justify: Justify) -> TextLayout {
    TextLayout {
        justify,
        linebreak: LineBreak::WordOrCharacter,
    }
}

fn font(span: &Span, face: &Face, code_face: &Face, raster: f32) -> TextFont {
    let drawn = if span.mono { code_face } else { face };
    TextFont {
        font: drawn.source.clone(),
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
        font_smoothing: drawn.smoothing,
        ..TextFont::from_font_size(span.size * raster)
    }
}

// Left, centred, or right, by the node's own `textAlign`.
pub(super) fn justify(extra: &serde_json::Map<String, serde_json::Value>) -> Justify {
    match extboard_core::align(extra) {
        Some(1) => Justify::Center,
        Some(2) => Justify::Right,
        _ => Justify::Left,
    }
}

// A link and a code span are the runs that are not body text, so they take the
// two slots that are not the node's own. The body takes the node's stroke.
pub(super) fn color(span: &Span, theme: &Theme, face: &Face, code_face: &Face) -> Color {
    if span.space.is_some() {
        theme.slot(ACCENT_B, ACCENT)
    } else if span.link {
        theme.color(PRIMARY)
    } else if span.mono {
        code_face.ink
    } else {
        face.ink
    }
}

#[cfg(test)]
mod tests;
