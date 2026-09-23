use crate::error::MutationError;
use crate::{Canvas, Edge, Node, NodeKind};

impl Canvas {
    pub fn remove_node(&mut self, node_id: &str) -> Result<(), MutationError> {
        let before = self.nodes.len();
        self.nodes.retain(|node| node.id != node_id);

        if self.nodes.len() == before {
            return Err(MutationError::NoSuchNode(node_id.to_owned()));
        }

        self.edges
            .retain(|edge| edge.from_node != node_id && edge.to_node != node_id);

        Ok(())
    }

    pub fn add_node(&mut self, node: Node) -> Result<(), MutationError> {
        if self.nodes.iter().any(|existing| existing.id == node.id) {
            return Err(MutationError::DuplicateNodeId(node.id));
        }

        self.nodes.push(node);
        Ok(())
    }

    pub fn move_node(&mut self, node_id: &str, x: i64, y: i64) -> Result<(), MutationError> {
        let node = self.node_mut(node_id)?;
        node.x = x;
        node.y = y;
        Ok(())
    }

    pub fn resize_node(
        &mut self,
        node_id: &str,
        width: i64,
        height: i64,
    ) -> Result<(), MutationError> {
        if width <= 0 || height <= 0 {
            return Err(MutationError::InvalidSize {
                node: node_id.to_owned(),
                width,
                height,
            });
        }

        let node = self.node_mut(node_id)?;
        node.width = width;
        node.height = height;
        Ok(())
    }

    pub fn set_text(&mut self, node_id: &str, text: String) -> Result<(), MutationError> {
        match &mut self.node_mut(node_id)?.kind {
            NodeKind::Text { text: existing } => {
                *existing = text;
                Ok(())
            }
            _ => Err(MutationError::NotATextNode(node_id.to_owned())),
        }
    }

    pub fn add_edge(&mut self, edge: Edge) -> Result<(), MutationError> {
        if self.edges.iter().any(|existing| existing.id == edge.id) {
            return Err(MutationError::DuplicateEdgeId(edge.id));
        }

        for endpoint in [&edge.from_node, &edge.to_node] {
            if !self.nodes.iter().any(|node| &node.id == endpoint) {
                return Err(MutationError::NoSuchNode(endpoint.clone()));
            }
        }

        self.edges.push(edge);
        Ok(())
    }

    pub fn remove_edge(&mut self, edge_id: &str) -> Result<(), MutationError> {
        let before = self.edges.len();
        self.edges.retain(|edge| edge.id != edge_id);

        if self.edges.len() == before {
            return Err(MutationError::NoSuchEdge(edge_id.to_owned()));
        }

        Ok(())
    }

    // Three mutations need the same lookup-or-fail, so it lives once here.
    fn node_mut(&mut self, node_id: &str) -> Result<&mut Node, MutationError> {
        self.nodes
            .iter_mut()
            .find(|node| node.id == node_id)
            .ok_or_else(|| MutationError::NoSuchNode(node_id.to_owned()))
    }
}
