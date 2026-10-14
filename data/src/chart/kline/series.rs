use super::viewport::{BarUnitWindow, TickIndex, XAxis};
use crate::aggr::{
    ticks::{TickAccumulation, TickAggr},
    time::TimeSeries,
};
use crate::chart::kline::IndicatorUpdateScope;
use crate::chart::kline::bar::KlineDataPoint;
use crate::chart::kline::footprint::{ClusterKind, KlineTrades};
use crate::chart::kline::indicators::OpenInterestSeries;
use crate::chart::kline::poc::{NPoc, NakedPocIndex, PocFillAt, PocLineTracker, PointOfControl};
pub use crate::chart::kline::studies::big_trades::BigTradeEntry;
use crate::chart::kline::studies::big_trades::BigTradesCache;
use crate::chart::kline::studies::volume_profile::{
    AnchoredProfileDistribution, ProfileAccumulator, ProfileDistribution, ProfileLevel,
};
use crate::chart::kline::{LineStrokeStyle, MAX_LINE_WIDTH, MIN_LINE_WIDTH};
use crate::chart::{Basis, PlotData};
use crate::config::theme::{ThemeAccent, ThemeColor, ThemeColorRole, ThemeColorShade};
use exchange::{
    Kline, TickerInfo, Timeframe, Trade, UnixMs,
    unit::{Price, PriceStep, qty::Qty},
};
use indicator_engine::{ChartFrame, IndicatorFrame, InstrumentContext, TimeOrTick, TradeAggregate};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::Arc;

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
pub struct FootprintPocLine {
    pub start: UnixMs,
    pub poc: PointOfControl,
}

struct RecurringProfileCache {
    interval: Timeframe,
    exact_profiles: BTreeMap<UnixMs, FxHashMap<Price, (Qty, Qty)>>,
    projections: BTreeMap<PriceStep, RecurringProfileProjection>,
}

impl RecurringProfileCache {
    fn new(interval: Timeframe) -> Self {
        Self {
            interval,
            exact_profiles: BTreeMap::new(),
            projections: BTreeMap::new(),
        }
    }

    fn insert_trades(&mut self, trades: &[Trade]) {
        for trade in trades {
            let row = self
                .exact_profiles
                .entry(trade.time.floor_to(self.interval))
                .or_default()
                .entry(trade.price)
                .or_default();
            if trade.is_sell {
                row.1 += trade.qty;
            } else {
                row.0 += trade.qty;
            }
        }
        for projection in self.projections.values_mut() {
            projection.insert_trades(trades);
        }
    }

    fn configure_steps(&mut self, steps: &BTreeSet<PriceStep>) {
        self.projections.retain(|step, _| steps.contains(step));
        for &step in steps {
            if self.projections.contains_key(&step) {
                continue;
            }
            let mut projection = RecurringProfileProjection::new(self.interval, step);
            for (&start, rows) in &self.exact_profiles {
                let mut profile = ProfileAccumulator::new(step);
                for (&price, &(buy, sell)) in rows {
                    profile.add_volume(price.round_to_step(step), buy, sell);
                }
                projection.profiles.insert(start, profile);
            }
            projection.rebuild_poc_statuses();
            self.projections.insert(step, projection);
        }
    }
}

struct RecurringProfileProjection {
    interval: Timeframe,
    tick_size: PriceStep,
    profiles: BTreeMap<UnixMs, ProfileAccumulator>,
    poc_lines: PocLineTracker,
}

impl RecurringProfileProjection {
    fn new(interval: Timeframe, tick_size: PriceStep) -> Self {
        Self {
            interval,
            tick_size,
            profiles: BTreeMap::new(),
            poc_lines: PocLineTracker::default(),
        }
    }

    fn add_trade(&mut self, trade: &Trade) -> UnixMs {
        let start = trade.time.floor_to(self.interval);
        let price = trade.price.round_to_step(self.tick_size);
        let (buy_qty, sell_qty) = if trade.is_sell {
            (Qty::ZERO, trade.qty)
        } else {
            (trade.qty, Qty::ZERO)
        };
        self.profiles
            .entry(start)
            .or_insert_with(|| ProfileAccumulator::new(self.tick_size))
            .add_volume(price, buy_qty, sell_qty);
        start
    }

    fn insert_trades(&mut self, trades: &[Trade]) {
        let mut affected_starts = BTreeSet::new();
        for trade in trades {
            affected_starts.insert(self.add_trade(trade));
        }

        if affected_starts.len() == 1
            && self.profiles.keys().next_back().copied() == affected_starts.first().copied()
        {
            let start = affected_starts
                .first()
                .copied()
                .expect("affected profile is present");
            self.poc_lines.process_profile(&mut self.profiles, start);
        } else if !affected_starts.is_empty() {
            self.rebuild_poc_statuses();
        }
    }

