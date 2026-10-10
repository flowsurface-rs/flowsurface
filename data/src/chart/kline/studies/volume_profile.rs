use super::{MAX_PROFILE_PRICE_ROWS, StudyDataSource};
use crate::chart::Basis;
use crate::chart::kline::poc::{NPoc, PocLineTracker, PocProfile, PointOfControl};
use crate::config::theme::{ThemeColor, ThemeColorRole, ThemeColorShade};
use exchange::{
    Kline, Timeframe, UnixMs,
    unit::price::{Price, PriceStep},
    unit::qty::Qty,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};

pub const DEFAULT_RECURRING_VOLUME_PROFILE_INTERVAL: Timeframe = Timeframe::M15;
pub const MIN_RECURRING_VOLUME_PROFILE_BASIS_MULTIPLIER: u64 = 4;
pub const DEFAULT_VOLUME_PROFILE_WIDTH_PERCENT: u8 = 20;
pub const DEFAULT_VISIBLE_VOLUME_PROFILE_OPACITY_PERCENT: u8 = 20;
pub const DEFAULT_RECURRING_VOLUME_PROFILE_OPACITY_PERCENT: u8 = 55;
pub const MIN_VOLUME_PROFILE_OPACITY_PERCENT: u8 = 5;
pub const MAX_VOLUME_PROFILE_OPACITY_PERCENT: u8 = 100;
pub const MIN_VOLUME_PROFILE_WIDTH_PERCENT: u8 = 5;
pub const MAX_VOLUME_PROFILE_WIDTH_PERCENT: u8 = 100;

pub(super) fn default_volume_profile_width_percent() -> u8 {
    DEFAULT_VOLUME_PROFILE_WIDTH_PERCENT
}

const fn default_show_volume_profile_poc() -> bool {
    false
}

