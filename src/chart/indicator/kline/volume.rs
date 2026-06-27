use crate::chart::{
    Caches, Message, ViewState,
    indicator::{
        indicator_row,
        kline::{BasisSeries, BasisSeriesExt, KlineIndicatorImpl},
        plot::{
            PlotTooltip,
            bar::{BarClass, BarPlot},
        },
    },
};

use data::chart::{
    PlotData,
    indicator::{KlineIndicatorConfig, KlineVolumeSettings},
    kline::KlineDataPoint,
};
use data::util::format_with_commas;
use exchange::{Kline, Trade, Volume};

use std::ops::RangeInclusive;

pub struct VolumeIndicator {
    cache: Caches,
    data: BasisSeries<Volume>,
    settings: KlineVolumeSettings,
}

impl VolumeIndicator {
    pub fn new(settings: KlineVolumeSettings) -> Self {
        Self {
            cache: Caches::default(),
            data: BasisSeries::default(),
            settings,
        }
    }

    fn indicator_elem<'a>(
        &'a self,
        main_chart: &'a ViewState,
        data_labels_always_visible: bool,
        visible_range: RangeInclusive<u64>,
    ) -> iced::Element<'a, Message> {
        let tooltip = |volume: &Volume, _next: Option<&Volume>| {
            if let Some((buy, sell)) = volume.buy_sell() {
                let buy_t = format!("Buy Volume: {}", format_with_commas(f64::from(buy)));
                let sell_t = format!("Sell Volume: {}", format_with_commas(f64::from(sell)));
                PlotTooltip::new(format!("{buy_t}\n{sell_t}"))
            } else {
                PlotTooltip::new(format!(
                    "Volume: {}",
                    format_with_commas(f64::from(volume.total()))
                ))
            }
        };

        let bar_kind = |volume: &Volume| {
            if let Some((buy, sell)) = volume.buy_sell() {
                BarClass::Overlay {
                    overlay: buy.to_f32_lossy() - sell.to_f32_lossy(),
                    positive: self
                        .settings
                        .custom_buy_color
                        .filter(|_| self.settings.custom_color_enabled)
                        .unwrap_or(self.settings.colors.buy_color),
                    negative: self
                        .settings
                        .custom_sell_color
                        .filter(|_| self.settings.custom_color_enabled)
                        .unwrap_or(self.settings.colors.sell_color),
                }
            } else {
                BarClass::Single {
                    color: self
                        .settings
                        .custom_buy_color
                        .filter(|_| self.settings.custom_color_enabled)
                        .unwrap_or(self.settings.colors.buy_color),
                }
            }
        };

        let value_fn = |volume: &Volume| volume.total().to_f32_lossy();

        let mut plot = BarPlot::new(value_fn, bar_kind)
            .bar_width_factor(self.settings.bar_width_factor.clamp(0.2, 1.0));
        if self.settings.show_tooltip {
            plot = plot.with_tooltip(tooltip);
        }

        indicator_row(
            main_chart,
            &self.cache,
            data_labels_always_visible,
            plot,
            self.data.as_plot_series(),
            visible_range,
        )
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
        data_labels_always_visible: bool,
        visible_range: RangeInclusive<u64>,
    ) -> iced::Element<'a, Message> {
        self.indicator_elem(chart, data_labels_always_visible, visible_range)
    }

    fn rebuild_from_source(&mut self, source: &PlotData<KlineDataPoint>) {
        self.data = source.map_basis_series(
            |timeseries| timeseries.volume_data(),
            |tickseries| tickseries.volume_data(),
        );
        self.clear_all_caches();
    }

    fn on_insert_klines(&mut self, klines: &[Kline], _source: &PlotData<KlineDataPoint>) {
        let Some(data) = self.data.time_mut() else {
            return;
        };

        for kline in klines {
            data.insert(kline.time, kline.volume);
        }

        self.clear_all_caches();
    }

    fn on_insert_trades(
        &mut self,
        _trades: &[Trade],
        old_dp_len: usize,
        source: &PlotData<KlineDataPoint>,
    ) {
        let Some(data) = self.data.tick_mut() else {
            return;
        };

        match source {
            PlotData::TimeBased(_) => {}
            PlotData::TickBased(tickseries) => {
                let start_idx = old_dp_len.saturating_sub(1);
                for (idx, dp) in tickseries.datapoints.iter().enumerate().skip(start_idx) {
                    data.insert(idx as u64, dp.kline.volume);
                }
                self.clear_all_caches();
            }
        }
    }

    fn on_ticksize_change(&mut self, source: &PlotData<KlineDataPoint>) {
        self.rebuild_from_source(source);
    }

    fn on_basis_change(&mut self, source: &PlotData<KlineDataPoint>) {
        self.rebuild_from_source(source);
    }

    fn apply_config(&mut self, config: &KlineIndicatorConfig, _source: &PlotData<KlineDataPoint>) {
        if let KlineIndicatorConfig::Volume(settings) = config
            && self.settings != *settings
        {
            self.settings = *settings;
            self.clear_all_caches();
        }
    }
}