    fn rebuild_poc_statuses(&mut self) {
        self.poc_lines.rebuild(&mut self.profiles);
    }

    fn visible_profiles(
        &self,
        earliest: UnixMs,
        latest: UnixMs,
    ) -> Vec<AnchoredProfileDistribution> {
        if earliest > latest {
            return Vec::new();
        }

        let visible_starts =
            self.poc_lines
                .visible_starts(self.interval, earliest, latest, &self.profiles);

        visible_starts
            .into_iter()
            .filter_map(|start| {
                self.profiles
                    .get(&start)
                    .map(|profile| AnchoredProfileDistribution {
                        start,
                        end: start.saturating_add(self.interval.to_milliseconds()),
                        distribution: profile.distribution(),
                    })
            })
            .collect()
    }
}

#[derive(Default)]
struct VisibleTradeProfileRow {
    buy_qty: Qty,
    sell_qty: Qty,
    footprint_rows: usize,
}

struct VisibleTradeProfileCache {
    earliest: UnixMs,
    latest: UnixMs,
    price_step: PriceStep,
    rows: BTreeMap<Price, VisibleTradeProfileRow>,
    snapshot: Option<Arc<ProfileDistribution>>,
}

impl VisibleTradeProfileCache {
    fn new(earliest: UnixMs, latest: UnixMs, price_step: PriceStep) -> Self {
        Self {
            earliest,
            latest,
            price_step,
            rows: BTreeMap::new(),
            snapshot: None,
        }
    }

    fn adjust_volume(&mut self, price: Price, buy: Qty, sell: Qty, footprint_rows: isize) {
        if buy.is_zero() && sell.is_zero() && footprint_rows == 0 {
            return;
        }
        let row = self.rows.entry(price).or_default();
        row.buy_qty += buy;
        row.sell_qty += sell;
        // Count footprint entries so zero-volume rows retain full-rebuild semantics.
        row.footprint_rows = row
            .footprint_rows
            .checked_add_signed(footprint_rows)
            .expect("visible profile footprint count underflowed");
        if row.footprint_rows == 0 {
            assert!(row.buy_qty.is_zero() && row.sell_qty.is_zero());
            self.rows.remove(&price);
        }
        self.snapshot = None;
    }

    fn apply_footprint(&mut self, footprint: &KlineTrades, direction: isize) {
        for (&price, group) in &footprint.trades {
            let (buy, sell) = if direction > 0 {
                (group.buy_qty, group.sell_qty)
            } else {
                (Qty::ZERO - group.buy_qty, Qty::ZERO - group.sell_qty)
            };
            self.adjust_volume(price.floor_to_step(self.price_step), buy, sell, direction);
        }
    }

    fn apply_range(
        &mut self,
        series: &TimeSeries<KlineDataPoint>,
        earliest: UnixMs,
        latest: UnixMs,
        direction: isize,
    ) {
        if earliest <= latest {
            for (_, point) in series.datapoints.range(earliest..=latest) {
                self.apply_footprint(&point.footprint, direction);
            }
        }
    }

    fn move_range(
        &mut self,
        series: &TimeSeries<KlineDataPoint>,
        earliest: UnixMs,
        latest: UnixMs,
    ) {
        if earliest > self.earliest {
            self.apply_range(
                series,
                self.earliest,
                self.latest.min(earliest.saturating_sub(1)),
                -1,
            );
        }
        if latest < self.latest {
            self.apply_range(
                series,
                self.earliest.max(latest.saturating_add(1)),
                self.latest,
                -1,
            );
        }
        if earliest < self.earliest {
            self.apply_range(
                series,
                earliest,
                latest.min(self.earliest.saturating_sub(1)),
                1,
            );
        }
        if latest > self.latest {
            self.apply_range(
                series,
                earliest.max(self.latest.saturating_add(1)),
                latest,
                1,
            );
        }
        self.earliest = earliest;
        self.latest = latest;
    }

    fn insert_trades(&mut self, series: &TimeSeries<KlineDataPoint>, trades: &[Trade]) {
        let mut new_rows = BTreeSet::new();
        for run in trades.chunk_by(|left, right| {
            left.time.floor_to(series.interval) == right.time.floor_to(series.interval)
        }) {
            let time = run[0].time.floor_to(series.interval);
            if time < self.earliest || time > self.latest {
                continue;
            }
            let Some(point) = series.datapoints.get(&time) else {
                continue;
            };
            for trade in run {
                let price = trade.price.round_to_step(series.tick_size);
                let new_row =
                    !point.footprint.trades.contains_key(&price) && new_rows.insert((time, price));
                let (buy, sell) = if trade.is_sell {
                    (Qty::ZERO, trade.qty)
                } else {
                    (trade.qty, Qty::ZERO)
                };
                self.adjust_volume(
                    price.floor_to_step(self.price_step),
                    buy,
                    sell,
                    isize::from(new_row),
                );
            }
        }
    }

