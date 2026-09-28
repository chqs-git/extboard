#![forbid(unsafe_code)]

pub mod error;
pub mod geometry;
pub mod model;
pub mod mutate;
pub mod rev;
pub mod validate;

pub use error::{MutationError, ValidationError};
pub use geometry::{edge_ends, side_anchor, side_facing};
pub use model::{Canvas, Edge, End, Node, NodeKind, Side};
pub use rev::{fresh_id, rev};
pub use validate::validate;
