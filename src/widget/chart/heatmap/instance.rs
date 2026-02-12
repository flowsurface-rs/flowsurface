use crate::widget::chart::heatmap::depth_grid::HeatmapPalette;
use crate::widget::chart::heatmap::scene::pipeline::circle::CircleInstance;
use crate::widget::chart::heatmap::scene::pipeline::rectangle::{MIN_BAR_PX, RectInstance};
use crate::widget::chart::heatmap::scene::pipeline::{DrawItem, DrawLayer, DrawOp};
use crate::widget::chart::heatmap::view::ViewWindow;

use data::aggr::time::TimeSeries;
use data::chart::heatmap::{HeatmapDataPoint, HistoricalDepth};
use exchange::util::{Price, PriceStep};

#[derive(Debug, Clone)]
pub struct OverlayBuild {
    pub circles: Vec<CircleInstance>,
    pub rects: Vec<RectInstance>,

    // Ranges into `rects` for typed layering.
    pub rect_profile_latest: std::ops::Range<u32>,
    pub rect_volume: std::ops::Range<u32>,
    pub rect_trade_profile: std::ops::Range<u32>,
}

impl OverlayBuild {
    #[inline]
    fn count(r: &std::ops::Range<u32>) -> u32 {
        r.end.saturating_sub(r.start)
    }

    pub fn draw_list(&self) -> Vec<DrawItem> {
        let mut out = Vec::new();

        // Background
        out.push(DrawItem::new(DrawLayer::HEATMAP, DrawOp::Heatmap));

        // Behind circles
        if Self::count(&self.rect_profile_latest) > 0 {
            out.push(DrawItem::new(
                DrawLayer::PROFILE_LATEST,
                DrawOp::Rects {
                    start: self.rect_profile_latest.start,
                    count: Self::count(&self.rect_profile_latest),
                },
            ));
        }

        // Circles
        if !self.circles.is_empty() {
            out.push(DrawItem::new(
                DrawLayer::CIRCLES,
                DrawOp::Circles {
                    start: 0,
                    count: self.circles.len() as u32,
                },
            ));
        }

        // Foreground overlays
        if Self::count(&self.rect_volume) > 0 {
            out.push(DrawItem::new(
                DrawLayer::VOLUME,
                DrawOp::Rects {
                    start: self.rect_volume.start,
                    count: Self::count(&self.rect_volume),
                },
            ));
        }

        if Self::count(&self.rect_trade_profile) > 0 {
            out.push(DrawItem::new(
                DrawLayer::TRADE_PROFILE,
                DrawOp::Rects {
                    start: self.rect_trade_profile.start,
                    count: Self::count(&self.rect_trade_profile),
                },
            ));
        }

        out
    }
}

pub struct InstanceBuilder {
    // Reusable buffers
    volume_acc: Vec<(f32, f32)>,
    volume_touched: Vec<usize>,
    profile_bid_acc: Vec<f32>,
    profile_ask_acc: Vec<f32>,
    trade_profile_bid_acc: Vec<f32>,
    trade_profile_ask_acc: Vec<f32>,

    // Scale denominators (for external getters)
    pub profile_scale_max_qty: Option<f32>,
    pub volume_strip_scale_max_qty: Option<f32>,
    pub trade_profile_scale_max_qty: Option<f32>,
}

impl InstanceBuilder {
    pub fn new() -> Self {
        Self {
            volume_acc: Vec::new(),
            volume_touched: Vec::new(),
            profile_bid_acc: Vec::new(),
            profile_ask_acc: Vec::new(),
            trade_profile_bid_acc: Vec::new(),
            trade_profile_ask_acc: Vec::new(),
            profile_scale_max_qty: None,
            volume_strip_scale_max_qty: None,
            trade_profile_scale_max_qty: None,
        }
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
    ) -> OverlayBuild {
        // Reset denoms each rebuild to avoid stale overlay labels
        self.profile_scale_max_qty = None;
        self.volume_strip_scale_max_qty = None;
        self.trade_profile_scale_max_qty = None;

        let circles = self.build_circles(w, trades, base_price, step, scroll_ref_bucket, palette);

        let mut rects: Vec<RectInstance> = Vec::new();

        let prof_start = rects.len() as u32;
        self.build_depth_profile_rects(
            w,
            heatmap,
            base_price,
            step,
            latest_time,
            palette,
            &mut rects,
        );
        let prof_end = rects.len() as u32;

        let vol_start = rects.len() as u32;
        self.build_volume_strip_rects(w, trades, scroll_ref_bucket, palette, &mut rects);
        let vol_end = rects.len() as u32;

        let tp_start = rects.len() as u32;
        self.build_volume_profile_rects(w, trades, base_price, step, palette, &mut rects);
        let tp_end = rects.len() as u32;

        OverlayBuild {
            circles,
            rects,
            rect_profile_latest: prof_start..prof_end,
            rect_volume: vol_start..vol_end,
            rect_trade_profile: tp_start..tp_end,
        }
    }

