use data::chart::Basis;
use data::chart::kline::composition::{
    BarMode, HistogramMode, LayerDataKind, MarkKind, PanelScaleMode,
};
use data::chart::kline::{
    CandlestickConfig, FootprintImbalanceConfig, FootprintPocConfig, IndicatorColorSlot,
    IndicatorConfig, IndicatorInstanceId, KlineIndicator, KlineStudy, KlineStudyId,
    LineStrokeStyle, SeriesColor, ThemeAccent,
};
use data::chart::kline_data::{ClusterKind, ClusterScaling};
use data::config::theme::ThemeColorShade;
use exchange::adapter::StreamKind;
use exchange::{TickMultiplier, TickerInfo};

pub enum ChartCommand {
    SetPrimaryScaleMode(PanelScaleMode),
    SetBasis(Basis),
    SetTradeDataEnabled(bool),
    Candlestick(CandlestickCommand),
    Footprint(FootprintCommand),
    Indicator(IndicatorCommand),
    Study(StudyCommand),
    Panel(PanelCommand),
    Series(SeriesCommand),
    Ticker(TickerCommand),
}

pub enum CandlestickCommand {
    SetConfig(CandlestickConfig),
    ResetSettings,
    SetMark(MarkKind),
    SetVisible(bool),
}

pub enum FootprintCommand {
    SetVisible(bool),
    SetClusterKind(ClusterKind),
    SetClusterScaling(ClusterScaling),
    SetImbalanceConfig(FootprintImbalanceConfig),
    SetPocConfig(FootprintPocConfig),
    ResetSettings,
    SetTickMultiplier(TickMultiplier),
}

pub enum IndicatorCommand {
    SetDataConfig {
        id: IndicatorInstanceId,
        config: IndicatorConfig,
    },
    SetColor {
        id: IndicatorInstanceId,
        slot: IndicatorColorSlot,
        accent: Option<ThemeAccent>,
        shade: ThemeColorShade,
    },
    SetFillVisibility {
        id: IndicatorInstanceId,
        visible: bool,
    },
    SetLineWidth {
        id: IndicatorInstanceId,
        slot: IndicatorColorSlot,
        width: u8,
    },
    SetStrokeStyle {
        id: IndicatorInstanceId,
        slot: IndicatorColorSlot,
        stroke_style: LineStrokeStyle,
    },
    ResetSettings(IndicatorInstanceId),
    SetMark {
        id: IndicatorInstanceId,
        mark: MarkKind,
    },
    ToggleVisibility(IndicatorInstanceId),
    Add(KlineIndicator),
    Remove(IndicatorInstanceId),
}

pub enum StudyCommand {
    Set(Vec<KlineStudy>),
    Toggle(KlineStudy),
    ToggleVisibility(KlineStudyId),
    ToggleAllVisibility,
    ToggleExpansion,
    Remove(KlineStudyId),
    RemoveAll,
}

pub enum PanelCommand {
    SetMark {
        panel_index: usize,
        mark: MarkKind,
    },
    SetDataKind {
        panel_index: usize,
        data_kind: LayerDataKind,
    },
    SetBarMode {
        panel_index: usize,
        mode: BarMode,
    },
    SetHistogramMode {
        panel_index: usize,
        mode: HistogramMode,
    },
}

pub enum SeriesCommand {
    SetColor {
        ticker_info: TickerInfo,
        color: SeriesColor,
    },
    SetLineWidth {
        ticker_info: TickerInfo,
        width: u8,
    },
    SetStrokeStyle {
        ticker_info: TickerInfo,
        stroke_style: LineStrokeStyle,
    },
    ResetSettings {
        ticker_info: TickerInfo,
    },
}

pub enum TickerCommand {
    Add(TickerInfo),
    Remove(TickerInfo),
}

pub enum CommandResult {
    None,
    Changed(bool),
    Streams(Vec<StreamKind>),
    Action(Option<crate::screen::dashboard::pane::ChartAction>),
}
