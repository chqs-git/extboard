#import bevy_ui::ui_vertex_output::UiVertexOutput

// The box is the edge laid flat, `a` on the left and `b` on the right, and the
// node's rotation stands it back up.
struct Line {
    color: vec4<f32>,
    // Canvas units, `a` to `b`: the laid-out width over this is the zoom.
    length: f32,
    // Screen pixels, at every zoom.
    shaft: f32,
    // Half the gap around a label, in canvas units; zero for none.
    hole: f32,
    // 0 solid, 1 dashed, 2 dotted.
    dash: f32,
    // 1 where that end has an arrowhead: `a`, then `b`.
    heads: vec2<f32>,
    pad: vec2<f32>,
}

@group(1) @binding(0)
var<uniform> line: Line;

// Canvas units, so a head grows with the board the way its nodes do.
const HEAD_LEN: f32 = 18.0;
const HEAD_HALF_WIDTH: f32 = 8.0;

// Signed distance to a head whose tip is at `tip` on the x axis, pointing +x.
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
    // From the shaft's own start, so the pattern begins on a dash. Its spacing is
    // in canvas units, so it grows with the board; the dots stay the shaft's width.
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
