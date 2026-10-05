use data::chart::kline::FetchRange;
use exchange::{Kline, TickerInfo, Timeframe, UnixMs};
use std::collections::BTreeMap;

const PROFILE_KLINE_RETAINED_SPANS: u64 = 8;
const PROFILE_KLINE_MAX_BARS: u64 = 250_000;

struct MissingRangePlan {
    windows: Vec<(UnixMs, UnixMs)>,
    cutoff: UnixMs,
    ranges: Vec<(UnixMs, UnixMs)>,
}

#[derive(Default)]
pub(super) struct ProfileKlineSource {
    timeframe: Option<Timeframe>,
    bars: BTreeMap<UnixMs, Kline>,
    empty_ranges: Vec<(UnixMs, UnixMs)>,
    retained: Option<(UnixMs, UnixMs)>,
    revision: u64,
    missing_plan: Option<MissingRangePlan>,
}

impl ProfileKlineSource {
    pub(super) fn insert_bars(&mut self, timeframe: Timeframe, bars: &[Kline]) -> Vec<UnixMs> {
        if self.timeframe != Some(timeframe) {
            return Vec::new();
        }
        let Some(retained) = self.retained else {
            return Vec::new();
        };
        let mut changed = Vec::new();
        for bar in bars {
            if !contains(retained, bar.time) {
                continue;
            }
            if self
                .bars
                .get(&bar.time)
                .is_some_and(|current| current == bar)
            {
                continue;
            }
            if self.bars.insert(bar.time, *bar).is_none() {
                self.missing_plan = None;
            }
            changed.push(bar.time);
        }
        if !changed.is_empty() {
            self.revision = self.revision.wrapping_add(1);
        }
        changed
    }

