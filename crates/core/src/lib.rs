#![forbid(unsafe_code)]

pub mod error;
pub mod geometry;
pub mod model;
pub mod mutate;
pub mod rev;
pub mod style;
pub mod theme;
pub mod validate;

pub use error::{MutationError, ValidationError};
pub use geometry::{edge_ends, side_anchor, side_facing, sides_inset, sides_polygon};
pub use model::{
    CIRCLE_SIDES, Canvas, Edge, End, MIN_SIDES, Node, NodeKind, Side, is_font, is_image,
    object_mut, sides_of, url_path,
};
pub use mutate::{PRESET_SLOTS, is_canvas_color};
pub use rev::{fresh_id, rev};
pub use style::{LEFT_ALIGN, MID_STROKE, STROKES, align, stroke_scale, stroke_width};
pub use theme::{
    ACCENT, BACKGROUND, DEFAULT_FONT, FONT_ROLES, Fonts, MAX_COLORS, MIN_COLORS, PRESETS, PRIMARY,
    PRIMARY_TEXT, ROLES, SECONDARY, SECONDARY_TEXT, TEXT, Theme,
};
pub use validate::validate;
