use super::chrome::IndicatorHeaderValue;
use super::composition::{
    self, BarMode, HistogramMode, LayerDataKind, MarkKind, PanelValueId, resolve_mark_for_data_kind,
};
use super::composition::{ChartComposition, PanelPresentation};
use super::scene::{BarUnitWindow, Scene, XAxis};
use data::chart::kline::series::KlineSeries;
use data::chart::kline::{
    IndicatorId, IndicatorInputCoverage, IndicatorInputCoverageState, IndicatorInputSource,
    IndicatorInstance, IndicatorOutputRef, IndicatorPanelRecipe, IndicatorReadoutSpec,
    LayerPurpose, LineStrokeStyle, OutputChannelSpec, OutputValue, OutputValueType,
    TableColumnColor,
};
use data::config::theme::{ACCENT_COUNT, ThemeColor, ThemeColorRole, ThemeColorShade};
use iced::theme::palette::Extended;
use std::{ops::Range, sync::Arc};

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

fn resolve_indicator_bar_values(
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
    fn is_loading(&self) -> bool {
        matches!(self, Self::Loading)
    }

    fn is_hard_diagnostic(&self) -> bool {
        matches!(self, Self::IndicatorUnavailable { .. })
    }

    fn is_unavailable(&self) -> bool {
        matches!(
            self,
            Self::Loading
                | Self::IndicatorUnavailable { .. }
                | Self::PresentationUnavailable { .. }
        )
    }

    fn message(&self) -> Option<&str> {
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
pub(crate) struct IndicatorPresentationContext {
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

    fn layer_for_output(&self, output: &IndicatorOutputRef) -> Option<&ResolvedIndicatorLayer> {
        self.presentations
            .iter()
            .flat_map(|presentation| presentation.resolved_layers.iter())
            .find(|layer| layer.base_output == *output)
    }

    fn is_unavailable(&self, value_id: PanelValueId) -> bool {
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

            for overlay in indicator
                .overlay_channels()
                .iter()
                .filter(|overlay| overlay.source == layer.presentation_output.channel_id)
            {
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

#[derive(Clone, Copy)]
pub(super) struct IndicatorView<'a> {
    pub(super) series: &'a [KlineSeries],
    pub(super) composition: &'a ChartComposition,
    pub(super) host: &'a data::chart::kline::IndicatorHost,
    pub(super) instances: &'a data::chart::kline::IndicatorInstanceStore,
    pub(super) presentation: &'a IndicatorPresentationContext,
}

impl<'a> IndicatorView<'a> {
    pub(super) fn indicator_is_unavailable(&self, value_id: Option<PanelValueId>) -> bool {
        value_id.is_some_and(|id| self.presentation.is_unavailable(id))
    }

    pub(super) fn panel_output_layers(&self, panel_index: usize) -> Vec<ResolvedIndicatorLayer> {
        let Some(panel) = self.composition.panels.get(panel_index) else {
            return Vec::new();
        };
        panel
            .layers
            .iter()
            .filter(|layer| {
                !matches!(panel.presentation, PanelPresentation::Table)
                    || layer.purpose == LayerPurpose::TableBinding
            })
            .filter_map(|layer| self.resolved_indicator_layer_for_composition_layer(layer))
            .collect()
    }

    /// Resolve the indicator represented by an auxiliary panel from its
    /// concrete output layer bindings.
    pub(super) fn indicator_panel_value_id(&self, panel_index: usize) -> Option<PanelValueId> {
        self.panel_output_layers(panel_index)
            .first()
            .map(|layer| layer.base_output.instance_id)
    }

    pub(super) fn panel_indicator_value_ids(&self, panel_index: usize) -> Vec<PanelValueId> {
        let mut value_ids = Vec::new();
        for value_id in self
            .panel_output_layers(panel_index)
            .into_iter()
            .map(|layer| layer.base_output.instance_id)
        {
            if !value_ids.contains(&value_id) {
                value_ids.push(value_id);
            }
        }
        value_ids
    }

    pub(super) fn panel_indicator_visible(
        &self,
        panel_index: usize,
        value_id: PanelValueId,
    ) -> bool {
        self.composition
            .panels
            .get(panel_index)
            .is_some_and(|panel| panel.indicator_visible(value_id))
    }

    fn output_ref_for_composition_layer(
        &self,
        layer: &composition::LayerSpec,
    ) -> Option<IndicatorOutputRef> {
        self.resolved_indicator_layer_for_composition_layer(layer)
            .map(|layer| layer.presentation_output)
    }

    pub(super) fn overlay_channels_for_panel_value(
        &self,
        value_id: Option<PanelValueId>,
    ) -> Vec<OverlayChannelSpec<'_>> {
        let Some(value_id) = value_id else {
            return Vec::new();
        };

        let mut channels = Vec::new();
        if let Some(primary_panel_id) = self.composition.primary_panel_id()
            && let Some(primary_panel) = self.composition.panel(primary_panel_id)
        {
            for layer in &primary_panel.layers {
                let Some(output) = layer.source.indicator_output() else {
                    continue;
                };
                if output.instance_id != value_id {
                    continue;
                }
                let Some(output) = self.output_ref_for_composition_layer(layer) else {
                    continue;
                };
                if self
                    .indicator_output_spec(&output)
                    .is_some_and(|spec| spec.readout == indicator_engine::ReadoutBehavior::Hidden)
                {
                    continue;
                }
                channels.extend(self.overlay_channels_for_layer(Some(&output)));
            }
        }

        channels
    }

    pub(super) fn primary_overlay_entries(&self) -> Vec<(PanelValueId, bool)> {
        let Some(primary_panel_id) = self.composition.primary_panel_id() else {
            return Vec::new();
        };

        let Some(primary_panel) = self.composition.panel(primary_panel_id) else {
            return Vec::new();
        };

        let mut entries = Vec::new();
        for layer in &primary_panel.layers {
            let Some(output) = layer.source.indicator_output() else {
                continue;
            };
            if let Some((_, visible)) = entries
                .iter_mut()
                .find(|(instance_id, _)| *instance_id == output.instance_id)
            {
                *visible |= layer.visible;
            } else {
                entries.push((output.instance_id, layer.visible));
            }
        }
        entries
    }

    fn indicator_source_index_for_scene_at_channel(
        &self,
        scene: &Scene,
        series_index: usize,
        value_id: PanelValueId,
        channel_id: &str,
        fallback_bar_index: usize,
    ) -> Option<usize> {
        scene
            .interaction
            .cursor
            .and_then(|cursor| {
                self.indicator_source_index_at_or_before_unit_for_channel(
                    scene,
                    series_index,
                    cursor.x_unit,
                    value_id,
                    channel_id,
                )
            })
            .or_else(|| {
                self.indicator_output_spec_for_channel(value_id, channel_id)
                    .and_then(|spec| offset_source_index(fallback_bar_index, spec.offset_bars))
            })
    }

    pub(super) fn indicator_source_value_for_scene_at(
        &self,
        scene: &Scene,
        series_index: usize,
        series: &KlineSeries,
        value_id: PanelValueId,
        fallback_bar_index: usize,
        channel_id: &str,
    ) -> Option<f64> {
        let source_index = self.indicator_source_index_for_scene_at_channel(
            scene,
            series_index,
            value_id,
            channel_id,
            fallback_bar_index,
        )?;
        let required_input = self
            .presentation
            .presentation(value_id)
            .and_then(|presentation| {
                presentation
                    .resolved_layers
                    .iter()
                    .find(|layer| {
                        layer.presentation_output.channel_id == channel_id
                            || layer.base_output.channel_id == channel_id
                    })
                    .and_then(|layer| layer.required_input)
                    .or_else(|| {
                        presentation
                            .effective_mark
                            .and_then(|mark| self.indicator_kind(value_id)?.mark_spec(mark))
                            .and_then(|spec| spec.required_input)
                    })
            });
        if !self.indicator_input_available_at(series, source_index, required_input) {
            return None;
        }
        self.indicator_raw_output_value_for_series_at_channel(
            series,
            value_id,
            channel_id,
            source_index,
        )
        .and_then(OutputValue::as_number)
    }

    /// Resolve the calculation bar that owns a channel at a chart coordinate.
    /// This is the inverse of `for_each_indicator_output_index_with_units` and
    /// is intentionally offset-aware for crosshair and legend readouts.
    fn indicator_source_index_at_or_before_unit_for_channel(
        &self,
        scene: &Scene,
        series_index: usize,
        target_unit: i64,
        value_id: PanelValueId,
        channel_id: &str,
    ) -> Option<usize> {
        let offset = self
            .indicator_output_spec_for_channel(value_id, channel_id)
            .map(|spec| i64::from(spec.offset_bars))
            .unwrap_or_default();
        let source_unit = scene.x.axis.source_unit(target_unit, offset);
        self.series
            .get(series_index)?
            .bar_index_at_or_before_unit(scene.x.axis, source_unit)
    }

    pub(super) fn indicator_source_value_for_latest_bar_with_offset(
        &self,
        value_id: PanelValueId,
        channel_id: &str,
        offset_bars: i32,
    ) -> Option<f64> {
        let series = self.series.first()?;
        let bar_index = series.point_count().checked_sub(1)?;
        self.indicator_source_value_for_series_at_with_offset(
            series,
            value_id,
            bar_index,
            channel_id,
            offset_bars,
        )
    }

    pub(super) fn for_each_indicator_output_index_with_units(
        &self,
        bar_units: &[BarUnitWindow],
        x_axis: XAxis,
        min_x_unit: i64,
        max_x_unit: i64,
        output: &IndicatorOutputRef,
        f: impl FnMut(&KlineSeries, usize, i64),
    ) {
        self.for_each_indicator_output_index_with_units_for_channel(
            bar_units,
            x_axis,
            min_x_unit,
            max_x_unit,
            output.instance_id,
            &output.channel_id,
            f,
        );
    }

    fn for_each_indicator_output_index_with_units_for_channel(
        &self,
        bar_units: &[BarUnitWindow],
        x_axis: XAxis,
        min_x_unit: i64,
        max_x_unit: i64,
        value_id: PanelValueId,
        channel_id: &str,
        f: impl FnMut(&KlineSeries, usize, i64),
    ) {
        let offset = self
            .indicator_output_spec_for_channel(value_id, channel_id)
            .map(|spec| i64::from(spec.offset_bars))
            .unwrap_or_default();
        data::chart::kline::series::KlineWindows {
            series: self.series,
            windows: bar_units,
            axis: x_axis,
        }
        .for_each_output(min_x_unit, max_x_unit, offset, f);
    }

    pub(super) fn for_each_scene_indicator_output_index(
        &self,
        scene: &Scene,
        output: &IndicatorOutputRef,
        f: impl FnMut(&KlineSeries, usize, i64),
    ) {
        self.for_each_indicator_output_index_with_units(
            &scene.bar_units,
            scene.x.axis,
            scene.x.min_unit,
            scene.x.max_unit,
            output,
            f,
        );
    }

    pub(super) fn for_each_scene_indicator_output_index_for_channel(
        &self,
        scene: &Scene,
        value_id: PanelValueId,
        channel_id: &str,
        f: impl FnMut(&KlineSeries, usize, i64),
    ) {
        self.for_each_indicator_output_index_with_units_for_channel(
            &scene.bar_units,
            scene.x.axis,
            scene.x.min_unit,
            scene.x.max_unit,
            value_id,
            channel_id,
            f,
        );
    }

    pub(super) fn for_each_scene_indicator_line_output_index_for_channel(
        &self,
        scene: &Scene,
        value_id: PanelValueId,
        channel_id: &str,
        traversal: IndicatorLineTraversal,
        mut f: impl FnMut(&KlineSeries, usize, i64) -> IndicatorLinePointStatus,
    ) {
        let Some(series) = self.series.first() else {
            return;
        };
        let Some(units) = scene.bar_units.first() else {
            return;
        };
        let offset = self
            .indicator_output_spec_for_channel(value_id, channel_id)
            .map(|spec| i64::from(spec.offset_bars))
            .unwrap_or_default();
        let end = series
            .point_count()
            .saturating_sub(units.start_index())
            .min(units.values().len());
        let range = units.output_range(
            scene.x.axis,
            scene.x.min_unit,
            scene.x.max_unit,
            offset,
            end,
        );
        if range.is_empty() {
            return;
        }
        let (start, end) = match traversal {
            IndicatorLineTraversal::VisibleWithNeighbors => (
                range.start.saturating_sub(1),
                range.end.saturating_add(1).min(end),
            ),
            IndicatorLineTraversal::SegmentWithNeighbors => (range.start, range.end),
        };

        if traversal != IndicatorLineTraversal::SegmentWithNeighbors {
            for local_index in start..end {
                let Some(source_index) = units.global_index(local_index) else {
                    continue;
                };
                let output_unit = scene
                    .x
                    .axis
                    .output_unit(units.values()[local_index], offset);
                let _ = f(series, source_index, output_unit);
            }
            return;
        }

        let Some(visible_start) = units.global_index(start) else {
            return;
        };
        let visible_end = units
            .global_index(end)
            .unwrap_or_else(|| series.point_count());

        for source_index in (0..visible_start).rev() {
            let Some(unit) = series.bar_unit_at_index(scene.x.axis, source_index) else {
                break;
            };
            match f(series, source_index, scene.x.axis.output_unit(unit, offset)) {
                IndicatorLinePointStatus::Rendered | IndicatorLinePointStatus::Break => break,
                IndicatorLinePointStatus::Skipped => {}
            }
        }

        for local_index in start..end {
            let Some(source_index) = units.global_index(local_index) else {
                continue;
            };
            let output_unit = scene
                .x
                .axis
                .output_unit(units.values()[local_index], offset);
            let _ = f(series, source_index, output_unit);
        }

        for source_index in visible_end..series.point_count() {
            let Some(unit) = series.bar_unit_at_index(scene.x.axis, source_index) else {
                break;
            };
            match f(series, source_index, scene.x.axis.output_unit(unit, offset)) {
                IndicatorLinePointStatus::Rendered | IndicatorLinePointStatus::Break => break,
                IndicatorLinePointStatus::Skipped => {}
            }
        }
    }

    fn indicator_instance(&self, id: PanelValueId) -> Option<&'a IndicatorInstance> {
        self.instances.get(id)
    }

    pub(super) fn indicator_kind(&self, id: PanelValueId) -> Option<&'a IndicatorId> {
        self.indicator_instance(id).map(|instance| &instance.kind)
    }

    pub(super) fn indicator_segment_spec(
        &self,
        value_id: PanelValueId,
        segment_id: &str,
    ) -> Option<data::chart::kline::IndicatorSegmentSpec> {
        self.indicator_kind(value_id)
            .and_then(|kind| {
                kind.segment_specs()
                    .iter()
                    .find(|spec| spec.id == segment_id)
            })
            .cloned()
    }

    fn indicator_output_value_for_series_at(
        &self,
        series: &KlineSeries,
        output: &IndicatorOutputRef,
        bar_index: usize,
    ) -> Option<OutputValue> {
        let source_index = self
            .indicator_output_spec(output)
            .and_then(|spec| offset_source_index(bar_index, spec.offset_bars))?;
        self.indicator_raw_output_value_for_series_at(series, output, source_index)
    }

    pub(super) fn indicator_output_value_for_series_at_channel(
        &self,
        series: &KlineSeries,
        value_id: PanelValueId,
        channel_id: &str,
        bar_index: usize,
    ) -> Option<OutputValue> {
        let source_index = self
            .indicator_output_spec_for_channel(value_id, channel_id)
            .and_then(|spec| offset_source_index(bar_index, spec.offset_bars))?;
        self.indicator_raw_output_value_for_series_at_channel(
            series,
            value_id,
            channel_id,
            source_index,
        )
    }

    /// Read a value at its calculation index without applying the output's
    /// presentation offset.  Keeping this separate from the bar-facing
    /// lookup prevents an output offset from being applied twice when a
    /// channel is read at its calculation index.
    fn indicator_raw_output_value_for_series_at(
        &self,
        series: &KlineSeries,
        output: &IndicatorOutputRef,
        source_index: usize,
    ) -> Option<OutputValue> {
        self.indicator_raw_output_value_for_series_at_channel(
            series,
            output.instance_id,
            &output.channel_id,
            source_index,
        )
    }

    pub(super) fn indicator_raw_output_value_for_series_at_channel(
        &self,
        series: &KlineSeries,
        value_id: PanelValueId,
        channel_id: &str,
        source_index: usize,
    ) -> Option<OutputValue> {
        if self.presentation.is_unavailable(value_id) {
            return None;
        }
        let source_index = series.indicator_source_index_for_visual(source_index)?;
        self.host
            .output_value_for(value_id, channel_id, source_index)
    }

    fn indicator_raw_numeric_output_value_for_series_at(
        &self,
        series: &KlineSeries,
        output: &IndicatorOutputRef,
        source_index: usize,
    ) -> Option<f64> {
        self.indicator_raw_output_value_for_series_at(series, output, source_index)
            .and_then(OutputValue::as_number)
    }

    pub(super) fn indicator_ohlc_output_for_series_at(
        &self,
        series: &KlineSeries,
        output: &IndicatorOutputRef,
        visual_index: usize,
    ) -> Option<indicator_engine::OhlcOutput> {
        if self.presentation.is_unavailable(output.instance_id) {
            return None;
        }
        let source_index = series.indicator_source_index_for_visual(visual_index)?;
        self.host
            .ohlc_output_for(output.instance_id, &output.channel_id, source_index)
    }

    pub(super) fn indicator_range_output_for_series_at(
        &self,
        series: &KlineSeries,
        output: &IndicatorOutputRef,
        visual_index: usize,
    ) -> Option<indicator_engine::RangeOutput> {
        if self.presentation.is_unavailable(output.instance_id) {
            return None;
        }
        let source_index = series.indicator_source_index_for_visual(visual_index)?;
        self.host
            .range_output_for(output.instance_id, &output.channel_id, source_index)
    }

    pub(super) fn indicator_markers_for_series_at(
        &self,
        series: &KlineSeries,
        value_id: PanelValueId,
        visual_index: usize,
    ) -> Vec<indicator_engine::MarkerOutput> {
        if self.presentation.is_unavailable(value_id) {
            return Vec::new();
        }
        let Some(source_index) = series.indicator_source_index_for_visual(visual_index) else {
            return Vec::new();
        };
        self.host.marker_outputs_for(value_id, source_index)
    }

    pub(super) fn indicator_segments(
        &self,
        value_id: PanelValueId,
        source_range: Range<usize>,
        scene: &Scene,
    ) -> Vec<indicator_engine::SegmentOutput> {
        if self.presentation.is_unavailable(value_id) {
            return Vec::new();
        }
        let Some(series) = self.series.first() else {
            return Vec::new();
        };
        self.host
            .segment_outputs_for(value_id, source_range, |start, end| {
                let Some(start) = series.bar_unit_at_index(scene.x.axis, start) else {
                    return false;
                };
                let Some(end) = series.bar_unit_at_index(scene.x.axis, end) else {
                    return false;
                };
                start.min(end) <= scene.x.max_unit && start.max(end) >= scene.x.min_unit
            })
    }

    pub(super) fn indicator_levels(
        &self,
        value_id: PanelValueId,
    ) -> Vec<indicator_engine::HorizontalLevelOutput> {
        if self.presentation.is_unavailable(value_id) {
            return Vec::new();
        }
        self.host.level_outputs_for(value_id)
    }

    pub(super) fn indicator_regions(
        &self,
        value_id: PanelValueId,
        source_range: Range<usize>,
    ) -> Vec<indicator_engine::RegionOutput> {
        if self.presentation.is_unavailable(value_id) {
            return Vec::new();
        }
        self.host
            .region_outputs_for_range(value_id, Some(source_range))
    }

    pub(super) fn indicator_layer_values_at(
        &self,
        series: &KlineSeries,
        layer: &ResolvedIndicatorLayer,
        bar_index: usize,
    ) -> Option<IndicatorBarValues> {
        if self
            .presentation
            .is_unavailable(layer.base_output.instance_id)
            || !self.indicator_input_available_at(series, bar_index, layer.required_input)
        {
            return None;
        }

        let base = self
            .indicator_output_value_for_series_at(series, &layer.base_output, bar_index)
            .and_then(OutputValue::as_number)?;
        let presentation = if matches!(
            layer.mark,
            MarkKind::Bar(BarMode::Histogram(HistogramMode::SignedOverlay))
        ) {
            self.indicator_output_value_for_series_at(series, &layer.presentation_output, bar_index)
                .and_then(OutputValue::as_number)
        } else {
            None
        };
        resolve_indicator_bar_values(layer.mark, Some(base), presentation)
    }

    fn indicator_layer_values_for_source_at(
        &self,
        series: &KlineSeries,
        layer: &ResolvedIndicatorLayer,
        source_index: usize,
    ) -> Option<IndicatorBarValues> {
        if self
            .presentation
            .is_unavailable(layer.base_output.instance_id)
            || !self.indicator_input_available_at(series, source_index, layer.required_input)
        {
            return None;
        }

        let base = self.indicator_raw_numeric_output_value_for_series_at(
            series,
            &layer.base_output,
            source_index,
        )?;
        let presentation = if matches!(
            layer.mark,
            MarkKind::Bar(BarMode::Histogram(HistogramMode::SignedOverlay))
        ) {
            self.indicator_raw_numeric_output_value_for_series_at(
                series,
                &layer.presentation_output,
                source_index,
            )
        } else {
            None
        };
        resolve_indicator_bar_values(layer.mark, Some(base), presentation)
    }

    pub(super) fn indicator_input_available_at(
        &self,
        series: &KlineSeries,
        visual_index: usize,
        required_input: Option<IndicatorInputSource>,
    ) -> bool {
        let Some(required_input) = required_input else {
            return true;
        };
        match self.presentation.input_coverage.state(required_input) {
            IndicatorInputCoverageState::Unknown => true,
            IndicatorInputCoverageState::Missing => false,
            IndicatorInputCoverageState::Available => {
                let Some(runtime_index) = series.indicator_source_index_for_visual(visual_index)
                else {
                    return false;
                };
                self.presentation
                    .input_coverage
                    .at(required_input, runtime_index)
            }
        }
    }

    fn indicator_source_value_for_series_at_with_offset(
        &self,
        series: &KlineSeries,
        value_id: PanelValueId,
        bar_index: usize,
        channel_id: &str,
        offset_bars: i32,
    ) -> Option<f64> {
        let effective_offset = if offset_bars == 0 {
            self.indicator_output_spec_for_channel(value_id, channel_id)
                .map(|spec| spec.offset_bars)
                .unwrap_or_default()
        } else {
            offset_bars
        };
        let source_index = offset_source_index(bar_index, effective_offset)?;
        self.indicator_raw_output_value_for_series_at_channel(
            series,
            value_id,
            channel_id,
            source_index,
        )
        .and_then(OutputValue::as_number)
    }

    pub(super) fn indicator_output_spec(
        &self,
        output: &IndicatorOutputRef,
    ) -> Option<&'static OutputChannelSpec> {
        self.indicator_output_spec_for_channel(output.instance_id, &output.channel_id)
    }

    pub(super) fn indicator_output_spec_for_channel(
        &self,
        value_id: PanelValueId,
        channel_id: &str,
    ) -> Option<&'static OutputChannelSpec> {
        self.indicator_kind(value_id).and_then(|kind| {
            kind.output_channels()
                .iter()
                .find(|spec| spec.id == channel_id)
        })
    }

    pub(super) fn indicator_default_style_slot(&self, value_id: PanelValueId) -> &'static str {
        self.indicator_kind(value_id)
            .and_then(|kind| kind.color_specs().first().map(|spec| spec.id.as_str()))
            .unwrap_or("main")
    }

    pub(super) fn indicator_style_slot_for_output(
        &self,
        output: &IndicatorOutputRef,
    ) -> &'static str {
        self.indicator_output_spec(output)
            .and_then(|spec| spec.style_slot.as_deref())
            .unwrap_or_else(|| self.indicator_default_style_slot(output.instance_id))
    }

    pub(super) fn indicator_fill_visible(&self, value_id: PanelValueId) -> bool {
        self.indicator_instance(value_id)
            .map(|instance| instance.style_overrides.fill_visible)
            .unwrap_or(true)
    }

    pub(super) fn resolved_indicator_layer_for_composition_layer(
        &self,
        layer: &composition::LayerSpec,
    ) -> Option<ResolvedIndicatorLayer> {
        let source_output = layer.source.indicator_output()?;
        if let Some(resolved) = self.presentation.layer_for_output(source_output) {
            return Some(resolved.clone());
        }

        let base_output = source_output.clone();
        let kind = self.indicator_kind(base_output.instance_id)?;
        let mark = resolve_mark_for_data_kind(layer.effective_mark(), layer.data_kind);
        let channel_id = kind
            .mark_source_channel(mark)
            .unwrap_or(base_output.channel_id.as_str());
        let instance_id = base_output.instance_id;
        let presentation_output = IndicatorOutputRef::new(instance_id, channel_id);
        Some(ResolvedIndicatorLayer {
            base_output,
            presentation_output,
            data_kind: layer.data_kind,
            purpose: layer.purpose,
            mark,
            required_input: kind.mark_spec(mark).and_then(|spec| spec.required_input),
            visible: layer.visible,
        })
    }

    pub(super) fn overlay_channels_for_layer(
        &self,
        output: Option<&IndicatorOutputRef>,
    ) -> Option<OverlayChannelSpec<'static>> {
        let output = output?;
        let spec = self.indicator_output_spec(output)?;
        if spec.value_type != OutputValueType::Number {
            return None;
        }

        if let Some(kind) = self.indicator_kind(output.instance_id)
            && let Some(channel) = kind
                .overlay_channels()
                .iter()
                .find(|channel| channel.source == output.channel_id)
        {
            return Some(OverlayChannelSpec {
                label: &channel.label,
                source: &channel.source,
                line_width: channel.line_width,
                color_role: channel.color_role,
                color_slot: &channel.color_slot,
                style_by: channel.style_by.as_deref(),
            });
        }

        let color_slot = self.indicator_style_slot_for_output(output);
        Some(OverlayChannelSpec {
            label: &spec.label,
            source: &spec.id,
            line_width: spec.line_width,
            color_role: ThemeColorRole::Neutral,
            color_slot,
            style_by: spec.style_by.as_deref(),
        })
    }

    pub(super) fn fill_specs_for_panel_value(
        &self,
        value_id: Option<PanelValueId>,
    ) -> &'static [data::chart::kline::IndicatorFillSpec] {
        value_id
            .and_then(|id| self.indicator_kind(id))
            .map(|kind| kind.fill_specs())
            .unwrap_or_default()
    }

    fn indicator_line_width(
        &self,
        value_id: Option<PanelValueId>,
        channel: &OverlayChannelSpec<'_>,
    ) -> f32 {
        value_id
            .and_then(|id| {
                self.indicator_instance(id).map(|instance| {
                    instance
                        .style_overrides
                        .line_width(channel.color_slot)
                        .map(f32::from)
                        .unwrap_or(channel.line_width)
                })
            })
            .unwrap_or(channel.line_width)
    }

    pub(super) fn indicator_line_width_for_output(
        &self,
        value_id: Option<PanelValueId>,
        output: Option<&IndicatorOutputRef>,
        channel: &OverlayChannelSpec<'_>,
    ) -> f32 {
        let Some(output) = output else {
            return self.indicator_line_width(value_id, channel);
        };
        let Some(value_id) = value_id else {
            return channel.line_width;
        };
        let Some(instance) = self.indicator_instance(value_id) else {
            return channel.line_width;
        };
        let style_slot = self
            .indicator_kind(value_id)
            .and_then(|kind| {
                kind.output_channels()
                    .iter()
                    .find(|spec| spec.id == output.channel_id)
            })
            .and_then(|spec| spec.style_slot.as_deref());
        style_slot
            .and_then(|slot| instance.style_overrides.line_width(slot))
            .map(f32::from)
            .unwrap_or_else(|| self.indicator_line_width(Some(value_id), channel))
    }

    pub(super) fn indicator_line_width_for_style_slot(
        &self,
        value_id: PanelValueId,
        style_slot: &str,
        fallback: f32,
    ) -> f32 {
        self.indicator_instance(value_id)
            .and_then(|instance| instance.style_overrides.line_width(style_slot))
            .map(f32::from)
            .unwrap_or(fallback)
    }

    fn indicator_stroke_style(
        &self,
        value_id: Option<PanelValueId>,
        channel: &OverlayChannelSpec<'_>,
    ) -> LineStrokeStyle {
        value_id
            .and_then(|id| {
                self.indicator_instance(id).map(|instance| {
                    instance
                        .style_overrides
                        .stroke_style(channel.color_slot)
                        .unwrap_or_default()
                })
            })
            .unwrap_or_default()
    }

    pub(super) fn indicator_stroke_style_for_output(
        &self,
        value_id: Option<PanelValueId>,
        output: Option<&IndicatorOutputRef>,
        channel: &OverlayChannelSpec<'_>,
    ) -> LineStrokeStyle {
        let Some(output) = output else {
            return self.indicator_stroke_style(value_id, channel);
        };
        let Some(value_id) = value_id else {
            return LineStrokeStyle::Solid;
        };
        let Some(instance) = self.indicator_instance(value_id) else {
            return LineStrokeStyle::Solid;
        };
        let style_slot = self
            .indicator_kind(value_id)
            .and_then(|kind| {
                kind.output_channels()
                    .iter()
                    .find(|spec| spec.id == output.channel_id)
            })
            .and_then(|spec| spec.style_slot.as_deref());
        style_slot
            .and_then(|slot| instance.style_overrides.stroke_style(slot))
            .unwrap_or_else(|| self.indicator_stroke_style(Some(value_id), channel))
    }

    pub(super) fn indicator_stroke_style_for_style_slot(
        &self,
        value_id: PanelValueId,
        style_slot: &str,
    ) -> LineStrokeStyle {
        self.indicator_instance(value_id)
            .and_then(|instance| instance.style_overrides.stroke_style(style_slot))
            .unwrap_or_default()
    }

    pub(super) fn overlay_channel_color(
        &self,
        value_id: Option<PanelValueId>,
        channel: &OverlayChannelSpec<'_>,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) -> iced::Color {
        self.indicator_value_color(
            value_id,
            channel.color_slot,
            channel.color_role,
            palette,
            generated_accents,
        )
    }

    pub(super) fn overlay_channel_color_for_output(
        &self,
        value_id: Option<PanelValueId>,
        output: Option<&IndicatorOutputRef>,
        channel: &OverlayChannelSpec<'_>,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) -> iced::Color {
        let style_by = channel.style_by.or_else(|| {
            output.and_then(|output| {
                self.indicator_output_spec(output)
                    .and_then(|spec| spec.style_by.as_deref())
            })
        });
        if style_by.is_some()
            && value_id.is_some()
            && let (Some(series), Some(output)) = (self.series.first(), output)
            && let Some(source_index) = series.point_count().checked_sub(1)
        {
            return self.overlay_channel_color_for_output_at(
                series,
                source_index,
                value_id,
                Some(output),
                channel,
                palette,
                generated_accents,
            );
        }
        let Some(output) = output else {
            return self.overlay_channel_color(value_id, channel, palette, generated_accents);
        };
        let style_slot = value_id
            .and_then(|id| self.indicator_kind(id))
            .and_then(|kind| {
                kind.output_channels()
                    .iter()
                    .find(|spec| spec.id == output.channel_id)
            })
            .and_then(|spec| spec.style_slot.as_deref());
        if let Some(color) = style_slot.and_then(|slot| {
            value_id
                .and_then(|id| self.indicator_instance(id))
                .and_then(|instance| instance.style_overrides.color(slot))
        }) {
            return crate::widget::resolve_theme_color(color, palette, generated_accents);
        }
        self.overlay_channel_color(value_id, channel, palette, generated_accents)
    }

    /// Resolve a channel's style slot for one calculation bar. `style_by`
    /// points at a boolean output; the source-declared true or false slot is
    /// selected without adding renderer-specific branches.
    pub(super) fn overlay_channel_color_for_output_at(
        &self,
        series: &KlineSeries,
        source_index: usize,
        value_id: Option<PanelValueId>,
        output: Option<&IndicatorOutputRef>,
        channel: &OverlayChannelSpec<'_>,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) -> iced::Color {
        let Some(value_id) = value_id else {
            return self.overlay_channel_color_for_output(
                value_id,
                output,
                channel,
                palette,
                generated_accents,
            );
        };
        let Some(output) = output else {
            return self.overlay_channel_color_for_output(
                Some(value_id),
                output,
                channel,
                palette,
                generated_accents,
            );
        };
        self.overlay_channel_color_for_channel_at(
            series,
            source_index,
            value_id,
            output.instance_id,
            &output.channel_id,
            channel,
            palette,
            generated_accents,
        )
    }

    pub(super) fn overlay_channel_color_for_channel_at(
        &self,
        series: &KlineSeries,
        source_index: usize,
        value_id: PanelValueId,
        output_instance_id: PanelValueId,
        output_channel: &str,
        channel: &OverlayChannelSpec<'_>,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) -> iced::Color {
        let runtime_index = series.indicator_source_index_for_visual(source_index);
        if !self.presentation.is_unavailable(output_instance_id)
            && let Some(color) = runtime_index.and_then(|source_index| {
                self.host
                    .output_color_for(output_instance_id, output_channel, source_index)
            })
        {
            return super::painting::resolve_indicator_color(color);
        }
        let style_by = channel.style_by.or_else(|| {
            self.indicator_output_spec_for_channel(output_instance_id, output_channel)
                .and_then(|spec| spec.style_by.as_deref())
        });
        let Some(style_by) = style_by else {
            return self.overlay_channel_color(Some(value_id), channel, palette, generated_accents);
        };
        let is_true = self
            .indicator_raw_output_value_for_series_at_channel(
                series,
                value_id,
                style_by,
                source_index,
            )
            .and_then(OutputValue::as_boolean);
        let style_slot =
            self.style_slot_for_output(value_id, output_channel, is_true, channel.color_slot);
        self.indicator_value_color(
            Some(value_id),
            style_slot,
            channel.color_role,
            palette,
            generated_accents,
        )
    }

    /// Resolve a boolean style binding from source-declared mark slots.
    pub(super) fn style_slot_for_output<'slot>(
        &self,
        value_id: PanelValueId,
        output_channel: &str,
        state: Option<bool>,
        fallback: &'slot str,
    ) -> &'slot str {
        let kind = self.indicator_kind(value_id);
        let output_slot = kind.and_then(|kind| {
            kind.output_channels()
                .iter()
                .find(|spec| spec.id == output_channel)
                .and_then(|spec| spec.style_slot.as_deref())
        });
        let Some(state) = state else {
            return output_slot.unwrap_or(fallback);
        };

        if let Some(mark) = kind.and_then(|kind| {
            kind.mark_specs()
                .iter()
                .find(|mark| mark.source_channel == output_channel)
        }) {
            return if state {
                mark.true_color_slot
                    .as_deref()
                    .unwrap_or(mark.color_slot.as_str())
            } else {
                mark.false_color_slot
                    .as_deref()
                    .unwrap_or(mark.color_slot.as_str())
            };
        }

        output_slot.unwrap_or(fallback)
    }

    fn style_slot_for_mark<'slot>(
        &self,
        value_id: PanelValueId,
        output_channel: &str,
        mark: MarkKind,
        state: Option<bool>,
        fallback: &'slot str,
    ) -> &'slot str {
        let Some(kind) = self.indicator_kind(value_id) else {
            return fallback;
        };
        let Some(spec) = kind
            .mark_specs()
            .iter()
            .find(|spec| spec.mark == mark && spec.source_channel == output_channel)
        else {
            return self.style_slot_for_output(value_id, output_channel, state, fallback);
        };
        match state {
            Some(true) => spec
                .true_color_slot
                .as_deref()
                .unwrap_or(spec.color_slot.as_str()),
            Some(false) => spec
                .false_color_slot
                .as_deref()
                .unwrap_or(spec.color_slot.as_str()),
            None => spec.color_slot.as_str(),
        }
    }

    pub(super) fn indicator_highlight_style_for_mark_at(
        &self,
        series: &KlineSeries,
        source_index: usize,
        output: &IndicatorOutputRef,
        mark: MarkKind,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) -> Option<(iced::Color, f32, LineStrokeStyle)> {
        let spec = self
            .indicator_kind(output.instance_id)?
            .mark_specs()
            .iter()
            .find(|spec| spec.mark == mark && spec.source_channel == output.channel_id)?;
        let highlight_by = spec.highlight_by.as_deref()?;
        let highlight_color_slot = spec.highlight_color_slot.as_deref()?;
        let is_highlighted = self
            .indicator_raw_output_value_for_series_at_channel(
                series,
                output.instance_id,
                highlight_by,
                source_index,
            )
            .and_then(OutputValue::as_boolean)?;
        is_highlighted.then(|| {
            let color = self.indicator_value_color(
                Some(output.instance_id),
                highlight_color_slot,
                ThemeColorRole::Neutral,
                palette,
                generated_accents,
            );
            let line_width = self.indicator_line_width_for_style_slot(
                output.instance_id,
                highlight_color_slot,
                1.0,
            );
            let stroke_style = self
                .indicator_stroke_style_for_style_slot(output.instance_id, highlight_color_slot);
            (color, line_width, stroke_style)
        })
    }

    pub(super) fn indicator_color_for_output_at(
        &self,
        series: &KlineSeries,
        source_index: usize,
        output: &IndicatorOutputRef,
        mark: Option<MarkKind>,
        fallback_slot: &str,
        color_role: ThemeColorRole,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) -> iced::Color {
        let state = self
            .indicator_output_spec(output)
            .and_then(|spec| spec.style_by.as_deref())
            .and_then(|style_by| {
                self.indicator_raw_output_value_for_series_at_channel(
                    series,
                    output.instance_id,
                    style_by,
                    source_index,
                )
            })
            .and_then(OutputValue::as_boolean);
        let style_slot = mark.map_or_else(
            || {
                self.style_slot_for_output(
                    output.instance_id,
                    output.channel_id.as_str(),
                    state,
                    fallback_slot,
                )
            },
            |mark| {
                self.style_slot_for_mark(
                    output.instance_id,
                    output.channel_id.as_str(),
                    mark,
                    state,
                    fallback_slot,
                )
            },
        );
        self.indicator_value_color(
            Some(output.instance_id),
            style_slot,
            color_role,
            palette,
            generated_accents,
        )
    }

    pub(super) fn indicator_value_color(
        &self,
        value_id: Option<PanelValueId>,
        color_slot: &str,
        color_role: ThemeColorRole,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) -> iced::Color {
        let configured = value_id
            .and_then(|id| self.indicator_instance(id))
            .and_then(|instance| instance.configured_color(color_slot));
        let color =
            configured.unwrap_or_else(|| ThemeColor::new(color_role, ThemeColorShade::Base));
        crate::widget::resolve_theme_color(color, palette, generated_accents).scale_alpha(
            match color_role {
                ThemeColorRole::Neutral => 0.72,
                _ => 0.78,
            },
        )
    }

    pub(super) fn indicator_color(
        &self,
        value_id: Option<PanelValueId>,
        slot: &str,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) -> iced::Color {
        let configured = value_id
            .and_then(|id| self.indicator_instance(id))
            .and_then(|instance| instance.configured_color(slot))
            .unwrap_or_else(|| ThemeColor::new(ThemeColorRole::Neutral, ThemeColorShade::Base));
        crate::widget::resolve_theme_color(configured, palette, generated_accents)
    }

    fn color_role_for_style_slot(&self, value_id: PanelValueId, slot: &str) -> ThemeColorRole {
        self.indicator_kind(value_id)
            .and_then(|kind| kind.color_specs().iter().find(|spec| spec.id == slot))
            .map(|spec| spec.default_role)
            .unwrap_or(ThemeColorRole::Neutral)
    }
}