    fn insert_buckets(&mut self, series: &TimeSeries<KlineDataPoint>, times: &[UnixMs]) {
        for time in times {
            if *time >= self.earliest && *time <= self.latest {
                self.apply_footprint(&series.datapoints[time].footprint, 1);
            }
        }
    }

    fn distribution(&mut self) -> Option<Arc<ProfileDistribution>> {
        if self.rows.is_empty() {
            return None;
        }
        if self.snapshot.is_none() {
            let mut distribution = ProfileDistribution {
                levels: Vec::with_capacity(self.rows.len()),
                price_step: self.price_step,
                ..ProfileDistribution::default()
            };
            for (&price, row) in &self.rows {
                let volume = row.buy_qty + row.sell_qty;
                distribution.max_qty = distribution.max_qty.max(volume);
                if distribution.poc.is_none_or(|poc| volume > poc.volume) {
                    distribution.poc = Some(PointOfControl {
                        price,
                        volume,
                        status: NPoc::None,
                    });
                }
                distribution.levels.push(ProfileLevel {
                    price,
                    positive_qty: row.buy_qty,
                    negative_qty: row.sell_qty,
                });
            }
            self.snapshot = Some(Arc::new(distribution));
        }
        self.snapshot.clone()
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
    data: PlotData<KlineDataPoint>,
    time_index: Vec<UnixMs>,
    naked_pocs: NakedPocIndex<usize>,
    color: SeriesColor,
    line_width: u8,
    stroke_style: LineStrokeStyle,
    raw_trades: Vec<Trade>,
    raw_trade_runs: Vec<Range<usize>>,
    trade_sequence_id: uuid::Uuid,
    open_interest: Option<OpenInterestSeries>,
    trade_time_range: Option<(UnixMs, UnixMs)>,
    recurring_profile_caches: BTreeMap<Timeframe, RecurringProfileCache>,
    visible_profile_cache: Option<VisibleTradeProfileCache>,
    big_trades_cache: Option<BigTradesCache>,
}

impl KlineSeries {
    pub fn bar_unit_window(
        &self,
        x_axis: XAxis,
        min_x_unit: i64,
        max_x_unit: i64,
    ) -> BarUnitWindow {
        let (start_index, end_index) = match x_axis {
            XAxis::Time { .. } => {
                let Some((earliest, latest)) = x_axis
                    .bucket_start(min_x_unit)
                    .zip(x_axis.bucket_end(max_x_unit))
                else {
                    return BarUnitWindow::from_units(Vec::new(), 0);
                };
                let range = self.time_index_range(earliest, latest);
                (range.start, range.end)
            }
            XAxis::Tick { .. } => {
                let point_count = self.point_count();
                let point_count_i64 = i64::try_from(point_count).unwrap_or(i64::MAX);
                let start = min_x_unit
                    .saturating_add(point_count_i64.saturating_sub(1))
                    .max(0);
                let end = max_x_unit
                    .saturating_add(point_count_i64)
                    .min(point_count_i64);
                (
                    usize::try_from(start)
                        .unwrap_or(point_count)
                        .min(point_count),
                    usize::try_from(end).unwrap_or(point_count).min(point_count),
                )
            }
        };

        let values = (start_index..end_index)
            .filter_map(|index| match x_axis {
                XAxis::Time { .. } => self
                    .time_at_index(index)
                    .map(|time| x_axis.unit_from_time(time)),
                XAxis::Tick { .. } => {
                    let point_count = self.point_count();
                    let from_latest = point_count.saturating_sub(1).saturating_sub(index) as u64;
                    Some(x_axis.unit_from_tick(TickIndex::from_u64(from_latest)))
                }
            })
            .collect();

        BarUnitWindow::from_units(values, start_index)
    }

    pub fn bar_at_or_before_unit(&self, x_axis: XAxis, target_unit: i64) -> Option<&Kline> {
        match x_axis {
            XAxis::Time { .. } => {
                let bucket_end = x_axis.bucket_end(target_unit)?;
                self.bar_at_or_before_time(bucket_end)
            }
            XAxis::Tick { .. } => {
                let point_count = self.point_count();
                let from_latest = x_axis
                    .tick_from_unit(target_unit)
                    .map(|index| index.as_u64())
                    .and_then(|index| usize::try_from(index).ok())
                    .unwrap_or_default();
                let index = point_count.checked_sub(from_latest.checked_add(1)?)?;
                self.point_at(index).map(|point| point.kline)
            }
        }
    }

