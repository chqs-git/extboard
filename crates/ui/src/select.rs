use bevy::diagnostic::FrameCount;
use bevy::prelude::*;

use crate::camera::screen_to_world;
use crate::client::Document;
use crate::edit::{MIN_SIZE, Reach, TIP_PX, anchor_under, created, rects, tip_under};
use crate::node::NodeRect;
use crate::scene::{EdgeId, nearest};

// scene.rs paints a selected edge with it.
pub const OUTLINE: Color = Color::srgb(0.95, 0.75, 0.30);
// How near the cursor has to be to hit an edge, in screen pixels. text.rs asks
// the same question when a double-click opens a label.
pub const EDGE_PX: f32 = 8.0;
const BAND: Color = Color::srgb(0.55, 0.65, 0.80);
// Shift turns the sweep into the outline of a new node: a white edge over the
// wash that shows the node's footprint.
const BAND_CREATE: Color = Color::WHITE;
// The knob: over empty canvas there is nothing behind the wash, so the alpha is
// only how light the block reads. Sweep it over a node to see it blend.
const BAND_FILL: Color = Color::srgba(0.55, 0.65, 0.80, 0.1);
// Above the node rects and the edge labels: it is an overlay on all of them.
const BAND_Z: f32 = 2.0;

pub struct SelectPlugin;

#[derive(Component)]
pub struct Selected;

// Gizmos draw lines, not fills, so the wash is one quad, the same unit mesh
// node.rs scales. It lives for the whole run and hides itself: spawning per
// gesture would lag a frame behind.
#[derive(Component)]
struct BandFill;

#[derive(Resource)]
struct Band {
    start: Vec2,
    base: Vec<Entity>,
    // Shift already means additive, so a band that began with it stays a sweep.
    additive: bool,
    // The rect to become a node on release, while shift is down.
    creating: Option<Rect>,
}

impl Plugin for SelectPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_fill)
            // `press` only: a click that ends an edit session is spent on that,
            // and no band can exist while one is open.
            .add_systems(
                Update,
                (
                    press.run_if(not(crate::text::editing)),
                    drag_band,
                    release,
                    outline,
                )
                    .chain(),
            );
    }
}

fn spawn_fill(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    commands.spawn((
        BandFill,
        Mesh2d(meshes.add(Rectangle::from_length(1.0))),
        // ColorMaterial reads the alpha and blends; a Sprite draws it opaque.
        MeshMaterial2d(materials.add(BAND_FILL)),
        Transform::from_xyz(0.0, 0.0, BAND_Z),
        Visibility::Hidden,
    ));
}

#[allow(
    clippy::too_many_arguments,
    reason = "a system's arguments are its query"
)]
fn press(
    mut commands: Commands,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform, &Projection), With<Camera2d>>,
    nodes: Query<(Entity, &Transform, &NodeRect)>,
    edges: Query<(Entity, &EdgeId)>,
    document: Option<Res<Document>>,
    selected: Query<Entity, With<Selected>>,
) {
    let (camera, cam_global, projection) = *camera;
    // Space+left is the camera's pan grab.
    if !buttons.just_pressed(MouseButton::Left) || keys.pressed(KeyCode::Space) {
        return;
    }
    let Some(world) = cursor_world(&window, (camera, cam_global)) else {
        return;
    };
    let Projection::Orthographic(ortho) = projection else {
        return;
    };
    // A press on the anchor circle itself is edit.rs's edge drag, not a sweep.
    if anchor_under(rects(&nodes), projection, world, Reach::Grip).is_some() {
        return;
    }
    // Same for the arrowhead on the end of an edge: that press is a redirect.
    if document
        .as_deref()
        .is_some_and(|document| tip_under(&document.0, world, TIP_PX * ortho.scale).is_some())
    {
        return;
    }
    let add = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);

    if let Some(entity) = pick(
        nodes.iter().map(|(entity, transform, rect)| {
            (entity, bounds(transform, rect), transform.translation.z)
        }),
        world,
    ) {
        return picked(&mut commands, &selected, add, entity);
    }

    // An edge is a thin target, so it is tried where no node was hit. A press on
    // one is not a drag, and never starts a band.
    if let Some(entity) = document
        .as_deref()
        .and_then(|document| nearest(&document.0, world, EDGE_PX * ortho.scale))
        .and_then(|id| {
            edges
                .iter()
                .find(|(_, edge)| edge.0 == id)
                .map(|(entity, _)| entity)
        })
    {
        return picked(&mut commands, &selected, add, entity);
    }

    let base: Vec<Entity> = if add {
        selected.iter().collect()
    } else {
        Vec::new()
    };
    replace(&mut commands, &selected, &base);
    commands.insert_resource(Band {
        start: world,
        base,
        additive: add,
        creating: None,
    });
}

