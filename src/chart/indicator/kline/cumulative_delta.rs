use crate::chart::{
    Caches, Message, ViewState,
    indicator::{
        indicator_row,
        kline::KlineIndicatorImpl,
        plot::{PlotTooltip, line::LinePlot},
    },
};

use data::chart::{PlotData, kline::KlineDataPoint};
use data::util::format_with_commas;
use exchange::{Kline, Trade, Volume};

use std::collections::BTreeMap;
use std::ops::RangeInclusive;

#[derive(Debug, Clone, Copy, Default)]
pub struct CumulativeDeltaPoint {
    /// Buy volume - sell volume for this candle / tick bucket.
    pub delta: f32,
    /// Running sum of delta from the oldest loaded datapoint to this datapoint.
    pub cumulative: f32,
}

pub struct CumulativeDeltaIndicator {
    cache: Caches,
    /// Per-bucket delta. Stored separately so inserting/replacing older klines can
    /// rebuild the cumulative line without needing the full chart source.
    delta: BTreeMap<u64, f32>,
    data: BTreeMap<u64, CumulativeDeltaPoint>,
}

impl CumulativeDeltaIndicator {
    pub fn new() -> Self {
        Self {
            cache: Caches::default(),
            delta: BTreeMap::new(),
            data: BTreeMap::new(),
        }
    }

    fn indicator_elem<'a>(
        &'a self,
        main_chart: &'a ViewState,
        visible_range: RangeInclusive<u64>,
    ) -> iced::Element<'a, Message> {
        let tooltip = |point: &CumulativeDeltaPoint, _next: Option<&CumulativeDeltaPoint>| {
            let cvd = format!("CVD: {}", format_with_commas(point.cumulative));
            let sign = if point.delta >= 0.0 { "+" } else { "" };
            let delta = format!("Delta: {sign}{}", format_with_commas(point.delta));
            PlotTooltip::new(format!("{cvd}\n{delta}"))
        };

        let value_fn = |point: &CumulativeDeltaPoint| point.cumulative;

        let plot = LinePlot::new(value_fn)
            .stroke_width(1.0)
            .show_points(true)
            .point_radius_factor(0.2)
            .padding(0.08)
            .with_tooltip(tooltip);

        indicator_row(main_chart, &self.cache, plot, &self.data, visible_range)
    }

    fn kline_delta(kline: &Kline) -> f32 {
        Self::volume_delta(kline.volume)
    }

    fn volume_delta(volume: Volume) -> f32 {
        volume
            .buy_sell()
            .map(|(buy, sell)| f32::from(buy) - f32::from(sell))
            .unwrap_or(0.0)
    }

    fn datapoint_delta(dp: &KlineDataPoint) -> f32 {
        let footprint_delta: f32 = dp
            .footprint
            .trades
            .values()
            .map(|group| f32::from(group.delta_qty()))
            .sum();

        if dp.footprint.trades.is_empty() {
            Self::volume_delta(dp.kline.volume)
        } else {
            footprint_delta
        }
    }

    fn rebuild_cumulative(&mut self) {
        self.data.clear();

        let mut cumulative = 0.0;
        for (&x, &delta) in &self.delta {
            cumulative += delta;
            self.data.insert(x, CumulativeDeltaPoint { delta, cumulative });
        }

        self.clear_all_caches();
    }

    fn rebuild_from_deltas(&mut self, deltas: BTreeMap<u64, f32>) {
        self.delta = deltas;
        self.rebuild_cumulative();
    }
}

impl KlineIndicatorImpl for CumulativeDeltaIndicator {
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
        let deltas = match source {
            PlotData::TimeBased(timeseries) => timeseries
                .datapoints
                .iter()
                .map(|(&time, dp)| (time, Self::datapoint_delta(dp)))
                .collect(),
            PlotData::TickBased(tickseries) => tickseries
                .datapoints
                .iter()
                .enumerate()
                .map(|(idx, dp)| {
                    let delta = Self::volume_delta(dp.kline.volume);
                    (idx as u64, delta)
                })
                .collect(),
        };

        self.rebuild_from_deltas(deltas);
    }

    fn on_insert_klines(&mut self, klines: &[Kline]) {
        for kline in klines {
            self.delta.insert(kline.time, Self::kline_delta(kline));
        }
        self.rebuild_cumulative();
    }

    fn on_insert_trades(
        &mut self,
        _trades: &[Trade],
        _old_dp_len: usize,
        source: &PlotData<KlineDataPoint>,
    ) {
        self.rebuild_from_source(source);
    }

    fn on_ticksize_change(&mut self, source: &PlotData<KlineDataPoint>) {
        self.rebuild_from_source(source);
    }

    fn on_basis_change(&mut self, source: &PlotData<KlineDataPoint>) {
        self.rebuild_from_source(source);
    }
}
