use exchange::adapter::PersistStreamKind;
use serde::{Deserialize, Serialize};

use crate::chart::{heatmap, kline};
use crate::panel::{ladder, timeandsales};
use crate::util::ok_or_default;

use crate::chart::{
    Basis, ViewConfig,
    heatmap::HeatmapStudy,
    indicator::{HeatmapIndicator, KlineIndicator},
    kline::KlineChartKind,
};

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum Pane {
    Split {
        axis: Axis,
        ratio: f32,
        a: Box<Pane>,
        b: Box<Pane>,
    },
    Starter {
        #[serde(deserialize_with = "ok_or_default", default)]
        link_group: Option<LinkGroup>,
    },
    HeatmapChart {
        layout: ViewConfig,
        #[serde(deserialize_with = "ok_or_default", default)]
        studies: Vec<HeatmapStudy>,
        #[serde(deserialize_with = "ok_or_default", default)]
        stream_type: Vec<PersistStreamKind>,
        #[serde(deserialize_with = "ok_or_default")]
        settings: Settings,
        #[serde(deserialize_with = "ok_or_default", default)]
        indicators: Vec<HeatmapIndicator>,
        #[serde(deserialize_with = "ok_or_default", default)]
        link_group: Option<LinkGroup>,
    },
    KlineChart {
        layout: ViewConfig,
        kind: KlineChartKind,
        #[serde(deserialize_with = "ok_or_default", default)]
        stream_type: Vec<PersistStreamKind>,
        #[serde(deserialize_with = "ok_or_default")]
        settings: Settings,
        #[serde(deserialize_with = "ok_or_default", default)]
        indicators: Vec<KlineIndicator>,
        #[serde(deserialize_with = "ok_or_default", default)]
        link_group: Option<LinkGroup>,
    },
    TimeAndSales {
        stream_type: Vec<PersistStreamKind>,
        settings: Settings,
        #[serde(deserialize_with = "ok_or_default", default)]
        link_group: Option<LinkGroup>,
    },
    Ladder {
        stream_type: Vec<PersistStreamKind>,
        settings: Settings,
        #[serde(deserialize_with = "ok_or_default", default)]
        link_group: Option<LinkGroup>,
    },
}

impl Default for Pane {
    fn default() -> Self {
        Pane::Starter { link_group: None }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct Settings {
    pub tick_multiply: Option<exchange::TickMultiplier>,
    pub visual_config: Option<VisualConfig>,
    pub selected_basis: Option<Basis>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum LinkGroup {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
}

impl LinkGroup {
    pub const ALL: [LinkGroup; 9] = [
        LinkGroup::A,
        LinkGroup::B,
        LinkGroup::C,
        LinkGroup::D,
        LinkGroup::E,
        LinkGroup::F,
        LinkGroup::G,
        LinkGroup::H,
        LinkGroup::I,
    ];
}

impl std::fmt::Display for LinkGroup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let c = match self {
            LinkGroup::A => "1",
            LinkGroup::B => "2",
            LinkGroup::C => "3",
            LinkGroup::D => "4",
            LinkGroup::E => "5",
            LinkGroup::F => "6",
            LinkGroup::G => "7",
            LinkGroup::H => "8",
            LinkGroup::I => "9",
        };
        write!(f, "{c}")
    }
}

/// Defines the specific configuration for different types of pane settings.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub enum VisualConfig {
    Heatmap(heatmap::Config),
    TimeAndSales(timeandsales::Config),
    Kline(kline::Config),
    Ladder(ladder::Config),
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

    pub fn ladder(&self) -> Option<ladder::Config> {
        match self {
            Self::Ladder(cfg) => Some(*cfg),
            _ => None,
        }
    }
}
