#import bevy_ui::ui_vertex_output::UiVertexOutput

struct Line {
    color: vec4<f32>,
    length: f32,
    // Screen pixels; the rest are canvas units.
    shaft: f32,
    hole: f32,
    dash: f32,
    heads: vec2<f32>,
    pad: vec2<f32>,
}

@group(1) @binding(0)
var<uniform> line: Line;

const HEAD_LEN: f32 = 18.0;
const HEAD_HALF_WIDTH: f32 = 8.0;

fn head(point: vec2<f32>, tip: f32, len: f32, half: f32) -> f32 {
    let base = tip - len - point.x;
    let side = (abs(point.y) * len + half * (point.x - tip)) / length(vec2<f32>(len, half));
    return max(base, side);
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let point = (in.uv - 0.5) * in.size;
    let zoom = in.size.x / max(line.length, 0.0001);
    let end = in.size.x * 0.5;
    let len = HEAD_LEN * zoom;
    let half = HEAD_HALF_WIDTH * zoom;
    let w = line.shaft;

    let start = -end + line.heads.x * len;
    let stop = end - line.heads.y * len;
    var shaft = max(max(start - point.x, point.x - stop), abs(point.y) - w * 0.5);
    let along = point.x - start;
    let unit = w * zoom;
    if line.dash > 1.5 {
        let period = unit * 2.5;
        let at = along - (floor(along / period) + 0.5) * period;
        shaft = max(shaft, length(vec2<f32>(at, point.y)) - w * 0.5);
    } else if line.dash > 0.5 {
        let period = unit * 5.0;
        let on = unit * 1.5;
        shaft = max(shaft, abs(along - floor(along / period) * period - on) - on);
    }
    if line.hole > 0.0 {
        shaft = max(shaft, line.hole * zoom - abs(point.x));
    }

    var past = shaft;
    if line.heads.y > 0.5 {
        past = min(past, head(point, end, len, half));
    }
    if line.heads.x > 0.5 {
        past = min(past, head(vec2<f32>(-point.x, point.y), end, len, half));
    }

    let fade = max(fwidth(past), 0.0001) * 0.5;
    let mask = 1.0 - smoothstep(-fade, fade, past);
    return vec4<f32>(line.color.rgb, line.color.a * mask);
}
