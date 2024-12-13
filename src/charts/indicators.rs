pub mod volume;
pub mod open_interest;

use std::{any::Any, fmt::{self, Debug, Display}};

use serde::{Deserialize, Serialize};

pub trait Indicator: PartialEq + Display + ToString + Debug + 'static  {
    fn get_available() -> &'static [Self] where Self: Sized;
    fn get_enabled(indicators: &[Self]) -> impl Iterator<Item = &Self> 
    where
        Self: Sized,
    {
        Self::get_available()
            .iter()
            .filter(move |indicator| indicators.contains(indicator))
    }
    fn as_any(&self) -> &dyn Any;
}

/// Candlestick chart indicators
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub enum CandlestickIndicator {
    Volume,
    OpenInterest,
}

impl Indicator for CandlestickIndicator {
    fn get_available() -> &'static [Self] {
        &Self::ALL
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl CandlestickIndicator {
    const ALL: [CandlestickIndicator; 2] = [CandlestickIndicator::Volume, CandlestickIndicator::OpenInterest];
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
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub enum HeatmapIndicator {
    Volume,
    Spread,
}

impl Indicator for HeatmapIndicator {
    fn get_available() -> &'static [Self] {
        &Self::ALL
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl HeatmapIndicator {
    const ALL: [HeatmapIndicator; 2] = [HeatmapIndicator::Volume, HeatmapIndicator::Spread];
}

impl Display for HeatmapIndicator {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            HeatmapIndicator::Volume => write!(f, "Volume"),
            HeatmapIndicator::Spread => write!(f, "Spread"),
        }
    }
}

/// Footprint chart indicators
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub enum FootprintIndicator {
    Volume,
    OpenInterest,
}

impl Indicator for FootprintIndicator {
    fn get_available() -> &'static [Self] {
        &Self::ALL
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl FootprintIndicator {
    const ALL: [FootprintIndicator; 2] = [FootprintIndicator::Volume, FootprintIndicator::OpenInterest];
}

impl Display for FootprintIndicator {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            FootprintIndicator::Volume => write!(f, "Volume"),
            FootprintIndicator::OpenInterest => write!(f, "Open Interest"),
        }
    }
}