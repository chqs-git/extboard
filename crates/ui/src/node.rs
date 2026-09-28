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

#[derive(Component)]
pub struct NodeKind(pub extboard_core::NodeKind);

impl Plugin for NodePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            spawn_nodes.run_if(resource_exists_and_changed::<Document>),
        );
    }
}

/// Canvas space -> Bevy world: JSONCanvas +y is down and (x, y) is a node's
/// top-left corner; Bevy +y is up and a Transform is the centre of the mesh.
/// Every conversion goes through here, so there is one place to be wrong.
pub fn to_world(canvas: Vec2) -> Vec2 {
    Vec2::new(canvas.x, -canvas.y)
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

    for node in &document.0.nodes {
        commands.spawn((
            NodeId(node.id.clone()),
            NodeRect {
                w: node.width,
                h: node.height,
            },
            NodeKind(node.kind.clone()),
            Mesh2d(meshes.add(Rectangle::new(node.width as f32, node.height as f32))),
            MeshMaterial2d(materials.add(node_color(&node.kind))),
            Transform::from_translation(
                to_world(Vec2::new(
                    node.x as f32 + node.width as f32 / 2.0,
                    node.y as f32 + node.height as f32 / 2.0,
                ))
                .extend(depth(node)),
            ),
        ));
    }
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
