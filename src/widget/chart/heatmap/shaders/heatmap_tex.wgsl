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
    fade: vec4<f32>, // (x_left, width, alpha_min, alpha_max)
};
@group(0) @binding(1)
var<uniform> params: Params;

@group(1) @binding(0)
var heatmap_bid: texture_2d<u32>;
@group(1) @binding(1)
var heatmap_ask: texture_2d<u32>;

fn fade_factor(world_x: f32) -> f32 {
    let x0 = params.fade.x;
    let w = max(params.fade.y, 1e-6);
    let t = clamp((world_x - x0) / w, 0.0, 1.0);
    let s = smoothstep(0.0, 1.0, t);
    return params.fade.z + s * (params.fade.w - params.fade.z);
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.pos = vec4<f32>(input.local_pos * 2.0, 0.0, 1.0);
    out.uv = input.local_pos + vec2<f32>(0.5, 0.5);
    return out;
}

fn screen_to_world(screen_xy: vec2<f32>) -> vec2<f32> {
    let scale = camera.a.xy;
    let center = camera.a.zw;
    let viewport = camera.b.xy;

    let view_px = screen_xy - 0.5 * viewport;

    return vec2<f32>(
        center.x + (view_px.x / max(scale.x, 1e-6)),
        center.y - (view_px.y / max(scale.y, 1e-6)),
    );
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let viewport = camera.b.xy;

    let px = input.uv * viewport;
    let screen_xy = floor(px) + vec2<f32>(0.5, 0.5);
    let world = screen_to_world(screen_xy);

    // Reserve x>0 for latest-profile overlay
    if (world.x > 0.0) {
        return vec4<f32>(0.0);
    }

    let col_w = max(params.grid.x, 1e-12);
    let row_h = max(params.grid.y, 1e-12);
    let steps_per = max(params.grid.z, 1.0);
    let bin_h = row_h * steps_per;

    let tex_w_u = u32(params.heatmap_b.x);
    let tex_h_u = u32(params.heatmap_b.y);
    if (tex_w_u < 1u || tex_h_u < 1u) {
        return vec4<f32>(0.0);
    }

    // Y (bins)
    let y_bin_rel = i32(floor((-world.y) / max(bin_h, 1e-12)));
    let y_start_bin = i32(params.heatmap_a.y);
    let yi = y_bin_rel - y_start_bin;
    if (yi < 0 || u32(yi) >= tex_h_u) {
        return vec4<f32>(0.0);
    }

    // X (ring):
    // params.origin.x is (render_bucket - scroll_ref_bucket) + phase
    // params.heatmap_a.x is (latest_data_bucket - scroll_ref_bucket)
    // params.heatmap_a.w is latest_x_ring
    let latest_bucket_rel = i32(params.heatmap_a.x);
    let render_rel = i32(floor(params.origin.x + (world.x / col_w)));

    // Convert render-relative bucket to latest-relative bucket offset
    var bucket_rel_from_latest = render_rel - latest_bucket_rel;

    // Smooth scrolling "future": clamp anything newer than latest to latest
    bucket_rel_from_latest = min(bucket_rel_from_latest, 0);

    // Cull older than ring horizon to avoid wrap artifacts
    let oldest = -i32(tex_w_u) + 1;
    if (bucket_rel_from_latest < oldest) {
        return vec4<f32>(0.0);
    }

    let latest_x_ring = i32(u32(params.heatmap_a.w));
    let tex_w_mask = i32(u32(params.heatmap_b.z));
    let xi = (latest_x_ring + bucket_rel_from_latest) & tex_w_mask;

    let inv_qty_scale = params.heatmap_b.w;
    let bid_qty = f32(textureLoad(heatmap_bid, vec2<i32>(xi, yi), 0).x) * inv_qty_scale;
    let ask_qty = f32(textureLoad(heatmap_ask, vec2<i32>(xi, yi), 0).x) * inv_qty_scale;

    let max_depth = max(params.depth.x, 1e-12);
    let alpha_min = params.depth.y;
    let alpha_max = params.depth.z;

    let bid_t = clamp(bid_qty / max_depth, 0.0, 1.0);
    let ask_t = clamp(ask_qty / max_depth, 0.0, 1.0);

    // Map t∈[0,1] -> alpha∈[alpha_min, alpha_max], but keep qty==0 fully transparent
    let bid_a = select(0.0, alpha_min + bid_t * (alpha_max - alpha_min), bid_qty > 0.0);
    let ask_a = select(0.0, alpha_min + ask_t * (alpha_max - alpha_min), ask_qty > 0.0);

    // premultiplied blend
    var c = params.ask_rgb.xyz * ask_a;
    var a = ask_a;

    c = c + (1.0 - a) * params.bid_rgb.xyz * bid_a;
    a = a + (1.0 - a) * bid_a;

    let fade = fade_factor(world.x);
    return vec4<f32>(c * fade, a * fade);
}