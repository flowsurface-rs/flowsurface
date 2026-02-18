use crate::widget::chart::heatmap::scene::{
    camera::Camera,
    cell::{Cell, MIN_ROW_PX},
};
use data::chart::heatmap::HistoricalDepth;
use exchange::unit::{Price, PriceStep};

use iced::time::Instant;

// Throttle depth denom recompute while interacting (keeps zoom smooth)
const NORM_RECOMPUTE_THROTTLE_MS: u64 = 100;

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

    pub fn x_phase_bucket(&self) -> f32 {
        match self {
            Anchor::Live { x_phase_bucket, .. } => *x_phase_bucket,
            Anchor::Paused { x_phase_bucket, .. } => *x_phase_bucket,
        }
    }

    /// Update monotonic render time + phase while Live.
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

    /// Render time used for view/overlay computations.
    /// While paused, clamp to the latest bucket we actually have data for to avoid "future" drift.
    pub fn effective_render_latest_time(&self, latest_time: u64) -> u64 {
        let render_latest_time = self.render_latest_time();
        if self.is_paused() && render_latest_time > 0 && latest_time > 0 {
            render_latest_time.min(latest_time)
        } else {
            render_latest_time
        }
    }

    /// Handles manual resume from pause. Returns pending_price to apply and whether camera should reset.
    pub fn resume_from_pause(self) -> Result<(Anchor, Option<Price>), ()> {
        match self {
            Anchor::Live { .. } => Err(()), // Can't resume when already live
            Anchor::Paused {
                pending_mid_price,
                scroll_ref_bucket,
                render_latest_time,
                x_phase_bucket,
                ..
            } => {
                let new_anchor = Anchor::Live {
                    scroll_ref_bucket,
                    render_latest_time,
                    x_phase_bucket,
                    resume: ResumeAction::FullRebuildFromHistorical,
                };
                Ok((new_anchor, pending_mid_price))
            }
        }
    }

    /// Updates anchor state based on x=0 visibility.
    /// Returns (state_changed, pending_price_to_apply)
    pub fn update_auto_follow(
        &mut self,
        x0_visible: bool,
        live_render_latest_time: u64,
        live_x_phase_bucket: f32,
    ) -> (bool, Option<Price>) {
        match self {
            Anchor::Live {
                scroll_ref_bucket,
                render_latest_time,
                x_phase_bucket,
                ..
            } => {
                if !x0_visible {
                    // Transition to Paused
                    *self = Anchor::Paused {
                        render_latest_time: live_render_latest_time.max(*render_latest_time),
                        x_phase_bucket: live_x_phase_bucket.max(*x_phase_bucket),
                        pending_mid_price: None,
                        scroll_ref_bucket: *scroll_ref_bucket,
                        resume: ResumeAction::FullRebuildFromHistorical,
                    };
                    (true, None)
                } else {
                    (false, None)
                }
            }
            Anchor::Paused {
                pending_mid_price,
                scroll_ref_bucket,
                render_latest_time,
                x_phase_bucket,
                resume,
            } => {
                if x0_visible {
                    let price_to_apply = *pending_mid_price;

                    // Transition to Live
                    *self = Anchor::Live {
                        scroll_ref_bucket: *scroll_ref_bucket,
                        render_latest_time: *render_latest_time,
                        x_phase_bucket: *x_phase_bucket,
                        resume: *resume,
                    };

                    (true, price_to_apply)
                } else {
                    (false, None)
                }
            }
        }
    }

    /// Apply a new mid price update.
    /// - If paused: store as pending (so resuming can snap cleanly)
    /// - If live: immediately updates `base_price`
    pub fn apply_mid_price(&mut self, mid_rounded: Price, base_price: &mut Option<Price>) {
        match self {
            Anchor::Paused {
                pending_mid_price, ..
            } => *pending_mid_price = Some(mid_rounded),
            Anchor::Live { .. } => *base_price = Some(mid_rounded),
        }
    }

    /// Ensure scroll_ref_bucket is initialized and compute origin.x (bucket delta + phase).
    /// Returns (scroll_ref_bucket, origin_x).
    pub fn sync_scroll_ref_and_origin_x(&mut self, render_bucket: i64) -> (i64, f32) {
        self.set_scroll_ref_bucket_if_zero(render_bucket);
        let scroll_ref_bucket = self.scroll_ref_bucket();

        let delta_buckets: i64 = render_bucket - scroll_ref_bucket;
        let origin_x: f32 = (delta_buckets as f32) + self.x_phase_bucket();

        (scroll_ref_bucket, origin_x)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ResumeAction {
    None,
    FullRebuildFromHistorical,
}

#[derive(Default, Debug, Clone, Copy)]
pub enum RebuildPolicy {
    /// Full rebuild should run immediately
    Immediate { force_rebuild_from_historical: bool },
    /// Full rebuild should run once interaction settles
    Debounced {
        last_input: Instant,
        force_rebuild_from_historical: bool,
    },
    /// No pending rebuild requested.
    #[default]
    Idle,
}

impl RebuildPolicy {
    #[inline]
    pub fn immediate() -> Self {
        RebuildPolicy::Immediate {
            force_rebuild_from_historical: false,
        }
    }

    /// Promote current policy to Immediate while preserving any pending "force from historical".
    #[inline]
    pub fn promote_to_immediate(self) -> Self {
        match self {
            RebuildPolicy::Idle => RebuildPolicy::immediate(),
            RebuildPolicy::Immediate {
                force_rebuild_from_historical,
            } => RebuildPolicy::Immediate {
                force_rebuild_from_historical,
            },
            RebuildPolicy::Debounced {
                force_rebuild_from_historical,
                ..
            } => RebuildPolicy::Immediate {
                force_rebuild_from_historical,
            },
        }
    }

    /// Request that the next full rebuild is done by rebuilding from historical (one-shot).
    /// If currently Idle, this also schedules an Immediate rebuild.
    #[inline]
    pub fn request_rebuild_from_historical(self) -> Self {
        match self {
            RebuildPolicy::Idle => RebuildPolicy::Immediate {
                force_rebuild_from_historical: true,
            },
            RebuildPolicy::Immediate { .. } => RebuildPolicy::Immediate {
                force_rebuild_from_historical: true,
            },
            RebuildPolicy::Debounced { last_input, .. } => RebuildPolicy::Debounced {
                last_input,
                force_rebuild_from_historical: true,
            },
        }
    }

    /// Consume the one-shot directive (used by `rebuild_all()`).
    #[inline]
    pub fn take_force_rebuild_from_historical(&mut self) -> bool {
        match self {
            RebuildPolicy::Immediate {
                force_rebuild_from_historical,
            } => std::mem::replace(force_rebuild_from_historical, false),
            RebuildPolicy::Debounced {
                force_rebuild_from_historical,
                ..
            } => std::mem::replace(force_rebuild_from_historical, false),
            RebuildPolicy::Idle => false,
        }
    }

    pub fn mark_input(self, now: Instant) -> Self {
        match self {
            RebuildPolicy::Immediate {
                force_rebuild_from_historical,
            } => RebuildPolicy::Debounced {
                last_input: now,
                force_rebuild_from_historical,
            },
            RebuildPolicy::Debounced {
                force_rebuild_from_historical,
                ..
            } => RebuildPolicy::Debounced {
                last_input: now,
                force_rebuild_from_historical,
            },
            RebuildPolicy::Idle => RebuildPolicy::Debounced {
                last_input: now,
                force_rebuild_from_historical: false,
            },
        }
    }

    #[inline]
    pub fn decide(self, now: Instant, debounce_ms: u64) -> (bool, bool, RebuildPolicy) {
        match self {
            RebuildPolicy::Immediate { .. } => (false, true, RebuildPolicy::Idle),
            RebuildPolicy::Idle => (false, false, RebuildPolicy::Idle),
            RebuildPolicy::Debounced {
                last_input,
                force_rebuild_from_historical,
            } => {
                let due =
                    (now.saturating_duration_since(last_input).as_millis() as u64) >= debounce_ms;
                if due {
                    (true, true, RebuildPolicy::Idle)
                } else {
                    (
                        true,
                        false,
                        RebuildPolicy::Debounced {
                            last_input,
                            force_rebuild_from_historical,
                        },
                    )
                }
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
    // Overlays
    pub profile_col_width_px: f32,
    pub volume_area_height_pct: f32,
    pub volume_profile_width_pct: f32,

    // Y downsampling
    pub max_steps_per_y_bin: i64,
}

#[derive(Debug, Clone, Copy)]
pub struct ViewInputs {
    pub aggr_time: u64,
    pub latest_time_data: u64,
    pub latest_time_render: u64,

    pub base_price: Price,
    pub step: PriceStep,

    pub cell: Cell,
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
    pub cam_scale: f32,

    // Overlays
    pub volume_profile_max_width: f32,
    pub depth_profile_max_width: f32,
    pub volume_area_max_height: f32,
    pub volume_area_bottom_y: f32,

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
        viewport_px: iced::Size,
        input: ViewInputs,
    ) -> Option<Self> {
        let [vw_px, vh_px] = viewport_px.into();

        if input.aggr_time == 0 || input.latest_time_data == 0 {
            return None;
        }

        let cam_scale = camera.scale();

        // world x-range visible (plus margins)
        let x_max = camera.right_edge(vw_px);
        let x_min = x_max - (vw_px / cam_scale);

        let col_w_world = input.cell.width_world();

        // Strict buckets (what is actually visible)
        let bucket_min_strict = (x_min / col_w_world).floor() as i64;
        let bucket_max_strict = (x_max / col_w_world).ceil() as i64;

        // Padded buckets (used for building content without edge artifacts)
        let bucket_min = bucket_min_strict.saturating_sub(2);
        let bucket_max = bucket_max_strict.saturating_add(2);

        // world y-range visible
        let y_center = camera.offset[1];
        let half_h_world = (vh_px / cam_scale) * 0.5;
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

        let row_h = input.cell.height_world();

        let min_steps = (-(y_max) / row_h).floor() as i64;
        let max_steps = (-(y_min) / row_h).ceil() as i64;

        let lowest = input.base_price.add_steps(min_steps, input.step);
        let highest = input.base_price.add_steps(max_steps, input.step);

        // overlays (profile width depends on how much x>0 is visible)
        let visible_space_right_of_zero_world = (x_max - 0.0).max(0.0);
        let depth_profile_max_width =
            (cfg.profile_col_width_px / cam_scale).clamp(0.0, visible_space_right_of_zero_world);

        let strip_h_world: f32 = (vh_px * cfg.volume_area_height_pct) / cam_scale;
        let strip_bottom_y: f32 = y_max;

        let visible_w_world = vw_px / cam_scale;
        let volume_profile_max_width = visible_w_world * cfg.volume_profile_width_pct;

        // y-downsampling
        let px_per_step = row_h * cam_scale;
        let mut steps_per_y_bin: i64 = 1;
        if px_per_step.is_finite() && px_per_step > 0.0 {
            steps_per_y_bin = (MIN_ROW_PX / px_per_step).ceil() as i64;
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
            cam_scale,
            volume_profile_max_width,
            depth_profile_max_width,
            volume_area_max_height: strip_h_world,
            volume_area_bottom_y: strip_bottom_y,
            left_edge_world: x_min,
            steps_per_y_bin,
            y_bin_h_world,
        })
    }

    /// Shader-consistent mapping: price -> y-bin (using Euclidean division, matching `floor`).
    pub fn y_bin_for_price(&self, price: Price, base_price: Price, step: PriceStep) -> i64 {
        let step_units = step.units.max(1);
        let steps_per = self.steps_per_y_bin.max(1);

        let dy_steps: i64 = (price.units - base_price.units).div_euclid(step_units);
        dy_steps.div_euclid(steps_per)
    }

    /// Shader-consistent y-center for a y-bin (center of the bin).
    pub fn y_center_for_bin(&self, y_bin: i64) -> f32 {
        -((y_bin as f32 + 0.5) * self.y_bin_h_world)
    }

    /// Convenience: price -> y-center in world coordinates (bin-centered).
    pub fn y_center_for_price(&self, price: Price, base_price: Price, step: PriceStep) -> f32 {
        let yb = self.y_bin_for_price(price, base_price, step);
        self.y_center_for_bin(yb)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NormKey {
    start_bucket: i64,
    end_bucket_excl: i64,
    y0_bin: i64,
    y1_bin: i64,
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

    fn make_key(&self, w: &ViewWindow, latest_incl: u64, step: PriceStep) -> NormKey {
        let aggr = w.aggr_time.max(1);
        let start_bucket = (w.earliest / aggr) as i64;
        let end_bucket_excl = latest_incl.div_ceil(aggr) as i64;

        let step_units = step.units.max(1);
        let y_div = w.steps_per_y_bin.max(1);

        let mut y0_bin = (w.lowest.units / step_units).div_euclid(y_div);
        let mut y1_bin = (w.highest.units / step_units).div_euclid(y_div);
        if y0_bin > y1_bin {
            std::mem::swap(&mut y0_bin, &mut y1_bin);
        }

        NormKey {
            start_bucket,
            end_bucket_excl,
            y0_bin,
            y1_bin,
        }
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
    ) -> f32 {
        let key = self.make_key(w, latest_incl, step);
        let key_changed = self.key != Some(key);

        let throttle_ms = if is_interacting {
            NORM_RECOMPUTE_THROTTLE_MS
        } else {
            w.aggr_time.max(1)
        };

        let dt_ms = self
            .last_recompute
            .map(|last| now.saturating_duration_since(last).as_millis() as u64)
            .unwrap_or(u64::MAX);

        if !key_changed && dt_ms < throttle_ms {
            return self.value.max(1e-6);
        }

        self.last_recompute = Some(now);
        self.compute_with_key(hist, w, latest_incl, key, data_gen)
    }

    fn compute_with_key(
        &mut self,
        hist: &HistoricalDepth,
        w: &ViewWindow,
        latest_incl: u64,
        key: NormKey,
        data_gen: u64,
    ) -> f32 {
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

/// Round a millisecond timestamp down to the start of its aggregation bucket
pub fn round_time_to_bucket(t_ms: u64, aggr_time: u64) -> u64 {
    let aggr_time = aggr_time.max(1);
    (t_ms / aggr_time) * aggr_time
}

/// Computes "live" render timing from an exchange clock
///
/// Returns:
/// - `bucketed`: exchange_now rounded down to bucket start
/// - `live_render_latest_time`: monotonic render latest time while live (paused stays frozen)
/// - `live_phase_bucket`: fractional phase within the current bucket [0, 1)
pub fn live_timing(anchor: Anchor, exchange_now_ms: u64, aggr_time: u64) -> (u64, u64, f32) {
    let aggr_time = aggr_time.max(1);
    let bucketed = round_time_to_bucket(exchange_now_ms, aggr_time);

    let live_render_latest_time = match anchor {
        Anchor::Live {
            render_latest_time, ..
        } => render_latest_time.max(bucketed),
        Anchor::Paused {
            render_latest_time, ..
        } => render_latest_time,
    };

    let live_phase_ms = exchange_now_ms.saturating_sub(live_render_latest_time);
    let live_phase_bucket = (live_phase_ms as f32 / aggr_time as f32).clamp(0.0, 0.999_999);

    (bucketed, live_render_latest_time, live_phase_bucket)
}
