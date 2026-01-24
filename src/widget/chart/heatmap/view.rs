use crate::widget::chart::heatmap::scene::camera::Camera;
use data::chart::heatmap::HistoricalDepth;
use exchange::util::{Price, PriceStep};

use iced::time::Instant;

#[derive(Debug, Clone, Copy)]
pub enum Anchor {
    Live {
        scroll_ref_bucket: i64,
        render_latest_time: u64,
        x_phase_bucket: f32,
        /// One-shot directive to be consumed by the next rebuild.
        resume: ResumeAction,
    },
    Paused {
        scroll_ref_bucket: i64,
        render_latest_time: u64,
        x_phase_bucket: f32,
        pending_mid_price: Option<Price>,
        /// Applied when transitioning to Live (then consumed in Live).
        resume: ResumeAction,
    },
}

impl Default for Anchor {
    fn default() -> Self {
        Anchor::Live {
            scroll_ref_bucket: 0,
            render_latest_time: 0,
            x_phase_bucket: 0.0,
            resume: ResumeAction::None,
        }
    }
}

impl Anchor {
    pub fn is_paused(&self) -> bool {
        matches!(self, Anchor::Paused { .. })
    }

    #[inline]
    pub fn scroll_ref_bucket(&self) -> i64 {
        match self {
            Anchor::Live {
                scroll_ref_bucket, ..
            } => *scroll_ref_bucket,
            Anchor::Paused {
                scroll_ref_bucket, ..
            } => *scroll_ref_bucket,
        }
    }

    #[inline]
    pub fn set_scroll_ref_bucket_if_zero(&mut self, v: i64) {
        let slot = match self {
            Anchor::Live {
                scroll_ref_bucket, ..
            } => scroll_ref_bucket,
            Anchor::Paused {
                scroll_ref_bucket, ..
            } => scroll_ref_bucket,
        };
        if *slot == 0 {
            *slot = v;
        }
    }

    #[inline]
    pub fn render_latest_time(&self) -> u64 {
        match self {
            Anchor::Live {
                render_latest_time, ..
            } => *render_latest_time,
            Anchor::Paused {
                render_latest_time, ..
            } => *render_latest_time,
        }
    }

    #[inline]
    pub fn x_phase_bucket(&self) -> f32 {
        match self {
            Anchor::Live { x_phase_bucket, .. } => *x_phase_bucket,
            Anchor::Paused { x_phase_bucket, .. } => *x_phase_bucket,
        }
    }

    /// Update monotonic render time + phase while Live.
    #[inline]
    pub fn update_live_timing(&mut self, bucketed_time: u64, phase_bucket: f32) {
        if let Anchor::Live {
            render_latest_time,
            x_phase_bucket,
            ..
        } = self
        {
            *render_latest_time = (*render_latest_time).max(bucketed_time);
            *x_phase_bucket = phase_bucket;
        }
    }

