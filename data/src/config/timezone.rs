use chrono::DateTime;
use serde::{Deserialize, Serialize};
use std::fmt;

// GitHub Issue: https://github.com/terrylica/flowsurface/issues/2
/// Compact bar timestamp: "2026 Feb 26 14:35:42.123" — year prefix, no weekday, always ms precision.
/// Used for range-bar open/close fields in the crosshair tooltip.
const FMT_BAR_TIME_MS: &str = "%Y %b %-d %H:%M:%S.%3f";

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
    /// `timeframe` controls whether output is second-level (`MM:SS`) or minute-level (`HH:MM`).
    /// At exact midnight for non-sub-10s intervals, this returns the day-of-month (`D`) to
    /// emphasize date boundaries on the chart.
    fn format_by_timeframe(
        datetime: &DateTime<chrono::FixedOffset>,
        timeframe: exchange::Timeframe,
    ) -> String {
        let interval = timeframe.to_milliseconds();

        if interval < 10_000 {
            datetime.format("%M:%S").to_string()
        } else if datetime.format("%H:%M").to_string() == "00:00" {
            datetime.format("%-d").to_string()
        } else {
            datetime.format("%H:%M").to_string()
        }
    }

    /// Formats a Unix timestamp (ms) for the range-bar crosshair tooltip timing row.
    ///
    /// Always includes milliseconds. Output: `"Jan 20 14:35:42.123"`.
    /// Used for the compact open/close timestamp fields in the OHLC tooltip header.
    // GitHub Issue: https://github.com/terrylica/flowsurface/issues/2
    pub fn format_bar_time_ms(&self, timestamp_ms: i64) -> Option<String> {
        DateTime::from_timestamp_millis(timestamp_ms).map(|datetime| {
            self.with_user_timezone(datetime, |time_with_zone| {
                time_with_zone.format(FMT_BAR_TIME_MS).to_string()
            })
        })
    }

    /// Formats a timestamp for range bar axis labels.
    /// `label_span_ms` is the time span between adjacent labels (not between bars).
    pub fn format_range_bar_label(&self, timestamp: i64, label_span_ms: u64) -> String {
        if let Some(datetime) = DateTime::from_timestamp(timestamp, 0) {
            match self {
                UserTimezone::Local => {
                    let dt = datetime.with_timezone(&chrono::Local);
                    Self::format_range_bar_dt(&dt, label_span_ms)
                }
                UserTimezone::Utc => {
                    let dt = datetime.with_timezone(&chrono::Utc);
                    Self::format_range_bar_dt(&dt, label_span_ms)
                }
            }
        } else {
            String::new()
        }
    }

    // GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97
    fn format_range_bar_dt<Tz: chrono::TimeZone>(
        datetime: &DateTime<Tz>,
        label_span_ms: u64,
    ) -> String
    where
        Tz::Offset: std::fmt::Display,
    {
        if label_span_ms >= 30 * 86_400_000 {
            // Labels > 30 days apart: show month + year
            datetime.format("%b %Y").to_string()
        } else if label_span_ms >= 86_400_000 {
            // Labels > 1 day apart: show month + day + short year
            datetime.format("%b %-d '%y").to_string()
        } else if label_span_ms >= 3_600_000 {
            // Labels > 1 hour apart: show day + short year + time
            datetime.format("%b %-d '%y %H:%M").to_string()
        } else if label_span_ms >= 300_000 {
            // Labels > 5 minutes apart: show HH:MM only
            datetime.format("%H:%M").to_string()
        } else if label_span_ms >= 1_000 {
            // Labels 1s–5min apart: show seconds
            datetime.format("%H:%M:%S").to_string()
        } else {
            // Labels < 1s apart: show milliseconds
            datetime.format("%H:%M:%S.%3f").to_string()
        }
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
