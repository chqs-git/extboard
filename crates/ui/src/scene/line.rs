use bevy::asset::{AssetPath, embedded_asset, embedded_path};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

// UI rather than gizmos, which are drawn in the world under every panel: an edge
// is stacked in among the nodes instead of always beneath them.
#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct LineMaterial {
    #[uniform(0)]
    pub paint: LinePaint,
}

#[derive(ShaderType, Clone, Copy, Debug, PartialEq, Default)]
pub struct LinePaint {
    pub color: Vec4,
    pub length: f32,
    pub shaft: f32,
    pub hole: f32,
    pub dash: f32,
    pub heads: Vec2,
    pub pad: Vec2,
}

impl UiMaterial for LineMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("line.wgsl")).with_source("embedded"),
        )
    }
}

pub(super) fn register(app: &mut App) {
    embedded_asset!(app, "line.wgsl");
    app.add_plugins(UiMaterialPlugin::<LineMaterial>::default());
}
