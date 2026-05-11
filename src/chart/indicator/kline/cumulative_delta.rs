use crate::chart::{
    Caches, Message, ViewState,
    indicator::{
        indicator_row,
        kline::{
            AvailabilityCause, BasisSeries, BasisSeriesExt, IndicatorAvailability,
            KlineIndicatorImpl,
        },
        plot::{PlotTooltip, line::LinePlot},
    },
};

use data::chart::{
    PlotData,
    kline::{KlineDataPoint, KlineTrades},
};
use data::util::format_with_commas;
use exchange::{Kline, Trade, Volume, unit::Qty};

use iced::widget::{center, text};

use std::collections::BTreeSet;
use std::ops::RangeInclusive;

#[derive(Debug, Clone, Copy, Default)]
pub struct CumulativeDeltaPoint {
    /// Buy volume - sell volume for this candle / tick bucket.
    pub delta: Qty,
    /// Running sum of delta from the oldest loaded datapoint to this datapoint.
    pub cumulative: Qty,
}

pub struct CumulativeDeltaIndicator {
    cache: Caches,
    /// Per-bucket delta. Stored separately so inserting/replacing older klines can
    /// rebuild the cumulative line without needing the full chart source.
    delta: BasisSeries<Qty>,
    data: BasisSeries<CumulativeDeltaPoint>,
    availability: IndicatorAvailability,
}

impl CumulativeDeltaIndicator {
    pub fn new() -> Self {
        Self {
            cache: Caches::default(),
            delta: BasisSeries::default(),
            data: BasisSeries::default(),
            availability: IndicatorAvailability::Unknown,
        }
    }

