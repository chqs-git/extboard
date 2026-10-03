use bevy::prelude::*;
use extboard_core::{PRIMARY, SECONDARY, stroke_scale, style};

use crate::client::Document;
use crate::select::Selected;
use crate::theme::Theme;

pub struct NodePlugin;

// The frame's rebuild. Anything that picks an entity and then writes to it runs
// after this, or it writes to one already on its way out, which is a panic.
#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Respawn;

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
                spawn_nodes
                    .in_set(Respawn)
                    .run_if(resource_exists_and_changed::<Document>),
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

fn spawn_nodes(
    mut commands: Commands,
    document: Res<Document>,
    existing: Query<(Entity, &NodeId, Has<Selected>)>,
) {
    // The selection is carried across, or a tick script's write takes it.
    let mut selected: Vec<&str> = Vec::new();
    for (entity, id, picked) in &existing {
        if picked {
            selected.push(id.0.as_str());
        }
        commands.entity(entity).despawn();
    }

    // The model of a node, not its picture: for hit testing and for the panel.
    for node in &document.0.nodes {
        let mut spawned = commands.spawn((
            NodeId(node.id.clone()),
            NodeRect {
                w: node.width,
                h: node.height,
            },
            NodeKind(node.kind.clone()),
            placement(node),
        ));
        if selected.contains(&node.id.as_str()) {
            spawned.insert(Selected);
        }
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

// Bigger rects behind smaller, within (-1, 0) so z=1 is left for edge labels.
pub fn depth(node: &extboard_core::Node) -> f32 {
    -(node.width as f32 * node.height as f32) / 1.0e6
}

// A group is a container: what sits inside it, edges included, shows through.
const GROUP_ALPHA: f32 = 0.45;

// The rim at the middle stroke weight, in canvas units.
const OUTLINE: f32 = 2.0;

pub fn body_color(theme: &Theme, node: &extboard_core::Node) -> Color {
    match node.kind {
        extboard_core::NodeKind::Group { .. } => node_color(theme, node).with_alpha(GROUP_ALPHA),
        _ => node_color(theme, node),
    }
}

// The rim: the node's own outline colour when it names one, and the palette's
// primary when it does not. It fades with the body, so a group's rim is
// see-through too.
pub fn outline_color(theme: &Theme, node: &extboard_core::Node) -> Color {
    theme
        .paint(style::outline_color(&node.extra), PRIMARY)
        .with_alpha(body_color(theme, node).alpha())
}

// The rim's width in canvas units, which the shader multiplies by the zoom.
pub fn outline_px(node: &extboard_core::Node) -> f32 {
    OUTLINE * stroke_scale(&node.extra)
}

// The body: the spec's own colour field, which is the node's background.
pub fn node_color(theme: &Theme, node: &extboard_core::Node) -> Color {
    theme.paint(node.color.as_deref(), SECONDARY)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(kind: extboard_core::NodeKind, color: Option<&str>) -> extboard_core::Node {
        extboard_core::Node {
            id: "n".to_owned(),
            x: 0,
            y: 0,
            width: 100,
            height: 100,
            color: color.map(str::to_owned),
            sides: None,
            kind,
            extra: serde_json::Map::new(),
        }
    }

    fn text() -> extboard_core::NodeKind {
        extboard_core::NodeKind::Text {
            text: String::new(),
        }
    }

    fn themed(colors: &[&str]) -> Theme {
        Theme::from_colors(colors)
    }

    #[test]
    fn only_a_group_lets_what_is_inside_it_show_through() {
        let theme = Theme::default();
        let group = body_color(
            &theme,
            &node(extboard_core::NodeKind::Group { label: None }, None),
        );
        let text = body_color(&theme, &node(text(), None));
        assert!(group.alpha() < 1.0, "a group has to be see-through");
        assert_eq!(text.alpha(), 1.0, "everything else has to cover");
    }

    // A short theme wraps rather than leaving a node with no colour.
    #[test]
    fn a_preset_index_is_a_slot_in_the_theme_and_it_wraps() {
        let theme = themed(&["#000000", "#111111", "#222222", "#333333"]);
        let slot = |color| node_color(&theme, &node(text(), Some(color)));
        assert_eq!(slot("1"), Color::srgb_u8(0x11, 0x11, 0x11));
        assert_eq!(slot("3"), Color::srgb_u8(0x33, 0x33, 0x33));
        // Past the end of a short theme, and still a colour.
        assert_eq!(slot("6"), slot("2"));
        for index in ["1", "2", "3", "4", "5", "6"] {
            assert_eq!(slot(index).alpha(), 1.0, "{index} drew as nothing");
        }
    }

    #[test]
    fn a_hex_colour_is_the_nodes_own_and_anything_else_falls_to_the_theme() {
        let theme = themed(&["#000000", "#111111", "#222222", "#333333"]);
        let drawn = |color| node_color(&theme, &node(text(), color));
        assert_eq!(drawn(Some("#1a2b3c")), Color::srgb_u8(0x1a, 0x2b, 0x3c));
        let kind = drawn(None);
        assert_eq!(kind, theme.color(SECONDARY));
        for junk in ["#abc", "#nothex", "rebeccapurple"] {
            assert_eq!(drawn(Some(junk)), kind, "{junk}");
        }
    }

    // The point of the theme: no node is drawn from a constant any more.
    #[test]
    fn changing_the_theme_moves_every_node_that_has_no_colour_of_its_own() {
        let kinds = [
            text(),
            extboard_core::NodeKind::File {
                file: "a.png".to_owned(),
                subpath: None,
            },
            extboard_core::NodeKind::Link {
                url: "https://example.com".to_owned(),
            },
            extboard_core::NodeKind::Group { label: None },
        ];
        let before = themed(&["#000000", "#111111", "#222222", "#333333"]);
        let after = themed(&["#aaaaaa", "#bbbbbb", "#cccccc", "#dddddd"]);
        for kind in kinds {
            let node = node(kind, None);
            assert_ne!(
                body_color(&before, &node),
                body_color(&after, &node),
                "{:?}",
                node.kind
            );
        }
    }

    #[test]
    fn the_outline_is_primary_and_fades_with_the_body() {
        let theme = themed(&["#000000", "#111111", "#222222", "#333333"]);
        let primary = theme.color(PRIMARY);
        let own = node(text(), Some("#1a2b3c"));
        assert_eq!(outline_color(&theme, &own), primary);
        let group = node(extboard_core::NodeKind::Group { label: None }, None);
        let rim = outline_color(&theme, &group);
        assert_eq!(rim.to_srgba().with_alpha(1.0), primary.to_srgba());
        assert_eq!(rim.alpha(), body_color(&theme, &group).alpha());
        assert!(rim.alpha() < 1.0, "a group's rim has to be see-through too");
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
