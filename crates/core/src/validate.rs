use std::collections::HashSet;

use crate::Canvas;
use crate::error::ValidationError;

// validate the following rules on a [canvas]:
// -> edge {edge} points at node {node}, which does not exist
// -> node id {0} appears more than once
// -> edge id {0} appears more than once
pub fn validate(canvas: &Canvas) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();

    let mut node_ids = HashSet::with_capacity(canvas.nodes.len());
    for node in &canvas.nodes {
        if !node_ids.insert(node.id.as_str()) {
            errors.push(ValidationError::DuplicateNodeId(node.id.clone()));
        }
    }

    let mut edge_ids = HashSet::with_capacity(canvas.edges.len());
    for edge in &canvas.edges {
        if !edge_ids.insert(edge.id.as_str()) {
            errors.push(ValidationError::DuplicateEdgeId(edge.id.clone()));
        }
        // both endpoints, so an edge dangling at each end reports twice
        for endpoint in [&edge.from_node, &edge.to_node] {
            if !node_ids.contains(endpoint.as_str()) {
                errors.push(ValidationError::DanglingEdge {
                    edge: edge.id.clone(),
                    node: endpoint.clone(),
                });
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
