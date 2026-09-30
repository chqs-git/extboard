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
    document: Res<Document>,
    existing: Query<Entity, With<NodeId>>,
) {
    // despawn-all then respawn-all.
    for entity in &existing {
        commands.entity(entity).despawn();
    }

    // The model of a node, not its picture: where it is and how big, for hit
    // testing and for the panel that draws it.
    for node in &document.0.nodes {
        commands.spawn((
            NodeId(node.id.clone()),
            NodeRect {
                w: node.width,
                h: node.height,
            },
            NodeKind(node.kind.clone()),
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
pub fn depth(node: &extboard_core::Node) -> f32 {
    -(node.width as f32 * node.height as f32) / 1.0e6
}

// The spec's colour field if the node carries one, and otherwise enough of a
// palette to tell the four kinds apart.
pub fn node_color(node: &extboard_core::Node) -> Color {
    node.color
        .as_deref()
        .and_then(spec_color)
        .unwrap_or_else(|| kind_color(&node.kind))
}

// Obsidian's own picker writes a preset index; the spec permits `#rrggbb` too,
// and anything else is a colour we do not know, so the kind decides instead.
fn spec_color(color: &str) -> Option<Color> {
    if let Some(hex) = color.strip_prefix('#') {
        let hex = u32::from_str_radix(hex, 16)
            .ok()
            .filter(|_| hex.len() == 6)?;
        return Some(Color::srgb_u8(
            (hex >> 16) as u8,
            (hex >> 8) as u8,
            hex as u8,
        ));
    }
    // Obsidian's canvas presets, in its own order.
    Some(match color {
        "1" => Color::srgb_u8(0xfb, 0x46, 0x4c),
        "2" => Color::srgb_u8(0xe9, 0x97, 0x3f),
        "3" => Color::srgb_u8(0xe0, 0xde, 0x71),
        "4" => Color::srgb_u8(0x44, 0xcf, 0x6e),
        "5" => Color::srgb_u8(0x53, 0xdf, 0xdd),
        "6" => Color::srgb_u8(0xa8, 0x82, 0xff),
        _ => return None,
    })
}

fn kind_color(kind: &extboard_core::NodeKind) -> Color {
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
    fn a_colour_is_a_preset_index_a_hex_code_or_neither() {
        assert_eq!(spec_color("4"), Some(Color::srgb_u8(0x44, 0xcf, 0x6e)));
        assert_eq!(
            spec_color("#1a2b3c"),
            Some(Color::srgb_u8(0x1a, 0x2b, 0x3c))
        );
        // Not ours to guess: the kind decides.
        assert_eq!(spec_color("7"), None);
        assert_eq!(spec_color("#abc"), None);
        assert_eq!(spec_color("#nothex"), None);
        assert_eq!(spec_color("rebeccapurple"), None);
    }

    #[test]
    fn to_canvas_undoes_the_placement_of_a_node() {
        let size = Vec2::new(240.0, 90.0);
        for top_left in [Vec2::ZERO, Vec2::new(-130.0, 40.0), Vec2::new(70.0, -500.0)] {
            let center = to_world(top_left + size / 2.0);
            assert_eq!(to_canvas(center, size), top_left);
        }
    }
}
