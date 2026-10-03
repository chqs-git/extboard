use bevy::prelude::*;
use bevy::text::TextLayoutInfo;
use extboard_core::{End, PRIMARY_TEXT, SECONDARY, TEXT};
use std::collections::HashMap;

use crate::client::Document;
use crate::select::{OUTLINE, Selected};
use crate::theme::Theme;

mod arrow;
mod geometry;

pub use arrow::draw_arrow;
use geometry::{edge_ends, resolved_edges};
pub use geometry::{nearest, segments};

const LABEL_SIZE: f32 = 12.0;
// Above the node rects, which sit at 0.
const LABEL_Z: f32 = 1.0;

pub struct ScenePlugin;

#[derive(Component)]
pub struct EdgeId(pub String);

impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup);
        app.add_systems(
            Update,
            (
                spawn_edges
                    .in_set(crate::node::Respawn)
                    .run_if(resource_exists_and_changed::<Document>),
                // Every frame, like the edges themselves: a drag moves nodes
                // without waking change detection, and the label has to keep up.
                place_edges.run_if(resource_exists::<Document>),
                draw_edges.run_if(resource_exists::<Document>),
            )
                .chain(),
        );
    }
}

fn setup(mut store: ResMut<GizmoConfigStore>) {
    store.config_mut::<DefaultGizmoConfigGroup>().0.line.width = 3.0;
}

fn draw_edges(
    mut gizmos: Gizmos,
    document: Res<Document>,
    theme: Res<Theme>,
    selected: Query<&EdgeId, With<Selected>>,
    // Its size is what the shaft makes room for: measured, not guessed.
    labels: Query<(&EdgeId, &TextLayoutInfo)>,
) {
    for (edge, from, to) in resolved_edges(&document.0) {
        let (a, b) = edge_ends(from, edge.from_side, to, edge.to_side);
        let color = if selected.iter().any(|picked| picked.0 == edge.id) {
            OUTLINE
        } else {
            theme.color(SECONDARY)
        };
        let heads = (
            edge.from_end.unwrap_or(End::None) == End::Arrow,
            edge.to_end.unwrap_or(End::Arrow) == End::Arrow,
        );
        let hole = labels
            .iter()
            .find(|(id, _)| id.0 == edge.id)
            .map(|(_, label)| label.size);
        draw_arrow(&mut gizmos, a, b, heads, hole, color);
    }
}

fn spawn_edges(
    mut commands: Commands,
    document: Res<Document>,
    theme: Res<Theme>,
    existing: Query<(Entity, &EdgeId, Has<Selected>)>,
) {
    let mut selected: Vec<&str> = Vec::new();
    for (entity, id, picked) in &existing {
        if picked {
            selected.push(id.0.as_str());
        }
        commands.entity(entity).despawn();
    }

    for (edge, from, to) in resolved_edges(&document.0) {
        let (a, b) = edge_ends(from, edge.from_side, to, edge.to_side);
        let mut spawned = commands.spawn((
            EdgeId(edge.id.clone()),
            Transform::from_translation(a.midpoint(b).extend(LABEL_Z)),
        ));
        if selected.contains(&edge.id.as_str()) {
            spawned.insert(Selected);
        }
        if let Some(label) = &edge.label {
            spawned.insert((
                Text2d::new(label.clone()),
                TextFont {
                    font: theme.text_font(PRIMARY_TEXT),
                    font_smoothing: theme.smoothing(),
                    ..TextFont::from_font_size(LABEL_SIZE)
                },
                TextColor(theme.color(TEXT)),
            ));
        }
    }
}

fn place_edges(document: Res<Document>, mut edges: Query<(&EdgeId, &mut Transform)>) {
    let placed: HashMap<&str, Vec3> = segments(&document.0)
        .map(|(id, a, b)| (id, a.midpoint(b).extend(LABEL_Z)))
        .collect();

    for (id, mut transform) in &mut edges {
        let Some(&want) = placed.get(id.0.as_str()) else {
            continue;
        };
        if transform.translation != want {
            transform.translation = want;
        }
    }
}

#[cfg(test)]
mod tests;
