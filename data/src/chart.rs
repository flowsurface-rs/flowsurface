use serde::{Deserialize, Serialize};

pub mod heatmap;
pub mod indicators;
pub mod kline;
pub mod timeandsales;

pub use kline::KlineChartKind;

pub fn round_to_tick(value: f32, tick_size: f32) -> f32 {
    (value / tick_size).round() * tick_size
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChartLayout {
    pub crosshair: bool,
    pub splits: Vec<f32>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub enum VisualConfig {
    Heatmap(heatmap::Config),
    TimeAndSales(timeandsales::Config),
    Kline(kline::Config),
}

impl VisualConfig {
    pub fn heatmap(&self) -> Option<heatmap::Config> {
        match self {
            Self::Heatmap(cfg) => Some(*cfg),
            _ => None,
        }
    }

    pub fn time_and_sales(&self) -> Option<timeandsales::Config> {
        match self {
            Self::TimeAndSales(cfg) => Some(*cfg),
            _ => None,
        }
    }

    pub fn kline(&self) -> Option<kline::Config> {
        match self {
            Self::Kline(cfg) => Some(*cfg),
            _ => None,
        }
    }
}

/// Defines how chart data is aggregated and displayed along the x-axis.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Basis {
    /// Time-based aggregation where each datapoint represents a fixed time interval.
    ///
    /// The u64 value represents milliseconds. Common values include:
    /// - `60_000` (1 minute)
    /// - `300_000` (5 minutes)
    /// - `3_600_000` (1 hour)
    Time(u64),

    /// Trade-based aggregation where each datapoint represents a fixed number of trades.
    ///
    /// The u64 value represents the number of trades per aggregation unit.
    /// Common values include 100, 500, or 1000 trades per bar/candle.
    Tick(u64),
}

impl Basis {
    pub fn is_time(&self) -> bool {
        matches!(self, Basis::Time(_))
    }
}

impl std::fmt::Display for Basis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Basis::Time(millis) => match *millis {
                60_000 => write!(f, "1m"),
                180_000 => write!(f, "3m"),
                300_000 => write!(f, "5m"),
                900_000 => write!(f, "15m"),
                1_800_000 => write!(f, "30m"),
                3_600_000 => write!(f, "1h"),
                7_200_000 => write!(f, "2h"),
                14_400_000 => write!(f, "4h"),
                _ => write!(f, "{millis}ms"),
            },
            Basis::Tick(count) => write!(f, "{count}T"),
        }
    }
}
