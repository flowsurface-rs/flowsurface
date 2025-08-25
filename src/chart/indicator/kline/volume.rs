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

use std::collections::BTreeMap;
use std::ops::RangeInclusive;

pub struct VolumeIndicator {
    cache: Caches,
    data: BTreeMap<u64, (f32, f32)>,
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
        datapoints: &'a BTreeMap<u64, (f32, f32)>,
        visible_range: RangeInclusive<u64>,
    ) -> iced::Element<'a, Message> {
        let tooltip = |&(buy, sell): &(f32, f32), _next: Option<&(f32, f32)>| {
            if buy == -1.0 {
                PlotTooltip::new(format!("Volume: {}", format_with_commas(sell)))
            } else {
                let buy_t = format!("Buy Volume: {}", format_with_commas(buy));
                let sell_t = format!("Sell Volume: {}", format_with_commas(sell));
                PlotTooltip::new(format!("{buy_t}\n{sell_t}"))
            }
        };

        let bar_kind = |&(buy, sell): &(f32, f32)| {
            if buy == -1.0 {
                BarClass::Single // bybit workaround: single bar
            } else {
                BarClass::Overlay {
                    overlay: buy - sell,
                } // use the overlay for volume delta, sign determines up/down color
            }
        };

        let y_value = |&(buy, sell): &(f32, f32)| {
            if buy == -1.0 { sell } else { buy + sell }
        };

        let plot = BarPlot::new(y_value, bar_kind)
            .bar_width_factor(0.9)
            .with_tooltip(tooltip);

        indicator_row(main_chart, &self.cache, plot, datapoints, visible_range)
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
        self.indicator_elem(chart, &self.data, visible_range)
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

    fn on_new_klines(&mut self, klines: &[Kline]) {
        for kline in klines {
            self.data
                .insert(kline.time, (kline.volume.0, kline.volume.1));
        }
        self.clear_all_caches();
    }

    fn on_insert_trades(&mut self, _trades: &[Trade], source: &PlotData<KlineDataPoint>) {
        // For tick-based, recompute tail cheaply; for simplicity, rebuild from source.
        // If needed, you can optimize by only updating affected indexes.
        self.rebuild_from_source(source);
    }

    fn on_change_tick_size(&mut self, source: &PlotData<KlineDataPoint>) {
        self.rebuild_from_source(source);
    }

    fn on_basis_changed(&mut self, source: &PlotData<KlineDataPoint>) {
        self.rebuild_from_source(source);
    }
}
