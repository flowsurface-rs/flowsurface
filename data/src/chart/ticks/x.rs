use super::MAX_GRID_LINES;
use crate::{UserTimezone, config::timezone::TimeLabelKind};
use exchange::{Timeframe, UnixMs};

use chrono::DateTime;

/// Nominal local day length in ms (DST days vary slightly around this).
const DAY_MS: u64 = 86_400_000;
/// Upper bound on a local day's length in ms (25h, covers DST fall-back).
const MAX_LOCAL_DAY_MS: u64 = 90_000_000;

const MS_TIME_STEPS: [u64; 12] = [
    1000 * 120,
    1000 * 60,
    1000 * 30,
    1000 * 20,
    1000 * 15,
    1000 * 10,
    1000 * 5,
    1000 * 2,
    1000,
    500,
    200,
    100,
];

const M1_TIME_STEPS: [u64; 13] = [
    1000 * 60 * 720, // 12 hour
    1000 * 60 * 480, // 8 hour
    1000 * 60 * 360, // 6 hour
    1000 * 60 * 240, // 4 hour
    1000 * 60 * 180, // 3 hour
    1000 * 60 * 120, // 2 hour
    1000 * 60 * 60,  // 1 hour
    1000 * 60 * 30,  // 30 min
    1000 * 60 * 15,  // 15 min
    1000 * 60 * 10,  // 10 min
    1000 * 60 * 5,   // 5 min
    1000 * 60 * 2,   // 2 min
    1000 * 60,       // 1 min
];

const M3_TIME_STEPS: [u64; 11] = [
    1000 * 60 * 1440, // 24 hour
    1000 * 60 * 720,  // 12 hour
    1000 * 60 * 360,  // 6 hour
    1000 * 60 * 180,  // 3 hour
    1000 * 60 * 120,  // 2 hour
    1000 * 60 * 60,   // 1 hour
    1000 * 60 * 30,   // 30 min
    1000 * 60 * 15,   // 15 min
    1000 * 60 * 9,    // 9 min
    1000 * 60 * 5,    // 5 min
    1000 * 60 * 3,    // 3 min
];

const M5_TIME_STEPS: [u64; 10] = [
    1000 * 60 * 1440, // 24 hour
    1000 * 60 * 720,  // 12 hour
    1000 * 60 * 480,  // 8 hour
    1000 * 60 * 360,  // 6 hour
    1000 * 60 * 240,  // 4 hour
    1000 * 60 * 120,  // 2 hour
    1000 * 60 * 60,   // 1 hour
    1000 * 60 * 30,   // 30 min
    1000 * 60 * 15,   // 15 min
    1000 * 60 * 5,    // 5 min
];

const HOURLY_TIME_STEPS: [u64; 9] = [
    1000 * 60 * 5760, // 96 hour
    1000 * 60 * 2880, // 48 hour
    1000 * 60 * 1440, // 24 hour
    1000 * 60 * 720,  // 12 hour
    1000 * 60 * 480,  // 8 hour
    1000 * 60 * 360,  // 6 hour
    1000 * 60 * 240,  // 4 hour
    1000 * 60 * 120,  // 2 hour
    1000 * 60 * 60,   // 1 hour
];

/// Intraday weights, coarsest first (e.g. a 2h step → `Hour1`, a 4h step →
/// `Hour3`).
const INTRADAY_DIVISORS: [(u64, TickWeight); 8] = [
    (12 * 60 * 60 * 1000, TickWeight::Hour12),
    (6 * 60 * 60 * 1000, TickWeight::Hour6),
    (3 * 60 * 60 * 1000, TickWeight::Hour3),
    (60 * 60 * 1000, TickWeight::Hour1),
    (30 * 60 * 1000, TickWeight::Minute30),
    (5 * 60 * 1000, TickWeight::Minute5),
    (60 * 1000, TickWeight::Minute1),
    (1_000, TickWeight::Second),
];

/// A typed table of candidate time steps used to pick the X-axis grid step
/// for a visible span. Owns both the candidate list and the selection rules,
/// so callers never touch raw step tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TimeSteps(&'static [u64]);

