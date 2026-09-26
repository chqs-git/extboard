use bevy::prelude::*;
use extboard_core::Canvas;
use std::sync::{Arc, Mutex};

// extd's default port
pub const BASE_URL: &str = "http://127.0.0.1:7777";
pub const TESTING_SPACE_ID: &str = "test";

pub struct ClientPlugin;

#[derive(Resource)]
pub struct Document(pub Canvas);

#[derive(Resource, Default)]
struct CanvasLoader(Arc<Mutex<Option<Canvas>>>);

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CanvasLoader>();
        app.add_systems(Startup, fetch_space);
        app.add_systems(Update, take_canvas);
    }
}

fn fetch_space(loader: Res<CanvasLoader>) {
    let slot = loader.0.clone();
    let request = ehttp::Request::get(format!("{BASE_URL}/api/spaces/{TESTING_SPACE_ID}"));
    ehttp::fetch(request, move |result: ehttp::Result<ehttp::Response>| {
        let response = result.unwrap();
        println!("Status code: {:?}", response.status);
        *slot.lock().unwrap() = match serde_json::from_slice::<Canvas>(&response.bytes) {
            Ok(canvas) => Some(canvas),
            Err(_) => None,
        };
    });
}

fn take_canvas(mut commands: Commands, loader: Res<CanvasLoader>) {
    if let Some(canvas) = loader.0.lock().unwrap().take() {
        info!("number of nodes: {}", canvas.nodes.len());
        commands.insert_resource(Document(canvas));
    }
}
