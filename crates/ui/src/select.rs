use bevy::prelude::*;

use crate::camera::screen_to_world;
use crate::node::NodeRect;

const OUTLINE: Color = Color::srgb(0.95, 0.75, 0.30);
const BAND: Color = Color::srgb(0.55, 0.65, 0.80);

pub struct SelectPlugin;

#[derive(Component)]
pub struct Selected;

#[derive(Resource)]
struct Band {
    start: Vec2,
    base: Vec<Entity>,
}

impl Plugin for SelectPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (press, drag_band, release, outline).chain());
    }
}

fn press(
    mut commands: Commands,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    nodes: Query<(Entity, &Transform, &NodeRect)>,
    selected: Query<Entity, With<Selected>>,
) {
    // Space+left is the camera's pan grab.
    if !buttons.just_pressed(MouseButton::Left) || keys.pressed(KeyCode::Space) {
        return;
    }
    let Some(world) = cursor_world(&window, *camera) else {
        return;
    };
    let add = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);

    if let Some(entity) = pick(
        nodes.iter().map(|(entity, transform, rect)| {
            (entity, bounds(transform, rect), transform.translation.z)
        }),
        world,
    ) {
        match (add, selected.contains(entity)) {
            (true, true) => commands.entity(entity).remove::<Selected>(),
            (true, false) => commands.entity(entity).insert(Selected),
            (false, _) => return replace(&mut commands, &selected, &[entity]),
        };
        return;
    }

    let base: Vec<Entity> = if add {
        selected.iter().collect()
    } else {
        Vec::new()
    };
    replace(&mut commands, &selected, &base);
    commands.insert_resource(Band { start: world, base });
}

fn drag_band(
    mut commands: Commands,
    mut gizmos: Gizmos,
    band: Option<Res<Band>>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    nodes: Query<(Entity, &Transform, &NodeRect)>,
    selected: Query<Entity, With<Selected>>,
) {
    let Some(band) = band else {
        return;
    };
    let Some(world) = cursor_world(&window, *camera) else {
        return;
    };
    let rect = Rect::from_corners(band.start, world);
    gizmos.rect_2d(
        Isometry2d::from_translation(rect.center()),
        rect.size(),
        BAND,
    );

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
    band: Option<Res<Band>>,
) {
    if band.is_some() && !buttons.pressed(MouseButton::Left) {
        commands.remove_resource::<Band>();
    }
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

fn cursor_world(
    window: &Window,
    (camera, cam_global): (&Camera, &GlobalTransform),
) -> Option<Vec2> {
    screen_to_world(camera, cam_global, window.cursor_position()?)
}

fn bounds(transform: &Transform, node: &NodeRect) -> Rect {
    Rect::from_center_size(
        transform.translation.truncate(),
        Vec2::new(node.w as f32, node.h as f32),
    )
}

fn pick<T>(candidates: impl Iterator<Item = (T, Rect, f32)>, point: Vec2) -> Option<T> {
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
