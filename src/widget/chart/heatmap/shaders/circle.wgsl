struct Camera {
    a: vec4<f32>,
    b: vec4<f32>,
};
@group(0) @binding(0)
var<uniform> camera: Camera;

struct Params {
    depth: vec4<f32>,
    bid_rgb: vec4<f32>,
    ask_rgb: vec4<f32>,
    grid: vec4<f32>,
    origin: vec4<f32>,
    heatmap_map: vec4<f32>,
    heatmap_tex: vec4<f32>,
    fade: vec4<f32>, // (x_left, width, alpha_min, alpha_max)
};
@group(0) @binding(1)
var<uniform> params: Params;

struct VsIn {
    @location(0) corner: vec2<f32>,
    @location(1) y_world: f32,
    @location(2) x_bin_rel: i32,
    @location(3) x_frac: f32,
    @location(4) radius_px: f32,
    @location(5) color: vec4<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) world_x: f32,
};

@vertex
fn vs_main(input: VsIn) -> VsOut {
    let scale = camera.a.xy;
    let cam_center = camera.a.zw;
    let viewport = camera.b.xy;

    let col_w = params.grid.x;
    let now_bucket_rel_f = params.origin.x;

    let x_trade = f32(input.x_bin_rel) + input.x_frac;
    let center_x = -((now_bucket_rel_f) - x_trade) * col_w;
    let center = vec2<f32>(center_x, input.y_world);

    let radius_world = vec2<f32>(
        input.radius_px / max(scale.x, 1e-6),
        input.radius_px / max(scale.y, 1e-6),
    );

    let world_pos = center + input.corner * radius_world;
    let view = (world_pos - cam_center) * scale;

    var out: VsOut;
    out.pos = vec4<f32>(
        view.x / (viewport.x * 0.5),
        -view.y / (viewport.y * 0.5),
        0.0,
        1.0
    );
    out.local = input.corner;
    out.color = input.color;
    out.world_x = world_pos.x;
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

    let fade = fade_factor(input.world_x);
    return vec4<f32>(input.color.rgb * a * fade, input.color.a * a * fade);
}

fn fade_factor(world_x: f32) -> f32 {
    let x0 = params.fade.x;
    let w = max(params.fade.y, 1e-6);
    let t = clamp((world_x - x0) / w, 0.0, 1.0);

    // More aggressive than plain smoothstep: stays near alpha_min longer.
    var s = smoothstep(0.0, 1.0, t);
    s = s * s;

    return params.fade.z + s * (params.fade.w - params.fade.z);
}