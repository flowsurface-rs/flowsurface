struct VertexInput {
    @location(0) local_pos: vec2<f32>,
    @location(1) position: vec2<f32>,
    @location(2) size: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) x0_bin: i32,
    @location(5) x1_bin_excl: i32,
    @location(6) x_from_bins: u32,
    @location(7) fade_mode: u32, // 0=fade, 1=skip
};

struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) world_x: f32,
    @location(2) fade_mode: u32,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var world_pos: vec2<f32>;

    let col_w = params.grid.x;
    let now_bucket_rel_f = params.origin.x;

    let volume_min_w_world = params.origin.y;
    let volume_gap_frac = params.origin.z;
    let volume_x_shift_bucket = params.origin.w;

    let x_from_bins = input.x_from_bins != 0u;

    if !x_from_bins {
        let snapped_center_y = y_bin_center_world(input.position.y);
        let center_y = mix(input.position.y, snapped_center_y, y_blend_weight());
        let center = vec2<f32>(input.position.x, center_y);
        world_pos = center + input.local_pos * input.size;
    } else {
        let start = f32(input.x0_bin) + volume_x_shift_bucket;
        let end_excl = f32(input.x1_bin_excl) + volume_x_shift_bucket;

        let end = min(end_excl, now_bucket_rel_f);

        let x0 = bucket_rel_to_world_x(start, now_bucket_rel_f, col_w);
        let x1 = bucket_rel_to_world_x(end, now_bucket_rel_f, col_w);

        let left = min(x0, x1);
        let right = max(x0, x1);

        let bin_w = max(right - left, 0.0);
        let bar_w0 = max(bin_w * (1.0 - volume_gap_frac), volume_min_w_world);

        let sx = max(camera.a.x, 1e-6);
        let bar_w_px = max(round(bar_w0 * sx), 1.0);
        let bar_w = bar_w_px / sx;

        let center_x = 0.5 * (left + right);
        let center_y = input.position.y;
        let h = input.size.y;

        world_pos = vec2<f32>(center_x, center_y) + input.local_pos * vec2<f32>(bar_w, h);
    }

    var out: VertexOutput;
    out.pos = world_to_clip(world_pos);

    if (x_from_bins) {
        out.pos.x = snap_ndc_x_to_pixel_edge(out.pos.x);
    }

    out.color = input.color;
    out.world_x = world_pos.x;
    out.fade_mode = input.fade_mode;

    return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let fade = select(fade_factor(input.world_x), 1.0, input.fade_mode != 0u);
    return vec4<f32>(input.color.rgb * fade, input.color.a * fade);
}