use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
pub enum KlineChartKind {
    #[default]
    Candles,
    Footprint(ClusterKind),
}

impl KlineChartKind {
    pub fn min_scaling(&self) -> f32 {
        match self {
            KlineChartKind::Footprint(_) => 0.4,
            KlineChartKind::Candles => 0.6,
        }
    }

    pub fn max_scaling(&self) -> f32 {
        match self {
            KlineChartKind::Footprint(_) => 1.2,
            KlineChartKind::Candles => 2.5,
        }
    }

    pub fn max_cell_width(&self) -> f32 {
        match self {
            KlineChartKind::Footprint(_) => 360.0,
            KlineChartKind::Candles => 16.0,
        }
    }

    pub fn min_cell_width(&self) -> f32 {
        match self {
            KlineChartKind::Footprint(_) => 80.0,
            KlineChartKind::Candles => 1.0,
        }
    }

    pub fn max_cell_height(&self) -> f32 {
        match self {
            KlineChartKind::Footprint(_) => 90.0,
            KlineChartKind::Candles => 8.0,
        }
    }

    pub fn min_cell_height(&self) -> f32 {
        match self {
            KlineChartKind::Footprint(_) => 1.0,
            KlineChartKind::Candles => 0.001,
        }
    }

    pub fn default_cell_width(&self) -> f32 {
        match self {
            KlineChartKind::Footprint(_) => 80.0,
            KlineChartKind::Candles => 4.0,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub enum ClusterKind {
    #[default]
    BidAsk,
    VolumeProfile,
    DeltaProfile,
}

impl ClusterKind {
    pub const ALL: [ClusterKind; 3] = [
        ClusterKind::BidAsk,
        ClusterKind::VolumeProfile,
        ClusterKind::DeltaProfile,
    ];
}

impl std::fmt::Display for ClusterKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClusterKind::BidAsk => write!(f, "Bid/Ask"),
            ClusterKind::VolumeProfile => write!(f, "Volume Profile"),
            ClusterKind::DeltaProfile => write!(f, "Delta Profile"),
        }
    }
}

#[derive(Debug, Default, Copy, Clone, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub cluster_kind: Option<ClusterKind>,
}

impl Config {
    pub fn new() -> Self {
        Self { cluster_kind: None }
    }

    pub fn with_cluster_kind(cluster_kind: ClusterKind) -> Self {
        Self {
            cluster_kind: Some(cluster_kind),
        }
    }
}
