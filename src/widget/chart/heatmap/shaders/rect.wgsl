struct VertexInput {
    @location(0) local_pos: vec2<f32>,
    @location(1) position: vec2<f32>,
    @location(2) size: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) x0_bin: i32,
    @location(5) x1_bin_excl: i32,
    @location(6) x_layout: u32,
    @location(7) fade_mode: u32, // 0=fade, 1=skip
    @location(8) subpx_alpha: f32,
};

struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) world_x: f32,
    @location(2) fade_mode: u32,
    @location(3) subpx_alpha: f32,
    @location(4) pixel_y: f32,
    @location(5) @interpolate(flat) coverage_x: vec3<f32>,
    @location(6) @interpolate(flat) coverage_y: vec3<f32>,
    @location(7) @interpolate(flat) coverage_mode: u32,
};

const X_ABSOLUTE: u32 = 0u;
const X_VOLUME_BAR: u32 = 1u;
const X_INDICATOR_PAIR: u32 = 2u;
const X_PIXEL_BAR: u32 = 4u;

const SUBPX_PERCEPTUAL_GAMMA: f32 = 0.8;
const SUBPX_VIS_FLOOR: f32 = 0.1;

fn subpx_visibility(alpha: f32) -> f32 {
    let x = clamp(alpha, 0.0, 1.0);
    if (x <= 0.0) {
        return 0.0;
    }

    let lifted = pow(x, SUBPX_PERCEPTUAL_GAMMA);
    return max(lifted, SUBPX_VIS_FLOOR);
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var world_pos: vec2<f32>;
    var out: VertexOutput;
    var subpx_alpha = clamp(input.subpx_alpha, 0.0, 1.0);

    let col_w = params.grid.x;
    let now_bucket_rel_f = params.origin.x;

    let volume_min_w_world = params.origin.y;
    let volume_gap_frac = params.origin.z;
    let volume_x_shift_bucket = params.origin.w;

    let paired = input.x_layout == X_INDICATOR_PAIR;
    let volume = input.x_layout == X_VOLUME_BAR;

    if input.x_layout == X_ABSOLUTE {
        world_pos = input.position + input.local_pos * input.size;
    } else if volume {
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
        let raw_w_px = max(bar_w0 * sx, 0.0);
        let bar_w_px = max(round(raw_w_px), 1.0);
        let bar_w = bar_w_px / sx;
        subpx_alpha = subpx_alpha * clamp(raw_w_px, 0.0, 1.0);

        let center_x = 0.5 * (left + right);
        let center_y = input.position.y;
        let h = input.size.y;

        world_pos = vec2<f32>(center_x, center_y) + input.local_pos * vec2<f32>(bar_w, h);
    } else {
        let start = f32(input.x0_bin) + volume_x_shift_bucket;
        let end_excl = f32(input.x1_bin_excl) + volume_x_shift_bucket;

        let end = min(end_excl, now_bucket_rel_f);

        let x0 = bucket_rel_to_world_x(start, now_bucket_rel_f, col_w);
        let x1 = bucket_rel_to_world_x(end, now_bucket_rel_f, col_w);

        let sx = max(camera.a.x, 1e-6);
        let sy = max(camera.a.y, 1e-6);
        let ratio = max(camera.b.z, 1e-6);
        let origin = camera.b.w;
        let viewport = camera.b.xy;
        let left = min(x0, x1);
        let right = max(x0, x1);
        let left_pixel = ((left - camera.a.z) * sx + viewport.x * 0.5 + origin) * ratio;
        let right_pixel = ((right - camera.a.z) * sx + viewport.x * 0.5 + origin) * ratio;
        let body_end = max(left_pixel, right_pixel - 1.0);
        let midpoint = select(body_end, (left_pixel + body_end) * 0.5, paired);
        let bottom = ((input.position.y - camera.a.w) * sy + viewport.y * 0.5) * ratio;
        let tops = vec2<f32>(bottom) - input.size * sy * ratio;

        // Expand by half a pixel so every pixel touched by either rectangle gets a fragment.
        let pixel_x = mix(left_pixel - 0.5, body_end + 0.5, input.local_pos.x + 0.5);
        let pixel_y = mix(min(tops.x, tops.y) - 0.5, bottom + 0.5, input.local_pos.y + 0.5);
        let screen_x = pixel_x / ratio - origin;
        let screen_y = pixel_y / ratio;

        out.pos = vec4<f32>(
            (screen_x / viewport.x - 0.5) * 2.0,
            (0.5 - screen_y / viewport.y) * 2.0,
            0.0, 1.0,
        );

        out.pixel_y = pixel_y;
        out.coverage_x = vec3<f32>(left_pixel, midpoint, body_end);
        out.coverage_y = vec3<f32>(tops, bottom);
        out.coverage_mode = input.x_layout;

        world_pos = vec2<f32>(
            (screen_x - viewport.x * 0.5) / sx + camera.a.z,
            (screen_y - viewport.y * 0.5) / sy + camera.a.w,
        );
    }

    if input.x_layout == X_ABSOLUTE || volume {
        out.pos = world_to_clip(world_pos);
        if volume {
            out.pos.x = snap_ndc_x_to_pixel_edge(out.pos.x);
        }
    }

    out.color = input.color;
    out.world_x = world_pos.x;
    out.fade_mode = input.fade_mode;
    out.subpx_alpha = subpx_alpha;

    return out;
}

fn pixel_overlap(start: f32, end: f32, center: f32) -> f32 {
    return max(0.0, min(end, center + 0.5) - max(start, center - 0.5));
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    if input.coverage_mode == X_INDICATOR_PAIR || input.coverage_mode == X_PIXEL_BAR {
        let primary_coverage = pixel_overlap(input.coverage_x.x, input.coverage_x.y, input.pos.x)
            * pixel_overlap(input.coverage_y.x, input.coverage_y.z, input.pixel_y);
        var alpha: f32;
        var premultiplied: vec3<f32>;
        let ask_coverage = pixel_overlap(input.coverage_x.y, input.coverage_x.z, input.pos.x)
            * pixel_overlap(input.coverage_y.y, input.coverage_y.z, input.pixel_y);
        let bid_weight = primary_coverage * input.color.a;
        let ask_weight = ask_coverage * params.ask_rgb.a;
        alpha = bid_weight + ask_weight;
        // Combine both disjoint areas before alpha blending to avoid a dark shared edge.
        premultiplied = input.color.rgb * bid_weight + params.ask_rgb.rgb * ask_weight;
        if alpha > 0.0 {
            return vec4<f32>(premultiplied / alpha, clamp(alpha, 0.0, 1.0));
        }

        return vec4<f32>(0.0);
    }

    let fade = select(fade_factor(input.world_x), 1.0, input.fade_mode != 0u);
    let visibility = fade * subpx_visibility(input.subpx_alpha);
    return vec4<f32>(input.color.rgb, input.color.a * visibility);
}
