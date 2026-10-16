use crate::aggr::{
    TickCount,
    ticks::{TickAccumulation, TickAggr},
    time::TimeSeries,
};
use crate::chart::kline::IndicatorUpdateScope;
use crate::chart::kline::indicators::OpenInterestSeries;
use crate::chart::kline::{LineStrokeStyle, MAX_LINE_WIDTH, MIN_LINE_WIDTH};
use crate::chart::kline_data::{ClusterKind, KlineDataPoint, KlineTrades, NPoc, PointOfControl};
use crate::chart::{Basis, PlotData};
use crate::config::theme::{ThemeAccent, ThemeColor, ThemeColorRole, ThemeColorShade};
use exchange::{
    Kline, TickerInfo, Timeframe, Trade, UnixMs,
    unit::{Price, PriceStep, qty::Qty},
};
use indicator_engine::{ChartFrame, IndicatorFrame, InstrumentContext, TimeOrTick, TradeAggregate};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

const NAKED_POC_LINE_END: UnixMs = UnixMs::new(u64::MAX);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KlineIndexChange {
    None,
    Prefix(usize),
    Explicit(Vec<Option<usize>>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KlineUpdate {
    pub scope: IndicatorUpdateScope,
    pub index_change: KlineIndexChange,
}

struct KlineIndexSnapshot {
    count: usize,
    pure_prefix: bool,
    pure_tail: bool,
    previous_times: Option<Vec<UnixMs>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SeriesColor {
    Theme(ThemeColor),
    Custom(iced_core::Color),
}

impl SeriesColor {
    pub const fn theme(accent: Option<ThemeAccent>, shade: ThemeColorShade) -> Self {
        Self::Theme(ThemeColor::with_accent(
            ThemeColorRole::Neutral,
            accent,
            shade,
        ))
    }

    pub const fn with_accent(accent: ThemeAccent) -> Self {
        Self::theme(Some(accent), ThemeColorShade::Base)
    }

    pub fn theme_color(self) -> Option<ThemeColor> {
        match self {
            Self::Theme(color) => Some(color),
            Self::Custom(_) => None,
        }
    }

    pub fn accent_value(self) -> Option<ThemeAccent> {
        self.theme_color().and_then(|color| color.accent)
    }

    pub fn is_default(self) -> bool {
        self == Self::default()
    }
}

impl Default for SeriesColor {
    fn default() -> Self {
        Self::with_accent(ThemeAccent::Four)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PriceLevelTrade {
    pub price: Price,
    pub buy_qty: Qty,
    pub sell_qty: Qty,
}

impl PriceLevelTrade {
    pub fn total_qty(self) -> Qty {
        self.buy_qty + self.sell_qty
    }
}

#[derive(Debug, Clone, Default)]
pub struct PriceLevelTradeDistribution {
    pub levels: Vec<PriceLevelTrade>,
    pub max_qty: Qty,
    pub poc: Option<PointOfControl>,
}

#[derive(Debug, Clone)]
pub struct AnchoredPriceLevelTradeDistribution {
    pub start: UnixMs,
    pub end: UnixMs,
    pub distribution: PriceLevelTradeDistribution,
}

#[derive(Debug, Clone, Copy)]
pub struct FootprintPocLine {
    pub start: UnixMs,
    pub poc: PointOfControl,
}

struct RecurringProfile {
    levels: BTreeMap<Price, (Qty, Qty)>,
    max_qty: Qty,
    poc: Option<PointOfControl>,
}

impl RecurringProfile {
    fn new() -> Self {
        Self {
            levels: BTreeMap::new(),
            max_qty: Qty::ZERO,
            poc: None,
        }
    }

    fn add_volume(&mut self, price: Price, buy_qty: Qty, sell_qty: Qty) {
        let level = self.levels.entry(price).or_insert((Qty::ZERO, Qty::ZERO));
        level.0 += buy_qty;
        level.1 += sell_qty;

        let total_qty = level.0 + level.1;
        self.max_qty = self.max_qty.max(total_qty);

        let should_replace_poc = self.poc.is_none_or(|poc| {
            total_qty > poc.volume || (total_qty == poc.volume && price < poc.price)
        });
        if should_replace_poc {
            self.poc = Some(PointOfControl {
                price,
                volume: total_qty,
                status: NPoc::None,
            });
        } else if self.poc.is_some_and(|poc| poc.price == price)
            && let Some(poc) = self.poc.as_mut()
        {
            poc.volume = total_qty;
        }
    }

    fn distribution(&self) -> PriceLevelTradeDistribution {
        let levels = self
            .levels
            .iter()
            .map(|(&price, &(buy_qty, sell_qty))| PriceLevelTrade {
                price,
                buy_qty,
                sell_qty,
            })
            .collect();

        PriceLevelTradeDistribution {
            levels,
            max_qty: self.max_qty,
            poc: self.poc,
        }
    }

    fn price_range(&self) -> Option<(Price, Price)> {
        self.levels
            .first_key_value()
            .zip(self.levels.last_key_value())
            .map(|((low, _), (high, _))| (*low, *high))
    }
}

struct RecurringProfileCache {
    interval: Timeframe,
    tick_size: PriceStep,
    profiles: BTreeMap<UnixMs, RecurringProfile>,
    naked_pocs: BTreeMap<Price, BTreeSet<UnixMs>>,
    poc_lines_by_end: BTreeMap<UnixMs, BTreeSet<UnixMs>>,
}

impl RecurringProfileCache {
    fn new(interval: Timeframe, tick_size: PriceStep) -> Self {
        Self {
            interval,
            tick_size,
            profiles: BTreeMap::new(),
            naked_pocs: BTreeMap::new(),
            poc_lines_by_end: BTreeMap::new(),
        }
    }

    fn add_trade(&mut self, trade: &Trade, tick_size: PriceStep) -> UnixMs {
        let start = trade.time.floor_to(self.interval);
        let price = trade.price.round_to_step(tick_size);
        let (buy_qty, sell_qty) = if trade.is_sell {
            (Qty::ZERO, trade.qty)
        } else {
            (trade.qty, Qty::ZERO)
        };
        self.profiles
            .entry(start)
            .or_insert_with(RecurringProfile::new)
            .add_volume(price, buy_qty, sell_qty);
        start
    }

    fn add_footprint(&mut self, time: UnixMs, footprint: &KlineTrades) {
        let start = time.floor_to(self.interval);
        let profile = self
            .profiles
            .entry(start)
            .or_insert_with(RecurringProfile::new);
        for (&price, group) in &footprint.trades {
            let price = price.floor_to_step(self.tick_size);
            profile.add_volume(price, group.buy_qty, group.sell_qty);
        }
    }

    fn insert_trades(&mut self, trades: &[Trade], tick_size: PriceStep) {
        let mut affected_starts = BTreeSet::new();
        for trade in trades {
            affected_starts.insert(self.add_trade(trade, tick_size));
        }

        if affected_starts.len() == 1
            && self.profiles.keys().next_back().copied() == affected_starts.first().copied()
        {
            let start = affected_starts
                .first()
                .copied()
                .expect("affected profile is present");
            self.process_profile(start);
        } else if !affected_starts.is_empty() {
            self.rebuild_poc_statuses();
        }
    }

    fn rebuild_poc_statuses(&mut self) {
        self.naked_pocs.clear();
        self.poc_lines_by_end.clear();
        let starts = self.profiles.keys().copied().collect::<Vec<_>>();
        for profile in self.profiles.values_mut() {
            if let Some(poc) = profile.poc.as_mut() {
                poc.status = NPoc::None;
            }
        }
        for start in starts {
            self.process_profile(start);
        }
    }

    fn process_profile(&mut self, start: UnixMs) {
        let Some((low, high)) = self
            .profiles
            .get(&start)
            .and_then(RecurringProfile::price_range)
        else {
            return;
        };

        self.activate_previous_profile(start, low, high);

        let touched_prices = self
            .naked_pocs
            .range(low..=high)
            .map(|(&price, _)| price)
            .collect::<Vec<_>>();
        for price in touched_prices {
            if let Some(touched_starts) = self.naked_pocs.remove(&price) {
                for touched_start in touched_starts {
                    self.mark_poc_filled(touched_start, start);
                }
            }
        }

        if self.profiles.keys().next_back().copied() == Some(start) {
            return;
        }

        let Some(poc_price) = self
            .profiles
            .get(&start)
            .and_then(|profile| profile.poc.map(|poc| poc.price))
        else {
            return;
        };
        self.mark_poc_naked(start, poc_price);
    }

    fn activate_previous_profile(&mut self, start: UnixMs, low: Price, high: Price) {
        let Some(previous_start) = self
            .profiles
            .range(..start)
            .next_back()
            .map(|(&start, _)| start)
        else {
            return;
        };
        let Some(previous_poc) = self
            .profiles
            .get(&previous_start)
            .and_then(|profile| profile.poc)
        else {
            return;
        };
        if previous_poc.status != NPoc::None {
            return;
        }

        if low <= previous_poc.price && high >= previous_poc.price {
            self.mark_poc_filled(previous_start, start);
        } else {
            self.mark_poc_naked(previous_start, previous_poc.price);
        }
    }

    fn mark_poc_naked(&mut self, start: UnixMs, poc_price: Price) {
        if let Some(poc) = self
            .profiles
            .get_mut(&start)
            .and_then(|profile| profile.poc.as_mut())
        {
            poc.status = NPoc::Naked;
        }
        self.naked_pocs.entry(poc_price).or_default().insert(start);
        self.poc_lines_by_end
            .entry(NAKED_POC_LINE_END)
            .or_default()
            .insert(start);
    }

    fn mark_poc_filled(&mut self, start: UnixMs, at: UnixMs) {
        self.remove_poc_line(start, NAKED_POC_LINE_END);
        if let Some(poc) = self
            .profiles
            .get_mut(&start)
            .and_then(|profile| profile.poc.as_mut())
        {
            poc.status = NPoc::Filled { at: at.as_u64() };
        }
        self.poc_lines_by_end.entry(at).or_default().insert(start);
    }

    fn remove_poc_line(&mut self, start: UnixMs, end: UnixMs) {
        let mut remove_end = false;
        if let Some(starts) = self.poc_lines_by_end.get_mut(&end) {
            starts.remove(&start);
            remove_end = starts.is_empty();
        }
        if remove_end {
            self.poc_lines_by_end.remove(&end);
        }
    }

    fn visible_profiles(
        &self,
        earliest: UnixMs,
        latest: UnixMs,
    ) -> Vec<AnchoredPriceLevelTradeDistribution> {
        if earliest > latest {
            return Vec::new();
        }

        let body_start = earliest.saturating_sub(self.interval.to_milliseconds());
        let mut visible_starts = self
            .profiles
            .range(body_start..=latest)
            .map(|(&start, _)| start)
            .collect::<BTreeSet<_>>();
        for starts in self
            .poc_lines_by_end
            .range(earliest..)
            .map(|(_, starts)| starts)
        {
            visible_starts.extend(starts.iter().copied().filter(|&start| start <= latest));
        }

        visible_starts
            .into_iter()
            .filter_map(|start| {
                self.profiles
                    .get(&start)
                    .map(|profile| AnchoredPriceLevelTradeDistribution {
                        start,
                        end: start.saturating_add(self.interval.to_milliseconds()),
                        distribution: profile.distribution(),
                    })
            })
            .collect()
    }
}

#[derive(Clone, Copy)]
pub struct KlineSeriesPoint<'a> {
    pub index: usize,
    pub kline: &'a Kline,
    pub footprint: &'a KlineTrades,
}

pub enum KlineSeriesPoints<'a> {
    Time(std::iter::Enumerate<std::collections::btree_map::Values<'a, UnixMs, KlineDataPoint>>),
    Tick(std::iter::Enumerate<std::slice::Iter<'a, TickAccumulation>>),
}

impl<'a> Iterator for KlineSeriesPoints<'a> {
    type Item = KlineSeriesPoint<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Time(iter) => iter.next().map(|(index, point)| KlineSeriesPoint {
                index,
                kline: &point.kline,
                footprint: &point.footprint,
            }),
            Self::Tick(iter) => iter.next().map(|(index, point)| KlineSeriesPoint {
                index,
                kline: &point.kline,
                footprint: &point.footprint,
            }),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Time(iter) => iter.size_hint(),
            Self::Tick(iter) => iter.size_hint(),
        }
    }
}

impl<'a> DoubleEndedIterator for KlineSeriesPoints<'a> {
    fn next_back(&mut self) -> Option<Self::Item> {
        match self {
            Self::Time(iter) => iter.next_back().map(|(index, point)| KlineSeriesPoint {
                index,
                kline: &point.kline,
                footprint: &point.footprint,
            }),
            Self::Tick(iter) => iter.next_back().map(|(index, point)| KlineSeriesPoint {
                index,
                kline: &point.kline,
                footprint: &point.footprint,
            }),
        }
    }
}

impl<'a> ExactSizeIterator for KlineSeriesPoints<'a> {
    fn len(&self) -> usize {
        match self {
            Self::Time(iter) => iter.len(),
            Self::Tick(iter) => iter.len(),
        }
    }
}

pub struct KlineSeries {
    pub ticker_info: TickerInfo,
    pub data: PlotData<KlineDataPoint>,
    time_index: Vec<UnixMs>,
    color: SeriesColor,
    line_width: u8,
    stroke_style: LineStrokeStyle,
    raw_trades: Vec<Trade>,
    trade_sequence_id: uuid::Uuid,
    open_interest: Option<OpenInterestSeries>,
    trade_time_range: Option<(UnixMs, UnixMs)>,
    time_trade_timeframe: Option<Timeframe>,
    recurring_profile_caches: BTreeMap<(Timeframe, PriceStep), RecurringProfileCache>,
}

impl KlineSeries {
    pub fn new(ticker_info: TickerInfo, color: SeriesColor) -> Self {
        Self::new_with_basis(
            ticker_info,
            color,
            Basis::Time(Timeframe::M1),
            ticker_info.min_ticksize.into(),
        )
    }

    pub fn new_with_basis(
        ticker_info: TickerInfo,
        color: SeriesColor,
        basis: Basis,
        tick_size: PriceStep,
    ) -> Self {
        Self {
            ticker_info,
            data: empty_plot_data(basis, tick_size),
            time_index: Vec::new(),
            color,
            line_width: MIN_LINE_WIDTH,
            stroke_style: LineStrokeStyle::Solid,
            raw_trades: Vec::new(),
            trade_sequence_id: uuid::Uuid::new_v4(),
            open_interest: None,
            trade_time_range: None,
            time_trade_timeframe: None,
            recurring_profile_caches: BTreeMap::new(),
        }
    }

    pub fn new_base(ticker_info: TickerInfo) -> Self {
        Self::new(ticker_info, SeriesColor::default())
    }

    pub fn color(&self) -> SeriesColor {
        self.color
    }

    pub fn basis(&self) -> Basis {
        self.data.basis()
    }

    pub fn footprint_tick_size(&self) -> PriceStep {
        self.data.tick_size()
    }

    pub fn point_count(&self) -> usize {
        self.data.len()
    }

    pub fn time_index_range(&self, earliest: UnixMs, latest: UnixMs) -> Range<usize> {
        if earliest > latest {
            return 0..0;
        }

        self.time_index.partition_point(|time| *time < earliest)
            ..self.time_index.partition_point(|time| *time <= latest)
    }

    pub fn points(&self) -> KlineSeriesPoints<'_> {
        match &self.data {
            PlotData::TimeBased(series) => {
                KlineSeriesPoints::Time(series.datapoints.values().enumerate())
            }
            PlotData::TickBased(series) => {
                KlineSeriesPoints::Tick(series.datapoints.iter().enumerate())
            }
        }
    }

    pub fn bar_at_or_before_time(&self, time: UnixMs) -> Option<&Kline> {
        let PlotData::TimeBased(series) = &self.data else {
            return None;
        };
        series
            .datapoints
            .range(..=time)
            .next_back()
            .map(|(_, point)| &point.kline)
    }

    pub fn bar_index_at_or_before_time(&self, time: UnixMs) -> Option<usize> {
        matches!(&self.data, PlotData::TimeBased(_))
            .then(|| {
                self.time_index
                    .partition_point(|candidate| *candidate <= time)
            })
            .and_then(|index| index.checked_sub(1))
    }

    pub fn point_at(&self, index: usize) -> Option<KlineSeriesPoint<'_>> {
        match &self.data {
            PlotData::TimeBased(series) => {
                series
                    .datapoints
                    .values()
                    .nth(index)
                    .map(|point| KlineSeriesPoint {
                        index,
                        kline: &point.kline,
                        footprint: &point.footprint,
                    })
            }
            PlotData::TickBased(series) => {
                series.datapoints.get(index).map(|point| KlineSeriesPoint {
                    index,
                    kline: &point.kline,
                    footprint: &point.footprint,
                })
            }
        }
    }

    pub fn for_each_point_in_range<F>(
        &self,
        point_range: Range<usize>,
        time_range: Option<(UnixMs, UnixMs)>,
        mut f: F,
    ) where
        F: FnMut(KlineSeriesPoint<'_>),
    {
        let end = point_range.end.min(self.point_count());
        if point_range.start >= end {
            return;
        }

        match &self.data {
            PlotData::TimeBased(series) => {
                let Some((start_time, end_time)) = time_range else {
                    return;
                };
                for (index, (_, point)) in
                    (point_range.start..end).zip(series.datapoints.range(start_time..=end_time))
                {
                    f(KlineSeriesPoint {
                        index,
                        kline: &point.kline,
                        footprint: &point.footprint,
                    });
                }
            }
            PlotData::TickBased(series) => {
                for (offset, point) in series.datapoints[point_range.start..end].iter().enumerate()
                {
                    f(KlineSeriesPoint {
                        index: point_range.start + offset,
                        kline: &point.kline,
                        footprint: &point.footprint,
                    });
                }
            }
        }
    }

    pub fn first_point(&self) -> Option<KlineSeriesPoint<'_>> {
        self.points().next()
    }

    pub fn latest_point(&self) -> Option<KlineSeriesPoint<'_>> {
        self.points().next_back()
    }

    pub fn committed_points(&self) -> impl Iterator<Item = KlineSeriesPoint<'_>> {
        self.points().take(self.point_count().saturating_sub(1))
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn max_cluster_qty_in_point_range(
        &self,
        cluster_kind: ClusterKind,
        point_range: Range<usize>,
        lowest: Price,
        highest: Price,
    ) -> Qty {
        if lowest > highest {
            return Qty::ZERO;
        }

        let point_count = self.point_count();
        let start = point_range.start.min(point_count);
        let end = point_range.end.min(point_count);
        if start >= end {
            return Qty::ZERO;
        }

        match &self.data {
            PlotData::TimeBased(series) => {
                let Some(first_time) = self.time_index.get(start).copied() else {
                    return Qty::ZERO;
                };
                let Some(last_time) = self.time_index.get(end - 1).copied() else {
                    return Qty::ZERO;
                };
                series.max_qty_ts_range(cluster_kind, first_time, last_time, highest, lowest)
            }
            PlotData::TickBased(series) => {
                let reverse_start = point_count - end;
                let reverse_end = point_count - start;
                series.max_qty_idx_range(
                    cluster_kind,
                    reverse_start,
                    reverse_end - 1,
                    highest,
                    lowest,
                )
            }
        }
    }

    pub fn set_basis(&mut self, basis: Basis) {
        if self.basis() != basis {
            let preserve_raw_trades = self.basis().is_time() == basis.is_time();
            self.set_aggregation_basis(basis, preserve_raw_trades);
        }
    }

    pub fn trade_sequence_id(&self) -> uuid::Uuid {
        self.trade_sequence_id
    }

    pub fn set_aggregation_basis(&mut self, basis: Basis, preserve_raw_trades: bool) {
        let preserve_raw_trades = preserve_raw_trades && self.basis().is_time() == basis.is_time();
        let raw_trades = if preserve_raw_trades {
            self.raw_trades.clone()
        } else {
            Vec::new()
        };
        self.data = empty_plot_data_with_trades(basis, self.footprint_tick_size(), &raw_trades);
        if !preserve_raw_trades {
            self.raw_trades.clear();
            self.trade_sequence_id = uuid::Uuid::new_v4();
        }
        self.time_index.clear();
        self.clear_profile_caches();
        self.trade_time_range = calculate_trade_time_range(&self.raw_trades);
        self.time_trade_timeframe = match basis {
            Basis::Time(timeframe) => Some(timeframe),
            Basis::Tick(_) => None,
        };
        self.open_interest = None;
    }

    pub fn has_directional_volume(&self, trade_data_enabled: bool) -> bool {
        self.points().any(|point| {
            point.kline.volume.is_directional()
                || (trade_data_enabled && point.footprint.trade_summary().is_some())
        })
    }

    pub fn indicator_chart_frame(
        &self,
        index: usize,
        time: UnixMs,
        trade_data_enabled: bool,
    ) -> ChartFrame {
        let point = match self.basis() {
            Basis::Time(_) => match &self.data {
                PlotData::TimeBased(series) => series
                    .datapoints
                    .get(&time)
                    .map(|point| (&point.kline, &point.footprint)),
                PlotData::TickBased(_) => None,
            },
            Basis::Tick(_) => self
                .point_at(index)
                .map(|point| (point.kline, point.footprint)),
        };
        let footprint = point.as_ref().map(|(_, footprint)| *footprint);
        let trade = trade_data_enabled
            .then(|| self.trade_totals(footprint))
            .flatten();
        let trade_buy_qty = trade.map_or(0.0, |trade| trade.buy_qty);
        let trade_sell_qty = trade.map_or(0.0, |trade| trade.sell_qty);
        let (buy_qty, sell_qty, directional_volume_available) = point
            .as_ref()
            .and_then(|(kline, _)| kline.volume.buy_sell())
            .map(|(buy, sell)| (buy.to_f64(), sell.to_f64(), true))
            .unwrap_or((trade_buy_qty, trade_sell_qty, trade.is_some()));
        ChartFrame {
            open: point.as_ref().map(|(kline, _)| kline.open.to_f64()),
            high: point.as_ref().map(|(kline, _)| kline.high.to_f64()),
            low: point.as_ref().map(|(kline, _)| kline.low.to_f64()),
            close: point.as_ref().map(|(kline, _)| kline.close.to_f64()),
            volume: point
                .as_ref()
                .map(|(kline, _)| kline.volume.total().to_f64()),
            directional_volume_available,
            buy_qty,
            sell_qty,
            trade,
        }
    }

    pub fn indicator_frame_basis(&self) -> TimeOrTick {
        match self.basis() {
            Basis::Time(_) => TimeOrTick::Time,
            Basis::Tick(_) => TimeOrTick::Tick,
        }
    }

    fn trade_totals(&self, footprint: Option<&KlineTrades>) -> Option<TradeAggregate> {
        let summary = footprint?.trade_summary()?;
        Some(TradeAggregate {
            buy_qty: summary.buy_qty.to_f64(),
            sell_qty: summary.sell_qty.to_f64(),
            buy_count: summary.buy_count,
            sell_count: summary.sell_count,
        })
    }

    pub fn indicator_frame(
        &self,
        index: usize,
        time: UnixMs,
        open_interest: Option<f64>,
        trade_data_enabled: bool,
    ) -> IndicatorFrame {
        IndicatorFrame {
            index,
            time: time.as_u64() as i64,
            basis: self.indicator_frame_basis(),
            instrument: InstrumentContext {
                min_tick: Some(self.ticker_info.min_ticksize.as_f64()),
            },
            chart: self.indicator_chart_frame(index, time, trade_data_enabled),
            open_interest,
            series_inputs: Vec::new().into(),
        }
    }

    pub fn set_color(&mut self, color: iced_core::Color) -> bool {
        self.set_series_color(SeriesColor::Custom(color))
    }

    pub fn set_series_color(&mut self, color: SeriesColor) -> bool {
        if self.color == color {
            return false;
        }

        self.color = color;
        true
    }

    pub fn line_width(&self) -> u8 {
        self.line_width
    }

    pub fn stroke_style(&self) -> LineStrokeStyle {
        self.stroke_style
    }

    pub fn set_line_width(&mut self, width: u8) -> bool {
        let width = width.clamp(MIN_LINE_WIDTH, MAX_LINE_WIDTH);
        if self.line_width == width {
            return false;
        }
        self.line_width = width;
        true
    }

    pub fn set_stroke_style(&mut self, stroke_style: LineStrokeStyle) -> bool {
        if self.stroke_style == stroke_style {
            return false;
        }
        self.stroke_style = stroke_style;
        true
    }

    pub fn has_custom_line_style(&self) -> bool {
        self.line_width != MIN_LINE_WIDTH || self.stroke_style != LineStrokeStyle::Solid
    }

    pub fn time_at_index(&self, index: usize) -> Option<UnixMs> {
        match self.basis() {
            Basis::Time(_) => self.time_index.get(index).copied(),
            Basis::Tick(_) => self.point_at(index).map(|point| point.kline.time),
        }
    }

    fn index_snapshot(&self, klines: &[Kline]) -> KlineIndexSnapshot {
        let count = self.point_count();
        let first = self.first_point().map(|point| point.kline.time);
        let last = self.latest_point().map(|point| point.kline.time);
        let pure_prefix = first.is_some_and(|first| {
            klines.iter().any(|kline| kline.time < first)
                && klines
                    .iter()
                    .all(|kline| kline.time < first || self.contains_time(kline.time))
        });
        let pure_tail = last.is_some_and(|last| {
            klines.iter().any(|kline| kline.time > last)
                && klines
                    .iter()
                    .all(|kline| kline.time > last || self.contains_time(kline.time))
        });
        let all_existing =
            !pure_prefix && !pure_tail && klines.iter().all(|kline| self.contains_time(kline.time));
        let previous_times = (count > 0 && !pure_prefix && !pure_tail && !all_existing)
            .then(|| self.points().map(|point| point.kline.time).collect());

        KlineIndexSnapshot {
            count,
            pure_prefix,
            pure_tail,
            previous_times,
        }
    }

    fn index_change_since(&self, snapshot: KlineIndexSnapshot) -> KlineIndexChange {
        let count = self.point_count();
        if count <= snapshot.count || snapshot.count == 0 {
            return KlineIndexChange::None;
        }

        let inserted = count - snapshot.count;
        if snapshot.pure_prefix {
            return KlineIndexChange::Prefix(inserted);
        }
        if snapshot.pure_tail {
            return KlineIndexChange::None;
        }

        let Some(previous_times) = snapshot.previous_times else {
            return KlineIndexChange::None;
        };
        let mut previous_index = 0;
        let current_indices = self
            .points()
            .map(|point| point.kline.time)
            .map(|time| {
                while previous_index < previous_times.len() && previous_times[previous_index] < time
                {
                    previous_index += 1;
                }
                if previous_index < previous_times.len() && previous_times[previous_index] == time {
                    Some(previous_index)
                } else {
                    None
                }
            })
            .collect();

        KlineIndexChange::Explicit(current_indices)
    }

    fn contains_time(&self, time: UnixMs) -> bool {
        match &self.data {
            PlotData::TimeBased(series) => series.datapoints.contains_key(&time),
            PlotData::TickBased(_) => false,
        }
    }

    pub fn insert_klines(&mut self, klines: &[Kline], basis: Basis) -> Option<KlineUpdate> {
        if klines.is_empty() {
            return None;
        }
        if self.basis() != basis {
            self.set_aggregation_basis(basis, false);
        }
        let index_snapshot = self.index_snapshot(klines);

        let report = match &mut self.data {
            PlotData::TimeBased(series) => series.insert_klines_report(klines),
            PlotData::TickBased(_) => {
                return Some(KlineUpdate {
                    scope: IndicatorUpdateScope::Full,
                    index_change: KlineIndexChange::None,
                });
            }
        }?;

        self.insert_time_index_entries(&report.inserted_timestamps);

        if let PlotData::TimeBased(series) = &mut self.data {
            series.backfill_new_buckets(&self.raw_trades, &report.inserted_timestamps);
            series.update_poc_status();
        }

        let scope = match report.ordered_change {
            crate::aggr::time::OrderedChange::Tail { from } => {
                IndicatorUpdateScope::From(self.time_index(from).unwrap_or(0).saturating_sub(1))
            }
            crate::aggr::time::OrderedChange::Rebuild => IndicatorUpdateScope::Full,
        };

        Some(KlineUpdate {
            scope,
            index_change: self.index_change_since(index_snapshot),
        })
    }

    pub fn missing_kline_range(
        &self,
        earliest: UnixMs,
        latest: UnixMs,
        _timeframe: Timeframe,
    ) -> Option<(UnixMs, UnixMs)> {
        match &self.data {
            PlotData::TimeBased(series) => series.missing_kline_range(earliest, latest),
            PlotData::TickBased(_) => None,
        }
    }

    pub fn update_latest_kline(&mut self, kline: &Kline, basis: Basis) -> Option<KlineUpdate> {
        if self.basis() != basis {
            self.set_aggregation_basis(basis, false);
        }
        let index_snapshot = self.index_snapshot(std::slice::from_ref(kline));
        let previous_latest = self.latest_point().map(|point| point.kline.time);
        let previous_len = self.point_count();
        let mut replay_new_bucket = false;
        let mut inserted_new_bucket = false;

        let scope = match (&mut self.data, previous_latest) {
            (PlotData::TimeBased(series), None) => {
                let report = series.insert_klines_report(std::slice::from_ref(kline));
                inserted_new_bucket = report
                    .as_ref()
                    .is_some_and(|report| report.inserted_timestamps.contains(&kline.time));
                replay_new_bucket = inserted_new_bucket;
                Some(IndicatorUpdateScope::From(0))
            }
            (PlotData::TimeBased(series), Some(latest)) if kline.time >= latest => {
                let report = series.insert_klines_report(std::slice::from_ref(kline));
                inserted_new_bucket = report
                    .as_ref()
                    .is_some_and(|report| report.inserted_timestamps.contains(&kline.time));
                replay_new_bucket = inserted_new_bucket;
                Some(IndicatorUpdateScope::From(if kline.time > latest {
                    previous_len
                } else {
                    previous_len.saturating_sub(1)
                }))
            }
            (PlotData::TimeBased(series), Some(_)) => {
                let report = series.insert_klines_report(std::slice::from_ref(kline));
                inserted_new_bucket = report
                    .as_ref()
                    .is_some_and(|report| report.inserted_timestamps.contains(&kline.time));
                replay_new_bucket = inserted_new_bucket;
                Some(IndicatorUpdateScope::Full)
            }
            (PlotData::TickBased(_), _) => None,
        };

        if inserted_new_bucket {
            self.insert_time_index_entries(std::slice::from_ref(&kline.time));
        }

        if replay_new_bucket && let PlotData::TimeBased(series) = &mut self.data {
            series.backfill_new_buckets(&self.raw_trades, std::slice::from_ref(&kline.time));
            series.update_poc_status();
        }

        scope.map(|scope| KlineUpdate {
            scope,
            index_change: self.index_change_since(index_snapshot),
        })
    }

    pub fn indicator_source_index_for_visual(&self, visual_index: usize) -> Option<usize> {
        (visual_index < self.point_count()).then_some(visual_index)
    }

    pub fn enable_tick_aggregation(&mut self, interval: TickCount, footprint_tick_size: PriceStep) {
        self.set_aggregation_basis(Basis::Tick(interval), true);
        if let PlotData::TickBased(series) = &mut self.data {
            series.change_tick_size(footprint_tick_size, &self.raw_trades);
        }
    }

    pub fn disable_tick_aggregation(&mut self) {
        if self.basis().is_time() {
            return;
        }
        self.set_aggregation_basis(Basis::Time(Timeframe::M1), false);
    }

    pub fn reset_for_basis(&mut self, preserve_raw_trades: bool) {
        let basis = self.basis();
        self.set_aggregation_basis(basis, preserve_raw_trades);
    }

    pub fn insert_tick_trades(&mut self, trades: &[Trade]) -> IndicatorUpdateScope {
        let PlotData::TickBased(tick_aggr) = &mut self.data else {
            return IndicatorUpdateScope::From(self.point_count());
        };
        if trades.is_empty() {
            return IndicatorUpdateScope::From(self.point_count());
        }
        merge_trade_time_range(&mut self.trade_time_range, trades);
        self.raw_trades.extend_from_slice(trades);
        self.recurring_profile_caches.clear();
        let first_updated = tick_aggr
            .insert_trades(trades)
            .unwrap_or(self.point_count());
        IndicatorUpdateScope::From(first_updated)
    }

    pub fn insert_time_trades(
        &mut self,
        trades: &[Trade],
        timeframe: Timeframe,
    ) -> IndicatorUpdateScope {
        if !matches!(self.data, PlotData::TimeBased(_)) {
            return IndicatorUpdateScope::From(self.point_count());
        }
        if trades.is_empty() {
            return IndicatorUpdateScope::From(self.point_count());
        }
        merge_trade_time_range(&mut self.trade_time_range, trades);
        self.raw_trades.extend_from_slice(trades);
        if self
            .time_trade_timeframe
            .is_some_and(|current| current != timeframe)
        {
            self.clear_profile_caches();
        }
        self.time_trade_timeframe = Some(timeframe);
        let PlotData::TimeBased(series) = &mut self.data else {
            unreachable!("time basis checked above");
        };
        series.insert_trades_existing_buckets(trades);
        series.update_poc_status();
        self.update_recurring_profile_caches(trades);
        let first_time = trades
            .iter()
            .map(|trade| trade.time.floor_to(timeframe))
            .min();
        IndicatorUpdateScope::From(
            first_time
                .map(|time| self.first_bar_at_or_after(time))
                .unwrap_or_else(|| self.point_count()),
        )
    }

    pub fn set_footprint_tick_size(&mut self, tick_size: PriceStep) {
        if self.footprint_tick_size() == tick_size {
            return;
        }
        let raw_trades = self.raw_trades.clone();
        match &mut self.data {
            PlotData::TimeBased(series) => series.change_tick_size(tick_size, &raw_trades),
            PlotData::TickBased(series) => series.change_tick_size(tick_size, &raw_trades),
        }
        self.clear_profile_caches();
    }

    pub fn rebuild_time_footprints(&mut self, timeframe: Timeframe) {
        let raw_trades = self.raw_trades.clone();
        if !matches!(self.data, PlotData::TimeBased(_)) {
            return;
        }
        self.clear_profile_caches();
        let PlotData::TimeBased(series) = &mut self.data else {
            unreachable!("time basis checked above");
        };
        series.clear_trades();
        series.insert_trades_existing_buckets(&raw_trades);
        series.update_poc_status();
        self.time_trade_timeframe = Some(timeframe);
        self.trade_time_range = calculate_trade_time_range(&raw_trades);
    }

    pub fn configure_recurring_profile_intervals(&mut self, intervals: &[(Timeframe, PriceStep)]) {
        let desired_intervals = intervals.iter().copied().collect::<BTreeSet<_>>();
        self.recurring_profile_caches
            .retain(|interval, _| desired_intervals.contains(interval));

        let intervals_to_build = desired_intervals
            .into_iter()
            .filter(|interval| !self.recurring_profile_caches.contains_key(interval))
            .collect::<Vec<_>>();
        for (interval, tick_size) in intervals_to_build {
            let cache = self.build_recurring_profile_cache(interval, tick_size);
            self.recurring_profile_caches
                .insert((interval, tick_size), cache);
        }
    }

    pub fn configure_footprint_poc_cache(&mut self, _enabled: bool) {}

    pub fn footprint_poc_lines(&self, earliest: UnixMs, latest: UnixMs) -> Vec<FootprintPocLine> {
        if earliest > latest {
            return Vec::new();
        }
        let tick_basis = matches!(self.basis(), Basis::Tick(_));
        let point_count = self.point_count();
        self.points()
            .filter_map(|point| {
                let start = if tick_basis {
                    UnixMs::new(point.index as u64)
                } else {
                    point.kline.time
                };
                let poc = point.footprint.poc?;
                let end = match poc.status {
                    NPoc::None | NPoc::Naked => NAKED_POC_LINE_END,
                    NPoc::Filled { at } if tick_basis => UnixMs::new(
                        point_count
                            .saturating_sub(1)
                            .saturating_sub(at as usize) as u64,
                    ),
                    NPoc::Filled { at } => UnixMs::new(at),
                };
                (start <= latest && end >= earliest).then_some(FootprintPocLine { start, poc })
            })
            .collect()
    }

    fn clear_profile_caches(&mut self) {
        self.recurring_profile_caches.clear();
    }

    fn insert_time_index_entries(&mut self, timestamps: &[UnixMs]) {
        let Some(&first_timestamp) = timestamps.first() else {
            return;
        };
        if self
            .time_index
            .last()
            .is_none_or(|last| first_timestamp > *last)
        {
            self.time_index.extend_from_slice(timestamps);
            return;
        }

        let existing = std::mem::take(&mut self.time_index);
        self.time_index = merge_sorted_time_index(&existing, timestamps);
    }

    fn update_recurring_profile_caches(&mut self, trades: &[Trade]) {
        if trades.is_empty() {
            return;
        }

        for cache in self.recurring_profile_caches.values_mut() {
            cache.insert_trades(trades, cache.tick_size);
        }
    }

    fn build_recurring_profile_cache(
        &self,
        interval: Timeframe,
        tick_size: PriceStep,
    ) -> RecurringProfileCache {
        let interval_ms = interval.to_milliseconds();
        let can_use_footprints = self.time_trade_timeframe.is_some_and(|timeframe| {
            timeframe.to_milliseconds() <= interval_ms
                && interval_ms.is_multiple_of(timeframe.to_milliseconds())
        });
        let mut cache = RecurringProfileCache::new(interval, tick_size);

        let mut used_footprints = false;
        if can_use_footprints && let PlotData::TimeBased(series) = &self.data {
            used_footprints = series
                .datapoints
                .values()
                .any(|point| !point.footprint.trades.is_empty());
            for (&time, point) in &series.datapoints {
                cache.add_footprint(time, &point.footprint);
            }
        }
        if !used_footprints {
            for trade in &self.raw_trades {
                cache.add_trade(trade, tick_size);
            }
        }

        cache.rebuild_poc_statuses();
        cache
    }

    pub fn trade_time_range(&self) -> Option<(UnixMs, UnixMs)> {
        self.trade_time_range
    }

    pub fn suggest_trade_fetch_range(
        &self,
        visible_earliest: UnixMs,
        visible_latest: UnixMs,
    ) -> Option<(UnixMs, UnixMs)> {
        match &self.data {
            PlotData::TimeBased(series) => {
                series.suggest_trade_fetch_range(visible_earliest, visible_latest)
            }
            PlotData::TickBased(_) => None,
        }
    }

    pub fn trade_price_distribution(
        &self,
        earliest: UnixMs,
        latest: UnixMs,
        lowest: Price,
        highest: Price,
        tick_size: PriceStep,
    ) -> Option<PriceLevelTradeDistribution> {
        let mut levels = BTreeMap::new();
        let mut add_footprint = |footprint: &KlineTrades| {
            for (price, group) in &footprint.trades {
                let price = price.floor_to_step(tick_size);
                if price < lowest || price > highest {
                    continue;
                }
                let level = levels.entry(price).or_insert((Qty::ZERO, Qty::ZERO));
                level.0 += group.buy_qty;
                level.1 += group.sell_qty;
            }
        };

        let tick_basis = matches!(self.basis(), Basis::Tick(_));
        for point in self.points() {
            let in_range = if tick_basis {
                let index = point.index as u64;
                index >= earliest.as_u64() && index <= latest.as_u64()
            } else {
                point.kline.time >= earliest && point.kline.time <= latest
            };
            if in_range {
                add_footprint(point.footprint);
            }
        }

        Self::distribution_from_levels(levels)
    }

    pub fn recurring_trade_price_distributions(
        &self,
        interval: Timeframe,
        tick_size: PriceStep,
        earliest: UnixMs,
        latest: UnixMs,
    ) -> Vec<AnchoredPriceLevelTradeDistribution> {
        self.recurring_profile_caches
            .get(&(interval, tick_size))
            .map(|cache| cache.visible_profiles(earliest, latest))
            .unwrap_or_default()
    }

    fn distribution_from_levels(
        levels: BTreeMap<Price, (Qty, Qty)>,
    ) -> Option<PriceLevelTradeDistribution> {
        if levels.is_empty() {
            return None;
        }

        let mut max_qty = Qty::ZERO;
        let levels: Vec<PriceLevelTrade> = levels
            .into_iter()
            .map(|(price, (buy_qty, sell_qty))| {
                let level = PriceLevelTrade {
                    price,
                    buy_qty,
                    sell_qty,
                };
                max_qty = max_qty.max(level.total_qty());
                level
            })
            .collect();

        let poc = levels
            .iter()
            .copied()
            .fold(None::<PriceLevelTrade>, |best, level| match best {
                Some(best) if best.total_qty() >= level.total_qty() => Some(best),
                _ => Some(level),
            })
            .map(|level| PointOfControl {
                price: level.price,
                volume: level.total_qty(),
                status: NPoc::None,
            });

        Some(PriceLevelTradeDistribution {
            levels,
            max_qty,
            poc,
        })
    }

    pub fn insert_open_interest(&mut self, series: &OpenInterestSeries) {
        self.open_interest
            .get_or_insert_with(OpenInterestSeries::default)
            .merge(series);
    }

    pub fn open_interest_range(&self) -> Option<(UnixMs, UnixMs)> {
        self.open_interest.as_ref()?.covered_range()
    }

    pub fn open_interest(&self) -> Option<&OpenInterestSeries> {
        self.open_interest.as_ref()
    }

    pub fn clear_open_interest(&mut self) -> bool {
        self.open_interest.take().is_some()
    }

    fn first_bar_at_or_after(&self, time: UnixMs) -> usize {
        self.points()
            .find(|point| point.kline.time >= time)
            .map_or(self.point_count(), |point| point.index)
    }

    fn time_index(&self, time: UnixMs) -> Option<usize> {
        self.points()
            .find(|point| point.kline.time == time)
            .map(|point| point.index)
    }
}

