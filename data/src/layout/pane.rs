use exchange::{TickMultiplier, TickerInfo, adapter::StreamKind};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::chart::{
    Basis, ViewConfig, VisualConfig,
    indicator::{HeatmapIndicator, KlineIndicator},
    kline::KlineChartKind,
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
        layout: ViewConfig,
        #[serde(deserialize_with = "ok_or_default")]
        stream_type: Vec<StreamKind>,
        #[serde(deserialize_with = "ok_or_default")]
        settings: Settings,
        indicators: Vec<HeatmapIndicator>,
    },
    KlineChart {
        layout: ViewConfig,
        kind: KlineChartKind,
        #[serde(deserialize_with = "ok_or_default")]
        stream_type: Vec<StreamKind>,
        #[serde(deserialize_with = "ok_or_default")]
        settings: Settings,
        indicators: Vec<KlineIndicator>,
    },
    TimeAndSales {
        stream_type: Vec<StreamKind>,
        settings: Settings,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct Settings {
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
