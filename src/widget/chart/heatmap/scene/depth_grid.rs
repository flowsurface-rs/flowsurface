use std::sync::Arc;

use data::chart::heatmap::HistoricalDepth;
use exchange::depth::Depth;
use exchange::util::{Price, PriceStep};

use crate::widget::chart::heatmap::scene;

const Y_BLOCK_H: u32 = 16;
const DEPTH_GRID_GRACE_MS: u64 = 500;

#[derive(Debug, Clone)]
pub struct GridRing {
    /// How many aggregated buckets to keep in the ring.
    /// NOTE: texture width is still rounded up to a power-of-two for fast wrap.
    horizon_buckets: u32,

    /// tolerate short gaps/out-of-order updates (ms).
    pub grace_ms: u64,

    tex_w: u32,
    tex_h: u32,

    aggr_time_ms: u64,
    last_bucket: Option<i64>,

    y_anchor: Option<Price>,

    pub bid: Vec<u32>,
    pub ask: Vec<u32>,

    pub col_max_bid: Vec<u32>,
    pub col_max_ask: Vec<u32>,

    steps_per_y_bin: i64,

    full_dirty: bool,
    dirty_cols: Vec<u32>,

    block_max_bid: Vec<u32>,
    block_max_ask: Vec<u32>,
}

impl GridRing {
    pub fn new(horizon_buckets: u32, tex_h: u32) -> Self {
        Self {
            horizon_buckets: horizon_buckets.max(1),
            grace_ms: DEPTH_GRID_GRACE_MS,
            tex_w: 0,
            tex_h,

            aggr_time_ms: 0,
            last_bucket: None,

            y_anchor: None,

            bid: Vec::new(),
            ask: Vec::new(),

            col_max_bid: Vec::new(),
            col_max_ask: Vec::new(),

            steps_per_y_bin: 1,

            full_dirty: false,
            dirty_cols: Vec::new(),

            block_max_bid: Vec::new(),
            block_max_ask: Vec::new(),
        }
    }

    #[inline]
    fn y_block_count(&self) -> u32 {
        if self.tex_h == 0 {
            0
        } else {
            self.tex_h.div_ceil(Y_BLOCK_H)
        }
    }

    #[inline]
    fn block_idx(&self, x: u32, block_y: u32) -> usize {
        (block_y as usize) * (self.tex_w as usize) + (x as usize)
    }

    /// Returns `true` once after an internal reset/recenter/layout change that requires
    /// a full texture upload. Consumes the flag.
    #[inline]
    pub fn take_full_dirty(&mut self) -> bool {
        if self.full_dirty {
            self.full_dirty = false;
            self.dirty_cols.clear();
            true
        } else {
            false
        }
    }

    /// Returns the set of x-columns that changed since last call (sorted, deduped).
    /// If the grid is in "full dirty" state, returns an empty vec (caller should full-upload).
    pub fn drain_dirty_columns(&mut self) -> Vec<u32> {
        if self.full_dirty || self.dirty_cols.is_empty() {
            self.dirty_cols.clear();
            return Vec::new();
        }

        let mut out = std::mem::take(&mut self.dirty_cols);
        out.sort_unstable();
        out.dedup();
        out
    }

    #[inline]
    fn mark_full_dirty(&mut self) {
        self.full_dirty = true;
        self.dirty_cols.clear();
    }

    #[inline]
    fn mark_dirty(&mut self, x: u32) {
        if !self.full_dirty {
            self.dirty_cols.push(x);
        }
    }

    #[inline]
    pub fn tex_w(&self) -> u32 {
        self.tex_w
    }

    #[inline]
    pub fn tex_h(&self) -> u32 {
        self.tex_h
    }

    #[inline]
    fn grace_buckets(&self) -> i64 {
        let aggr = self.aggr_time_ms.max(1);
        let b = self.grace_ms.div_ceil(aggr) as i64;
        b.max(0)
    }

    /// Current y-anchor used for mapping (if any).
    #[inline]
    pub fn y_anchor_price(&self) -> Option<Price> {
        self.y_anchor
    }

