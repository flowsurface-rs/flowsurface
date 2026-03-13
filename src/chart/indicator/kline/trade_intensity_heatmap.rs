// GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97
//! Rolling log-quantile percentile heatmap for trade intensity.
//!
//! Trade intensity has extreme power-law tails (BPR25: skewness=322, max/median = 700,000×).
//! This indicator computes a percentile rank of log10(intensity) within a rolling lookback
//! window and maps it to a thermal colour gradient (blue → cyan → green → amber → red).
//!
//! **Algorithm (no look-ahead bias)**:
//! ```text
//! log_val  = log10(intensity).max(0.0)
//! K_actual = max(round(cbrt(window.len())), 5)  ← fully adaptive, no cap
//! rank     = count(window ≤ log_val) / window.len()  ← binary search on sorted window
//! bin      = ceil(rank × K_actual).clamp(1, K_actual)
//! t        = (bin − 1) / (K_actual − 1)  ← normalise to [0, 1] for thermal colour
//! bar_height = bin  ← bounded 1..K, immune to outlier spikes
//! push log_val to ring-buffer; pop_front if len > lookback
//! ```
//!
//! The rank is computed BEFORE pushing the current bar — the current bar ranks
//! itself against previous bars only, giving zero look-ahead bias.
//!
//! **Incremental updates**: rolling window state (ring buffer + sorted freq map) is kept
//! on the struct. `on_insert_trades` processes only newly completed bars in O(1), avoiding
//! the O(N) full rebuild that caused UI freezes with large bar counts.

use crate::chart::{
    Caches, Message, ViewState,
    indicator::{
        indicator_row_slice_with_legend,
        kline::KlineIndicatorImpl,
        plot::{
            Plot, PlotTooltip, Series, YScale,
            bar::{BarClass, BarPlot, Baseline},
        },
    },
};
use iced::widget::canvas;

use data::chart::{PlotData, kline::KlineDataPoint, kline::adaptive_k};
use exchange::{Kline, Trade};

use iced::{Color, Point, Size};
use iced::widget::center;
use std::collections::VecDeque;
use std::ops::RangeInclusive;

// GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97

/// Per-bar data point for the heatmap indicator.
#[derive(Clone, Copy)]
struct HeatmapPoint {
    /// Raw trade intensity (t/s).
    intensity: f32,
    /// Bin index in 1..=k_actual.
    bin: u8,
    /// Adaptive K at the time this bar was processed.
    k_actual: u8,
    /// Candle direction (close >= open). Used for bar colouring.
    bullish: bool,
}

impl HeatmapPoint {
    /// Normalise bin to [0, 1] for the thermal colour gradient.
    fn t(self) -> f32 {
        if self.k_actual <= 1 {
            return 0.0;
        }
        (self.bin - 1) as f32 / (self.k_actual - 1) as f32
    }
}

/// 300° HSV hue sweep: blue → cyan → green → yellow → orange → red → magenta.
///
/// At K=19 this gives ≈16.7° of hue per bin — about 8× the ~2° just-noticeable
/// difference threshold — so every adjacent pair is clearly distinct across the full
/// 1–19 range, including the hot end (bins 13–19) which sweeps orange→red→crimson→magenta.
///
/// Turbo was tried first but its orange-to-dark-red hot zone compressed bins 13–19
/// into only ~30° of hue change (~5°/bin ≈ 2× JND), causing the "stops diversifying
/// at K≈13" problem. The 300° HSV sweep avoids this by continuing through red into
/// the highly-saturated pink/magenta region.
///
/// Key hue anchors at K=19 (S=0.95, V=0.92 throughout — vivid on dark backgrounds):
///   K1=blue(240°) K5=cyan(173°) K8=green(123°) K10=lime(90°)
///   K12=yellow(57°) K14=orange(23°) K16=red(350°) K19=magenta(300°)
fn thermal_color(t: f32) -> Color {
    // Hue sweeps backward from 240° (blue) by 300° total → ends at 300° (magenta).
    // rem_euclid handles the wrap from negative values (e.g. -60° → 300°).
    let hue_deg = (240.0 - t.clamp(0.0, 1.0) * 300.0).rem_euclid(360.0);
    let h = hue_deg / 60.0; // sector index in [0, 6)
    let s = 0.95_f32;
    let v = 0.92_f32;

    let i = h as u32;
    let f = h - i as f32;
    let p = v * (1.0 - s);           // low component (≈0.046 — near-zero for vivid colours)
    let q = v * (1.0 - s * f);       // falling ramp
    let u = v * (1.0 - s * (1.0 - f)); // rising ramp

    let (r, g, b) = match i % 6 {
        0 => (v, u, p),
        1 => (q, v, p),
        2 => (p, v, u),
        3 => (p, q, v),
        4 => (u, p, v),
        _ => (v, p, q),
    };
    Color::from_rgb(r, g, b)
}

