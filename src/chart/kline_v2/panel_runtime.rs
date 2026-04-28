use crate::widget::chart::kline::KlinePanelKind;
use crate::widget::chart::kline::composition::{
    BarMode, ChartComposition, HistogramMode, LayerDataKind, MarkKind, PanelId, PanelRole,
    PanelScaleMode, default_mark_for_data_kind,
};

use super::KlineIndicator;

#[derive(Debug, Clone, Default)]
pub struct PanelRuntimeState {
    pub kinds: Vec<KlinePanelKind>,
    pub splits: Vec<f32>,
    pub titles: Vec<Option<String>>,
    pub marks: Vec<MarkKind>,
    pub scale_modes: Vec<PanelScaleMode>,
    pub data_kinds: Vec<LayerDataKind>,
    pub indicators: Vec<Option<KlineIndicator>>,
}

impl PanelRuntimeState {
    pub fn panel_indicators(&self) -> &[Option<KlineIndicator>] {
        &self.indicators
    }
}

pub fn derive_panel_runtime_state<FIndicator, FSignedOverlay>(
    composition: &ChartComposition,
    min_panel_ratio: f32,
    mut indicator_for_panel: FIndicator,
    mut signed_overlay_input: FSignedOverlay,
) -> PanelRuntimeState
where
    FIndicator: FnMut(PanelId) -> Option<KlineIndicator>,
    FSignedOverlay: FnMut(Option<KlineIndicator>) -> bool,
{
    let mut runtime = PanelRuntimeState::default();

    for panel in &composition.panels {
        let panel_kind = match panel.role {
            PanelRole::Primary => KlinePanelKind::PrimaryChart,
            PanelRole::Auxiliary => KlinePanelKind::Indicator,
        };

        runtime.kinds.push(panel_kind);
        runtime.titles.push(panel.title.clone());

        let fallback_data_kind = match panel_kind {
            KlinePanelKind::PrimaryChart => LayerDataKind::Ohlc,
            KlinePanelKind::Indicator => LayerDataKind::Scalar,
        };

        let base_layer_id = panel
            .base_layer
            .or_else(|| panel.layers.first().map(|layer| layer.id));

        let effective_data_kind = panel
            .base_layer
            .and_then(|base| panel.layers.iter().find(|layer| layer.id == base))
            .or_else(|| panel.layers.first())
            .map(|layer| layer.data_kind)
            .unwrap_or(fallback_data_kind);

        let panel_indicator = indicator_for_panel(panel.id);
        let signed_overlay = signed_overlay_input(panel_indicator);

        let fallback_mark = match panel_kind {
            KlinePanelKind::PrimaryChart => default_mark_for_data_kind(LayerDataKind::Ohlc),
            KlinePanelKind::Indicator => MarkKind::Bar(BarMode::Histogram(HistogramMode::Plain)),
        };

        let effective_mark = composition
            .resolved_panel_marks_with_runtime(panel.id, signed_overlay)
            .and_then(|resolved_marks| {
                base_layer_id
                    .and_then(|base| {
                        resolved_marks
                            .iter()
                            .find(|(layer_id, _)| *layer_id == base)
                            .map(|(_, mark)| *mark)
                    })
                    .or_else(|| resolved_marks.first().map(|(_, mark)| *mark))
            })
            .unwrap_or(fallback_mark);

        runtime.marks.push(effective_mark);

        let mut scale_mode = composition
            .panel_effective_scale_mode(panel.id)
            .unwrap_or(PanelScaleMode::Absolute);

        if matches!(panel_indicator, Some(KlineIndicator::Volume))
            && matches!(scale_mode, PanelScaleMode::Absolute)
        {
            scale_mode = PanelScaleMode::FitVisibleIncludeZero;
        }

        runtime.scale_modes.push(scale_mode);
        runtime.data_kinds.push(effective_data_kind);
        runtime.indicators.push(panel_indicator);
    }

    runtime.splits = composition.normalized_splits(min_panel_ratio);
    runtime
}
