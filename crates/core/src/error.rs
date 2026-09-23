use thiserror::Error;

// canvas validation errors
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("edge {edge} points at node {node}, which does not exist")]
    DanglingEdge { edge: String, node: String },

    #[error("node id {0} appears more than once")]
    DuplicateNodeId(String),

    #[error("edge id {0} appears more than once")]
    DuplicateEdgeId(String),
}

// canvas mutations errors
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MutationError {
    #[error("no node with id {0}")]
    NoSuchNode(String),

    #[error("a node with id {0} is already in the document")]
    DuplicateNodeId(String),

    #[error("no edge with id {0}")]
    NoSuchEdge(String),

    #[error("an edge with id {0} is already in the document")]
    DuplicateEdgeId(String),

    #[error("node {0} is not a text node")]
    NotATextNode(String),

    #[error("{width}x{height} is not a usable size for node {node}")]
    InvalidSize {
        node: String,
        width: i64,
        height: i64,
    },
}