    fn indicator_elem<'a>(
        &'a self,
        main_chart: &'a ViewState,
        visible_range: RangeInclusive<u64>,
    ) -> iced::Element<'a, Message> {
        if let Some(message) = self.unavailable_message(main_chart, "CVD") {
            return center(text(message)).into();
        }

        let tooltip = |point: &CumulativeDeltaPoint, _next: Option<&CumulativeDeltaPoint>| {
            let cvd = format!(
                "CVD: {}",
                format_with_commas(point.cumulative.to_f32_lossy())
            );
            let sign = if point.delta >= Qty::ZERO { "+" } else { "" };
            let delta = format!(
                "Delta: {sign}{}",
                format_with_commas(point.delta.to_f32_lossy())
            );
            PlotTooltip::new(format!("{cvd}\n{delta}"))
        };

        let value_fn = |point: &CumulativeDeltaPoint| point.cumulative.to_f32_lossy();

        let plot = LinePlot::new(value_fn)
            .stroke_width(1.0)
            .show_points(true)
            .point_radius_factor(0.2)
            .padding(0.08)
            .with_tooltip(tooltip);

        indicator_row(
            main_chart,
            &self.cache,
            plot,
            self.data.as_plot_series(),
            visible_range,
        )
    }

    fn has_directional_volume(volume: Volume) -> bool {
        volume.buy_sell().is_some()
    }

    fn volume_delta(volume: Volume) -> Qty {
        volume
            .buy_sell()
            .map(|(buy, sell)| buy - sell)
            .unwrap_or(Qty::ZERO)
    }

    fn delta_from_parts(footprint: &KlineTrades, volume: Volume) -> Qty {
        if footprint.trades.is_empty() {
            Self::volume_delta(volume)
        } else {
            footprint
                .trades
                .values()
                .fold(Qty::ZERO, |acc, group| acc + group.delta_qty())
        }
    }

    fn is_directional_parts(footprint: &KlineTrades, volume: Volume) -> bool {
        !footprint.trades.is_empty() || Self::has_directional_volume(volume)
    }

    fn datapoint_delta(dp: &KlineDataPoint) -> Qty {
        Self::delta_from_parts(&dp.footprint, dp.kline.volume)
    }

    fn is_datapoint_directional(dp: &KlineDataPoint) -> bool {
        Self::is_directional_parts(&dp.footprint, dp.kline.volume)
    }

    fn set_availability(&mut self, has_points: bool, has_directional: bool) {
        self.availability = if !has_points {
            IndicatorAvailability::Unknown
        } else if has_directional {
            IndicatorAvailability::Available
        } else {
            IndicatorAvailability::Unavailable(AvailabilityCause::TradeData)
        };
    }

    fn rebuild_cumulative(&mut self) {
        let mut cumulative = Qty::ZERO;
        self.data = self.delta.map(|delta| {
            cumulative += *delta;
            CumulativeDeltaPoint {
                delta: *delta,
                cumulative,
            }
        });

        self.clear_all_caches();
    }

    fn rebuild_from_deltas(&mut self, deltas: BasisSeries<Qty>) {
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

    fn availability(&self, _chart: &ViewState) -> IndicatorAvailability {
        self.availability.clone()
    }

    fn rebuild_from_source(&mut self, source: &PlotData<KlineDataPoint>) {
        let deltas = source.map_basis_series(
            |timeseries| {
                timeseries
                    .datapoints
                    .iter()
                    .map(|(&time, dp)| (time, Self::datapoint_delta(dp)))
                    .collect()
            },
            |tickseries| {
                tickseries
                    .datapoints
                    .iter()
                    .enumerate()
                    .map(|(idx, dp)| {
                        (
                            idx as u64,
                            Self::delta_from_parts(&dp.footprint, dp.kline.volume),
                        )
                    })
                    .collect()
            },
        );

        let (deltas, has_points, has_directional) = match source {
            PlotData::TimeBased(timeseries) => {
                let has_points = !timeseries.datapoints.is_empty();
                let has_directional = timeseries
                    .datapoints
                    .values()
                    .any(Self::is_datapoint_directional);

                (deltas, has_points, has_directional)
            }
            PlotData::TickBased(tickseries) => {
                let has_points = !tickseries.datapoints.is_empty();
                let has_directional = tickseries
                    .datapoints
                    .iter()
                    .any(|dp| Self::is_directional_parts(&dp.footprint, dp.kline.volume));

                (deltas, has_points, has_directional)
            }
        };

        self.set_availability(has_points, has_directional);

        self.rebuild_from_deltas(deltas);
    }

    fn on_insert_klines(&mut self, klines: &[Kline], source: &PlotData<KlineDataPoint>) {
        let mut has_directional = false;

        let has_data = {
            let PlotData::TimeBased(timeseries) = source else {
                return;
            };

            let Some(deltas) = self.delta.time_mut() else {
                return;
            };

            for kline in klines {
                let (delta, directional) = if let Some(dp) = timeseries.datapoints.get(&kline.time)
                {
                    (
                        Self::datapoint_delta(dp),
                        Self::is_datapoint_directional(dp),
                    )
                } else {
                    (
                        Self::volume_delta(kline.volume),
                        Self::has_directional_volume(kline.volume),
                    )
                };

                deltas.insert(kline.time, delta);
                has_directional |= directional;
            }

            !deltas.is_empty()
        };

        if has_directional {
            self.availability = IndicatorAvailability::Available;
        }

        if self.availability == IndicatorAvailability::Unknown && has_data {
            self.availability = IndicatorAvailability::Unavailable(AvailabilityCause::TradeData);
        }

        self.rebuild_cumulative();
    }

    fn on_insert_trades(
        &mut self,
        trades: &[Trade],
        old_dp_len: usize,
        source: &PlotData<KlineDataPoint>,
    ) {
        let mut touched = false;

        match source {
            PlotData::TimeBased(timeseries) => {
                if trades.is_empty() {
                    return;
                }

                let Some(deltas) = self.delta.time_mut() else {
                    return;
                };

                let mut touched_times = BTreeSet::new();

                for trade in trades {
                    let rounded_time = trade.time.floor_to(timeseries.interval);
                    touched_times.insert(rounded_time);
                }

                for time in touched_times {
                    if let Some(dp) = timeseries.datapoints.get(&time) {
                        deltas.insert(time, Self::datapoint_delta(dp));
                        touched = true;
                    }
                }
            }
            PlotData::TickBased(tickseries) => {
                let Some(deltas) = self.delta.tick_mut() else {
                    return;
                };

                let start_idx = old_dp_len.saturating_sub(1);

                for (idx, dp) in tickseries.datapoints.iter().enumerate().skip(start_idx) {
                    deltas.insert(
                        idx as u64,
                        Self::delta_from_parts(&dp.footprint, dp.kline.volume),
                    );
                    touched = true;
                }
            }
        }

        if touched {
            self.availability = IndicatorAvailability::Available;
            self.rebuild_cumulative();
        }
    }

    fn on_ticksize_change(&mut self, source: &PlotData<KlineDataPoint>) {
        self.rebuild_from_source(source);
    }

    fn on_basis_change(&mut self, source: &PlotData<KlineDataPoint>) {
        self.rebuild_from_source(source);
    }
}
