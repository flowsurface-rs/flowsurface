use crate::chart::{
    Caches, Message, ViewState,
    indicator::{
        indicator_row,
        kline::KlineIndicatorImpl,
        plot::{
            PlotTooltip,
            bar::{BarClass, BarPlot},
        },
    },
};

use data::chart::{PlotData, kline::KlineDataPoint};
use data::util::format_with_commas;
use exchange::{Kline, Trade};

use iced::widget::{center, text};
use std::collections::BTreeMap;
use std::ops::RangeInclusive;

/// Trade count indicator: individual_trade_count per bar.
/// Only available for range bars with microstructure data from ClickHouse.
// GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97
pub struct TradeCountIndicator {
    cache: Caches,
    data: BTreeMap<u64, f32>,
}

impl TradeCountIndicator {
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
        if self.data.is_empty() {
            return center(text("Trade Count: no microstructure data")).into();
        }

        let tooltip = |value: &f32, _next: Option<&f32>| {
            PlotTooltip::new(format!("Trades: {}", format_with_commas(*value)))
        };

        let bar_kind = |_value: &f32| BarClass::Single;

        let value_fn = |v: &f32| *v;

        let plot = BarPlot::new(value_fn, bar_kind)
            .bar_width_factor(0.9)
            .with_tooltip(tooltip);

        indicator_row(main_chart, &self.cache, plot, &self.data, visible_range)
    }
}

impl KlineIndicatorImpl for TradeCountIndicator {
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
                self.data = tickseries.trade_count_data();
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
                        self.data.insert(idx as u64, m.trade_count as f32);
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
