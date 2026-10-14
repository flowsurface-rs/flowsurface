use exchange::unit::price::Price;
use exchange::unit::qty::Qty;
use exchange::{Timeframe, UnixMs};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy)]
pub struct PointOfControl {
    pub price: Price,
    pub volume: Qty,
    pub status: NPoc,
}

impl Default for PointOfControl {
    fn default() -> Self {
        Self {
            price: Price::from_f32(0.0),
            volume: Qty::ZERO,
            status: NPoc::default(),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum NPoc {
    #[default]
    None,
    Naked,
    Filled {
        at: PocFillAt,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PocFillAt {
    Time(UnixMs),
    TickBar(usize),
}

const NAKED_POC_LINE_END: UnixMs = UnixMs::new(u64::MAX);

pub(crate) trait PocProfile {
    fn price_range(&self) -> Option<(Price, Price)>;
    fn poc(&self) -> Option<PointOfControl>;
    fn set_poc_status(&mut self, status: NPoc);
}

#[derive(Default)]
pub(crate) struct NakedPocIndex<K> {
    by_price: BTreeMap<Price, BTreeSet<K>>,
}

impl<K: Ord> NakedPocIndex<K> {
    pub(crate) fn clear(&mut self) {
        self.by_price.clear();
    }

    pub(crate) fn insert(&mut self, price: Price, key: K) {
        self.by_price.entry(price).or_default().insert(key);
    }

    pub(crate) fn remove(&mut self, price: Price, key: &K) {
        if let Some(keys) = self.by_price.get_mut(&price) {
            keys.remove(key);
            if keys.is_empty() {
                self.by_price.remove(&price);
            }
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.by_price.is_empty()
    }

    pub(crate) fn extend(&mut self, other: Self) {
        for (price, keys) in other.by_price {
            self.by_price.entry(price).or_default().extend(keys);
        }
    }

    pub(crate) fn take_touched(&mut self, low: Price, high: Price) -> Vec<K> {
        if low > high {
            return Vec::new();
        }
        let prices = self
            .by_price
            .range(low..=high)
            .map(|(&price, _)| price)
            .collect::<Vec<_>>();
        prices
            .into_iter()
            .filter_map(|price| self.by_price.remove(&price))
            .flatten()
            .collect()
    }
}

#[derive(Default)]
pub(crate) struct PocLineTracker {
    naked_pocs: NakedPocIndex<UnixMs>,
    poc_lines_by_end: BTreeMap<UnixMs, BTreeSet<UnixMs>>,
}

impl PocLineTracker {
    pub(crate) fn rebuild<P: PocProfile>(&mut self, profiles: &mut BTreeMap<UnixMs, P>) {
        self.naked_pocs.clear();
        self.poc_lines_by_end.clear();
        let starts = profiles.keys().copied().collect::<Vec<_>>();
        for profile in profiles.values_mut() {
            profile.set_poc_status(NPoc::None);
        }
        for start in starts {
            self.process_profile(profiles, start);
        }
    }

    pub(crate) fn process_profile<P: PocProfile>(
        &mut self,
        profiles: &mut BTreeMap<UnixMs, P>,
        start: UnixMs,
    ) {
        let Some((low, high)) = profiles.get(&start).and_then(PocProfile::price_range) else {
            return;
        };

        self.activate_previous_profile(profiles, start, low, high);

        for touched_start in self.naked_pocs.take_touched(low, high) {
            self.mark_poc_filled(profiles, touched_start, start);
        }

        if !profiles
            .range((std::ops::Bound::Excluded(start), std::ops::Bound::Unbounded))
            .any(|(_, profile)| profile.poc().is_some())
        {
            return;
        }

        let Some(poc_price) = profiles
            .get(&start)
            .and_then(|profile| profile.poc().map(|poc| poc.price))
        else {
            return;
        };
        self.mark_poc_naked(profiles, start, poc_price);
    }

    fn activate_previous_profile<P: PocProfile>(
        &mut self,
        profiles: &mut BTreeMap<UnixMs, P>,
        start: UnixMs,
        low: Price,
        high: Price,
    ) {
        let Some((previous_start, previous_poc)) = profiles
            .range(..start)
            .rev()
            .find_map(|(&start, profile)| profile.poc().map(|poc| (start, poc)))
        else {
            return;
        };
        if previous_poc.status != NPoc::None {
            return;
        }

        if low <= previous_poc.price && high >= previous_poc.price {
            self.mark_poc_filled(profiles, previous_start, start);
        } else {
            self.mark_poc_naked(profiles, previous_start, previous_poc.price);
        }
    }

    fn mark_poc_naked<P: PocProfile>(
        &mut self,
        profiles: &mut BTreeMap<UnixMs, P>,
        start: UnixMs,
        poc_price: Price,
    ) {
        if let Some(profile) = profiles.get_mut(&start) {
            profile.set_poc_status(NPoc::Naked);
        }
        self.naked_pocs.insert(poc_price, start);
        self.poc_lines_by_end
            .entry(NAKED_POC_LINE_END)
            .or_default()
            .insert(start);
    }

    fn mark_poc_filled<P: PocProfile>(
        &mut self,
        profiles: &mut BTreeMap<UnixMs, P>,
        start: UnixMs,
        at: UnixMs,
    ) {
        self.remove_poc_line(start, NAKED_POC_LINE_END);
        if let Some(profile) = profiles.get_mut(&start) {
            profile.set_poc_status(NPoc::Filled {
                at: PocFillAt::Time(at),
            });
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

    pub(crate) fn visible_starts(
        &self,
        interval: Timeframe,
        earliest: UnixMs,
        latest: UnixMs,
        profiles: &BTreeMap<UnixMs, impl PocProfile>,
    ) -> BTreeSet<UnixMs> {
        if earliest > latest {
            return BTreeSet::new();
        }

        let body_start = earliest.saturating_sub(interval.to_milliseconds());
        let mut visible_starts = profiles
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
    }
}
