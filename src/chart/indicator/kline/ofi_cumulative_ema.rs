// GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97
//! Rolling-window sum of EMA-smoothed OFI. Shows directional accumulation of
//! smoothed order flow over the last N bars (where N = EMA period) as a bar
//! histogram with zero baseline. Shares the OFI EMA period from kline::Config.
//!
//! Unlike a raw cumulative sum (which drifts unboundedly and becomes dominated
//! by ancient history), the rolling window stays responsive to recent order flow
//! direction changes.
//!
//! **Incremental updates**: EMA state, rolling window and sum are kept on the struct.
//! `on_insert_trades` processes only newly completed bars in O(1).

// GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97
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

use data::chart::{PlotData, kline::KlineDataPoint};
use data::conditional_ema::ConditionalEma;
use exchange::{Kline, Trade};

use iced::widget::{center, text};
use std::collections::VecDeque;
use std::ops::RangeInclusive;

/// Per-bar data point: rolling sum of EMA(OFI) and candle direction.
#[derive(Clone, Copy)]
struct CumOfiPoint {
    cumsum: f32,
    bullish: bool,
}

/// Cumulative EMA(OFI) indicator using a rolling window sum.
/// Applies EMA to raw OFI, then sums the last `ema_period` EMA values.
///
/// Data is stored in a `Vec<CumOfiPoint>` (forward-indexed, 0=oldest) for O(1) push
/// during rebuild, replacing the O(N log N) BTreeMap that caused UI freezes at 30K+ bars.
pub struct OFICumulativeEmaIndicator {
    cache: Caches,
    /// Forward-indexed storage (0 = oldest bar). Index matches storage_idx directly.
    data: Vec<CumOfiPoint>,
    ema_period: usize,
    // Rolling window state — maintained across calls for O(1) incremental updates.
    ema: ConditionalEma,
    ema_window: VecDeque<f32>,
    rolling_sum: f32,
    /// Number of tickseries datapoints processed so far (global index).
    next_idx: usize,
}

impl OFICumulativeEmaIndicator {
    pub fn new() -> Self {
        Self::with_ema_period(20)
    }

    pub fn with_ema_period(period: usize) -> Self {
        Self {
            cache: Caches::default(),
            data: Vec::new(),
            ema_period: period,
            ema: ConditionalEma::new(period),
            ema_window: VecDeque::new(),
            rolling_sum: 0.0,
            next_idx: 0,
        }
    }

    fn reset_state(&mut self) {
        // Recreate EMA to pick up any period change and reset internal value.
        self.ema = ConditionalEma::new(self.ema_period);
        self.ema_window.clear();
        self.rolling_sum = 0.0;
        self.data.clear(); // Vec::clear keeps allocation
        self.next_idx = 0;
    }

    fn process_one(&mut self, idx: usize, ofi: f32, bullish: bool) {
        let ema_val = self.ema.update(ofi, true);
        self.ema_window.push_back(ema_val);
        self.rolling_sum += ema_val;
        if self.ema_window.len() > self.ema_period {
            self.rolling_sum -= self.ema_window.pop_front().unwrap_or(0.0);
        }
        // Vec push: O(1) amortised. Resize with sentinel for any gap (bars without micro).
        if idx > self.data.len() {
            self.data.resize(idx, CumOfiPoint { cumsum: 0.0, bullish: false });
        }
        self.data.push(CumOfiPoint { cumsum: self.rolling_sum, bullish });
    }

    fn indicator_elem<'a>(
        &'a self,
        main_chart: &'a ViewState,
        visible_range: RangeInclusive<u64>,
    ) -> iced::Element<'a, Message> {
        if self.data.is_empty() {
            return center(text("OFI \u{03A3}EMA: no microstructure data")).into();
        }

        let tooltip = |p: &CumOfiPoint, _next: Option<&CumOfiPoint>| {
            let sign = if p.cumsum >= 0.0 { "+" } else { "" };
            PlotTooltip::new(format!("OFI \u{03A3}EMA: {sign}{:.3}", p.cumsum))
        };

        let bar_kind = |p: &CumOfiPoint| BarClass::CandleColored { bullish: p.bullish };
        let value_fn = |p: &CumOfiPoint| p.cumsum;

        let plot = BarPlot::new(value_fn, bar_kind)
            .bar_width_factor(0.9)
            .baseline(Baseline::Zero)
            .with_tooltip(tooltip);

        indicator_row_slice(main_chart, &self.cache, plot, &self.data, visible_range)
    }
}

impl KlineIndicatorImpl for OFICumulativeEmaIndicator {
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
                // Always process every bar (0.0 ofi for bars without microstructure)
                // to keep the Vec densely packed (idx == data.len() invariant).
                let ofi = dp.microstructure.map(|m| m.ofi).unwrap_or(0.0);
                let bullish = dp.kline.close >= dp.kline.open;
                self.process_one(idx, ofi, bullish);
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
                        let ofi = dp.microstructure.map(|m| m.ofi).unwrap_or(0.0);
                        let bullish = dp.kline.close >= dp.kline.open;
                        self.process_one(idx, ofi, bullish);
                    }
                    self.next_idx = new_len;
                } else {
                    // State mismatch: full rebuild.
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
}