    pub(super) fn insert_empty_range(
        &mut self,
        timeframe: Timeframe,
        from: UnixMs,
        to: UnixMs,
    ) -> bool {
        if self.timeframe != Some(timeframe) {
            return false;
        }
        let Some(retained) = self.retained else {
            return false;
        };
        let timeframe_ms = timeframe.to_milliseconds();
        let start = from.floor_to(timeframe).max(retained.0);
        let end = to
            .saturating_add(timeframe_ms.saturating_sub(1))
            .floor_to(timeframe)
            .min(retained.1);
        if start >= end
            || self
                .empty_ranges
                .iter()
                .any(|(range_start, range_end)| *range_start <= start && *range_end >= end)
        {
            return false;
        }

        let mut merged = Vec::with_capacity(self.empty_ranges.len() + 1);
        let (mut start, mut end) = (start, end);
        let mut inserted = false;
        for (range_start, range_end) in std::mem::take(&mut self.empty_ranges) {
            if range_end < start {
                merged.push((range_start, range_end));
            } else if end < range_start {
                if !inserted {
                    merged.push((start, end));
                    inserted = true;
                }
                merged.push((range_start, range_end));
            } else {
                start = start.min(range_start);
                end = end.max(range_end);
            }
        }
        if !inserted {
            merged.push((start, end));
        }
        self.empty_ranges = merged;
        self.missing_plan = None;
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub(super) fn bars_ref(&self, timeframe: Timeframe) -> Option<&BTreeMap<UnixMs, Kline>> {
        (self.timeframe == Some(timeframe)).then_some(&self.bars)
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(super) fn fetch_ranges(
        &mut self,
        windows: &[(UnixMs, UnixMs)],
        now: UnixMs,
        ticker_info: TickerInfo,
        timeframe: Timeframe,
    ) -> Vec<(FetchRange, Vec<TickerInfo>)> {
        if self.timeframe != Some(timeframe) {
            return Vec::new();
        }
        let Some(retained) = self.retained else {
            return Vec::new();
        };
        let cutoff = now.floor_to(timeframe);
        if !self
            .missing_plan
            .as_ref()
            .is_some_and(|plan| plan.windows == windows && plan.cutoff == cutoff)
        {
            let mut ranges = windows
                .iter()
                .copied()
                .filter_map(|(start, end)| {
                    let start = start.max(retained.0);
                    let end = end.min(retained.1);
                    (start < end).then_some((start, end))
                })
                .flat_map(|(start, end)| self.missing_ranges(start, end, cutoff, timeframe))
                .collect::<Vec<_>>();
            ranges.sort_unstable();
            ranges.dedup();
            self.missing_plan = Some(MissingRangePlan {
                windows: windows.to_vec(),
                cutoff,
                ranges,
            });
        }

        self.missing_plan
            .as_ref()
            .into_iter()
            .flat_map(|plan| &plan.ranges)
            .map(|&(from, to)| {
                (
                    FetchRange::ProfileKline {
                        timeframe,
                        from,
                        to,
                    },
                    vec![ticker_info],
                )
            })
            .collect()
    }

    pub(super) fn configure_retention(
        &mut self,
        windows: &[(UnixMs, UnixMs)],
        visible_span: u64,
        timeframe: Timeframe,
    ) -> bool {
        let timeframe_changed = self.timeframe != Some(timeframe);
        if timeframe_changed {
            self.timeframe = Some(timeframe);
            self.bars.clear();
            self.empty_ranges.clear();
            self.retained = None;
        }
        let required = windows.iter().fold(
            (UnixMs::new(u64::MAX), UnixMs::ZERO),
            |(start, end), (next_start, next_end)| (start.min(*next_start), end.max(*next_end)),
        );
        let span = visible_span.max(timeframe.to_milliseconds());
        let padding = span.saturating_mul(PROFILE_KLINE_RETAINED_SPANS);
        let timeframe_span = timeframe.to_milliseconds();
        let desired = (
            required.0.saturating_sub(padding).floor_to(timeframe),
            required
                .1
                .saturating_add(padding)
                .saturating_add(timeframe_span - 1)
                .floor_to(timeframe),
        );
        let max_span = timeframe
            .to_milliseconds()
            .saturating_mul(PROFILE_KLINE_MAX_BARS);
        let desired = cap_range(
            if range_span(required) > max_span {
                required
            } else {
                desired
            },
            max_span,
            timeframe,
        );
        let next = self
            .retained
            .filter(|current| {
                contains_range(*current, required)
                    && range_span(*current) <= range_span(desired).saturating_mul(2)
            })
            .unwrap_or(desired);
        let retained_changed = self.retained != Some(next);
        if !retained_changed && !timeframe_changed {
            return false;
        }
        self.missing_plan = None;
        self.retained = Some(next);
        self.bars.retain(|time, _| contains(next, *time));
        self.empty_ranges = self
            .empty_ranges
            .drain(..)
            .filter_map(|(start, end)| {
                let clipped = (start.max(next.0), end.min(next.1));
                (clipped.0 < clipped.1).then_some(clipped)
            })
            .collect();
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub(crate) fn is_active(&self, timeframe: Timeframe, time: UnixMs) -> bool {
        self.timeframe == Some(timeframe)
            && self
                .retained
                .is_some_and(|retained| contains(retained, time))
    }

    pub(crate) fn retained(&self, timeframe: Timeframe) -> Option<(UnixMs, UnixMs)> {
        (self.timeframe == Some(timeframe))
            .then_some(self.retained)
            .flatten()
    }

    pub(crate) fn clear(&mut self) -> bool {
        let changed = !self.bars.is_empty()
            || !self.empty_ranges.is_empty()
            || self.retained.is_some()
            || self.timeframe.is_some();
        if changed {
            self.bars.clear();
            self.empty_ranges.clear();
            self.retained = None;
            self.timeframe = None;
            self.missing_plan = None;
            self.revision = self.revision.wrapping_add(1);
        }
        changed
    }

    fn missing_ranges(
        &self,
        start: UnixMs,
        end: UnixMs,
        now: UnixMs,
        timeframe: Timeframe,
    ) -> Vec<(UnixMs, UnixMs)> {
        let start = start.floor_to(timeframe);
        let end = end
            .saturating_add(timeframe.to_milliseconds() - 1)
            .floor_to(timeframe)
            .min(now.floor_to(timeframe));
        if start >= end {
            return Vec::new();
        }

        let mut missing = Vec::new();
        let mut missing_start = None;
        let mut expected = start;
        while expected < end {
            let covered = self.bars.contains_key(&expected) || self.empty_range_contains(expected);
            if covered {
                if let Some(from) = missing_start.take() {
                    missing.push((from, expected));
                }
            } else if missing_start.is_none() {
                missing_start = Some(expected);
            }
            let next = expected.offset_by_timeframe(timeframe, 1);
            if next <= expected {
                break;
            }
            expected = next;
        }
        if let Some(from) = missing_start {
            missing.push((from, end));
        }
        missing
    }

    fn empty_range_contains(&self, time: UnixMs) -> bool {
        let index = self.empty_ranges.partition_point(|(_, end)| *end <= time);
        self.empty_ranges
            .get(index)
            .is_some_and(|(start, end)| contains((*start, *end), time))
    }
}

fn contains((start, end): (UnixMs, UnixMs), time: UnixMs) -> bool {
    time >= start && time < end
}

fn contains_range(
    (start, end): (UnixMs, UnixMs),
    (required_start, required_end): (UnixMs, UnixMs),
) -> bool {
    required_start >= start && required_end <= end
}

fn range_span((start, end): (UnixMs, UnixMs)) -> u64 {
    end.saturating_diff(start)
}

fn cap_range(range: (UnixMs, UnixMs), max_span: u64, timeframe: Timeframe) -> (UnixMs, UnixMs) {
    if range_span(range) <= max_span {
        return range;
    }

    let start = range.0.as_u64();
    let center = start.saturating_add(range_span(range) / 2);
    let capped_start = center
        .saturating_sub(max_span / 2)
        .saturating_add(timeframe.to_milliseconds() - 1)
        / timeframe.to_milliseconds()
        * timeframe.to_milliseconds();
    let capped_start = UnixMs::new(capped_start);
    (capped_start, capped_start.saturating_add(max_span))
}
