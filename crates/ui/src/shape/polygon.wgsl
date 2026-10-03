#import bevy_ui::ui_vertex_output::UiVertexOutput

struct Polygon {
    color: vec4<f32>,
    sides: f32,
#ifdef SIXTEEN_BYTE_ALIGNMENT
    // WebGL2 wants the struct rounded up to sixteen bytes.
    _webgl2_padding: vec3<f32>,
#endif
}

@group(1) @binding(0)
var<uniform> polygon: Polygon;

const TAU: f32 = 6.2831855;
const UP: f32 = 1.5707964;
const CIRCLE: f32 = 10.0;

// The same vertices `extboard_core::sides_polygon` computes, and they have to
// stay the same: the phone view clips from that one.
fn rim(i: f32, sides: f32) -> vec2<f32> {
    let turn = TAU / sides;
    let half = select(0.0, 0.5, sides % 2.0 < 0.5);
    let angle = UP + turn * (i + half);
    return vec2<f32>(cos(angle), -sin(angle));
}

// How far outside the polygon a point is: the furthest past any one edge,
// which for a convex shape is the distance to the shape itself.
fn outside(point: vec2<f32>, sides: f32, size: vec2<f32>) -> f32 {
    // Fitted end to end, the same fit core does, so four sides is the box.
    var low = vec2<f32>(1.0, 1.0);
    var high = vec2<f32>(-1.0, -1.0);
    for (var i = 0.0; i < sides; i += 1.0) {
        let corner = rim(i, sides);
        low = min(low, corner);
        high = max(high, corner);
    }
    let span = max(high - low, vec2<f32>(0.0001, 0.0001));

    var far = -3.4e38;
    for (var i = 0.0; i < sides; i += 1.0) {
        // Not `from`/`to`: both are reserved words in WGSL.
        let here = ((rim(i, sides) - low) / span - 0.5) * size;
        let next = ((rim(i + 1.0, sides) - low) / span - 0.5) * size;
        let edge = next - here;
        let normal = normalize(vec2<f32>(-edge.y, edge.x));
        far = max(far, dot(normal, point - here));
    }
    return far;
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let point = (in.uv - 0.5) * in.size;

    // Not `distance`: shadowing a WGSL builtin does not compile.
    var past: f32;
    if polygon.sides >= CIRCLE {
        let radius = max(in.size * 0.5, vec2<f32>(0.0001, 0.0001));
        past = (length(point / radius) - 1.0) * min(radius.x, radius.y);
    } else {
        past = outside(point, max(polygon.sides, 3.0), in.size);
    }

    // One pixel of fade: the panel is sized in screen pixels.
    let fade = max(fwidth(past), 0.0001);
    let mask = 1.0 - smoothstep(-fade, fade, past);
    return vec4<f32>(polygon.color.rgb, polygon.color.a * mask);
}