    /// Current y-binning in steps.
    #[inline]
    pub fn steps_per_y_bin(&self) -> i64 {
        self.steps_per_y_bin.max(1)
    }

    /// Rebuild the entire ring grid from `HistoricalDepth` for the time window
    /// [oldest_time, latest_time] (bucketed by `aggr_time_ms`) using the current view binning.
    ///
    /// This is intended for interaction spikes (zoom/pan changes), not per-frame.
    pub fn rebuild_from_historical(
        &mut self,
        hist: &HistoricalDepth,
        oldest_time: u64,
        latest_time: u64,
        base_price: Price,
        step: PriceStep,
        steps_per_y_bin: i64,
        qty_scale: f32,
        highest: Price,
        lowest: Price,
    ) {
        if self.aggr_time_ms == 0 || self.tex_w == 0 || self.tex_h == 0 {
            return;
        }

        self.steps_per_y_bin = steps_per_y_bin.max(1);
        self.y_anchor = Some(base_price);

        self.clear_all();
        self.last_bucket = None;

        let step_units = step.units.max(1);
        let half_h = (self.tex_h as i64) / 2;

        // Clamp time window to bucket boundaries.
        let aggr = self.aggr_time_ms.max(1);
        let oldest_time = (oldest_time / aggr) * aggr;
        let latest_time = (latest_time / aggr) * aggr;

        let oldest_bucket: i64 = (oldest_time / aggr) as i64;
        let latest_bucket: i64 = (latest_time / aggr) as i64;

        if latest_bucket < oldest_bucket {
            return;
        }

        let w = self.tex_w as i64;

        for (price, runs) in
            hist.iter_time_filtered(oldest_time, latest_time + aggr, highest, lowest)
        {
            // Map price -> y bin (view-relative around base_price).
            let dy_steps: i64 = (price.units - base_price.units).div_euclid(step_units);
            let dy_bins: i64 = dy_steps.div_euclid(self.steps_per_y_bin);

            let y_i64 = half_h + dy_bins;
            if y_i64 < 0 || y_i64 >= self.tex_h as i64 {
                continue;
            }
            let y = y_i64 as usize;

            for run in runs.iter() {
                // Overlap run with [oldest_time, latest_time+aggr)
                let run_start = run.start_time.max(oldest_time);
                let run_end_excl = run.until_time.min(latest_time.saturating_add(aggr));
                if run_end_excl <= run_start {
                    continue;
                }

                let b0 = (run_start / aggr) as i64;
                // run_end_excl is exclusive; last bucket touched is (end_excl-1)/aggr
                let b1 = ((run_end_excl - 1) / aggr) as i64;

                let b0 = b0.max(oldest_bucket);
                let b1 = b1.min(latest_bucket);
                if b1 < b0 {
                    continue;
                }

                let q = run.qty().max(0.0);
                if !q.is_finite() || q <= 0.0 {
                    continue;
                }
                let q_u32 = (q * qty_scale).round().clamp(0.0, u32::MAX as f32) as u32;
                if q_u32 == 0 {
                    continue;
                }

                // Fill each bucket column this run covers.
                for bucket in b0..=b1 {
                    let x = (bucket.rem_euclid(w)) as usize;
                    let idx = y * (self.tex_w as usize) + x;

                    if run.is_bid {
                        self.bid[idx] = self.bid[idx].max(q_u32);
                        self.col_max_bid[x] = self.col_max_bid[x].max(q_u32);
                        let by = (y as u32) / Y_BLOCK_H;
                        let bidx = (by as usize) * (self.tex_w as usize) + x;
                        if bidx < self.block_max_bid.len() {
                            self.block_max_bid[bidx] = self.block_max_bid[bidx].max(q_u32);
                        }
                    } else {
                        self.ask[idx] = self.ask[idx].max(q_u32);
                        self.col_max_ask[x] = self.col_max_ask[x].max(q_u32);
                        let by = (y as u32) / Y_BLOCK_H;
                        let bidx = (by as usize) * (self.tex_w as usize) + x;
                        if bidx < self.block_max_ask.len() {
                            self.block_max_ask[bidx] = self.block_max_ask[bidx].max(q_u32);
                        }
                    }
                }
            }
        }

        self.last_bucket = Some(latest_bucket);

        // After rebuild, renderer must reupload full texture.
        self.mark_full_dirty();
    }

