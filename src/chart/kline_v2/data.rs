use super::KlineSeries;
use super::fetch::FetchCoordinator;
use data::chart::Basis;
use data::chart::kline_v2::{IndicatorEngine, IndicatorInput, SeriesColor, ThemeAccent};
use exchange::{
    Kline, OpenInterest, TickMultiplier, TickerInfo, Timeframe, Trade, adapter::StreamKind,
};
use rustc_hash::FxHashMap;

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

    pub fn configure_recurring_profile_intervals(&mut self, intervals: &[Timeframe]) {
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

    pub fn add_ticker(
        &mut self,
        ticker_info: TickerInfo,
        basis: Basis,
        indicators: &IndicatorEngine,
    ) -> bool {
        if self.selected_tickers.contains(&ticker_info) {
            return false;
        }

        let idx = self.series.len();
        let mut series = if ticker_info == self.base_ticker {
            KlineSeries::new_base(ticker_info)
        } else {
            KlineSeries::new(ticker_info, SeriesColor::Accent(self.next_unused_accent()))
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
        indicators.configure_series(&mut series, basis);

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
                    .any(|series| series.color() == SeriesColor::Accent(*accent))
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
        indicators: &IndicatorEngine,
    ) {
        for series in &mut self.series {
            series.reset_for_basis(preserve_raw_trades);
        }
        self.configure_series_for_basis(basis, indicators);
    }

    pub fn configure_series_for_basis(&mut self, basis: Basis, indicators: &IndicatorEngine) {
        match basis {
            Basis::Time(timeframe) => {
                for series in &mut self.series {
                    series.set_footprint_tick_size(
                        self.footprint_tick_multiplier
                            .multiply_with_min_tick_step(series.ticker_info),
                    );
                    series.disable_tick_aggregation();
                    series.rebuild_time_footprints(timeframe);
                    indicators.configure_series(series, basis);
                }
            }
            Basis::Tick(interval) => {
                for series in &mut self.series {
                    let tick_size = self
                        .footprint_tick_multiplier
                        .multiply_with_min_tick_step(series.ticker_info);
                    series.set_footprint_tick_size(tick_size);
                    series.enable_tick_aggregation(interval, tick_size);
                    indicators.configure_series(series, basis);
                }
            }
        }
    }

    pub fn configure_indicators(&mut self, basis: Basis, indicators: &IndicatorEngine) {
        for series in &mut self.series {
            indicators.configure_series(series, basis);
        }
    }

    pub fn insert_historical_klines(
        &mut self,
        req_id: Option<uuid::Uuid>,
        ticker_info: TickerInfo,
        klines: &[Kline],
        basis: Basis,
        fetch: &mut FetchCoordinator,
        indicators: &IndicatorEngine,
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
        indicators.configure_series(&mut self.series[idx], basis);
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
        indicators: &IndicatorEngine,
    ) -> bool {
        let Some(idx) = self.series_index.get(ticker_info).copied() else {
            return false;
        };
        let scope = self.series[idx].update_latest_kline(kline, basis);
        if let Some(scope) = scope {
            indicators.update_series_after_klines(&mut self.series[idx], scope);
        }
        scope.is_some()
    }

    pub fn insert_live_trades(
        &mut self,
        ticker_info: &TickerInfo,
        trades: &[Trade],
        basis: Basis,
        indicators: &IndicatorEngine,
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
        indicators.update_series_after_trades(&mut self.series[index], basis, scope);
        true
    }

    pub fn insert_historical_trades(
        &mut self,
        trades: &[Trade],
        is_batches_done: bool,
        req_id: Option<uuid::Uuid>,
        basis: Basis,
        fetch: &mut FetchCoordinator,
        indicators: &IndicatorEngine,
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
            indicators.update_series_after_trades(base_series, basis, scope);
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
        indicators: &IndicatorEngine,
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
        self.series[idx].insert_indicator_input(
            IndicatorInput::OpenInterest { data, basis },
            indicators.configs(),
        );
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
