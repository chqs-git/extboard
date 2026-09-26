use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;

const ZOOM_MIN: f32 = 0.05;
const ZOOM_MAX: f32 = 20.0;
const ZOOM_PER_NOTCH: f32 = 1.2;
const ZOOM_SMOOTHING: f32 = 25.0;

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_camera)
            .add_systems(Update, (pan, zoom));
    }
}

#[derive(Component)]
struct ZoomTarget(f32);

fn spawn_camera(mut commands: Commands) {
    commands.spawn((Camera2d, ZoomTarget(1.0)));
}

fn pan(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    camera: Single<(&mut Transform, &Projection), With<Camera2d>>,
) {
    let dragging = buttons.pressed(MouseButton::Middle)
        || (keys.pressed(KeyCode::Space) && buttons.pressed(MouseButton::Left));
    if !dragging || motion.delta == Vec2::ZERO {
        return;
    }

    let (mut transform, projection) = camera.into_inner();
    let Projection::Orthographic(ortho) = projection else {
        return;
    };

    // The content follows the cursor, so the camera moves the opposite way.
    transform.translation.x -= motion.delta.x * ortho.scale;
    transform.translation.y += motion.delta.y * ortho.scale;
}

fn zoom(
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    window: Single<&Window>,
    camera: Single<(&mut Transform, &mut Projection, &mut ZoomTarget), With<Camera2d>>,
) {
    let (mut transform, mut projection, mut target) = camera.into_inner();
    let Projection::Orthographic(ortho) = &mut *projection else {
        return;
    };

    let notches = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / 50.0,
    };
    if notches != 0.0 {
        target.0 = (target.0 * ZOOM_PER_NOTCH.powf(-notches)).clamp(ZOOM_MIN, ZOOM_MAX);
    }

    let old = ortho.scale;
    if (target.0 - old).abs() < 1e-4 {
        return;
    }

    // Frame-rate independent exponential ease
    let new = old + (target.0 - old) * (1.0 - (-ZOOM_SMOOTHING * time.delta_secs()).exp());
    ortho.scale = new;

    // Re-anchor every frame of the ramp, at wherever the cursor is now.
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let cam = transform.translation.truncate();
    let anchor = cursor_world(cursor, window.size(), cam, old);
    transform.translation = zoom_anchored(cam, anchor, old, new).extend(transform.translation.z);
}

// Cursor pixel → world, from the camera's *current* transform and scale.
fn cursor_world(cursor: Vec2, viewport: Vec2, cam: Vec2, scale: f32) -> Vec2 {
    let offset = (cursor - viewport / 2.0) * scale;
    cam + Vec2::new(offset.x, -offset.y)
}

fn zoom_anchored(cam: Vec2, anchor: Vec2, old: f32, new: f32) -> Vec2 {
    anchor - (anchor - cam) * (new / old)
}

#[expect(dead_code, reason = "E5 hit-tests with it")]
pub fn screen_to_world(
    camera: &Camera,
    cam_global: &GlobalTransform,
    screen: Vec2,
) -> Option<Vec2> {
    camera.viewport_to_world_2d(cam_global, screen).ok()
}

#[expect(dead_code, reason = "E3-T6 anchors node content with it")]
pub fn world_to_screen(camera: &Camera, cam_global: &GlobalTransform, world: Vec2) -> Option<Vec2> {
    camera.world_to_viewport(cam_global, world.extend(0.0)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The one thing worth testing here: the point under the cursor does not
    // move. Its pixel offset from the camera is what must stay fixed.
    #[test]
    fn zoom_keeps_the_anchor_under_the_cursor() {
        let cam = Vec2::new(-70.0, 40.0);
        let anchor = Vec2::new(250.0, -90.0);

        for (old, new) in [(1.0, 0.5), (1.0, 2.0), (0.25, 4.0)] {
            let moved = zoom_anchored(cam, anchor, old, new);
            let before = (anchor - cam) / old;
            let after = (anchor - moved) / new;
            assert!(
                (before - after).length() < 1e-4,
                "anchor drifted: {before} -> {after}"
            );
        }
    }

    #[test]
    fn cursor_world_matches_the_viewport_corners() {
        let viewport = Vec2::new(800.0, 600.0);
        let cam = Vec2::new(100.0, -50.0);

        // Centre pixel is the camera itself, whatever the scale.
        assert_eq!(cursor_world(viewport / 2.0, viewport, cam, 3.0), cam);
        // Top-left pixel: half a viewport left and, since screen y is down,
        // half a viewport *up* in world space.
        assert_eq!(
            cursor_world(Vec2::ZERO, viewport, cam, 2.0),
            cam + Vec2::new(-800.0, 600.0)
        );
    }

    #[test]
    fn zooming_out_and_back_returns_the_camera() {
        let cam = Vec2::new(12.0, -34.0);
        let anchor = Vec2::new(500.0, 500.0);
        let out = zoom_anchored(cam, anchor, 1.0, 4.0);
        let back = zoom_anchored(out, anchor, 4.0, 1.0);
        assert!((back - cam).length() < 1e-4, "{back} != {cam}");
    }
}
