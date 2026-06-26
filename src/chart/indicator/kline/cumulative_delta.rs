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
    indicator::{CumulativeDeltaSettings, KlineIndicatorConfig},
    kline::KlineDataPoint,
};
use data::util::format_with_commas;
use exchange::{Kline, Trade, unit::Qty};

use iced::widget::{center, text};

use std::collections::{BTreeMap, BTreeSet};
use std::ops::RangeInclusive;

#[derive(Debug, Clone, Copy, Default)]
struct CumulativeDeltaPoint {
    /// Buy volume - sell volume for this candle / tick bucket.
    delta: Qty,
    /// Running sum of delta from the oldest loaded datapoint to this datapoint.
    cumulative: Qty,
    /// Whether this point is considered trustworthy.  A point is
    /// reliable only when it has a directional predecessor within a
    /// run of at least `MIN_DIRECTIONAL_RUN` consecutive bars with
    /// non-zero delta.  The first bar of every qualifying run is
    /// excluded (it lacks a directional predecessor to anchor
    /// against), as are points outside any qualifying run.
    reliable: bool,
}

pub struct CumulativeDeltaIndicator {
    cache: Caches,
    /// Per-bucket delta. Stored separately so inserting/replacing older klines can
    /// rebuild the cumulative line without needing the full chart source.
    delta: BasisSeries<Qty>,
    data: BasisSeries<CumulativeDeltaPoint>,
    availability: IndicatorAvailability,
    settings: CumulativeDeltaSettings,
}

impl CumulativeDeltaIndicator {
    pub fn new(settings: CumulativeDeltaSettings) -> Self {
        Self {
            cache: Caches::default(),
            delta: BasisSeries::default(),
            data: BasisSeries::default(),
            availability: IndicatorAvailability::Unknown,
            settings,
        }
    }

    fn indicator_elem<'a>(
        &'a self,
        main_chart: &'a ViewState,
        data_labels_always_visible: bool,
        visible_range: RangeInclusive<u64>,
    ) -> iced::Element<'a, Message> {
        if let Some(message) = self.unavailable_message(main_chart, "CVD") {
            return center(text(message)).into();
        }

        let tooltip = |point: &CumulativeDeltaPoint, _next: Option<&CumulativeDeltaPoint>| {
            let cvd = format!("CVD: {}", format_with_commas(point.cumulative.to_f64()));
            let sign = if point.delta >= Qty::ZERO { "+" } else { "" };
            let delta = format!("Delta: {sign}{}", format_with_commas(point.delta.to_f64()));
            PlotTooltip::new(format!("{cvd}\n{delta}"))
        };

        let value_fn = |point: &CumulativeDeltaPoint| point.cumulative.to_f64() as f32;
        let line_color = self
            .settings
            .custom_color
            .filter(|_| self.settings.custom_color_enabled);

        let plot = LinePlot::new(value_fn)
            .stroke_width(self.settings.line_width.clamp(0.5, 4.0))
            .show_points(self.settings.show_points)
            .point_radius_factor(0.2)
            .padding(0.08)
            // Only treat bars as valid when they belong to a long enough
            // run of consecutive directional data
            .valid_when(|point: &CumulativeDeltaPoint| point.reliable)
            .invalid_point_message(format!(
                "CVD requires {}+ consecutive bars\nwith directional volume",
                self.settings.min_directional_run,
            ))
            .with_tooltip(tooltip);

        indicator_row(
            main_chart,
            &self.cache,
            data_labels_always_visible,
            plot.with_line_color(line_color),
            self.data.as_plot_series(),
            visible_range,
        )
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
        match &self.delta {
            BasisSeries::Time(deltas) => {
                let entries: Vec<_> = deltas.iter().collect();
                let reliable =
                    Self::reliable_indices(&entries, self.settings.min_directional_run.max(1));

                let mut cumulative = Qty::ZERO;
                let data: BTreeMap<_, _> = entries
                    .iter()
                    .enumerate()
                    .map(|(i, &(&time, &delta))| {
                        cumulative += delta;
                        (
                            time,
                            CumulativeDeltaPoint {
                                delta,
                                cumulative,
                                reliable: reliable[i],
                            },
                        )
                    })
                    .collect();

                self.data = BasisSeries::Time(data);
            }
            BasisSeries::Tick(deltas) => {
                let entries: Vec<_> = deltas.iter().collect();
                let reliable =
                    Self::reliable_indices(&entries, self.settings.min_directional_run.max(1));

                let mut cumulative = Qty::ZERO;
                let data: BTreeMap<_, _> = entries
                    .iter()
                    .enumerate()
                    .map(|(i, &(&idx, &delta))| {
                        cumulative += delta;
                        (
                            idx,
                            CumulativeDeltaPoint {
                                delta,
                                cumulative,
                                reliable: reliable[i],
                            },
                        )
                    })
                    .collect();

                self.data = BasisSeries::Tick(data);
            }
        }

        self.clear_all_caches();
    }

    /// Mark which positions in `entries` belong to a qualifying run
    /// (≥ `min_run` consecutive non-zero deltas, excluding the first
    /// bar of each run).
    fn reliable_indices<K>(entries: &[(&K, &Qty)], min_run: usize) -> Vec<bool> {
        let n = entries.len();
        let mut reliable = vec![false; n];
        let mut i = 0;
        while i < n {
            if *entries[i].1 != Qty::ZERO {
                let run_start = i;
                while i < n && *entries[i].1 != Qty::ZERO {
                    i += 1;
                }
                // Skip the first bar of every qualifying run — it lacks
                // a directional predecessor to anchor its delta against.
                if i - run_start >= min_run {
                    for slot in reliable.iter_mut().take(i).skip(run_start + 1) {
                        *slot = true;
                    }
                }
            } else {
                i += 1;
            }
        }
        reliable
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
        data_labels_always_visible: bool,
        visible_range: RangeInclusive<u64>,
    ) -> iced::Element<'a, Message> {
        self.indicator_elem(chart, data_labels_always_visible, visible_range)
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
                    .map(|(&time, dp)| (time, dp.volume_delta()))
                    .collect()
            },
            |tickseries| {
                tickseries
                    .datapoints
                    .iter()
                    .enumerate()
                    .map(|(idx, dp)| (idx as u64, dp.volume_delta()))
                    .collect()
            },
        );

        let (deltas, has_points, has_directional) = match source {
            PlotData::TimeBased(timeseries) => {
                let has_points = !timeseries.datapoints.is_empty();
                let has_directional = timeseries.datapoints.values().any(|dp| dp.is_directional());

                (deltas, has_points, has_directional)
            }
            PlotData::TickBased(tickseries) => {
                let has_points = !tickseries.datapoints.is_empty();
                let has_directional = tickseries.datapoints.iter().any(|dp| dp.is_directional());

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
                    (dp.volume_delta(), dp.is_directional())
                } else {
                    (kline.volume.delta(), kline.volume.is_directional())
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
                        deltas.insert(time, dp.volume_delta());
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
                    deltas.insert(idx as u64, dp.volume_delta());
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

    fn apply_config(&mut self, config: &KlineIndicatorConfig, source: &PlotData<KlineDataPoint>) {
        if let KlineIndicatorConfig::CumulativeDelta(settings) = config
            && self.settings != *settings
        {
            self.settings = *settings;
            self.rebuild_from_source(source);
        }
    }
}
