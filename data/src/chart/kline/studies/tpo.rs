use super::MAX_PROFILE_PRICE_ROWS;
use crate::config::theme::{ThemeAccent, ThemeColor, ThemeColorRole, ThemeColorShade};
use chrono::{Datelike, Months, NaiveDate};
use exchange::unit::price::{Price, PriceStep};
use exchange::{Kline, Timeframe, Trade, UnixMs};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const TPO_TRADE_COVERAGE_TIMEFRAME: Timeframe = Timeframe::M1;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TpoAnchor {
    #[default]
    Daily,
    Weekly,
    Monthly,
}

impl TpoAnchor {
    pub const ALL: [Self; 3] = [Self::Daily, Self::Weekly, Self::Monthly];

    pub fn floor(self, time: UnixMs) -> UnixMs {
        match self {
            Self::Daily => time.floor_to(Timeframe::D1),
            Self::Weekly => {
                let Some(date_time) = time.as_datetime_utc() else {
                    return time.floor_to(Timeframe::D1);
                };
                let days_from_monday = u64::from(date_time.weekday().num_days_from_monday());
                UnixMs::new(
                    time.floor_to(Timeframe::D1)
                        .as_u64()
                        .saturating_sub(days_from_monday * Timeframe::D1.to_milliseconds()),
                )
            }
            Self::Monthly => time
                .as_datetime_utc()
                .and_then(|date_time| {
                    let date = date_time.date_naive();
                    NaiveDate::from_ymd_opt(date.year(), date.month(), 1)
                })
                .and_then(Self::midnight)
                .unwrap_or(time),
        }
    }

    pub fn offset(self, time: UnixMs, periods: i64) -> UnixMs {
        match self {
            Self::Daily => time.offset_by_timeframe(Timeframe::D1, periods),
            Self::Weekly => time.offset_by_timeframe(Timeframe::D1, periods.saturating_mul(7)),
            Self::Monthly => {
                let Some(date) = time
                    .as_datetime_utc()
                    .map(|date_time| date_time.date_naive())
                else {
                    return time;
                };
                let Ok(months) = u32::try_from(periods.unsigned_abs()) else {
                    return time;
                };
                let months = Months::new(months);
                let date = if periods < 0 {
                    date.checked_sub_months(months)
                } else {
                    date.checked_add_months(months)
                };
                date.and_then(Self::midnight).unwrap_or(time)
            }
        }
    }

    fn midnight(date: NaiveDate) -> Option<UnixMs> {
        u64::try_from(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis())
            .ok()
            .map(UnixMs::from_millis)
    }
}

impl std::fmt::Display for TpoAnchor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Daily => "Daily",
            Self::Weekly => "Weekly",
            Self::Monthly => "Monthly",
        })
    }
}

pub const DEFAULT_TPO_LETTER_PERIOD: Timeframe = Timeframe::M30;
pub const DEFAULT_TPO_VALUE_AREA_PERCENT: u8 = 70;
pub const MIN_TPO_VALUE_AREA_PERCENT: u8 = 5;
pub const MAX_TPO_VALUE_AREA_PERCENT: u8 = 100;

pub(super) const fn default_tpo_letter_period() -> Timeframe {
    DEFAULT_TPO_LETTER_PERIOD
}

pub(super) const fn default_tpo_anchor() -> TpoAnchor {
    TpoAnchor::Daily
}

pub(super) const fn default_tpo_value_area_percent() -> u8 {
    DEFAULT_TPO_VALUE_AREA_PERCENT
}

pub(super) const fn default_tpo_visible() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TpoColorConfig {
    pub letters: ThemeColor,
    pub poc: ThemeColor,
    pub value_area: ThemeColor,
    pub initial_balance: ThemeColor,
    pub single_print: ThemeColor,
    pub poor_high: ThemeColor,
    pub poor_low: ThemeColor,
    #[serde(default)]
    pub show_poc: bool,
    #[serde(default)]
    pub show_initial_balance: bool,
    #[serde(default)]
    pub show_single_print: bool,
    #[serde(default)]
    pub show_poor_high: bool,
    #[serde(default)]
    pub show_poor_low: bool,
}

