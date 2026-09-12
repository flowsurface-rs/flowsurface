#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaneTarget {
    Main,
    Indicator,
    Named(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlotStyle {
    Line,
    Step,
    Histogram,
    Area,
    Candles,
    Bars,
    Cloud,
    BrokenCloud,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlotValueType {
    Number,
    Boolean,
    Ohlc,
    Range,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlotScale {
    Price,
    Independent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReadoutBehavior {
    Hidden,
    Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresentationViewKind {
    Plot,
    Table,
    PrimaryOverlay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresentationAttachmentMode {
    Overlay,
    Panel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresentationDataKind {
    Scalar,
    Histogram,
    Ohlc,
    Range,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PresentationMarkKind {
    Candle,
    RegularBar,
    Histogram,
    HistogramSignedOverlay,
    Line,
    Step,
    Area,
    Point,
    Cross,
    Segment,
    HorizontalLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresentationInputSource {
    Klines,
    DirectionalVolume,
    Trades,
    OpenInterest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresentationAxis {
    Primary,
    Secondary,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PresentationPrecision {
    BaseTickerMinTick,
    BaseTickerMinQty,
    FixedStep(f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresentationLabelMode {
    Compact,
    Commas,
    Abbreviated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresentationScale {
    Absolute,
    FitVisible,
    FitVisibleIncludeZero,
    Logarithmic,
    PercentFromBase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PresentationGeometry {
    PanelBand,
    SeriesBand,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PresentationStyle {
    pub id: String,
    pub label: String,
    pub role: String,
    pub shade: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PresentationLayer {
    pub channel: String,
    pub name: String,
    pub pane: PaneTarget,
    pub data_kind: Option<PresentationDataKind>,
    pub mark: Option<PresentationMarkKind>,
    pub axis: Option<PresentationAxis>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PresentationMark {
    pub label: String,
    pub kind: PresentationMarkKind,
    pub source_channel: String,
    pub required_input: Option<PresentationInputSource>,
    pub fallback_mark: Option<PresentationMarkKind>,
    /// The color used when no conditional style is active.
    pub color_slot: String,
    /// Optional explicit colors selected by a boolean style binding.
    pub true_color_slot: Option<String>,
    pub false_color_slot: Option<String>,
    /// Optional boolean output that draws a non-destructive bar outline.
    pub highlight_by: Option<String>,
    /// Color slot used by the optional bar outline.
    pub highlight_color_slot: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PresentationSegment {
    pub id: String,
    pub label: String,
    pub color_slot: String,
    pub line_width: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PresentationFill {
    pub upper: String,
    pub lower: String,
    pub upper_offset_bars: i32,
    pub lower_offset_bars: i32,
    pub style_by: Option<String>,
    pub geometry: PresentationGeometry,
    pub color_slot: String,
    pub true_color_slot: Option<String>,
    pub false_color_slot: Option<String>,
    pub opacity: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PresentationReadout {
    pub label: String,
    pub channel: String,
    pub color_slot: String,
    pub mark: Option<PresentationMarkKind>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PresentationColumnColor {
    Static {
        slot: String,
    },
    Signed {
        positive_slot: String,
        negative_slot: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct PresentationColumn {
    pub label: String,
    pub channel: String,
    pub format: String,
    pub color: PresentationColumnColor,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndicatorPresentation {
    pub view: PresentationViewKind,
    pub attachment: Option<PresentationAttachmentMode>,
    pub panel_title: Option<String>,
    pub layer_name: Option<String>,
    pub value_precision: Option<PresentationPrecision>,
    pub axis_label_mode: Option<PresentationLabelMode>,
    pub header_label_mode: Option<PresentationLabelMode>,
    pub max_decimals: Option<u8>,
    pub preferred_scale: Option<PresentationScale>,
    pub layers: Vec<PresentationLayer>,
    pub styles: Vec<PresentationStyle>,
    pub region_style_slots: Vec<String>,
    pub marks: Vec<PresentationMark>,
    pub segments: Vec<PresentationSegment>,
    pub fills: Vec<PresentationFill>,
    pub readouts: Vec<PresentationReadout>,
    pub columns: Vec<PresentationColumn>,
}

impl Default for IndicatorPresentation {
    fn default() -> Self {
        Self {
            view: PresentationViewKind::Plot,
            attachment: None,
            panel_title: None,
            layer_name: None,
            value_precision: None,
            axis_label_mode: None,
            header_label_mode: None,
            max_decimals: None,
            preferred_scale: None,
            layers: Vec::new(),
            styles: Vec::new(),
            region_style_slots: Vec::new(),
            marks: Vec::new(),
            segments: Vec::new(),
            fills: Vec::new(),
            readouts: Vec::new(),
            columns: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlotDescriptor {
    pub id: String,
    pub label: String,
    pub value_type: PlotValueType,
    pub pane: PaneTarget,
    pub scale: PlotScale,
    pub width: f32,
    pub output_precision: Option<OutputPrecision>,
    pub style: PlotStyle,
    pub readout: ReadoutBehavior,
    pub offset_bars: i32,
    pub color_slot: Option<String>,
    pub style_by: Option<String>,
    pub true_color_slot: Option<String>,
    pub false_color_slot: Option<String>,
    pub layer_title: Option<String>,
    pub overlay: bool,
    pub presentation_data_kind: Option<PresentationDataKind>,
    pub presentation_mark: Option<PresentationMarkKind>,
    pub presentation_axis: Option<PresentationAxis>,
    pub layer: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputPrecision {
    Instrument,
    Decimals(u8),
    Step(f64),
}

impl OutputPrecision {
    pub const fn is_valid(self) -> bool {
        match self {
            Self::Instrument | Self::Decimals(_) => true,
            Self::Step(step) => step.is_finite() && step > 0.0,
        }
    }
}
