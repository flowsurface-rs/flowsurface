use crate::widget::chart::heatmap::scene::depth_grid::HeatmapPalette;
use crate::widget::chart::heatmap::scene::pipeline::circle::CircleInstance;
use crate::widget::chart::heatmap::scene::pipeline::rectangle::RectInstance;
use crate::widget::chart::heatmap::scene::pipeline::{DrawItem, DrawLayer, DrawOp};
use crate::widget::chart::heatmap::view::ViewWindow;

use data::aggr::time::TimeSeries;
use data::chart::heatmap::{Config, HeatmapDataPoint, ProfileKind};
use exchange::SizeUnit;
use exchange::adapter::MarketKind;
use exchange::unit::{Price, PriceStep, Qty};

#[derive(Debug, Clone)]
pub struct OverlayBuild {
    pub circles: Vec<CircleInstance>,
    pub rects: Vec<RectInstance>,

    // Ranges into `rects` for typed layering.
    pub rect_depth_profile: std::ops::Range<u32>,
    pub rect_volume: std::ops::Range<u32>,
    pub rect_volume_profile: std::ops::Range<u32>,
}

impl OverlayBuild {
    fn count(r: &std::ops::Range<u32>) -> u32 {
        r.end.saturating_sub(r.start)
    }

    pub fn draw_list(&self) -> Vec<DrawItem> {
        let mut out = Vec::new();

        // Background
        out.push(DrawItem::new(DrawLayer::HEATMAP, DrawOp::Heatmap));

        // Behind circles
        if Self::count(&self.rect_depth_profile) > 0 {
            out.push(DrawItem::new(
                DrawLayer::DEPTH_PROFILE,
                DrawOp::Rects {
                    start: self.rect_depth_profile.start,
                    count: Self::count(&self.rect_depth_profile),
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

        if Self::count(&self.rect_volume_profile) > 0 {
            out.push(DrawItem::new(
                DrawLayer::VOLUME_PROFILE,
                DrawOp::Rects {
                    start: self.rect_volume_profile.start,
                    count: Self::count(&self.rect_volume_profile),
                },
            ));
        }

        out
    }
}

#[derive(Default)]
pub struct InstanceBuilder {
    // Reusable buffers
    volume_acc: Vec<(Qty, Qty)>,
    volume_touched: Vec<usize>,
    depth_profile_bid_acc: Vec<Qty>,
    depth_profile_ask_acc: Vec<Qty>,
    volume_profile_bid_acc: Vec<Qty>,
    volume_profile_ask_acc: Vec<Qty>,

    // Scale denominators (for external getters)
    pub depth_profile_scale_max_qty: Option<Qty>,
    pub volume_strip_scale_max_qty: Option<Qty>,
    pub volume_profile_scale_max_qty: Option<Qty>,
}

impl InstanceBuilder {
    pub fn build_instances(
        &mut self,
        w: &ViewWindow,
        trades: &TimeSeries<HeatmapDataPoint>,
        latest_depth: impl IntoIterator<Item = (Price, Qty, bool)>,
        base_price: Price,
        step: PriceStep,
        latest_time: u64,
        scroll_ref_bucket: i64,
        palette: &HeatmapPalette,
        config: &Config,
        market_type: &MarketKind,
        profile_kind: Option<&ProfileKind>,
        show_volume_strip: bool,
    ) -> OverlayBuild {
        // Reset denoms each rebuild to avoid stale overlay labels
        self.depth_profile_scale_max_qty = None;
        self.volume_strip_scale_max_qty = None;
        self.volume_profile_scale_max_qty = None;

        let circles = self.build_circles(
            w,
            trades,
            base_price,
            step,
            scroll_ref_bucket,
            palette,
            config,
            market_type,
        );

        let mut rects: Vec<RectInstance> = Vec::new();

        let prof_start = rects.len() as u32;
        self.build_depth_profile_rects(w, latest_depth, base_price, step, palette, &mut rects);
        let prof_end = rects.len() as u32;

        let vol_start = rects.len() as u32;
        if show_volume_strip {
            self.build_volume_strip_rects(w, trades, scroll_ref_bucket, palette, &mut rects);
        }
        let vol_end = rects.len() as u32;

        let tp_start = rects.len() as u32;
        if let Some(kind) = profile_kind {
            self.build_volume_profile_rects(
                w,
                trades,
                base_price,
                step,
                latest_time,
                kind,
                palette,
                &mut rects,
            );
        }
        let tp_end = rects.len() as u32;

        OverlayBuild {
            circles,
            rects,
            rect_depth_profile: prof_start..prof_end,
            rect_volume: vol_start..vol_end,
            rect_volume_profile: tp_start..tp_end,
        }
    }

    fn build_volume_profile_rects(
        &mut self,
        w: &ViewWindow,
        trades: &TimeSeries<HeatmapDataPoint>,
        base_price: Price,
        step: PriceStep,
        latest_time: u64,
        profile_kind: &ProfileKind,
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

        self.volume_profile_bid_acc.resize(len, Qty::ZERO);
        self.volume_profile_ask_acc.resize(len, Qty::ZERO);
        self.volume_profile_bid_acc[..].fill(Qty::ZERO);
        self.volume_profile_ask_acc[..].fill(Qty::ZERO);

        let mut max_total = Qty::ZERO;

        let latest_profile_time = latest_time.min(w.latest_vis);

        let (earliest_profile_time, latest_profile_time) = match profile_kind {
            ProfileKind::VisibleRange => (w.earliest, w.latest_vis),
            ProfileKind::FixedWindow(datapoints) => {
                let aggr = w.aggr_time.max(1);
                let window_ms = (*datapoints as u64).saturating_mul(aggr);
                (
                    latest_profile_time.saturating_sub(window_ms),
                    latest_profile_time,
                )
            }
        };

        if latest_profile_time < earliest_profile_time {
            return;
        }

        for (_time, dp) in trades
            .datapoints
            .range(earliest_profile_time..=latest_profile_time)
        {
            for t in dp.grouped_trades.iter() {
                let rel_y_bin = w.y_bin_for_price(t.price, base_price, step);
                let idx = rel_y_bin - min_rel_y_bin;
                if idx < 0 || idx >= len as i64 {
                    continue;
                }

                let i = idx as usize;
                if t.is_sell {
                    self.volume_profile_ask_acc[i] += t.qty;
                } else {
                    self.volume_profile_bid_acc[i] += t.qty;
                }

                let total = self.volume_profile_bid_acc[i] + self.volume_profile_ask_acc[i];
                max_total = max_total.max(total);
            }
        }

        if max_total.is_zero() {
            return;
        }

        let max_total_f32 = max_total.to_f32_lossy();
        self.volume_profile_scale_max_qty = Some(max_total);

        for i in 0..len {
            let rel_y_bin = min_rel_y_bin + i as i64;
            let y_world = w.y_center_for_bin(rel_y_bin);

            let buy_qty = self.volume_profile_bid_acc[i];
            let sell_qty = self.volume_profile_ask_acc[i];
            let total = buy_qty + sell_qty;

            if total.is_zero() {
                continue;
            }

            let buy_qty_f32 = buy_qty.to_f32_lossy();
            let sell_qty_f32 = sell_qty.to_f32_lossy();
            let total_f32 = total.to_f32_lossy();

            let total_w = (total_f32 / max_total_f32) * w.volume_profile_max_width;
            if total_w <= 0.0 {
                continue;
            }
            let buy_w = total_w * (buy_qty_f32 / total_f32);
            let sell_w = total_w * (sell_qty_f32 / total_f32);

            let mut x = w.left_edge_world;

            if !sell_qty.is_zero() && sell_w > 0.0 {
                rects.push(RectInstance::volume_profile_split_bar(
                    y_world,
                    sell_w,
                    x,
                    w,
                    palette.sell_rgb,
                ));
                x += sell_w;
            }

            if !buy_qty.is_zero() && buy_w > 0.0 {
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
        config: &Config,
        market_type: &MarketKind,
    ) -> Vec<CircleInstance> {
        let mut visible = vec![];
        let mut max_qty = Qty::ZERO;
        let size_in_quote_ccy = exchange::unit::qty::volume_size_unit() == SizeUnit::Quote;
        let trade_size_filter = config.trade_size_filter.max(0.0);
        let fallback_radius_px = (0.5 * w.row_h * w.cam_scale).max(CircleInstance::R_MIN_PX);

        for (bucket_time, dp) in trades.datapoints.range(w.earliest..=w.latest_vis) {
            let bucket = (*bucket_time / w.aggr_time) as i64;

            for trade in dp.grouped_trades.iter() {
                if trade.price < w.lowest || trade.price > w.highest {
                    continue;
                }

                max_qty = max_qty.max(trade.qty);

                let trade_size =
                    market_type.qty_in_quote_value(trade.qty, trade.price, size_in_quote_ccy);
                if trade_size <= trade_size_filter {
                    continue;
                }

                visible.push((bucket, trade));
            }
        }

        if max_qty.is_zero() || visible.is_empty() {
            return vec![];
        }

        let mut out = Vec::with_capacity(visible.len());
        for (bucket, trade) in visible {
            out.push(CircleInstance::from_trade(
                trade,
                bucket,
                ref_bucket,
                base_price,
                step,
                w,
                palette,
                max_qty,
                config.trade_size_scale,
                fallback_radius_px,
            ));
        }

        out
    }

    fn build_depth_profile_rects(
        &mut self,
        w: &ViewWindow,
        latest_depth: impl IntoIterator<Item = (Price, Qty, bool)>,
        base_price: Price,
        step: PriceStep,
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

        self.depth_profile_bid_acc.resize(len, Qty::ZERO);
        self.depth_profile_ask_acc.resize(len, Qty::ZERO);
        self.depth_profile_bid_acc[..].fill(Qty::ZERO);
        self.depth_profile_ask_acc[..].fill(Qty::ZERO);

        let mut max_qty = Qty::ZERO;

        for (price, qty, is_bid) in latest_depth {
            if price < w.lowest || price > w.highest {
                continue;
            }

            let rel_y_bin = w.y_bin_for_price(price, base_price, step);
            let idx = rel_y_bin - min_rel_y_bin;
            if idx < 0 || idx >= len as i64 {
                continue;
            }

            let i = idx as usize;
            let v = if is_bid {
                &mut self.depth_profile_bid_acc[i]
            } else {
                &mut self.depth_profile_ask_acc[i]
            };

            *v += qty;
            max_qty = max_qty.max(*v);
        }

        if max_qty.is_zero() {
            return;
        }

        self.depth_profile_scale_max_qty = Some(max_qty);

        for i in 0..len {
            let rel_y_bin = min_rel_y_bin + i as i64;
            let y = w.y_center_for_bin(rel_y_bin);

            for (is_bid, qty) in [
                (true, self.depth_profile_bid_acc[i]),
                (false, self.depth_profile_ask_acc[i]),
            ] {
                if !qty.is_zero() {
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
        self.volume_acc.resize(bins_len, (Qty::ZERO, Qty::ZERO));
        self.volume_acc
            .iter_mut()
            .for_each(|e| *e = (Qty::ZERO, Qty::ZERO));
        self.volume_touched.clear();

        for (time, dp) in trades.datapoints.range(w.earliest..=w.latest_vis) {
            let bucket = (*time / w.aggr_time) as i64;
            let x_bin = bucket.div_euclid(cols_per_x_bin);
            let idx = (x_bin - min_x_bin) as usize;

            if idx >= bins_len {
                continue;
            }

            let (buy, sell) = dp.buy_sell;
            if buy.is_zero() && sell.is_zero() {
                continue;
            }

            let e = &mut self.volume_acc[idx];
            let was_zero = e.0.is_zero() && e.1.is_zero();
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
        let mut max_total = Qty::ZERO;
        for &idx in &self.volume_touched {
            let (buy, sell) = self.volume_acc[idx];
            max_total = max_total.max(buy + sell);
        }
        if max_total.is_zero() {
            return;
        }

        self.volume_strip_scale_max_qty = Some(max_total);

        // Build rectangle instances
        for &idx in &self.volume_touched {
            let (buy, sell) = self.volume_acc[idx];
            let total_qty = buy + sell;
            if total_qty.is_zero() {
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
                total_qty, max_total, buy, sell, x0_bin, x1_bin, w, palette,
            );
            let total_h = total_bar.size[1];
            let base_rgb = [total_bar.color[0], total_bar.color[1], total_bar.color[2]];
            rects.push(total_bar);

            // Delta overlay (if not tied)
            let diff = buy.abs_diff(sell);
            if !diff.is_zero() {
                rects.push(RectInstance::volume_delta_bar(
                    diff, total_h, max_total, base_rgb, x0_bin, x1_bin, w,
                ));
            }
        }
    }
}
