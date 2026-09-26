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
            MeshMaterial2d(materials.add(Color::hsl(360., 0.95, 0.7))),
            Transform::from_xyz(
                node.x as f32 + node.width as f32 / 2.0,
                -(node.y as f32 + node.height as f32 / 2.0),
                0.0,
            ),
        ));
    }
}