fn offset_source_index(index: usize, offset_bars: i32) -> Option<usize> {
    if offset_bars >= 0 {
        index.checked_sub(offset_bars as usize)
    } else {
        index.checked_add(offset_bars.unsigned_abs() as usize)
    }
}

impl IndicatorView<'_> {
    pub(super) fn indicator_header_values_for_indicator(
        &self,
        scene: &Scene,
        panel_index: usize,
        value_id: PanelValueId,
        series_index: usize,
        series: &KlineSeries,
        bar_index: usize,
    ) -> Vec<IndicatorHeaderValue<'_>> {
        if self.indicator_is_unavailable(Some(value_id)) {
            return Vec::new();
        }

        let Some(kind) = self.indicator_kind(value_id) else {
            return Vec::new();
        };
        let presentation = self.presentation.presentation(value_id);
        let effective_mark = presentation
            .and_then(|presentation| presentation.effective_mark)
            .or_else(|| {
                self.composition
                    .panels
                    .get(panel_index)
                    .and_then(|panel| self.composition.panel_effective_mark(panel.id))
            });
        let required_input = effective_mark
            .and_then(|mark| kind.mark_spec(mark))
            .and_then(|spec| spec.required_input);

        let readouts = presentation
            .map(|presentation| presentation.active_readouts.as_slice())
            .unwrap_or_else(|| kind.readout_specs());
        if !readouts.is_empty() {
            return readouts
                .iter()
                .filter(|readout| effective_mark.is_none_or(|mark| readout.scope.matches(mark)))
                .filter_map(|readout| {
                    let channel_id = &readout.source;
                    let source_index = self.indicator_source_index_for_scene_at_channel(
                        scene,
                        series_index,
                        value_id,
                        channel_id,
                        bar_index,
                    )?;
                    if !self.indicator_input_available_at(series, source_index, required_input) {
                        return None;
                    }
                    let value = self
                        .indicator_raw_output_value_for_series_at_channel(
                            series,
                            value_id,
                            channel_id,
                            source_index,
                        )
                        .and_then(OutputValue::as_number)?;
                    Some(IndicatorHeaderValue {
                        value_id,
                        label: readout.label.as_str(),
                        value,
                        color_role: readout.color_role,
                        color_slot: readout.color_slot.as_str(),
                    })
                })
                .collect();
        }

        self.panel_output_layers(panel_index)
            .into_iter()
            .filter(|layer| layer.base_output.instance_id == value_id)
            .filter(|layer| layer.visible)
            .filter_map(|layer| {
                let output = &layer.presentation_output;
                let spec = self.indicator_output_spec(output)?;
                if spec.readout == indicator_engine::ReadoutBehavior::Hidden {
                    return None;
                }

                let source_index = self.indicator_source_index_for_scene_at_channel(
                    scene,
                    series_index,
                    output.instance_id,
                    &output.channel_id,
                    bar_index,
                )?;

                if !self.indicator_input_available_at(series, source_index, layer.required_input) {
                    return None;
                }
                let value = self.indicator_raw_numeric_output_value_for_series_at(
                    series,
                    output,
                    source_index,
                );
                let value = value?;
                let color_slot = self.indicator_style_slot_for_output(output);
                Some(IndicatorHeaderValue {
                    value_id,
                    label: spec.label.as_str(),
                    value,
                    color_role: self.color_role_for_style_slot(value_id, color_slot),
                    color_slot,
                })
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IndicatorLineTraversal {
    VisibleWithNeighbors,
    SegmentWithNeighbors,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IndicatorLinePointStatus {
    Rendered,
    Skipped,
    Break,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OverlayChannelSpec<'a> {
    pub(crate) label: &'a str,
    pub(crate) source: &'a str,
    line_width: f32,
    color_role: ThemeColorRole,
    color_slot: &'a str,
    pub(crate) style_by: Option<&'a str>,
}

impl IndicatorView<'_> {
    pub(super) fn for_each_layer_domain_value(
        &self,
        series: &KlineSeries,
        layer: &ResolvedIndicatorLayer,
        source_index: usize,
        mut emit: impl FnMut(f64),
    ) {
        let Some(spec) = self.indicator_output_spec(&layer.presentation_output) else {
            return;
        };
        match spec.value_type {
            data::chart::kline::OutputValueType::Number => {
                if matches!(layer.mark, MarkKind::Candle | MarkKind::Bar(_)) {
                    let Some(values) =
                        self.indicator_layer_values_for_source_at(series, layer, source_index)
                    else {
                        return;
                    };
                    match values {
                        IndicatorBarValues::Plain { value } => {
                            emit(value);
                        }
                        IndicatorBarValues::SignedOverlay {
                            background_value,
                            signed_value,
                        } => {
                            emit(background_value);
                            emit(signed_value.abs());
                        }
                    }
                } else if let Some(value) = self.indicator_raw_numeric_output_value_for_series_at(
                    series,
                    &layer.presentation_output,
                    source_index,
                ) {
                    emit(value);
                }
            }
            data::chart::kline::OutputValueType::Ohlc => {
                let Some(values) = self.indicator_ohlc_output_for_series_at(
                    series,
                    &layer.presentation_output,
                    source_index,
                ) else {
                    return;
                };
                for value in [values.open, values.high, values.low, values.close]
                    .into_iter()
                    .flatten()
                {
                    emit(value);
                }
            }
            data::chart::kline::OutputValueType::Range => {
                let Some(values) = self.indicator_range_output_for_series_at(
                    series,
                    &layer.presentation_output,
                    source_index,
                ) else {
                    return;
                };
                if let Some(upper) = values.upper {
                    emit(upper);
                }
                if let Some(lower) = values.lower {
                    emit(lower);
                }
            }
            data::chart::kline::OutputValueType::Boolean => {}
        }
    }

    pub(super) fn visible_primary_overlay_layers(&self) -> Vec<ResolvedIndicatorLayer> {
        let Some(primary_panel_id) = self.composition.primary_panel_id() else {
            return Vec::new();
        };
        let Some(primary_panel) = self.composition.panel(primary_panel_id) else {
            return Vec::new();
        };

        primary_panel
            .layers
            .iter()
            .filter_map(|layer| {
                let output = layer.source.indicator_output()?;
                if !layer.visible || self.indicator_is_unavailable(Some(output.instance_id)) {
                    return None;
                }
                self.resolved_indicator_layer_for_composition_layer(layer)
            })
            .collect()
    }
}