    pub fn bar_unit_at_index(&self, axis: XAxis, point_index: usize) -> Option<i64> {
        if point_index >= self.point_count() {
            return None;
        }
        match axis {
            XAxis::Time { .. } => self
                .time_at_index(point_index)
                .map(|time| axis.unit_from_time(time)),
            XAxis::Tick { .. } => Some(
                axis.unit_from_tick(TickIndex::from_u64(
                    self.point_count()
                        .saturating_sub(1)
                        .saturating_sub(point_index) as u64,
                )),
            ),
        }
    }

    pub fn bar_index_at_or_before_unit(&self, axis: XAxis, target_unit: i64) -> Option<usize> {
        match axis {
            XAxis::Time { .. } => self.bar_index_at_or_before_time(axis.bucket_end(target_unit)?),
            XAxis::Tick { .. } => {
                let from_latest = axis
                    .tick_from_unit(target_unit)
                    .and_then(|index| usize::try_from(index.as_u64()).ok())
                    .unwrap_or_default();
                self.point_count().checked_sub(from_latest.checked_add(1)?)
            }
        }
    }

    pub fn new_with_basis(
        ticker_info: TickerInfo,
        color: SeriesColor,
        basis: Basis,
        tick_size: PriceStep,
    ) -> Self {
        Self {
            ticker_info,
            data: empty_plot_data_with_trades(basis, tick_size, &[]),
            time_index: Vec::new(),
            naked_pocs: NakedPocIndex::default(),
            color,
            line_width: MIN_LINE_WIDTH,
            stroke_style: LineStrokeStyle::Solid,
            raw_trades: Vec::new(),
            raw_trade_runs: Vec::new(),
            trade_sequence_id: uuid::Uuid::new_v4(),
            open_interest: None,
            trade_time_range: None,
            recurring_profile_caches: BTreeMap::new(),
            visible_profile_cache: None,
            big_trades_cache: None,
        }
    }

    pub fn color(&self) -> SeriesColor {
        self.color
    }

    pub fn basis(&self) -> Basis {
        self.data.basis()
    }