/// Trade intensity heatmap indicator.
///
/// Distinct from [`TradeIntensityIndicator`] (raw single-colour bars) — this
/// variant applies rolling log-quantile percentile binning with a thermal
/// colour gradient for regime-adaptive intensity visualisation.
///
/// Data is stored in a `Vec<HeatmapPoint>` (forward-indexed, 0=oldest) for O(1) push
/// during rebuild, replacing the O(N log N) BTreeMap that caused UI freezes at 30K+ bars.
pub struct TradeIntensityHeatmapIndicator {
    cache: Caches,
    /// Forward-indexed storage (0 = oldest bar). Index matches storage_idx directly.
    data: Vec<HeatmapPoint>,
    lookback: usize,
    // Rolling window state — maintained across calls for O(1) incremental updates.
    ring: VecDeque<f32>,
    /// Sorted Vec of log values in the window (capacity ≤ lookback).
    /// Binary search gives O(log N) rank queries; Vec insert/remove is cache-friendly
    /// and avoids the O(K) pointer-chasing BTreeMap range scan that froze UI on rebuild.
    sorted: Vec<f32>,
    /// Number of tickseries datapoints processed so far (global index).
    next_idx: usize,
}

impl TradeIntensityHeatmapIndicator {
    pub fn new() -> Self {
        Self::with_lookback(2000)
    }

    pub fn with_lookback(lookback: usize) -> Self {
        Self {
            cache: Caches::default(),
            data: Vec::new(),
            lookback,
            ring: VecDeque::new(),
            sorted: Vec::with_capacity(lookback + 1),
            next_idx: 0,
        }
    }

    fn reset_state(&mut self) {
        self.ring.clear();
        self.sorted.clear(); // Vec::clear keeps allocation — no realloc on next rebuild
        self.data.clear(); // Vec::clear keeps allocation
        self.next_idx = 0;
    }

    /// Process a single bar at `idx` with the given `intensity` and candle `bullish` direction.
    /// Updates the rolling window and stores the result in `self.data`.
    ///
    /// Uses a sorted Vec for rank queries: O(log N) binary search, cache-friendly insert/remove.
    /// This is significantly faster than a BTreeMap range scan on rebuild (avoids pointer chasing).
    fn process_one(&mut self, idx: usize, intensity: f32, bullish: bool) {
        let log_val = intensity.log10().max(0.0);

        // --- Compute bin BEFORE pushing (no look-ahead) ---
        let n = self.sorted.len(); // same as ring.len()
        let k_actual = if n == 0 {
            5u8 // minimum K; first bar always gets bin 1
        } else {
            adaptive_k(n)
        };

        let bin = if n == 0 {
            1u8
        } else {
            // count(window ≤ log_val) via binary search: O(log N) — upper_bound position
            let rank_count = self.sorted.partition_point(|&v| v <= log_val);
            let rank = rank_count as f32 / n as f32;
            ((rank * k_actual as f32).ceil() as u8).clamp(1, k_actual)
        };

        // Vec push: O(1) amortised. idx must equal self.data.len() (sequential calls only).
        // Resize with sentinel (bin=0) for any gap caused by bars without microstructure.
        if idx > self.data.len() {
            self.data.resize(
                idx,
                HeatmapPoint {
                    intensity: 0.0,
                    bin: 0,
                    k_actual: 0,
                    bullish: false,
                },
            );
        }
        let t_val = if k_actual <= 1 {
            0.0
        } else {
            (bin - 1) as f32 / (k_actual - 1) as f32
        };
        // Per-bar bin assignment (trace level to avoid 2000 log calls per rebuild).
        // Use [oracle-rebuild-tail] and [oracle-incr-tail] for verification instead.
        log::trace!(
            "[oracle-bin] idx={} ti={:.4} bin={}/{} t={:.4} window={}",
            idx,
            intensity,
            bin,
            k_actual,
            t_val,
            n,
        );
        self.data.push(HeatmapPoint {
            intensity,
            bin,
            k_actual,
            bullish,
        });

        // --- Push current bar into sorted window ---
        // Insert in sorted order: O(N) Vec shift, but cache-friendly (contiguous memory).
        let ins_pos = self.sorted.partition_point(|&v| v < log_val);
        self.sorted.insert(ins_pos, log_val);
        self.ring.push_back(log_val);

        // Evict oldest if over capacity
        if self.ring.len() > self.lookback {
            let old = self.ring.pop_front().unwrap();
            // Find and remove exactly one copy of `old` from the sorted Vec.
            let pos = self.sorted.partition_point(|&v| v < old);
            if pos < self.sorted.len() {
                self.sorted.remove(pos);
            }
        }
    }

