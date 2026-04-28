use super::{IndicatorAvailability, IndicatorPanelRecipe};
use crate::widget::chart::kline::composition::{
    AxisBinding, BarMode, DataSourceId, HistogramMode, LayerDataKind, LayerPresentation, MarkKind,
    PanelScaleMode,
};

pub fn panel_recipe() -> IndicatorPanelRecipe {
    IndicatorPanelRecipe::AuxPanel {
        panel_title: "Volume",
        layer_name: "Volume",
        source: DataSourceId::Primary,
        data_kind: LayerDataKind::Histogram,
        presentation: LayerPresentation {
            mark: MarkKind::Bar(BarMode::Histogram(HistogramMode::SignedOverlay)),
        },
        axis: AxisBinding::Secondary,
        preferred_scale: PanelScaleMode::FitVisibleIncludeZero,
    }
}

pub fn availability() -> IndicatorAvailability {
    IndicatorAvailability::Available
}
