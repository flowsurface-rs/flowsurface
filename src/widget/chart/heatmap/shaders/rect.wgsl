struct VertexInput {
    @location(0) local_pos: vec2<f32>,

    @location(1) position: vec2<f32>,
    @location(2) size: vec2<f32>,
    @location(3) color: vec4<f32>,

    @location(4) qty: f32,
    @location(5) side_sign: f32,

    @location(6) x0_bin: i32,
    @location(7) x1_bin_excl: i32,
    @location(8) abs_y_bin: i32,
    @location(9) flags: u32,
};

struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) qty: f32,
    @location(2) side_sign: f32,
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

fn world_to_clip(world_pos: vec2<f32>) -> vec4<f32> {
    let scale = camera.a.xy;
    let center = camera.a.zw;
    let viewport = camera.b.xy;

    let view = (world_pos - center) * scale;
    let ndc_x = view.x / (viewport.x * 0.5);
    let ndc_y = -view.y / (viewport.y * 0.5);
    return vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var world_pos: vec2<f32>;

    let col_w = params.grid.x;
    let row_h = params.grid.y;
    let steps_per = max(params.grid.z, 1.0);
    let bin_h = row_h * steps_per;

    let now_bucket_rel_f = params.origin.x;

    // origin.y/z will be used by volume bars (min width + gap)
    let volume_min_w_world = params.origin.y;
    let volume_gap_frac = params.origin.z;

    let is_overlay = (input.flags & 1u) == 1u;
    let x_from_bins = (input.flags & 2u) == 2u;

    if (is_overlay && !x_from_bins) {
        // pure overlay: x/y/size fully in instance
        world_pos = input.position + input.local_pos * input.size;
    } else if (!is_overlay) {
        // depth cells (bin-space x and bin-space y)
        let start = f32(input.x0_bin);
        let end_excl = f32(input.x1_bin_excl);

        // EXPAND LATEST: clamp end to "now" (historical end_excl <= 0 stays unchanged)
        let end = min(end_excl, now_bucket_rel_f);

        let x0 = -((now_bucket_rel_f) - start) * col_w;
        let x1 = -((now_bucket_rel_f) - end) * col_w;

        let left = min(x0, x1);
        let right = max(x0, x1);

        let width = max(right - left, 0.0);
        let center_x = 0.5 * (left + right);

        let y_bin_rel = f32(input.abs_y_bin);
        let center_y = -((y_bin_rel + 0.5) * bin_h);

        world_pos = vec2<f32>(center_x, center_y) + input.local_pos * vec2<f32>(width, bin_h);
    } else {
        // overlay with bin-space x (volume strip): y + height from instance, x + width from bins
        let start = f32(input.x0_bin);
        let end_excl = f32(input.x1_bin_excl);
        let end = min(end_excl, now_bucket_rel_f);

        let x0 = -((now_bucket_rel_f) - start) * col_w;
        let x1 = -((now_bucket_rel_f) - end) * col_w;

        let left = min(x0, x1);
        let right = max(x0, x1);

        let bin_w = max(right - left, 0.0);
        let bar_w = max(bin_w * (1.0 - volume_gap_frac), volume_min_w_world);

        let center_x = 0.5 * (left + right);
        let center_y = input.position.y;
        let h = input.size.y;

        world_pos = vec2<f32>(center_x, center_y) + input.local_pos * vec2<f32>(bar_w, h);
    }

    var out: VertexOutput;
    out.pos = world_to_clip(world_pos);
    out.color = input.color;
    out.qty = input.qty;
    out.side_sign = input.side_sign;
    return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    if (input.qty <= 0.0) {
        return input.color;
    }

    let max_depth = max(params.depth.x, 1e-12);
    let alpha_min = params.depth.y;
    let alpha_max = params.depth.z;

    let t = clamp(input.qty / max_depth, 0.0, 1.0);
    let a = clamp(t, alpha_min, alpha_max);

    let rgb = select(params.ask_rgb.xyz, params.bid_rgb.xyz, input.side_sign > 0.0);
    return vec4<f32>(rgb, a);
}