    /// Oracle probe: logs the full colour spectrum for the current K and a bin
    /// distribution histogram for the last 500 bars. Also writes to
    /// `/tmp/flowsurface-oracle.log` for easy inspection after an .app launch.
    fn log_oracle_spectrum(&self) {
        let k = adaptive_k(self.lookback) as usize;
        let mut lines: Vec<String> = Vec::new();
        lines.push(format!(
            "=== oracle-spectrum: lookback={} K={} data_len={} ===",
            self.lookback, k, self.data.len()
        ));

        // Section 1: colour table — one row per bin with actual hex and hue description
        lines.push("--- Colour table (bin → t → #RRGGBB) ---".into());
        for bin in 1..=(k as u8) {
            let t = if k <= 1 { 0.0 } else { (bin - 1) as f32 / (k - 1) as f32 };
            let c = thermal_color(t);
            let (r, g, b) = (
                (c.r * 255.0) as u8,
                (c.g * 255.0) as u8,
                (c.b * 255.0) as u8,
            );
            lines.push(format!("  K{bin:2}  t={t:.4}  #{r:02X}{g:02X}{b:02X}"));
        }

        // Section 2a: all-bars histogram (last 500, all k_actual values mixed)
        let sample_n = self.data.len().min(500);
        if sample_n > 0 {
            let sample = &self.data[self.data.len() - sample_n..];

            lines.push(format!("--- Bin histogram ALL k_actual (last {sample_n} bars) ---"));
            let mut hist_all = vec![0u32; k + 1];
            let mut zero_count = 0u32;
            for p in sample {
                if p.bin == 0 {
                    zero_count += 1;
                } else if p.bin as usize <= k {
                    hist_all[p.bin as usize] += 1;
                }
            }
            let peak_all = hist_all[1..].iter().copied().max().unwrap_or(1).max(1);
            for bin in 1..=(k as u8) {
                let count = hist_all[bin as usize];
                let bar_len = (count as f32 / peak_all as f32 * 30.0) as usize;
                lines.push(format!("  K{bin:2}  {count:4} bars  {}", "#".repeat(bar_len)));
            }
            if zero_count > 0 {
                lines.push(format!("  K0 (no-micro sentinel): {zero_count} bars"));
            }

            // Section 2b: filtered histogram — only bars where k_actual == current K
            // This shows the true percentile distribution at full lookback capacity.
            let k_u8 = k as u8;
            let mut hist_k = vec![0u32; k + 1];
            let mut current_k_total = 0u32;
            for p in sample {
                if p.k_actual == k_u8 {
                    current_k_total += 1;
                    if p.bin >= 1 && p.bin as usize <= k {
                        hist_k[p.bin as usize] += 1;
                    }
                }
            }
            lines.push(format!(
                "--- Bin histogram k_actual=={k} ONLY (n={current_k_total}/{sample_n}) ---"
            ));
            if current_k_total == 0 {
                lines.push(format!(
                    "  (no bars with k_actual={k} in sample — window not yet full; try larger lookback or more data)"
                ));
            } else {
                let peak_k = hist_k[1..].iter().copied().max().unwrap_or(1).max(1);
                let expected_per_bin = current_k_total as f32 / k as f32;
                for bin in 1..=(k as u8) {
                    let count = hist_k[bin as usize];
                    let bar_len = (count as f32 / peak_k as f32 * 30.0) as usize;
                    let ratio = count as f32 / expected_per_bin;
                    lines.push(format!(
                        "  K{bin:2}  {count:4} bars  {}  ({ratio:.2}x expected)",
                        "#".repeat(bar_len)
                    ));
                }
            }
        }

        let report = lines.join("\n");
        log::warn!("[oracle-spectrum]\n{report}");
        let _ = std::fs::write("/tmp/flowsurface-oracle.log", &report);
    }

