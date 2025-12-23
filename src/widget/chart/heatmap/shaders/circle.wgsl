struct Camera {
    a: vec4<f32>, // (scale.x, scale.y, center.x, center.y)
    b: vec4<f32>, // (viewport_w, viewport_h, 0, 0)
};

@group(0) @binding(0)
var<uniform> camera: Camera;

struct VsIn {
    @location(0) corner: vec2<f32>, // [-1, +1]
    @location(1) center: vec2<f32>, // world units
    @location(2) radius_px: f32,    // SCREEN pixels
    @location(3) color: vec4<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>, // [-1, +1]
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(input: VsIn) -> VsOut {
    let scale = camera.a.xy;
    let cam_center = camera.a.zw;
    let viewport = camera.b.xy;

    // Convert desired pixel radius into world offsets (separately for x/y to stay circular on screen)
    let radius_world = vec2<f32>(
        input.radius_px / max(scale.x, 1e-6),
        input.radius_px / max(scale.y, 1e-6),
    );

    let world_pos = input.center + input.corner * radius_world;

    let view = (world_pos - cam_center) * scale;

    let ndc_x = view.x / (viewport.x * 0.5);
    let ndc_y = -view.y / (viewport.y * 0.5);

    var out: VsOut;
    out.pos = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.local = input.corner;
    out.color = input.color;
    return out;
}

@fragment
fn fs_main(input: VsOut) -> @location(0) vec4<f32> {
    let d = length(input.local);

    let outer = 1.0;
    let inner = 0.92;
    let a = 1.0 - smoothstep(inner, outer, d);

    if (a <= 0.0) {
        discard;
    }

    return vec4<f32>(input.color.rgb * a, input.color.a * a);
}