impl TimeSteps {
    /// Pick the step closest to `span / budget`, keeping label density roughly
    /// constant as the user pans or zooms. Falls back to the first table entry
    /// when the table is empty.
    fn pick_closest(&self, span_ms: u64, budget: u32) -> u64 {
        let target = span_ms as f64 / budget.max(1) as f64;

        let mut best: Option<(u64, f64)> = None;
        for &step in self.0 {
            if step == 0 {
                continue;
            }
            let dist = (step as f64 - target).abs();
            if best.is_none_or(|(_, d)| dist < d) {
                best = Some((step, dist));
            }
        }
        best.map(|(s, _)| s)
            .unwrap_or(*self.0.first().unwrap_or(&1))
    }
}

impl From<Timeframe> for TimeSteps {
    fn from(timeframe: Timeframe) -> Self {
        let timeframe_in_min = timeframe.to_milliseconds() / 60_000;

        match timeframe_in_min {
            0_u64..1_u64 => Self(&MS_TIME_STEPS),
            1..=30 => match timeframe_in_min {
                1 => Self(&M1_TIME_STEPS),
                3 => Self(&M3_TIME_STEPS),
                5 => Self(&M5_TIME_STEPS),
                15 => Self(&M5_TIME_STEPS[..8]),
                30 => Self(&M5_TIME_STEPS[..7]),
                _ => Self(&HOURLY_TIME_STEPS),
            },
            31.. => Self(&HOURLY_TIME_STEPS),
        }
    }
}

/// Calendar significance of a tick, coarsest first. Greater weights are placed
/// before finer ones, so calendar marks are never crowded out by time marks.
///
/// Ordering: `Year` > `Month` > `Day` > … > `Second`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TickWeight {
    Year,
    Month,
    Day,
    Hour12,
    Hour6,
    Hour3,
    Hour1,
    Minute30,
    Minute5,
    Minute1,
    Second,
}

impl TickWeight {
    /// How this tick's label should be rendered.
    fn label_kind(self, timeframe: Timeframe) -> TimeLabelKind<'static> {
        match self {
            TickWeight::Year => TimeLabelKind::Custom("%Y"),
            TickWeight::Month => TimeLabelKind::Custom("%b"),
            TickWeight::Day => TimeLabelKind::Custom("%d"),
            _ => TimeLabelKind::Axis { timeframe },
        }
    }

    /// Intraday weight of a regular `step_ms` grid: the coarsest divisor it covers.
    fn for_intraday_step(step_ms: u64) -> Self {
        INTRADAY_DIVISORS
            .iter()
            .find(|(divisor, _)| step_ms >= *divisor)
            .map(|&(_, weight)| weight)
            .unwrap_or(Self::Second)
    }

    /// Placement priority: coarser units come first.
    fn priority(self) -> u8 {
        match self {
            TickWeight::Year => 10,
            TickWeight::Month => 9,
            TickWeight::Day => 8,
            TickWeight::Hour12 => 7,
            TickWeight::Hour6 => 6,
            TickWeight::Hour3 => 5,
            TickWeight::Hour1 => 4,
            TickWeight::Minute30 => 3,
            TickWeight::Minute5 => 2,
            TickWeight::Minute1 => 1,
            TickWeight::Second => 0,
        }
    }
}

impl PartialOrd for TickWeight {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TickWeight {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority().cmp(&other.priority())
    }
}

/// A candidate time-axis tick: a timestamp plus its calendar weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TickMark {
    /// Unix timestamp in milliseconds.
    time_ms: u64,
    /// Calendar significance of the tick (coarsest first).
    weight: TickWeight,
}

impl TickMark {
    /// Format a tick by its calendar weight, in the user's timezone.
    fn format_tick(
        &self,
        timeframe: exchange::Timeframe,
        timezone: crate::UserTimezone,
    ) -> Option<String> {
        let kind = self.weight.label_kind(timeframe);
        timezone.format_with_kind(self.time_ms as i64, kind)
    }
}