    fn indicator_elem<'a>(
        &'a self,
        main_chart: &'a ViewState,
        visible_range: RangeInclusive<u64>,
    ) -> iced::Element<'a, Message> {
        if self.data.is_empty() {
            return center(iced::widget::text("Intensity Heatmap: no microstructure data")).into();
        }

        let tooltip = |p: &HeatmapPoint, _next: Option<&HeatmapPoint>| {
            PlotTooltip::new(format!(
                "Intensity: {:.1} t/s (bin {}/{})",
                p.intensity, p.bin, p.k_actual
            ))
        };

        let bar_kind = |p: &HeatmapPoint| BarClass::CandleColored { bullish: p.bullish };
        let value_fn = |p: &HeatmapPoint| p.bin as f32;

        let k_max = adaptive_k(self.lookback) as f32;
        let k_actual = self.data.last().map(|p| p.k_actual).unwrap_or(5);

        let bar_plot = BarPlot::new(value_fn, bar_kind)
            .bar_width_factor(0.9)
            .baseline(Baseline::Zero)
            .fixed_max(k_max)
            .with_tooltip(tooltip);

        let plot = HeatmapPlot { inner: bar_plot, k_actual };

        indicator_row_slice_with_legend(
            main_chart,
            &self.cache,
            self.cache.legend_cache(),
            plot,
            &self.data,
            visible_range,
        )
    }
}

/// Wraps a `BarPlot` to add a persistent bottom-left colour-scale legend via
/// `Plot::draw_panel_legend`. The legend is drawn in screen space (untransformed frame),
/// so coordinates are relative to the canvas widget's top-left corner.
struct HeatmapPlot<P> {
    inner: P,
    k_actual: u8,
}

impl<S, P> Plot<S> for HeatmapPlot<P>
where
    S: Series,
    P: Plot<S>,
{
    fn y_extents(&self, s: &S, range: RangeInclusive<u64>) -> Option<(f32, f32)> {
        self.inner.y_extents(s, range)
    }

    fn adjust_extents(&self, min: f32, max: f32) -> (f32, f32) {
        self.inner.adjust_extents(min, max)
    }

    fn draw<'a>(
        &'a self,
        frame: &'a mut canvas::Frame,
        ctx: &'a ViewState,
        theme: &iced::Theme,
        s: &S,
        range: RangeInclusive<u64>,
        scale: &YScale,
    ) {
        self.inner.draw(frame, ctx, theme, s, range, scale);
    }

    fn tooltip_fn(&self) -> Option<&crate::chart::indicator::plot::TooltipFn<S::Y>> {
        self.inner.tooltip_fn()
    }

    fn draw_panel_legend(&self, frame: &mut canvas::Frame) {
        draw_heatmap_legend(frame, self.k_actual);
    }
}

