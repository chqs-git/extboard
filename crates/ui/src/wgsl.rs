const PREAMBLE: &str = "
struct UiVertexOutput {
    @location(0) uv: vec2<f32>,
    @location(1) border_widths: vec4<f32>,
    @location(2) border_radius: vec4<f32>,
    @location(3) @interpolate(flat) size: vec2<f32>,
    @builtin(position) position: vec4<f32>,
};
fn hsv_to_linear_rgb(hsv: vec3<f32>) -> vec3<f32> { return hsv; }
";

pub fn compiles(wgsl: &str) {
    let mut source = String::from(PREAMBLE);
    let mut skipping = false;
    for line in wgsl.lines() {
        match line.trim_start() {
            directive if directive.starts_with("#ifdef") => skipping = true,
            directive if directive.starts_with("#endif") => skipping = false,
            directive if directive.starts_with('#') => {}
            _ if skipping => {}
            code => {
                source.push_str(code);
                source.push('\n');
            }
        }
    }

    let module = naga::front::wgsl::parse_str(&source)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
}