fn merge_sorted_time_index(existing: &[UnixMs], timestamps: &[UnixMs]) -> Vec<UnixMs> {
    let mut merged = Vec::with_capacity(existing.len().saturating_add(timestamps.len()));
    let mut existing_index = 0;
    let mut timestamp_index = 0;

    while existing_index < existing.len() || timestamp_index < timestamps.len() {
        let next = match (
            existing.get(existing_index),
            timestamps.get(timestamp_index),
        ) {
            (Some(existing), Some(timestamp)) if existing <= timestamp => {
                existing_index += 1;
                *existing
            }
            (Some(_), Some(timestamp)) => {
                timestamp_index += 1;
                *timestamp
            }
            (Some(existing), None) => {
                existing_index += 1;
                *existing
            }
            (None, Some(timestamp)) => {
                timestamp_index += 1;
                *timestamp
            }
            (None, None) => break,
        };
        if merged.last().copied() != Some(next) {
            merged.push(next);
        }
    }

    merged
}

fn empty_plot_data(basis: Basis, tick_size: PriceStep) -> PlotData<KlineDataPoint> {
    empty_plot_data_with_trades(basis, tick_size, &[])
}

fn empty_plot_data_with_trades(
    basis: Basis,
    tick_size: PriceStep,
    raw_trades: &[Trade],
) -> PlotData<KlineDataPoint> {
    match basis {
        Basis::Time(interval) => {
            PlotData::TimeBased(TimeSeries::<KlineDataPoint>::new(interval, tick_size, &[]))
        }
        Basis::Tick(interval) => {
            PlotData::TickBased(TickAggr::new(interval, tick_size, raw_trades))
        }
    }
}

fn calculate_trade_time_range(trades: &[Trade]) -> Option<(UnixMs, UnixMs)> {
    trades
        .iter()
        .map(|trade| trade.time)
        .fold(None, |range, time| {
            Some(match range {
                Some((min, max)) => (min.min(time), max.max(time)),
                None => (time, time),
            })
        })
}

fn merge_trade_time_range(range: &mut Option<(UnixMs, UnixMs)>, trades: &[Trade]) {
    let Some((batch_min, batch_max)) =
        trades
            .iter()
            .map(|trade| trade.time)
            .fold(None::<(UnixMs, UnixMs)>, |range, time| {
                Some(match range {
                    Some((min, max)) => (min.min(time), max.max(time)),
                    None => (time, time),
                })
            })
    else {
        return;
    };
    *range = Some(match *range {
        Some((min, max)) => (min.min(batch_min), max.max(batch_max)),
        None => (batch_min, batch_max),
    });
}