/// Draw the 9-stop colour-scale legend at the bottom-left of a screen-space canvas frame.
/// The frame must be in screen space (no prior translate/scale calls).
fn draw_heatmap_legend(frame: &mut canvas::Frame, k_actual: u8) {
    // One row per bin, hottest (bin K) at top, coldest (bin 1) at bottom.
    const SWATCH_W: f32 = 10.0;
    const SWATCH_H: f32 = 9.0;
    const ROW_H: f32 = 11.0;
    const PAD: f32 = 4.0;
    const GAP: f32 = 3.0;
    const FONT_SIZE: f32 = 9.0;
    const TEXT_OFFSET_X: f32 = SWATCH_W + GAP;
    // "K19 Max" = 7 chars × ~5.5 px — longest label (only top/bottom rows get suffix)
    const LEGEND_W: f32 = PAD + TEXT_OFFSET_X + 7.0 * 5.5 + PAD;
    let legend_h = k_actual as f32 * ROW_H + PAD * 2.0;

    let origin_x = PAD;
    let origin_y = (frame.height() - legend_h - PAD).max(PAD);

    // Semi-transparent background
    frame.fill_rectangle(
        Point::new(origin_x, origin_y),
        Size::new(LEGEND_W, legend_h),
        Color::from_rgba(0.0, 0.0, 0.0, 0.65),
    );

    for bin in (1..=k_actual).rev() {
        // row_idx 0 = top (hottest = bin K), row_idx K-1 = bottom (coldest = bin 1)
        let row_idx = (k_actual - bin) as f32;
        let row_y = origin_y + PAD + row_idx * ROW_H;

        let t = if k_actual <= 1 {
            0.0
        } else {
            (bin - 1) as f32 / (k_actual - 1) as f32
        };
        let color = thermal_color(t);

        // Colour swatch
        frame.fill_rectangle(
            Point::new(origin_x + PAD, row_y + (ROW_H - SWATCH_H) * 0.5),
            Size::new(SWATCH_W, SWATCH_H),
            color,
        );

        // Label: top and bottom bins get a suffix; intermediate bins show number only
        let label_text = if bin == k_actual {
            format!("K{bin} Max")
        } else if bin == 1 {
            "K1 Calm".to_string()
        } else {
            format!("K{bin}")
        };
        frame.fill_text(canvas::Text {
            content: label_text,
            position: Point::new(
                origin_x + PAD + TEXT_OFFSET_X,
                row_y + (ROW_H - FONT_SIZE) * 0.5,
            ),
            size: iced::Pixels(FONT_SIZE),
            color: Color::from_rgb(0.85, 0.85, 0.85),
            ..canvas::Text::default()
        });
    }
}

impl KlineIndicatorImpl for TradeIntensityHeatmapIndicator {
    fn clear_all_caches(&mut self) {
        self.cache.clear_all();
    }

    fn clear_crosshair_caches(&mut self) {
        self.cache.clear_crosshair();
    }

