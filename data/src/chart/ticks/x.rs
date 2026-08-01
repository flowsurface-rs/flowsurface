use super::MAX_GRID_LINES;
use crate::util::{reset_to_start_of_month_utc, reset_to_start_of_year_utc};
use chrono::{DateTime, Months};
use exchange::{Timeframe, UnixMs};

pub const ONE_DAY_MS: u64 = 24 * 60 * 60 * 1000;

const MS_TIME_STEPS: [u64; 10] = [
    1000 * 120,
    1000 * 60,
    1000 * 30,
    1000 * 10,
    1000 * 5,
    1000 * 2,
    1000,
    500,
    200,
    100,
];

const M1_TIME_STEPS: [u64; 9] = [
    1000 * 60 * 720, // 12 hour
    1000 * 60 * 180, // 3 hour
    1000 * 60 * 60,  // 1 hour
    1000 * 60 * 30,  // 30 min
    1000 * 60 * 15,  // 15 min
    1000 * 60 * 10,  // 10 min
    1000 * 60 * 5,   // 5 min
    1000 * 60 * 2,   // 2 min
    1000 * 60,       // 1 min
];

const M3_TIME_STEPS: [u64; 9] = [
    1000 * 60 * 1440, // 24 hour
    1000 * 60 * 720,  // 12 hour
    1000 * 60 * 360,  // 6 hour
    1000 * 60 * 120,  // 2 hour
    1000 * 60 * 60,   // 1 hour
    1000 * 60 * 30,   // 30 min
    1000 * 60 * 15,   // 15 min
    1000 * 60 * 9,    // 9 min
    1000 * 60 * 3,    // 3 min
];

const M5_TIME_STEPS: [u64; 9] = [
    1000 * 60 * 1440, // 24 hour
    1000 * 60 * 720,  // 12 hour
    1000 * 60 * 480,  // 8 hour
    1000 * 60 * 240,  // 4 hour
    1000 * 60 * 120,  // 2 hour
    1000 * 60 * 60,   // 1 hour
    1000 * 60 * 30,   // 30 min
    1000 * 60 * 15,   // 15 min
    1000 * 60 * 5,    // 5 min
];

const HOURLY_TIME_STEPS: [u64; 8] = [
    1000 * 60 * 5760, // 96 hour
    1000 * 60 * 2880, // 48 hour
    1000 * 60 * 1440, // 24 hour
    1000 * 60 * 720,  // 12 hour
    1000 * 60 * 480,  // 8 hour
    1000 * 60 * 240,  // 4 hour
    1000 * 60 * 120,  // 2 hour
    1000 * 60 * 60,   // 1 hour
];

/// A typed table of candidate time steps used to pick the X-axis grid step
/// for a visible span. Owns both the candidate list and the selection rules,
/// so callers never touch raw step tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSteps(&'static [u64]);

impl TimeSteps {
    /// Wrap a step table.
    pub const fn new(steps: &'static [u64]) -> Self {
        Self(steps)
    }

    /// The candidate steps in table order.
    pub fn as_slice(&self) -> &'static [u64] {
        self.0
    }

    /// The step table appropriate for a timeframe (in minutes).
    pub fn from_timeframe(timeframe: Timeframe) -> Self {
        let timeframe_in_min = timeframe.to_milliseconds() / 60_000;

        match timeframe_in_min {
            0_u64..1_u64 => Self(&MS_TIME_STEPS),
            1..=30 => match timeframe_in_min {
                1 => Self(&M1_TIME_STEPS),
                3 => Self(&M3_TIME_STEPS),
                5 => Self(&M5_TIME_STEPS),
                15 => Self(&M5_TIME_STEPS[..7]),
                30 => Self(&M5_TIME_STEPS[..6]),
                _ => Self(&HOURLY_TIME_STEPS),
            },
            31.. => Self(&HOURLY_TIME_STEPS),
        }
    }

    /// Pick the largest step in the table with `step <= span / budget` (i.e.
    /// at least `budget` intervals fit), so the grid shows roughly `budget`
    /// labels. Falls back to the smallest step that still fits within the
    /// span, then to the first table entry, when the budget cannot be met.
    pub fn pick_leq(&self, span_ms: u64, budget: u32) -> u64 {
        let span = span_ms as u128;
        let budget = budget.max(1) as u128;

        let mut best_label: Option<u64> = None;
        let mut best_fit: Option<u64> = None;
        for &step in self.0 {
            if step == 0 {
                continue;
            }
            if (step as u128) * budget <= span {
                best_label = Some(best_label.map_or(step, |b| b.max(step)));
            } else if step <= span_ms {
                best_fit = Some(best_fit.map_or(step, |b| b.min(step)));
            }
        }
        best_label
            .or(best_fit)
            .unwrap_or(*self.0.first().unwrap_or(&1))
    }
}

