use super::KlineSeries;
use super::fetch::FetchCoordinator;
use data::chart::Basis;
use data::chart::kline_v2::{
    FootprintPocConfig, IndicatorInput, IndicatorInstanceId, IndicatorInstanceStore, KlineStudy,
    LineStrokeStyle, MIN_LINE_WIDTH, SeriesColor, ThemeAccent,
};
use exchange::{
    Kline, OpenInterest, TickMultiplier, TickerInfo, Timeframe, Trade, adapter::StreamKind,
    unit::PriceStep,
};
use rustc_hash::FxHashMap;

pub enum DataUpdate<'a> {
    HistoricalKlines {
        req_id: Option<uuid::Uuid>,
        ticker_info: TickerInfo,
        klines: &'a [Kline],
    },
    LiveKline {
        ticker_info: &'a TickerInfo,
        kline: &'a Kline,
    },
    LiveTrades {
        ticker_info: &'a TickerInfo,
        trades: &'a [Trade],
    },
    HistoricalTrades {
        trades: &'a [Trade],
        is_batches_done: bool,
        req_id: Option<uuid::Uuid>,
    },
    HistoricalOpenInterest {
        req_id: Option<uuid::Uuid>,
        ticker_info: TickerInfo,
        data: &'a [OpenInterest],
    },
}

pub struct KlineData {
    pub series: Vec<KlineSeries>,
    base_ticker: TickerInfo,
    selected_tickers: Vec<TickerInfo>,
    series_index: FxHashMap<TickerInfo, usize>,
    footprint_tick_multiplier: TickMultiplier,
}

impl KlineData {
    pub fn new(base_ticker: TickerInfo, footprint_tick_multiplier: TickMultiplier) -> Self {
        Self {
            series: Vec::new(),
            base_ticker,
            selected_tickers: Vec::new(),
            series_index: FxHashMap::default(),
            footprint_tick_multiplier,
        }
    }

    pub fn base_ticker(&self) -> TickerInfo {
        self.base_ticker
    }