    pub fn ensure_layout(&mut self, aggr_time_ms: u64) {
        let aggr_time_ms = aggr_time_ms.max(1);

        let buckets_needed = self.horizon_buckets.max(1);
        let tex_w = next_pow2_u32(buckets_needed);

        let needs_realloc = self.aggr_time_ms != aggr_time_ms || self.tex_w != tex_w;
        if !needs_realloc {
            return;
        }

        self.aggr_time_ms = aggr_time_ms;
        self.tex_w = tex_w;
        self.last_bucket = None;

        let len = (self.tex_w as usize) * (self.tex_h as usize);
        self.bid.clear();
        self.ask.clear();
        self.bid.resize(len, 0);
        self.ask.resize(len, 0);

        self.col_max_bid.clear();
        self.col_max_ask.clear();
        self.col_max_bid.resize(self.tex_w as usize, 0);
        self.col_max_ask.resize(self.tex_w as usize, 0);

        let blocks = self.y_block_count() as usize;
        self.block_max_bid.clear();
        self.block_max_ask.clear();
        self.block_max_bid.resize((self.tex_w as usize) * blocks, 0);
        self.block_max_ask.resize((self.tex_w as usize) * blocks, 0);

        // Layout change => full upload required.
        self.mark_full_dirty();
    }

    fn scatter_side(
        &mut self,
        side: &std::collections::BTreeMap<Price, f32>,
        x: u32,
        anchor: Price,
        step: PriceStep,
        step_units: i64,
        qty_scale: f32,
        is_bid: bool,
    ) {
        let half_h = (self.tex_h as i64) / 2;
        let steps_per_y_bin = self.steps_per_y_bin.max(1);

        let grid = if is_bid { &mut self.bid } else { &mut self.ask };
        let col_max = if is_bid {
            &mut self.col_max_bid
        } else {
            &mut self.col_max_ask
        };

        let block_max = if is_bid {
            &mut self.block_max_bid
        } else {
            &mut self.block_max_ask
        };

        let w = self.tex_w as usize;
        let x_usize = x as usize;

        // Merge consecutive levels that round to the same side-step price (like HistoricalDepth).
        let mut current_price: Option<Price> = None;
        let mut current_qty: f32 = 0.0;

        let mut flush = |rounded_price: Price, qty_sum: f32| {
            let q = qty_sum.max(0.0);
            if q <= 0.0 || !q.is_finite() {
                return;
            }

            let dy_steps: i64 = (rounded_price.units - anchor.units).div_euclid(step_units);
            let dy_bins: i64 = dy_steps.div_euclid(steps_per_y_bin);

            let y_i64 = half_h + dy_bins;
            if y_i64 < 0 || y_i64 >= self.tex_h as i64 {
                return;
            }

            let q_u32 = (q * qty_scale).round().clamp(0.0, u32::MAX as f32) as u32;
            if q_u32 == 0 {
                return;
            }

            let y = y_i64 as u32;
            let idx = (y as usize) * w + x_usize;

            grid[idx] = grid[idx].max(q_u32);
            col_max[x_usize] = col_max[x_usize].max(q_u32);

            // update y-block max
            let by = y / Y_BLOCK_H;
            let bidx = (by as usize) * w + x_usize;
            if bidx < block_max.len() {
                block_max[bidx] = block_max[bidx].max(q_u32);
            }
        };

        for (price, qty) in side.iter() {
            let rounded = price.round_to_side_step(is_bid, step);

            if Some(rounded) == current_price {
                current_qty += *qty;
            } else {
                if let Some(p) = current_price {
                    flush(p, current_qty);
                }
                current_price = Some(rounded);
                current_qty = *qty;
            }
        }

        if let Some(p) = current_price {
            flush(p, current_qty);
        }
    }