impl TimeAxisGrid {
    /// All multiples of `step_ms` within `[earliest, latest]`, ascending.
    ///
    /// Guaranteed to terminate: values strictly increase and the count is
    /// capped at [`MAX_GRID_LINES`]. Returns an empty vec for a zero step or
    /// an empty/inverted range.
    pub fn flat_ticks(earliest: UnixMs, latest: UnixMs, step_ms: u64) -> Vec<u64> {
        if step_ms == 0 {
            return Vec::new();
        }

        let e = earliest.as_u64();
        let l = latest.as_u64();
        if l < e {
            return Vec::new();
        }

        let mut out = Vec::new();
        let mut t = e.div_ceil(step_ms).saturating_mul(step_ms);
        let mut guard = 0;
        while t <= l && guard < MAX_GRID_LINES {
            out.push(t);
            guard += 1;
            let Some(next) = t.checked_add(step_ms) else {
                break;
            };
            t = next;
        }
        out
    }

    /// Select the grid step (in milliseconds) for a visible range, plus the
    /// earliest timestamp aligned down to that step.
    ///
    /// `step_ms` is always one of the timeframe's table values and non-zero;
    /// `rounded_earliest` is `(earliest / step) * step`.
    pub fn calc_step(
        earliest: UnixMs,
        latest: UnixMs,
        labels_can_fit: i32,
        timeframe: Timeframe,
    ) -> (u64, u64) {
        let duration = latest
            .duration_since(earliest)
            .map_or(0, |d| d.as_millis() as u64);

        let step_ms =
            TimeSteps::from_timeframe(timeframe).pick_leq(duration, labels_can_fit.max(1) as u32);

        let rounded_earliest = (earliest.as_u64() / step_ms) * step_ms;

        (step_ms, rounded_earliest)
    }
}

/// The full set of X-axis label timestamps for a visible range, grouped by
/// layer. Exactly one of the two modes is populated:
///
/// * `step_ms < ONE_DAY_MS`: [`Self::sub_daily`] holds step multiples.
/// * `step_ms >= ONE_DAY_MS`: [`Self::daily`], [`Self::monthly`] and
///   [`Self::yearly`] hold start-of-day / start-of-month / start-of-year UTC
///   boundaries.
///
/// `step_ms == 0` signals "no grid fits" (callers render nothing).
#[derive(Debug, Clone, Default)]
pub struct TimeAxisGrid {
    /// Selected grid step in milliseconds (`0` when no grid fits).
    pub step_ms: u64,
    /// Visible range start (ms); used by [`Self::x_position`].
    pub earliest: UnixMs,
    /// Visible range end (ms); used by [`Self::x_position`].
    pub latest: UnixMs,
    /// Sub-daily step multiples within `[earliest, latest]` (ms).
    pub sub_daily: Vec<u64>,
    /// Start-of-day (00:00 UTC) boundaries within `[earliest, latest]` (ms).
    pub daily: Vec<u64>,
    /// Start-of-month UTC boundaries within `[earliest, latest]` (ms).
    pub monthly: Vec<u64>,
    /// Start-of-year UTC boundaries within `[earliest, latest]` (ms).
    pub yearly: Vec<u64>,
}

