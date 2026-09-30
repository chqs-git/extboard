mod camera;
mod client;
mod edit;
mod hud;
mod node;
mod scene;
mod script;
mod select;
mod sync;
mod text;
mod undo;

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
        .insert_resource(ClearColor(script::DARK))
        .add_plugins((
            camera::CameraPlugin,
            client::ClientPlugin,
            edit::EditPlugin,
            hud::HudPlugin,
            node::NodePlugin,
            scene::ScenePlugin,
            script::ScriptPlugin,
            select::SelectPlugin,
            sync::SyncPlugin,
            text::TextPlugin,
            undo::UndoPlugin,
        ))
        .run();
}
