use std::fmt::{self, Debug, Display};

use enum_map::Enum;
use exchange::adapter::MarketKind;
use serde::{Deserialize, Serialize};

use crate::chart::style::BuySellColors;

pub trait Indicator: PartialEq + Display + 'static {
    fn for_market(market: MarketKind) -> &'static [Self]
    where
        Self: Sized;
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize, Eq, Enum)]
pub enum KlineIndicator {
    Volume,
    BarAnalysis,
    CumulativeDelta,
    OpenInterest,
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
    // Indicator togglers on UI menus depend on these arrays.
    // Every variant needs to be in either SPOT, PERPS or both.
    /// Indicators that can be used with spot market tickers
    const FOR_SPOT: [KlineIndicator; 3] = [
        KlineIndicator::Volume,
        KlineIndicator::BarAnalysis,
        KlineIndicator::CumulativeDelta,
    ];
    /// Indicators that can be used with perpetual swap market tickers
    const FOR_PERPS: [KlineIndicator; 4] = [
        KlineIndicator::Volume,
        KlineIndicator::BarAnalysis,
        KlineIndicator::CumulativeDelta,
        KlineIndicator::OpenInterest,
    ];
}

impl Display for KlineIndicator {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            KlineIndicator::Volume => write!(f, "Volume"),
            KlineIndicator::BarAnalysis => write!(f, "Bar Analysis"),
            KlineIndicator::CumulativeDelta => write!(f, "CVD"),
            KlineIndicator::OpenInterest => write!(f, "Open Interest"),
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

impl PartialEq for UiIndicator {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (UiIndicator::Heatmap(a), UiIndicator::Heatmap(b)) => a == b,
            (UiIndicator::Kline(a), UiIndicator::Kline(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for UiIndicator {}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KlineVolumeSettings {
    pub bar_width_factor: f32,
    pub show_tooltip: bool,
    pub custom_color_enabled: bool,
    pub custom_buy_color: Option<iced_core::Color>,
    pub custom_sell_color: Option<iced_core::Color>,
    #[serde(flatten)]
    pub colors: BuySellColors,
}

impl Default for KlineVolumeSettings {
    fn default() -> Self {
        Self {
            bar_width_factor: 0.9,
            show_tooltip: true,
            custom_color_enabled: false,
            custom_buy_color: None,
            custom_sell_color: None,
            colors: BuySellColors::default_imbalance(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BarAnalysisSettings {
    pub show_buy_sell: bool,
    pub show_volume: bool,
    pub show_delta: bool,
    pub show_delta_pct: bool,
}

impl Default for BarAnalysisSettings {
    fn default() -> Self {
        Self {
            show_buy_sell: true,
            show_volume: true,
            show_delta: true,
            show_delta_pct: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CumulativeDeltaSettings {
    pub min_directional_run: usize,
    pub show_points: bool,
    pub line_width: f32,
    pub custom_color_enabled: bool,
    pub custom_color: Option<iced_core::Color>,
}

impl Default for CumulativeDeltaSettings {
    fn default() -> Self {
        Self {
            min_directional_run: 2,
            show_points: true,
            line_width: 1.0,
            custom_color_enabled: false,
            custom_color: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenInterestSettings {
    pub show_points: bool,
    pub line_width: f32,
    pub custom_color_enabled: bool,
    pub custom_color: Option<iced_core::Color>,
}

impl Default for OpenInterestSettings {
    fn default() -> Self {
        Self {
            show_points: true,
            line_width: 1.0,
            custom_color_enabled: false,
            custom_color: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct HeatmapVolumeSettings;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum KlineIndicatorConfig {
    Volume(KlineVolumeSettings),
    BarAnalysis(BarAnalysisSettings),
    CumulativeDelta(CumulativeDeltaSettings),
    OpenInterest(OpenInterestSettings),
}

impl<'de> Deserialize<'de> for KlineIndicatorConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        enum Current {
            Volume(KlineVolumeSettings),
            BarAnalysis(BarAnalysisSettings),
            CumulativeDelta(CumulativeDeltaSettings),
            OpenInterest(OpenInterestSettings),
        }

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Compat {
            Current(Current),
            Legacy(KlineIndicator),
        }

        match Compat::deserialize(deserializer)? {
            Compat::Current(Current::Volume(settings)) => Ok(Self::Volume(settings)),
            Compat::Current(Current::BarAnalysis(settings)) => Ok(Self::BarAnalysis(settings)),
            Compat::Current(Current::CumulativeDelta(settings)) => {
                Ok(Self::CumulativeDelta(settings))
            }
            Compat::Current(Current::OpenInterest(settings)) => Ok(Self::OpenInterest(settings)),
            Compat::Legacy(kind) => Ok(Self::default_for(kind)),
        }
    }
}

impl KlineIndicatorConfig {
    pub fn kind(&self) -> KlineIndicator {
        match self {
            Self::Volume(_) => KlineIndicator::Volume,
            Self::BarAnalysis(_) => KlineIndicator::BarAnalysis,
            Self::CumulativeDelta(_) => KlineIndicator::CumulativeDelta,
            Self::OpenInterest(_) => KlineIndicator::OpenInterest,
        }
    }

    pub fn default_for(kind: KlineIndicator) -> Self {
        match kind {
            KlineIndicator::Volume => Self::Volume(KlineVolumeSettings::default()),
            KlineIndicator::BarAnalysis => Self::BarAnalysis(BarAnalysisSettings::default()),
            KlineIndicator::CumulativeDelta => {
                Self::CumulativeDelta(CumulativeDeltaSettings::default())
            }
            KlineIndicator::OpenInterest => Self::OpenInterest(OpenInterestSettings::default()),
        }
    }

    pub fn has_settings(&self) -> bool {
        true
    }
}

impl Display for KlineIndicatorConfig {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(&self.kind(), f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum HeatmapIndicatorConfig {
    Volume(HeatmapVolumeSettings),
}

impl<'de> Deserialize<'de> for HeatmapIndicatorConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        enum Current {
            Volume(HeatmapVolumeSettings),
        }

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Compat {
            Current(Current),
            Legacy(HeatmapIndicator),
        }

        match Compat::deserialize(deserializer)? {
            Compat::Current(Current::Volume(settings)) => Ok(Self::Volume(settings)),
            Compat::Legacy(kind) => Ok(Self::default_for(kind)),
        }
    }
}

impl HeatmapIndicatorConfig {
    pub fn kind(&self) -> HeatmapIndicator {
        match self {
            Self::Volume(_) => HeatmapIndicator::Volume,
        }
    }

    pub fn default_for(kind: HeatmapIndicator) -> Self {
        match kind {
            HeatmapIndicator::Volume => Self::Volume(HeatmapVolumeSettings),
        }
    }

    pub fn has_settings(&self) -> bool {
        false
    }
}

impl Display for HeatmapIndicatorConfig {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(&self.kind(), f)
    }
}
