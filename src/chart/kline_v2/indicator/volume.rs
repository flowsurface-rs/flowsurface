use super::{IndicatorAvailability, IndicatorPanelRecipe};
use crate::widget::chart::kline::composition::{
    AxisBinding, BarMode, DataSourceId, HistogramMode, LayerDataKind, MarkKind, PanelScaleMode,
    PanelValueLabelMode, PanelValueLabelPolicy, PanelValuePrecision,
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
        value_label_policy: PanelValueLabelPolicy {
            axis_mode: PanelValueLabelMode::Abbreviated,
            header_mode: PanelValueLabelMode::Commas,
            max_decimals: None,
        },
        preferred_scale: PanelScaleMode::FitVisibleIncludeZero,
    }
}

pub fn availability() -> IndicatorAvailability {
    IndicatorAvailability::Available
}
