use std::{
    any::Any,
    fmt::{self, Display},
};

use exchange::adapter::MarketType;
use serde::{Deserialize, Serialize};

pub trait Indicator: PartialEq + Display + std::fmt::Debug + 'static {
    fn get_available(market_type: Option<MarketType>) -> &'static [Self]
    where
        Self: Sized;

    fn get_enabled(
        indicators: &[Self],
        market_type: Option<MarketType>,
    ) -> impl Iterator<Item = &Self>
    where
        Self: Sized,
    {
        Self::get_available(market_type)
            .iter()
            .filter(move |indicator| indicators.contains(indicator))
    }
    fn as_any(&self) -> &dyn Any;
}

/// Candlestick chart indicators
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, serde::Serialize, Eq, Hash)]
pub enum CandlestickIndicator {
    Volume,
    OpenInterest,
}

impl Indicator for CandlestickIndicator {
    fn get_available(market_type: Option<MarketType>) -> &'static [Self] {
        match market_type {
            Some(MarketType::Spot) => &Self::SPOT,
            Some(MarketType::LinearPerps | MarketType::InversePerps) => &Self::PERPS,
            _ => &Self::ALL,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl CandlestickIndicator {
    const ALL: [CandlestickIndicator; 2] = [
        CandlestickIndicator::Volume,
        CandlestickIndicator::OpenInterest,
    ];
    const SPOT: [CandlestickIndicator; 1] = [CandlestickIndicator::Volume];
    const PERPS: [CandlestickIndicator; 2] = [
        CandlestickIndicator::Volume,
        CandlestickIndicator::OpenInterest,
    ];
}

impl Display for CandlestickIndicator {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CandlestickIndicator::Volume => write!(f, "Volume"),
            CandlestickIndicator::OpenInterest => write!(f, "Open Interest"),
        }
    }
}

/// Heatmap chart indicators
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize, Eq, Hash)]
pub enum HeatmapIndicator {
    Volume,
}

impl Indicator for HeatmapIndicator {
    fn get_available(market_type: Option<MarketType>) -> &'static [Self] {
        match market_type {
            Some(MarketType::Spot) => &Self::SPOT,
            Some(MarketType::LinearPerps | MarketType::InversePerps) => &Self::PERPS,
            _ => &Self::ALL,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl HeatmapIndicator {
    const ALL: [HeatmapIndicator; 1] = [HeatmapIndicator::Volume];
    const SPOT: [HeatmapIndicator; 1] = [HeatmapIndicator::Volume];
    const PERPS: [HeatmapIndicator; 1] = [HeatmapIndicator::Volume];
}

impl Display for HeatmapIndicator {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            HeatmapIndicator::Volume => write!(f, "Volume"),
        }
    }
}

/// Footprint chart indicators
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize, Eq, Hash)]
pub enum FootprintIndicator {
    Volume,
    OpenInterest,
}

impl Indicator for FootprintIndicator {
    fn get_available(market_type: Option<MarketType>) -> &'static [Self] {
        match market_type {
            Some(MarketType::Spot) => &Self::SPOT,
            Some(MarketType::LinearPerps | MarketType::InversePerps) => &Self::PERPS,
            _ => &Self::ALL,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl FootprintIndicator {
    const ALL: [FootprintIndicator; 2] =
        [FootprintIndicator::Volume, FootprintIndicator::OpenInterest];
    const SPOT: [FootprintIndicator; 1] = [FootprintIndicator::Volume];
    const PERPS: [FootprintIndicator; 2] =
        [FootprintIndicator::Volume, FootprintIndicator::OpenInterest];
}

impl Display for FootprintIndicator {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            FootprintIndicator::Volume => write!(f, "Volume"),
            FootprintIndicator::OpenInterest => write!(f, "Open Interest"),
        }
    }
}
