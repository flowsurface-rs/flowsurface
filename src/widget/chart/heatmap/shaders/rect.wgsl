struct VertexInput {
    @location(0) local_pos: vec2<f32>,
    @location(1) position: vec2<f32>, // world pixels
    @location(2) size: vec2<f32>,     // world pixels
    @location(3) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
};

struct Camera {
    a: vec4<f32>, // (scale.x, scale.y, center.x, center.y)
    b: vec4<f32>, // (viewport_w, viewport_h, 0, 0)
};
@group(0) @binding(0)
var<uniform> camera: Camera;

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    // World position in pixels
    let world_pos = input.position + input.local_pos * input.size;

    let scale = camera.a.xy;
    let center = camera.a.zw;
    let viewport = camera.b.xy;

    // View space (pixels, centered), with zoom
    let view = (world_pos - center) * scale;

    // View -> NDC
    let ndc_x = view.x / (viewport.x * 0.5);
    let ndc_y = -view.y / (viewport.y * 0.5);

    var out: VertexOutput;
    out.pos = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.color = input.color;
    return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return input.color;
}