impl Default for TpoColorConfig {
    fn default() -> Self {
        Self::defaults()
    }
}

impl TpoColorConfig {
    pub const fn defaults() -> Self {
        Self {
            letters: ThemeColor::new(ThemeColorRole::Neutral, ThemeColorShade::Soft),
            poc: ThemeColor::new(ThemeColorRole::Warning, ThemeColorShade::Strong),
            value_area: ThemeColor::new(ThemeColorRole::Secondary, ThemeColorShade::Strong),
            initial_balance: ThemeColor::new(ThemeColorRole::Primary, ThemeColorShade::Base),
            single_print: ThemeColor::with_accent(
                ThemeColorRole::Neutral,
                Some(ThemeAccent::Three),
                ThemeColorShade::Base,
            ),
            poor_high: ThemeColor::with_accent(
                ThemeColorRole::Primary,
                Some(ThemeAccent::Seven),
                ThemeColorShade::Base,
            ),
            poor_low: ThemeColor::with_accent(
                ThemeColorRole::Primary,
                Some(ThemeAccent::Seven),
                ThemeColorShade::Base,
            ),
            show_poc: false,
            show_initial_balance: false,
            show_single_print: false,
            show_poor_high: false,
            show_poor_low: false,
        }
    }

    pub const fn with_show_poc(self, show_poc: bool) -> Self {
        Self { show_poc, ..self }
    }

    pub const fn with_show_initial_balance(self, show_initial_balance: bool) -> Self {
        Self {
            show_initial_balance,
            ..self
        }
    }

    pub const fn with_show_single_print(self, show_single_print: bool) -> Self {
        Self {
            show_single_print,
            ..self
        }
    }