const fn default_show_volume_profile_npoc() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeProfileColorSlot {
    Buy,
    Sell,
    NakedPoc,
    FilledPoc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeProfileColorSpec {
    pub label: &'static str,
    pub slot: VolumeProfileColorSlot,
}

const VOLUME_PROFILE_COLOR_SPECS: [VolumeProfileColorSpec; 4] = [
    VolumeProfileColorSpec {
        label: "Buy volume",
        slot: VolumeProfileColorSlot::Buy,
    },
    VolumeProfileColorSpec {
        label: "Sell volume",
        slot: VolumeProfileColorSlot::Sell,
    },
    VolumeProfileColorSpec {
        label: "NPOC",
        slot: VolumeProfileColorSlot::NakedPoc,
    },
    VolumeProfileColorSpec {
        label: "POC",
        slot: VolumeProfileColorSlot::FilledPoc,
    },
];

const KLINE_VOLUME_PROFILE_COLOR_SPECS: [VolumeProfileColorSpec; 4] = [
    VolumeProfileColorSpec {
        label: "Up volume",
        slot: VolumeProfileColorSlot::Buy,
    },
    VolumeProfileColorSpec {
        label: "Down volume",
        slot: VolumeProfileColorSlot::Sell,
    },
    VolumeProfileColorSpec {
        label: "NPOC",
        slot: VolumeProfileColorSlot::NakedPoc,
    },
    VolumeProfileColorSpec {
        label: "POC",
        slot: VolumeProfileColorSlot::FilledPoc,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeProfileColorConfig {
    pub buy: ThemeColor,
    pub sell: ThemeColor,
    pub naked_poc: ThemeColor,
    pub filled_poc: ThemeColor,
    #[serde(default = "default_show_volume_profile_poc")]
    pub show_poc: bool,
    #[serde(default = "default_show_volume_profile_npoc")]
    pub show_npoc: bool,
}

impl Default for VolumeProfileColorConfig {
    fn default() -> Self {
        Self::defaults()
    }
}

impl VolumeProfileColorConfig {
    pub const fn defaults() -> Self {
        Self {
            buy: ThemeColor::new(ThemeColorRole::Secondary, ThemeColorShade::Strong),
            sell: ThemeColor::new(ThemeColorRole::Secondary, ThemeColorShade::Soft),
            naked_poc: ThemeColor::new(ThemeColorRole::Warning, ThemeColorShade::Strong),
            filled_poc: ThemeColor::new(ThemeColorRole::Neutral, ThemeColorShade::Strong),
            show_poc: false,
            show_npoc: true,
        }
    }

    pub const fn color_specs() -> &'static [VolumeProfileColorSpec] {
        &VOLUME_PROFILE_COLOR_SPECS
    }

    pub fn color_specs_for_source(
        source: StudyDataSource,
        schedule: VolumeProfileSchedule,
    ) -> &'static [VolumeProfileColorSpec] {
        let specs = match source {
            StudyDataSource::Kline => &KLINE_VOLUME_PROFILE_COLOR_SPECS,
            StudyDataSource::Trades => &VOLUME_PROFILE_COLOR_SPECS,
        };
        match schedule {
            VolumeProfileSchedule::VisibleRange => &specs[..2],
            VolumeProfileSchedule::Recurring { .. } => specs,
        }
    }

    pub const fn color_for(self, slot: VolumeProfileColorSlot) -> ThemeColor {
        match slot {
            VolumeProfileColorSlot::Buy => self.buy,
            VolumeProfileColorSlot::Sell => self.sell,
            VolumeProfileColorSlot::NakedPoc => self.naked_poc,
            VolumeProfileColorSlot::FilledPoc => self.filled_poc,
        }
    }

    pub const fn with_color(self, slot: VolumeProfileColorSlot, color: ThemeColor) -> Self {
        match slot {
            VolumeProfileColorSlot::Buy => Self { buy: color, ..self },
            VolumeProfileColorSlot::Sell => Self {
                sell: color,
                ..self
            },
            VolumeProfileColorSlot::NakedPoc => Self {
                naked_poc: color,
                ..self
            },
            VolumeProfileColorSlot::FilledPoc => Self {
                filled_poc: color,
                ..self
            },
        }
    }

    pub const fn with_show_poc(self, show_poc: bool) -> Self {
        Self { show_poc, ..self }
    }

    pub const fn with_show_npoc(self, show_npoc: bool) -> Self {
        Self { show_npoc, ..self }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VolumeProfileSchedule {
    VisibleRange,
    Recurring { interval: Timeframe },
}

impl VolumeProfileSchedule {
    pub(super) const fn default_opacity_percent(self) -> u8 {
        match self {
            Self::VisibleRange => DEFAULT_VISIBLE_VOLUME_PROFILE_OPACITY_PERCENT,
            Self::Recurring { .. } => DEFAULT_RECURRING_VOLUME_PROFILE_OPACITY_PERCENT,
        }
    }

    pub fn recurring_intervals_for(basis: Basis) -> Vec<Timeframe> {
        let Some(minimum_interval) = Self::minimum_recurring_interval(basis) else {
            return Vec::new();
        };

        Timeframe::KLINE
            .iter()
            .copied()
            .filter(|interval| interval.to_milliseconds() >= minimum_interval.to_milliseconds())
            .collect()
    }

    pub fn minimum_recurring_interval(basis: Basis) -> Option<Timeframe> {
        let Basis::Time(basis_interval) = basis else {
            return None;
        };

        let minimum_interval_ms = basis_interval
            .to_milliseconds()
            .saturating_mul(MIN_RECURRING_VOLUME_PROFILE_BASIS_MULTIPLIER);

        Timeframe::KLINE
            .iter()
            .copied()
            .find(|interval| interval.to_milliseconds() >= minimum_interval_ms)
    }

    pub fn is_available(self, basis: Basis) -> bool {
        match (self, basis) {
            (Self::VisibleRange, Basis::Time(_)) => true,
            (Self::VisibleRange, Basis::Tick(_)) => false,
            (Self::Recurring { .. }, Basis::Time(_)) => {
                Self::minimum_recurring_interval(basis).is_some()
            }
            (Self::Recurring { .. }, Basis::Tick(_)) => false,
        }
    }

    pub fn is_interval_available(self, basis: Basis) -> bool {
        match (self, basis) {
            (Self::VisibleRange, _) => self.is_available(basis),
            (Self::Recurring { interval }, Basis::Time(_)) => {
                Self::minimum_recurring_interval(basis).is_some_and(|minimum_interval| {
                    interval.to_milliseconds() >= minimum_interval.to_milliseconds()
                })
            }
            (Self::Recurring { .. }, Basis::Tick(_)) => false,
        }
    }
}

impl Display for VolumeProfileSchedule {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::VisibleRange => formatter.write_str("Visible Range"),
            Self::Recurring { interval } => write!(formatter, "Recurring {interval}"),
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct ProfileLevel {
    pub price: Price,
    pub positive_qty: Qty,
    pub negative_qty: Qty,
}

impl ProfileLevel {
    pub fn total_qty(self) -> Qty {
        self.positive_qty + self.negative_qty
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProfileDistribution {
    pub levels: Vec<ProfileLevel>,
    pub max_qty: Qty,
    pub poc: Option<PointOfControl>,
    pub price_step: PriceStep,
}

pub(crate) struct ProfileAccumulator {
    levels: BTreeMap<Price, (Qty, Qty)>,
    max_qty: Qty,
    poc: Option<PointOfControl>,
    price_step: PriceStep,
}

impl ProfileAccumulator {
    pub(crate) fn new(price_step: PriceStep) -> Self {
        Self {
            levels: BTreeMap::new(),
            max_qty: Qty::ZERO,
            poc: None,
            price_step,
        }
    }

    pub(crate) fn add_volume(&mut self, price: Price, positive_qty: Qty, negative_qty: Qty) {
        let level = self.levels.entry(price).or_insert((Qty::ZERO, Qty::ZERO));
        level.0 += positive_qty;
        level.1 += negative_qty;

        let total_qty = level.0 + level.1;
        self.max_qty = self.max_qty.max(total_qty);
        if self.poc.is_none_or(|current| {
            total_qty > current.volume || (total_qty == current.volume && price < current.price)
        }) {
            self.poc = Some(PointOfControl {
                price,
                volume: total_qty,
                status: NPoc::None,
            });
        } else if self.poc.is_some_and(|current| current.price == price)
            && let Some(poc) = self.poc.as_mut()
        {
            poc.volume = total_qty;
        }
    }

    pub(crate) fn distribution(&self) -> ProfileDistribution {
        ProfileDistribution {
            levels: self
                .levels
                .iter()
                .map(|(&price, &(positive_qty, negative_qty))| ProfileLevel {
                    price,
                    positive_qty,
                    negative_qty,
                })
                .collect(),
            max_qty: self.max_qty,
            poc: self.poc,
            price_step: self.price_step,
        }
    }

    pub(crate) fn finish(self) -> Option<ProfileDistribution> {
        if self.levels.is_empty() {
            return None;
        }

        let levels = self
            .levels
            .into_iter()
            .map(|(price, (positive_qty, negative_qty))| ProfileLevel {
                price,
                positive_qty,
                negative_qty,
            })
            .collect();
        Some(ProfileDistribution {
            levels,
            max_qty: self.max_qty,
            poc: self.poc,
            price_step: self.price_step,
        })
    }

    fn add_kline(&mut self, bar: &Kline) {
        let tick_size = self.price_step;
        let (up_total, down_total) = bar.volume.buy_sell().unwrap_or_else(|| {
            let total = bar.volume.total();
            if bar.close >= bar.open {
                (total, Qty::ZERO)
            } else {
                (Qty::ZERO, total)
            }
        });
        if up_total.is_zero() && down_total.is_zero() {
            return;
        }

        let low = bar.low.floor_to_step(tick_size);
        let high = bar.high.floor_to_step(tick_size);
        let Some(row_count) = Price::steps_between_inclusive(low, high, tick_size) else {
            return;
        };
        if row_count == 0 {
            return;
        }
        let row_count = i64::try_from(row_count).unwrap_or(i64::MAX);
        let up_row_volume = up_total.units / row_count;
        let up_remainder = up_total.units % row_count;
        let down_row_volume = down_total.units / row_count;
        let down_remainder = down_total.units % row_count;

        for index in 0..row_count {
            let Some(price) = low.checked_add_steps(index, tick_size) else {
                break;
            };
            self.add_volume(
                price,
                Qty::from_units(up_row_volume + if index < up_remainder { 1 } else { 0 }),
                Qty::from_units(down_row_volume + if index < down_remainder { 1 } else { 0 }),
            );
        }
    }
}

impl PocProfile for ProfileAccumulator {
    fn price_range(&self) -> Option<(Price, Price)> {
        self.levels
            .first_key_value()
            .zip(self.levels.last_key_value())
            .map(|((low, _), (high, _))| (*low, *high))
    }

    fn poc(&self) -> Option<PointOfControl> {
        self.poc
    }

    fn set_poc_status(&mut self, status: NPoc) {
        if let Some(poc) = self.poc.as_mut() {
            poc.status = status;
        }
    }
}

impl PocProfile for ProfileDistribution {
    fn price_range(&self) -> Option<(Price, Price)> {
        self.levels
            .first()
            .zip(self.levels.last())
            .map(|(low, high)| (low.price, high.price))
    }

    fn poc(&self) -> Option<PointOfControl> {
        self.poc
    }

    fn set_poc_status(&mut self, status: NPoc) {
        if let Some(poc) = self.poc.as_mut() {
            poc.status = status;
        }
    }
}

#[derive(Debug, Clone)]
pub struct AnchoredProfileDistribution {
    pub start: UnixMs,
    pub end: UnixMs,
    pub distribution: ProfileDistribution,
}

pub fn kline_volume_profile(
    bars: &[Kline],
    earliest: UnixMs,
    latest: UnixMs,
    tick_size: PriceStep,
) -> Option<ProfileDistribution> {
    if tick_size.units <= 0 || earliest >= latest {
        return None;
    }

    let mut range: Option<(Price, Price)> = None;
    for bar in bars
        .iter()
        .filter(|bar| bar.time >= earliest && bar.time < latest)
    {
        range = Some(match range {
            Some((low, high)) => (low.min(bar.low), high.max(bar.high)),
            None => (bar.low, bar.high),
        });
    }
    let (low, high) = range?;
    let price_step = tick_size.coarsened_to_fit(low, high, MAX_PROFILE_PRICE_ROWS)?;
    let mut accumulator = ProfileAccumulator::new(price_step);
    for &bar in bars
        .iter()
        .filter(|bar| bar.time >= earliest && bar.time < latest)
    {
        accumulator.add_kline(&bar);
    }
    accumulator.finish()
}

pub fn recurring_kline_volume_profiles(
    bars: &[Kline],
    interval: Timeframe,
    earliest: UnixMs,
    latest: UnixMs,
    tick_size: PriceStep,
) -> Vec<AnchoredProfileDistribution> {
    if tick_size.units <= 0 || earliest >= latest {
        return Vec::new();
    }

    let mut index = RecurringKlineProfileIndex::new(interval, tick_size);
    index.update(bars);
    index.visible_profiles(earliest, latest)
}

#[derive(Default)]
struct KlineProfileSession {
    bars: BTreeMap<UnixMs, Kline>,
    distribution: Option<ProfileDistribution>,
}

impl PocProfile for KlineProfileSession {
    fn price_range(&self) -> Option<(Price, Price)> {
        self.distribution.as_ref()?.price_range()
    }

    fn poc(&self) -> Option<PointOfControl> {
        self.distribution.as_ref()?.poc
    }

    fn set_poc_status(&mut self, status: NPoc) {
        if let Some(distribution) = &mut self.distribution {
            distribution.set_poc_status(status);
        }
    }
}

/// Retained candle sessions and their calculated volume distributions and POC lines.
pub struct RecurringKlineProfileIndex {
    interval: Timeframe,
    tick_size: PriceStep,
    profiles: BTreeMap<UnixMs, KlineProfileSession>,
    poc_lines: PocLineTracker,
}

impl RecurringKlineProfileIndex {
    pub fn new(interval: Timeframe, tick_size: PriceStep) -> Self {
        Self {
            interval,
            tick_size,
            profiles: BTreeMap::new(),
            poc_lines: PocLineTracker::default(),
        }
    }

    /// Replaces a complete retained candle snapshot outside the live update path.
    pub fn update(&mut self, bars: &[Kline]) -> bool {
        let bars = bars
            .iter()
            .map(|bar| (bar.time, *bar))
            .collect::<BTreeMap<_, _>>();
        if self
            .profiles
            .values()
            .map(|profile| profile.bars.len())
            .sum::<usize>()
            == bars.len()
            && bars.values().all(|bar| {
                self.profiles
                    .get(&bar.time.floor_to(self.interval))
                    .and_then(|profile| profile.bars.get(&bar.time))
                    == Some(bar)
            })
        {
            return false;
        }
        self.profiles.clear();
        self.poc_lines = PocLineTracker::default();
        self.upsert_bars(&bars.into_values().collect::<Vec<_>>());
        true
    }

    pub fn upsert_bars(&mut self, bars: &[Kline]) -> bool {
        if bars.is_empty() {
            return false;
        }
        let previous_latest = self.latest_profile_range();
        let mut changed = BTreeSet::new();
        for &bar in bars {
            let start = bar.time.floor_to(self.interval);
            let profile = self.profiles.entry(start).or_default();
            if profile.bars.get(&bar.time) == Some(&bar) {
                continue;
            }
            profile.bars.insert(bar.time, bar);
            changed.insert(start);
        }
        self.recalculate_profiles(changed, previous_latest, false)
    }

    pub fn retain_range(&mut self, start: UnixMs, end: UnixMs) -> bool {
        let previous_latest = self.latest_profile_range();
        let mut changed = BTreeSet::new();
        let previous_count = self.profiles.len();
        self.profiles.retain(|profile_start, profile| {
            let previous_len = profile.bars.len();
            profile.bars.retain(|time, _| *time >= start && *time < end);
            if profile.bars.is_empty() {
                return false;
            }
            if profile.bars.len() != previous_len {
                changed.insert(*profile_start);
            }
            true
        });
        self.recalculate_profiles(
            changed,
            previous_latest,
            self.profiles.len() != previous_count,
        )
    }

    fn latest_profile_range(&self) -> Option<(UnixMs, (Price, Price))> {
        self.profiles
            .iter()
            .rev()
            .find_map(|(&start, profile)| profile.price_range().map(|range| (start, range)))
    }

    fn recalculate_profiles(
        &mut self,
        changed: BTreeSet<UnixMs>,
        previous_latest: Option<(UnixMs, (Price, Price))>,
        removed: bool,
    ) -> bool {
        for &start in &changed {
            let profile = self
                .profiles
                .get_mut(&start)
                .expect("changed profile exists");
            let bars = profile.bars.values().copied().collect::<Vec<_>>();
            profile.distribution = kline_volume_profile(
                &bars,
                start,
                start.offset_by_timeframe(self.interval, 1),
                self.tick_size,
            );
        }
        if changed.is_empty() && !removed {
            return false;
        }

        let append_only = !removed
            && previous_latest.is_none_or(|(latest, (low, high))| {
                changed.iter().all(|start| *start >= latest)
                    && self
                        .profiles
                        .get(&latest)
                        .and_then(PocProfile::price_range)
                        .is_some_and(|(next_low, next_high)| next_low <= low && next_high >= high)
            });
        if append_only {
            for start in changed {
                self.poc_lines.process_profile(&mut self.profiles, start);
            }
        } else {
            self.poc_lines.rebuild(&mut self.profiles);
        }
        true
    }

    pub fn visible_profiles(
        &self,
        earliest: UnixMs,
        latest: UnixMs,
    ) -> Vec<AnchoredProfileDistribution> {
        if earliest >= latest {
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
                    .and_then(|profile| profile.distribution.clone())
                    .map(|distribution| AnchoredProfileDistribution {
                        start,
                        end: start.offset_by_timeframe(self.interval, 1),
                        distribution,
                    })
            })
            .collect()
    }
}
