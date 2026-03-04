use crate::chart::{
    Caches, Message, ViewState,
    indicator::{
        indicator_row,
        kline::KlineIndicatorImpl,
        plot::{
            PlotTooltip,
            bar::{BarClass, BarPlot, Baseline},
        },
    },
};

use data::chart::{PlotData, kline::KlineDataPoint};
use data::util::format_with_commas;
use exchange::{Kline, Trade, Volume};

use std::collections::BTreeMap;
use std::ops::RangeInclusive;

// GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97

fn kline_volume_delta(volume: &Volume) -> f32 {
    volume.buy_sell()
        .map(|(b, s)| f32::from(b) - f32::from(s))
        .unwrap_or(0.0)
}

/// Delta indicator: buy_volume - sell_volume per bar.
/// Histogram direction shows +/-, bar color follows candle direction for divergence.
pub struct DeltaIndicator {
    cache: Caches,
    /// (delta_value, bullish) — bullish = close >= open
    data: BTreeMap<u64, (f32, bool)>,
}

impl DeltaIndicator {
    pub fn new() -> Self {
        Self {
            cache: Caches::default(),
            data: BTreeMap::new(),
        }
    }

    fn indicator_elem<'a>(
        &'a self,
        main_chart: &'a ViewState,
        visible_range: RangeInclusive<u64>,
    ) -> iced::Element<'a, Message> {
        let tooltip = |value: &(f32, bool), _next: Option<&(f32, bool)>| {
            let sign = if value.0 >= 0.0 { "+" } else { "" };
            PlotTooltip::new(format!("Delta: {sign}{}", format_with_commas(value.0)))
        };

        let bar_kind = |value: &(f32, bool)| BarClass::CandleColored { bullish: value.1 };

        let value_fn = |v: &(f32, bool)| v.0;

        let plot = BarPlot::new(value_fn, bar_kind)
            .bar_width_factor(0.9)
            .baseline(Baseline::Zero)
            .with_tooltip(tooltip);

        indicator_row(main_chart, &self.cache, plot, &self.data, visible_range)
    }
}

impl KlineIndicatorImpl for DeltaIndicator {
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
            PlotData::TimeBased(timeseries) => {
                self.data = timeseries
                    .volume_data()
                    .into_iter()
                    // GitHub Issue: https://github.com/terrylica/flowsurface/issues/1 (upstream-merge: Volume newtype)
                    .map(|(k, volume)| {
                        let (buy, sell) = volume.buy_sell()
                            .map(|(b, s)| (f32::from(b), f32::from(s)))
                            .unwrap_or((0.0, 0.0));
                        (k, (buy - sell, buy >= sell))
                    })
                    .collect();
            }
            PlotData::TickBased(tickseries) => {
                self.data = tickseries
                    .datapoints
                    .iter()
                    .enumerate()
                    .map(|(idx, dp)| {
                        let delta = kline_volume_delta(&dp.kline.volume);
                        let bullish = dp.kline.close >= dp.kline.open;
                        (idx as u64, (delta, bullish))
                    })
                    .collect();
            }
        }
        self.clear_all_caches();
    }

    fn on_insert_klines(&mut self, klines: &[Kline]) {
        for kline in klines {
            let delta = kline_volume_delta(&kline.volume);
            let bullish = kline.close >= kline.open;
            self.data.insert(kline.time, (delta, bullish));
        }
        self.clear_all_caches();
    }

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
                    let delta = kline_volume_delta(&dp.kline.volume);
                    let bullish = dp.kline.close >= dp.kline.open;
                    self.data.insert(idx as u64, (delta, bullish));
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
