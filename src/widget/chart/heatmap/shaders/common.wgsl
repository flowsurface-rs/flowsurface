struct Camera {
    a: vec4<f32>, // (scale_x, scale_y, center_x, center_y)
    b: vec4<f32>, // (viewport_w, viewport_h, _, _)
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

fn world_to_clip(world_pos: vec2<f32>) -> vec4<f32> {
    let scale = camera.a.xy;
    let center = camera.a.zw;
    let viewport = camera.b.xy;

    let view = (world_pos - center) * scale;
    let ndc_x = view.x / (viewport.x * 0.5);
    let ndc_y = -view.y / (viewport.y * 0.5);
    return vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
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

fn snap_ndc_x_to_pixel_edge(ndc_x: f32) -> f32 {
    let viewport_x = max(camera.b.x, 1.0);
    let screen_x = (ndc_x * 0.5 + 0.5) * viewport_x;
    let snapped_x = round(screen_x);
    return ((snapped_x / viewport_x) - 0.5) * 2.0;
}

fn bucket_rel_to_world_x(bucket_rel: f32, now_bucket_rel_f: f32, col_w: f32) -> f32 {
    return -((now_bucket_rel_f - bucket_rel) * col_w);
}

fn fade_factor(world_x: f32) -> f32 {
    let x0 = params.fade.x;
    let w = max(params.fade.y, 1e-6);
    let t = clamp((world_x - x0) / w, 0.0, 1.0);

    var s = smoothstep(0.0, 1.0, t);
    s = s * s;

    return params.fade.z + s * (params.fade.w - params.fade.z);
}