    fn element<'a>(
        &'a self,
        chart: &'a ViewState,
        visible_range: RangeInclusive<u64>,
    ) -> iced::Element<'a, Message> {
        self.indicator_elem(chart, visible_range)
    }

    fn rebuild_from_source(&mut self, source: &PlotData<KlineDataPoint>) {
        let prev_data_len = self.data.len();
        let prev_next_idx = self.next_idx;
        self.reset_state();
        if let PlotData::TickBased(tickseries) = source {
            for (idx, dp) in tickseries.datapoints.iter().enumerate() {
                // Always process every bar (0.0 intensity for bars without microstructure)
                // to keep the Vec densely packed (idx == data.len() invariant).
                let intensity = dp.microstructure.map(|m| m.trade_intensity).unwrap_or(0.0);
                let bullish = dp.kline.close >= dp.kline.open;
                self.process_one(idx, intensity, bullish);
            }
            self.next_idx = tickseries.datapoints.len();
            // Sample the last 3 bars' bins for color-shift detection
            let tail_sample: Vec<_> = self
                .data
                .iter()
                .rev()
                .take(3)
                .map(|p| (p.intensity, p.bin, p.k_actual, p.t()))
                .collect();
            log::info!(
                "[intensity-rebuild] FULL REBUILD: prev_data={} prev_next_idx={} \
                 new_data={} new_next_idx={} dp_count={} tail_bins={:?}",
                prev_data_len,
                prev_next_idx,
                self.data.len(),
                self.next_idx,
                tickseries.datapoints.len(),
                tail_sample,
            );

            // Oracle: log the LAST bar's bin (the newly added one) after rebuild.
            // This is the definitive assertion: did the newest bar get a non-zero bin?
            if let Some(last) = self.data.last() {
                let has_micro = tickseries
                    .datapoints
                    .last()
                    .and_then(|dp| dp.microstructure)
                    .is_some();
                log::info!(
                    "[oracle-rebuild-tail] idx={} ti={:.4} bin={}/{} t={:.4} \
                     has_micro={} bin_nonzero={}",
                    self.data.len() - 1,
                    last.intensity,
                    last.bin,
                    last.k_actual,
                    last.t(),
                    has_micro,
                    last.bin != 0,
                );
                if has_micro && last.bin == 0 {
                    log::error!(
                        "[oracle-FAIL] Newest bar has microstructure but bin=0 (sentinel)! \
                         Intensity coloring will be wrong (default blue instead of thermal).",
                    );
                    exchange::tg_alert!(
                        exchange::telegram::Severity::Critical,
                        "oracle",
                        "Oracle FAIL: bar has micro but bin=0 sentinel"
                    );
                }
            }
        }
        self.log_oracle_spectrum();
        self.clear_all_caches();
    }

    fn on_insert_klines(&mut self, _klines: &[Kline]) {}

    fn on_insert_trades(
        &mut self,
        _trades: &[Trade],
        old_dp_len: usize,
        source: &PlotData<KlineDataPoint>,
    ) {
        match source {
            PlotData::TimeBased(_) => return,
            PlotData::TickBased(tickseries) => {
                let new_len = tickseries.datapoints.len();
                if self.next_idx == old_dp_len {
                    // Incremental path: only process newly completed bars.
                    let new_bars = new_len.saturating_sub(old_dp_len);
                    for idx in old_dp_len..new_len {
                        let dp = &tickseries.datapoints[idx];
                        let intensity = dp.microstructure.map(|m| m.trade_intensity).unwrap_or(0.0);
                        let bullish = dp.kline.close >= dp.kline.open;
                        self.process_one(idx, intensity, bullish);
                    }
                    self.next_idx = new_len;
                    if new_bars > 0 {
                        let last = self
                            .data
                            .last()
                            .map(|p| (p.intensity, p.bin, p.k_actual, p.t()));
                        log::info!(
                            "[intensity-incr] +{} bars: old_dp={} new_dp={} data_len={} \
                             last_bar={:?}",
                            new_bars,
                            old_dp_len,
                            new_len,
                            self.data.len(),
                            last,
                        );

                        // Oracle: verify incrementally-added bar has non-zero bin
                        if let Some(last_p) = self.data.last() {
                            let has_micro = tickseries
                                .datapoints
                                .last()
                                .and_then(|dp| dp.microstructure)
                                .is_some();
                            log::info!(
                                "[oracle-incr-tail] idx={} ti={:.4} bin={}/{} t={:.4} \
                                 has_micro={} bin_nonzero={}",
                                self.data.len() - 1,
                                last_p.intensity,
                                last_p.bin,
                                last_p.k_actual,
                                last_p.t(),
                                has_micro,
                                last_p.bin != 0,
                            );
                            if has_micro && last_p.bin == 0 {
                                log::error!(
                                    "[oracle-FAIL] Incremental bar has micro but bin=0! \
                                     Intensity coloring bug on incremental path.",
                                );
                                exchange::tg_alert!(
                                    exchange::telegram::Severity::Critical,
                                    "oracle",
                                    "Oracle FAIL: incremental bar micro but bin=0"
                                );
                            }
                        }
                    }
                } else {
                    // State mismatch (e.g. after seek/reset): full rebuild.
                    log::warn!(
                        "[intensity-mismatch] next_idx={} != old_dp_len={} dp_count={} \
                         data_len={} → FULL REBUILD",
                        self.next_idx,
                        old_dp_len,
                        new_len,
                        self.data.len(),
                    );
                    self.rebuild_from_source(source);
                    return; // rebuild_from_source already cleared caches
                }
            }
        }
        self.clear_all_caches();
    }

    fn on_ticksize_change(&mut self, source: &PlotData<KlineDataPoint>) {
        self.rebuild_from_source(source);
    }

    fn on_basis_change(&mut self, source: &PlotData<KlineDataPoint>) {
        self.rebuild_from_source(source);
    }

    /// Return the thermal body colour for the candle at `storage_idx` (oldest-first index).
    /// Used by the main chart to colour candle bodies when this indicator is active.
    fn thermal_body_color(&self, storage_idx: u64) -> Option<Color> {
        self.data
            .get(storage_idx as usize)
            .filter(|p| p.bin != 0) // bin=0 is a sentinel for bars with no microstructure
            .map(|p| thermal_color(p.t()))
    }

    fn data_len(&self) -> usize {
        self.data.len()
    }

    fn draw_screen_legend(&self, frame: &mut iced::widget::canvas::Frame) {
        if self.data.is_empty() {
            return;
        }
        // Use the configured max K (asymptotic K for full lookback window) so the
        // legend always shows the full scale the user set, not the current window fill.
        draw_heatmap_legend(frame, adaptive_k(self.lookback));
    }
}
