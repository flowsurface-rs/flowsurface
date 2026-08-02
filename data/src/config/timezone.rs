use chrono::{DateTime, Datelike, Months, TimeZone};
use serde::{Deserialize, Serialize};
use std::fmt;

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
