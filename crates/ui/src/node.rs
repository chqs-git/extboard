use bevy::prelude::*;

use crate::client::Document;

pub struct NodePlugin;

#[derive(Component)]
pub struct NodeId(pub String);

#[derive(Component)]
pub struct NodeRect {
    pub w: i64,
    pub h: i64,
}

impl NodeRect {
    pub fn size(&self) -> Vec2 {
        Vec2::new(self.w as f32, self.h as f32)
    }
}

#[derive(Component)]
pub struct NodeKind(pub extboard_core::NodeKind);

impl Plugin for NodePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                spawn_nodes.run_if(resource_exists_and_changed::<Document>),
                place_nodes.run_if(resource_exists::<Document>),
            ),
        );
    }
}

pub fn to_world(canvas: Vec2) -> Vec2 {
    Vec2::new(canvas.x, -canvas.y)
}

pub fn to_canvas(center: Vec2, size: Vec2) -> Vec2 {
    to_world(center) - size / 2.0
}

// trigger on canvas changes; sync nodes
fn spawn_nodes(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    document: Res<Document>,
    existing: Query<Entity, With<NodeId>>,
) {
    // despawn-all then respawn-all.
    for entity in &existing {
        commands.entity(entity).despawn();
    }

    // One unit quad for every node: a resize is then a scale, not a new mesh.
    let quad = meshes.add(Rectangle::from_length(1.0));

    for node in &document.0.nodes {
        commands.spawn((
            NodeId(node.id.clone()),
            NodeRect {
                w: node.width,
                h: node.height,
            },
            NodeKind(node.kind.clone()),
            Mesh2d(quad.clone()),
            MeshMaterial2d(materials.add(node_color(&node.kind))),
            placement(node),
        ));
    }
}

fn place_nodes(
    document: Res<Document>,
    mut nodes: Query<(&NodeId, &mut Transform, &mut NodeRect)>,
) {
    for (id, mut transform, mut rect) in &mut nodes {
        let Some(node) = document.0.nodes.iter().find(|node| node.id == id.0) else {
            continue;
        };
        let want = placement(node);
        if *transform != want {
            *transform = want;
        }
        if (rect.w, rect.h) != (node.width, node.height) {
            (rect.w, rect.h) = (node.width, node.height);
        }
    }
}

fn placement(node: &extboard_core::Node) -> Transform {
    let size = Vec2::new(node.width as f32, node.height as f32);
    Transform::from_translation(
        to_world(Vec2::new(node.x as f32, node.y as f32) + size / 2.0).extend(depth(node)),
    )
    .with_scale(size.extend(1.0))
}

// Bigger rects sit behind smaller ones, so a group never hides what is inside
// it. All within (-1, 0), leaving z=1 for edge labels.
fn depth(node: &extboard_core::Node) -> f32 {
    -(node.width as f32 * node.height as f32) / 1.0e6
}

// Placeholder palette: enough to tell the four kinds apart. The real one is
// E6 — `node.color` is a palette index string, not a hex code.
fn node_color(kind: &extboard_core::NodeKind) -> Color {
    match kind {
        extboard_core::NodeKind::Text { .. } => Color::hsl(210.0, 0.45, 0.58),
        extboard_core::NodeKind::File { .. } => Color::hsl(150.0, 0.40, 0.48),
        extboard_core::NodeKind::Link { .. } => Color::hsl(285.0, 0.40, 0.60),
        extboard_core::NodeKind::Group { .. } => Color::hsl(220.0, 0.15, 0.26),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_canvas_undoes_the_placement_of_a_node() {
        let size = Vec2::new(240.0, 90.0);
        for top_left in [Vec2::ZERO, Vec2::new(-130.0, 40.0), Vec2::new(70.0, -500.0)] {
            let center = to_world(top_left + size / 2.0);
            assert_eq!(to_canvas(center, size), top_left);
        }
    }
}
