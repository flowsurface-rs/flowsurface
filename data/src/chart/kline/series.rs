use crate::aggr::{TickCount, ticks::TickAggr};
use crate::chart::Basis;
use crate::chart::kline::IndicatorUpdateScope;
use crate::chart::kline::composition::PanelValueId;
use crate::chart::kline::indicators::{
    IndicatorInput, IndicatorRuntimeSet, IndicatorRuntimeSnapshot, NormalizedTradeBar,
    TradeBarValues,
};
use crate::chart::kline::values::IndicatorData;
use crate::chart::kline::{
    IndicatorInstanceId, IndicatorInstanceStore, LineStrokeStyle, MAX_LINE_WIDTH, MIN_LINE_WIDTH,
};
use crate::chart::kline_data::{KlineTrades, NPoc, PointOfControl};
use crate::config::theme::{ThemeAccent, ThemeColor, ThemeColorRole, ThemeColorShade};
use exchange::{
    Kline, TickerInfo, Timeframe, Trade, UnixMs,
    unit::{Price, PriceStep, qty::Qty},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const SERIES_MAX_BARS: usize = 12_000;
const TICK_CLEANUP_BATCH: usize = 100;
const MAX_INTEGRITY_SCAN: u64 = 1_000_000;
const NAKED_POC_LINE_END: UnixMs = UnixMs::new(u64::MAX);

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
        Self::theme(None, ThemeColorShade::Base)
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

    fn retain_from(&mut self, earliest: UnixMs) {
        while self
            .profiles
            .first_key_value()
            .is_some_and(|(&start, _)| start < earliest)
        {
            let (start, profile) = self
                .profiles
                .pop_first()
                .expect("profile is present before retention boundary");
            self.remove_poc_indexes(start, profile.poc);
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

    fn remove_poc_indexes(&mut self, start: UnixMs, poc: Option<PointOfControl>) {
        let Some(poc) = poc else {
            return;
        };

        match poc.status {
            NPoc::None => {}
            NPoc::Naked => self.remove_naked_poc(start, poc.price),
            NPoc::Filled { at } => self.remove_poc_line(start, UnixMs::new(at)),
        }
    }

    fn remove_naked_poc(&mut self, start: UnixMs, poc_price: Price) {
        let mut remove_price = false;
        if let Some(starts) = self.naked_pocs.get_mut(&poc_price) {
            starts.remove(&start);
            remove_price = starts.is_empty();
        }
        if remove_price {
            self.naked_pocs.remove(&poc_price);
        }
        self.remove_poc_line(start, NAKED_POC_LINE_END);
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

struct FootprintPocRecord {
    poc: Option<PointOfControl>,
    price_range: Option<(Price, Price)>,
}

impl FootprintPocRecord {
    fn from_footprint(footprint: &KlineTrades) -> Self {
        let poc = footprint.poc.map(|mut poc| {
            poc.status = NPoc::None;
            poc
        });
        let price_range = footprint
            .trades
            .keys()
            .min()
            .zip(footprint.trades.keys().max())
            .map(|(&low, &high)| (low, high));
        Self { poc, price_range }
    }
}

struct FootprintPocCache {
    records: BTreeMap<UnixMs, FootprintPocRecord>,
    naked_pocs: BTreeMap<Price, BTreeSet<UnixMs>>,
    poc_lines_by_end: BTreeMap<UnixMs, BTreeSet<UnixMs>>,
}

impl FootprintPocCache {
    fn new() -> Self {
        Self {
            records: BTreeMap::new(),
            naked_pocs: BTreeMap::new(),
            poc_lines_by_end: BTreeMap::new(),
        }
    }

    fn update_footprints<'a, I>(&mut self, footprints: I)
    where
        I: IntoIterator<Item = (UnixMs, &'a KlineTrades)>,
    {
        let mut affected_starts = BTreeSet::new();
        for (start, footprint) in footprints {
            affected_starts.insert(start);
            self.records
                .insert(start, FootprintPocRecord::from_footprint(footprint));
        }

        if affected_starts.len() == 1 && self.records.keys().next_back() == affected_starts.first()
        {
            let start = affected_starts
                .first()
                .copied()
                .expect("affected footprint is present");
            self.process_footprint(start);
        } else if !affected_starts.is_empty() {
            self.rebuild_poc_statuses();
        }
    }

    fn retain_from(&mut self, earliest: UnixMs) {
        while self
            .records
            .first_key_value()
            .is_some_and(|(&start, _)| start < earliest)
        {
            let (start, record) = self
                .records
                .pop_first()
                .expect("footprint is present before retention boundary");
            self.remove_poc_indexes(start, record.poc);
        }
    }

    fn rebuild_poc_statuses(&mut self) {
        self.naked_pocs.clear();
        self.poc_lines_by_end.clear();
        let starts = self.records.keys().copied().collect::<Vec<_>>();
        for record in self.records.values_mut() {
            if let Some(poc) = record.poc.as_mut() {
                poc.status = NPoc::None;
            }
        }
        for start in starts {
            self.process_footprint(start);
        }
    }

    fn process_footprint(&mut self, start: UnixMs) {
        let Some((low, high)) = self
            .records
            .get(&start)
            .and_then(|record| record.price_range)
        else {
            return;
        };

        self.activate_previous_footprint(start, low, high);

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

        if self.records.keys().next_back().copied() == Some(start) {
            return;
        }

        let Some(poc_price) = self
            .records
            .get(&start)
            .and_then(|record| record.poc.map(|poc| poc.price))
        else {
            return;
        };
        self.mark_poc_naked(start, poc_price);
    }

    fn activate_previous_footprint(&mut self, start: UnixMs, low: Price, high: Price) {
        let Some(previous_start) = self
            .records
            .range(..start)
            .next_back()
            .map(|(&start, _)| start)
        else {
            return;
        };
        let Some(previous_poc) = self
            .records
            .get(&previous_start)
            .and_then(|record| record.poc)
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
            .records
            .get_mut(&start)
            .and_then(|record| record.poc.as_mut())
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
            .records
            .get_mut(&start)
            .and_then(|record| record.poc.as_mut())
        {
            poc.status = NPoc::Filled { at: at.as_u64() };
        }
        self.poc_lines_by_end.entry(at).or_default().insert(start);
    }

    fn remove_poc_indexes(&mut self, start: UnixMs, poc: Option<PointOfControl>) {
        let Some(poc) = poc else {
            return;
        };

        match poc.status {
            NPoc::None => {}
            NPoc::Naked => self.remove_naked_poc(start, poc.price),
            NPoc::Filled { at } => self.remove_poc_line(start, UnixMs::new(at)),
        }
    }

    fn remove_naked_poc(&mut self, start: UnixMs, poc_price: Price) {
        let mut remove_price = false;
        if let Some(starts) = self.naked_pocs.get_mut(&poc_price) {
            starts.remove(&start);
            remove_price = starts.is_empty();
        }
        if remove_price {
            self.naked_pocs.remove(&poc_price);
        }
        self.remove_poc_line(start, NAKED_POC_LINE_END);
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

    fn visible_lines(&self, earliest: UnixMs, latest: UnixMs) -> Vec<FootprintPocLine> {
        if earliest > latest {
            return Vec::new();
        }

        let mut visible_starts = self
            .records
            .range(earliest..=latest)
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
                self.records
                    .get(&start)
                    .and_then(|record| record.poc)
                    .map(|poc| FootprintPocLine { start, poc })
            })
            .collect()
    }
}

pub struct KlineSeries {
    pub ticker_info: TickerInfo,
    color: SeriesColor,
    line_width: u8,
    stroke_style: LineStrokeStyle,
    pub bars: Vec<Kline>,
    pub indicators: Option<IndicatorRuntimeSet>,
    pub footprints: Option<BTreeMap<UnixMs, KlineTrades>>,
    pub footprint_tick_size: PriceStep,
    raw_trades: Vec<Trade>,
    pub tick_times: Vec<UnixMs>,
    tick_index_offset: u64,
    trade_values: BTreeMap<UnixMs, TradeBarValues>,
    trade_time_range: Option<(UnixMs, UnixMs)>,
    last_trade_retention_start: Option<UnixMs>,
    time_trade_timeframe: Option<Timeframe>,
    tick_aggr: Option<TickAggr>,
    recurring_profile_caches: BTreeMap<(Timeframe, PriceStep), RecurringProfileCache>,
    footprint_poc_cache: Option<FootprintPocCache>,
}

impl KlineSeries {
    pub fn new(ticker_info: TickerInfo, color: SeriesColor) -> Self {
        Self {
            ticker_info,
            color,
            line_width: MIN_LINE_WIDTH,
            stroke_style: LineStrokeStyle::Solid,
            bars: Vec::new(),
            indicators: None,
            footprints: None,
            footprint_tick_size: ticker_info.min_ticksize.into(),
            raw_trades: Vec::new(),
            tick_times: Vec::new(),
            tick_index_offset: 0,
            trade_values: BTreeMap::new(),
            trade_time_range: None,
            last_trade_retention_start: None,
            time_trade_timeframe: None,
            tick_aggr: None,
            recurring_profile_caches: BTreeMap::new(),
            footprint_poc_cache: None,
        }
    }

    pub fn new_base(ticker_info: TickerInfo) -> Self {
        let mut series = Self::new(ticker_info, SeriesColor::default());
        series.indicators = Some(IndicatorRuntimeSet::default());
        series.footprints = Some(BTreeMap::new());
        series
    }

    pub fn color(&self) -> SeriesColor {
        self.color
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

    pub fn indicator_snapshot(&self, id: IndicatorInstanceId) -> IndicatorRuntimeSnapshot {
        self.indicators
            .as_ref()
            .map(|indicators| indicators.snapshot_for(id))
            .unwrap_or_default()
    }

    /// Whether this indicator currently produces a signed-overlay field for any
    /// bar, derived from the same produced [`IndicatorData`] the renderer
    /// consumes.
    pub fn indicator_produces_signed_overlay(&self, id: IndicatorInstanceId) -> bool {
        self.indicators
            .as_ref()
            .is_some_and(|indicators| indicators.signed_overlay_available(id))
    }

    pub(crate) fn set_active_indicators(
        &mut self,
        active: &[IndicatorInstanceId],
        instances: &IndicatorInstanceStore,
    ) {
        if let Some(indicators) = self.indicators.as_mut() {
            indicators.set_active(active, instances);
        }
    }

    pub(crate) fn refresh_indicator_inputs(&mut self, instances: &IndicatorInstanceStore) {
        self.refresh_indicator_inputs_from(instances, IndicatorUpdateScope::Full);
    }

    pub(crate) fn refresh_indicator_inputs_from(
        &mut self,
        instances: &IndicatorInstanceStore,
        scope: IndicatorUpdateScope,
    ) {
        if let Some(indicators) = self.indicators.as_mut() {
            let times =
                (self.tick_times.len() == self.bars.len()).then_some(self.tick_times.as_slice());
            indicators.on_input_from(
                IndicatorInput::Klines {
                    bars: &self.bars,
                    times,
                },
                instances,
                &self.bars,
                scope,
            );
        }
    }

    pub fn configure_indicators(
        &mut self,
        instances: &IndicatorInstanceStore,
        basis: Basis,
        active: &[IndicatorInstanceId],
    ) {
        self.set_active_indicators(active, instances);
        self.refresh_indicators(instances, basis);
    }

    pub fn refresh_indicators(&mut self, instances: &IndicatorInstanceStore, basis: Basis) {
        self.refresh_indicator_inputs(instances);
        let trade_bars = self.normalized_trade_bars(basis);
        self.insert_indicator_input(IndicatorInput::Trades { bars: &trade_bars }, instances);
    }

    pub fn update_indicators_after_trades(
        &mut self,
        instances: &IndicatorInstanceStore,
        basis: Basis,
        scope: IndicatorUpdateScope,
    ) {
        if matches!(basis, Basis::Tick(_)) {
            self.refresh_indicator_inputs_from(instances, scope);
        }

        let trade_bars = self.normalized_trade_bars(basis);
        self.insert_indicator_input_from(
            IndicatorInput::Trades { bars: &trade_bars },
            instances,
            scope,
        );
    }

    pub fn update_indicators_after_klines(
        &mut self,
        instances: &IndicatorInstanceStore,
        scope: IndicatorUpdateScope,
    ) {
        self.refresh_indicator_inputs_from(instances, scope);
    }

    pub fn time_at_index(&self, index: usize) -> Option<UnixMs> {
        self.tick_times
            .get(index)
            .copied()
            .or_else(|| self.bars.get(index).map(|bar| bar.time))
    }

    pub fn insert_klines(&mut self, klines: &[Kline], basis: Basis) -> bool {
        let incoming = Self::normalize_klines(klines, basis);
        if incoming.is_empty() {
            return false;
        }

        Self::merge_bars(&mut self.bars, incoming);
        self.trim_bars();
        self.trim_trade_storage(false);
        true
    }

    /// Returns the first and last missing timeframe slots in the requested range.
    pub fn missing_kline_range(
        &self,
        earliest: UnixMs,
        latest: UnixMs,
        timeframe: Timeframe,
    ) -> Option<(UnixMs, UnixMs)> {
        let interval = timeframe.to_milliseconds();
        if interval == 0 {
            return None;
        }

        let series_earliest = self.bars.first()?.time;
        let series_latest = self.bars.last()?.time;
        let phase = UnixMs::new(series_earliest.as_u64() % interval);
        let check_earliest = align_down_to_phase(earliest.max(series_earliest), phase, interval)
            .max(series_earliest);
        let check_latest =
            align_down_to_phase(latest.min(series_latest), phase, interval).min(series_latest);

        if check_earliest >= check_latest {
            return None;
        }

        let mut expected = check_earliest;
        let mut missing_earliest = None;
        let mut missing_latest = None;

        for _ in 0..MAX_INTEGRITY_SCAN {
            if expected >= check_latest {
                break;
            }

            if self
                .bars
                .binary_search_by_key(&expected, |bar| bar.time)
                .is_err()
            {
                missing_earliest.get_or_insert(expected);
                missing_latest = Some(expected);
            }

            let Some(next) = expected.as_u64().checked_add(interval) else {
                break;
            };
            expected = UnixMs::new(next);
        }

        missing_earliest.zip(missing_latest)
    }

    pub fn update_latest_kline(
        &mut self,
        kline: &Kline,
        basis: Basis,
    ) -> Option<IndicatorUpdateScope> {
        let new_bar = Self::normalize_kline(kline, basis);
        let changed = match self.bars.last_mut() {
            Some(last) if last.time == new_bar.time => {
                *last = new_bar;
                true
            }
            Some(last) if new_bar.time > last.time => {
                self.bars.push(new_bar);
                true
            }
            Some(_) => return None,
            None => {
                self.bars.push(new_bar);
                true
            }
        };

        if !changed {
            return None;
        }

        self.trim_bars();
        self.trim_trade_storage(false);
        Some(IndicatorUpdateScope::From(
            self.bars.len().saturating_sub(1),
        ))
    }

    pub fn indicator_data_for_panel_value(
        &self,
        panel_value: Option<PanelValueId>,
        bar: &Kline,
    ) -> Option<IndicatorData> {
        let indicators = self.indicators.as_ref()?;
        indicators.data_for_indicator(panel_value, bar)
    }

    pub fn indicator_data_for_panel_value_at(
        &self,
        panel_value: Option<PanelValueId>,
        index: usize,
    ) -> Option<IndicatorData> {
        let indicators = self.indicators.as_ref()?;
        indicators.data_for_indicator_at(panel_value, index)
    }

    pub fn enable_tick_aggregation(&mut self, interval: TickCount, footprint_tick_size: PriceStep) {
        self.clear_profile_caches();
        self.tick_index_offset = 0;
        self.bars.clear();
        if let Some(indicators) = self.indicators.as_mut() {
            indicators.clear();
        }
        self.trade_values.clear();
        self.trade_time_range = None;
        self.last_trade_retention_start = None;
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
        self.clear_profile_caches();
        self.bars.clear();
        if let Some(indicators) = self.indicators.as_mut() {
            indicators.clear();
        }
        if let Some(footprints) = self.footprints.as_mut() {
            footprints.clear();
        }
        self.trade_values.clear();
        self.trade_time_range = None;
        self.last_trade_retention_start = None;
        self.disable_tick_aggregation();
        self.tick_index_offset = 0;
        self.time_trade_timeframe = None;

        if !preserve_raw_trades {
            self.raw_trades.clear();
        }
    }

    pub fn insert_tick_trades(&mut self, trades: &[Trade]) -> IndicatorUpdateScope {
        if self.tick_aggr.is_none() {
            return IndicatorUpdateScope::From(self.bars.len());
        }

        self.recurring_profile_caches.clear();
        self.raw_trades.extend_from_slice(trades);

        let first_updated = {
            let tick_aggr = self
                .tick_aggr
                .as_mut()
                .expect("tick aggregation was checked above");
            tick_aggr.insert_trades(trades).unwrap_or_default()
        };
        let first_updated = self.sync_bars_from_tick_aggregation(first_updated);
        self.update_footprint_poc_cache_for_ticks(first_updated);
        IndicatorUpdateScope::From(first_updated)
    }

    pub fn insert_time_trades(
        &mut self,
        trades: &[Trade],
        timeframe: Timeframe,
    ) -> IndicatorUpdateScope {
        if self.footprints.is_none() {
            return IndicatorUpdateScope::From(self.bars.len());
        }

        self.raw_trades.extend_from_slice(trades);
        self.apply_time_trades(trades, timeframe)
    }

    fn apply_time_trades(
        &mut self,
        trades: &[Trade],
        timeframe: Timeframe,
    ) -> IndicatorUpdateScope {
        if self.time_trade_timeframe != Some(timeframe) {
            self.clear_profile_caches();
            self.time_trade_timeframe = Some(timeframe);
        }

        let mut updated_times = BTreeSet::new();

        for trade in trades {
            let bar_time = trade.time.floor_to(timeframe);
            updated_times.insert(bar_time);

            self.trade_values
                .entry(bar_time)
                .or_default()
                .add_trade(trade);
            self.extend_trade_time_range(trade.time);
        }

        let Some(footprints) = self.footprints.as_mut() else {
            return IndicatorUpdateScope::From(self.bars.len());
        };
        for trade in trades {
            let bar_time = trade.time.floor_to(timeframe);
            footprints
                .entry(bar_time)
                .or_insert_with(KlineTrades::new)
                .add_trade_to_nearest_bin(trade, self.footprint_tick_size);
        }

        for time in updated_times.iter() {
            if let Some(footprint) = footprints.get_mut(time) {
                footprint.calculate_poc();
            }
        }

        self.update_recurring_profile_caches(trades);
        self.update_footprint_poc_cache_for_times(&updated_times);
        let has_trade_before_storage_start = self
            .last_trade_retention_start
            .is_some_and(|start| trades.iter().any(|trade| trade.time < start));
        self.trim_trade_storage(has_trade_before_storage_start);

        let first_time = updated_times.first().copied();
        let first_index = first_time
            .map(|time| self.first_bar_at_or_after(time))
            .unwrap_or(self.bars.len());
        IndicatorUpdateScope::From(first_index)
    }

    pub fn set_footprint_tick_size(&mut self, tick_size: PriceStep) {
        if self.footprint_tick_size == tick_size {
            return;
        }
        self.footprint_tick_size = tick_size;
        self.clear_profile_caches();
    }

    pub fn rebuild_time_footprints(&mut self, timeframe: Timeframe) {
        self.clear_profile_caches();
        if let Some(footprints) = self.footprints.as_mut() {
            footprints.clear();
        } else {
            return;
        }

        self.trade_values.clear();
        self.trade_time_range = None;
        self.last_trade_retention_start = None;
        let raw_trades = self.raw_trades.clone();
        self.apply_time_trades(&raw_trades, timeframe);
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
        self.trim_trade_storage(false);
    }

    pub fn configure_footprint_poc_cache(&mut self, enabled: bool) {
        match (enabled, self.footprint_poc_cache.is_some()) {
            (true, false) => self.footprint_poc_cache = Some(self.build_footprint_poc_cache()),
            (false, true) => self.footprint_poc_cache = None,
            _ => {}
        }
    }

    pub fn footprint_poc_lines(&self, earliest: UnixMs, latest: UnixMs) -> Vec<FootprintPocLine> {
        self.footprint_poc_cache
            .as_ref()
            .map(|cache| cache.visible_lines(earliest, latest))
            .unwrap_or_default()
    }

    fn clear_profile_caches(&mut self) {
        self.recurring_profile_caches.clear();
        self.footprint_poc_cache = None;
    }

    fn update_recurring_profile_caches(&mut self, trades: &[Trade]) {
        if trades.is_empty() {
            return;
        }

        for cache in self.recurring_profile_caches.values_mut() {
            cache.insert_trades(trades, cache.tick_size);
        }
    }

    fn update_footprint_poc_cache_for_times(&mut self, times: &BTreeSet<UnixMs>) {
        let Some(footprints) = self.footprints.as_ref() else {
            return;
        };
        let Some(cache) = self.footprint_poc_cache.as_mut() else {
            return;
        };

        cache.update_footprints(
            times
                .iter()
                .filter_map(|&time| footprints.get(&time).map(|footprint| (time, footprint))),
        );
    }

    fn update_footprint_poc_cache_for_ticks(&mut self, first_updated: usize) {
        let Some(tick_aggr) = self.tick_aggr.as_ref() else {
            return;
        };
        let Some(cache) = self.footprint_poc_cache.as_mut() else {
            return;
        };

        cache.update_footprints(
            tick_aggr
                .datapoints
                .iter()
                .enumerate()
                .skip(first_updated)
                .map(|(index, point)| {
                    (
                        UnixMs::new(self.tick_index_offset + index as u64),
                        &point.footprint,
                    )
                }),
        );
    }

    fn build_footprint_poc_cache(&self) -> FootprintPocCache {
        let mut cache = FootprintPocCache::new();

        if let Some(tick_aggr) = self.tick_aggr.as_ref() {
            cache.update_footprints(tick_aggr.datapoints.iter().enumerate().map(
                |(index, point)| {
                    (
                        UnixMs::new(self.tick_index_offset + index as u64),
                        &point.footprint,
                    )
                },
            ));
        } else if let Some(footprints) = self.footprints.as_ref() {
            cache.update_footprints(
                footprints
                    .iter()
                    .map(|(&time, footprint)| (time, footprint)),
            );
        }

        cache
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

        if can_use_footprints {
            if let Some(footprints) = self.footprints.as_ref() {
                for (&time, footprint) in footprints {
                    cache.add_footprint(time, footprint);
                }
            }
        } else {
            for trade in &self.raw_trades {
                cache.add_trade(trade, tick_size);
            }
        }

        cache.rebuild_poc_statuses();
        cache
    }

    pub fn trade_time_range(&self) -> Option<(UnixMs, UnixMs)> {
        let (earliest, latest) = self.trade_time_range?;
        Some((earliest, latest))
    }

    pub fn normalized_trade_bars(&self, basis: Basis) -> Vec<NormalizedTradeBar> {
        match basis {
            Basis::Time(timeframe) => self.normalized_time_trade_bars(timeframe),
            Basis::Tick(_) => self.normalized_tick_trade_bars(),
        }
    }

    fn normalized_time_trade_bars(&self, timeframe: Timeframe) -> Vec<NormalizedTradeBar> {
        let coverage = self.trade_time_range;
        let coverage = coverage
            .map(|(earliest, latest)| (earliest.floor_to(timeframe), latest.floor_to(timeframe)));

        self.bars
            .iter()
            .map(|bar| {
                let values = coverage
                    .filter(|(earliest, latest)| bar.time >= *earliest && bar.time <= *latest)
                    .map(|_| {
                        self.trade_values
                            .get(&bar.time)
                            .copied()
                            .unwrap_or_default()
                    });
                NormalizedTradeBar {
                    time: bar.time,
                    values,
                }
            })
            .collect()
    }

    fn normalized_tick_trade_bars(&self) -> Vec<NormalizedTradeBar> {
        let datapoints = self
            .tick_aggr
            .as_ref()
            .map(|aggregation| &aggregation.datapoints);

        self.bars
            .iter()
            .enumerate()
            .map(|(index, bar)| NormalizedTradeBar {
                time: bar.time,
                values: datapoints
                    .and_then(|datapoints| datapoints.get(index))
                    .map(|datapoint| TradeBarValues {
                        trade_count: datapoint.tick_count as u64,
                        buy_qty: datapoint.kline.volume.buy_qty_or_zero(),
                        sell_qty: datapoint.kline.volume.sell_qty_or_zero(),
                    }),
            })
            .collect()
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

    pub fn footprint_tick_index_offset(&self) -> u64 {
        self.tick_index_offset
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

        if self.tick_aggr.is_some() {
            for bar in &self.bars {
                if bar.time < earliest || bar.time > latest {
                    continue;
                }

                if let Some(footprint) = self.footprint_for_bar(bar) {
                    add_footprint(footprint);
                }
            }
        } else if let Some(footprints) = self.footprints.as_ref() {
            for (_, footprint) in footprints.range(earliest..=latest) {
                add_footprint(footprint);
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

    pub fn rebuild_bars_from_tick_aggregation(&mut self) {
        if self.tick_aggr.is_none() {
            self.bars.clear();
            return;
        }

        self.sync_bars_from_tick_aggregation(0);
    }

    pub fn insert_indicator_input(
        &mut self,
        input: IndicatorInput<'_>,
        instances: &IndicatorInstanceStore,
    ) {
        if let Some(indicators) = self.indicators.as_mut() {
            indicators.on_input(input, instances, &self.bars);
        }
    }

    pub fn insert_indicator_input_from(
        &mut self,
        input: IndicatorInput<'_>,
        instances: &IndicatorInstanceStore,
        scope: IndicatorUpdateScope,
    ) {
        if let Some(indicators) = self.indicators.as_mut() {
            indicators.on_input_from(input, instances, &self.bars, scope);
        }
    }

    fn sync_bars_from_tick_aggregation(&mut self, first_updated: usize) -> usize {
        let Some(tick_aggr) = self.tick_aggr.as_mut() else {
            self.bars.clear();
            self.tick_times.clear();
            return 0;
        };

        let to_drop = tick_aggr.datapoints.len().saturating_sub(SERIES_MAX_BARS);
        let to_drop = if to_drop >= TICK_CLEANUP_BATCH {
            to_drop
        } else {
            0
        };
        if to_drop > 0 {
            tick_aggr.datapoints.drain(0..to_drop);
            self.tick_index_offset = self.tick_index_offset.saturating_add(to_drop as u64);
        }

        let first_index = if to_drop > 0 { 0 } else { first_updated };
        let data_len = tick_aggr.datapoints.len();
        self.bars.truncate(data_len);
        self.tick_times.truncate(data_len);
        if self.bars.len() < data_len
            && let Some(template) = tick_aggr.datapoints.first().map(|point| point.kline)
        {
            self.bars.resize(data_len, template);
        }
        self.tick_times.resize(data_len, UnixMs::new(0));

        for (index, point) in tick_aggr.datapoints.iter().enumerate().skip(first_index) {
            self.tick_times[index] = point.kline.time;
            let mut bar = point.kline;
            // Tick indicators are keyed by bar time. Synthetic monotonic
            // timestamps preserve one-to-one identity even when trades share
            // an exchange timestamp.
            bar.time = UnixMs::new(index as u64);
            self.bars[index] = bar;
        }

        self.trim_trade_storage(false);
        if to_drop > 0
            && let Some(cache) = self.footprint_poc_cache.as_mut()
        {
            cache.retain_from(UnixMs::new(self.tick_index_offset));
        }

        first_index
    }

    fn trim_trade_storage(&mut self, force: bool) {
        let (retention_start, earliest_bar) = if let Some(tick_aggr) = self.tick_aggr.as_ref() {
            let Some(retention_start) = tick_aggr.datapoints.first().map(|point| point.kline.time)
            else {
                return;
            };
            (retention_start, None)
        } else {
            let Some(earliest_bar) = self.bars.first().map(|bar| bar.time) else {
                return;
            };
            let retention_start = self
                .recurring_profile_caches
                .values()
                .map(|cache| earliest_bar.saturating_sub(cache.interval.to_milliseconds()))
                .min()
                .unwrap_or(earliest_bar);
            (retention_start, Some(earliest_bar))
        };

        let retention_start_changed = self.last_trade_retention_start != Some(retention_start);
        if !force && !retention_start_changed {
            return;
        }

        let mut retained_trade_time_range: Option<(UnixMs, UnixMs)> = None;
        self.raw_trades.retain(|trade| {
            if trade.time < retention_start {
                return false;
            }

            retained_trade_time_range = Some(match retained_trade_time_range {
                Some((earliest, latest)) => (earliest.min(trade.time), latest.max(trade.time)),
                None => (trade.time, trade.time),
            });
            true
        });
        if self.raw_trades.len().saturating_mul(2) < self.raw_trades.capacity() {
            self.raw_trades.shrink_to_fit();
        }
        self.trade_time_range = retained_trade_time_range;

        if let Some(earliest_bar) = earliest_bar {
            if let Some(footprints) = self.footprints.as_mut() {
                while footprints
                    .first_key_value()
                    .is_some_and(|(&time, _)| time < retention_start)
                {
                    footprints.pop_first();
                }
            }
            while self
                .trade_values
                .first_key_value()
                .is_some_and(|(&time, _)| time < earliest_bar)
            {
                self.trade_values.pop_first();
            }
            for cache in self.recurring_profile_caches.values_mut() {
                let cache_start = earliest_bar.saturating_sub(cache.interval.to_milliseconds());
                cache.retain_from(cache_start.max(retention_start));
            }
            if let Some(cache) = self.footprint_poc_cache.as_mut() {
                cache.retain_from(earliest_bar);
            }
        }

        self.last_trade_retention_start = Some(retention_start);
    }

    fn extend_trade_time_range(&mut self, time: UnixMs) {
        self.trade_time_range = Some(match self.trade_time_range {
            Some((earliest, latest)) => (earliest.min(time), latest.max(time)),
            None => (time, time),
        });
    }

    fn first_bar_at_or_after(&self, time: UnixMs) -> usize {
        self.bars
            .binary_search_by_key(&time, |bar| bar.time)
            .unwrap_or_else(|index| index)
    }

    fn normalize_kline(kline: &Kline, basis: Basis) -> Kline {
        let mut adjusted = *kline;
        adjusted.time = match basis {
            Basis::Time(timeframe) => kline.time.floor_to(timeframe),
            Basis::Tick(_) => kline.time,
        };
        adjusted
    }

    fn normalize_klines(klines: &[Kline], basis: Basis) -> Vec<Kline> {
        let mut normalized: Vec<Kline> = klines
            .iter()
            .map(|kline| Self::normalize_kline(kline, basis))
            .collect();

        normalized.sort_by_key(|bar| bar.time);
        normalized.dedup_by_key(|bar| bar.time);
        normalized
    }

    fn merge_bars(dst: &mut Vec<Kline>, mut incoming: Vec<Kline>) {
        if incoming.is_empty() {
            return;
        }

        if dst.is_empty() {
            *dst = incoming;
            return;
        }

        incoming.sort_by_key(|bar| bar.time);
        incoming.dedup_by_key(|bar| bar.time);

        let mut dst_index = 0;
        let mut incoming_index = 0;
        let mut merged = Vec::with_capacity(dst.len() + incoming.len());

        while dst_index < dst.len() && incoming_index < incoming.len() {
            let existing = dst[dst_index];
            let next = incoming[incoming_index];

            if existing.time < next.time {
                merged.push(existing);
                dst_index += 1;
            } else if next.time < existing.time {
                merged.push(next);
                incoming_index += 1;
            } else {
                merged.push(next);
                dst_index += 1;
                incoming_index += 1;
            }
        }

        if dst_index < dst.len() {
            merged.extend_from_slice(&dst[dst_index..]);
        }

        if incoming_index < incoming.len() {
            merged.extend_from_slice(&incoming[incoming_index..]);
        }

        *dst = merged;
    }

    fn trim_bars(&mut self) {
        if self.bars.len() > SERIES_MAX_BARS {
            let to_drop = self.bars.len() - SERIES_MAX_BARS;
            self.bars.drain(0..to_drop);
        }
    }
}

fn align_down_to_phase(time: UnixMs, phase: UnixMs, interval: u64) -> UnixMs {
    if time >= phase {
        let timestamp = time.as_u64();
        let phase = phase.as_u64();
        UnixMs::new(timestamp.saturating_sub((timestamp - phase) % interval))
    } else {
        phase
    }
}
