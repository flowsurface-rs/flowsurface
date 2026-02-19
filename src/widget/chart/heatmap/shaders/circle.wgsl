// Camera/Params + shared helpers come from common.wgsl

struct VertexInput {
    @location(0) corner: vec2<f32>,
    @location(1) y_world: f32,
    @location(2) x_bin_rel: i32,
    @location(3) x_frac: f32,
    @location(4) radius_px: f32,
    @location(5) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) world_x: f32,
    @location(3) radius_px: f32,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let scale = camera.a.xy;

    let col_w = params.grid.x;
    let now_bucket_rel_f = params.origin.x;

    let x_trade = f32(input.x_bin_rel) + input.x_frac;
    let center_x = bucket_rel_to_world_x(x_trade, now_bucket_rel_f, col_w);
    let snapped_center_y = y_bin_center_world(input.y_world);
    let center_y = mix(input.y_world, snapped_center_y, y_blend_weight());
    let center = vec2<f32>(center_x, center_y);

    let radius_world = vec2<f32>(
        input.radius_px / max(scale.x, 1e-6),
        input.radius_px / max(scale.y, 1e-6),
    );

    let world_pos = center + input.corner * radius_world;

    var out: VertexOutput;
    out.pos = world_to_clip(world_pos);
    out.local = input.corner;
    out.color = input.color;
    out.world_x = world_pos.x;
    out.radius_px = input.radius_px;

    return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let d_px = length(input.local) * input.radius_px;

    let feather_px = 1.0;
    let r = input.radius_px;
    let inner = max(r - feather_px, 0.0);
    let outer = max(r, 1e-6);

    let a = 1.0 - smoothstep(inner, outer, d_px);
    if (a <= 0.0) {
        discard;
    }

    let fade = fade_factor(input.world_x);
    return vec4<f32>(input.color.rgb * a * fade, input.color.a * a * fade);
}