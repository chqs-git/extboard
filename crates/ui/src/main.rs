mod camera;
mod client;
mod dropdown_menu;
mod edit;
mod hud;
mod node;
mod scene;
mod script;
mod select;
mod shape;
mod sync;
mod text;
mod theme;
mod undo;
#[cfg(test)]
mod wgsl;

use bevy::asset::AssetMetaCheck;
use bevy::prelude::*;
use bevy::window::PresentMode;

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "ext.board".to_owned(),
                        present_mode: PresentMode::AutoNoVsync,
                        ..default()
                    }),
                    ..default()
                })
                .set(AssetPlugin {
                    // An image node names a file in the spaces dir, so that dir
                    // is the asset root. `Forbid` stays the default: a canvas is
                    // untrusted input and `../` in a path is not ours to read.
                    file_path: client::files_root(),
                    // Nothing here has a `.meta`, and on wasm each check is a 404.
                    meta_check: AssetMetaCheck::Never,
                    ..default()
                }),
        )
        .insert_resource(ClearColor(theme::DARK))
        .add_plugins((
            camera::CameraPlugin,
            client::ClientPlugin,
            dropdown_menu::MenuPlugin,
            edit::EditPlugin,
            hud::HudPlugin,
            node::NodePlugin,
            scene::ScenePlugin,
            script::ScriptPlugin,
            select::SelectPlugin,
            shape::ShapePlugin,
            sync::SyncPlugin,
            text::TextPlugin,
            theme::ThemePlugin,
            undo::UndoPlugin,
        ))
        .run();
}