    /// Update the ring for a new snapshot at `rounded_t_ms` (must already be bucket-rounded).
    ///
    /// - Uses max-reduction per (bucket, y) to match the `accumulate_max` behavior.
    pub fn ingest_snapshot(
        &mut self,
        depth: &Depth,
        rounded_t_ms: u64,
        step: PriceStep,
        qty_scale: f32,
        recenter_target: Price,
        steps_per_y_bin: i64,
    ) {
        let steps_per_y_bin = steps_per_y_bin.max(1);
        if self.steps_per_y_bin != steps_per_y_bin {
            self.steps_per_y_bin = steps_per_y_bin;
            self.clear_all();
            self.last_bucket = None;
            self.y_anchor = None;

            // Binning change => full upload required.
            self.mark_full_dirty();
        }

        if self.aggr_time_ms == 0 || self.tex_w == 0 || self.tex_h == 0 {
            return;
        }

        let step_units = step.units.max(1);
        let recenter_threshold_bins: i64 = (self.tex_h as i64) / 4;

        match self.y_anchor {
            None => self.y_anchor = Some(recenter_target),
            Some(anchor) => {
                let delta_steps = (recenter_target.units - anchor.units).div_euclid(step_units);
                let delta_bins = delta_steps.div_euclid(self.steps_per_y_bin.max(1));

                if delta_bins.unsigned_abs() as i64 > recenter_threshold_bins.max(1) {
                    self.y_anchor = Some(recenter_target);
                    self.clear_all();
                    self.last_bucket = None;

                    // Recentering => full upload required.
                    self.mark_full_dirty();
                }
            }
        }
        let Some(anchor) = self.y_anchor else {
            return;
        };

        let bucket: i64 = (rounded_t_ms / self.aggr_time_ms) as i64;
        let w = self.tex_w as i64;
        let x: u32 = (bucket.rem_euclid(w)) as u32;

        // Late/out-of-order within grace: clear+rewrite that older bucket.
        if let Some(prev) = self.last_bucket
            && bucket < prev
        {
            let grace_b = self.grace_buckets();
            let oldest_kept = prev - (self.tex_w as i64) + 1;

            if grace_b > 0 && (prev - bucket) <= grace_b && bucket >= oldest_kept {
                self.clear_column(x);
                self.scatter_side(&depth.bids, x, anchor, step, step_units, qty_scale, true);
                self.scatter_side(&depth.asks, x, anchor, step, step_units, qty_scale, false);
                return;
            } else {
                return;
            }
        }

        self.advance_and_fill_columns(bucket);

        self.clear_column(x);
        self.scatter_side(&depth.bids, x, anchor, step, step_units, qty_scale, true);
        self.scatter_side(&depth.asks, x, anchor, step, step_units, qty_scale, false);
    }

    pub fn extract_bid_column(&self, x: u32) -> Vec<u32> {
        self.extract_column_impl(x, true)
    }

    pub fn extract_ask_column(&self, x: u32) -> Vec<u32> {
        self.extract_column_impl(x, false)
    }

    fn copy_column(&mut self, from_x: u32, to_x: u32) {
        if from_x == to_x {
            return;
        }
        let w = self.tex_w as usize;
        let from = from_x as usize;
        let to = to_x as usize;

        for y in 0..(self.tex_h as usize) {
            let from_idx = y * w + from;
            let to_idx = y * w + to;
            self.bid[to_idx] = self.bid[from_idx];
            self.ask[to_idx] = self.ask[from_idx];
        }

        if from < self.col_max_bid.len() && to < self.col_max_bid.len() {
            self.col_max_bid[to] = self.col_max_bid[from];
        }
        if from < self.col_max_ask.len() && to < self.col_max_ask.len() {
            self.col_max_ask[to] = self.col_max_ask[from];
        }

        // copy y-block maxima
        let blocks = self.y_block_count();
        for by in 0..blocks {
            let fi = self.block_idx(from_x, by);
            let ti = self.block_idx(to_x, by);
            if fi < self.block_max_bid.len() && ti < self.block_max_bid.len() {
                self.block_max_bid[ti] = self.block_max_bid[fi];
                self.block_max_ask[ti] = self.block_max_ask[fi];
            }
        }

        self.mark_dirty(to_x);
    }

