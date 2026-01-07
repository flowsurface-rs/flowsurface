use exchange::util::{Price, PriceStep};

use crate::widget::chart::heatmap::scene::camera::Camera;

#[derive(Debug, Clone, Copy)]
pub struct ViewConfig {
    pub min_camera_scale: f32,

    // Overlays
    pub profile_col_width_px: f32,
    pub strip_height_frac: f32,

    // Y downsampling
    pub depth_min_row_px: f32,
    pub max_steps_per_y_bin: i64,

    // Row clamp
    pub min_row_h_world: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct ViewInputs {
    pub aggr_time: u64,
    pub latest_time_data: u64,
    pub latest_time_render: u64,

    pub base_price: Price,
    pub step: PriceStep,

    pub row_h_world: f32,
    pub col_w_world: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct ViewWindow {
    // Derived time window (padded; used for building buffers/textures safely)
    pub aggr_time: u64,
    pub earliest: u64,
    pub latest_vis: u64,
    pub latest_bucket: i64,

    // NEW: strict visible time window (no extra +/- buckets; use for normalization)
    pub earliest_strict: u64,
    pub latest_vis_strict: u64,

    // Derived price window
    pub lowest: Price,
    pub highest: Price,
    pub row_h: f32,

    // Camera scale (world->px)
    pub sx: f32,
    pub sy: f32,

    // Overlays
    pub profile_max_w_world: f32,
    pub strip_h_world: f32,
    pub strip_bottom_y: f32,

    // Y downsampling
    pub steps_per_y_bin: i64,
    pub y_bin_h_world: f32,
}

impl ViewWindow {
    pub fn compute(
        cfg: ViewConfig,
        camera: &Camera,
        viewport_px: [f32; 2],
        input: ViewInputs,
    ) -> Option<Self> {
        let [vw_px, vh_px] = viewport_px;

        if input.aggr_time == 0 || input.latest_time_data == 0 {
            return None;
        }

        let sx = camera.scale[0].max(cfg.min_camera_scale);
        let sy = camera.scale[1].max(cfg.min_camera_scale);

        // world x-range visible (plus margins)
        let x_max = camera.right_edge(vw_px);
        let x_min = x_max - (vw_px / sx);

        // Strict buckets (what is actually visible)
        let bucket_min_strict = (x_min / input.col_w_world).floor() as i64;
        let bucket_max_strict = (x_max / input.col_w_world).ceil() as i64;

        // Padded buckets (used for building content without edge artifacts)
        let bucket_min = bucket_min_strict.saturating_sub(2);
        let bucket_max = bucket_max_strict.saturating_add(2);

        // world y-range visible
        let y_center = camera.offset[1];
        let half_h_world = (vh_px / sy) * 0.5;
        let y_min = y_center - half_h_world;
        let y_max = y_center + half_h_world;

        // time range derived from buckets
        let latest_time_for_view: u64 = input.latest_time_render.max(input.latest_time_data);

        let latest_t = latest_time_for_view as i128;
        let aggr_i = input.aggr_time as i128;

        let t_min_i = latest_t + (bucket_min as i128) * aggr_i;
        let t_max_i = latest_t + (bucket_max as i128) * aggr_i;

        let earliest = t_min_i.clamp(0, latest_t) as u64;
        let latest_vis = t_max_i.clamp(0, latest_t) as u64;

        // strict time window (no padding)
        let t_min_strict_i = latest_t + (bucket_min_strict as i128) * aggr_i;
        let t_max_strict_i = latest_t + (bucket_max_strict as i128) * aggr_i;

        let earliest_strict = t_min_strict_i.clamp(0, latest_t) as u64;
        let latest_vis_strict = t_max_strict_i.clamp(0, latest_t) as u64;

        if earliest >= latest_vis {
            return None;
        }

        let row_h = input.row_h_world.max(cfg.min_row_h_world);

        let min_steps = (-(y_max) / row_h).floor() as i64;
        let max_steps = (-(y_min) / row_h).ceil() as i64;

        let lowest = input.base_price.add_steps(min_steps, input.step);
        let highest = input.base_price.add_steps(max_steps, input.step);

        let latest_bucket: i64 = (latest_time_for_view / input.aggr_time) as i64;

        // overlays (profile width depends on how much x>0 is visible)
        let full_right_edge = camera.right_edge(vw_px);

        let visible_space_right_of_zero_world = (full_right_edge - 0.0).max(0.0);
        let desired_profile_w_world = (cfg.profile_col_width_px / sx).max(0.0);
        let profile_max_w_world = desired_profile_w_world.min(visible_space_right_of_zero_world);

        let strip_h_world: f32 = (vh_px * cfg.strip_height_frac) / sy;
        let strip_bottom_y: f32 = y_max;

        // y-downsampling
        let px_per_step = row_h * sy;
        let mut steps_per_y_bin: i64 = 1;
        if px_per_step.is_finite() && px_per_step > 0.0 {
            steps_per_y_bin = (cfg.depth_min_row_px / px_per_step).ceil() as i64;
            steps_per_y_bin = steps_per_y_bin.clamp(1, cfg.max_steps_per_y_bin.max(1));
        }
        let y_bin_h_world = row_h * steps_per_y_bin as f32;

        Some(ViewWindow {
            aggr_time: input.aggr_time,
            earliest,
            latest_vis,
            latest_bucket,
            earliest_strict,
            latest_vis_strict,
            lowest,
            highest,
            row_h,
            sx,
            sy,
            profile_max_w_world,
            strip_h_world,
            strip_bottom_y,
            steps_per_y_bin,
            y_bin_h_world,
        })
    }
}