/// Candidate ticks for a visible range, plus the selection math that turns
/// them into the drawn labels: coarser marks are placed first under a minimum
/// pixel spacing, so month/year boundaries are never crowded out by finer
/// labels.
#[derive(Debug, Clone, Default)]
struct TimeAxisGrid {
    /// Visible range start (ms); used by [`Self::x_position`].
    earliest: UnixMs,
    /// Visible range end (ms); used by [`Self::x_position`].
    latest: UnixMs,
    /// Candidate ticks within `[earliest, latest]`, deduplicated (a timestamp
    /// keeps its coarsest weight) and sorted by time.
    marks: Vec<TickMark>,
}

impl TimeAxisGrid {
    /// Compute the X-axis time grid for a visible range. Pure and deterministic.
    fn new(
        earliest: UnixMs,
        latest: UnixMs,
        labels_can_fit: i32,
        timeframe: Timeframe,
        timezone: UserTimezone,
        width: f32,
        min_spacing_px: f32,
    ) -> Self {
        let step_ms = Self::calc_step(earliest, latest, labels_can_fit, timeframe);

        if step_ms == 0 {
            return TimeAxisGrid {
                earliest,
                latest,
                ..Default::default()
            };
        }

        let (Some(start), Some(end)) = (earliest.as_datetime_utc(), latest.as_datetime_utc())
        else {
            // Extreme timestamps: fall back to step multiples only.
            return TimeAxisGrid {
                earliest,
                latest,
                marks: Self::step_marks(earliest, latest, step_ms, timezone, width, min_spacing_px),
            };
        };

        let mut marks =
            Self::step_marks(earliest, latest, step_ms, timezone, width, min_spacing_px);

        // Calendar boundaries guarantee coarse anchors even when the intraday
        // step is too coarse to land on every one of them.
        marks.extend(
            Self::collect_daily(
                earliest,
                latest,
                start,
                end,
                timezone,
                width,
                min_spacing_px,
            )
            .into_iter()
            .map(|t| TickMark {
                time_ms: t,
                weight: TickWeight::Day,
            }),
        );
        marks.extend(
            Self::collect_monthly(earliest, latest, start, end, timezone)
                .into_iter()
                .map(|t| TickMark {
                    time_ms: t,
                    weight: TickWeight::Month,
                }),
        );
        marks.extend(
            Self::collect_yearly(earliest, latest, start, end, timezone)
                .into_iter()
                .map(|t| TickMark {
                    time_ms: t,
                    weight: TickWeight::Year,
                }),
        );

        marks.sort_by_key(|m| m.time_ms);
        marks.dedup_by(|removed, kept| {
            if removed.time_ms == kept.time_ms {
                if removed.weight > kept.weight {
                    kept.weight = removed.weight;
                }
                true
            } else {
                false
            }
        });

        TimeAxisGrid {
            earliest,
            latest,
            marks,
        }
    }

    /// Intraday step ticks (every `step_ms` within each local day, starting at
    /// local midnight) with their nominal intraday weight; day/month/year
    /// boundaries are layered on top separately.
    fn step_marks(
        earliest: UnixMs,
        latest: UnixMs,
        step_ms: u64,
        timezone: UserTimezone,
        width: f32,
        min_spacing_px: f32,
    ) -> Vec<TickMark> {
        let weight = TickWeight::for_intraday_step(step_ms);
        Self::day_anchored_ticks(earliest, latest, step_ms, timezone, width, min_spacing_px)
            .into_iter()
            .map(|t| TickMark { time_ms: t, weight })
            .collect()
    }

