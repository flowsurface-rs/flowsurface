use crate::widget::chart::heatmap::depth_grid::HeatmapPalette;
use crate::widget::chart::heatmap::scene::pipeline::circle::CircleInstance;
use crate::widget::chart::heatmap::scene::pipeline::rectangle::RectInstance;
use crate::widget::chart::heatmap::view::ViewWindow;

use data::aggr::time::TimeSeries;
use data::chart::heatmap::{HeatmapDataPoint, HistoricalDepth};
use exchange::util::{Price, PriceStep};

pub struct InstanceBuilder {
    // Reusable buffers
    volume_acc: Vec<(f32, f32)>,
    volume_touched: Vec<usize>,
    profile_bid_acc: Vec<f32>,
    profile_ask_acc: Vec<f32>,

    // Scale denominators (for external getters)
    pub profile_scale_max_qty: Option<f32>,
    pub volume_strip_scale_max_qty: Option<f32>,
}

impl InstanceBuilder {
    pub fn new() -> Self {
        Self {
            volume_acc: Vec::new(),
            volume_touched: Vec::new(),
            profile_bid_acc: Vec::new(),
            profile_ask_acc: Vec::new(),
            profile_scale_max_qty: None,
            volume_strip_scale_max_qty: None,
        }
    }

    pub fn clear(&mut self) {
        self.volume_acc.clear();
        self.volume_touched.clear();
        self.profile_bid_acc.clear();
        self.profile_ask_acc.clear();

        self.profile_scale_max_qty = None;
        self.volume_strip_scale_max_qty = None;
    }

    pub fn build_instances(
        &mut self,
        w: &ViewWindow,
        trades: &TimeSeries<HeatmapDataPoint>,
        heatmap: &HistoricalDepth,
        base_price: Price,
        step: PriceStep,
        latest_time: u64,
        scroll_ref_bucket: i64,
        palette: &HeatmapPalette,
    ) -> (Vec<CircleInstance>, Vec<RectInstance>) {
        let circles = self.build_circles(w, trades, base_price, step, scroll_ref_bucket, palette);
        let mut rects = Vec::new();

        self.build_profile_rects(
            w,
            heatmap,
            base_price,
            step,
            latest_time,
            palette,
            &mut rects,
        );
        self.build_volume_strip_rects(w, trades, scroll_ref_bucket, palette, &mut rects);

        (circles, rects)
    }

    fn build_circles(
        &self,
        w: &ViewWindow,
        trades: &TimeSeries<HeatmapDataPoint>,
        base_price: Price,
        step: PriceStep,
        ref_bucket: i64,
        palette: &HeatmapPalette,
    ) -> Vec<CircleInstance> {
        let max_qty = self.max_trade_qty(w, trades);
        if max_qty <= 0.0 {
            return vec![];
        }

        let aggr = w.aggr_time.max(1);
        let mut out = Vec::new();

        for (bucket_time, dp) in trades.datapoints.range(w.earliest..=w.latest_vis) {
            let bucket = (*bucket_time / aggr) as i64;

            for tr in dp.grouped_trades.iter() {
                out.push(CircleInstance::from_trade(
                    tr, bucket, ref_bucket, base_price, step, w, palette, max_qty,
                ));
            }
        }

        out
    }

    fn build_profile_rects(
        &mut self,
        w: &ViewWindow,
        heatmap: &HistoricalDepth,
        base_price: Price,
        step: PriceStep,
        latest_time: u64,
        palette: &HeatmapPalette,
        rects: &mut Vec<RectInstance>,
    ) {
        if w.profile_max_w_world <= 0.0 {
            return;
        }

        let step_units = step.units.max(1);
        let y_div = w.steps_per_y_bin.max(1);
        let base_steps = base_price.units / step_units;
        let base_abs_y_bin = base_steps.div_euclid(y_div);

        let lowest_abs_steps = w.lowest.units / step_units;
        let highest_abs_steps = w.highest.units / step_units;
        let min_abs_y_bin = lowest_abs_steps.div_euclid(y_div);
        let max_abs_y_bin = highest_abs_steps.div_euclid(y_div);

        if max_abs_y_bin < min_abs_y_bin {
            return;
        }

        let len = (max_abs_y_bin - min_abs_y_bin + 1) as usize;

        // Accumulate quantities into bins
        self.profile_bid_acc.resize(len, 0.0);
        self.profile_ask_acc.resize(len, 0.0);
        self.profile_bid_acc[..].fill(0.0);
        self.profile_ask_acc[..].fill(0.0);

        let mut max_qty = 0.0f32;

        for (price, run) in heatmap.latest_order_runs(w.highest, w.lowest, latest_time) {
            if *price < w.lowest || *price > w.highest {
                continue;
            }

            let abs_steps = price.units / step_units;
            let abs_y_bin = abs_steps.div_euclid(y_div);
            let idx = (abs_y_bin - min_abs_y_bin) as usize;

            let v = if run.is_bid {
                &mut self.profile_bid_acc[idx]
            } else {
                &mut self.profile_ask_acc[idx]
            };

            *v += run.qty();
            max_qty = max_qty.max(*v);
        }

        if max_qty <= 0.0 {
            return;
        }

        self.profile_scale_max_qty = Some(max_qty);

        // Build rectangle instances
        for i in 0..len {
            let abs_y_bin = min_abs_y_bin + i as i64;
            let rel_y_bin = abs_y_bin - base_abs_y_bin;
            let y = RectInstance::y_center_for_bin(rel_y_bin, w);

            for (is_bid, qty) in [
                (true, self.profile_bid_acc[i]),
                (false, self.profile_ask_acc[i]),
            ] {
                if qty > 0.0 {
                    rects.push(RectInstance::profile_bar(
                        y, qty, max_qty, is_bid, w, palette,
                    ));
                }
            }
        }
    }