impl TimeAxisGrid {
    /// Compute the X-axis time grid for a visible range. Pure and deterministic.
    pub fn new(
        earliest: UnixMs,
        latest: UnixMs,
        labels_can_fit: i32,
        timeframe: Timeframe,
    ) -> Self {
        let (step_ms, _rounded_earliest) =
            Self::calc_step(earliest, latest, labels_can_fit, timeframe);

        if step_ms == 0 {
            return TimeAxisGrid {
                earliest,
                latest,
                ..Default::default()
            };
        }

        if step_ms >= ONE_DAY_MS {
            let (Some(start), Some(end)) = (earliest.as_datetime_utc(), latest.as_datetime_utc())
            else {
                return TimeAxisGrid {
                    earliest,
                    latest,
                    ..Default::default()
                };
            };

            TimeAxisGrid {
                step_ms,
                earliest,
                latest,
                sub_daily: Vec::new(),
                daily: Self::collect_daily(earliest, latest, start, end),
                monthly: Self::collect_monthly(earliest, latest, start, end),
                yearly: Self::collect_yearly(earliest, latest, start, end),
            }
        } else {
            TimeAxisGrid {
                step_ms,
                earliest,
                latest,
                sub_daily: Self::flat_ticks(earliest, latest, step_ms),
                daily: Vec::new(),
                monthly: Vec::new(),
                yearly: Vec::new(),
            }
        }
    }

    /// Map a timestamp to a horizontal position within this grid's visible
    /// range (`[earliest, latest]`) scaled to `width`. Returns `0.0` when the
    /// range is empty or the timestamp precedes `earliest` (callers drop
    /// non-drawable positions).
    pub fn x_position(&self, time_ms: u64, width: f32) -> f64 {
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
    /// Terminates even when `next` does not advance (e.g. chrono overflow at
    /// year 9999).
    fn collect_boundaries(
        mut current: u64,
        end_ms: u64,
        earliest: UnixMs,
        latest: UnixMs,
        next: impl Fn(u64) -> Option<u64>,
    ) -> Vec<u64> {
        let mut out = Vec::with_capacity(MAX_GRID_LINES.min(64));
        let mut guard = 0;
        while current <= end_ms && guard < MAX_GRID_LINES {
            if current >= earliest.as_u64() && current <= latest.as_u64() {
                out.push(current);
            }
            guard += 1;
            let Some(next_val) = next(current) else {
                break;
            };
            if next_val <= current {
                break;
            }
            current = next_val;
        }
        out
    }

    /// Start-of-day (00:00 UTC) boundaries within `[earliest, latest]`.
    fn collect_daily(
        earliest: UnixMs,
        latest: UnixMs,
        start: DateTime<chrono::Utc>,
        end: DateTime<chrono::Utc>,
    ) -> Vec<u64> {
        let start_ms = Self::start_of_day_ms(start.timestamp_millis() as u64);
        let end_ms = end.timestamp_millis() as u64;
        Self::collect_boundaries(start_ms, end_ms, earliest, latest, |ts| {
            ts.checked_add(ONE_DAY_MS)
        })
    }

    /// Start-of-month UTC boundaries within `[earliest, latest]`.
    fn collect_monthly(
        earliest: UnixMs,
        latest: UnixMs,
        start: DateTime<chrono::Utc>,
        end: DateTime<chrono::Utc>,
    ) -> Vec<u64> {
        let start_ms = reset_to_start_of_month_utc(start).timestamp_millis() as u64;
        let end_ms = end.timestamp_millis() as u64;
        Self::collect_boundaries(start_ms, end_ms, earliest, latest, |ts| {
            let dt = UnixMs::new(ts).as_datetime_utc()?;
            dt.checked_add_months(Months::new(1))
                .map(reset_to_start_of_month_utc)
                .map(|d| d.timestamp_millis() as u64)
        })
    }

    /// Start-of-year UTC boundaries within `[earliest, latest]`.
    fn collect_yearly(
        earliest: UnixMs,
        latest: UnixMs,
        start: DateTime<chrono::Utc>,
        end: DateTime<chrono::Utc>,
    ) -> Vec<u64> {
        let start_ms = reset_to_start_of_year_utc(start).timestamp_millis() as u64;
        let end_ms = end.timestamp_millis() as u64;
        Self::collect_boundaries(start_ms, end_ms, earliest, latest, |ts| {
            let dt = UnixMs::new(ts).as_datetime_utc()?;
            dt.checked_add_months(Months::new(12))
                .map(reset_to_start_of_year_utc)
                .map(|d| d.timestamp_millis() as u64)
        })
    }

    /// Floor a millisecond timestamp to its UTC day boundary.
    fn start_of_day_ms(ts: u64) -> u64 {
        ts - (ts % ONE_DAY_MS)
    }
}