    /// Consume the one-shot resume directive (Live only; Paused is not consumed).
    #[inline]
    pub fn take_live_resume(&mut self) -> ResumeAction {
        match self {
            Anchor::Live { resume, .. } => {
                let r = *resume;
                *resume = ResumeAction::None;
                r
            }
            Anchor::Paused { .. } => ResumeAction::None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ResumeAction {
    None,
    FullRebuildFromHistorical,
}

#[derive(Debug, Clone, Copy)]
pub enum RebuildPolicy {
    /// Full rebuild should run immediately (rare: resize/layout).
    Immediate,
    /// Full rebuild should run once interaction settles.
    Debounced { last_input: Instant },
    /// No pending rebuild requested.
    Idle,
}

impl RebuildPolicy {
    pub fn mark_input(self, now: Instant) -> Self {
        RebuildPolicy::Debounced { last_input: now }
    }

    #[allow(dead_code)]
    pub fn should_rebuild(self, now: Instant, debounce_ms: u64) -> bool {
        match self {
            RebuildPolicy::Immediate => true,
            RebuildPolicy::Idle => false,
            RebuildPolicy::Debounced { last_input } => {
                (now.saturating_duration_since(last_input).as_millis() as u64) >= debounce_ms
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ExchangeClock {
    Uninit,
    Anchored {
        anchor_exchange_ms: u64,
        anchor_instant: Instant,
        monotonic_estimate_ms: u64,
    },
}

impl ExchangeClock {
    pub fn anchor_with_update(self, depth_update_t: u64) -> Self {
        let now = Instant::now();

        let predicted = match self {
            ExchangeClock::Anchored {
                anchor_exchange_ms,
                anchor_instant,
                monotonic_estimate_ms,
            } => {
                let elapsed_ms = now.saturating_duration_since(anchor_instant).as_millis() as u64;
                let p = anchor_exchange_ms.saturating_add(elapsed_ms);
                p.max(monotonic_estimate_ms)
            }
            ExchangeClock::Uninit => 0,
        };

        let monotonic = depth_update_t.max(predicted);

        ExchangeClock::Anchored {
            anchor_exchange_ms: monotonic,
            anchor_instant: now,
            monotonic_estimate_ms: monotonic,
        }
    }

    pub fn estimate_now_ms(self, now: Instant) -> Option<u64> {
        match self {
            ExchangeClock::Uninit => None,
            ExchangeClock::Anchored {
                anchor_exchange_ms,
                anchor_instant,
                monotonic_estimate_ms,
            } => {
                let elapsed_ms = now.saturating_duration_since(anchor_instant).as_millis() as u64;
                Some(
                    anchor_exchange_ms
                        .saturating_add(elapsed_ms)
                        .max(monotonic_estimate_ms),
                )
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ViewConfig {
    pub min_camera_scale: f32,

    // Overlays
    pub profile_col_width_px: f32,
    pub strip_height_frac: f32,
    pub trade_profile_width_frac: f32,

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
    pub trade_profile_max_w_world: f32,

    // World x bounds
    pub left_edge_world: f32,

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

        if earliest >= latest_vis {
            return None;
        }

        let row_h = input.row_h_world.max(cfg.min_row_h_world);

        let min_steps = (-(y_max) / row_h).floor() as i64;
        let max_steps = (-(y_min) / row_h).ceil() as i64;

        let lowest = input.base_price.add_steps(min_steps, input.step);
        let highest = input.base_price.add_steps(max_steps, input.step);

        // overlays (profile width depends on how much x>0 is visible)
        let full_right_edge = camera.right_edge(vw_px);

        let visible_space_right_of_zero_world = (full_right_edge - 0.0).max(0.0);
        let desired_profile_w_world = (cfg.profile_col_width_px / sx).max(0.0);
        let profile_max_w_world = desired_profile_w_world.min(visible_space_right_of_zero_world);

        let strip_h_world: f32 = (vh_px * cfg.strip_height_frac) / sy;
        let strip_bottom_y: f32 = y_max;

        let visible_w_world = vw_px / sx;
        let trade_profile_max_w_world = visible_w_world * cfg.trade_profile_width_frac;

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
            lowest,
            highest,
            row_h,
            sx,
            sy,
            profile_max_w_world,
            strip_h_world,
            strip_bottom_y,
            trade_profile_max_w_world,
            left_edge_world: x_min,
            steps_per_y_bin,
            y_bin_h_world,
        })
    }

    /// Shader-consistent mapping: price -> y-bin (using Euclidean division, matching `floor`).
    #[inline]
    pub fn y_bin_for_price(&self, price: Price, base_price: Price, step: PriceStep) -> i64 {
        let step_units = step.units.max(1);
        let steps_per = self.steps_per_y_bin.max(1);

        let dy_steps: i64 = (price.units - base_price.units).div_euclid(step_units);
        dy_steps.div_euclid(steps_per)
    }

    /// Shader-consistent y-center for a y-bin (center of the bin).
    #[inline]
    pub fn y_center_for_bin(&self, y_bin: i64) -> f32 {
        -((y_bin as f32 + 0.5) * self.y_bin_h_world)
    }

    /// Convenience: price -> y-center in world coordinates (bin-centered).
    #[inline]
    pub fn y_center_for_price(&self, price: Price, base_price: Price, step: PriceStep) -> f32 {
        let yb = self.y_bin_for_price(price, base_price, step);
        self.y_center_for_bin(yb)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NormKey {
    pub start_bucket: i64,
    pub end_bucket_excl: i64,
    pub y0_bin: i64,
    pub y1_bin: i64,
}

#[derive(Debug)]
pub struct DepthNormCache {
    key: Option<NormKey>,
    value: f32,
    generation: u64,
    last_recompute: Option<Instant>,
}

impl DepthNormCache {
    pub fn new() -> Self {
        Self {
            key: None,
            value: 1.0,
            generation: 0,
            last_recompute: None,
        }
    }

    pub fn invalidate(&mut self) {
        self.key = None;
    }

    pub fn compute_throttled(
        &mut self,
        hist: &HistoricalDepth,
        w: &ViewWindow,
        latest_incl: u64,
        step: PriceStep,
        data_gen: u64,
        now: Instant,
        is_interacting: bool,
        throttle_ms: u64,
    ) -> f32 {
        if is_interacting && let Some(last) = self.last_recompute {
            let dt_ms = now.saturating_duration_since(last).as_millis() as u64;
            if dt_ms < throttle_ms {
                return self.value.max(1e-6);
            }
        }

        self.last_recompute = Some(now);
        self.compute(hist, w, latest_incl, step, data_gen)
    }

    pub fn compute(
        &mut self,
        hist: &HistoricalDepth,
        w: &ViewWindow,
        latest_incl: u64,
        step: PriceStep,
        data_gen: u64,
    ) -> f32 {
        let aggr = w.aggr_time.max(1);
        let start_bucket = (w.earliest / aggr) as i64;
        let end_bucket_excl = (latest_incl / aggr) as i64;

        let step_units = step.units.max(1);
        let y_div = w.steps_per_y_bin.max(1);

        let mut y0_bin = (w.lowest.units / step_units).div_euclid(y_div);
        let mut y1_bin = (w.highest.units / step_units).div_euclid(y_div);
        if y0_bin > y1_bin {
            std::mem::swap(&mut y0_bin, &mut y1_bin);
        }

        let key = NormKey {
            start_bucket,
            end_bucket_excl,
            y0_bin,
            y1_bin,
        };

        if self.key == Some(key) && self.generation == data_gen {
            return self.value.max(1e-6);
        }

        let max_qty = hist
            .max_qty_in_range_raw(w.earliest, latest_incl, w.highest, w.lowest)
            .max(1e-6);

        self.key = Some(key);
        self.value = max_qty;
        self.generation = data_gen;

        max_qty
    }
}