    fn clear_column(&mut self, x: u32) {
        let w = self.tex_w as usize;
        let x_usize = x as usize;

        for y in 0..(self.tex_h as usize) {
            let idx = y * w + x_usize;
            self.bid[idx] = 0;
            self.ask[idx] = 0;
        }

        if x_usize < self.col_max_bid.len() {
            self.col_max_bid[x_usize] = 0;
        }
        if x_usize < self.col_max_ask.len() {
            self.col_max_ask[x_usize] = 0;
        }

        // clear y-block maxima
        let blocks = self.y_block_count();
        for by in 0..blocks {
            let i = self.block_idx(x, by);
            if i < self.block_max_bid.len() {
                self.block_max_bid[i] = 0;
                self.block_max_ask[i] = 0;
            }
        }

        self.mark_dirty(x);
    }

    fn clear_all(&mut self) {
        self.bid.fill(0);
        self.ask.fill(0);
        self.col_max_bid.fill(0);
        self.col_max_ask.fill(0);

        self.block_max_bid.fill(0);
        self.block_max_ask.fill(0);

        // Whole texture changed => force full upload.
        self.mark_full_dirty();
    }

    fn extract_column_impl(&self, x: u32, is_bid: bool) -> Vec<u32> {
        let w = self.tex_w as usize;
        let h = self.tex_h as usize;
        let x = x as usize;

        let grid = if is_bid { &self.bid } else { &self.ask };

        let mut out = vec![0u32; h];
        if w == 0 || h == 0 || grid.len() != w * h || x >= w {
            return out;
        }

        for y in 0..h {
            out[y] = grid[y * w + x];
        }
        out
    }

    fn advance_and_fill_columns(&mut self, new_bucket: i64) {
        let Some(prev) = self.last_bucket else {
            self.clear_column((new_bucket.rem_euclid(self.tex_w as i64)) as u32);
            self.last_bucket = Some(new_bucket);
            return;
        };

        if new_bucket <= prev {
            return;
        }

        let jump = new_bucket - prev;
        let w = self.tex_w as i64;
        let max_steps = self.tex_w as i64;

        if jump > max_steps {
            self.clear_all();
            self.last_bucket = Some(new_bucket);
            return;
        }

        let grace_b = self.grace_buckets();

        if grace_b > 0 && jump <= grace_b {
            // Fill only the *missing* buckets (prev+1 .. new_bucket-1) by copying forward
            let mut from_x = (prev.rem_euclid(w)) as u32;

            let end_excl = new_bucket; // IMPORTANT: exclude new_bucket itself
            for b in (prev + 1)..end_excl {
                let to_x = (b.rem_euclid(w)) as u32;
                self.copy_column(from_x, to_x);
                from_x = to_x;
            }
        } else {
            // Clear only the *missing* buckets (prev+1 .. new_bucket-1)
            let mut b = prev + 1;
            while b < new_bucket {
                let x = (b.rem_euclid(w)) as u32;
                self.clear_column(x);
                b += 1;
            }
        }

        self.last_bucket = Some(new_bucket);
    }

    /// Force the next `build_scene_upload_plan()` to produce a full texture upload.
    /// Useful when GPU resources were recreated (e.g. window hidden/re-shown).
    #[inline]
    pub fn force_full_upload(&mut self) {
        self.mark_full_dirty();
    }