    pub fn selected_tickers(&self) -> &[TickerInfo] {
        &self.selected_tickers
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

    pub fn configure_footprint_poc_cache(&mut self, enabled: bool) {
        if let Some(base_series) = self.base_series_mut() {
            base_series.configure_footprint_poc_cache(enabled);
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

    pub fn reset_series_styles(&mut self) -> bool {
        let mut changed = false;
        let mut comparison_index = 0;

        for series in &mut self.series {
            let color = if series.ticker_info == self.base_ticker {
                SeriesColor::default()
            } else {
                let accent = ThemeAccent::ALL
                    .get(comparison_index)
                    .copied()
                    .unwrap_or(ThemeAccent::One);
                comparison_index += 1;
                SeriesColor::with_accent(accent)
            };

            changed |= series.set_series_color(color);
            changed |= series.set_line_width(MIN_LINE_WIDTH);
            changed |= series.set_stroke_style(LineStrokeStyle::Solid);
        }

        changed
    }

    pub fn add_ticker(
        &mut self,
        ticker_info: TickerInfo,
        basis: Basis,
        indicators: &IndicatorInstanceStore,
        active_indicators: &[IndicatorInstanceId],
    ) -> bool {
        if self.selected_tickers.contains(&ticker_info) {
            return false;
        }

        let idx = self.series.len();
        let mut series = if ticker_info == self.base_ticker {
            KlineSeries::new_base(ticker_info)
        } else {
            KlineSeries::new(
                ticker_info,
                SeriesColor::with_accent(self.next_unused_accent()),
            )
        };
        match basis {
            Basis::Time(timeframe) => series.rebuild_time_footprints(timeframe),
            Basis::Tick(interval) => {
                let tick_size = self
                    .footprint_tick_multiplier
                    .multiply_with_min_tick_step(series.ticker_info);
                series.set_footprint_tick_size(tick_size);
                series.enable_tick_aggregation(interval, tick_size);
            }
        }
        series.configure_indicators(indicators, basis, active_indicators);

        self.series.push(series);
        self.series_index.insert(ticker_info, idx);
        self.selected_tickers.push(ticker_info);
        true
    }

    fn next_unused_accent(&self) -> ThemeAccent {
        ThemeAccent::ALL
            .into_iter()
            .find(|accent| {
                !self
                    .series
                    .iter()
                    .any(|series| series.color().accent_value() == Some(*accent))
            })
            .unwrap_or(ThemeAccent::One)
    }

    pub fn remove_ticker(&mut self, ticker_info: TickerInfo) -> bool {
        let Some(idx) = self.series_index.remove(&ticker_info) else {
            return false;
        };

        self.series.remove(idx);
        self.selected_tickers
            .retain(|ticker| *ticker != ticker_info);
        self.rebuild_series_index();
        true
    }

    pub fn reset_for_basis(
        &mut self,
        basis: Basis,
        preserve_raw_trades: bool,
        indicators: &IndicatorInstanceStore,
        active_indicators: &[IndicatorInstanceId],
    ) {
        for series in &mut self.series {
            series.reset_for_basis(preserve_raw_trades);
        }
        self.configure_series_for_basis(basis, indicators, active_indicators);
    }

    pub fn configure_series_for_basis(
        &mut self,
        basis: Basis,
        indicators: &IndicatorInstanceStore,
        active_indicators: &[IndicatorInstanceId],
    ) {
        match basis {
            Basis::Time(timeframe) => {
                for series in &mut self.series {
                    series.set_footprint_tick_size(
                        self.footprint_tick_multiplier
                            .multiply_with_min_tick_step(series.ticker_info),
                    );
                    series.disable_tick_aggregation();
                    series.rebuild_time_footprints(timeframe);
                    series.configure_indicators(indicators, basis, active_indicators);
                }
            }
            Basis::Tick(interval) => {
                for series in &mut self.series {
                    let tick_size = self
                        .footprint_tick_multiplier
                        .multiply_with_min_tick_step(series.ticker_info);
                    series.set_footprint_tick_size(tick_size);
                    series.enable_tick_aggregation(interval, tick_size);
                    series.configure_indicators(indicators, basis, active_indicators);
                }
            }
        }
    }

    pub fn configure_indicators(
        &mut self,
        basis: Basis,
        indicators: &IndicatorInstanceStore,
        active_indicators: &[IndicatorInstanceId],
    ) {
        for series in &mut self.series {
            series.configure_indicators(indicators, basis, active_indicators);
        }
    }

    pub fn insert_historical_klines(
        &mut self,
        req_id: Option<uuid::Uuid>,
        ticker_info: TickerInfo,
        klines: &[Kline],
        basis: Basis,
        fetch: &mut FetchCoordinator,
        indicators: &IndicatorInstanceStore,
    ) -> bool {
        let Some(idx) = self.series_index.get(&ticker_info).copied() else {
            if let Some(req_id) = req_id {
                fetch.mark_failed_for(ticker_info, req_id);
            }
            return false;
        };
        if klines.is_empty() {
            if let Some(req_id) = req_id {
                fetch.mark_failed_for(ticker_info, req_id);
            }
            return false;
        }

        self.series[idx].insert_klines(klines, basis);
        self.series[idx].refresh_indicators(indicators, basis);
        if let Some(req_id) = req_id {
            fetch.mark_completed(ticker_info, req_id);
        }
        true
    }

    pub fn update_live_kline(
        &mut self,
        ticker_info: &TickerInfo,
        kline: &Kline,
        basis: Basis,
        indicators: &IndicatorInstanceStore,
    ) -> bool {
        let Some(idx) = self.series_index.get(ticker_info).copied() else {
            return false;
        };
        let scope = self.series[idx].update_latest_kline(kline, basis);
        if let Some(scope) = scope {
            self.series[idx].update_indicators_after_klines(indicators, scope);
        }
        scope.is_some()
    }

    pub fn insert_live_trades(
        &mut self,
        ticker_info: &TickerInfo,
        trades: &[Trade],
        basis: Basis,
        indicators: &IndicatorInstanceStore,
    ) -> bool {
        if trades.is_empty() {
            return false;
        }
        let Some(index) = self.series_index.get(ticker_info).copied() else {
            return false;
        };

        let scope = match basis {
            Basis::Tick(_) => self.series[index].insert_tick_trades(trades),
            Basis::Time(timeframe) => {
                if *ticker_info != self.base_ticker {
                    return false;
                }
                self.series[index].raw_trades.extend_from_slice(trades);
                self.series[index].insert_time_trades(trades, timeframe)
            }
        };
        self.series[index].update_indicators_after_trades(indicators, basis, scope);
        true
    }

    pub fn insert_historical_trades(
        &mut self,
        trades: &[Trade],
        is_batches_done: bool,
        req_id: Option<uuid::Uuid>,
        basis: Basis,
        fetch: &mut FetchCoordinator,
        indicators: &IndicatorInstanceStore,
    ) -> bool {
        let Basis::Time(timeframe) = basis else {
            return false;
        };

        let scope = if let Some(base_series) = self.base_series_mut() {
            base_series.raw_trades.extend_from_slice(trades);
            Some(base_series.insert_time_trades(trades, timeframe))
        } else {
            None
        };
        if !trades.is_empty()
            && let Some(scope) = scope
            && let Some(base_series) = self.base_series_mut()
        {
            base_series.update_indicators_after_trades(indicators, basis, scope);
        }
        fetch.complete_trade_fetch(self.base_ticker, req_id, is_batches_done);
        !trades.is_empty()
    }

    pub fn insert_historical_open_interest(
        &mut self,
        req_id: Option<uuid::Uuid>,
        ticker_info: TickerInfo,
        data: &[OpenInterest],
        basis: Basis,
        fetch: &mut FetchCoordinator,
        indicators: &IndicatorInstanceStore,
    ) -> bool {
        if ticker_info != self.base_ticker {
            return false;
        }
        let Some(idx) = self.series_index.get(&ticker_info).copied() else {
            if let Some(req_id) = req_id {
                fetch.mark_failed_for(ticker_info, req_id);
            }
            return false;
        };
        if let Some(req_id) = req_id {
            if data.is_empty() {
                fetch.mark_failed_for(ticker_info, req_id);
            } else {
                fetch.mark_completed(ticker_info, req_id);
            }
        }
        if data.is_empty() {
            return false;
        }
        self.series[idx]
            .insert_indicator_input(IndicatorInput::OpenInterest { data, basis }, indicators);
        true
    }

    pub fn streams_for_all(&self, basis: Basis) -> Vec<StreamKind> {
        match basis {
            Basis::Time(timeframe) => {
                let mut streams = vec![StreamKind::Trades {
                    ticker_info: self.base_ticker,
                }];
                streams.extend(self.selected_tickers.iter().copied().map(|ticker_info| {
                    StreamKind::Kline {
                        ticker_info,
                        timeframe,
                    }
                }));
                streams
            }
            Basis::Tick(_) => self
                .selected_tickers
                .iter()
                .copied()
                .map(|ticker_info| StreamKind::Trades { ticker_info })
                .collect(),
        }
    }

    fn rebuild_series_index(&mut self) {
        self.series_index.clear();
        for (idx, series) in self.series.iter().enumerate() {
            self.series_index.insert(series.ticker_info, idx);
        }
    }
}

impl KlineData {
    /// Resolves the row tick size a volume profile study aggregates at.
    ///
    /// A study without an explicit tick multiplier inherits the pane's
    /// footprint multiplier. A study can only choose a tick multiplier that is
    /// at least as coarse as the footprint multiplier, so the resolved step is
    /// never finer than the footprint step.
    pub fn effective_study_tick_size(&self, study: KlineStudy) -> PriceStep {
        let footprint = self.footprint_tick_multiplier;
        let multiplier = study.volume_profile_tick_multiplier().unwrap_or(footprint);
        let multiplier = TickMultiplier(multiplier.0.max(footprint.0));
        multiplier.multiply_with_min_tick_step(self.base_ticker)
    }

    pub fn configure_profile_caches(
        &mut self,
        studies: &[KlineStudy],
        basis: Basis,
        footprints_visible: bool,
        poc_config: FootprintPocConfig,
    ) {
        let recurring_intervals = studies
            .iter()
            .filter_map(|study| {
                study
                    .recurring_profile_interval()
                    .filter(|_| study.is_interval_available(basis))
                    .map(|interval| (interval, self.effective_study_tick_size(*study)))
            })
            .collect::<Vec<_>>();
        let footprint_poc_enabled = footprints_visible
            && matches!(basis, Basis::Time(_) | Basis::Tick(_))
            && (poc_config.show_poc || poc_config.show_npoc);
        self.configure_recurring_profile_intervals(&recurring_intervals);
        self.configure_footprint_poc_cache(footprint_poc_enabled);
    }
}
