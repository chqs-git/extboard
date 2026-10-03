use bevy::asset::{AssetPath, embedded_asset, embedded_path};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use extboard_core::Node as CanvasNode;

use crate::node::body_color;

use super::RECT_SIDES;

// The shape is UI, not a mesh: the node's text is a child drawn over it, and
// two nodes stack by the board's order rather than by which pass they are in.
#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct PolygonMaterial {
    #[uniform(0)]
    paint: Paint,
}

#[derive(ShaderType, Clone, Debug)]
struct Paint {
    color: Vec4,
    sides: f32,
}

impl UiMaterial for PolygonMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("polygon.wgsl")).with_source("embedded"),
        )
    }
}

// One per look, not per node: the UI batches what shares a handle.
#[derive(Resource, Default)]
pub struct Palette(HashMap<(u8, [u32; 4]), Handle<PolygonMaterial>>);

pub(super) fn register(app: &mut App) {
    embedded_asset!(app, "polygon.wgsl");
    app.add_plugins(UiMaterialPlugin::<PolygonMaterial>::default())
        .init_resource::<Palette>();
}

pub fn paint(
    palette: &mut Palette,
    materials: &mut Assets<PolygonMaterial>,
    node: &CanvasNode,
) -> Handle<PolygonMaterial> {
    let sides = node.sides.unwrap_or(RECT_SIDES);
    let color = body_color(node).to_linear().to_f32_array();
    palette
        .0
        .entry((sides, color.map(f32::to_bits)))
        .or_insert_with(|| {
            materials.add(PolygonMaterial {
                paint: Paint {
                    color: Vec4::from_array(color),
                    sides: f32::from(sides),
                },
            })
        })
        .clone()
}