    pub fn tick_size(&self) -> PriceStep {
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

    pub fn klines_in_range(&self, start: UnixMs, end: UnixMs) -> Vec<Kline> {
        if start >= end {
            return Vec::new();
        }
        let latest = end.saturating_sub(1);
        let indices = self.time_index_range(start, latest);
        let mut bars = Vec::with_capacity(indices.len());
        self.for_each_point_in_range(indices, Some((start, latest)), |point| {
            bars.push(*point.kline);
        });
        bars
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

    fn bar_at_or_before_time(&self, time: UnixMs) -> Option<&Kline> {
        let PlotData::TimeBased(series) = &self.data else {
            return None;
        };
        series
            .datapoints
            .range(..=time)
            .next_back()
            .map(|(_, point)| &point.kline)
    }

    fn bar_index_at_or_before_time(&self, time: UnixMs) -> Option<usize> {
        matches!(&self.data, PlotData::TimeBased(_))
            .then(|| {
                self.time_index
                    .partition_point(|candidate| *candidate <= time)
            })
            .and_then(|index| index.checked_sub(1))
    }

    pub fn point_at(&self, index: usize) -> Option<KlineSeriesPoint<'_>> {
        match &self.data {
            PlotData::TimeBased(series) => self
                .time_index
                .get(index)
                .and_then(|time| series.datapoints.get(time))
                .map(|point| KlineSeriesPoint {
                    index,
                    kline: &point.kline,
                    footprint: &point.footprint,
                }),
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
        self.data = empty_plot_data_with_trades(basis, self.tick_size(), &raw_trades);
        if !preserve_raw_trades {
            self.raw_trades.clear();
            self.raw_trade_runs.clear();
            self.trade_sequence_id = uuid::Uuid::new_v4();
            self.reset_big_trades_cache();
        }
        self.time_index.clear();
        self.clear_profile_caches();
        self.trade_time_range = calculate_trade_time_range(&self.raw_trades);
        self.open_interest = None;
        self.rebuild_footprint_pocs();
    }

    pub fn has_directional_volume(&self, trade_data_enabled: bool) -> bool {
        self.points().any(|point| {
            point.kline.volume.is_directional()
                || (trade_data_enabled && point.footprint.trade_summary().is_some())
        })
    }

    fn indicator_chart_frame(
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

    fn indicator_frame_basis(&self) -> TimeOrTick {
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

        self.backfill_new_buckets(&report.inserted_timestamps);

        let scope = match report.ordered_change {
            crate::aggr::time::OrderedChange::Tail { from } => {
                let start = self.time_index(from).unwrap_or(0);
                self.update_footprint_pocs(start..self.point_count());
                IndicatorUpdateScope::From(start.saturating_sub(1))
            }
            crate::aggr::time::OrderedChange::Rebuild => {
                self.rebuild_footprint_pocs();
                IndicatorUpdateScope::Full
            }
        };

        Some(KlineUpdate {
            scope,
            index_change: self.index_change_since(index_snapshot),
        })
    }

    /// Missing candle intervals within an aligned, half-open time range.
    pub fn missing_kline_ranges(&self, start: UnixMs, end: UnixMs) -> Vec<(UnixMs, UnixMs)> {
        let Basis::Time(timeframe) = self.basis() else {
            return Vec::new();
        };
        if start >= end {
            return Vec::new();
        }

        let indices = self.time_index_range(start, end.saturating_sub(1));
        let mut ranges = Vec::new();
        let mut cursor = start;
        for &time in &self.time_index[indices] {
            if cursor < time {
                ranges.push((cursor, time));
            }
            cursor = time.offset_by_timeframe(timeframe, 1).min(end);
        }
        if cursor < end {
            ranges.push((cursor, end));
        }
        ranges
    }

    pub fn missing_kline_range(
        &self,
        earliest: UnixMs,
        latest: UnixMs,
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
        let previous_kline = self.latest_point().map(|point| *point.kline);
        let previous_latest = previous_kline.map(|kline| kline.time);
        let previous_len = self.point_count();
        let mut inserted_new_bucket = false;

        let scope = match (&mut self.data, previous_latest) {
            (PlotData::TimeBased(series), None) => {
                let report = series.insert_klines_report(std::slice::from_ref(kline));
                inserted_new_bucket = report
                    .as_ref()
                    .is_some_and(|report| report.inserted_timestamps.contains(&kline.time));
                Some(IndicatorUpdateScope::From(0))
            }
            (PlotData::TimeBased(series), Some(latest)) if kline.time >= latest => {
                let report = series.insert_klines_report(std::slice::from_ref(kline));
                inserted_new_bucket = report
                    .as_ref()
                    .is_some_and(|report| report.inserted_timestamps.contains(&kline.time));
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
                Some(IndicatorUpdateScope::Full)
            }
            (PlotData::TickBased(_), _) => None,
        }?;

        if inserted_new_bucket {
            self.insert_time_index_entries(std::slice::from_ref(&kline.time));
        }

        if inserted_new_bucket {
            self.backfill_new_buckets(std::slice::from_ref(&kline.time));
        }

        match previous_kline {
            Some(previous) if kline.time < previous.time => self.rebuild_footprint_pocs(),
            Some(previous) if kline.time == previous.time => {
                let step = self.tick_size();
                let previous_low = previous.low.round_to_step(step);
                let previous_high = previous.high.round_to_step(step);
                let low = kline.low.round_to_step(step);
                let high = kline.high.round_to_step(step);
                if low > previous_low || high < previous_high {
                    self.rebuild_footprint_pocs();
                } else if (low, high) != (previous_low, previous_high) {
                    self.update_footprint_pocs(previous_len.saturating_sub(1)..self.point_count());
                }
            }
            _ => self.update_footprint_pocs(previous_len..self.point_count()),
        }

        Some(KlineUpdate {
            scope,
            index_change: self.index_change_since(index_snapshot),
        })
    }

    pub fn indicator_source_index_for_visual(&self, visual_index: usize) -> Option<usize> {
        (visual_index < self.point_count()).then_some(visual_index)
    }

    pub fn insert_tick_trades(&mut self, trades: &[Trade]) -> IndicatorUpdateScope {
        if !matches!(self.data, PlotData::TickBased(_)) {
            return IndicatorUpdateScope::From(self.point_count());
        }
        if trades.is_empty() {
            return IndicatorUpdateScope::From(self.point_count());
        }
        merge_trade_time_range(&mut self.trade_time_range, trades);
        self.append_raw_trades(trades);
        self.fold_big_trades_cache();
        self.recurring_profile_caches.clear();
        let PlotData::TickBased(tick_aggr) = &mut self.data else {
            unreachable!("tick basis checked above");
        };
        let first_updated = tick_aggr
            .insert_trades(trades)
            .unwrap_or(self.point_count());
        self.update_footprint_pocs(first_updated..self.point_count());
        IndicatorUpdateScope::From(first_updated)
    }

    pub fn insert_time_trades(&mut self, trades: &[Trade]) -> IndicatorUpdateScope {
        if !matches!(self.data, PlotData::TimeBased(_)) {
            return IndicatorUpdateScope::From(self.point_count());
        }
        if trades.is_empty() {
            return IndicatorUpdateScope::From(self.point_count());
        }
        merge_trade_time_range(&mut self.trade_time_range, trades);
        self.append_raw_trades(trades);
        self.fold_big_trades_cache();
        let PlotData::TimeBased(series) = &mut self.data else {
            unreachable!("time basis checked above");
        };
        if let Some(cache) = &mut self.visible_profile_cache {
            cache.insert_trades(series, trades);
        }
        let report = series.insert_trades_existing_buckets_report(trades);
        if let Some(report) = &report {
            self.update_changed_footprint_pocs(&report.previous_poc_prices);
        }
        self.update_recurring_profile_caches(trades);
        IndicatorUpdateScope::From(
            report
                .map(|report| self.first_bar_at_or_after(report.earliest_updated))
                .unwrap_or_else(|| self.point_count()),
        )
    }

    pub fn set_tick_size(&mut self, tick_size: PriceStep) {
        if self.tick_size() == tick_size {
            return;
        }
        match &mut self.data {
            PlotData::TimeBased(series) => series.change_tick_size(tick_size),
            PlotData::TickBased(series) => series.change_tick_size(tick_size),
        }
        self.rebuild_footprint_pocs();
        self.visible_profile_cache = None;
    }

    pub fn configure_recurring_profile_intervals(&mut self, intervals: &[(Timeframe, PriceStep)]) {
        let mut desired = BTreeMap::<Timeframe, BTreeSet<PriceStep>>::new();
        for &(interval, step) in intervals {
            desired.entry(interval).or_default().insert(step);
        }
        self.recurring_profile_caches
            .retain(|interval, _| desired.contains_key(interval));
        for (interval, steps) in desired {
            if !self.recurring_profile_caches.contains_key(&interval) {
                let cache = self.build_recurring_profile_cache(interval);
                self.recurring_profile_caches.insert(interval, cache);
            }
            self.recurring_profile_caches
                .get_mut(&interval)
                .expect("recurring source initialized")
                .configure_steps(&steps);
        }
    }

    pub fn footprint_poc_lines(&self, earliest: UnixMs, latest: UnixMs) -> Vec<FootprintPocLine> {
        if earliest > latest {
            return Vec::new();
        }
        let tick_basis = matches!(self.basis(), Basis::Tick(_));
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
                    NPoc::Filled {
                        at: PocFillAt::Time(time),
                    } => time,
                    NPoc::Filled {
                        at: PocFillAt::TickBar(index),
                    } => UnixMs::new(index as u64),
                };
                (start <= latest && end >= earliest).then_some(FootprintPocLine { start, poc })
            })
            .collect()
    }

