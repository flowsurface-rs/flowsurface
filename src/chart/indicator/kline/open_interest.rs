use crate::chart::{
    Basis, Caches, Message, ViewState,
    indicator::{
        indicator_row,
        kline::{AvailabilityCause, FetchCtx, IndicatorAvailability, KlineIndicatorImpl},
        plot::{AnySeries, PlotTooltip, line::LinePlot},
    },
};
use crate::connector::fetcher::FetchRange;

use data::chart::{
    PlotData,
    indicator::{KlineIndicatorConfig, OpenInterestSettings},
    kline::KlineDataPoint,
};
use data::util::format_with_commas;
use exchange::adapter::Exchange;
use exchange::{Kline, Timeframe, Trade, UnixMs};

use iced::widget::{center, row, text};
use std::{collections::BTreeMap, ops::RangeInclusive};

pub struct OpenInterestIndicator {
    cache: Caches,
    pub data: BTreeMap<UnixMs, f64>,
    settings: OpenInterestSettings,
}

impl OpenInterestIndicator {
    pub fn new(settings: OpenInterestSettings) -> Self {
        Self {
            cache: Caches::default(),
            data: BTreeMap::new(),
            settings,
        }
    }

    fn indicator_elem<'a>(
        &'a self,
        main_chart: &'a ViewState,
        data_labels_always_visible: bool,
        visible_range: RangeInclusive<u64>,
    ) -> iced::Element<'a, Message> {
        if let Some(message) = self.unavailable_message(main_chart, "Open Interest") {
            return center(text(message)).into();
        }

        let (earliest, latest) = visible_range.clone().into_inner();
        if latest < earliest {
            return row![].into();
        }

        let tooltip = |value: &f64, next: Option<&f64>| {
            let value_text = format!("Open Interest: {}", format_with_commas(*value));
            let change_text = if let Some(next_value) = next {
                let delta = next_value - *value;
                let sign = if delta >= 0.0 { "+" } else { "" };
                format!("Change: {}{}", sign, format_with_commas(delta))
            } else {
                "Change: N/A".to_string()
            };
            PlotTooltip::new(format!("{value_text}\n{change_text}"))
        };

        let value_fn = |v: &f64| *v as f32;
        let line_color = self
            .settings
            .custom_color
            .filter(|_| self.settings.custom_color_enabled);

        let plot = LinePlot::new(value_fn)
            .stroke_width(self.settings.line_width.clamp(0.5, 4.0))
            .show_points(self.settings.show_points)
            .point_radius_factor(0.2)
            // Open interest is snapshotted at candle open, not computed from close like regular indicators.
            // Shift left by 1 so each OI value aligns with the equivalent candle close.
            .shift(-1)
            .padding(0.08)
            .with_tooltip(tooltip)
            .with_line_color(line_color);

        indicator_row(
            main_chart,
            &self.cache,
            data_labels_always_visible,
            plot,
            AnySeries::forward_unix_ms(&self.data),
            visible_range,
        )
    }

    // helper to compute (earliest, latest) present OI keys
    fn oi_timerange(&self, latest_kline: UnixMs) -> (UnixMs, UnixMs) {
        let mut from_time = latest_kline;
        let mut to_time = UnixMs::ZERO;

        self.data.iter().for_each(|(time, _)| {
            from_time = from_time.min(*time);
            to_time = to_time.max(*time);
        });
        (from_time, to_time)
    }

    fn is_supported_exchange(exchange: Exchange) -> bool {
        exchange.is_perps()
            && exchange != Exchange::HyperliquidLinear
            && exchange != Exchange::MexcLinear
            && exchange != Exchange::MexcInverse
    }

    fn is_supported_timeframe(timeframe: Timeframe) -> bool {
        timeframe >= Timeframe::M5 && timeframe <= Timeframe::H4 && timeframe != Timeframe::H2
    }

    fn availability_for(basis: Basis, exchange: Exchange) -> IndicatorAvailability {
        match basis {
            Basis::Tick(_) => IndicatorAvailability::Unavailable(AvailabilityCause::Basis(basis)),
            Basis::Time(timeframe) => {
                if !Self::is_supported_exchange(exchange) {
                    IndicatorAvailability::Unavailable(AvailabilityCause::Exchange(exchange))
                } else if !Self::is_supported_timeframe(timeframe) {
                    IndicatorAvailability::Unavailable(AvailabilityCause::Timeframe(timeframe))
                } else {
                    IndicatorAvailability::Available
                }
            }
        }
    }
}

impl KlineIndicatorImpl for OpenInterestIndicator {
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

    fn availability(&self, chart: &ViewState) -> IndicatorAvailability {
        Self::availability_for(chart.basis, chart.ticker_info.exchange())
    }

    fn fetch_range(&mut self, ctx: &FetchCtx) -> Option<FetchRange> {
        let availability = Self::availability_for(
            Basis::Time(ctx.timeframe),
            ctx.main_chart.ticker_info.exchange(),
        );
        if !matches!(availability, IndicatorAvailability::Available) {
            return None;
        }

        let (oi_earliest, oi_latest) = self.oi_timerange(ctx.kline_latest);

        if ctx.visible_earliest < oi_earliest {
            return Some(FetchRange::OpenInterest(ctx.prefetch_earliest, oi_earliest));
        }

        if oi_latest < ctx.kline_latest {
            return Some(FetchRange::OpenInterest(
                oi_latest.max(ctx.prefetch_earliest),
                ctx.kline_latest,
            ));
        }

        None
    }

    fn rebuild_from_source(&mut self, _source: &PlotData<KlineDataPoint>) {
        // OI comes from network via external fetches(trade-fetch alike)
        self.clear_all_caches();
    }

    fn on_insert_klines(&mut self, _klines: &[Kline], _source: &PlotData<KlineDataPoint>) {}

    fn on_insert_trades(
        &mut self,
        _trades: &[Trade],
        _old_dp_len: usize,
        _source: &PlotData<KlineDataPoint>,
    ) {
    }

    fn on_ticksize_change(&mut self, _source: &PlotData<KlineDataPoint>) {}

    fn on_basis_change(&mut self, _source: &PlotData<KlineDataPoint>) {}

    fn on_open_interest(&mut self, data: &[exchange::OpenInterest]) {
        self.data.extend(data.iter().map(|oi| (oi.time, oi.value)));
        self.clear_all_caches();
    }

    fn apply_config(&mut self, config: &KlineIndicatorConfig, _source: &PlotData<KlineDataPoint>) {
        if let KlineIndicatorConfig::OpenInterest(settings) = config
            && self.settings != *settings
        {
            self.settings = *settings;
            self.clear_all_caches();
        }
    }
}
