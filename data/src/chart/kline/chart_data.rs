use super::indicators::{
    IndicatorHost, IndicatorInputCoverage, IndicatorInputSource, OpenInterestSeries,
};
use super::series::KlineSeries;
use super::{
    IndicatorInstanceId, IndicatorInstanceStore, IndicatorUpdateScope, KlineIndexChange,
    KlineStudy, KlineUpdate, LineStrokeStyle, MIN_LINE_WIDTH, SeriesColor, StudyDataSource,
    ThemeAccent,
};
use crate::chart::Basis;
use exchange::{Kline, TickMultiplier, TickerInfo, Timeframe, Trade, unit::PriceStep};
use indicator_engine::IndicatorFrame;
use rustc_hash::FxHashMap;
use std::sync::Arc;

pub struct LiveIndicatorUpdate {
    pub committed_bar: bool,
    pub rebuild_indicators: bool,
    pub closed_trade_correction: bool,
    pub coverage_frames: Vec<IndicatorFrame>,
    pub index_change: KlineIndexChange,
}

pub enum KlineDataChange {
    Unchanged,
    HistoricalKlines {
        update: KlineUpdate,
    },
    LiveKline {
        update: LiveIndicatorUpdate,
    },
    LiveTrades {
        update: LiveIndicatorUpdate,
    },
    HistoricalTrades,
    HistoricalOpenInterest {
        update: HistoricalOpenInterestUpdate,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoricalOpenInterestUpdate {
    Rejected,
    NoData { has_consumer: bool },
    Ignored,
    Applied,
}

impl HistoricalOpenInterestUpdate {
    pub fn changed(self) -> bool {
        matches!(self, Self::NoData { has_consumer: true } | Self::Applied)
    }
}

pub struct KlineData {
    series: Vec<KlineSeries>,
    indicator_host: IndicatorHost,
    base_ticker: TickerInfo,
    series_index: FxHashMap<TickerInfo, usize>,
    footprint_tick_multiplier: TickMultiplier,
}

impl KlineData {
    pub fn new(base_ticker: TickerInfo, footprint_tick_multiplier: TickMultiplier) -> Self {
        Self {
            series: Vec::new(),
            indicator_host: IndicatorHost::default(),
            base_ticker,
            series_index: FxHashMap::default(),
            footprint_tick_multiplier,
        }
    }

    pub fn base_ticker(&self) -> TickerInfo {
        self.base_ticker
    }

    pub fn selected_tickers(&self) -> Vec<TickerInfo> {
        self.series
            .iter()
            .map(|series| series.ticker_info)
            .collect()
    }

    pub fn series(&self) -> &[KlineSeries] {
        &self.series
    }

    pub fn indicator_host(&self) -> &IndicatorHost {
        &self.indicator_host
    }

    pub fn merge_indicator_host(
        &mut self,
        rebuilt: IndicatorHost,
        replaced_ids: &[IndicatorInstanceId],
        active_ids: &[IndicatorInstanceId],
        indicators: &IndicatorInstanceStore,
    ) {
        self.indicator_host
            .merge_rebuilt(rebuilt, replaced_ids, active_ids, indicators);
    }

    pub fn remap_indicator_outputs(&mut self, change: KlineIndexChange) {
        match change {
            KlineIndexChange::None => {}
            KlineIndexChange::Prefix(prefix) => self.indicator_host.remap_outputs_prefix(prefix),
            KlineIndexChange::Explicit(previous_indices) => {
                self.indicator_host.remap_outputs(&previous_indices)
            }
        }
    }

    pub fn dependent_indicator_ids(
        &self,
        roots: &[IndicatorInstanceId],
    ) -> Vec<IndicatorInstanceId> {
        self.indicator_host.dependent_ids(roots)
    }

    pub fn calculation_indicator_ids(
        &self,
        roots: &[IndicatorInstanceId],
    ) -> Vec<IndicatorInstanceId> {
        self.indicator_host.calculation_ids(roots)
    }

    pub fn has_indicator_entries(&self, ids: &[IndicatorInstanceId]) -> bool {
        self.indicator_host.has_entries(ids)
    }

    pub fn validate_indicator_source_binding(
        &self,
        active_indicators: &[IndicatorInstanceId],
        indicators: &IndicatorInstanceStore,
        id: IndicatorInstanceId,
        input_id: &str,
    ) -> Result<(), String> {
        self.indicator_host
            .validate_source_binding(active_indicators, indicators, id, input_id)
    }

    pub fn footprint_tick_multiplier(&self) -> TickMultiplier {
        self.footprint_tick_multiplier
    }

    pub fn base_series(&self) -> Option<&KlineSeries> {
        let idx = self.series_index.get(&self.base_ticker).copied()?;
        self.series.get(idx)
    }

    pub fn base_series_mut(&mut self) -> Option<&mut KlineSeries> {
        let idx = self.series_index.get(&self.base_ticker).copied()?;
        self.series.get_mut(idx)
    }

    pub fn configure_recurring_profile_intervals(&mut self, intervals: &[(Timeframe, PriceStep)]) {
        if let Some(base_series) = self.base_series_mut() {
            base_series.configure_recurring_profile_intervals(intervals);
        }
    }

    pub fn configure_big_trades_threshold(&mut self, threshold: Option<u64>) {
        if let Some(base_series) = self.base_series_mut() {
            base_series.configure_big_trades_threshold(threshold);
        }
    }

    pub fn set_footprint_tick_multiplier(&mut self, multiplier: TickMultiplier) -> bool {
        if self.footprint_tick_multiplier == multiplier {
            return false;
        }
        self.footprint_tick_multiplier = multiplier;
        true
    }

    pub fn set_series_color(&mut self, ticker_info: TickerInfo, color: SeriesColor) -> bool {
        let Some(idx) = self.series_index.get(&ticker_info).copied() else {
            return false;
        };

        self.series[idx].set_series_color(color)
    }

    pub fn set_series_line_width(&mut self, ticker_info: TickerInfo, width: u8) -> bool {
        let Some(idx) = self.series_index.get(&ticker_info).copied() else {
            return false;
        };

        self.series[idx].set_line_width(width)
    }

    pub fn set_series_stroke_style(
        &mut self,
        ticker_info: TickerInfo,
        stroke_style: LineStrokeStyle,
    ) -> bool {
        let Some(idx) = self.series_index.get(&ticker_info).copied() else {
            return false;
        };

        self.series[idx].set_stroke_style(stroke_style)
    }

    pub fn reset_series_style(&mut self, ticker_info: TickerInfo) -> bool {
        let Some(index) = self.series_index.get(&ticker_info).copied() else {
            return false;
        };

        let color = self.default_series_color(index);
        let series = &mut self.series[index];
        let color_changed = series.set_series_color(color);
        let width_changed = series.set_line_width(MIN_LINE_WIDTH);
        let stroke_changed = series.set_stroke_style(LineStrokeStyle::Solid);

        color_changed || width_changed || stroke_changed
    }

    fn default_series_color(&self, index: usize) -> SeriesColor {
        if self.series[index].ticker_info == self.base_ticker {
            return SeriesColor::default();
        }

        SeriesColor::with_accent(self.next_unused_accent(Some(index)))
    }

    pub fn add_ticker(&mut self, ticker_info: TickerInfo, basis: Basis) -> bool {
        if self.series_index.contains_key(&ticker_info) {
            return false;
        }

        let idx = self.series.len();
        let tick_size = self
            .footprint_tick_multiplier
            .multiply_with_min_tick_step(ticker_info);
        let color = if ticker_info == self.base_ticker {
            SeriesColor::default()
        } else {
            SeriesColor::with_accent(self.next_unused_accent(None))
        };
        let series = KlineSeries::new_with_basis(ticker_info, color, basis, tick_size);

        self.series.push(series);
        self.series_index.insert(ticker_info, idx);
        true
    }

    /// The first accent no series uses, ignoring `excluding` when a single series is
    /// being recolored.
    fn next_unused_accent(&self, excluding: Option<usize>) -> ThemeAccent {
        let is_used = |accent: ThemeAccent| {
            self.series.iter().enumerate().any(|(index, series)| {
                Some(index) != excluding && series.color().accent_value() == Some(accent)
            })
        };

        ThemeAccent::ALL
            .into_iter()
            .find(|accent| !is_used(*accent))
            .unwrap_or(ThemeAccent::One)
    }

    pub fn remove_ticker(&mut self, ticker_info: TickerInfo) -> bool {
        let Some(idx) = self.series_index.remove(&ticker_info) else {
            return false;
        };

        self.series.remove(idx);
        self.rebuild_series_index();
        true
    }

    fn rebuild_series_index(&mut self) {
        self.series_index.clear();
        for (idx, series) in self.series.iter().enumerate() {
            self.series_index.insert(series.ticker_info, idx);
        }
    }

    pub fn reset_for_basis(&mut self, basis: Basis, preserve_raw_trades: bool) {
        for series in &mut self.series {
            series.set_aggregation_basis(basis, preserve_raw_trades);
        }
    }

    pub fn configure_series_for_basis(&mut self, basis: Basis) {
        for series in &mut self.series {
            let tick_size = self
                .footprint_tick_multiplier
                .multiply_with_min_tick_step(series.ticker_info);
            if series.basis() != basis {
                series.set_aggregation_basis(basis, true);
            }
            series.set_footprint_tick_size(tick_size);
        }
    }

    pub fn configure_indicators(
        &mut self,
        indicators: &IndicatorInstanceStore,
        active_indicators: &[IndicatorInstanceId],
    ) -> Vec<IndicatorInstanceId> {
        self.indicator_host
            .set_active(active_indicators, indicators)
    }

    pub fn recalculate_indicators(
        &mut self,
        indicators: &IndicatorInstanceStore,
        indicator_ids: &[IndicatorInstanceId],
        scope: IndicatorUpdateScope,
        trade_data_enabled: bool,
    ) -> Arc<[IndicatorFrame]> {
        let scope = if scope.is_full() || !self.indicator_host.has_entries(indicator_ids) {
            IndicatorUpdateScope::Full
        } else {
            scope
        };
        let full_rebuild = scope.is_full();
        let frames = match scope {
            IndicatorUpdateScope::Full => {
                let frames = self.committed_indicator_frames(trade_data_enabled);
                self.indicator_host
                    .recalculate_selected(indicator_ids, indicators, &frames);
                frames
            }
            IndicatorUpdateScope::From(start) => self
                .evaluate_closed_append_from(start, indicator_ids, trade_data_enabled)
                .into(),
        };
        if let Some(frame) = self.latest_indicator_frame(trade_data_enabled) {
            let preview_ids = if full_rebuild {
                self.indicator_host.dependent_ids(indicator_ids)
            } else {
                indicator_ids.to_vec()
            };
            self.indicator_host
                .preview_open_selected(&preview_ids, &frame);
        }
        frames
    }

    pub fn committed_indicator_frames(&self, trade_data_enabled: bool) -> Arc<[IndicatorFrame]> {
        let Some(base_series) = self.base_series() else {
            return Arc::from([]);
        };
        let frames = base_series
            .committed_points()
            .map(|point| self.indicator_frame(point.index, point.kline.time, trade_data_enabled))
            .collect::<Vec<_>>();
        frames.into()
    }

    pub fn latest_indicator_frame(&self, trade_data_enabled: bool) -> Option<IndicatorFrame> {
        let base_series = self.base_series()?;
        let point = base_series.latest_point()?;
        Some(self.indicator_frame(point.index, point.kline.time, trade_data_enabled))
    }

    pub fn indicator_input_coverage_for_frames(
        &self,
        committed_frames: &[IndicatorFrame],
        trade_data_enabled: bool,
    ) -> IndicatorInputCoverage {
        let mut frames = committed_frames.to_vec();
        if let Some(latest) = self.latest_indicator_frame(trade_data_enabled) {
            if let Some(existing) = frames.iter_mut().find(|frame| frame.index == latest.index) {
                *existing = latest;
            } else {
                frames.push(latest);
            }
        }
        IndicatorInputCoverage::from_frames(&frames)
    }

    pub fn preview_indicator_open_for(
        &mut self,
        ids: &[IndicatorInstanceId],
        trade_data_enabled: bool,
    ) -> bool {
        let Some(frame) = self.latest_indicator_frame(trade_data_enabled) else {
            return false;
        };
        self.indicator_host.preview_open_selected(ids, &frame);
        true
    }

    fn evaluate_closed_previous_frame(
        &mut self,
        time: Option<exchange::UnixMs>,
        indicator_ids: &[IndicatorInstanceId],
        trade_data_enabled: bool,
    ) -> Option<IndicatorFrame> {
        let time = time?;
        let base_series = self.base_series()?;

        let committed_len = base_series.point_count().saturating_sub(1);
        let point = base_series
            .points()
            .take(committed_len)
            .next_back()
            .filter(|point| point.kline.time == time)?;

        let frame = self.indicator_frame(point.index, point.kline.time, trade_data_enabled);

        self.indicator_host
            .evaluate_closed_selected(indicator_ids, &frame);
        Some(frame)
    }

    fn evaluate_closed_append_from(
        &mut self,
        start: usize,
        indicator_ids: &[IndicatorInstanceId],
        trade_data_enabled: bool,
    ) -> Vec<IndicatorFrame> {
        let points = self
            .base_series()
            .map(|series| {
                series
                    .committed_points()
                    .skip(start)
                    .map(|point| (point.index, point.kline.time))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut frames = Vec::with_capacity(points.len());
        for (index, time) in points {
            let frame = self.indicator_frame(index, time, trade_data_enabled);
            self.indicator_host
                .evaluate_closed_selected(indicator_ids, &frame);
            frames.push(frame);
        }
        frames
    }

    fn indicator_frame(
        &self,
        index: usize,
        time: exchange::UnixMs,
        trade_data_enabled: bool,
    ) -> IndicatorFrame {
        let base_series = self
            .base_series()
            .expect("indicator frame requires a base series");
        // Open interest is an as-of input; plot offsets are presentation metadata.
        let open_interest = base_series
            .open_interest()
            .and_then(|series| series.latest_at_or_before(time));
        base_series.indicator_frame(index, time, open_interest, trade_data_enabled)
    }

    fn closed_trade_correction_start(
        &self,
        trades: &[Trade],
        timeframe: Timeframe,
    ) -> Option<usize> {
        let earliest_trade_bar = trades
            .iter()
            .map(|trade| trade.time.floor_to(timeframe))
            .min()?;
        let base_series = self.base_series()?;
        let latest_closed_time = base_series
            .points()
            .rev()
            .nth(1)
            .map(|point| point.kline.time)?;
        if earliest_trade_bar > latest_closed_time {
            return None;
        }

        Some(
            base_series
                .committed_points()
                .find(|point| point.kline.time >= earliest_trade_bar)
                .map_or(0, |point| point.index),
        )
    }

    pub fn has_closed_trade_correction(&self, trades: &[Trade], basis: Basis) -> bool {
        match basis {
            Basis::Time(timeframe) => self
                .closed_trade_correction_start(trades, timeframe)
                .is_some(),
            Basis::Tick(_) => false,
        }
    }

    fn committed_indicator_frames_from(
        &self,
        start: usize,
        trade_data_enabled: bool,
    ) -> Vec<IndicatorFrame> {
        self.base_series()
            .map(|series| {
                series
                    .committed_points()
                    .skip(start)
                    .map(|point| {
                        self.indicator_frame(point.index, point.kline.time, trade_data_enabled)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn reconfigure_indicator(
        &mut self,
        id: IndicatorInstanceId,
        indicators: &IndicatorInstanceStore,
    ) {
        let Some(instance) = indicators.get(id) else {
            return;
        };
        self.indicator_host.reconfigure(instance, indicators);
    }

    pub fn insert_historical_klines(
        &mut self,
        ticker_info: TickerInfo,
        klines: &[Kline],
        basis: Basis,
    ) -> KlineDataChange {
        let Some(idx) = self.series_index.get(&ticker_info).copied() else {
            return KlineDataChange::Unchanged;
        };

        if klines.is_empty() {
            return KlineDataChange::Unchanged;
        }
        self.series[idx]
            .insert_klines(klines, basis)
            .map_or(KlineDataChange::Unchanged, |update| {
                KlineDataChange::HistoricalKlines { update }
            })
    }

    pub fn update_live_kline(
        &mut self,
        ticker_info: &TickerInfo,
        kline: &Kline,
        basis: Basis,
        indicator_ids: &[IndicatorInstanceId],
        trade_data_enabled: bool,
    ) -> KlineDataChange {
        let Some(idx) = self.series_index.get(ticker_info).copied() else {
            return KlineDataChange::Unchanged;
        };
        let previous_time = self.series[idx]
            .latest_point()
            .map(|point| point.kline.time);
        let next_time = kline.time;
        let is_base = *ticker_info == self.base_ticker;
        let starts_new_bar = previous_time.is_some_and(|time| next_time > time);
        let Some(update) = self.series[idx].update_latest_kline(kline, basis) else {
            return KlineDataChange::Unchanged;
        };
        let rebuild_indicators =
            is_base && update.scope.is_full() && previous_time.is_some_and(|time| next_time < time);
        let coverage_frames = if starts_new_bar && is_base {
            let committed_frame = self.evaluate_closed_previous_frame(
                previous_time,
                indicator_ids,
                trade_data_enabled,
            );
            self.preview_indicator_open_for(indicator_ids, trade_data_enabled);
            committed_frame.into_iter().collect()
        } else {
            Vec::new()
        };
        KlineDataChange::LiveKline {
            update: LiveIndicatorUpdate {
                committed_bar: starts_new_bar && is_base,
                rebuild_indicators,
                closed_trade_correction: false,
                coverage_frames,
                index_change: update.index_change,
            },
        }
    }

    pub fn insert_live_trades(
        &mut self,
        ticker_info: &TickerInfo,
        trades: &[Trade],
        basis: Basis,
        indicator_ids: &[IndicatorInstanceId],
        trade_data_enabled: bool,
    ) -> KlineDataChange {
        if trades.is_empty() {
            return KlineDataChange::Unchanged;
        }
        let Some(index) = self.series_index.get(ticker_info).copied() else {
            return KlineDataChange::Unchanged;
        };

        let is_base = *ticker_info == self.base_ticker;
        let correction_start = is_base
            .then(|| match basis {
                Basis::Time(timeframe) => self.closed_trade_correction_start(trades, timeframe),
                Basis::Tick(_) => None,
            })
            .flatten();
        let previous_committed_len = if is_base {
            self.base_series()
                .map(|series| series.point_count().saturating_sub(1))
                .unwrap_or_default()
        } else {
            0
        };
        let scope = match basis {
            Basis::Tick(_) => self.series[index].insert_tick_trades(trades),
            Basis::Time(timeframe) => self.series[index].insert_time_trades(trades, timeframe),
        };
        let committed_len = if matches!(basis, Basis::Tick(_)) && is_base {
            self.base_series()
                .map(|series| series.point_count().saturating_sub(1))
                .unwrap_or_default()
        } else {
            previous_committed_len
        };
        let newly_committed_frames = if committed_len > previous_committed_len {
            let frames = self.evaluate_closed_append_from(
                previous_committed_len,
                indicator_ids,
                trade_data_enabled,
            );
            self.preview_indicator_open_for(indicator_ids, trade_data_enabled);
            frames
        } else {
            Vec::new()
        };
        let coverage_frames = correction_start
            .map(|start| self.committed_indicator_frames_from(start, trade_data_enabled))
            .unwrap_or(newly_committed_frames);
        KlineDataChange::LiveTrades {
            update: LiveIndicatorUpdate {
                committed_bar: committed_len > previous_committed_len,
                rebuild_indicators: is_base && scope.is_full(),
                closed_trade_correction: correction_start.is_some(),
                coverage_frames,
                index_change: KlineIndexChange::None,
            },
        }
    }

    pub fn insert_historical_trades(
        &mut self,
        ticker_info: TickerInfo,
        trades: &[Trade],
        is_batches_done: bool,
        basis: Basis,
    ) -> KlineDataChange {
        let Basis::Time(timeframe) = basis else {
            return KlineDataChange::Unchanged;
        };

        if !trades.is_empty() {
            let _ = self
                .series_index
                .get(&ticker_info)
                .copied()
                .and_then(|index| self.series.get_mut(index))
                .map(|series| series.insert_time_trades(trades, timeframe));
        }
        if trades.is_empty() && !is_batches_done {
            KlineDataChange::Unchanged
        } else {
            KlineDataChange::HistoricalTrades
        }
    }

    pub fn insert_historical_open_interest(
        &mut self,
        ticker_info: TickerInfo,
        series: &OpenInterestSeries,
    ) -> KlineDataChange {
        if ticker_info != self.base_ticker {
            return KlineDataChange::HistoricalOpenInterest {
                update: HistoricalOpenInterestUpdate::Rejected,
            };
        }
        let Some(idx) = self.series_index.get(&ticker_info).copied() else {
            return KlineDataChange::HistoricalOpenInterest {
                update: HistoricalOpenInterestUpdate::Rejected,
            };
        };

        let has_open_interest_consumer = self
            .indicator_host
            .has_input_consumer(IndicatorInputSource::OpenInterest);
        if series.is_empty() {
            return KlineDataChange::HistoricalOpenInterest {
                update: HistoricalOpenInterestUpdate::NoData {
                    has_consumer: has_open_interest_consumer,
                },
            };
        }
        if !has_open_interest_consumer {
            return KlineDataChange::HistoricalOpenInterest {
                update: HistoricalOpenInterestUpdate::Ignored,
            };
        }
        self.series[idx].insert_open_interest(series);
        KlineDataChange::HistoricalOpenInterest {
            update: HistoricalOpenInterestUpdate::Applied,
        }
    }

    /// Resolves the row tick size a profile study aggregates at.
    ///
    /// A study without an explicit tick multiplier inherits the pane's
    /// footprint multiplier. A study can only choose a tick multiplier that is
    /// at least as coarse as the footprint multiplier, so the resolved step is
    /// never finer than the footprint step.
    pub fn effective_study_tick_size(&self, study: KlineStudy) -> PriceStep {
        let footprint = self
            .footprint_tick_multiplier
            .multiply_with_min_tick_step(self.base_ticker);
        study.effective_profile_tick_size(self.base_ticker, footprint)
    }

    pub fn recurring_profile_cache_requirements(
        &self,
        studies: &[KlineStudy],
        basis: Basis,
    ) -> Vec<(Timeframe, PriceStep)> {
        let mut recurring_intervals = studies
            .iter()
            .filter(|study| study.source() == StudyDataSource::Trades)
            .filter_map(|study| {
                study
                    .recurring_profile_interval()
                    .filter(|_| study.is_interval_available(basis))
                    .map(|interval| (interval, self.effective_study_tick_size(*study)))
            })
            .collect::<Vec<_>>();
        recurring_intervals.sort_unstable();
        recurring_intervals.dedup();
        recurring_intervals
    }
}
