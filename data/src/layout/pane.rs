use exchange::{TickMultiplier, TickerInfo, adapter::StreamType};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::chart::{
    Basis, ChartLayout, VisualConfig,
    indicators::{CandlestickIndicator, FootprintIndicator, HeatmapIndicator},
};

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub enum Pane {
    Split {
        axis: Axis,
        ratio: f32,
        a: Box<Pane>,
        b: Box<Pane>,
    },
    #[default]
    Starter,
    HeatmapChart {
        layout: ChartLayout,
        #[serde(deserialize_with = "ok_or_default")]
        stream_type: Vec<StreamType>,
        #[serde(deserialize_with = "ok_or_default")]
        settings: PaneSettings,
        indicators: Vec<HeatmapIndicator>,
    },
    FootprintChart {
        layout: ChartLayout,
        #[serde(deserialize_with = "ok_or_default")]
        stream_type: Vec<StreamType>,
        #[serde(deserialize_with = "ok_or_default")]
        settings: PaneSettings,
        indicators: Vec<FootprintIndicator>,
    },
    CandlestickChart {
        layout: ChartLayout,
        #[serde(deserialize_with = "ok_or_default")]
        stream_type: Vec<StreamType>,
        #[serde(deserialize_with = "ok_or_default")]
        settings: PaneSettings,
        indicators: Vec<CandlestickIndicator>,
    },
    TimeAndSales {
        stream_type: Vec<StreamType>,
        settings: PaneSettings,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct PaneSettings {
    pub ticker_info: Option<TickerInfo>,
    pub tick_multiply: Option<TickMultiplier>,
    pub visual_config: Option<VisualConfig>,
    pub selected_basis: Option<Basis>,
}

pub fn ok_or_default<'a, T, D>(deserializer: D) -> Result<T, D::Error>
where
    T: Deserialize<'a> + Default,
    D: Deserializer<'a>,
{
    let v: Value = Deserialize::deserialize(deserializer)?;
    Ok(T::deserialize(v).unwrap_or_default())
}