    fn build_volume_profile_rects(
        &mut self,
        w: &ViewWindow,
        trades: &TimeSeries<HeatmapDataPoint>,
        base_price: Price,
        step: PriceStep,
        palette: &HeatmapPalette,
        rects: &mut Vec<RectInstance>,
    ) {
        if w.volume_profile_max_width <= 0.0 {
            return;
        }

        let min_rel_y_bin = w.y_bin_for_price(w.lowest, base_price, step);
        let max_rel_y_bin = w.y_bin_for_price(w.highest, base_price, step);
        if max_rel_y_bin < min_rel_y_bin {
            return;
        }

        let len = (max_rel_y_bin - min_rel_y_bin + 1) as usize;

        self.trade_profile_bid_acc.resize(len, 0.0);
        self.trade_profile_ask_acc.resize(len, 0.0);
        self.trade_profile_bid_acc[..].fill(0.0);
        self.trade_profile_ask_acc[..].fill(0.0);

        let mut max_total = 0.0f32;

        for (_time, dp) in trades.datapoints.range(w.earliest..=w.latest_vis) {
            for t in dp.grouped_trades.iter() {
                let rel_y_bin = w.y_bin_for_price(t.price, base_price, step);
                let idx = rel_y_bin - min_rel_y_bin;
                if idx < 0 || idx >= len as i64 {
                    continue;
                }

                let i = idx as usize;
                if t.is_sell {
                    self.trade_profile_ask_acc[i] += t.qty;
                } else {
                    self.trade_profile_bid_acc[i] += t.qty;
                }

                let total = self.trade_profile_bid_acc[i] + self.trade_profile_ask_acc[i];
                max_total = max_total.max(total);
            }
        }

        if max_total <= 0.0 {
            return;
        }

        self.trade_profile_scale_max_qty = Some(max_total);

        let min_w_world = MIN_BAR_PX / w.cam_scale;

        for i in 0..len {
            let rel_y_bin = min_rel_y_bin + i as i64;
            let y_world = w.y_center_for_bin(rel_y_bin);

            let buy_qty = self.trade_profile_bid_acc[i];
            let sell_qty = self.trade_profile_ask_acc[i];
            let total = buy_qty + sell_qty;

            if total <= 0.0 {
                continue;
            }

            let total_w = ((total / max_total) * w.volume_profile_max_width).max(min_w_world);
            let buy_w = total_w * (buy_qty / total);
            let sell_w = total_w * (sell_qty / total);

            let mut x = w.left_edge_world;

            if sell_qty > 0.0 && sell_w > 0.0 {
                rects.push(RectInstance::volume_profile_split_bar(
                    y_world,
                    sell_w,
                    x,
                    w,
                    palette.sell_rgb,
                ));
                x += sell_w;
            }

            if buy_qty > 0.0 && buy_w > 0.0 {
                rects.push(RectInstance::volume_profile_split_bar(
                    y_world,
                    buy_w,
                    x,
                    w,
                    palette.buy_rgb,
                ));
            }
        }
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
        let mut visible = vec![];
        let mut max_qty = 0.0f32;

        for (bucket_time, dp) in trades.datapoints.range(w.earliest..=w.latest_vis) {
            let bucket = (*bucket_time / w.aggr_time) as i64;

            for trade in dp.grouped_trades.iter() {
                if trade.price < w.lowest || trade.price > w.highest {
                    continue;
                }

                max_qty = max_qty.max(trade.qty);
                visible.push((bucket, trade));
            }
        }

        if max_qty <= 0.0 || visible.is_empty() {
            return vec![];
        }

        let mut out = Vec::with_capacity(visible.len());
        for (bucket, trade) in visible {
            out.push(CircleInstance::from_trade(
                trade, bucket, ref_bucket, base_price, step, w, palette, max_qty,
            ));
        }

        out
    }

    fn build_depth_profile_rects(
        &mut self,
        w: &ViewWindow,
        heatmap: &HistoricalDepth,
        base_price: Price,
        step: PriceStep,
        latest_time: u64,
        palette: &HeatmapPalette,
        rects: &mut Vec<RectInstance>,
    ) {
        if w.depth_profile_max_width <= 0.0 {
            return;
        }

        let min_rel_y_bin = w.y_bin_for_price(w.lowest, base_price, step);
        let max_rel_y_bin = w.y_bin_for_price(w.highest, base_price, step);
        if max_rel_y_bin < min_rel_y_bin {
            return;
        }

        let len = (max_rel_y_bin - min_rel_y_bin + 1) as usize;

        self.profile_bid_acc.resize(len, 0.0);
        self.profile_ask_acc.resize(len, 0.0);
        self.profile_bid_acc[..].fill(0.0);
        self.profile_ask_acc[..].fill(0.0);

        let mut max_qty = 0.0f32;

        for (price, run) in heatmap.latest_order_runs(w.highest, w.lowest, latest_time) {
            if *price < w.lowest || *price > w.highest {
                continue;
            }

            let rel_y_bin = w.y_bin_for_price(*price, base_price, step);
            let idx = rel_y_bin - min_rel_y_bin;
            if idx < 0 || idx >= len as i64 {
                continue;
            }

            let i = idx as usize;
            let v = if run.is_bid {
                &mut self.profile_bid_acc[i]
            } else {
                &mut self.profile_ask_acc[i]
            };

            *v += run.qty();
            max_qty = max_qty.max(*v);
        }

        if max_qty <= 0.0 {
            return;
        }

        self.profile_scale_max_qty = Some(max_qty);

        for i in 0..len {
            let rel_y_bin = min_rel_y_bin + i as i64;
            let y = w.y_center_for_bin(rel_y_bin);

            for (is_bid, qty) in [
                (true, self.profile_bid_acc[i]),
                (false, self.profile_ask_acc[i]),
            ] {
                if qty > 0.0 {
                    rects.push(RectInstance::depth_profile_bar(
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

        if w.volume_area_max_height <= 0.0 {
            return;
        }

        // Compute X binning
        let px_per_col = w.cam_scale;
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
}