    fn footprint_mut(&mut self, index: usize) -> &mut KlineTrades {
        match &mut self.data {
            PlotData::TimeBased(series) => {
                &mut series
                    .datapoints
                    .get_mut(&self.time_index[index])
                    .expect("time index refers to an existing candle")
                    .footprint
            }
            PlotData::TickBased(series) => &mut series.datapoints[index].footprint,
        }
    }

    fn register_footprint_poc(&mut self, index: usize) {
        let Some(poc) = self.footprint_mut(index).poc.as_mut() else {
            return;
        };
        if poc.status != NPoc::None {
            return;
        }
        poc.status = NPoc::Naked;
        let price = poc.price;
        self.naked_pocs.insert(price, index);
    }

    fn update_changed_footprint_pocs(&mut self, previous_prices: &BTreeMap<UnixMs, Option<Price>>) {
        let Some((&first, _)) = previous_prices.first_key_value() else {
            return;
        };
        let start = self.time_index(first).expect("changed POC candle exists");
        let mut end = start;
        // Keep unrelated POCs out of the historical scan, including those born later.
        let mut unaffected = std::mem::take(&mut self.naked_pocs);
        for (&time, &previous_price) in previous_prices {
            let index = self.time_index(time).expect("changed POC candle exists");
            end = index + 1;
            if let Some(price) = previous_price {
                unaffected.remove(price, &index);
            }
        }
        self.update_footprint_pocs(start..end);
        unaffected.extend(std::mem::take(&mut self.naked_pocs));
        self.naked_pocs = unaffected;
    }

    fn update_footprint_pocs(&mut self, activation_range: Range<usize>) {
        let count = self.point_count();
        if activation_range.start >= count {
            return;
        }
        let step = self.tick_size();
        if let Some(previous) = activation_range.start.checked_sub(1) {
            self.register_footprint_poc(previous);
        }
        for index in activation_range.start..count {
            let bar = *self.point_at(index).expect("POC candle exists").kline;
            let at = match self.basis() {
                Basis::Time(_) => PocFillAt::Time(bar.time),
                Basis::Tick(_) => PocFillAt::TickBar(index),
            };
            for touched in self
                .naked_pocs
                .take_touched(bar.low.round_to_step(step), bar.high.round_to_step(step))
            {
                self.footprint_mut(touched)
                    .set_poc_status(NPoc::Filled { at });
            }
            // Only a subsequent candle can touch this candle's POC.
            if index < activation_range.end && index + 1 < count {
                self.register_footprint_poc(index);
            }
            if index + 1 >= activation_range.end && self.naked_pocs.is_empty() {
                break;
            }
        }
    }

