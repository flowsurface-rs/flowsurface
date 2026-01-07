struct VertexInput {
    @location(0) local_pos: vec2<f32>,

    @location(1) position: vec2<f32>,
    @location(2) size: vec2<f32>,
    @location(3) color: vec4<f32>,

    @location(4) x0_bin: i32,
    @location(5) x1_bin_excl: i32,
    @location(6) x_from_bins: u32, // 0/1
};

struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
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
    let now_bucket_rel_f = params.origin.x;

    let volume_min_w_world = params.origin.y;
    let volume_gap_frac = params.origin.z;

    let x_from_bins = input.x_from_bins != 0u;

    if !x_from_bins {
        world_pos = input.position + input.local_pos * input.size;
    } else {
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
    return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return input.color;
}