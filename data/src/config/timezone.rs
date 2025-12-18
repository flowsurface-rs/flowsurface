use std::fmt;

use chrono::{DateTime, TimeZone};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum UserTimezone {
    #[default]
    Utc,
    Local,
}

impl UserTimezone {
    /// Converts UTC timestamp to the appropriate timezone and formats it according to timeframe
    pub fn format_timestamp(&self, timestamp: i64, timeframe: exchange::Timeframe) -> String {
        if let Some(datetime) = DateTime::from_timestamp(timestamp, 0) {
            match self {
                UserTimezone::Local => {
                    let time_with_zone = datetime.with_timezone(&chrono::Local);
                    Self::format_by_timeframe(&time_with_zone, timeframe)
                }
                UserTimezone::Utc => {
                    let time_with_zone = datetime.with_timezone(&chrono::Utc);
                    Self::format_by_timeframe(&time_with_zone, timeframe)
                }
            }
        } else {
            String::new()
        }
    }

    /// Formats a `DateTime` with appropriate format based on timeframe
    fn format_by_timeframe<Tz: chrono::TimeZone>(
        datetime: &DateTime<Tz>,
        timeframe: exchange::Timeframe,
    ) -> String
    where
        Tz::Offset: std::fmt::Display,
    {
        let interval = timeframe.to_milliseconds();

        if interval < 10000 {
            datetime.format("%M:%S").to_string()
        } else if datetime.format("%H:%M").to_string() == "00:00" {
            datetime.format("%-d").to_string()
        } else {
            datetime.format("%H:%M").to_string()
        }
    }

    /// Format timestamp for chart crosshair with timeframe-aware precision
    /// Uses seconds format for sub-minute timeframes, date+time for longer ones
    pub fn format_crosshair_timestamp(&self, timestamp_ms: u64, interval_ms: u64) -> String {
        let format_str = if interval_ms < 60_000 {
            "%H:%M:%S"
        } else {
            "%a %b %-d %H:%M"
        };

        let ts_i64 = timestamp_ms as i64;
        let ms_part = (timestamp_ms % 1000) as u32;

        match self {
            UserTimezone::Utc => {
                if let Some(dt) = chrono::Utc.timestamp_millis_opt(ts_i64).single() {
                    let base = dt.format(format_str).to_string();
                    if interval_ms < 1000 {
                        format!("{}.{:03}", base, ms_part)
                    } else {
                        base
                    }
                } else {
                    timestamp_ms.to_string()
                }
            }
            UserTimezone::Local => {
                if let Some(dt) = chrono::Local.timestamp_millis_opt(ts_i64).single() {
                    let base = dt.format(format_str).to_string();
                    if interval_ms < 1000 {
                        format!("{}.{:03}", base, ms_part)
                    } else {
                        base
                    }
                } else {
                    timestamp_ms.to_string()
                }
            }
        }
    }

    /// Convert UTC timestamp to timezone-adjusted milliseconds for display alignment
    /// Note: This shifts the timestamp value itself, useful for aligning tick marks
    pub fn adjust_ms_for_display(&self, ts_ms: u64) -> u64 {
        match self {
            UserTimezone::Utc => ts_ms,
            UserTimezone::Local => {
                if let Some(dt) = chrono::Local.timestamp_millis_opt(ts_ms as i64).single() {
                    let off_ms = (dt.offset().local_minus_utc() as i64) * 1000;
                    if off_ms >= 0 {
                        ts_ms.saturating_add(off_ms as u64)
                    } else {
                        ts_ms.saturating_sub((-off_ms) as u64)
                    }
                } else {
                    ts_ms
                }
            }
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
