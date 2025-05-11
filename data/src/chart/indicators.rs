use std::{
    any::Any,
    fmt::{self, Debug, Display},
};

use exchange::adapter::MarketKind;
use serde::{Deserialize, Serialize};

pub trait Indicator: PartialEq + Display + 'static {
    fn get_available(market: MarketKind) -> &'static [Self]
    where
        Self: Sized;

    fn get_enabled(indicators: &[Self], market: MarketKind) -> impl Iterator<Item = &Self>
    where
        Self: Sized,
    {
        Self::get_available(market)
            .iter()
            .filter(move |indicator| indicators.contains(indicator))
    }
    fn as_any(&self) -> &dyn Any;
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize, Eq, Hash)]
pub enum KlineIndicator {
    Volume,
    OpenInterest,
}

impl Indicator for KlineIndicator {
    fn get_available(market_type: MarketKind) -> &'static [Self] {
        match market_type {
            MarketKind::Spot => &Self::SPOT,
            MarketKind::LinearPerps | MarketKind::InversePerps => &Self::PERPS,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl KlineIndicator {
    const SPOT: [KlineIndicator; 1] = [KlineIndicator::Volume];
    const PERPS: [KlineIndicator; 2] = [KlineIndicator::Volume, KlineIndicator::OpenInterest];
}

impl Display for KlineIndicator {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            KlineIndicator::Volume => write!(f, "Volume"),
            KlineIndicator::OpenInterest => write!(f, "Open Interest"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize, Eq, Hash)]
pub enum CandlestickIndicator {
    Volume,
    OpenInterest,
}

impl Indicator for CandlestickIndicator {
    fn get_available(market_type: MarketKind) -> &'static [Self] {
        match market_type {
            MarketKind::Spot => &Self::SPOT,
            MarketKind::LinearPerps | MarketKind::InversePerps => &Self::PERPS,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl CandlestickIndicator {
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

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize, Eq, Hash)]
pub enum HeatmapIndicator {
    Volume,
    SessionVolumeProfile,
}

impl Indicator for HeatmapIndicator {
    fn get_available(market_type: MarketKind) -> &'static [Self] {
        match market_type {
            MarketKind::Spot => &Self::SPOT,
            MarketKind::LinearPerps | MarketKind::InversePerps => &Self::PERPS,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl HeatmapIndicator {
    const SPOT: [HeatmapIndicator; 2] = [
        HeatmapIndicator::Volume,
        HeatmapIndicator::SessionVolumeProfile,
    ];
    const PERPS: [HeatmapIndicator; 2] = [
        HeatmapIndicator::Volume,
        HeatmapIndicator::SessionVolumeProfile,
    ];
}

impl Display for HeatmapIndicator {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            HeatmapIndicator::Volume => write!(f, "Volume"),
            HeatmapIndicator::SessionVolumeProfile => write!(f, "VPSR"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize, Eq, Hash)]
pub enum FootprintIndicator {
    Volume,
    OpenInterest,
}

impl Indicator for FootprintIndicator {
    fn get_available(market_type: MarketKind) -> &'static [Self] {
        match market_type {
            MarketKind::Spot => &Self::SPOT,
            MarketKind::LinearPerps | MarketKind::InversePerps => &Self::PERPS,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl FootprintIndicator {
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
