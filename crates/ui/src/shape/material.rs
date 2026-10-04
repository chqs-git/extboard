use bevy::asset::{AssetPath, embedded_asset, embedded_path};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use extboard_core::Node as CanvasNode;

use crate::node::{body_color, outline_color, outline_px};
use crate::theme::Theme;

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
    outline: Vec4,
    // Canvas units; the laid-out box is this times the zoom.
    size: Vec2,
    sides: f32,
    // The rim's width, in those same units.
    edge: f32,
}

impl UiMaterial for PolygonMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("polygon.wgsl")).with_source("embedded"),
        )
    }
}

type Look = (u8, [u32; 4], [u32; 4], [u32; 2], u32);

const LOOKS: usize = 512;

// One per look, not per node: the UI batches what shares a handle.
#[derive(Resource, Default)]
pub struct Palette(HashMap<Look, Handle<PolygonMaterial>>);

pub(super) fn register(app: &mut App) {
    embedded_asset!(app, "polygon.wgsl");
    app.add_plugins(UiMaterialPlugin::<PolygonMaterial>::default())
        .init_resource::<Palette>();
}

pub fn paint(
    palette: &mut Palette,
    materials: &mut Assets<PolygonMaterial>,
    theme: &Theme,
    node: &CanvasNode,
) -> Handle<PolygonMaterial> {
    let sides = node.sides.unwrap_or(RECT_SIDES);
    let color = body_color(theme, node).to_linear().to_f32_array();
    let outline = outline_color(theme, node).to_linear().to_f32_array();
    let size = Vec2::new(node.width as f32, node.height as f32);
    let edge = outline_px(node);
    if palette.0.len() > LOOKS {
        palette.0.clear();
    }
    palette
        .0
        .entry((
            sides,
            color.map(f32::to_bits),
            outline.map(f32::to_bits),
            size.to_array().map(f32::to_bits),
            edge.to_bits(),
        ))
        .or_insert_with(|| {
            materials.add(PolygonMaterial {
                paint: Paint {
                    color: Vec4::from_array(color),
                    outline: Vec4::from_array(outline),
                    size,
                    sides: f32::from(sides),
                    edge,
                },
            })
        })
        .clone()
}
