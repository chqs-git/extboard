mod camera;
mod client;
mod hud;
mod node;

use bevy::prelude::*;
use bevy::window::PresentMode;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "ext.board".to_owned(),
                present_mode: PresentMode::AutoNoVsync,
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.07, 0.07, 0.09)))
        .add_plugins((
            camera::CameraPlugin,
            client::ClientPlugin,
            hud::HudPlugin,
            node::NodePlugin,
        ))
        .run();
}