#[allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    reason = "a system's arguments are its query"
)]
fn drag_band(
    mut commands: Commands,
    mut gizmos: Gizmos,
    keys: Res<ButtonInput<KeyCode>>,
    band: Option<ResMut<Band>>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    nodes: Query<(Entity, &Transform, &NodeRect)>,
    selected: Query<Entity, With<Selected>>,
    // Without<NodeRect>: `nodes` wants a Transform too, and only one of the two
    // may have it mutably.
    fill: Single<(&mut Transform, &mut Visibility), (With<BandFill>, Without<NodeRect>)>,
) {
    let (mut fill_transform, mut fill_visibility) = fill.into_inner();
    let Some(mut band) = band else {
        fill_visibility.set_if_neq(Visibility::Hidden);
        return;
    };
    let Some(world) = cursor_world(&window, *camera) else {
        return;
    };
    let rect = Rect::from_corners(band.start, world);
    let creating = !band.additive && keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    gizmos.rect_2d(
        Isometry2d::from_translation(rect.center()),
        rect.size(),
        if creating { BAND_CREATE } else { BAND },
    );
    if creating {
        fill_transform.translation = rect.center().extend(BAND_Z);
        fill_transform.scale = rect.size().extend(1.0);
    }
    fill_visibility.set_if_neq(if creating {
        Visibility::Visible
    } else {
        Visibility::Hidden
    });

    // Drawing a node, not sweeping one up: leave the selection where it was, so
    // letting shift go puts the sweep back exactly as it stood.
    band.creating = creating.then_some(rect);
    if creating {
        return;
    }

    // A despawn between frames would leave a dangling id in `base`.
    let mut keep: Vec<Entity> = band
        .base
        .iter()
        .copied()
        .filter(|e| nodes.contains(*e))
        .collect();
    keep.extend(
        nodes
            .iter()
            .filter(|(_, transform, node)| !bounds(transform, node).intersect(rect).is_empty())
            .map(|(entity, _, _)| entity),
    );
    replace(&mut commands, &selected, &keep);
}

fn release(
    mut commands: Commands,
    buttons: Res<ButtonInput<MouseButton>>,
    frames: Res<FrameCount>,
    band: Option<Res<Band>>,
    document: Option<ResMut<Document>>,
) {
    let Some(band) = band else {
        return;
    };
    if buttons.pressed(MouseButton::Left) {
        return;
    }
    if let (Some(rect), Some(mut document)) = (band.creating, document)
        && big_enough(rect)
    {
        created(&mut document.0, frames.0, rect);
    }
    commands.remove_resource::<Band>();
}

// A band this small is a slipped click, not a node someone drew.
fn big_enough(rect: Rect) -> bool {
    rect.size().min_element() >= MIN_SIZE
}

fn outline(mut gizmos: Gizmos, selected: Query<(&Transform, &NodeRect), With<Selected>>) {
    for (transform, node) in &selected {
        let rect = bounds(transform, node);
        gizmos.rect_2d(
            Isometry2d::from_translation(rect.center()),
            rect.size(),
            OUTLINE,
        );
    }
}

