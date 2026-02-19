// Camera/Params + shared helpers come from common.wgsl

struct VertexInput {
    @location(0) local_pos: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(1) @binding(0)
var heatmap_rg: texture_2d<u32>;

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.pos = vec4<f32>(input.local_pos * 2.0, 0.0, 1.0);
    out.uv = input.local_pos + vec2<f32>(0.5, 0.5);
    return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let viewport = camera.b.xy;

    let px = input.uv * viewport;
    let screen_xy = floor(px) + vec2<f32>(0.5, 0.5);
    let world = screen_to_world(screen_xy);

    if (world.x > 0.0) {
        return vec4<f32>(0.0);
    }

    let col_w = max(params.grid.x, 1e-12);
    let bin_h = y_bin_h_world();
    let y_blend_w = y_blend_weight();

    let tex_w_u = u32(params.heatmap_tex.x);
    let tex_h_u = u32(params.heatmap_tex.y);
    if (tex_w_u < 1u || tex_h_u < 1u) {
        return vec4<f32>(0.0);
    }

    let y_bin_f = (-world.y) / max(bin_h, 1e-12);
    let y_sample_f = y_bin_f - (0.5 * y_blend_w);
    let y_bin_rel = i32(floor(y_sample_f));
    let y_frac = clamp(fract(y_sample_f), 0.0, 1.0);
    let y_start_bin = i32(params.heatmap_map.y);
    let yi0 = y_bin_rel - y_start_bin;
    if (yi0 < 0 || u32(yi0) >= tex_h_u) {
        return vec4<f32>(0.0);
    }
    let yi1 = yi0 + 1;
    let yi1_in_bounds = yi1 >= 0 && u32(yi1) < tex_h_u;

    let latest_bucket_rel = i32(params.heatmap_map.x);
    let render_rel = i32(floor(params.origin.x + (world.x / col_w)));

    var bucket_rel_from_latest = render_rel - latest_bucket_rel;
    bucket_rel_from_latest = min(bucket_rel_from_latest, 0);

    let oldest = -i32(tex_w_u) + 1;
    if (bucket_rel_from_latest < oldest) {
        return vec4<f32>(0.0);
    }

    let latest_x_ring = i32(u32(params.heatmap_map.w));
    let tex_w_mask = i32(u32(params.heatmap_tex.z));
    let xi = (latest_x_ring + bucket_rel_from_latest) & tex_w_mask;

    let inv_qty_scale = params.heatmap_tex.w;

    let v0 = textureLoad(heatmap_rg, vec2<i32>(xi, yi0), 0);
    var bid_qty_u = f32(v0.x);
    var ask_qty_u = f32(v0.y);

    if (yi1_in_bounds) {
        let v1 = textureLoad(heatmap_rg, vec2<i32>(xi, yi1), 0);
        let y_mix = y_frac * y_blend_w;
        bid_qty_u = mix(bid_qty_u, f32(v1.x), y_mix);
        ask_qty_u = mix(ask_qty_u, f32(v1.y), y_mix);
    }

    let bid_qty = bid_qty_u * inv_qty_scale;
    let ask_qty = ask_qty_u * inv_qty_scale;

    let max_depth = max(params.depth.x, 1e-12);
    let alpha_min = params.depth.y;
    let alpha_max = params.depth.z;

    let bid_t = clamp(bid_qty / max_depth, 0.0, 1.0);
    let ask_t = clamp(ask_qty / max_depth, 0.0, 1.0);

    let bid_a = select(0.0, alpha_min + bid_t * (alpha_max - alpha_min), bid_qty > 0.0);
    let ask_a = select(0.0, alpha_min + ask_t * (alpha_max - alpha_min), ask_qty > 0.0);

    var c = params.ask_rgb.xyz * ask_a;
    var a = ask_a;

    c = c + (1.0 - a) * params.bid_rgb.xyz * bid_a;
    a = a + (1.0 - a) * bid_a;

    let fade = fade_factor(world.x);
    return vec4<f32>(c * fade, a * fade);
}