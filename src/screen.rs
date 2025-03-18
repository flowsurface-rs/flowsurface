use std::fmt;

use chrono::DateTime;
use iced::{
    Element, Theme,
    widget::{button, tooltip::Position as TooltipPosition},
};
use serde::{Deserialize, Serialize};

use crate::widget::tooltip;

pub mod dashboard;
pub mod modal;

pub fn create_button<'a, M: Clone + 'a>(
    content: impl Into<Element<'a, M>>,
    message: M,
    tooltip_text: Option<&'a str>,
    tooltip_pos: TooltipPosition,
    style_fn: impl Fn(&Theme, button::Status) -> button::Style + 'static,
) -> Element<'a, M> {
    let btn = button(content).style(style_fn).on_press(message);

    if let Some(text) = tooltip_text {
        tooltip(btn, Some(text), tooltip_pos)
    } else {
        btn.into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum UserTimezone {
    #[default]
    Utc,
    Local,
}

impl UserTimezone {
    /// Converts UTC timestamp to the appropriate timezone and formats it according to timeframe
    pub fn format_timestamp(&self, timestamp: i64, timeframe: u64) -> String {
        if let Some(datetime) = DateTime::from_timestamp(timestamp, 0) {
            match self {
                UserTimezone::Local => {
                    let time_with_zone = datetime.with_timezone(&chrono::Local);
                    Self::format_by_timeframe(time_with_zone, timeframe)
                }
                UserTimezone::Utc => {
                    let time_with_zone = datetime.with_timezone(&chrono::Utc);
                    Self::format_by_timeframe(time_with_zone, timeframe)
                }
            }
        } else {
            String::new()
        }
    }

    /// Formats a `DateTime` with appropriate format based on timeframe
    fn format_by_timeframe<Tz: chrono::TimeZone>(datetime: DateTime<Tz>, timeframe: u64) -> String
    where
        Tz::Offset: std::fmt::Display,
    {
        if timeframe < 10000 {
            datetime.format("%M:%S").to_string()
        } else if datetime.format("%H:%M").to_string() == "00:00" {
            datetime.format("%-d").to_string()
        } else {
            datetime.format("%H:%M").to_string()
        }
    }

    /// Formats a `DateTime` with detailed format for crosshair display
    pub fn format_crosshair_timestamp(&self, timestamp_millis: i64, timeframe: u64) -> String {
        if let Some(datetime) = DateTime::from_timestamp_millis(timestamp_millis) {
            if timeframe < 10000 {
                return datetime.format("%M:%S:%3f").to_string().replace('.', "");
            }

            match self {
                UserTimezone::Local => datetime
                    .with_timezone(&chrono::Local)
                    .format("%a %b %-d  %H:%M")
                    .to_string(),
                UserTimezone::Utc => datetime
                    .with_timezone(&chrono::Utc)
                    .format("%a %b %-d  %H:%M")
                    .to_string(),
            }
        } else {
            String::new()
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

#[derive(thiserror::Error, Debug, Clone)]
pub enum DashboardError {
    #[error("Fetch error: {0}")]
    Fetch(String),
    #[error("Pane set error: {0}")]
    PaneSet(String),
    #[error("Unknown error: {0}")]
    Unknown(String),
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub enum PresentMode {
    #[default]
    AutoVsync,
    AutoNoVsync,
}

impl std::fmt::Display for PresentMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PresentMode::AutoVsync => write!(f, "Auto Vsync(Default)"),
            PresentMode::AutoNoVsync => write!(f, "Auto No-Vsync"),
        }
    }
}

impl PresentMode {
    pub const ALL: [PresentMode; 2] = [PresentMode::AutoVsync, PresentMode::AutoNoVsync];

    pub fn get_env_name(&self) -> &'static str {
        match self {
            PresentMode::AutoVsync => "vsync",
            PresentMode::AutoNoVsync => "no_vsync",
        }
    }
}