fn picked(
    commands: &mut Commands,
    selected: &Query<Entity, With<Selected>>,
    add: bool,
    entity: Entity,
) {
    match (add, selected.contains(entity)) {
        (true, true) => {
            commands.entity(entity).remove::<Selected>();
        }
        (true, false) => {
            commands.entity(entity).insert(Selected);
        }
        // Already selected: leave the rest of the selection alone, the press is
        // the start of dragging all of it.
        (false, true) => {}
        (false, false) => replace(commands, selected, &[entity]),
    }
}

fn replace(commands: &mut Commands, selected: &Query<Entity, With<Selected>>, keep: &[Entity]) {
    for entity in selected {
        if !keep.contains(&entity) {
            commands.entity(entity).remove::<Selected>();
        }
    }
    for &entity in keep {
        commands.entity(entity).insert(Selected);
    }
}

pub fn cursor_world(
    window: &Window,
    (camera, cam_global): (&Camera, &GlobalTransform),
) -> Option<Vec2> {
    screen_to_world(camera, cam_global, window.cursor_position()?)
}

pub fn bounds(transform: &Transform, node: &NodeRect) -> Rect {
    Rect::from_center_size(transform.translation.truncate(), node.size())
}

pub fn pick<T>(candidates: impl Iterator<Item = (T, Rect, f32)>, point: Vec2) -> Option<T> {
    candidates
        .filter(|(_, rect, _)| rect.contains(point))
        .max_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(item, _, _)| item)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, w: f32) -> Rect {
        Rect::from_center_size(Vec2::new(x, 0.0), Vec2::splat(w))
    }

    #[test]
    fn pick_takes_the_front_of_the_overlap_and_nothing_outside() {
        // A small card sitting on a big group: same point, the card wins.
        let group = (0usize, rect(0.0, 400.0), -0.16);
        let card = (1usize, rect(0.0, 100.0), -0.01);
        assert_eq!(pick([group, card].into_iter(), Vec2::ZERO), Some(1));
        assert_eq!(pick([card, group].into_iter(), Vec2::ZERO), Some(1));
        // Outside the card, only the group is under the cursor.
        assert_eq!(
            pick([group, card].into_iter(), Vec2::new(150.0, 0.0)),
            Some(0)
        );
        assert_eq!(pick([card].into_iter(), Vec2::new(150.0, 0.0)), None);
    }

    // Both queries want a Transform and one wants it mutably; Bevy panics on
    // that at system init, which is a running app, not a compile.
    #[test]
    fn the_fill_and_the_node_queries_stay_disjoint() {
        let mut app = App::new();
        // Nothing this system reads exists here, so every parameter fails
        // validation. Init is what is under test, and it runs first.
        app.set_error_handler(bevy::ecs::error::ignore)
            .add_systems(Update, drag_band);
        app.world_mut().run_schedule(Update);
    }

    #[test]
    fn only_a_band_worth_drawing_becomes_a_node() {
        assert!(big_enough(Rect::from_corners(
            Vec2::ZERO,
            Vec2::splat(MIN_SIZE)
        )));
        // A slipped click, and a sliver too thin to hold text.
        assert!(!big_enough(Rect::from_corners(
            Vec2::ZERO,
            Vec2::splat(2.0)
        )));
        assert!(!big_enough(Rect::from_corners(
            Vec2::ZERO,
            Vec2::new(400.0, 5.0)
        )));
    }

    #[test]
    fn a_band_catches_what_it_touches_and_a_click_catches_nothing() {
        let band = Rect::from_corners(Vec2::new(-30.0, -30.0), Vec2::new(30.0, 30.0));
        assert!(!rect(60.0, 100.0).intersect(band).is_empty());
        assert!(rect(200.0, 100.0).intersect(band).is_empty());

        // Press and release on the same pixel: zero area, so no sweep.
        let click = Rect::from_corners(Vec2::ZERO, Vec2::ZERO);
        assert!(rect(0.0, 100.0).intersect(click).is_empty());
    }
}
