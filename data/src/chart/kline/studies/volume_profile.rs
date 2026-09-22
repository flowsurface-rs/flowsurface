use std::collections::BTreeMap;

use super::super::poc_lines::{PocLineTracker, PocProfile};
use super::MAX_PROFILE_PRICE_ROWS;

use crate::chart::kline_data::{NPoc, PointOfControl};
use exchange::{
    Kline, Timeframe, UnixMs,
    unit::price::{Price, PriceStep},
    unit::qty::Qty,
};

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

    pub(crate) fn add_kline(&mut self, bar: &Kline) {
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

    let mut ranges = BTreeMap::<UnixMs, (Price, Price)>::new();
    for &bar in bars {
        let range = ranges
            .entry(bar.time.floor_to(interval))
            .or_insert((bar.low, bar.high));
        range.0 = range.0.min(bar.low);
        range.1 = range.1.max(bar.high);
    }

    let mut accumulators = ranges
        .into_iter()
        .filter_map(|(start, (low, high))| {
            tick_size
                .coarsened_to_fit(low, high, MAX_PROFILE_PRICE_ROWS)
                .map(|step| (start, ProfileAccumulator::new(step)))
        })
        .collect::<BTreeMap<_, _>>();

    for &bar in bars {
        if let Some(accumulator) = accumulators.get_mut(&bar.time.floor_to(interval)) {
            accumulator.add_kline(&bar);
        }
    }

    let profiles = accumulators
        .into_iter()
        .filter_map(|(start, accumulator)| {
            accumulator
                .finish()
                .map(|distribution| (start, distribution))
        })
        .collect();

    RecurringKlineProfileIndex::new(interval, profiles).visible_profiles(earliest, latest)
}

struct RecurringKlineProfileIndex {
    interval: Timeframe,
    profiles: BTreeMap<UnixMs, ProfileDistribution>,
    poc_lines: PocLineTracker,
}

impl RecurringKlineProfileIndex {
    fn new(interval: Timeframe, profiles: BTreeMap<UnixMs, ProfileDistribution>) -> Self {
        let mut index = Self {
            interval,
            profiles,
            poc_lines: PocLineTracker::default(),
        };
        index.rebuild_poc_statuses();
        index
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
                    .cloned()
                    .map(|distribution| AnchoredProfileDistribution {
                        start,
                        end: start.offset_by_timeframe(self.interval, 1),
                        distribution,
                    })
            })
            .collect()
    }
}
