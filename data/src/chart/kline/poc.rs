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
        at: u64,
    },
}

impl NPoc {
    pub fn filled(&mut self, at: u64) {
        *self = NPoc::Filled { at };
    }

    pub fn unfilled(&mut self) {
        *self = NPoc::Naked;
    }
}

const NAKED_POC_LINE_END: UnixMs = UnixMs::new(u64::MAX);

pub(crate) trait PocProfile {
    fn price_range(&self) -> Option<(Price, Price)>;
    fn poc(&self) -> Option<PointOfControl>;
    fn set_poc_status(&mut self, status: NPoc);
}

#[derive(Default)]
pub(crate) struct PocLineTracker {
    naked_pocs: BTreeMap<Price, BTreeSet<UnixMs>>,
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

        let touched_prices = self
            .naked_pocs
            .range(low..=high)
            .map(|(&price, _)| price)
            .collect::<Vec<_>>();
        for price in touched_prices {
            if let Some(touched_starts) = self.naked_pocs.remove(&price) {
                for touched_start in touched_starts {
                    self.mark_poc_filled(profiles, touched_start, start);
                }
            }
        }

        if profiles.keys().next_back().copied() == Some(start) {
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
        let Some(previous_start) = profiles.range(..start).next_back().map(|(&start, _)| start)
        else {
            return;
        };
        let Some(previous_poc) = profiles.get(&previous_start).and_then(PocProfile::poc) else {
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
        self.naked_pocs.entry(poc_price).or_default().insert(start);
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
            profile.set_poc_status(NPoc::Filled { at: at.as_u64() });
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
