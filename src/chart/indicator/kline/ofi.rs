// GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97

use crate::chart::{
    Caches, Message, ViewState,
    indicator::{
        indicator_row,
        kline::KlineIndicatorImpl,
        plot::{
            PlotTooltip,
            bar::BarClass,
            bar_with_ema_overlay::{BarWithEmaOverlay, EmaLineConfig},
        },
    },
};

use data::chart::{PlotData, kline::KlineDataPoint};
use data::conditional_ema::ConditionalEma;
use exchange::{Kline, Trade};

use iced::widget::{center, text};
use std::collections::BTreeMap;
use std::ops::RangeInclusive;

/// Per-bar data point for OFI with precomputed directional EMA values.
#[derive(Clone, Copy)]
pub(crate) struct OfiPoint {
    pub ofi: f32,
    pub bullish: bool,
    pub green_ema: f32,
    pub red_ema: f32,
}

/// Order Flow Imbalance indicator with directional EMA overlays.
/// Green EMA tracks only bullish-bar OFI, red EMA tracks only bearish-bar OFI.
/// Between non-matching bars each EMA carries forward (continuous flat line).
pub struct OFIIndicator {
    cache: Caches,
    data: BTreeMap<u64, OfiPoint>,
    ema_period: usize,
    green_ema: ConditionalEma,
    red_ema: ConditionalEma,
}

impl OFIIndicator {
    pub fn new() -> Self {
        Self::with_ema_period(20)
    }

    pub fn with_ema_period(period: usize) -> Self {
        Self {
            cache: Caches::default(),
            data: BTreeMap::new(),
            ema_period: period,
            green_ema: ConditionalEma::new(period),
            red_ema: ConditionalEma::new(period),
        }
    }

    fn indicator_elem<'a>(
        &'a self,
        main_chart: &'a ViewState,
        visible_range: RangeInclusive<u64>,
    ) -> iced::Element<'a, Message> {
        if self.data.is_empty() {
            return center(text("OFI: no microstructure data")).into();
        }

        let tooltip = |p: &OfiPoint, _next: Option<&OfiPoint>| {
            let sign = if p.ofi >= 0.0 { "+" } else { "" };
            PlotTooltip::new(format!(
                "OFI: {sign}{:.3} | \u{2191}EMA: {:.3} | \u{2193}EMA: {:.3}",
                p.ofi, p.green_ema, p.red_ema
            ))
        };

        let bar_kind = |p: &OfiPoint| BarClass::CandleColored { bullish: p.bullish };
        let value_fn = |p: &OfiPoint| p.ofi;

        let ema_lines = vec![
            EmaLineConfig {
                extract: |p: &OfiPoint| p.green_ema,
                color: |palette| palette.success.base.color,
                stroke_width: 1.5,
            },
            EmaLineConfig {
                extract: |p: &OfiPoint| p.red_ema,
                color: |palette| palette.danger.base.color,
                stroke_width: 1.5,
            },
        ];

        let plot = BarWithEmaOverlay::new(value_fn, bar_kind, ema_lines)
            .with_tooltip(tooltip);

        indicator_row(main_chart, &self.cache, plot, &self.data, visible_range)
    }

}

impl KlineIndicatorImpl for OFIIndicator {
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
        match source {
            PlotData::TimeBased(_) => {
                self.data.clear();
            }
            PlotData::TickBased(tickseries) => {
                self.green_ema = ConditionalEma::new(self.ema_period);
                self.red_ema = ConditionalEma::new(self.ema_period);

                self.data = tickseries
                    .datapoints
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, dp)| {
                        dp.microstructure.map(|m| {
                            let bullish = dp.kline.close >= dp.kline.open;
                            let g = self.green_ema.update(m.ofi, bullish);
                            let r = self.red_ema.update(m.ofi, !bullish);
                            (idx as u64, OfiPoint {
                                ofi: m.ofi,
                                bullish,
                                green_ema: g,
                                red_ema: r,
                            })
                        })
                    })
                    .collect();
            }
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
                let start_idx = old_dp_len.saturating_sub(1);
                for (idx, dp) in tickseries.datapoints.iter().enumerate().skip(start_idx) {
                    if let Some(m) = dp.microstructure {
                        let bullish = dp.kline.close >= dp.kline.open;
                        let g = self.green_ema.update(m.ofi, bullish);
                        let r = self.red_ema.update(m.ofi, !bullish);
                        self.data.insert(idx as u64, OfiPoint {
                            ofi: m.ofi,
                            bullish,
                            green_ema: g,
                            red_ema: r,
                        });
                    }
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