    fn rebuild_footprint_pocs(&mut self) {
        self.naked_pocs.clear();
        match &mut self.data {
            PlotData::TimeBased(series) => {
                for point in series.datapoints.values_mut() {
                    point.set_poc_status(NPoc::None);
                }
            }
            PlotData::TickBased(series) => {
                for point in &mut series.datapoints {
                    point.set_poc_status(NPoc::None);
                }
            }
        }
        self.update_footprint_pocs(0..self.point_count());
    }

    fn clear_profile_caches(&mut self) {
        self.recurring_profile_caches.clear();
        self.visible_profile_cache = None;
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
            cache.insert_trades(trades);
        }
    }

    fn build_recurring_profile_cache(&self, interval: Timeframe) -> RecurringProfileCache {
        let mut cache = RecurringProfileCache::new(interval);
        cache.insert_trades(&self.raw_trades);
        cache
    }

    pub fn trade_time_range(&self) -> Option<(UnixMs, UnixMs)> {
        self.trade_time_range
    }

    fn append_raw_trades(&mut self, trades: &[Trade]) {
        let start = self.raw_trades.len();
        self.raw_trades.extend_from_slice(trades);
        if matches!(self.data, PlotData::TickBased(_)) {
            return;
        }
        for index in start..self.raw_trades.len() {
            if let Some(run) = self.raw_trade_runs.last_mut()
                && self.raw_trades[index].time >= self.raw_trades[index - 1].time
            {
                run.end = index + 1;
            } else {
                self.raw_trade_runs.push(index..index + 1);
            }
        }
    }

    fn backfill_new_buckets(&mut self, newborn: &[UnixMs]) {
        let Some((&earliest, &latest)) = newborn.iter().min().zip(newborn.iter().max()) else {
            return;
        };
        let PlotData::TimeBased(series) = &mut self.data else {
            return;
        };
        let interval = series.interval;
        let trades = self.raw_trade_runs.iter().flat_map(|run| {
            let trades = &self.raw_trades[run.clone()];
            let start = trades.partition_point(|trade| trade.time.floor_to(interval) < earliest);
            let end = trades.partition_point(|trade| trade.time.floor_to(interval) <= latest);
            trades[start..end].iter()
        });
        series.backfill_new_buckets(trades, newborn);
        if let Some(cache) = &mut self.visible_profile_cache {
            cache.insert_buckets(series, newborn);
        }
    }

    /// Rebuilds the big-trade cache for `threshold`, or drops it when no study needs it.
    /// Linear in `raw_trades`, which is the cost the caller opts into off the live path.
    pub fn configure_big_trades_threshold(&mut self, threshold: Option<u64>) {
        let Some(threshold) = threshold else {
            self.big_trades_cache = None;
            return;
        };
        if self
            .big_trades_cache
            .as_ref()
            .is_some_and(|cache| cache.threshold() == threshold)
        {
            return;
        }
        self.big_trades_cache = Some(BigTradesCache::new(
            threshold,
            &self.raw_trades,
            &self.ticker_info,
            self.basis(),
        ));
    }

    /// Requires a time-based series; tick-series entries follow trade arrival order.
    pub fn big_trades_in_time_range(&self, start: UnixMs, end: UnixMs) -> &[BigTradeEntry] {
        if start > end {
            return &[];
        }
        self.big_trades_cache
            .as_ref()
            .map_or(&[], |cache| cache.in_time_range(start, end))
    }

    pub fn big_trades_in_index_range(&self, lo: usize, hi: usize) -> &[BigTradeEntry] {
        if lo >= hi {
            return &[];
        }
        self.big_trades_cache
            .as_ref()
            .map_or(&[], |cache| cache.in_index_range(lo, hi))
    }

    fn fold_big_trades_cache(&mut self) {
        let basis = self.basis();
        let Self {
            raw_trades,
            ticker_info,
            big_trades_cache,
            ..
        } = self;
        if let Some(cache) = big_trades_cache {
            cache.fold(raw_trades, ticker_info, basis);
        }
    }

    fn reset_big_trades_cache(&mut self) {
        if let Some(cache) = self.big_trades_cache.as_mut() {
            cache.clear();
        }
    }

