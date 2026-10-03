#import bevy_ui::ui_vertex_output::UiVertexOutput
#import bevy_render::color_operations::hsv_to_linear_rgb

struct Dial {
    // Turns, not degrees: the conversion takes 0..1.
    hue: f32,
    // Zero draws the plane, one the hue strip.
    strip: f32,
#ifdef SIXTEEN_BYTE_ALIGNMENT
    // WebGL2 wants the struct rounded up to sixteen bytes.
    _webgl2_padding: vec2<f32>,
#endif
}

@group(1) @binding(0)
var<uniform> dial: Dial;

// Saturation across and value up, for the hue the strip holds.
// `hsv_to_linear_rgb` is bevy's own: it carries the sRGB curve the UI pass
// expects, and writing it here would mean writing the gamma ramp too.
@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    var hsv = vec3<f32>(dial.hue, in.uv.x, 1.0 - in.uv.y);
    if dial.strip > 0.5 {
        hsv = vec3<f32>(in.uv.x, 1.0, 1.0);
    }
    return vec4<f32>(hsv_to_linear_rgb(hsv), 1.0);
}