    pub const fn with_show_poor_extremes(self, show: bool) -> Self {
        Self {
            show_poor_high: show,
            show_poor_low: show,
            ..self
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TpoProfileSettings {
    pub anchor: TpoAnchor,
    pub letter_period: Timeframe,
    pub price_step: PriceStep,
    pub value_area_percent: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TpoBoundaryEdits {
    #[serde(default)]
    pub added: Vec<UnixMs>,
    #[serde(default)]
    pub removed: Vec<UnixMs>,
}

impl TpoBoundaryEdits {
    pub fn clear(&mut self) {
        self.added.clear();
        self.removed.clear();
    }

    pub fn has_boundary_in_range(&self, start: UnixMs, end: UnixMs) -> bool {
        let (start, end) = (start.min(end), start.max(end));
        self.added
            .iter()
            .chain(self.removed.iter())
            .any(|boundary| *boundary >= start && *boundary <= end)
    }

    pub fn clear_range(&mut self, start: UnixMs, end: UnixMs, anchor: TpoAnchor) {
        let (start, end) = (start.min(end), start.max(end));
        let added = self
            .added
            .iter()
            .copied()
            .filter(|boundary| *boundary >= start && *boundary <= end)
            .collect::<Vec<_>>();
        let mut related_removed = BTreeSet::new();
        for boundary in added {
            if boundary == start {
                let floor = anchor.floor(boundary);
                let previous = if floor < boundary {
                    floor
                } else {
                    anchor.offset(floor, -1)
                };
                self.add_removed_boundary_run(previous, -1, anchor, &mut related_removed);
            }
            if boundary == end {
                let floor = anchor.floor(boundary);
                let next = if floor > boundary {
                    floor
                } else {
                    anchor.offset(floor, 1)
                };
                self.add_removed_boundary_run(next, 1, anchor, &mut related_removed);
            }
        }
        self.added
            .retain(|boundary| *boundary < start || *boundary > end);
        self.removed.retain(|boundary| {
            (*boundary < start || *boundary > end) && !related_removed.contains(boundary)
        });
    }

    fn add_removed_boundary_run(
        &self,
        seed: UnixMs,
        direction: i64,
        anchor: TpoAnchor,
        related_removed: &mut BTreeSet<UnixMs>,
    ) {
        if !self.removed.contains(&seed) {
            return;
        }
        let mut boundary = seed;
        loop {
            if !self.removed.contains(&boundary) {
                break;
            }
            related_removed.insert(boundary);
            let next = anchor.offset(boundary, direction);
            if (direction < 0 && next >= boundary) || (direction > 0 && next <= boundary) {
                break;
            }
            boundary = next;
        }
    }

    pub fn split_at(&mut self, boundary: UnixMs) {
        self.removed.retain(|value| *value != boundary);
        if !self.added.contains(&boundary) {
            self.added.push(boundary);
            self.added.sort_unstable();
        }
    }

    pub fn merge_at(&mut self, boundary: UnixMs) {
        self.added.retain(|value| *value != boundary);
        if !self.removed.contains(&boundary) {
            self.removed.push(boundary);
            self.removed.sort_unstable();
        }
    }

    pub fn move_boundary(&mut self, from: UnixMs, to: UnixMs, anchor: TpoAnchor) {
        if from == to {
            return;
        }

        let first = from.min(to);
        let last = from.max(to);
        self.added.retain(|value| *value != from);
        self.added
            .retain(|value| !(*value > first && *value < last));
        self.removed.retain(|value| *value != from && *value != to);

        if anchor.floor(from) == from {
            self.removed.push(from);
        }

        let mut boundary = anchor.floor(first);
        while boundary < last {
            if boundary > first {
                self.removed.push(boundary);
            }
            let next = anchor.offset(boundary, 1);
            if next <= boundary {
                break;
            }
            boundary = next;
        }

        if anchor.floor(to) != to {
            self.added.push(to);
        }

        self.added.sort_unstable();
        self.added.dedup();
        self.removed.sort_unstable();
        self.removed.dedup();
    }

    pub fn profile_ranges_overlapping(
        &self,
        start: UnixMs,
        end: UnixMs,
        anchor: TpoAnchor,
    ) -> Vec<(UnixMs, UnixMs)> {
        let (start, end) = (start.min(end), start.max(end));
        if start >= end {
            return Vec::new();
        }

        let mut first = anchor.floor(start);
        while self.removed.contains(&first) {
            let previous = anchor.offset(first, -1);
            if previous >= first {
                break;
            }
            first = previous;
        }

        let mut last = anchor.offset(anchor.floor(end), 1);
        while self.removed.contains(&last) {
            let next = anchor.offset(last, 1);
            if next <= last {
                break;
            }
            last = next;
        }

        self.boundaries_between(first, last, anchor)
            .windows(2)
            .filter_map(|window| {
                let profile_start = window[0];
                let profile_end = window[1];
                (profile_start < end && profile_end > start).then_some((profile_start, profile_end))
            })
            .collect()
    }

    fn boundaries_between(&self, first: UnixMs, last: UnixMs, anchor: TpoAnchor) -> Vec<UnixMs> {
        let mut boundaries = BTreeSet::new();
        let mut boundary = first;
        while boundary <= last {
            boundaries.insert(boundary);
            let next = anchor.offset(boundary, 1);
            if next <= boundary {
                break;
            }
            boundary = next;
        }

        boundaries.extend(
            self.added
                .iter()
                .copied()
                .filter(|value| *value > first && *value < last),
        );
        for removed in &self.removed {
            boundaries.remove(removed);
        }

        boundaries.into_iter().collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TpoProfileStatus {
    Developing,
    Complete,
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TpoRow {
    pub price: Price,
    pub periods: Vec<u16>,
}

impl TpoRow {
    pub fn count(&self) -> usize {
        self.periods.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TpoProfile {
    pub start: UnixMs,
    pub end: UnixMs,
    pub period_count: usize,
    pub rows: Vec<TpoRow>,
    pub session_high: Option<Price>,
    pub session_low: Option<Price>,
    pub initial_balance_high: Option<Price>,
    pub initial_balance_low: Option<Price>,
    pub poc: Option<Price>,
    pub vah: Option<Price>,
    pub val: Option<Price>,
    pub single_prints: Vec<Price>,
    pub poor_highs: Vec<Price>,
    pub poor_lows: Vec<Price>,
    pub status: TpoProfileStatus,
}

impl TpoProfile {
    pub fn letter_label(index: usize) -> String {
        const LETTERS: &[u8; 52] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
        let mut value = index;
        let mut label = Vec::new();
        loop {
            label.push(char::from(LETTERS[value % LETTERS.len()]));
            if value < LETTERS.len() {
                break;
            }
            value = value / LETTERS.len() - 1;
        }
        label.into_iter().rev().collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TpoCalculationError {
    InvalidLetterPeriod,
    InvalidPriceStep,
    TooManyRows {
        start: UnixMs,
        end: UnixMs,
        rows: usize,
    },
}

pub struct TradeTpoCache {
    settings: TpoProfileSettings,
    edits: TpoBoundaryEdits,
    trade_count: usize,
    profiles: BTreeMap<UnixMs, TradeTpoProfileCache>,
}

struct TradeTpoProfileCache {
    start: UnixMs,
    end: UnixMs,
    periods: Vec<TradeTpoPeriodCache>,
    session_high: Option<Price>,
    session_low: Option<Price>,
    initial_balance_high: Option<Price>,
    initial_balance_low: Option<Price>,
}

struct TradeTpoPeriodCache {
    start: UnixMs,
    end: UnixMs,
    high: Option<Price>,
    low: Option<Price>,
    rows: BTreeSet<Price>,
}

impl TradeTpoCache {
    pub fn new(
        trades: &[Trade],
        settings: TpoProfileSettings,
        edits: TpoBoundaryEdits,
    ) -> Result<Self, TpoCalculationError> {
        validate_settings(settings)?;
        let mut cache = Self {
            settings,
            edits,
            trade_count: 0,
            profiles: BTreeMap::new(),
        };
        if !cache.sync(trades) {
            unreachable!("trade cache ranges cover all source trades");
        }
        Ok(cache)
    }

    pub fn sync(&mut self, trades: &[Trade]) -> bool {
        if trades.len() < self.trade_count {
            return false;
        }
        let new_trades = &trades[self.trade_count..];
        if let Some((first, last)) = new_trades.iter().map(|trade| trade.time).fold(
            None,
            |range: Option<(UnixMs, UnixMs)>, time| {
                Some(range.map_or((time, time), |(first, last)| {
                    (first.min(time), last.max(time))
                }))
            },
        ) {
            self.extend_windows(first, last.saturating_add(1));
        }
        for trade in new_trades {
            let start = self
                .profile_start_for(trade.time)
                .expect("trade cache ranges cover all source trades");
            self.profiles
                .get_mut(&start)
                .expect("trade cache range exists")
                .add_trade(trade, self.settings);
        }
        self.trade_count = trades.len();
        true
    }

    pub fn build_profile(
        &self,
        start: UnixMs,
        now: UnixMs,
        status: TpoProfileStatus,
    ) -> Option<Result<TpoProfile, TpoCalculationError>> {
        self.profiles
            .get(&start)
            .map(|profile| profile.build(self.settings, now, status))
    }

    pub fn build_profiles(
        &self,
        now: UnixMs,
        status: impl Fn(UnixMs, UnixMs) -> TpoProfileStatus,
    ) -> Result<Vec<TpoProfile>, TpoCalculationError> {
        self.profiles
            .values()
            .map(|profile| profile.build(self.settings, now, status(profile.start, profile.end)))
            .collect()
    }

    fn profile_start_for(&self, time: UnixMs) -> Option<UnixMs> {
        self.profiles
            .range(..=time)
            .next_back()
            .and_then(|(&start, profile)| (time < profile.end).then_some(start))
    }

    fn extend_windows(&mut self, start: UnixMs, end: UnixMs) {
        let anchor = self.settings.anchor;

        let start = anchor.floor(start);
        let end = anchor.offset(anchor.floor(end.saturating_sub(1)), 1);

        let (start, end) = match (
            self.profiles.first_key_value(),
            self.profiles.last_key_value(),
        ) {
            (Some((&first, _)), Some((_, last))) => {
                if start >= first && end <= last.end {
                    return;
                }
                (start.min(first), end.max(last.end))
            }
            _ => (start, end),
        };

        for (start, end) in self
            .edits
            .profile_ranges_overlapping(start, end, self.settings.anchor)
        {
            self.profiles.entry(start).or_insert_with(|| {
                TradeTpoProfileCache::new(start, end, self.settings.letter_period)
            });
        }
    }
}

impl TradeTpoProfileCache {
    fn new(start: UnixMs, end: UnixMs, letter_period: Timeframe) -> Self {
        let mut periods = Vec::new();
        let mut period_start = start;
        while period_start < end {
            let period_end = period_start.offset_by_timeframe(letter_period, 1).min(end);
            periods.push(TradeTpoPeriodCache {
                start: period_start,
                end: period_end,
                high: None,
                low: None,
                rows: BTreeSet::new(),
            });
            let next = period_start.offset_by_timeframe(letter_period, 1);
            if next <= period_start {
                break;
            }
            period_start = next;
        }
        Self {
            start,
            end,
            periods,
            session_high: None,
            session_low: None,
            initial_balance_high: None,
            initial_balance_low: None,
        }
    }

    fn add_trade(&mut self, trade: &Trade, settings: TpoProfileSettings) {
        if trade.time < self.start || trade.time >= self.end {
            return;
        }
        self.session_high = Some(
            self.session_high
                .map_or(trade.price, |high| high.max(trade.price)),
        );
        self.session_low = Some(
            self.session_low
                .map_or(trade.price, |low| low.min(trade.price)),
        );
        let initial_balance_end = self.start.offset_by_timeframe(Timeframe::H1, 1);
        if trade.time < initial_balance_end {
            self.initial_balance_high = Some(
                self.initial_balance_high
                    .map_or(trade.price, |high| high.max(trade.price)),
            );
            self.initial_balance_low = Some(
                self.initial_balance_low
                    .map_or(trade.price, |low| low.min(trade.price)),
            );
        }
        let period_index = usize::try_from(
            trade.time.as_u64().saturating_sub(self.start.as_u64())
                / settings.letter_period.to_milliseconds().max(1),
        )
        .unwrap_or(usize::MAX);
        let Some(period) = self.periods.get_mut(period_index) else {
            return;
        };
        period.high = Some(
            period
                .high
                .map_or(trade.price, |high| high.max(trade.price)),
        );
        period.low = Some(period.low.map_or(trade.price, |low| low.min(trade.price)));
        period
            .rows
            .insert(trade.price.floor_to_step(settings.price_step));
    }

    fn build(
        &self,
        settings: TpoProfileSettings,
        now: UnixMs,
        status: TpoProfileStatus,
    ) -> Result<TpoProfile, TpoCalculationError> {
        let Some(session_low_price) = self.session_low else {
            return Ok(empty_profile_with_status(self.start, self.end, status));
        };
        let Some(session_high_price) = self.session_high else {
            return Ok(empty_profile_with_status(self.start, self.end, status));
        };
        let (session_low, row_count, settings) = bounded_profile_settings(
            session_low_price,
            session_high_price,
            settings,
            self.start,
            self.end,
        )?;

        let periods = self
            .periods
            .iter()
            .map(|period| (period.start, period.end))
            .collect::<Vec<_>>();
        let mut row_periods = vec![Vec::<u16>::new(); row_count];
        for (period_index, cached) in self.periods.iter().enumerate() {
            let period_index = u16::try_from(period_index).unwrap_or(u16::MAX);
            for price in &cached.rows {
                let price = price.floor_to_step(settings.price_step);
                let Some(row_index) =
                    Price::steps_between_inclusive(session_low, price, settings.price_step)
                        .and_then(|count| count.checked_sub(1))
                else {
                    continue;
                };
                let row_index = row_index.min(row_count.saturating_sub(1));
                if row_periods[row_index].last().copied() != Some(period_index) {
                    row_periods[row_index].push(period_index);
                }
            }
        }

        finalize_profile(
            self.start,
            self.end,
            periods,
            row_periods,
            session_high_price,
            session_low_price,
            self.initial_balance_high,
            self.initial_balance_low,
            status,
            settings,
            now,
        )
    }
}

pub fn build_tpo_profiles_with_source_timeframe(
    bars: &[Kline],
    settings: TpoProfileSettings,
    edits: TpoBoundaryEdits,
    source_timeframe: Timeframe,
    now: UnixMs,
) -> Result<Vec<TpoProfile>, TpoCalculationError> {
    validate_settings(settings)?;
    let Some(first_bar) = bars.iter().map(|bar| bar.time).min() else {
        return Ok(Vec::new());
    };
    let Some(last_bar) = bars.iter().map(|bar| bar.time).max() else {
        return Ok(Vec::new());
    };

    let first = settings.anchor.floor(first_bar);
    let last = settings.anchor.offset(settings.anchor.floor(last_bar), 1);
    let boundaries = edits.boundaries_between(first, last, settings.anchor);

    boundaries
        .windows(2)
        .map(|window| {
            build_tpo_profile_with_source_timeframe(
                bars,
                window[0],
                window[1],
                settings,
                source_timeframe,
                now,
            )
        })
        .collect()
}

pub fn build_tpo_profile_with_source_timeframe(
    bars: &[Kline],
    start: UnixMs,
    end: UnixMs,
    settings: TpoProfileSettings,
    source_timeframe: Timeframe,
    now: UnixMs,
) -> Result<TpoProfile, TpoCalculationError> {
    validate_settings(settings)?;
    let profile_bars: Vec<Kline> = bars
        .iter()
        .copied()
        .filter(|bar| bar.time >= start && bar.time < end)
        .collect();

    let session_high = profile_bars.iter().map(|bar| bar.high).max();
    let session_low = profile_bars.iter().map(|bar| bar.low).min();
    let initial_balance_end = start.offset_by_timeframe(Timeframe::H1, 1);
    let initial_balance_high = profile_bars
        .iter()
        .filter(|bar| bar.time < initial_balance_end)
        .map(|bar| bar.high)
        .max();
    let initial_balance_low = profile_bars
        .iter()
        .filter(|bar| bar.time < initial_balance_end)
        .map(|bar| bar.low)
        .min();

    let Some(session_low_price) = session_low else {
        return Ok(empty_profile(start, end, source_timeframe, now));
    };
    let Some(session_high_price) = session_high else {
        return Ok(empty_profile(start, end, source_timeframe, now));
    };
    let (session_low, row_count, settings) =
        bounded_profile_settings(session_low_price, session_high_price, settings, start, end)?;

    let mut periods = Vec::new();
    let mut period_start = start;
    while period_start < end {
        let period_end = period_start
            .offset_by_timeframe(settings.letter_period, 1)
            .min(end);
        periods.push((period_start, period_end));
        period_start = period_start.offset_by_timeframe(settings.letter_period, 1);
        if period_start <= periods.last().expect("period was pushed").0 {
            break;
        }
    }
    let mut row_periods = vec![Vec::<u16>::new(); row_count];
    for (period_index, (period_start, period_end)) in periods.iter().enumerate() {
        let mut touched = BTreeSet::new();
        for bar in profile_bars
            .iter()
            .filter(|bar| bar.time >= *period_start && bar.time < *period_end)
        {
            let low = bar.low.floor_to_step(settings.price_step);
            let high = bar.high.floor_to_step(settings.price_step);
            let Some(low_index) =
                Price::steps_between_inclusive(session_low, low, settings.price_step)
                    .and_then(|count| count.checked_sub(1))
            else {
                continue;
            };
            let Some(high_index) =
                Price::steps_between_inclusive(session_low, high, settings.price_step)
                    .and_then(|count| count.checked_sub(1))
            else {
                continue;
            };
            let high_index = high_index.min(row_count.saturating_sub(1));
            for row_index in low_index.min(high_index)..=high_index {
                touched.insert(row_index);
            }
        }
        for row_index in touched {
            row_periods[row_index].push(u16::try_from(period_index).unwrap_or(u16::MAX));
        }
    }

    finalize_profile(
        start,
        end,
        periods,
        row_periods,
        session_high_price,
        session_low_price,
        initial_balance_high,
        initial_balance_low,
        profile_coverage(start, end, &profile_bars, source_timeframe, now),
        settings,
        now,
    )
}

fn finalize_profile(
    start: UnixMs,
    end: UnixMs,
    periods: Vec<(UnixMs, UnixMs)>,
    row_periods: Vec<Vec<u16>>,
    session_high: Price,
    session_low: Price,
    initial_balance_high: Option<Price>,
    initial_balance_low: Option<Price>,
    status: TpoProfileStatus,
    settings: TpoProfileSettings,
    now: UnixMs,
) -> Result<TpoProfile, TpoCalculationError> {
    let rows = row_periods
        .into_iter()
        .enumerate()
        .map(|(index, periods)| TpoRow {
            price: session_low
                .floor_to_step(settings.price_step)
                .checked_add_steps(index as i64, settings.price_step)
                .expect("validated TPO row range"),
            periods,
        })
        .collect::<Vec<_>>();
    let (poc_index, vah_index, val_index) = value_area_indexes(
        &rows,
        settings
            .value_area_percent
            .clamp(MIN_TPO_VALUE_AREA_PERCENT, MAX_TPO_VALUE_AREA_PERCENT),
    );
    let single_prints = rows
        .iter()
        .filter(|row| row.periods.len() == 1)
        .filter(|row| {
            let Some(period_index) = row.periods.first().copied().map(usize::from) else {
                return false;
            };
            status != TpoProfileStatus::Incomplete
                && periods
                    .get(period_index)
                    .is_some_and(|(_, period_end)| *period_end <= now)
        })
        .map(|row| row.price)
        .collect();
    let poor_highs = repeated_extremes(&rows, true);
    let poor_lows = repeated_extremes(&rows, false);
    let poc = poc_index.and_then(|index| rows.get(index).map(|row| row.price));
    let vah = vah_index.and_then(|index| rows.get(index).map(|row| row.price));
    let val = val_index.and_then(|index| rows.get(index).map(|row| row.price));

    Ok(TpoProfile {
        start,
        end,
        period_count: periods.len(),
        rows,
        session_high: Some(session_high),
        session_low: Some(session_low),
        initial_balance_high,
        initial_balance_low,
        poc,
        vah,
        val,
        single_prints,
        poor_highs,
        poor_lows,
        status,
    })
}

fn bounded_profile_settings(
    session_low: Price,
    session_high: Price,
    mut settings: TpoProfileSettings,
    start: UnixMs,
    end: UnixMs,
) -> Result<(Price, usize, TpoProfileSettings), TpoCalculationError> {
    let Some(price_step) =
        settings
            .price_step
            .coarsened_to_fit(session_low, session_high, MAX_PROFILE_PRICE_ROWS)
    else {
        return Err(TpoCalculationError::TooManyRows {
            start,
            end,
            rows: usize::MAX,
        });
    };
    settings.price_step = price_step;
    let session_low = session_low.floor_to_step(price_step);
    let session_high = session_high.floor_to_step(price_step);
    let row_count =
        Price::steps_between_inclusive(session_low, session_high, price_step).unwrap_or(usize::MAX);
    if row_count == 0 || row_count > MAX_PROFILE_PRICE_ROWS {
        return Err(TpoCalculationError::TooManyRows {
            start,
            end,
            rows: row_count,
        });
    }
    Ok((session_low, row_count, settings))
}

fn validate_settings(settings: TpoProfileSettings) -> Result<(), TpoCalculationError> {
    if !matches!(
        settings.letter_period,
        Timeframe::M15 | Timeframe::M30 | Timeframe::H1 | Timeframe::H2 | Timeframe::H4
    ) {
        return Err(TpoCalculationError::InvalidLetterPeriod);
    }
    if settings.price_step.units <= 0 {
        return Err(TpoCalculationError::InvalidPriceStep);
    }
    Ok(())
}

fn empty_profile(
    start: UnixMs,
    end: UnixMs,
    source_timeframe: Timeframe,
    now: UnixMs,
) -> TpoProfile {
    empty_profile_with_status(
        start,
        end,
        profile_coverage(start, end, &[], source_timeframe, now),
    )
}

fn empty_profile_with_status(start: UnixMs, end: UnixMs, status: TpoProfileStatus) -> TpoProfile {
    TpoProfile {
        start,
        end,
        period_count: 0,
        rows: Vec::new(),
        session_high: None,
        session_low: None,
        initial_balance_high: None,
        initial_balance_low: None,
        poc: None,
        vah: None,
        val: None,
        single_prints: Vec::new(),
        poor_highs: Vec::new(),
        poor_lows: Vec::new(),
        status,
    }
}

fn profile_coverage(
    start: UnixMs,
    end: UnixMs,
    bars: &[Kline],
    source_timeframe: Timeframe,
    now: UnixMs,
) -> TpoProfileStatus {
    let expected_end = end.min(now.floor_to(source_timeframe)).max(start);
    let mut expected = start;
    let mut times = BTreeSet::new();
    times.extend(bars.iter().map(|bar| bar.time));
    while expected < expected_end {
        if !times.contains(&expected) {
            return TpoProfileStatus::Incomplete;
        }
        let next = expected.offset_by_timeframe(source_timeframe, 1);
        if next <= expected {
            break;
        }
        expected = next;
    }
    if end <= now {
        TpoProfileStatus::Complete
    } else {
        TpoProfileStatus::Developing
    }
}

fn repeated_extremes(rows: &[TpoRow], highs: bool) -> Vec<Price> {
    let extreme = if highs {
        rows.iter().rev().find(|row| !row.periods.is_empty())
    } else {
        rows.iter().find(|row| !row.periods.is_empty())
    };
    extreme
        .filter(|row| row.periods.len() >= 2)
        .map(|row| vec![row.price])
        .unwrap_or_default()
}

fn value_area_indexes(
    rows: &[TpoRow],
    value_area_percent: u8,
) -> (Option<usize>, Option<usize>, Option<usize>) {
    let Some(mut poc) = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.count() > 0)
        .max_by(|(left_index, left), (right_index, right)| {
            left.count()
                .cmp(&right.count())
                .then_with(|| right_index.cmp(left_index))
        })
        .map(|(index, _)| index)
    else {
        return (None, None, None);
    };

    let midpoint = (rows.len().saturating_sub(1)) as i64;
    let midpoint_distance = |index: usize| ((index as i64 * 2) - midpoint).unsigned_abs();
    for (index, row) in rows.iter().enumerate() {
        if row.count() > rows[poc].count()
            || (row.count() == rows[poc].count()
                && (midpoint_distance(index) < midpoint_distance(poc)
                    || (midpoint_distance(index) == midpoint_distance(poc) && index < poc)))
        {
            poc = index;
        }
    }

    let target = (rows.iter().map(TpoRow::count).sum::<usize>() * usize::from(value_area_percent))
        .div_ceil(100);
    let mut low = poc;
    let mut high = poc;
    let mut covered = rows[poc].count();
    while covered < target {
        let lower = (low > 0).then(|| rows[low - 1].count());
        let upper = (high + 1 < rows.len()).then(|| rows[high + 1].count());
        match (lower, upper) {
            (None, None) => break,
            (Some(lower), None) => {
                low -= 1;
                covered += lower;
            }
            (None, Some(upper)) => {
                high += 1;
                covered += upper;
            }
            (Some(lower), Some(upper)) if lower == upper => {
                let lower_distance = poc - (low - 1);
                let upper_distance = (high + 1) - poc;
                if upper_distance <= lower_distance {
                    high += 1;
                    covered += upper;
                } else {
                    low -= 1;
                    covered += lower;
                }
            }
            (Some(lower), Some(upper)) if lower > upper => {
                low -= 1;
                covered += lower;
            }
            (Some(_), Some(upper)) => {
                high += 1;
                covered += upper;
            }
        }
    }
    (Some(poc), Some(high), Some(low))
}
