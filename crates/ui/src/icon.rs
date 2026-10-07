use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageSampler, ImageType};
use bevy::prelude::*;

// Material Symbols, 24dp, flat #E3E3E3 so a tint is the icon's colour.
//
// Embedded rather than loaded from a file: the asset root is the spaces dir,
// which holds a person's boards and not our furniture, and in the browser a
// file would be one more thing for extd to serve. Five of them is a kilobyte.
const ICONS: [(&str, &[u8]); 5] = [
    ("send_to_back", include_bytes!("../icons/send_to_back.png")),
    (
        "send_backward",
        include_bytes!("../icons/send_backward.png"),
    ),
    ("send_forward", include_bytes!("../icons/send_forward.png")),
    ("send_to_top", include_bytes!("../icons/send_to_top.png")),
    ("sensor_door", include_bytes!("../icons/sensor_door.png")),
];

pub struct IconPlugin;

// Decoded once at startup: a panel respawns on every document change, and
// decoding a PNG per respawn is a cost for nothing.
#[derive(Resource)]
pub struct Icons(Vec<(&'static str, Handle<Image>)>);

impl Plugin for IconPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_icons);
    }
}

impl Icons {
    // A missing name is a typo in our own table, so it draws as nothing rather
    // than taking the app down -- but it is still worth saying out loud once.
    pub fn get(&self, name: &str) -> Handle<Image> {
        match self.0.iter().find(|(known, _)| *known == name) {
            Some((_, handle)) => handle.clone(),
            None => {
                warn_once!("no icon named {name}");
                Handle::default()
            }
        }
    }
}

fn load_icons(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let loaded = ICONS
        .iter()
        .filter_map(|&(name, bytes)| Some((name, images.add(decode(name, bytes)?))))
        .collect();
    commands.insert_resource(Icons(loaded));
}

fn decode(name: &str, bytes: &[u8]) -> Option<Image> {
    match Image::from_buffer(
        bytes,
        ImageType::Extension("png"),
        default(),
        true,
        ImageSampler::linear(),
        RenderAssetUsages::RENDER_WORLD,
    ) {
        Ok(image) => Some(image),
        // The bytes are compiled in, so this is a broken build, not bad input.
        Err(e) => {
            error!("icon {name} did not decode: {e}");
            None
        }
    }
}