    /// Build a renderer upload plan from the ring's dirty flags.
    pub fn build_scene_upload_plan(&mut self) -> scene::HeatmapUploadPlan {
        if self.tex_w == 0 || self.tex_h == 0 {
            return scene::HeatmapUploadPlan::None;
        }

        if self.take_full_dirty() {
            return scene::HeatmapUploadPlan::Full(scene::HeatmapTextureCpuFull {
                width: self.tex_w,
                height: self.tex_h,
                bid: Arc::new(self.bid.clone()),
                ask: Arc::new(self.ask.clone()),
                generation: 0, // filled by Scene::apply_heatmap_upload_plan
            });
        }

        let xs = self.drain_dirty_columns();
        if xs.is_empty() {
            return scene::HeatmapUploadPlan::None;
        }

        let mut cols: Vec<scene::HeatmapColumnCpu> = Vec::with_capacity(xs.len());
        for x in xs {
            cols.push(scene::HeatmapColumnCpu {
                width: self.tex_w,
                height: self.tex_h,
                x,
                bid_col: Arc::new(self.extract_bid_column(x)),
                ask_col: Arc::new(self.extract_ask_column(x)),
            });
        }

        scene::HeatmapUploadPlan::Cols(cols)
    }

    /// Map an absolute bucket index to the ring texture x coordinate.
    /// Returns 0 if the ring is not laid out yet.
    #[inline]
    pub fn ring_x_for_bucket(&self, bucket: i64) -> u32 {
        if self.tex_w == 0 {
            return 0;
        }
        (bucket.rem_euclid(self.tex_w as i64)) as u32
    }

    /// Value for the shader uniform `heatmap_a[1]` (y_start_bin).
    ///
    /// This is the y-bin offset that aligns the texture's internal `y_anchor`
    /// with the current `base_price`.
    #[inline]
    pub fn heatmap_y_start_bin(&self, base_price: Price, step: PriceStep) -> f32 {
        let tex_h = self.tex_h.max(1);
        let half_h = (tex_h as i32) / 2;

        let Some(anchor) = self.y_anchor else {
            return -(half_h as f32);
        };

        let step_units = step.units.max(1);
        let steps_per_y_bin = self.steps_per_y_bin.max(1);

        // delta_bins = (base - anchor) in y-bins
        let delta_steps: i64 = (base_price.units - anchor.units).div_euclid(step_units);
        let delta_bins: i64 = delta_steps.div_euclid(steps_per_y_bin);

        -(half_h as f32) - (delta_bins as f32)
    }
}

#[inline]
fn next_pow2_u32(x: u32) -> u32 {
    x.next_power_of_two()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeatmapPalette {
    pub bid_rgb: [f32; 3],
    pub ask_rgb: [f32; 3],
    pub buy_rgb: [f32; 3],
    pub sell_rgb: [f32; 3],
    pub secondary_rgb: [f32; 3],
}

impl HeatmapPalette {
    pub fn from_theme(theme: &iced_core::Theme) -> Self {
        let palette = theme.extended_palette();

        let bid = palette.success.strong.color;
        let bid_linear = Self::srgb_to_linear([bid.r, bid.g, bid.b]);

        let ask = palette.danger.strong.color;
        let ask_linear = Self::srgb_to_linear([ask.r, ask.g, ask.b]);

        let buy = palette.success.base.color;
        let buy_linear = Self::srgb_to_linear([buy.r, buy.g, buy.b]);
        let sell = palette.danger.base.color;
        let sell_linear = Self::srgb_to_linear([sell.r, sell.g, sell.b]);

        let secondary = palette.secondary.base.color;
        let secondary_linear = Self::srgb_to_linear([secondary.r, secondary.g, secondary.b]);

        Self {
            bid_rgb: bid_linear,
            ask_rgb: ask_linear,
            buy_rgb: buy_linear,
            sell_rgb: sell_linear,
            secondary_rgb: secondary_linear,
        }
    }

    #[inline]
    fn srgb_to_linear_channel(u: f32) -> f32 {
        if u <= 0.04045 {
            u / 12.92
        } else {
            ((u + 0.055) / 1.055).powf(2.4)
        }
    }

    #[inline]
    fn srgb_to_linear(rgb: [f32; 3]) -> [f32; 3] {
        [
            Self::srgb_to_linear_channel(rgb[0]),
            Self::srgb_to_linear_channel(rgb[1]),
            Self::srgb_to_linear_channel(rgb[2]),
        ]
    }
}
