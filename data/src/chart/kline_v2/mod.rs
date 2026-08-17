use enum_map::Enum;
use exchange::Ticker;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

use composition::{
    ChartComposition, LayerSource as DomainLayerSource, LayerSpec, PanelSpec,
    PanelValuePrecision as DomainPanelValuePrecision,
};

pub mod composition;
pub mod drawing;
pub mod indicators;
pub mod series;
pub mod values;
pub mod viewport;

pub use viewport::{ChartCoord, ChartStepMs, HorizontalWorld, PanelYViewport, WorldAxis, YUnit};

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize, Eq, Enum)]
pub enum KlineIndicator {
    Volume,
    BollingerBands,
    Rsi,
    OpenInterest,
    CumulativeVolumeDelta,
}

impl KlineIndicator {
    pub fn for_market(market: exchange::adapter::MarketKind) -> &'static [Self] {
        indicators::indicators_for_market(market)
    }
}

impl Display for KlineIndicator {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(indicators::display_name(*self))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u16,
    pub horizontal: HorizontalConfig,
    pub primary_autoscale: bool,
    pub panels: Vec<PanelConfig>,
    pub splits: Vec<f32>,
    pub y_viewports: Vec<ViewportConfig>,
    pub drawings: Vec<DrawingConfig>,
    pub active_drawing_tool: DrawingTool,
    pub rsi: RsiConfig,
    pub series: Vec<SeriesConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            horizontal: HorizontalConfig::default(),
            primary_autoscale: true,
            panels: Vec::new(),
            splits: Vec::new(),
            y_viewports: Vec::new(),
            drawings: Vec::new(),
            active_drawing_tool: DrawingTool::Cursor,
            rsi: RsiConfig::default(),
            series: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct HorizontalConfig {
    pub cell_width: f32,
    pub scaling: f32,
    pub offset: f32,
}

impl Default for HorizontalConfig {
    fn default() -> Self {
        Self {
            cell_width: 5.0,
            scaling: 1.0,
            offset: 8.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelConfig {
    pub id: u32,
    pub role: PanelRole,
    pub title: Option<String>,
    pub value_id: Option<PanelValueId>,
    pub value_precision: Option<ValuePrecision>,
    pub value_label_policy: ValueLabelPolicy,
    pub base_layer: Option<u32>,
    pub preferred_scale: PanelScaleMode,
    pub comparison_policy: ComparisonPolicy,
    pub layers: Vec<LayerConfig>,
}

impl Default for PanelConfig {
    fn default() -> Self {
        Self {
            id: 0,
            role: PanelRole::Auxiliary,
            title: None,
            value_id: None,
            value_precision: None,
            value_label_policy: ValueLabelPolicy::default(),
            base_layer: None,
            preferred_scale: PanelScaleMode::Absolute,
            comparison_policy: ComparisonPolicy::default(),
            layers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerConfig {
    pub id: u32,
    pub name: String,
    pub source: LayerSource,
    pub data_kind: LayerDataKind,
    pub mark: MarkKind,
    pub axis: AxisBinding,
    pub visible: bool,
    pub style: LayerStyle,
}

impl Default for LayerConfig {
    fn default() -> Self {
        Self {
            id: 0,
            name: String::new(),
            source: LayerSource::Primary,
            data_kind: LayerDataKind::Scalar,
            mark: MarkKind::Line,
            axis: AxisBinding::Primary,
            visible: true,
            style: LayerStyle::default(),
        }
    }
}

pub use composition::{
    AxisBinding, BarMode, HistogramMode, LayerDataKind, LayerStyle, MarkKind, PanelRole,
    PanelScaleMode, PanelValueId, PanelValueLabelMode, PanelValueLabelPolicy,
};
pub type ComparisonPolicy = composition::PanelComparisonPolicy;
pub type ValueLabelPolicy = composition::PanelValueLabelPolicy;
pub type ValueLabelMode = composition::PanelValueLabelMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LayerSource {
    #[default]
    Primary,
    Comparison,
    Indicator(PanelValueId),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ValuePrecision {
    BaseTickerMinTick,
    BaseTickerMinQty,
    FixedPower10(i8),
    FixedStep(f32),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ViewportConfig {
    pub panel_id: u32,
    pub min_value: f32,
    pub max_value: f32,
}

pub use drawing::{DrawingAnchor, DrawingEntity, DrawingObject, DrawingStyle, DrawingTool, Rgba};
pub type DrawingConfig = DrawingEntity;
pub use indicators::{RsiConfig, RsiSmoothing};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeriesConfig {
    pub ticker: Ticker,
    pub name: Option<String>,
}

pub fn panel_config_from_spec(panel: &PanelSpec) -> PanelConfig {
    PanelConfig {
        id: panel.id.0,
        role: panel.role,
        title: panel.title.clone(),
        value_id: panel.value_id,
        value_precision: panel.value_precision.map(value_precision_from_domain),
        value_label_policy: panel.value_label_policy,
        base_layer: panel.base_layer.map(|layer_id| layer_id.0),
        preferred_scale: panel.preferred_scale,
        comparison_policy: panel.comparison_policy,
        layers: panel.layers.iter().map(layer_config_from_spec).collect(),
    }
}

pub fn layer_config_from_spec(layer: &LayerSpec) -> LayerConfig {
    LayerConfig {
        id: layer.id.0,
        name: layer.name.clone(),
        source: layer_source_from_domain(&layer.source),
        data_kind: layer.data_kind,
        mark: layer.mark,
        axis: layer.axis,
        visible: layer.visible,
        style: layer.style.clone(),
    }
}

pub fn panel_spec_from_config(panel: &PanelConfig) -> PanelSpec {
    PanelSpec {
        id: composition::PanelId(panel.id),
        role: panel.role,
        title: panel.title.clone(),
        value_id: panel.value_id,
        value_precision: panel.value_precision.map(value_precision_into_domain),
        value_label_policy: panel.value_label_policy,
        base_layer: panel.base_layer.map(composition::LayerId),
        preferred_scale: panel.preferred_scale,
        comparison_policy: panel.comparison_policy,
        layers: panel.layers.iter().map(layer_spec_from_config).collect(),
    }
}

pub fn layer_spec_from_config(layer: &LayerConfig) -> LayerSpec {
    LayerSpec {
        id: composition::LayerId(layer.id),
        name: layer.name.clone(),
        source: layer_source_into_domain(layer.source),
        data_kind: layer.data_kind,
        mark: layer.mark,
        axis: layer.axis,
        visible: layer.visible,
        style: layer.style.clone(),
    }
}

pub fn composition_from_config(config: &Config) -> ChartComposition {
    ChartComposition::from_parts(
        config.panels.iter().map(panel_spec_from_config).collect(),
        config.splits.clone(),
    )
}

fn layer_source_from_domain(source: &DomainLayerSource) -> LayerSource {
    match source {
        DomainLayerSource::RawKline {
            source: composition::DataSourceId::Synthetic("Comparison"),
        } => LayerSource::Comparison,
        DomainLayerSource::RawKline { .. } => LayerSource::Primary,
        DomainLayerSource::RawIndicator { value_id, .. } => LayerSource::Indicator(*value_id),
    }
}

fn layer_source_into_domain(source: LayerSource) -> DomainLayerSource {
    match source {
        LayerSource::Primary => DomainLayerSource::RawKline {
            source: composition::DataSourceId::Primary,
        },
        LayerSource::Comparison => DomainLayerSource::RawKline {
            source: composition::DataSourceId::Synthetic("Comparison"),
        },
        LayerSource::Indicator(value_id) => DomainLayerSource::RawIndicator {
            source: composition::DataSourceId::Primary,
            value_id,
        },
    }
}

fn value_precision_from_domain(precision: DomainPanelValuePrecision) -> ValuePrecision {
    match precision {
        DomainPanelValuePrecision::BaseTickerMinTick => ValuePrecision::BaseTickerMinTick,
        DomainPanelValuePrecision::BaseTickerMinQty => ValuePrecision::BaseTickerMinQty,
        DomainPanelValuePrecision::FixedPower10(step) => ValuePrecision::FixedPower10(step.power),
        DomainPanelValuePrecision::FixedStep(step) => ValuePrecision::FixedStep(step),
    }
}

fn value_precision_into_domain(precision: ValuePrecision) -> DomainPanelValuePrecision {
    match precision {
        ValuePrecision::BaseTickerMinTick => DomainPanelValuePrecision::BaseTickerMinTick,
        ValuePrecision::BaseTickerMinQty => DomainPanelValuePrecision::BaseTickerMinQty,
        ValuePrecision::FixedPower10(power) => {
            DomainPanelValuePrecision::FixedPower10(exchange::unit::Power10::new(power))
        }
        ValuePrecision::FixedStep(step) => DomainPanelValuePrecision::FixedStep(step),
    }
}