    /// Select the marks to draw: place coarsest marks first, keeping each only
    /// when it is at least `min_spacing_px` from already-kept marks on both
    /// sides. Returns the kept marks sorted by time.
    fn select_marks(&self, width: f32, min_spacing_px: f32) -> Vec<TickMark> {
        if self.marks.is_empty() || width <= 0.0 || min_spacing_px <= 0.0 {
            return self.marks.clone();
        }

        let earliest = self.earliest.as_u64();
        let latest = self.latest.as_u64();
        if latest <= earliest {
            return Vec::new();
        }
        let span = latest - earliest;

        let x = |t: u64| -> f64 {
            (t.saturating_sub(earliest) as f64 / span as f64) * f64::from(width)
        };
        let min_spacing = f64::from(min_spacing_px);

        let mut kept: Vec<TickMark> = Vec::new(); // sorted by time

        for weight in [
            TickWeight::Year,
            TickWeight::Month,
            TickWeight::Day,
            TickWeight::Hour12,
            TickWeight::Hour6,
            TickWeight::Hour3,
            TickWeight::Hour1,
            TickWeight::Minute30,
            TickWeight::Minute5,
            TickWeight::Minute1,
            TickWeight::Second,
        ] {
            // `marks` is time-sorted, so filtering preserves order: no sort needed.
            for mark in self.marks.iter().filter(|m| m.weight == weight) {
                let idx = kept.partition_point(|k| k.time_ms < mark.time_ms);
                let left_ok = idx == 0 || x(mark.time_ms) - x(kept[idx - 1].time_ms) >= min_spacing;
                let right_ok =
                    idx >= kept.len() || x(kept[idx].time_ms) - x(mark.time_ms) >= min_spacing;
                if left_ok && right_ok {
                    kept.insert(idx, *mark);
                }
            }
        }

        kept
    }

    /// Map a timestamp to a horizontal position within this grid's visible
    /// range (`[earliest, latest]`) scaled to `width`. Returns `0.0` when the
    /// range is empty or the timestamp precedes `earliest` (callers drop
    /// non-drawable positions).
    fn x_position(&self, time_ms: u64, width: f32) -> f64 {
        let earliest = self.earliest.as_u64();
        let latest = self.latest.as_u64();
        if latest > earliest {
            let span = latest - earliest;
            (time_ms.saturating_sub(earliest) as f64 / span as f64) * f64::from(width)
        } else {
            0.0
        }
    }

    /// Advance `current` by `next` while it stays within `[0, end_ms]`,
    /// collecting every value that lands inside `[earliest, latest]`.
    /// `skip_ms` thins the series: boundaries closer than `skip_ms` to the
    /// last collected one are skipped, so dense series (e.g. daily on a
    /// multi-year range) stay bounded. Terminates even when `next` does not
    /// advance.
    fn collect_boundaries(
        mut current: u64,
        end_ms: u64,
        earliest: UnixMs,
        latest: UnixMs,
        skip_ms: u64,
        next: impl Fn(u64) -> Option<u64>,
    ) -> Vec<u64> {
        let mut out = Vec::with_capacity(MAX_GRID_LINES.min(64));
        let mut guard = 0;
        while current <= end_ms && guard < MAX_GRID_LINES {
            if current >= earliest.as_u64() && current <= latest.as_u64() {
                out.push(current);
            }
            guard += 1;

            let Some(mut next_val) = next(current) else {
                break;
            };
            if next_val <= current {
                break;
            }

            // Skip boundaries that would sit closer than `skip_ms` to the one
            // just collected (pixel-based thinning for long ranges;
            // `select_marks` re-checks the real spacing afterwards).
            let target = current.saturating_add(skip_ms);
            while next_val < target {
                let Some(after) = next(next_val) else {
                    break;
                };
                if after <= next_val {
                    break;
                }
                next_val = after;
            }
            current = next_val;
        }
        out
    }

    /// Pixel-based thinning for a boundary series whose nominal spacing is
    /// `nominal_ms` (e.g. one day): the smallest skip, in ms, such that
    /// consecutive collected boundaries are at least `min_spacing_px` apart.
    fn boundary_skip_ms(
        nominal_ms: u64,
        earliest: UnixMs,
        latest: UnixMs,
        width: f32,
        min_spacing_px: f32,
    ) -> u64 {
        if width <= 0.0 || min_spacing_px <= 0.0 {
            return 0;
        }
        let span = latest.as_u64().saturating_sub(earliest.as_u64()).max(1);
        let px_per_boundary = (nominal_ms as f64 / span as f64) * f64::from(width);
        let count = (f64::from(min_spacing_px) / px_per_boundary).ceil() as u64;
        nominal_ms.saturating_mul(count.max(1))
    }

