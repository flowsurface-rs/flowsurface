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
        indicator_row_slice,
        kline::KlineIndicatorImpl,
        plot::{
            PlotTooltip,
            bar::{BarClass, BarPlot, Baseline},
        },
    },
};

use data::chart::{PlotData, kline::KlineDataPoint, kline::adaptive_k};
use exchange::{Kline, Trade};

use iced::widget::{center, text};
use iced::Color;
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

/// 5-stop thermal colour gradient: blue → cyan → green → amber → red.
/// `t = 0.0` = cold (calm), `t = 1.0` = hot (spike).
fn thermal_color(t: f32) -> Color {
    // (stop, r, g, b)
    const STOPS: [(f32, f32, f32, f32); 5] = [
        (0.00, 0.129, 0.588, 0.953), // #2196F3 blue
        (0.25, 0.000, 0.737, 0.831), // #00BCD4 cyan
        (0.50, 0.298, 0.686, 0.314), // #4CAF50 green
        (0.75, 1.000, 0.757, 0.027), // #FFC107 amber
        (1.00, 0.957, 0.263, 0.212), // #F44336 red
    ];
    let t = t.clamp(0.0, 1.0);
    let i = STOPS.partition_point(|&(s, _, _, _)| s < t).saturating_sub(1);
    let i = i.min(STOPS.len() - 2);
    let (t0, r0, g0, b0) = STOPS[i];
    let (t1, r1, g1, b1) = STOPS[i + 1];
    let f = if (t1 - t0).abs() < f32::EPSILON { 0.0 } else { (t - t0) / (t1 - t0) };
    Color::from_rgb(r0 + f * (r1 - r0), g0 + f * (g1 - g0), b0 + f * (b1 - b0))
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
        self.data.clear();   // Vec::clear keeps allocation
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
            self.data.resize(idx, HeatmapPoint { intensity: 0.0, bin: 0, k_actual: 0, bullish: false });
        }
        self.data.push(HeatmapPoint { intensity, bin, k_actual, bullish });

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

    fn indicator_elem<'a>(
        &'a self,
        main_chart: &'a ViewState,
        visible_range: RangeInclusive<u64>,
    ) -> iced::Element<'a, Message> {
        if self.data.is_empty() {
            return center(text("Intensity Heatmap: no microstructure data")).into();
        }

        let tooltip = |p: &HeatmapPoint, _next: Option<&HeatmapPoint>| {
            PlotTooltip::new(format!(
                "Intensity: {:.1} t/s (bin {}/{})",
                p.intensity, p.bin, p.k_actual
            ))
        };

        let bar_kind = |p: &HeatmapPoint| BarClass::CandleColored { bullish: p.bullish };
        // Height = bin level (1..K_actual) — bounded, outlier-immune.
        let value_fn = |p: &HeatmapPoint| p.bin as f32;

        // Pin Y scale to adaptive_k(lookback) so early K=5 bars don't appear
        // incorrectly short alongside mature K=13 bars when scrolling.
        let k_max = adaptive_k(self.lookback) as f32;
        let plot = BarPlot::new(value_fn, bar_kind)
            .bar_width_factor(0.9)
            .baseline(Baseline::Zero)
            .fixed_max(k_max)
            .with_tooltip(tooltip);

        indicator_row_slice(main_chart, &self.cache, plot, &self.data, visible_range)
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
        }
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
                    for idx in old_dp_len..new_len {
                        let dp = &tickseries.datapoints[idx];
                        let intensity = dp.microstructure.map(|m| m.trade_intensity).unwrap_or(0.0);
                        let bullish = dp.kline.close >= dp.kline.open;
                        self.process_one(idx, intensity, bullish);
                    }
                    self.next_idx = new_len;
                } else {
                    // State mismatch (e.g. after seek/reset): full rebuild.
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
}
