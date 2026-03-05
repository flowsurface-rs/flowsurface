use std::fmt::{self, Debug, Display};

use enum_map::Enum;
use exchange::adapter::MarketKind;
use serde::{Deserialize, Serialize};

pub trait Indicator: PartialEq + Display + 'static {
    fn for_market(market: MarketKind) -> &'static [Self]
    where
        Self: Sized;
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize, Eq, Enum)]
pub enum KlineIndicator {
    Volume,
    OpenInterest,
    Delta,
    TradeCount,
    OFI,
    // GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97
    OFICumulativeEma,
    TradeIntensity,
    /// Rolling log-quantile percentile heatmap for trade intensity (open deviation bars).
    // GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97
    TradeIntensityHeatmap,
    /// Streaming ZigZag swing structure overlay (confirmed + pending pivots).
    ZigZag,
}

impl Indicator for KlineIndicator {
    fn for_market(market: MarketKind) -> &'static [Self] {
        match market {
            MarketKind::Spot => &Self::FOR_SPOT,
            MarketKind::LinearPerps | MarketKind::InversePerps => &Self::FOR_PERPS,
        }
    }
}

impl KlineIndicator {
    /// Whether this indicator renders as its own subplot panel.
    /// Returns `false` for overlays that draw on the main candle pane.
    pub fn has_subplot(&self) -> bool {
        !matches!(self, Self::TradeIntensityHeatmap | Self::ZigZag)
    }

    // Indicator togglers on UI menus depend on these arrays.
    // Every variant needs to be in either SPOT, PERPS or both.
    /// Indicators that can be used with spot market tickers
    const FOR_SPOT: [KlineIndicator; 8] = [
        KlineIndicator::Volume,
        KlineIndicator::Delta,
        KlineIndicator::TradeCount,
        KlineIndicator::OFI,
        KlineIndicator::OFICumulativeEma,
        KlineIndicator::TradeIntensity,
        KlineIndicator::TradeIntensityHeatmap,
        KlineIndicator::ZigZag,
    ];
    /// Indicators that can be used with perpetual swap market tickers
    const FOR_PERPS: [KlineIndicator; 9] = [
        KlineIndicator::Volume,
        KlineIndicator::OpenInterest,
        KlineIndicator::Delta,
        KlineIndicator::TradeCount,
        KlineIndicator::OFI,
        KlineIndicator::OFICumulativeEma,
        KlineIndicator::TradeIntensity,
        KlineIndicator::TradeIntensityHeatmap,
        KlineIndicator::ZigZag,
    ];
}

impl Display for KlineIndicator {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            KlineIndicator::Volume => write!(f, "Volume"),
            KlineIndicator::OpenInterest => write!(f, "Open Interest"),
            KlineIndicator::Delta => write!(f, "Delta"),
            KlineIndicator::TradeCount => write!(f, "Trade Count"),
            KlineIndicator::OFI => write!(f, "OFI"),
            KlineIndicator::OFICumulativeEma => write!(f, "OFI Σ EMA"),
            KlineIndicator::TradeIntensity => write!(f, "Trade Intensity"),
            KlineIndicator::TradeIntensityHeatmap => write!(f, "Intensity Heatmap"),
            KlineIndicator::ZigZag => write!(f, "ZigZag"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize, Eq, Enum)]
pub enum HeatmapIndicator {
    Volume,
}

impl Indicator for HeatmapIndicator {
    fn for_market(market: MarketKind) -> &'static [Self] {
        match market {
            MarketKind::Spot => &Self::FOR_SPOT,
            MarketKind::LinearPerps | MarketKind::InversePerps => &Self::FOR_PERPS,
        }
    }
}

impl HeatmapIndicator {
    // Indicator togglers on UI menus depend on these arrays.
    // Every variant needs to be in either SPOT, PERPS or both.
    /// Indicators that can be used with spot market tickers
    const FOR_SPOT: [HeatmapIndicator; 1] = [HeatmapIndicator::Volume];
    /// Indicators that can be used with perpetual swap market tickers
    const FOR_PERPS: [HeatmapIndicator; 1] = [HeatmapIndicator::Volume];
}

impl Display for HeatmapIndicator {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            HeatmapIndicator::Volume => write!(f, "Volume"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
/// Temporary workaround,
/// represents any indicator type in the UI
pub enum UiIndicator {
    Heatmap(HeatmapIndicator),
    Kline(KlineIndicator),
}

impl From<KlineIndicator> for UiIndicator {
    fn from(k: KlineIndicator) -> Self {
        UiIndicator::Kline(k)
    }
}

impl From<HeatmapIndicator> for UiIndicator {
    fn from(h: HeatmapIndicator) -> Self {
        UiIndicator::Heatmap(h)
    }
}
