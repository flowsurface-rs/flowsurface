use crate::aggr::{TickCount, ticks::TickAggr};
use crate::chart::Basis;
use crate::chart::kline::KlineTrades;
use crate::chart::kline_v2::indicators::{RsiConfig, SeriesIndicatorData};
use crate::chart::kline_v2::{KlineIndicator, composition::PanelValueId};
use exchange::{Kline, OpenInterest, TickerInfo, Timeframe, Trade, UnixMs, unit::PriceStep};
use std::collections::BTreeMap;

pub const SERIES_MAX_BARS: usize = 5000;

pub struct KlineSeries {
    pub ticker_info: TickerInfo,
    pub name: Option<String>,
    pub bars: Vec<Kline>,
    pub indicators: Option<SeriesIndicatorData>,
    pub footprints: Option<BTreeMap<UnixMs, KlineTrades>>,
    pub footprint_tick_size: PriceStep,
    pub raw_trades: Vec<Trade>,
    pub tick_times: Vec<UnixMs>,
    tick_aggr: Option<TickAggr>,
}

impl KlineSeries {
    pub fn new(ticker_info: TickerInfo) -> Self {
        Self {
            ticker_info,
            name: None,
            bars: Vec::new(),
            indicators: None,
            footprints: None,
            footprint_tick_size: ticker_info.min_ticksize.into(),
            raw_trades: Vec::new(),
            tick_times: Vec::new(),
            tick_aggr: None,
        }
    }

    pub fn new_base(ticker_info: TickerInfo) -> Self {
        let mut series = Self::new(ticker_info);
        series.indicators = Some(SeriesIndicatorData::default());
        series.footprints = Some(BTreeMap::new());
        series
    }

    pub fn oi_timerange(&self) -> Option<(UnixMs, UnixMs)> {
        self.indicators.as_ref()?.oi_timerange()
    }

    pub fn refresh_indicator_inputs(&mut self) {
        if let Some(indicators) = self.indicators.as_mut() {
            indicators.refresh_from_bars(&self.bars);
        }
    }

    pub fn enable_tick_aggregation(&mut self, interval: TickCount, footprint_tick_size: PriceStep) {
        self.bars.clear();
        if let Some(indicators) = self.indicators.as_mut() {
            indicators.clear();
        }
        self.tick_aggr = Some(TickAggr::new(
            interval,
            footprint_tick_size,
            &self.raw_trades,
        ));
        self.rebuild_bars_from_tick_aggregation();
    }

    pub fn disable_tick_aggregation(&mut self) {
        self.tick_aggr = None;
        self.tick_times.clear();
    }

    pub fn reset_for_basis(&mut self, preserve_raw_trades: bool) {
        self.bars.clear();
        if let Some(indicators) = self.indicators.as_mut() {
            indicators.clear();
        }
        if let Some(footprints) = self.footprints.as_mut() {
            footprints.clear();
        }
        self.disable_tick_aggregation();

        if !preserve_raw_trades {
            self.raw_trades.clear();
        }
    }

    pub fn insert_tick_trades(&mut self, trades: &[Trade]) {
        if self.tick_aggr.is_none() {
            return;
        }

        self.raw_trades.extend_from_slice(trades);

        let tick_aggr = self
            .tick_aggr
            .as_mut()
            .expect("tick aggregation was checked above");

        tick_aggr.insert_trades(trades);
        self.rebuild_bars_from_tick_aggregation();
    }

    pub fn insert_time_trades(&mut self, trades: &[Trade], timeframe: Timeframe) {
        let Some(footprints) = self.footprints.as_mut() else {
            return;
        };

        let mut updated_times = Vec::new();

        for trade in trades {
            let bar_time = trade.time.floor_to(timeframe);
            if !updated_times.contains(&bar_time) {
                updated_times.push(bar_time);
            }

            footprints
                .entry(bar_time)
                .or_insert_with(KlineTrades::new)
                .add_trade_to_nearest_bin(trade, self.footprint_tick_size);
        }

        for time in updated_times {
            if let Some(footprint) = footprints.get_mut(&time) {
                footprint.calculate_poc();
            }
        }
    }

    pub fn set_footprint_tick_size(&mut self, tick_size: PriceStep) {
        self.footprint_tick_size = tick_size;
    }

    pub fn rebuild_time_footprints(&mut self, timeframe: Timeframe) {
        if let Some(footprints) = self.footprints.as_mut() {
            footprints.clear();
        } else {
            return;
        }

        let raw_trades = self.raw_trades.clone();
        self.insert_time_trades(&raw_trades, timeframe);
    }