    /// Retained trades in arrival order; appending history preserves existing indices.
    pub fn raw_trades(&self) -> &[Trade] {
        &self.raw_trades
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

    pub fn prepare_trade_price_distribution(
        &mut self,
        earliest: UnixMs,
        latest: UnixMs,
        tick_size: PriceStep,
    ) {
        if earliest > latest {
            self.visible_profile_cache = None;
            return;
        }
        if let PlotData::TimeBased(series) = &self.data {
            let cached = &mut self.visible_profile_cache;
            if cached
                .as_ref()
                .is_none_or(|cache| cache.price_step != tick_size)
            {
                let mut cache = VisibleTradeProfileCache::new(earliest, latest, tick_size);
                cache.apply_range(series, earliest, latest, 1);
                *cached = Some(cache);
            }
            let cache = cached.as_mut().expect("visible profile cache initialized");
            cache.move_range(series, earliest, latest);
            cache.distribution();
        }
    }

    pub fn trade_price_distribution(
        &self,
        earliest: UnixMs,
        latest: UnixMs,
        tick_size: PriceStep,
    ) -> Option<Arc<ProfileDistribution>> {
        if earliest > latest {
            return None;
        }
        if let Some(cache) = &self.visible_profile_cache
            && (cache.earliest, cache.latest, cache.price_step) == (earliest, latest, tick_size)
            && (cache.snapshot.is_some() || cache.rows.is_empty())
        {
            return cache.snapshot.clone();
        }

        self.calculate_trade_price_distribution(earliest, latest, tick_size)
            .map(Arc::new)
    }

    fn calculate_trade_price_distribution(
        &self,
        earliest: UnixMs,
        latest: UnixMs,
        tick_size: PriceStep,
    ) -> Option<ProfileDistribution> {
        let mut profile = ProfileAccumulator::new(tick_size);

        let tick_basis = matches!(self.basis(), Basis::Tick(_));
        for point in self.points() {
            let in_range = if tick_basis {
                let index = point.index as u64;
                index >= earliest.as_u64() && index <= latest.as_u64()
            } else {
                point.kline.time >= earliest && point.kline.time <= latest
            };
            if in_range {
                for (&price, group) in &point.footprint.trades {
                    profile.add_volume(
                        price.floor_to_step(tick_size),
                        group.buy_qty,
                        group.sell_qty,
                    );
                }
            }
        }

        profile.finish()
    }

    pub fn recurring_trade_price_distributions(
        &self,
        interval: Timeframe,
        tick_size: PriceStep,
        earliest: UnixMs,
        latest: UnixMs,
    ) -> Vec<AnchoredProfileDistribution> {
        self.recurring_profile_caches
            .get(&interval)
            .and_then(|cache| cache.projections.get(&tick_size))
            .map(|projection| projection.visible_profiles(earliest, latest))
            .unwrap_or_default()
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

    fn first_bar_at_or_after(&self, time: UnixMs) -> usize {
        self.time_index
            .partition_point(|candidate| *candidate < time)
    }

    fn time_index(&self, time: UnixMs) -> Option<usize> {
        self.time_index.binary_search(&time).ok()
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

pub struct KlineWindows<'a> {
    pub series: &'a [KlineSeries],
    pub windows: &'a [BarUnitWindow],
    pub axis: XAxis,
}

impl KlineWindows<'_> {
    pub fn for_each_point(
        &self,
        min_x_unit: i64,
        max_x_unit: i64,
        mut f: impl FnMut(usize, usize, &KlineSeries, i64, KlineSeriesPoint<'_>),
    ) {
        for (series_index, series) in self.series.iter().enumerate() {
            let Some(units) = self.windows.get(series_index) else {
                continue;
            };
            let range = units.index_range(min_x_unit, max_x_unit);
            let (start, end) = (range.start, range.end);
            let end = end.min(series.point_count().saturating_sub(units.start_index()));
            if start >= end {
                continue;
            }

            let time_range = match self.axis {
                XAxis::Time { .. } => self
                    .axis
                    .bucket_start(units.values()[start])
                    .zip(self.axis.bucket_end(units.values()[end - 1])),
                XAxis::Tick { .. } => None,
            };
            let global_range = units.global_range(start..end);
            series.for_each_point_in_range(global_range, time_range, |point| {
                let Some(local_index) = point.index.checked_sub(units.start_index()) else {
                    return;
                };
                if (start..end).contains(&local_index) {
                    f(
                        series_index,
                        point.index,
                        series,
                        units.values()[local_index],
                        point,
                    );
                }
            });
        }
    }

    pub fn unit_at_index(&self, series_index: usize, point_index: usize) -> Option<i64> {
        let series = self.series.get(series_index)?;
        if point_index >= series.point_count() {
            return None;
        }
        self.windows
            .get(series_index)
            .and_then(|units| units.unit_at_global_index(point_index))
            .or_else(|| series.bar_unit_at_index(self.axis, point_index))
    }

    pub fn for_each_output(
        &self,
        min: i64,
        max: i64,
        offset: i64,
        mut f: impl FnMut(&KlineSeries, usize, i64),
    ) {
        let Some(series) = self.series.first() else {
            return;
        };
        let Some(units) = self.windows.first() else {
            return;
        };
        let end = series
            .point_count()
            .saturating_sub(units.start_index())
            .min(units.values().len());
        for local_index in units.output_range(self.axis, min, max, offset, end) {
            let Some(index) = units.global_index(local_index) else {
                continue;
            };
            f(
                series,
                index,
                self.axis.output_unit(units.values()[local_index], offset),
            );
        }
    }
}
