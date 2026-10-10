use super::composition::{
    self, BarMode, HistogramMode, MarkKind, PanelId, PanelPresentation, PanelScaleMode,
    PanelValueId, PanelValueLabelPolicy, PanelValuePrecision, resolve_mark_for_data_kind,
};
use super::scene::Scene;
use super::{
    DEFAULT_MIN_PANEL_RATIO, IndicatorBarValues, KlinePanelKind, KlineSeries, KlineWidget,
    OverlayChannelSpec, ResolvedIndicatorLayer, YUnitRange, resolve_indicator_bar_values,
};
use data::chart::kline::{
    IndicatorId, IndicatorInputCoverageState, IndicatorInputSource, IndicatorInstance,
    IndicatorOutputRef, LayerPurpose, LineStrokeStyle, OutputChannelSpec, OutputValue,
    OutputValueType, SeriesColor,
};
use data::config::theme::{ACCENT_COUNT, ThemeColor, ThemeColorRole, ThemeColorShade};

use crate::window::PixelRatio;
use iced::Point;
use iced::theme::palette::Extended;
use std::ops::Range;

#[derive(Debug, Clone, Copy)]
pub(super) struct IndicatorHeaderValue<'a> {
    pub(super) value_id: PanelValueId,
    pub(super) label: &'a str,
    pub(super) value: f64,
    pub(super) color_role: ThemeColorRole,
    pub(super) color_slot: &'a str,
}

