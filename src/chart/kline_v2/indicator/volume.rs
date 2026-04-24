use super::{IndicatorAvailability, IndicatorPanelRecipe};
use crate::chart::composition::{
    AxisBinding, DataSourceId, LayerDataKind, MarkKind, PanelScaleMode,
};

pub fn panel_recipe() -> IndicatorPanelRecipe {
    IndicatorPanelRecipe::AuxPanel {
        panel_title: "Volume",
        layer_name: "Volume",
        source: DataSourceId::Primary,
        data_kind: LayerDataKind::Histogram,
        mark: MarkKind::Bar,
        axis: AxisBinding::Secondary,
        preferred_scale: PanelScaleMode::Absolute,
    }
}

pub fn availability() -> IndicatorAvailability {
    IndicatorAvailability::Available
}