    /// Start-of-local-day (00:00 in the user's timezone) boundaries within
    /// `[earliest, latest]`, thinned to the pixel budget so multi-year ranges
    /// stay bounded (`select_marks` re-checks the real spacing).
    fn collect_daily(
        earliest: UnixMs,
        latest: UnixMs,
        start: DateTime<chrono::Utc>,
        end: DateTime<chrono::Utc>,
        timezone: UserTimezone,
        width: f32,
        min_spacing_px: f32,
    ) -> Vec<u64> {
        let start_ms = timezone
            .start_of_local_day_utc_ms(start.timestamp_millis() as u64)
            .unwrap_or(0);
        let end_ms = end.timestamp_millis() as u64;
        let skip_ms = Self::boundary_skip_ms(DAY_MS, earliest, latest, width, min_spacing_px);
        Self::collect_boundaries(start_ms, end_ms, earliest, latest, skip_ms, |ts| {
            timezone.next_local_day_utc_ms(ts)
        })
    }

    /// Start-of-local-month boundaries within `[earliest, latest]`.
    fn collect_monthly(
        earliest: UnixMs,
        latest: UnixMs,
        start: DateTime<chrono::Utc>,
        end: DateTime<chrono::Utc>,
        timezone: UserTimezone,
    ) -> Vec<u64> {
        let start_ms = timezone
            .start_of_local_month_utc_ms(start.timestamp_millis() as u64)
            .unwrap_or(0);
        let end_ms = end.timestamp_millis() as u64;
        Self::collect_boundaries(start_ms, end_ms, earliest, latest, 0, |ts| {
            timezone.next_local_month_utc_ms(ts)
        })
    }

    /// Start-of-local-year boundaries within `[earliest, latest]`.
    fn collect_yearly(
        earliest: UnixMs,
        latest: UnixMs,
        start: DateTime<chrono::Utc>,
        end: DateTime<chrono::Utc>,
        timezone: UserTimezone,
    ) -> Vec<u64> {
        let start_ms = timezone
            .start_of_local_year_utc_ms(start.timestamp_millis() as u64)
            .unwrap_or(0);
        let end_ms = end.timestamp_millis() as u64;
        Self::collect_boundaries(start_ms, end_ms, earliest, latest, 0, |ts| {
            timezone.next_local_year_utc_ms(ts)
        })
    }

    /// Multiples of `step_ms` anchored to the user's local midnight, within
    /// `[earliest, latest]`, ascending.
    ///
    /// Each local day starts a fresh grid at its midnight, so midnight is
    /// always a tick and the grid is uniform in the user's timezone — matching
    /// the calendar marks layered on top, which never get crowded or leave
    /// lopsided gaps.
    ///
    /// Marks are pre-thinned to the pixel budget so a long range can never
    /// blow up the mark list or truncate the tail: generation starts at the
    /// first mark at or after `earliest` and walks at `stride = k * step`,
    /// where `k` is the smallest integer whose pixel width is at least
    /// `min_spacing_px`. When the stride reaches a full local day, the
    /// intraday grid degrades to plain stride multiples (the day-boundary
    /// layer already anchors every midnight), keeping the count around
    /// `width / min_spacing_px` regardless of the range. Guaranteed to
    /// terminate: values strictly increase and the walk is bounded. Returns an
    /// empty vec for a zero step or an empty/inverted range.
    fn day_anchored_ticks(
        earliest: UnixMs,
        latest: UnixMs,
        step_ms: u64,
        timezone: UserTimezone,
        width: f32,
        min_spacing_px: f32,
    ) -> Vec<u64> {
        if step_ms == 0 {
            return Vec::new();
        }

        let e = earliest.as_u64();
        let l = latest.as_u64();
        if l < e {
            return Vec::new();
        }
        let span = l - e;

        // Thinning stride: the smallest multiple of `step` whose pixel width
        // is at least `min_spacing_px`, so at most ~`width / min_spacing_px`
        // marks are materialized for selection to refine.
        let stride_ms = if width > 0.0 && min_spacing_px > 0.0 {
            let px_per_step = (step_ms as f64 / span as f64) * f64::from(width);
            let k = ((f64::from(min_spacing_px) / px_per_step).ceil() as u64).max(1);
            step_ms.saturating_mul(k).max(1)
        } else {
            step_ms
        };

        // Once the stride spans a full local day, day-anchoring would only
        // repeat the midnight anchor the day layer already provides, so fall
        // back to plain stride multiples: uniformly spaced and bounded by the
        // pixel budget.
        if stride_ms >= MAX_LOCAL_DAY_MS {
            let first = e.div_ceil(stride_ms).saturating_mul(stride_ms);
            let mut out = Vec::new();
            let mut t = first;
            while t <= l {
                out.push(t);
                let Some(next) = t.checked_add(stride_ms) else {
                    break;
                };
                t = next;
            }
            return out;
        }

        let mut out = Vec::new();
        let mut day_start = timezone.start_of_local_day_utc_ms(e).unwrap_or(e);
        while day_start <= l {
            let Some(next_day) = timezone.next_local_day_utc_ms(day_start) else {
                break;
            };

            // First mark of this day at or after `earliest`, aligned to the
            // day's midnight grid (a multiple of `step` from midnight).
            let first = day_start.max(e);
            let k = first.saturating_sub(day_start).div_ceil(stride_ms);
            let mut t = day_start.saturating_add(k.saturating_mul(stride_ms));

            while t < next_day && t <= l {
                out.push(t);
                let Some(next) = t.checked_add(stride_ms) else {
                    break;
                };
                t = next;
            }

            day_start = next_day;
        }
        out
    }