pub(super) type IndicatorOutputLayer = ResolvedIndicatorLayer;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct PixelSpan {
    pub(super) start: f32,
    pub(super) size: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct PixelGeometry {
    ratio: f32,
    origin: Point,
}

impl PixelGeometry {
    pub(super) fn new(pixel_ratio: PixelRatio, origin: Point) -> Self {
        Self::from_ratio(pixel_ratio.value(), origin)
    }

    pub(super) fn from_ratio(ratio: f32, origin: Point) -> Self {
        Self {
            ratio: if ratio.is_finite() && ratio > 0.0 {
                ratio
            } else {
                1.0
            },
            origin: Point::new(
                if origin.x.is_finite() { origin.x } else { 0.0 },
                if origin.y.is_finite() { origin.y } else { 0.0 },
            ),
        }
    }

    pub(super) fn ratio(self) -> f32 {
        self.ratio
    }

    pub(super) fn origin(self) -> Point {
        self.origin
    }

    pub(super) fn logical_width_from_physical(self, width_phys: i32) -> f32 {
        width_phys.max(1) as f32 / self.ratio
    }

    pub(super) fn quantized_stroke_width(self, width_logical: f32) -> (f32, i32) {
        let stroke_width_phys = (width_logical.max(0.0) * self.ratio).round() as i32;
        let stroke_width_phys = stroke_width_phys.max(1);
        (
            self.logical_width_from_physical(stroke_width_phys),
            stroke_width_phys,
        )
    }

    fn snap_axis_to_physical(self, value_local: f32, origin: f32) -> i32 {
        ((value_local + origin) * self.ratio).round() as i32
    }

    fn physical_to_local(self, physical: i32, origin: f32) -> f32 {
        physical as f32 / self.ratio - origin
    }

    fn snap_stroke_center(self, value_local: f32, origin: f32, stroke_width_phys: i32) -> f32 {
        let axis_phys = (value_local + origin) * self.ratio;
        let snapped_phys = if (stroke_width_phys & 1) == 0 {
            axis_phys.round()
        } else {
            (axis_phys - 0.5).round() + 0.5
        };

        (snapped_phys / self.ratio) - origin
    }

    fn snapped_span(self, start_local: f32, end_local: f32, origin: f32) -> PixelSpan {
        let start = start_local.min(end_local);
        let end = start_local.max(end_local);
        let start_phys = self.snap_axis_to_physical(start, origin);
        let end_phys = self.snap_axis_to_physical(end, origin).max(start_phys + 1);

        PixelSpan {
            start: self.physical_to_local(start_phys, origin),
            size: (end_phys - start_phys) as f32 / self.ratio,
        }
    }

    fn positive_span(self, start_local: f32, end_local: f32, origin: f32) -> Option<PixelSpan> {
        if !start_local.is_finite() || !end_local.is_finite() {
            return None;
        }

        let start = start_local.min(end_local);
        let end = start_local.max(end_local);
        (end > start).then(|| self.snapped_span(start, end, origin))
    }

    pub(super) fn snap_x_to_cell(self, x: f32) -> f32 {
        self.physical_to_local(self.snap_axis_to_physical(x, self.origin.x), self.origin.x)
    }

    pub(super) fn centered_left(self, x: f32, width_phys: i32) -> f32 {
        let center_phys = self.snap_axis_to_physical(x, self.origin.x);
        self.physical_to_local(center_phys - width_phys / 2, self.origin.x)
    }

    pub(super) fn snapped_x_span(self, start: f32, end: f32) -> PixelSpan {
        self.snapped_span(start, end, self.origin.x)
    }

    pub(super) fn snapped_y_span(self, start: f32, end: f32) -> PixelSpan {
        self.snapped_span(start, end, self.origin.y)
    }

    pub(super) fn positive_x_span(self, start: f32, end: f32) -> Option<PixelSpan> {
        self.positive_span(start, end, self.origin.x)
    }

    pub(super) fn positive_y_span(self, start: f32, end: f32) -> Option<PixelSpan> {
        self.positive_span(start, end, self.origin.y)
    }

    pub(super) fn positive_x_span_in_bounds(
        self,
        start: f32,
        end: f32,
        lower: f32,
        upper: f32,
    ) -> Option<PixelSpan> {
        if !start.is_finite()
            || !end.is_finite()
            || !lower.is_finite()
            || !upper.is_finite()
            || lower > upper
        {
            return None;
        }
        let raw_start = start.min(end);
        let raw_end = start.max(end);
        let clipped_start = raw_start.max(lower).min(upper);
        let clipped_end = raw_end.min(upper).max(lower);
        self.positive_x_span(clipped_start, clipped_end)
    }

    pub(super) fn positive_y_span_in_bounds(
        self,
        start: f32,
        end: f32,
        lower: f32,
        upper: f32,
    ) -> Option<PixelSpan> {
        if !start.is_finite()
            || !end.is_finite()
            || !lower.is_finite()
            || !upper.is_finite()
            || lower > upper
        {
            return None;
        }
        let raw_start = start.min(end);
        let raw_end = start.max(end);
        let clipped_start = raw_start.max(lower).min(upper);
        let clipped_end = raw_end.min(upper).max(lower);
        self.positive_y_span(clipped_start, clipped_end)
    }

    pub(super) fn snap_x_for_stroke(self, x: f32, stroke_width_phys: i32) -> f32 {
        let x_cell = self.snap_x_to_cell(x);
        self.snap_stroke_center(x_cell, self.origin.x, stroke_width_phys)
    }

    pub(super) fn snap_stroke_center_x(self, x: f32, stroke_width_phys: i32) -> f32 {
        self.snap_stroke_center(x, self.origin.x, stroke_width_phys)
    }

    pub(super) fn snap_stroke_center_y(self, y: f32, stroke_width_phys: i32) -> f32 {
        self.snap_stroke_center(y, self.origin.y, stroke_width_phys)
    }

    pub(super) fn snap_y_for_stroke(self, y: f32, stroke_width_phys: i32) -> f32 {
        self.snap_stroke_center(y, self.origin.y, stroke_width_phys)
    }

    pub(super) fn snap_point_for_stroke(self, point: Point, stroke_width_phys: i32) -> Point {
        Point::new(
            self.snap_stroke_center(point.x, self.origin.x, stroke_width_phys),
            self.snap_stroke_center(point.y, self.origin.y, stroke_width_phys),
        )
    }
}

impl<'a> KlineWidget<'a> {
    pub(super) fn indicator_instance(&self, id: PanelValueId) -> Option<&IndicatorInstance> {
        self.indicator_instances.get(id)
    }

    pub(super) fn indicator_kind(&self, id: PanelValueId) -> Option<&IndicatorId> {
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

    pub(super) fn indicator_output_value_for_series_at(
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
    pub(super) fn indicator_raw_output_value_for_series_at(
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
        if self.indicator_is_unavailable(Some(value_id)) {
            return None;
        }
        let source_index = series.indicator_source_index_for_visual(source_index)?;
        self.indicator_host
            .output_value_for(value_id, channel_id, source_index)
    }

    pub(super) fn indicator_raw_numeric_output_value_for_series_at(
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
        if self.indicator_is_unavailable(Some(output.instance_id)) {
            return None;
        }
        let source_index = series.indicator_source_index_for_visual(visual_index)?;
        self.indicator_host
            .ohlc_output_for(output.instance_id, &output.channel_id, source_index)
    }

    pub(super) fn indicator_range_output_for_series_at(
        &self,
        series: &KlineSeries,
        output: &IndicatorOutputRef,
        visual_index: usize,
    ) -> Option<indicator_engine::RangeOutput> {
        if self.indicator_is_unavailable(Some(output.instance_id)) {
            return None;
        }
        let source_index = series.indicator_source_index_for_visual(visual_index)?;
        self.indicator_host
            .range_output_for(output.instance_id, &output.channel_id, source_index)
    }

    pub(super) fn indicator_markers_for_series_at(
        &self,
        series: &KlineSeries,
        value_id: PanelValueId,
        visual_index: usize,
    ) -> Vec<indicator_engine::MarkerOutput> {
        if self.indicator_is_unavailable(Some(value_id)) {
            return Vec::new();
        }
        let Some(source_index) = series.indicator_source_index_for_visual(visual_index) else {
            return Vec::new();
        };
        self.indicator_host
            .marker_outputs_for(value_id, source_index)
    }

    pub(super) fn indicator_segments(
        &self,
        value_id: PanelValueId,
        source_range: Range<usize>,
        scene: &Scene,
    ) -> Vec<indicator_engine::SegmentOutput> {
        if self.indicator_is_unavailable(Some(value_id)) {
            return Vec::new();
        }
        self.indicator_host
            .segment_outputs_for(value_id, source_range, |start, end| {
                let Some(start) = self.bar_unit_at_series_index(scene, 0, start) else {
                    return false;
                };
                let Some(end) = self.bar_unit_at_series_index(scene, 0, end) else {
                    return false;
                };
                start.min(end) <= scene.x.max_unit && start.max(end) >= scene.x.min_unit
            })
    }

    pub(super) fn indicator_levels(
        &self,
        value_id: PanelValueId,
    ) -> Vec<indicator_engine::HorizontalLevelOutput> {
        if self.indicator_is_unavailable(Some(value_id)) {
            return Vec::new();
        }
        self.indicator_host.level_outputs_for(value_id)
    }

    pub(super) fn indicator_regions(
        &self,
        value_id: PanelValueId,
        source_range: Range<usize>,
    ) -> Vec<indicator_engine::RegionOutput> {
        if self.indicator_is_unavailable(Some(value_id)) {
            return Vec::new();
        }
        self.indicator_host
            .region_outputs_for_range(value_id, Some(source_range))
    }

    /// Resolve the calculation bar at the chart coordinate under the cursor.
    ///
    /// Output offsets move the value away from its calculation bar.  A normal
    /// bar-index lookup therefore gives the wrong value for displaced series
    /// (for example Ichimoku's leading and lagging lines).  The scene owns the
    /// chart axis and can invert that mapping for a coordinate-aware readout.
    pub(super) fn indicator_source_index_for_scene_at(
        &self,
        scene: &Scene,
        series_index: usize,
        output: &IndicatorOutputRef,
        fallback_bar_index: usize,
    ) -> Option<usize> {
        self.indicator_source_index_for_scene_at_channel(
            scene,
            series_index,
            output.instance_id,
            &output.channel_id,
            fallback_bar_index,
        )
    }

    pub(super) fn indicator_source_index_for_scene_at_channel(
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

    pub(super) fn indicator_layer_values_at(
        &self,
        series: &KlineSeries,
        layer: &IndicatorOutputLayer,
        bar_index: usize,
    ) -> Option<IndicatorBarValues> {
        if self.indicator_is_unavailable(Some(layer.base_output.instance_id))
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

    pub(super) fn indicator_layer_values_for_source_at(
        &self,
        series: &KlineSeries,
        layer: &IndicatorOutputLayer,
        source_index: usize,
    ) -> Option<IndicatorBarValues> {
        if self.indicator_is_unavailable(Some(layer.base_output.instance_id))
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
        match self
            .indicator_presentation
            .input_coverage
            .state(required_input)
        {
            IndicatorInputCoverageState::Unknown => true,
            IndicatorInputCoverageState::Missing => false,
            IndicatorInputCoverageState::Available => {
                let Some(runtime_index) = series.indicator_source_index_for_visual(visual_index)
                else {
                    return false;
                };
                self.indicator_presentation
                    .input_coverage
                    .at(required_input, runtime_index)
            }
        }
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
        let required_input =
            self.indicator_presentation
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

    pub(super) fn indicator_source_value_for_series_at_with_offset(
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

    pub(super) fn map_indicator_plot_for_output(
        &self,
        scene: &Scene,
        panel_index: usize,
        output: &IndicatorOutputRef,
        value: f64,
    ) -> Option<f32> {
        let independent_scale = self
            .indicator_output_spec(output)
            .is_some_and(|spec| spec.independent_scale);
        scene.map_indicator_plot_scaled(panel_index, value, independent_scale)
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

    pub(super) fn panel_output_layers(&self, panel_index: usize) -> Vec<IndicatorOutputLayer> {
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
        self.panel_output_layers(panel_index)
            .iter()
            .any(|layer| layer.base_output.instance_id == value_id && layer.visible)
    }

    pub(super) fn resolved_indicator_layer_for_composition_layer(
        &self,
        layer: &composition::LayerSpec,
    ) -> Option<IndicatorOutputLayer> {
        let source_output = layer.source.indicator_output()?;
        if let Some(resolved) = self.indicator_presentation.layer_for_output(source_output) {
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
        Some(IndicatorOutputLayer {
            base_output,
            presentation_output,
            data_kind: layer.data_kind,
            purpose: layer.purpose,
            mark,
            required_input: kind.mark_spec(mark).and_then(|spec| spec.required_input),
            visible: layer.visible,
        })
    }

    pub(super) fn output_ref_for_composition_layer(
        &self,
        layer: &composition::LayerSpec,
    ) -> Option<IndicatorOutputRef> {
        self.resolved_indicator_layer_for_composition_layer(layer)
            .map(|layer| layer.presentation_output)
    }

    pub(super) fn overlay_channels_for_layer(
        &self,
        output: Option<&IndicatorOutputRef>,
    ) -> Option<OverlayChannelSpec<'_>> {
        let output = output?;
        let spec = self.indicator_output_spec(output)?;
        if spec.value_type != OutputValueType::Number {
            return None;
        }

        if let Some(kind) = self.indicator_kind(output.instance_id)
            && let Some(channel) = kind.overlay_channels().iter().find(|channel| {
                let data::chart::kline::IndicatorOverlaySource::Channel(channel_id) =
                    &channel.source;
                channel_id == &output.channel_id
            })
        {
            let data::chart::kline::IndicatorOverlaySource::Channel(channel_id) = &channel.source;
            return Some(OverlayChannelSpec {
                label: &channel.label,
                source: channel_id,
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

    pub(super) fn panel_count(&self) -> usize {
        self.composition.panel_count().max(1)
    }

    pub(super) fn panel_index_for_id(&self, panel_id: PanelId) -> Option<usize> {
        self.composition
            .panels
            .iter()
            .position(|panel| panel.id == panel_id)
    }

    pub(super) fn panel_id(&self, panel_index: usize) -> Option<composition::PanelId> {
        self.composition
            .panels
            .get(panel_index)
            .map(|panel| panel.id)
    }

    pub(super) fn panel_value_precision(&self, panel_index: usize) -> Option<PanelValuePrecision> {
        self.composition
            .panels
            .get(panel_index)
            .and_then(|panel| panel.value_precision)
    }

    pub(super) fn panel_value_label_policy(&self, panel_index: usize) -> PanelValueLabelPolicy {
        self.composition
            .panels
            .get(panel_index)
            .map(|panel| panel.value_label_policy)
            .unwrap_or_default()
    }

    pub(super) fn panel_y_range(&self, panel_id: PanelId) -> Option<YUnitRange> {
        self.indicator_y_ranges
            .iter()
            .find(|(id, _)| *id == panel_id)
            .map(|(_, viewport)| *viewport)
    }

    pub(super) fn panel_y_range_for_index(&self, panel_index: usize) -> Option<YUnitRange> {
        self.panel_id(panel_index)
            .and_then(|panel_id| self.panel_y_range(panel_id))
    }

    pub(super) fn normalized_panel_splits(&self) -> Vec<f32> {
        if self.panel_count() <= 1 {
            Vec::new()
        } else {
            self.composition.normalized_splits(DEFAULT_MIN_PANEL_RATIO)
        }
    }

    pub(super) fn default_mark_for_panel(kind: KlinePanelKind) -> MarkKind {
        match kind {
            KlinePanelKind::PrimaryChart => MarkKind::Candle,
            KlinePanelKind::Indicator => MarkKind::Bar(BarMode::Histogram(HistogramMode::Plain)),
        }
    }

    pub(super) fn default_title_for_panel(kind: KlinePanelKind) -> Option<&'static str> {
        match kind {
            KlinePanelKind::PrimaryChart => None,
            KlinePanelKind::Indicator => Some("Indicator"),
        }
    }

    pub(super) fn resolved_panel_title(
        &self,
        panel_index: usize,
        panel_kind: KlinePanelKind,
    ) -> Option<&str> {
        self.composition
            .panels
            .get(panel_index)
            .and_then(|panel| panel.title.as_deref())
            .filter(|title| !title.is_empty())
            .or_else(|| Self::default_title_for_panel(panel_kind))
    }

    pub(super) fn resolved_panel_mark(
        &self,
        panel_index: usize,
        panel_kind: KlinePanelKind,
    ) -> MarkKind {
        let Some(panel_id) = self.panel_id(panel_index) else {
            return Self::default_mark_for_panel(panel_kind);
        };

        self.composition
            .panel_effective_mark(panel_id)
            .unwrap_or_else(|| Self::default_mark_for_panel(panel_kind))
    }

    pub(super) fn resolved_panel_scale_mode(&self, panel_index: usize) -> PanelScaleMode {
        let Some(panel) = self.composition.panels.get(panel_index) else {
            return PanelScaleMode::Absolute;
        };

        self.composition
            .panel_effective_scale_mode(panel.id)
            .unwrap_or(PanelScaleMode::Absolute)
    }

    pub(super) fn series_line_color(
        &self,
        series: &KlineSeries,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) -> iced::Color {
        match series.color() {
            SeriesColor::Theme(color) => {
                Self::resolve_theme_color(color, palette, generated_accents)
            }
            SeriesColor::Custom(color) => color,
        }
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

    /// Return visible named output bindings on the primary panel. The
    /// instance-level list above summarizes visibility, but a declaration-only
    /// indicator can expose a channel that has no
    /// named output channel or global color-slot counterpart.  Keeping the
    /// binding as an output reference lets scene autoscaling include that
    /// channel as well as the renderer.
    pub(super) fn visible_primary_overlay_layers(&self) -> Vec<IndicatorOutputLayer> {
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
        let presentation = self.indicator_presentation.presentation(value_id);
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
                    let data::chart::kline::IndicatorOverlaySource::Channel(channel_id) =
                        &readout.source;
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
                let source_index = self.indicator_source_index_for_scene_at(
                    scene,
                    series_index,
                    output,
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

    pub(super) fn fill_specs_for_panel_value(
        &self,
        value_id: Option<PanelValueId>,
    ) -> &'static [data::chart::kline::IndicatorFillSpec] {
        value_id
            .and_then(|id| self.indicator_kind(id))
            .map(|kind| kind.fill_specs())
            .unwrap_or_default()
    }

    pub(super) fn indicator_line_width(
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

    pub(super) fn indicator_stroke_style(
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
            return Self::resolve_theme_color(color, palette, generated_accents);
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
        if !self.indicator_is_unavailable(Some(output_instance_id))
            && let Some(color) = runtime_index.and_then(|source_index| {
                self.indicator_host.output_color_for(
                    output_instance_id,
                    output_channel,
                    source_index,
                )
            })
        {
            return Self::resolve_indicator_color(color);
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

    pub(super) fn style_slot_for_mark<'slot>(
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
        let style_slot = if let Some(style_by) = self
            .indicator_output_spec(output)
            .and_then(|spec| spec.style_by.as_deref())
        {
            let state = self
                .indicator_raw_output_value_for_series_at_channel(
                    series,
                    output.instance_id,
                    style_by,
                    source_index,
                )
                .and_then(OutputValue::as_boolean);
            mark.map_or_else(
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
            )
        } else {
            mark.map_or_else(
                || {
                    self.style_slot_for_output(
                        output.instance_id,
                        output.channel_id.as_str(),
                        None,
                        fallback_slot,
                    )
                },
                |mark| {
                    self.style_slot_for_mark(
                        output.instance_id,
                        output.channel_id.as_str(),
                        mark,
                        None,
                        fallback_slot,
                    )
                },
            )
        };
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
            .and_then(|instance| {
                instance.style_overrides.color(color_slot).or_else(|| {
                    instance
                        .kind
                        .color_specs()
                        .iter()
                        .find(|spec| spec.id == color_slot)
                        .map(|spec| spec.default_color())
                })
            });
        let color =
            configured.unwrap_or_else(|| ThemeColor::new(color_role, ThemeColorShade::Base));
        Self::resolve_theme_color(color, palette, generated_accents).scale_alpha(match color_role {
            ThemeColorRole::Neutral => 0.72,
            _ => 0.78,
        })
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
            .and_then(|instance| {
                instance.style_overrides.color(slot).or_else(|| {
                    instance
                        .kind
                        .color_specs()
                        .iter()
                        .find(|spec| spec.id == slot)
                        .map(|spec| spec.default_color())
                })
            })
            .unwrap_or_else(|| ThemeColor::new(ThemeColorRole::Neutral, ThemeColorShade::Base));
        Self::resolve_theme_color(configured, palette, generated_accents)
    }

    fn color_role_for_style_slot(&self, value_id: PanelValueId, slot: &str) -> ThemeColorRole {
        self.indicator_kind(value_id)
            .and_then(|kind| kind.color_specs().iter().find(|spec| spec.id == slot))
            .map(|spec| spec.default_role)
            .unwrap_or(ThemeColorRole::Neutral)
    }

    pub(super) fn resolve_theme_color(
        color: ThemeColor,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) -> iced::Color {
        crate::widget::resolve_theme_color(color, palette, generated_accents)
    }

    pub(super) fn resolve_indicator_color(color: indicator_engine::Color) -> iced::Color {
        indicator_color_from_argb(color.0)
    }

    pub(super) fn candlestick_widths(bar_spacing: f32, pixels: PixelGeometry) -> (i32, i32) {
        let pixel_ratio = pixels.ratio();
        let from = 2.5_f32;
        let to = 4.0_f32;
        let coeff_special = 3.0_f32;

        let mut body_width = if bar_spacing >= from && bar_spacing <= to {
            (coeff_special * pixel_ratio).floor() as i32
        } else {
            let reducing_coeff = 0.2_f32;
            let coeff = 1.0
                - (reducing_coeff * (bar_spacing.max(to) - to).atan())
                    / (std::f32::consts::PI * 0.5);
            let res = (bar_spacing * coeff * pixel_ratio).floor() as i32;
            let scaled_bar_spacing = (bar_spacing * pixel_ratio).floor() as i32;
            res.min(scaled_bar_spacing).max(pixel_ratio.floor() as i32)
        };

        let max_width = (bar_spacing * pixel_ratio).floor() as i32;
        body_width = body_width.clamp(1, max_width.max(1));
        if body_width >= 2 {
            let wick_width = pixel_ratio.floor() as i32;
            if (wick_width & 1) != (body_width & 1) {
                body_width -= 1;
            }
        }
        let wick_width = (pixel_ratio.floor() as i32).max(1).min(body_width);

        (body_width.max(1), wick_width)
    }
}

fn indicator_color_from_argb(value: u32) -> iced::Color {
    iced::Color::from_rgba8(
        ((value >> 16) & 0xff) as u8,
        ((value >> 8) & 0xff) as u8,
        (value & 0xff) as u8,
        f32::from(((value >> 24) & 0xff) as u8) / 255.0,
    )
}

fn offset_source_index(index: usize, offset_bars: i32) -> Option<usize> {
    if offset_bars >= 0 {
        index.checked_sub(offset_bars as usize)
    } else {
        index.checked_add(offset_bars.unsigned_abs() as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(left: f32, right: f32) {
        assert!((left - right).abs() < 0.0001, "{left} != {right}");
    }

    #[test]
    fn positive_spans_reject_zero_and_non_finite_extents() {
        let pixels = PixelGeometry::from_ratio(2.0, Point::new(0.25, 0.5));

        assert_eq!(pixels.positive_x_span(4.0, 4.0), None);
        assert_eq!(pixels.positive_x_span(f32::NAN, 4.0), None);
        assert_eq!(pixels.positive_x_span(4.0, f32::INFINITY), None);
    }

    #[test]
    fn positive_spans_normalize_reversed_endpoints_and_keep_a_subpixel_mark() {
        let pixels = PixelGeometry::from_ratio(2.0, Point::new(0.25, 0.5));

        let span = pixels.positive_x_span(1.01, 1.0).expect("positive span");
        close(span.size, 0.5);
        close(span.start, 1.25);
    }

    #[test]
    fn snapped_spans_keep_intentional_zero_centered_marks_visible() {
        let pixels = PixelGeometry::from_ratio(2.0, Point::new(0.25, 0.5));
        let span = pixels.snapped_y_span(3.0, 3.0);

        close(span.size, 0.5);
    }

    #[test]
    fn clipped_positive_spans_do_not_turn_zero_extent_into_a_mark() {
        let pixels = PixelGeometry::from_ratio(2.0, Point::ORIGIN);

        assert_eq!(pixels.positive_x_span_in_bounds(1.0, 2.0, 2.0, 3.0), None);
        assert_eq!(pixels.positive_x_span_in_bounds(1.0, 2.0, 3.0, 4.0), None);
        assert!(
            pixels
                .positive_x_span_in_bounds(1.0, 2.0, 1.5, 3.0)
                .is_some()
        );
    }

    #[test]
    fn origin_is_used_for_physical_pixel_snapping() {
        let pixels = PixelGeometry::from_ratio(2.0, Point::new(0.25, 0.5));

        close(pixels.snap_x_to_cell(1.0), 1.25);
        close(pixels.snap_y_for_stroke(1.0, 1), 1.25);
    }
}
