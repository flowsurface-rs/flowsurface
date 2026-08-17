use exchange::{Ticker, UnixMs};
use iced_core::Color;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PanelRole {
    Primary,
    #[default]
    Auxiliary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MarkKind {
    Candle,
    Bar(BarMode),
    #[default]
    Line,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BarMode {
    #[default]
    Regular,
    Histogram(HistogramMode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum HistogramMode {
    #[default]
    Plain,
    SignedOverlay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LayerDataKind {
    Ohlc,
    #[default]
    Scalar,
    Histogram,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AxisBinding {
    #[default]
    Primary,
    Secondary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LayerSource {
    #[default]
    Primary,
    Comparison,
    Indicator(PanelValueId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PanelScaleMode {
    #[default]
    Absolute,
    FitVisible,
    FitVisibleIncludeZero,
    Logarithmic,
    PercentFromBase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PanelValueId {
    Volume,
    BollingerBands,
    Rsi,
    OpenInterest,
    CumulativeVolumeDelta,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ValuePrecision {
    BaseTickerMinTick,
    BaseTickerMinQty,
    FixedPower10(i8),
    FixedStep(f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ValueLabelPolicy {
    pub axis_mode: ValueLabelMode,
    pub header_mode: ValueLabelMode,
    pub max_decimals: Option<u8>,
}

impl Default for ValueLabelPolicy {
    fn default() -> Self {
        Self {
            axis_mode: ValueLabelMode::Compact,
            header_mode: ValueLabelMode::Commas,
            max_decimals: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ValueLabelMode {
    #[default]
    Compact,
    Commas,
    Abbreviated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ComparisonPolicy {
    pub force_percent_scale_on_multi_source: bool,
    pub force_line_for_non_base_sources: bool,
}

impl Default for ComparisonPolicy {
    fn default() -> Self {
        Self {
            force_percent_scale_on_multi_source: true,
            force_line_for_non_base_sources: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerStyle {
    pub line_width: f32,
    pub opacity: f32,
}

impl Default for LayerStyle {
    fn default() -> Self {
        Self {
            line_width: 1.0,
            opacity: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ViewportConfig {
    pub panel_id: u32,
    pub min_value: f32,
    pub max_value: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DrawingTool {
    #[default]
    Cursor,
    Trendline,
    Box,
    HorizontalLine,
    VerticalLine,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DrawingAnchor {
    pub panel_id: u32,
    pub time: UnixMs,
    pub y_unit: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DrawingObject {
    Trendline {
        start: DrawingAnchor,
        end: DrawingAnchor,
    },
    Box {
        start: DrawingAnchor,
        end: DrawingAnchor,
    },
    HorizontalLine {
        panel_id: u32,
        y_unit: i64,
    },
    VerticalLine {
        time: UnixMs,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct DrawingStyle {
    pub stroke_color: Color,
    pub stroke_width: f32,
    pub fill_color: Option<Color>,
}

impl Default for DrawingStyle {
    fn default() -> Self {
        Self {
            stroke_color: Color::from_rgb(0.82, 0.84, 0.90),
            stroke_width: 1.2,
            fill_color: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrawingConfig {
    pub id: u64,
    pub object: DrawingObject,
    pub style: DrawingStyle,
    pub locked: bool,
    pub visible: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct RsiConfig {
    pub period: u16,
    pub smoothing: RsiSmoothing,
    pub upper_band: f32,
    pub lower_band: f32,
}

impl Default for RsiConfig {
    fn default() -> Self {
        Self {
            period: 14,
            smoothing: RsiSmoothing::Ema(9),
            upper_band: 70.0,
            lower_band: 30.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum RsiSmoothing {
    None,
    Ema(u16),
}

impl Default for RsiSmoothing {
    fn default() -> Self {
        Self::Ema(9)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeriesConfig {
    pub ticker: Ticker,
    pub name: Option<String>,
}
