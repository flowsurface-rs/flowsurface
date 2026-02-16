use chrono::DateTime;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum UserTimezone {
    #[default]
    Utc,
    Local,
}

impl UserTimezone {
    /// Formats a Unix timestamp (milliseconds) for axis labels based on the selected timezone.
    ///
    /// The input is interpreted as a UTC instant and then converted to either local time
    /// or UTC depending on `self`. The output format is chosen from the `timeframe`:
    ///
    /// - sub-10s intervals: `MM:SS`
    /// - larger intervals at midnight: day of month (`D`)
    /// - otherwise: `HH:MM`
    ///
    /// Returns `Some(formatted_timestamp)` when `timestamp_ms` is valid, otherwise `None`.
    pub fn format_timestamp(
        &self,
        timestamp_ms: i64,
        timeframe: exchange::Timeframe,
    ) -> Option<String> {
        DateTime::from_timestamp_millis(timestamp_ms).map(|datetime| {
            self.with_user_timezone(datetime, |time_with_zone| {
                Self::format_by_timeframe(&time_with_zone, timeframe)
            })
        })
    }

    /// Formats a Unix timestamp (milliseconds) for crosshair tooltips in the selected timezone.
    ///
    /// The input is interpreted as a UTC instant and converted to the user's timezone.
    /// Formatting depends on the provided candle/tick `interval` in milliseconds:
    ///
    /// - sub-10s intervals: `MM:SS.mmm`
    /// - otherwise: `Weekday Mon D HH:MM`
    ///
    /// Returns `Some(formatted_timestamp)` when `timestamp_ms` is valid, otherwise `None`.
    pub fn format_crosshair_timestamp(&self, timestamp_ms: i64, interval: u64) -> Option<String> {
        DateTime::from_timestamp_millis(timestamp_ms).map(|datetime| {
            self.with_user_timezone(datetime, |time_with_zone| {
                if interval < 10000 {
                    time_with_zone.format("%M:%S.%3f").to_string()
                } else {
                    time_with_zone.format("%a %b %-d %H:%M").to_string()
                }
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

        if interval < 10000 {
            datetime.format("%M:%S").to_string()
        } else if datetime.format("%H:%M").to_string() == "00:00" {
            datetime.format("%-d").to_string()
        } else {
            datetime.format("%H:%M").to_string()
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
