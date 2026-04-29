use super::{IndicatorAvailability, IndicatorPanelRecipe};
use crate::widget::chart::kline::composition::{
    AxisBinding, BarMode, DataSourceId, HistogramMode, LayerDataKind, MarkKind, PanelScaleMode,
    PanelValuePrecision,
};

pub fn panel_recipe() -> IndicatorPanelRecipe {
    IndicatorPanelRecipe::AuxPanel {
        panel_title: "Volume",
        layer_name: "Volume",
        source: DataSourceId::Primary,
        data_kind: LayerDataKind::Histogram,
        mark: MarkKind::Bar(BarMode::Histogram(HistogramMode::SignedOverlay)),
        axis: AxisBinding::Secondary,
        value_precision: PanelValuePrecision::BaseTickerMinQty,
        preferred_scale: PanelScaleMode::FitVisibleIncludeZero,
    }
}

pub fn availability() -> IndicatorAvailability {
    IndicatorAvailability::Available
}
