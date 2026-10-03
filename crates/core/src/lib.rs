#![forbid(unsafe_code)]

pub mod error;
pub mod geometry;
pub mod model;
pub mod mutate;
pub mod rev;
pub mod validate;

pub use error::{MutationError, ValidationError};
pub use geometry::{edge_ends, side_anchor, side_facing, sides_inset, sides_polygon};
pub use model::{CIRCLE_SIDES, Canvas, Edge, End, MIN_SIDES, Node, NodeKind, Side, sides_of};
pub use mutate::is_canvas_color;
pub use rev::{fresh_id, rev};
pub use validate::validate;