    fn build_volume_strip_rects(
        &mut self,
        w: &ViewWindow,
        trades: &TimeSeries<HeatmapDataPoint>,
        ref_bucket: i64,
        palette: &HeatmapPalette,
        rects: &mut Vec<RectInstance>,
    ) {
        const BUCKET_GAP_FRAC: f32 = 0.10;
        const MIN_BAR_W_PX: f32 = 2.0;
        const MAX_COLS_PER_X_BIN: i64 = 4096;
        const EPS: f32 = 1e-12;

        if w.strip_h_world <= 0.0 {
            return;
        }

        // Compute X binning
        let px_per_col = w.sx;
        let px_per_drawn_col = px_per_col * (1.0 - BUCKET_GAP_FRAC);
        let mut cols_per_x_bin = 1i64;
        if px_per_drawn_col.is_finite() && px_per_drawn_col > 0.0 {
            cols_per_x_bin = (MIN_BAR_W_PX / px_per_drawn_col).ceil() as i64;
            cols_per_x_bin = cols_per_x_bin.clamp(1, MAX_COLS_PER_X_BIN);
        }

        let start_bucket = (w.earliest / w.aggr_time) as i64;
        let latest_bucket = (w.latest_vis / w.aggr_time) as i64;

        let min_x_bin = start_bucket.div_euclid(cols_per_x_bin);
        let max_x_bin = latest_bucket.div_euclid(cols_per_x_bin);
        if max_x_bin < min_x_bin {
            return;
        }

        // Accumulate buy/sell volumes into bins
        let bins_len = (max_x_bin - min_x_bin + 1) as usize;
        self.volume_acc.resize(bins_len, (0.0, 0.0));
        self.volume_acc.iter_mut().for_each(|e| *e = (0.0, 0.0));
        self.volume_touched.clear();

        for (time, dp) in trades.datapoints.range(w.earliest..=w.latest_vis) {
            let bucket = (*time / w.aggr_time) as i64;
            let x_bin = bucket.div_euclid(cols_per_x_bin);
            let idx = (x_bin - min_x_bin) as usize;

            if idx >= bins_len {
                continue;
            }

            let (buy, sell) = dp.buy_sell;
            if buy == 0.0 && sell == 0.0 {
                continue;
            }

            let e = &mut self.volume_acc[idx];
            let was_zero = e.0 == 0.0 && e.1 == 0.0;
            e.0 += buy;
            e.1 += sell;
            if was_zero {
                self.volume_touched.push(idx);
            }
        }

        if self.volume_touched.is_empty() {
            return;
        }

        self.volume_touched.sort_unstable();
        self.volume_touched.dedup();

        // Find max total volume
        let mut max_total = 0.0f32;
        for &idx in &self.volume_touched {
            let (buy, sell) = self.volume_acc[idx];
            max_total = max_total.max(buy + sell);
        }
        if max_total <= 0.0 {
            return;
        }

        self.volume_strip_scale_max_qty = Some(max_total);

        // Build rectangle instances
        for &idx in &self.volume_touched {
            let (buy, sell) = self.volume_acc[idx];
            let total = buy + sell;
            if total <= 0.0 {
                continue;
            }

            let x_bin = min_x_bin + idx as i64;
            let start_bucket = x_bin * cols_per_x_bin;
            let end_bucket_excl = (start_bucket + cols_per_x_bin).min(latest_bucket + 1);
            if end_bucket_excl <= start_bucket {
                continue;
            }

            let x0_bin = (start_bucket - ref_bucket).clamp(i32::MIN as i64, i32::MAX as i64) as i32;
            let x1_bin =
                (end_bucket_excl - ref_bucket).clamp(i32::MIN as i64, i32::MAX as i64) as i32;

            // Total volume bar
            let total_bar = RectInstance::volume_total_bar(
                total, max_total, buy, sell, x0_bin, x1_bin, w, palette,
            );
            let total_h = total_bar.size[1];
            let base_rgb = [total_bar.color[0], total_bar.color[1], total_bar.color[2]];
            rects.push(total_bar);

            // Delta overlay (if not tied)
            let diff = (buy - sell).abs();
            if diff > EPS {
                rects.push(RectInstance::volume_delta_bar(
                    diff, total_h, max_total, base_rgb, x0_bin, x1_bin, w,
                ));
            }
        }
    }

    fn max_trade_qty(&self, w: &ViewWindow, trades: &TimeSeries<HeatmapDataPoint>) -> f32 {
        let lowest = w.lowest;
        let highest = w.highest;

        let earliest = w.earliest;
        let latest_vis = w.latest_vis;

        trades.max_trade_qty_in_range(earliest, latest_vis, highest, lowest)
    }
}
