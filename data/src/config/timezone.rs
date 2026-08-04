use chrono::{DateTime, Datelike, Months, TimeZone};
use serde::{Deserialize, Serialize};
use std::fmt;

const ORIGIN_YEAR: i32 = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum UserTimezone {
    #[default]
    Utc,
    Local,
}

/// Specifies the *purpose* of a timestamp label when requesting a formatted
/// string from a `UserTimezone` instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeLabelKind<'a> {
    /// Formatting suitable for axis ticks.  Will choose the appropriate
    /// `HH:MM`, `MM:SS`, or `D` style based on the timeframe.
    Axis { timeframe: exchange::Timeframe },
    /// Formatting for the crosshair tooltip.
    /// Sub-10-second intervals will show `HH:MM:SS.mmm`,
    /// while larger intervals will show `Day Mon D HH:MM`.
    Crosshair { show_millis: bool },
    /// Arbitrary formatting using the given `chrono` specifier string.
    Custom(&'a str),
}

impl UserTimezone {
    pub fn to_user_datetime(
        &self,
        datetime: DateTime<chrono::Utc>,
    ) -> DateTime<chrono::FixedOffset> {
        self.with_user_timezone(datetime, |time_with_zone| time_with_zone)
    }

    /// Formats a Unix timestamp (milliseconds) according to the kind.
    pub fn format_with_kind(&self, timestamp_ms: i64, kind: TimeLabelKind<'_>) -> Option<String> {
        DateTime::from_timestamp_millis(timestamp_ms).map(|datetime| {
            self.with_user_timezone(datetime, |time_with_zone| match kind {
                TimeLabelKind::Axis { timeframe } => {
                    Self::format_by_timeframe(&time_with_zone, timeframe)
                }
                TimeLabelKind::Crosshair { show_millis } => {
                    if show_millis {
                        time_with_zone.format("%H:%M:%S.%3f").to_string()
                    } else {
                        time_with_zone.format("%a %b %-d %H:%M").to_string()
                    }
                }
                TimeLabelKind::Custom(fmt) => time_with_zone.format(fmt).to_string(),
            })
        })
    }

    /// Converts a UTC `DateTime` into the user's configured timezone and normalizes it to
    /// `DateTime<FixedOffset>` so downstream formatting can use one concrete type.
    fn with_user_timezone<T>(
        &self,
        datetime: DateTime<chrono::Utc>,
        formatter: impl FnOnce(DateTime<chrono::FixedOffset>) -> T,
    ) -> T {
        let time_with_zone = match self {
            UserTimezone::Local => datetime.with_timezone(&chrono::Local).fixed_offset(),
            UserTimezone::Utc => datetime.fixed_offset(),
        };

        formatter(time_with_zone)
    }

    /// Formats an already timezone-adjusted timestamp for axis labels.
    ///
    /// `timeframe` controls whether output is second-level (`MM:SS`) or
    /// minute-level (`HH:MM`); calendar boundaries are rendered separately in
    /// `data::chart::ticks::x`.
    fn format_by_timeframe(
        datetime: &DateTime<chrono::FixedOffset>,
        timeframe: exchange::Timeframe,
    ) -> String {
        let interval = timeframe.to_milliseconds();

        if interval < 10_000 {
            datetime.format("%M:%S").to_string()
        } else {
            datetime.format("%H:%M").to_string()
        }
    }

    /// The calendar date of `datetime` in the user's timezone.
    fn local_date(&self, datetime: DateTime<chrono::Utc>) -> chrono::NaiveDate {
        match self {
            UserTimezone::Utc => datetime.date_naive(),
            UserTimezone::Local => datetime.with_timezone(&chrono::Local).date_naive(),
        }
    }

    /// The UTC timestamp of local midnight (00:00 in the user's timezone) on
    /// `date`. Uses the earliest candidate for ambiguous local times (DST
    /// fall-back); midnight is never in a spring-forward gap in practice.
    pub(crate) fn local_midnight_utc_ms(&self, date: chrono::NaiveDate) -> Option<u64> {
        let midnight = date.and_hms_opt(0, 0, 0)?;
        let utc = match self {
            UserTimezone::Utc => midnight.and_utc(),
            UserTimezone::Local => chrono::Local
                .from_local_datetime(&midnight)
                .earliest()?
                .with_timezone(&chrono::Utc),
        };
        utc.timestamp_millis().try_into().ok()
    }

    /// Start of the user's local day containing `timestamp_ms`, as UTC ms.
    pub(crate) fn start_of_local_day_utc_ms(&self, timestamp_ms: u64) -> Option<u64> {
        let datetime = DateTime::from_timestamp_millis(timestamp_ms as i64)?;
        self.local_midnight_utc_ms(self.local_date(datetime))
    }

    /// Start of the user's local month containing `timestamp_ms`, as UTC ms.
    pub(crate) fn start_of_local_month_utc_ms(&self, timestamp_ms: u64) -> Option<u64> {
        let datetime = DateTime::from_timestamp_millis(timestamp_ms as i64)?;
        self.local_midnight_utc_ms(self.local_date(datetime).with_day(1)?)
    }

