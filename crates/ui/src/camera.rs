use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::input::touch::Touches;
use bevy::prelude::*;
use extboard_core::{Canvas, Node as CanvasNode};

use crate::client::{Document, Rev, Space};
use crate::node::to_world;

const ZOOM_MIN: f32 = 0.05;
const ZOOM_MAX: f32 = 20.0;
const ZOOM_PER_NOTCH: f32 = 1.2;
const ZOOM_SMOOTHING: f32 = 25.0;

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Arriving>()
            .add_systems(Startup, spawn_camera)
            .add_systems(
                Update,
                (
                    arm.run_if(resource_changed::<Space>),
                    arrive.run_if(resource_exists_and_changed::<Document>),
                    pan,
                    zoom,
                    fingers,
                )
                    .chain(),
            );
    }
}

#[derive(Component)]
struct ZoomTarget(f32);

#[derive(Resource, Default)]
struct Arriving(bool);

fn arm(mut arriving: ResMut<Arriving>) {
    arriving.0 = true;
}

// `switch` empties the document first, so a rev is what says the load landed.
fn arrive(
    mut arriving: ResMut<Arriving>,
    rev: Res<Rev>,
    document: Res<Document>,
    camera: Single<&mut Transform, With<Camera2d>>,
) {
    if !arriving.0 || rev.0.is_empty() {
        return;
    }
    arriving.0 = false;
    let Some(middle) = center(&document.0) else {
        return;
    };
    let mut transform = camera.into_inner();
    transform.translation = middle.extend(transform.translation.z);
}

fn center(canvas: &Canvas) -> Option<Vec2> {
    let box_ = |node: &CanvasNode| {
        Rect::new(
            node.x as f32,
            node.y as f32,
            (node.x + node.width) as f32,
            (node.y + node.height) as f32,
        )
    };
    let bounds = canvas
        .nodes
        .iter()
        .map(box_)
        .reduce(|all, one| all.union(one))?;
    Some(to_world(bounds.center()))
}

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

    drag(&mut transform, motion.delta, ortho.scale);
}

// The content follows the pointer, so the camera moves the opposite way.
fn drag(transform: &mut Transform, delta: Vec2, scale: f32) {
    transform.translation.x -= delta.x * scale;
    transform.translation.y += delta.y * scale;
}

// One finger drags the board, two pinch it.
fn fingers(
    touches: Res<Touches>,
    window: Single<&Window>,
    camera: Single<(&mut Transform, &mut Projection, &mut ZoomTarget), With<Camera2d>>,
    mut span: Local<Option<f32>>,
) {
    let (mut transform, mut projection, mut target) = camera.into_inner();
    let Projection::Orthographic(ortho) = &mut *projection else {
        return;
    };
    let touching: Vec<&_> = touches.iter().collect();
    if touching.len() != 2 {
        *span = None;
        if let [one] = touching[..] {
            drag(&mut transform, one.delta(), ortho.scale);
        }
        return;
    }
    let (a, b) = (touching[0], touching[1]);

    drag(&mut transform, (a.delta() + b.delta()) / 2.0, ortho.scale);

    let now = a.position().distance(b.position());
    let Some(was) = span.replace(now) else {
        return;
    };
    if now < 1.0 || was < 1.0 {
        return;
    }
    let old = ortho.scale;
    let new = pinched(old, was, now);
    ortho.scale = new;
    // `zoom` eases towards this; leaving it stale would snap the pinch back.
    target.0 = new;

    let cam = transform.translation.truncate();
    let mid = (a.position() + b.position()) / 2.0;
    let anchor = cursor_world(mid, window.size(), cam, old);
    transform.translation = zoom_anchored(cam, anchor, old, new).extend(transform.translation.z);
}

fn pinched(old: f32, was: f32, now: f32) -> f32 {
    (old * was / now).clamp(ZOOM_MIN, ZOOM_MAX)
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

// select.rs hit-tests with it.
pub fn screen_to_world(
    camera: &Camera,
    cam_global: &GlobalTransform,
    screen: Vec2,
) -> Option<Vec2> {
    camera.viewport_to_world_2d(cam_global, screen).ok()
}

// text.rs places its screen-space panels with it.
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
    fn spreading_two_fingers_zooms_in() {
        assert_eq!(pinched(1.0, 100.0, 200.0), 0.5);
        assert_eq!(pinched(1.0, 200.0, 100.0), 2.0);
        assert_eq!(pinched(1.0, 100.0, 100.0), 1.0);
        assert_eq!(pinched(ZOOM_MIN, 100.0, 1e6), ZOOM_MIN);
        assert_eq!(pinched(ZOOM_MAX, 1e6, 100.0), ZOOM_MAX);
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

    fn canvas(boxes: &[(i64, i64, i64, i64)]) -> Canvas {
        Canvas {
            nodes: boxes
                .iter()
                .enumerate()
                .map(|(index, &(x, y, width, height))| CanvasNode {
                    id: format!("n{index}"),
                    x,
                    y,
                    width,
                    height,
                    color: None,
                    sides: None,
                    kind: extboard_core::NodeKind::Text {
                        text: String::new(),
                    },
                    extra: serde_json::Map::new(),
                })
                .collect(),
            ..Canvas::default()
        }
    }

    #[test]
    fn a_board_centres_on_everything_it_holds() {
        assert_eq!(
            center(&canvas(&[(0, 0, 200, 100)])),
            Some(Vec2::new(100.0, -50.0))
        );
        assert_eq!(
            center(&canvas(&[(0, 0, 100, 100), (900, 300, 100, 100)])),
            Some(Vec2::new(500.0, -200.0))
        );
        assert_eq!(
            center(&canvas(&[(-400, -200, 100, 100), (100, 0, 100, 100)])),
            Some(Vec2::new(-100.0, 50.0))
        );
        assert_eq!(center(&canvas(&[])), None);
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
