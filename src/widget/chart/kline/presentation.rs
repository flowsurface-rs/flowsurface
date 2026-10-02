use std::sync::Arc;

use super::composition::{BarMode, HistogramMode, LayerDataKind, MarkKind, PanelValueId};
use data::chart::kline::{
    IndicatorId, IndicatorInputCoverage, IndicatorInputCoverageState, IndicatorInputSource,
    IndicatorOutputRef, IndicatorPanelRecipe, IndicatorReadoutSpec, LayerPurpose, TableColumnColor,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum IndicatorBarValues {
    Plain {
        value: f64,
    },
    SignedOverlay {
        background_value: f64,
        signed_value: f64,
    },
}

pub(crate) fn resolve_indicator_bar_values(
    mark: MarkKind,
    base_value: Option<f64>,
    presentation_value: Option<f64>,
) -> Option<IndicatorBarValues> {
    let base_value = base_value?;
    if matches!(
        mark,
        MarkKind::Bar(BarMode::Histogram(HistogramMode::SignedOverlay))
    ) {
        return Some(IndicatorBarValues::SignedOverlay {
            background_value: base_value,
            signed_value: presentation_value?,
        });
    }
    Some(IndicatorBarValues::Plain { value: base_value })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IndicatorPresentationStatus {
    Available,
    Loading,
    Fallback {
        selected_mark: MarkKind,
        effective_mark: MarkKind,
        required_input: IndicatorInputSource,
    },
    IndicatorUnavailable {
        message: String,
    },
    PresentationUnavailable {
        message: String,
    },
}

pub(crate) fn resolve_indicator_presentation_status<F>(
    base_message: Option<&str>,
    input_coverage: &IndicatorInputCoverage,
    selected_mark: Option<MarkKind>,
    required_input: Option<IndicatorInputSource>,
    fallback_mark: Option<MarkKind>,
    missing_message: F,
) -> (Option<MarkKind>, IndicatorPresentationStatus)
where
    F: FnOnce(IndicatorInputSource) -> String,
{
    if let Some(message) = base_message {
        return (
            selected_mark,
            IndicatorPresentationStatus::IndicatorUnavailable {
                message: message.to_owned(),
            },
        );
    }
    let Some(required_input) = required_input else {
        return (selected_mark, IndicatorPresentationStatus::Available);
    };
    if !matches!(
        input_coverage.state(required_input),
        IndicatorInputCoverageState::Missing
    ) {
        return (selected_mark, IndicatorPresentationStatus::Available);
    }
    if let (Some(selected_mark), Some(effective_mark)) = (selected_mark, fallback_mark)
        && selected_mark != effective_mark
    {
        return (
            Some(effective_mark),
            IndicatorPresentationStatus::Fallback {
                selected_mark,
                effective_mark,
                required_input,
            },
        );
    }
    (
        selected_mark,
        IndicatorPresentationStatus::PresentationUnavailable {
            message: missing_message(required_input),
        },
    )
}

impl IndicatorPresentationStatus {
    pub(crate) fn is_loading(&self) -> bool {
        matches!(self, Self::Loading)
    }

    pub(crate) fn is_hard_diagnostic(&self) -> bool {
        matches!(self, Self::IndicatorUnavailable { .. })
    }

    pub(crate) fn is_unavailable(&self) -> bool {
        matches!(
            self,
            Self::Loading
                | Self::IndicatorUnavailable { .. }
                | Self::PresentationUnavailable { .. }
        )
    }

    pub(crate) fn message(&self) -> Option<&str> {
        match self {
            Self::Available | Self::Loading | Self::Fallback { .. } => None,
            Self::IndicatorUnavailable { message } | Self::PresentationUnavailable { message } => {
                Some(message.as_str())
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedIndicatorLayer {
    pub(crate) base_output: IndicatorOutputRef,
    pub(crate) presentation_output: IndicatorOutputRef,
    pub(crate) data_kind: LayerDataKind,
    pub(crate) purpose: LayerPurpose,
    pub(crate) mark: MarkKind,
    pub(crate) required_input: Option<IndicatorInputSource>,
    pub(crate) visible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedIndicatorStyleSlot {
    pub(crate) id: String,
    pub(crate) supports_line_styling: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedIndicatorPresentation {
    pub(crate) value_id: PanelValueId,
    pub(crate) effective_mark: Option<MarkKind>,
    pub(crate) status: IndicatorPresentationStatus,
    pub(crate) resolved_layers: Vec<ResolvedIndicatorLayer>,
    pub(crate) active_readouts: Vec<IndicatorReadoutSpec>,
}

#[derive(Debug, Clone)]
pub struct IndicatorPresentationContext {
    pub(crate) input_coverage: IndicatorInputCoverage,
    presentations: Arc<[ResolvedIndicatorPresentation]>,
    rebuild_in_flight_ids: Vec<PanelValueId>,
    pending_input_ids: Vec<PanelValueId>,
}

impl IndicatorPresentationContext {
    pub(crate) fn new(
        input_coverage: IndicatorInputCoverage,
        presentations: Arc<[ResolvedIndicatorPresentation]>,
    ) -> Self {
        Self {
            input_coverage,
            presentations,
            rebuild_in_flight_ids: Vec::new(),
            pending_input_ids: Vec::new(),
        }
    }

    pub(crate) fn with_rebuild_in_flight_ids(
        mut self,
        rebuild_in_flight_ids: Vec<PanelValueId>,
    ) -> Self {
        self.rebuild_in_flight_ids = rebuild_in_flight_ids;
        self
    }

    pub(crate) fn with_pending_input_ids(mut self, pending_input_ids: Vec<PanelValueId>) -> Self {
        self.pending_input_ids = pending_input_ids;
        self
    }

    pub(crate) fn presentation(
        &self,
        value_id: PanelValueId,
    ) -> Option<&ResolvedIndicatorPresentation> {
        self.presentations
            .iter()
            .find(|presentation| presentation.value_id == value_id)
    }

    pub(crate) fn layer_for_output(
        &self,
        output: &IndicatorOutputRef,
    ) -> Option<&ResolvedIndicatorLayer> {
        self.presentations
            .iter()
            .flat_map(|presentation| presentation.resolved_layers.iter())
            .find(|layer| layer.base_output == *output)
    }

    pub(crate) fn is_unavailable(&self, value_id: PanelValueId) -> bool {
        self.presentation(value_id)
            .is_some_and(|presentation| presentation.status.is_unavailable())
    }

    pub(crate) fn is_loading(&self, value_id: PanelValueId) -> bool {
        self.presentation(value_id).is_some_and(|presentation| {
            presentation.status.is_loading()
                || (self.pending_input_ids.contains(&value_id)
                    || self.rebuild_in_flight_ids.contains(&value_id))
                    && !presentation.status.is_hard_diagnostic()
        })
    }

    pub(crate) fn unavailable_message(&self, value_id: PanelValueId) -> Option<&str> {
        self.presentation(value_id)
            .and_then(|presentation| presentation.status.message())
    }

    pub(crate) fn style_slots_for(
        &self,
        value_id: PanelValueId,
        indicator: &IndicatorId,
    ) -> Vec<ResolvedIndicatorStyleSlot> {
        let Some(presentation) = self.presentation(value_id) else {
            return Vec::new();
        };

        let mut active_slots: Vec<ResolvedIndicatorStyleSlot> = Vec::new();
        let mut add_slot = |id: &str, supports_line_styling: bool| {
            if let Some(slot) = active_slots.iter_mut().find(|slot| slot.id == id) {
                slot.supports_line_styling |= supports_line_styling;
            } else {
                active_slots.push(ResolvedIndicatorStyleSlot {
                    id: id.to_owned(),
                    supports_line_styling,
                });
            }
        };

        if matches!(indicator.default_view(), IndicatorPanelRecipe::Table { .. }) {
            return table_style_slots(
                data::chart::kline::table_columns(indicator),
                indicator.color_specs(),
                &self.input_coverage,
            );
        }

        for layer in presentation
            .resolved_layers
            .iter()
            .filter(|layer| layer.visible)
        {
            let mark_spec = indicator.mark_specs().iter().find(|spec| {
                spec.mark == layer.mark
                    && spec.source_channel == layer.presentation_output.channel_id
            });
            if let Some(mark_spec) = mark_spec {
                let supports_line_styling = layer.mark.supports_line_styling();
                add_slot(&mark_spec.color_slot, supports_line_styling);
                if let Some(slot) = mark_spec.true_color_slot.as_deref() {
                    add_slot(slot, supports_line_styling);
                }
                if let Some(slot) = mark_spec.false_color_slot.as_deref() {
                    add_slot(slot, supports_line_styling);
                }
                if let Some(slot) = mark_spec.highlight_color_slot.as_deref() {
                    add_slot(slot, true);
                }
            } else if let Some(style_slot) = indicator
                .output_channels()
                .iter()
                .find(|output| output.id == layer.presentation_output.channel_id)
                .and_then(|output| output.style_slot.as_deref())
            {
                add_slot(style_slot, layer.mark.supports_line_styling());
            }

            for overlay in indicator.overlay_channels().iter().filter(|overlay| {
                let data::chart::kline::IndicatorOverlaySource::Channel(channel) = &overlay.source;
                channel == &layer.presentation_output.channel_id
            }) {
                add_slot(&overlay.color_slot, true);
            }
        }

        for readout in &presentation.active_readouts {
            add_slot(&readout.color_slot, false);
        }

        for fill in indicator.fill_specs() {
            add_slot(&fill.color_slot, false);
            if let Some(slot) = fill.true_color_slot.as_deref() {
                add_slot(slot, false);
            }
            if let Some(slot) = fill.false_color_slot.as_deref() {
                add_slot(slot, false);
            }
        }

        for segment in indicator.segment_specs() {
            add_slot(&segment.color_slot, true);
        }

        for slot in indicator.region_style_slots() {
            add_slot(slot, false);
        }

        indicator
            .color_specs()
            .iter()
            .filter(|spec| {
                spec.required_input.is_none_or(|source| {
                    !matches!(
                        self.input_coverage.state(source),
                        IndicatorInputCoverageState::Missing
                    )
                })
            })
            .filter_map(|spec| active_slots.iter().find(|slot| slot.id == spec.id).cloned())
            .collect()
    }
}

fn table_style_slots(
    columns: &[data::chart::kline::TableColumnSpec],
    style_specs: &[data::chart::kline::IndicatorColorSpec],
    input_coverage: &IndicatorInputCoverage,
) -> Vec<ResolvedIndicatorStyleSlot> {
    let mut active_slots: Vec<ResolvedIndicatorStyleSlot> = Vec::new();
    let mut add_slot = |id: &str| {
        if !active_slots.iter().any(|slot| slot.id == id) {
            active_slots.push(ResolvedIndicatorStyleSlot {
                id: id.to_owned(),
                supports_line_styling: false,
            });
        }
    };

    for column in columns {
        match &column.color {
            TableColumnColor::Static { slot } => add_slot(slot),
            TableColumnColor::Signed {
                positive_slot,
                negative_slot,
            } => {
                add_slot(positive_slot);
                add_slot(negative_slot);
            }
        }
    }

    style_specs
        .iter()
        .filter(|spec| {
            spec.required_input.is_none_or(|source| {
                !matches!(
                    input_coverage.state(source),
                    IndicatorInputCoverageState::Missing
                )
            })
        })
        .filter_map(|spec| active_slots.iter().find(|slot| slot.id == spec.id).cloned())
        .collect()
}