    /// Start of the user's local year containing `timestamp_ms`, as UTC ms.
    pub(crate) fn start_of_local_year_utc_ms(&self, timestamp_ms: u64) -> Option<u64> {
        let datetime = DateTime::from_timestamp_millis(timestamp_ms as i64)?;
        self.local_midnight_utc_ms(self.local_date(datetime).with_month(1)?.with_day(1)?)
    }

    /// Start of the user's local day after `timestamp_ms`, as UTC ms.
    pub(crate) fn next_local_day_utc_ms(&self, timestamp_ms: u64) -> Option<u64> {
        let datetime = DateTime::from_timestamp_millis(timestamp_ms as i64)?;
        self.local_midnight_utc_ms(self.local_date(datetime).succ_opt()?)
    }

    /// Start of the user's local month after `timestamp_ms`, as UTC ms.
    pub(crate) fn next_local_month_utc_ms(&self, timestamp_ms: u64) -> Option<u64> {
        let datetime = DateTime::from_timestamp_millis(timestamp_ms as i64)?;
        self.local_midnight_utc_ms(
            self.local_date(datetime)
                .checked_add_months(Months::new(1))?,
        )
    }

    /// Start of the user's local year after `timestamp_ms`, as UTC ms.
    pub(crate) fn next_local_year_utc_ms(&self, timestamp_ms: u64) -> Option<u64> {
        let datetime = DateTime::from_timestamp_millis(timestamp_ms as i64)?;
        self.local_midnight_utc_ms(
            self.local_date(datetime)
                .checked_add_months(Months::new(12))?,
        )
    }

    /// Advance `timestamp_ms` to the first local midnight whose date lies on
    /// the `skip_days`-period grid anchored at a fixed origin date
    /// (2000-01-01). Used to phase-align thinned boundary series (e.g. daily
    /// marks) to *absolute* dates so the collected set stays fixed while the
    /// view pans; without this, the thinning re-anchors to the sliding visible
    /// edge and the whole day-label set shifts by one day whenever the edge
    /// crosses a midnight.
    pub(crate) fn align_to_skip_grid(&self, timestamp_ms: u64, skip_days: u64) -> Option<u64> {
        let origin = chrono::NaiveDate::from_ymd_opt(ORIGIN_YEAR, 1, 1)?;
        let datetime = DateTime::from_timestamp_millis(timestamp_ms as i64)?;
        let date = self.local_date(datetime);
        let days = date.signed_duration_since(origin).num_days();
        let advance = advance_to_grid(skip_days, days);
        self.local_midnight_utc_ms(date.checked_add_days(chrono::Days::new(advance))?)
    }

    /// Advance `timestamp_ms` to the first month-start that lies on the
    /// `step_months`-period grid anchored at 2000-01, in the user's timezone.
    pub(crate) fn align_to_month_grid(&self, timestamp_ms: u64, step_months: u64) -> Option<u64> {
        let datetime = DateTime::from_timestamp_millis(timestamp_ms as i64)?;
        let date = self.local_date(datetime);
        let months = (date.year() - ORIGIN_YEAR) as i64 * 12 + i64::from(date.month0());
        let first = date.with_day(1)?;
        self.local_midnight_utc_ms(
            first.checked_add_months(Months::new(advance_to_grid(step_months, months) as u32))?,
        )
    }

    /// Advance `timestamp_ms` to the first year-start that lies on the
    /// `step_years`-period grid anchored at 2000-01-01, in the user's timezone.
    pub(crate) fn align_to_year_grid(&self, timestamp_ms: u64, step_years: u64) -> Option<u64> {
        let datetime = DateTime::from_timestamp_millis(timestamp_ms as i64)?;
        let date = self.local_date(datetime);
        let years = (date.year() - ORIGIN_YEAR) as i64;
        let first = date.with_month(1)?.with_day(1)?;
        self.local_midnight_utc_ms(first.checked_add_months(Months::new(
            (advance_to_grid(step_years, years) * 12) as u32,
        ))?)
    }
}

/// Forward distance from `offset` to the next multiple of `step` (0 when
/// already aligned).
fn advance_to_grid(step: u64, offset: i64) -> u64 {
    let step = step.max(1) as i64;
    ((step - offset.rem_euclid(step)) % step) as u64
}

impl fmt::Display for UserTimezone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UserTimezone::Utc => write!(f, "UTC"),
            UserTimezone::Local => {
                let local_offset = chrono::Local::now().offset().local_minus_utc();
                let hours = local_offset / 3600;
                let minutes = (local_offset % 3600) / 60;
                write!(f, "Local (UTC {hours:+03}:{minutes:02})")
            }
        }
    }
}

impl<'de> Deserialize<'de> for UserTimezone {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let timezone_str = String::deserialize(deserializer)?;
        match timezone_str.to_lowercase().as_str() {
            "utc" => Ok(UserTimezone::Utc),
            "local" => Ok(UserTimezone::Local),
            _ => Err(serde::de::Error::custom("Invalid UserTimezone")),
        }
    }
}

impl Serialize for UserTimezone {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            UserTimezone::Utc => serializer.serialize_str("UTC"),
            UserTimezone::Local => serializer.serialize_str("Local"),
        }
    }
}
