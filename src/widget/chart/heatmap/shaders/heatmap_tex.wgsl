struct VertexInput {
    @location(0) local_pos: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

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
    heatmap_a: vec4<f32>,
    heatmap_b: vec4<f32>,
};
@group(0) @binding(1)
var<uniform> params: Params;

@group(1) @binding(0)
var heatmap_tex: texture_2d<f32>;

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // local_pos is in [-0.5, 0.5]. Expand to clip [-1, 1].
    out.pos = vec4<f32>(input.local_pos * 2.0, 0.0, 1.0);

    // Map [-0.5,0.5] -> [0,1]
    out.uv = input.local_pos + vec2<f32>(0.5, 0.5);
    return out;
}

fn screen_to_world(screen_xy: vec2<f32>) -> vec2<f32> {
    let scale = camera.a.xy;
    let center = camera.a.zw;
    let viewport = camera.b.xy;

    // screen_xy is in pixels with origin at top-left, y down.
    let view_px = screen_xy - 0.5 * viewport;

    return vec2<f32>(
        center.x + (view_px.x / max(scale.x, 1e-6)),
        center.y - (view_px.y / max(scale.y, 1e-6)),
    );
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let viewport = camera.b.xy;

    // Stable pixel-center coords
    let px = input.uv * viewport;
    let screen_xy = floor(px) + vec2<f32>(0.5, 0.5);

    let world = screen_to_world(screen_xy);

    // Heatmap is only defined for history (x <= 0). Reserve x>0 for the latest profile overlay.
    if (world.x > 0.0) {
        return vec4<f32>(0.0);
    }

    let sx = max(camera.a.x, 1e-6);
    let sy = max(camera.a.y, 1e-6);
    let center = camera.a.zw;

    let col_w = params.grid.x;
    let row_h = params.grid.y;
    let steps_per = max(params.grid.z, 1.0);
    let bin_h = row_h * steps_per;

    let now_bucket_rel_f = params.origin.x;

    let x_start_group = params.heatmap_a.x;
    let y_start_bin = params.heatmap_a.y;
    let cols_per_x_bin = max(params.heatmap_a.z, 1.0);

    let tex_w = params.heatmap_b.x;
    let tex_h = params.heatmap_b.y;

    if (tex_w < 1.0 || tex_h < 1.0) {
        return vec4<f32>(0.0);
    }

    let y_bin_rel_f = floor((-world.y) / max(bin_h, 1e-12));
    let y_idx_f = y_bin_rel_f - y_start_bin;

    let yi = i32(y_idx_f);
    if (yi < 0 || f32(yi) >= tex_h) {
        return vec4<f32>(0.0);
    }

    let bin_px_x = max(col_w * sx * cols_per_x_bin, 1e-6);

    let x_bin_rel_at_g0 = (x_start_group * cols_per_x_bin);
    let world_x_at_g0 = (x_bin_rel_at_g0 - now_bucket_rel_f) * col_w;

    let x_anchor_px = ((world_x_at_g0 - center.x) * sx) + (0.5 * viewport.x);

    let gx = (screen_xy.x - x_anchor_px) / bin_px_x;

    let xi = i32(floor(gx));
    if (xi < 0 || f32(xi) >= tex_w) {
        return vec4<f32>(0.0);
    }

    let s = textureLoad(heatmap_tex, vec2<i32>(xi, yi), 0);

    let bid_qty = max(s.x, 0.0);
    let ask_qty = max(s.y, 0.0);

    let max_depth = max(params.depth.x, 1e-12);
    let alpha_min = params.depth.y;
    let alpha_max = params.depth.z;

    let bid_t = clamp(bid_qty / max_depth, 0.0, 1.0);
    let ask_t = clamp(ask_qty / max_depth, 0.0, 1.0);

    let bid_a = select(0.0, clamp(bid_t, alpha_min, alpha_max), bid_qty > 0.0);
    let ask_a = select(0.0, clamp(ask_t, alpha_min, alpha_max), ask_qty > 0.0);

    var c = params.ask_rgb.xyz * ask_a;
    var a = ask_a;

    c = c + (1.0 - a) * params.bid_rgb.xyz * bid_a;
    a = a + (1.0 - a) * bid_a;

    return vec4<f32>(c, a);
}