use bytemuck::{Pod, Zeroable};

// Shift volume-strip rects left by half a bucket to align with circle centers
const VOLUME_X_SHIFT_BUCKET: f32 = -0.5;

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct ParamsUniform {
    /// (max_depth_denom, alpha_min, alpha_max, *pad*)
    depth: [f32; 4],
    /// (r, g, b, *pad*)
    bid_rgb: [f32; 4],
    /// (r, g, b, *pad*)
    ask_rgb: [f32; 4],
    /// (cell_w_world, cell_h_world, steps_per_y_bin, *pad*)
    grid: [f32; 4],
    /// (now_bucket_rel_with_phase, volume_min_w_world, volume_gap_frac, volume_x_shift_bucket)
    origin: [f32; 4],
    /// (latest_rel_bucket, y_start_bin, *pad*, latest_x_ring)
    heatmap_map: [f32; 4],
    /// (tex_w, tex_h, tex_w_mask=(tex_w-1, requires power-of-two width), inv_qty_scale)
    heatmap_tex: [f32; 4],
    /// (x_left_world, width_world, alpha_min, alpha_max)
    fade: [f32; 4],
}

impl Default for ParamsUniform {
    fn default() -> Self {
        Self {
            depth: [1.0, 0.01, 0.99, 0.0],
            bid_rgb: [0.0, 1.0, 0.0, 0.0],
            ask_rgb: [1.0, 0.0, 0.0, 0.0],
            grid: [0.1, 0.1, 1.0, 0.0],
            origin: [0.0, 0.0, 0.0, VOLUME_X_SHIFT_BUCKET],
            heatmap_map: [0.0, 0.0, 1.0, 0.0],
            heatmap_tex: [0.0, 0.0, 0.0, 0.0],
            fade: [0.0, 0.0, 1.0, 1.0],
        }
    }
}

impl ParamsUniform {
    pub fn set_cell_world(&mut self, width: f32, height: f32) {
        self.grid[0] = width;
        self.grid[1] = height;
    }

    pub fn set_steps_per_y_bin(&mut self, steps_per_y_bin: i64) {
        self.grid[2] = (steps_per_y_bin.max(1)) as f32;
    }

    pub fn steps_per_y_bin(&self) -> i64 {
        self.grid[2].round().max(1.0) as i64
    }

    pub fn set_origin_x(&mut self, x: f32) {
        self.origin[0] = x;
    }

    pub fn set_heatmap_latest_rel_bucket(&mut self, latest_rel_bucket: i64) {
        self.heatmap_map[0] = latest_rel_bucket as f32;
    }

    pub fn set_heatmap_y_start_bin(&mut self, y_start_bin: f32) {
        self.heatmap_map[1] = y_start_bin;
    }

    pub fn set_heatmap_latest_x_ring(&mut self, x_ring: u32) {
        self.heatmap_map[3] = x_ring as f32;
    }

    pub fn set_heatmap_tex_info(&mut self, tex_w: u32, tex_h: u32, qty_scale_inv: f32) {
        debug_assert!(
            tex_w.is_power_of_two(),
            "heatmap tex width must be power-of-two because shader wraps with bitmask (& (tex_w-1))"
        );

        self.heatmap_tex = [
            tex_w as f32,
            tex_h as f32,
            (tex_w.saturating_sub(1)) as f32,
            qty_scale_inv,
        ];
    }

    pub fn set_palette_rgb(&mut self, bid_rgb: [f32; 3], ask_rgb: [f32; 3]) {
        self.bid_rgb = [bid_rgb[0], bid_rgb[1], bid_rgb[2], 0.0];
        self.ask_rgb = [ask_rgb[0], ask_rgb[1], ask_rgb[2], 0.0];
    }

    pub fn set_depth_denom(&mut self, denom: f32) {
        self.depth[0] = denom;
    }

    pub fn set_trade_fade(
        &mut self,
        x_left_world: f32,
        width_world: f32,
        alpha_min: f32,
        alpha_max: f32,
    ) {
        self.fade = [x_left_world, width_world, alpha_min, alpha_max];
    }

    pub fn fade_start(&self) -> f32 {
        self.fade[0]
    }

    pub fn fade_width(&self) -> f32 {
        self.fade[1]
    }

    pub fn heatmap_start_bin(&self) -> i64 {
        self.heatmap_map[1].round() as i64
    }

    pub fn origin_x(&self) -> f32 {
        self.origin[0]
    }
}