    pub fn trade_time_range(&self) -> Option<(UnixMs, UnixMs)> {
        let earliest = self.raw_trades.iter().map(|trade| trade.time).min()?;
        let latest = self.raw_trades.iter().map(|trade| trade.time).max()?;
        Some((earliest, latest))
    }

    pub fn suggest_trade_fetch_range(
        &self,
        visible_earliest: UnixMs,
        visible_latest: UnixMs,
        timeframe: Timeframe,
    ) -> Option<(UnixMs, UnixMs)> {
        let footprints = self.footprints.as_ref()?;
        let target_time = self
            .bars
            .iter()
            .rev()
            .find(|bar| {
                footprints
                    .get(&bar.time)
                    .map(|footprint| footprint.trades.is_empty())
                    .unwrap_or(true)
            })
            .map(|bar| bar.time)?;

        let last_trade_before_gap = self
            .raw_trades
            .iter()
            .filter(|trade| trade.time < target_time)
            .map(|trade| trade.time)
            .max();
        let first_trade_after_gap = self
            .raw_trades
            .iter()
            .filter(|trade| trade.time > target_time)
            .map(|trade| trade.time)
            .min();

        if last_trade_before_gap.is_none() && first_trade_after_gap.is_none() {
            return None;
        }

        let data_earliest = self.bars.first()?.time;
        let data_latest = self.bars.last()?.time;
        let gap_start = last_trade_before_gap.map_or(data_earliest, |time| time.saturating_add(1));
        let fetch_from = gap_start
            .max(visible_earliest)
            .floor_to(timeframe)
            .max(gap_start);
        let fetch_to = first_trade_after_gap.map_or(data_latest.min(visible_latest), |time| {
            time.saturating_sub(1)
        });

        if fetch_from >= fetch_to {
            return None;
        }

        if gap_start < visible_earliest {
            let interval_ms = timeframe.to_milliseconds();
            if fetch_to.as_u64().saturating_sub(fetch_from.as_u64()) < interval_ms {
                return None;
            }
        }

        Some((fetch_from, fetch_to))
    }

    pub fn footprint_for_bar(&self, bar: &Kline) -> Option<&KlineTrades> {
        if let Some(tick_aggr) = self.tick_aggr.as_ref() {
            return usize::try_from(bar.time.as_u64())
                .ok()
                .and_then(|index| tick_aggr.datapoints.get(index))
                .map(|point| &point.footprint);
        }

        self.footprints.as_ref()?.get(&bar.time)
    }

    pub fn rebuild_bars_from_tick_aggregation(&mut self) {
        let Some(tick_aggr) = self.tick_aggr.as_mut() else {
            self.bars.clear();
            return;
        };

        if tick_aggr.datapoints.len() > SERIES_MAX_BARS {
            let excess = tick_aggr.datapoints.len() - SERIES_MAX_BARS;
            tick_aggr.datapoints.drain(0..excess);
        }

        self.tick_times = tick_aggr
            .datapoints
            .iter()
            .map(|point| point.kline.time)
            .collect();
        self.bars = tick_aggr
            .datapoints
            .iter()
            .enumerate()
            .map(|(index, point)| {
                let mut bar = point.kline;
                // Tick indicators are keyed by bar time. Synthetic monotonic
                // timestamps preserve one-to-one identity even when trades
                // share an exchange timestamp.
                bar.time = UnixMs::new(index as u64);
                bar
            })
            .collect();
        self.refresh_indicator_inputs();
    }

    pub fn set_rsi_config(&mut self, config: RsiConfig) -> bool {
        self.indicators
            .as_mut()
            .map(|indicators| indicators.set_rsi_config(config))
            .unwrap_or(false)
    }

    pub fn insert_open_interest(
        &mut self,
        data: &[OpenInterest],
        basis: Basis,
        timeframe: Timeframe,
    ) {
        if let Some(indicators) = self.indicators.as_mut() {
            indicators.insert_open_interest_batch(data, basis, timeframe);
        }
    }
}

impl KlineSeries {
    pub fn indicator_for_panel_value(panel_value: Option<PanelValueId>) -> Option<KlineIndicator> {
        match panel_value {
            Some(PanelValueId::Volume) => Some(KlineIndicator::Volume),
            Some(PanelValueId::BollingerBands) => Some(KlineIndicator::BollingerBands),
            Some(PanelValueId::Rsi) => Some(KlineIndicator::Rsi),
            Some(PanelValueId::OpenInterest) => Some(KlineIndicator::OpenInterest),
            Some(PanelValueId::CumulativeVolumeDelta) => {
                Some(KlineIndicator::CumulativeVolumeDelta)
            }
            None => None,
        }
    }
}