    /// Select the grid step (in milliseconds) for a visible range.
    ///
    /// `step_ms` is always one of the timeframe's table values and non-zero.
    fn calc_step(
        earliest: UnixMs,
        latest: UnixMs,
        labels_can_fit: i32,
        timeframe: Timeframe,
    ) -> u64 {
        let duration = latest
            .duration_since(earliest)
            .map_or(0, |d| d.as_millis() as u64);

        TimeSteps::from(timeframe).pick_closest(duration, labels_can_fit.max(1) as u32)
    }
}

/// Visual emphasis tier for a time-axis tick.
///
/// [`TimeTickTier::Secondary`] marks the coarse calendar boundaries that stand
/// out from the fine-grained [`TimeTickTier::Main`] labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeTickTier {
    /// The primary, fine-grained ticks (the majority of labels).
    Main,
    /// Coarser boundary ticks that should stand out from the majority.
    Secondary,
}

/// A fully positioned, formatted X-axis tick label, ready to be drawn.
///
/// UI-agnostic: carries the horizontal position, text and emphasis tier, so
/// callers can map it onto their own rendering primitives.
#[derive(Debug, Clone, PartialEq)]
pub struct TimeTickLabel {
    /// Horizontal center position in pixels within the axis width.
    pub x_pos: f32,
    /// Rendered label text.
    pub content: String,
    /// Visual emphasis tier.
    pub tier: TimeTickTier,
}

impl TimeTickLabel {
    /// Compute the X-axis labels to draw for a visible range: build the grid,
    /// select marks under `min_spacing_px`, then format and position each one
    /// within `width` pixels. Pure and deterministic.
    pub fn for_range(
        earliest: UnixMs,
        latest: UnixMs,
        width: f32,
        min_spacing_px: f32,
        labels_can_fit: i32,
        timeframe: Timeframe,
        timezone: crate::UserTimezone,
    ) -> Vec<TimeTickLabel> {
        let grid = TimeAxisGrid::new(
            earliest,
            latest,
            labels_can_fit,
            timeframe,
            timezone,
            width,
            min_spacing_px,
        );

        if grid.marks.is_empty() {
            return Vec::new();
        }

        let selected = grid.select_marks(width, min_spacing_px);
        let max_weight = selected.iter().map(|m| m.weight).max();
        let has_finer_tier = max_weight.is_some_and(|max| selected.iter().any(|m| m.weight < max));

        let mut labels = Vec::with_capacity(selected.len());
        for mark in selected {
            let Some(content) = mark.format_tick(timeframe, timezone) else {
                continue;
            };

            // The coarsest weight present stands out.
            let tier = if has_finer_tier && Some(mark.weight) == max_weight {
                TimeTickTier::Secondary
            } else {
                TimeTickTier::Main
            };

            labels.push(TimeTickLabel {
                x_pos: grid.x_position(mark.time_ms, width) as f32,
                content,
                tier,
            });
        }

        labels
    }
}
