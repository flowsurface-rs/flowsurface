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
use exchange::{Kline, Trade, Volume};

use std::collections::BTreeMap;
use std::ops::RangeInclusive;

pub struct VolumeIndicator {
    cache: Caches,
    data: BTreeMap<u64, Volume>,
}

impl VolumeIndicator {
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
        let tooltip = |volume: &Volume, _next: Option<&Volume>| {
            if let Some((buy, sell)) = volume.buy_sell() {
                let buy_t = format!("Buy Volume: {}", format_with_commas(f32::from(buy)));
                let sell_t = format!("Sell Volume: {}", format_with_commas(f32::from(sell)));
                PlotTooltip::new(format!("{buy_t}\n{sell_t}"))
            } else {
                PlotTooltip::new(format!(
                    "Volume: {}",
                    format_with_commas(f32::from(volume.total()))
                ))
            }
        };

        let bar_kind = |volume: &Volume| {
            if let Some((buy, sell)) = volume.buy_sell() {
                BarClass::Overlay {
                    overlay: f32::from(buy) - f32::from(sell),
                }
            } else {
                BarClass::Single
            }
        };

        let value_fn = |volume: &Volume| f32::from(volume.total());

        let plot = BarPlot::new(value_fn, bar_kind)
            .bar_width_factor(0.9)
            .with_tooltip(tooltip);

        indicator_row(main_chart, &self.cache, plot, &self.data, visible_range)
    }
}

impl KlineIndicatorImpl for VolumeIndicator {
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
                self.data = timeseries.volume_data();
            }
            PlotData::TickBased(tickseries) => {
                self.data = tickseries.volume_data();
            }
        }
        self.clear_all_caches();
    }

    fn on_insert_klines(&mut self, klines: &[Kline]) {
        for kline in klines {
            self.data.insert(kline.time, kline.volume);
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
                    self.data.insert(idx as u64, dp.kline.volume);
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
