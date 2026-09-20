use super::composition::{MarkKind, PanelScaleMode, PanelValueId, PanelValuePrecision};
use super::helpers::IndicatorOutputLayer;
use super::layout::{LayoutHitZone, PanelLayoutTree};
use super::scene::{
    BarUnitWindow, CursorInfo, CursorTarget, IndicatorPanelScene, LegendTarget, PanelDomainState,
    PanelValueScale, PrimaryPercentScaleState, PrimaryScale, Scene, SceneBase, TickIndex, XAxis,
    XViewport, Y_UNIT_STEP_FALLBACK,
};
use super::{
    DragMode, KlinePanelKind, KlineSeries, KlineWidget, PanelId, PanelYValueRange, State, YUnit,
};

use data::chart::kline::series::KlineSeriesPoint;
use data::chart::kline::viewport::MAX_AUTOSCALE_SPAN_FACTOR;
use data::chart::kline::viewport::YUnitRange;
use data::chart::kline::{HorizontalWorld, IndicatorOutputRef, ZOOM_SENSITIVITY};
use exchange::{Kline, Timeframe, UnixMs, unit::Price};

use iced::advanced::Layout;
use iced::{Point, Rectangle, mouse};
use std::sync::Arc;

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

#[derive(Debug, Clone, Copy, PartialEq)]
enum PrimaryDomainValue {
    Price(Price),
    Scalar(f64),
}

#[derive(Debug, Clone, Copy)]
struct PanelYViewportLimits {
    min_span: f64,
    max_span: f64,
    bounds: (f64, f64),
}

impl PanelYViewportLimits {
    fn apply(self, viewport: PanelYValueRange) -> Option<PanelYValueRange> {
        let viewport = viewport.clamp_span(self.min_span, self.max_span)?;
        viewport.clamp_to_bounds(self.bounds.0, self.bounds.1)
    }
}

fn autoscale_span_limits(
    autoscale_span: f64,
    physical_height: f64,
    min_span: f64,
) -> Option<(f64, f64)> {
    if autoscale_span.is_nan()
        || autoscale_span < 0.0
        || !physical_height.is_finite()
        || physical_height <= 0.0
        || !min_span.is_finite()
    {
        return None;
    }

    let min_span = min_span.abs().max(1e-8);
    let factor = physical_height.clamp(1.0, MAX_AUTOSCALE_SPAN_FACTOR);
    let autoscale_span = autoscale_span.min(f64::MAX).max(min_span);
    let max_span = if autoscale_span > f64::MAX / factor {
        f64::MAX
    } else {
        autoscale_span * factor
    };
    Some((min_span, max_span.max(min_span)))
}

#[derive(Debug, Default)]
struct PrimaryDomainExtrema {
    min: Option<PrimaryDomainValue>,
    max: Option<PrimaryDomainValue>,
}

impl PrimaryDomainExtrema {
    fn record_price(&mut self, value: Price) {
        self.record(PrimaryDomainValue::Price(value));
    }

    fn record_scalar(&mut self, value: f64) {
        if value.is_finite() {
            self.record(PrimaryDomainValue::Scalar(value));
        }
    }

    fn record(&mut self, value: PrimaryDomainValue) {
        if self
            .min
            .is_none_or(|current| primary_domain_value_less(value, current))
        {
            self.min = Some(value);
        }
        if self
            .max
            .is_none_or(|current| primary_domain_value_less(current, value))
        {
            self.max = Some(value);
        }
    }
}

fn primary_domain_value_less(lhs: PrimaryDomainValue, rhs: PrimaryDomainValue) -> bool {
    match (lhs, rhs) {
        (PrimaryDomainValue::Price(lhs), PrimaryDomainValue::Price(rhs)) => lhs < rhs,
        (PrimaryDomainValue::Scalar(lhs), PrimaryDomainValue::Scalar(rhs)) => lhs < rhs,
        (lhs, rhs) => lhs.as_f64() < rhs.as_f64(),
    }
}

impl PrimaryDomainValue {
    fn as_f64(self) -> f64 {
        match self {
            Self::Price(value) => value.to_f64(),
            Self::Scalar(value) => value,
        }
    }
}

fn indicator_line_source_range(
    units: &[i64],
    x_axis: XAxis,
    min_x_unit: i64,
    max_x_unit: i64,
    offset: i64,
    end: usize,
    traversal: IndicatorLineTraversal,
) -> Option<(usize, usize)> {
    if end == 0 {
        return None;
    }

    match traversal {
        IndicatorLineTraversal::VisibleWithNeighbors => {
            let (start, stop) = indicator_output_source_range(
                &units[..end],
                x_axis,
                min_x_unit,
                max_x_unit,
                offset,
            );
            (start < stop).then_some((start.saturating_sub(1), stop.saturating_add(1).min(end)))
        }
        IndicatorLineTraversal::SegmentWithNeighbors => {
            let (start, stop) = indicator_output_source_range(
                &units[..end],
                x_axis,
                min_x_unit,
                max_x_unit,
                offset,
            );
            (start < stop).then_some((start, stop))
        }
    }
}

impl<'a> KlineWidget<'a> {
    pub(super) fn base_ticker_info(&self) -> Option<exchange::TickerInfo> {
        self.series.first().map(|series| series.ticker_info)
    }

    pub(super) fn panel_value_scale(&self, panel_index: usize) -> PanelValueScale {
        PanelValueScale::new(
            self.panel_value_precision(panel_index),
            self.base_ticker_info(),
        )
    }

    fn panel_viewport_limits(
        &self,
        scene: &Scene,
        panel_index: usize,
    ) -> Option<PanelYViewportLimits> {
        let panel = scene.layout.panel(panel_index)?;
        let physical_height =
            f64::from(panel.plot.height.max(1.0)) * f64::from(self.pixel_ratio.value());

        match panel.kind {
            KlinePanelKind::PrimaryChart => {
                if scene.primary.mode.uses_percent_base() {
                    return None;
                }

                let log_scale_enabled = scene.can_use_log_primary_scale();
                let (autoscale_min, autoscale_max) = scene.primary_autoscale_display_values();
                let min_span = if matches!(scene.primary.mode, PanelScaleMode::Logarithmic) {
                    1e-3
                } else {
                    self.panel_value_scale(scene.primary.panel_index)
                        .quantization_step_f32()
                        .map(f64::from)
                        .unwrap_or(1e-4)
                        .abs()
                        .max(1e-8)
                        * 8.0
                };
                let autoscale_span = (f64::from(autoscale_max) - f64::from(autoscale_min)).abs();
                let autoscale_span = if autoscale_span.is_finite() {
                    autoscale_span.max(min_span)
                } else {
                    autoscale_span
                };
                let (min_span, mut max_span) =
                    autoscale_span_limits(autoscale_span, physical_height, min_span)?;
                let bounds = if log_scale_enabled
                    && matches!(scene.primary.mode, PanelScaleMode::Logarithmic)
                {
                    (
                        f64::from(f32::MIN_POSITIVE).log10(),
                        f64::from(f32::MAX).log10(),
                    )
                } else {
                    (f64::from(f32::MIN), f64::from(f32::MAX))
                };
                max_span = max_span.min(bounds.1 - bounds.0);
                (max_span >= min_span).then_some(PanelYViewportLimits {
                    min_span,
                    max_span,
                    bounds,
                })
            }
            KlinePanelKind::Indicator => {
                let indicator = scene
                    .indicator_panels
                    .iter()
                    .find(|indicator| indicator.panel_index == panel_index)?;
                let value_scale = self.panel_value_scale(panel_index);
                let bounds = self
                    .panel_value_precision(panel_index)
                    .and_then(|precision| precision.value_bounds(self.base_ticker_info()))
                    .unwrap_or_else(|| {
                        let step = f64::from(Y_UNIT_STEP_FALLBACK);
                        (i64::MIN as f64 * step, i64::MAX as f64 * step)
                    });
                let (autoscale_min, autoscale_max) =
                    value_scale.values(indicator.domain.autoscale());
                let min_span = value_scale
                    .quantization_step()
                    .unwrap_or(1e-4)
                    .abs()
                    .max(1e-8)
                    * 8.0;
                let (min_span, max_span) = autoscale_span_limits(
                    (autoscale_max - autoscale_min).abs(),
                    physical_height,
                    min_span,
                )?;
                let max_span = max_span.min(bounds.1 - bounds.0);
                if max_span < min_span {
                    return None;
                }
                Some(PanelYViewportLimits {
                    min_span,
                    max_span,
                    bounds,
                })
            }
        }
    }

    fn value_range_to_unit_range(
        &self,
        panel_index: usize,
        viewport: PanelYValueRange,
    ) -> Option<YUnitRange> {
        let scale = self.panel_value_scale(panel_index);
        let min_unit = scale.value_to_unit(viewport.min_value());
        let max_unit = scale.value_to_unit(viewport.max_value());
        Some(YUnitRange::new(min_unit, max_unit))
    }

    fn resolve_panel_viewport(
        &self,
        scene: &Scene,
        panel_index: usize,
        range: YUnitRange,
    ) -> Option<YUnitRange> {
        let panel = scene.layout.panel(panel_index)?;
        let limits = self.panel_viewport_limits(scene, panel_index)?;

        match panel.kind {
            KlinePanelKind::PrimaryChart => {
                let (min_value, max_value) = scene.primary.value_scale.values(range);
                let viewport = PanelYValueRange::normalized(min_value, max_value)?;
                let log_scale_enabled = scene.can_use_log_primary_scale();
                if log_scale_enabled
                    && matches!(scene.primary.mode, PanelScaleMode::Logarithmic)
                    && (viewport.min_value() <= 0.0 || viewport.max_value() <= 0.0)
                {
                    return None;
                }
                let min_display = scene.primary.mode.value_to_display(
                    viewport.min_value() as f32,
                    scene.primary.anchor,
                    log_scale_enabled,
                );
                let max_display = scene.primary.mode.value_to_display(
                    viewport.max_value() as f32,
                    scene.primary.anchor,
                    log_scale_enabled,
                );
                let display_viewport =
                    PanelYValueRange::normalized(f64::from(min_display), f64::from(max_display))?;
                let display_viewport = limits.apply(display_viewport)?;
                let min_value = scene.primary_display_to_value(display_viewport.min_value() as f32);
                let max_value = scene.primary_display_to_value(display_viewport.max_value() as f32);
                if !min_value.is_finite() || !max_value.is_finite() {
                    return None;
                }
                let scale = scene.primary.value_scale;
                let min_unit = scale.value_to_unit_f32(min_value);
                let max_unit = scale.value_to_unit_f32(max_value);
                Some(YUnitRange::new(min_unit, max_unit))
            }
            KlinePanelKind::Indicator => {
                let scale = self.panel_value_scale(panel_index);
                let (min_value, max_value) = scale.values(range);
                let viewport = limits.apply(PanelYValueRange::normalized(min_value, max_value)?)?;
                self.value_range_to_unit_range(panel_index, viewport)
            }
        }
    }

    fn apply_panel_viewports(&self, scene: &mut Scene) {
        if !scene
            .primary
            .mode
            .primary_autoscale_enabled(self.primary_viewport.is_auto())
            && let Some(range) = self.primary_viewport.range()
            && let Some(range) =
                self.resolve_panel_viewport(scene, scene.primary.panel_index, range)
        {
            scene.primary.domain.set_active(range);
        }

        for index in 0..scene.indicator_panels.len() {
            let panel_index = scene.indicator_panels[index].panel_index;
            let Some(range) = self.panel_y_range_for_index(panel_index) else {
                continue;
            };
            if let Some(range) = self.resolve_panel_viewport(scene, panel_index, range) {
                scene.indicator_panels[index].domain.set_active(range);
            }
        }
    }

    pub(super) fn reconciled_y_ranges(&self, scene: &Scene) -> Vec<(PanelId, YUnitRange)> {
        let mut changes = Vec::new();

        if !scene.primary.mode.uses_percent_base()
            && let Some(range) = self.primary_viewport.range()
            && let Some(effective) =
                self.resolve_panel_viewport(scene, scene.primary.panel_index, range)
            && effective != range
            && let Some(panel_id) = self.panel_id(scene.primary.panel_index)
        {
            changes.push((panel_id, effective));
        }

        for (panel_id, range) in self.indicator_y_ranges.iter().copied() {
            let Some(panel_index) = self.panel_index_for_id(panel_id) else {
                continue;
            };
            let Some(effective) = self.resolve_panel_viewport(scene, panel_index, range) else {
                continue;
            };
            if effective != range {
                changes.push((panel_id, effective));
            }
        }

        changes
    }

    pub(super) fn panel_viewport_after_y_zoom(
        &self,
        scene: &Scene,
        panel_index: usize,
        cursor_y: f32,
        scroll_delta: f32,
    ) -> Option<YUnitRange> {
        if scroll_delta.abs() <= f32::EPSILON {
            return None;
        }

        if panel_index == scene.primary.panel_index
            && !scene.primary.mode.allows_primary_manual_zoom()
        {
            return None;
        }

        let panel = scene.layout.panel(panel_index)?;
        let panel_h = panel.plot.height.max(1.0);
        let y_in_panel =
            (cursor_y - scene.layout.regions.plot.y - panel.plot.y).clamp(0.0, panel_h);
        let anchor_ratio = if panel.kind == KlinePanelKind::PrimaryChart {
            scene.primary_value_ratio_at_plot_y(y_in_panel)
        } else {
            (1.0 - (y_in_panel / panel_h)).clamp(0.0, 1.0)
        };

        let zoom_amount = scroll_delta.abs().clamp(0.0, 6.0);
        let zoom_scale = (zoom_amount / ZOOM_SENSITIVITY).exp();
        let zoom_in = scroll_delta > 0.0;

        match panel.kind {
            KlinePanelKind::PrimaryChart => {
                let (min_display, max_display) = scene.primary_domain_display_values();
                let display_viewport =
                    PanelYValueRange::normalized(f64::from(min_display), f64::from(max_display))?;
                let new_display_viewport = display_viewport.zoom_around_ratio(
                    f64::from(anchor_ratio),
                    f64::from(zoom_scale),
                    zoom_in,
                )?;

                let display_viewport = PanelYValueRange::normalized(
                    f64::from(
                        scene.primary_display_to_value(new_display_viewport.min_value() as f32),
                    ),
                    f64::from(
                        scene.primary_display_to_value(new_display_viewport.max_value() as f32),
                    ),
                )?;
                let range = self.value_range_to_unit_range(panel_index, display_viewport)?;
                self.resolve_panel_viewport(scene, panel_index, range)
            }
            KlinePanelKind::Indicator => {
                let indicator = scene
                    .indicator_panels
                    .iter()
                    .find(|indicator| indicator.panel_index == panel_index)?;
                let viewport =
                    PanelYValueRange::normalized(indicator.min_value(), indicator.max_value())?;
                let viewport = viewport.zoom_around_ratio(
                    f64::from(anchor_ratio),
                    f64::from(zoom_scale),
                    zoom_in,
                )?;
                let range = self.value_range_to_unit_range(panel_index, viewport)?;
                self.resolve_panel_viewport(scene, panel_index, range)
            }
        }
    }

    pub(super) fn panel_viewport_after_primary_y_pan(
        &self,
        scene: &Scene,
        panel_index: usize,
        dy_px: f32,
    ) -> Option<YUnitRange> {
        if dy_px.abs() <= f32::EPSILON {
            return None;
        }

        if panel_index != scene.primary.panel_index
            || !scene
                .primary
                .mode
                .allows_primary_manual_pan(self.primary_viewport.is_auto())
        {
            return None;
        }

        let panel = scene.layout.panel(panel_index)?;
        let panel_h = f64::from(panel.plot.height.max(1.0));
        let delta_ratio = if scene.primary.inverted {
            -f64::from(dy_px) / panel_h
        } else {
            f64::from(dy_px) / panel_h
        };

        let (min_display, max_display) = scene.primary_domain_display_values();
        let display_viewport =
            PanelYValueRange::normalized(f64::from(min_display), f64::from(max_display))?;
        let shifted_display_viewport = display_viewport.pan_by_ratio(delta_ratio)?;

        let viewport = PanelYValueRange::normalized(
            f64::from(scene.primary_display_to_value(shifted_display_viewport.min_value() as f32)),
            f64::from(scene.primary_display_to_value(shifted_display_viewport.max_value() as f32)),
        )?;
        let range = self.value_range_to_unit_range(panel_index, viewport)?;
        self.resolve_panel_viewport(scene, panel_index, range)
    }

    fn format_panel_value_compact(
        &self,
        panel_index: usize,
        value_precision: Option<PanelValuePrecision>,
        value: f64,
        fallback_step: f64,
    ) -> String {
        let max_decimals = self.panel_value_label_policy(panel_index).max_decimals;

        if let Some(precision) = value_precision {
            return precision.format_compact(
                value,
                fallback_step,
                max_decimals,
                self.base_ticker_info(),
            );
        }

        let scale = self.panel_value_scale(panel_index);
        let quantized = scale.unit_to_value(scale.value_to_unit(value));
        let fallback = fallback_step.abs().max(1e-6);
        let decimals = PanelValuePrecision::decimals_for_step(fallback);
        let decimals = max_decimals
            .map(|max| decimals.min(max as usize))
            .unwrap_or(decimals);
        format!("{quantized:.decimals$}")
    }

    pub(super) fn format_panel_axis_value(
        &self,
        panel_index: usize,
        value_precision: Option<PanelValuePrecision>,
        value: f64,
        fallback_step: f64,
    ) -> String {
        let scale = self.panel_value_scale(panel_index);
        self.panel_value_label_policy(panel_index)
            .format_axis_value(
                || scale.unit_to_value(scale.value_to_unit(value)),
                || {
                    self.format_panel_value_compact(
                        panel_index,
                        value_precision,
                        value,
                        fallback_step,
                    )
                },
            )
    }

    pub(super) fn format_panel_header_value(
        &self,
        panel_index: usize,
        value_precision: Option<PanelValuePrecision>,
        value: f64,
    ) -> String {
        let scale = self.panel_value_scale(panel_index);
        self.panel_value_label_policy(panel_index)
            .format_header_value(
                || scale.unit_to_value(scale.value_to_unit(value)),
                || self.format_panel_value_compact(panel_index, value_precision, value, 0.01),
            )
    }

    pub(super) fn bar_at_or_before_unit<'b>(
        &self,
        series: &'b KlineSeries,
        x_axis: XAxis,
        target_unit: i64,
    ) -> Option<&'b Kline> {
        match x_axis {
            XAxis::Time { .. } => {
                let bucket_end = time_bucket_end_for_unit(x_axis, target_unit)?;
                series.bar_at_or_before_time(bucket_end)
            }
            XAxis::Tick { .. } => {
                let point_count = series.point_count();
                let from_latest = x_axis
                    .tick_from_unit(target_unit)
                    .map(|index| index.as_u64())
                    .and_then(|index| usize::try_from(index).ok())
                    .unwrap_or_default();
                let index = point_count.checked_sub(from_latest.checked_add(1)?)?;
                series.point_at(index).map(|point| point.kline)
            }
        }
    }

    pub(super) fn bar_index_at_or_before_unit(
        &self,
        scene: &Scene,
        series_index: usize,
        target_unit: i64,
    ) -> Option<usize> {
        let series = self.series.get(series_index)?;
        match scene.x.axis {
            XAxis::Time { .. } => {
                let bucket_end = time_bucket_end_for_unit(scene.x.axis, target_unit)?;
                series.bar_index_at_or_before_time(bucket_end)
            }
            XAxis::Tick { .. } => {
                let point_count = series.point_count();
                let from_latest = scene
                    .x
                    .axis
                    .tick_from_unit(target_unit)
                    .and_then(|index| usize::try_from(index.as_u64()).ok())
                    .unwrap_or_default();
                point_count.checked_sub(from_latest.checked_add(1)?)
            }
        }
    }

    pub(super) fn bar_unit_at_series_index(
        &self,
        scene: &Scene,
        series_index: usize,
        point_index: usize,
    ) -> Option<i64> {
        let series = self.series.get(series_index)?;
        if point_index >= series.point_count() {
            return None;
        }
        if let Some(unit) = scene
            .bar_units
            .get(series_index)
            .and_then(|units| units.unit_at_global_index(point_index))
        {
            return Some(unit);
        }

        match scene.x.axis {
            XAxis::Time { .. } => series
                .time_at_index(point_index)
                .map(|time| scene.x.axis.unit_from_time(time)),
            XAxis::Tick { .. } => {
                let from_latest = series
                    .point_count()
                    .saturating_sub(1)
                    .saturating_sub(point_index) as u64;
                Some(
                    scene
                        .x
                        .axis
                        .unit_from_tick(TickIndex::from_u64(from_latest)),
                )
            }
        }
    }

    /// Resolve the calculation bar that owns a channel at a chart coordinate.
    /// This is the inverse of `for_each_indicator_output_index_with_units` and
    /// is intentionally offset-aware for crosshair and legend readouts.
    pub(super) fn indicator_source_index_at_or_before_unit_for_channel(
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
        let source_unit = match scene.x.axis {
            XAxis::Time { .. } => target_unit.saturating_sub(offset),
            XAxis::Tick { .. } => target_unit.saturating_add(offset),
        };
        self.bar_index_at_or_before_unit(scene, series_index, source_unit)
    }

    fn compute_primary_scale_anchor(
        &self,
        bar_units: &[BarUnitWindow],
        x_axis: XAxis,
        min_x_unit: i64,
        max_x_unit: i64,
    ) -> Option<f32> {
        let mut first_unit = i64::MAX;
        let mut base_close = None;

        self.for_each_bar_unit_index_with_units(
            x_axis,
            bar_units,
            min_x_unit,
            max_x_unit,
            |series_index, _, _, unit, point| {
                if series_index != 0 {
                    return;
                }

                if unit < first_unit {
                    first_unit = unit;
                    base_close = Some(point.kline.close.to_f32_lossy());
                }
            },
        );

        base_close
    }

    fn compute_series_percent_anchors(
        &self,
        bar_units: &[BarUnitWindow],
        x_axis: XAxis,
        min_x_unit: i64,
        max_x_unit: i64,
    ) -> Vec<Option<f32>> {
        let mut anchors = vec![None; self.series.len()];
        let mut first_units = vec![i64::MAX; self.series.len()];

        self.for_each_bar_unit_index_with_units(
            x_axis,
            bar_units,
            min_x_unit,
            max_x_unit,
            |series_index, _, _, unit, point| {
                if unit < first_units[series_index] {
                    first_units[series_index] = unit;
                    anchors[series_index] = Some(point.kline.close.to_f32_lossy());
                }
            },
        );

        anchors
    }

    fn compute_primary_percent_domain(
        &self,
        bar_units: &[BarUnitWindow],
        x_axis: XAxis,
        min_x_unit: i64,
        max_x_unit: i64,
        primary_mark: MarkKind,
        anchors: &[Option<f32>],
    ) -> Option<(f32, f32)> {
        let mut min_pct: Option<f64> = None;
        let mut max_pct: Option<f64> = None;
        let primary_overlay_layers = self.visible_primary_overlay_layers();
        let include_primary_ohlc_range = self.uses_primary_ohlc_range(primary_mark);
        let include_footprint_range = self.trade_data_enabled && self.footprint.visible;
        let footprint_half_tick = self
            .series
            .first()
            .map(|series| series.footprint_tick_size().to_f32_lossy() * 0.5)
            .unwrap_or(0.0);
        self.for_each_bar_unit_index_with_units(
            x_axis,
            bar_units,
            min_x_unit,
            max_x_unit,
            |series_index, _bar_index, _series, _unit, point| {
                let Some(anchor) = anchors
                    .get(series_index)
                    .copied()
                    .flatten()
                    .filter(|anchor| anchor.abs() > f32::EPSILON)
                else {
                    return;
                };
                let anchor = f64::from(anchor);

                let mut emit_pct = |value: f32| {
                    let pct = ((f64::from(value) / anchor) - 1.0) * 100.0;
                    min_pct = Some(min_pct.map_or(pct, |current| current.min(pct)));
                    max_pct = Some(max_pct.map_or(pct, |current| current.max(pct)));
                };

                if series_index == 0 && include_primary_ohlc_range {
                    emit_pct(point.kline.low.to_f32_lossy());
                    emit_pct(point.kline.high.to_f32_lossy());
                } else {
                    emit_pct(point.kline.close.to_f32_lossy());
                }

                if series_index == 0 && include_footprint_range {
                    for price in point.footprint.trades.keys() {
                        let price = price.to_f32_lossy();
                        emit_pct(price - footprint_half_tick);
                        emit_pct(price + footprint_half_tick);
                    }
                }
            },
        );

        for layer in &primary_overlay_layers {
            let output = &layer.presentation_output;
            let Some(spec) = self.indicator_output_spec(output) else {
                continue;
            };
            self.for_each_indicator_output_index_with_units(
                bar_units,
                x_axis,
                min_x_unit,
                max_x_unit,
                output,
                |series, source_index, _output_unit| {
                    if !self.indicator_input_available_at(
                        series,
                        source_index,
                        layer.required_input,
                    ) {
                        return;
                    }
                    let Some(anchor) = anchors
                        .first()
                        .copied()
                        .flatten()
                        .filter(|anchor| anchor.abs() > f32::EPSILON)
                    else {
                        return;
                    };
                    let anchor = f64::from(anchor);
                    let mut emit_output_value = |value: f64| {
                        if !value.is_finite() {
                            return;
                        }
                        let pct = ((value / anchor) - 1.0) * 100.0;
                        min_pct = Some(min_pct.map_or(pct, |current| current.min(pct)));
                        max_pct = Some(max_pct.map_or(pct, |current| current.max(pct)));
                    };

                    match spec.value_type {
                        data::chart::kline::OutputValueType::Number => {
                            if matches!(layer.mark, MarkKind::Candle | MarkKind::Bar(_)) {
                                let Some(values) = self.indicator_layer_values_for_source_at(
                                    series,
                                    layer,
                                    source_index,
                                ) else {
                                    return;
                                };
                                match values {
                                    super::IndicatorBarValues::Plain { value } => {
                                        emit_output_value(value);
                                    }
                                    super::IndicatorBarValues::SignedOverlay {
                                        background_value,
                                        signed_value,
                                    } => {
                                        emit_output_value(background_value);
                                        emit_output_value(signed_value.abs());
                                    }
                                }
                            } else if let Some(value) = self
                                .indicator_raw_numeric_output_value_for_series_at(
                                    series,
                                    output,
                                    source_index,
                                )
                            {
                                emit_output_value(value);
                            }
                        }
                        data::chart::kline::OutputValueType::Ohlc => {
                            let Some(values) = self.indicator_ohlc_output_for_series_at(
                                series,
                                output,
                                source_index,
                            ) else {
                                return;
                            };
                            for value in [values.open, values.high, values.low, values.close]
                                .into_iter()
                                .flatten()
                            {
                                emit_output_value(value);
                            }
                        }
                        data::chart::kline::OutputValueType::Range => {
                            let Some(values) = self.indicator_range_output_for_series_at(
                                series,
                                output,
                                source_index,
                            ) else {
                                return;
                            };
                            if let Some(upper) = values.upper {
                                emit_output_value(upper);
                            }
                            if let Some(lower) = values.lower {
                                emit_output_value(lower);
                            }
                        }
                        data::chart::kline::OutputValueType::Boolean => {}
                    }
                },
            );
        }

        let min_pct = min_pct?;
        let max_pct = max_pct?;

        let pad = if (max_pct - min_pct).abs() <= f64::EPSILON {
            max_pct.abs().max(1.0)
        } else {
            (max_pct - min_pct).abs() * 0.05
        };

        Some(((min_pct - pad) as f32, (max_pct + pad) as f32))
    }

    fn uses_primary_ohlc_range(&self, primary_mark: MarkKind) -> bool {
        !primary_mark.is_line_like() || (self.trade_data_enabled && self.footprint.visible)
    }

    fn compute_primary_percent_scale_state(
        &self,
        bar_units: &[BarUnitWindow],
        x_axis: XAxis,
        min_x_unit: i64,
        max_x_unit: i64,
        primary_mark: MarkKind,
        primary_scale_mode: PanelScaleMode,
    ) -> Option<PrimaryPercentScaleState> {
        if !primary_scale_mode.uses_percent_base() {
            return Some(PrimaryPercentScaleState {
                primary_scale_anchor: None,
                series_percent_anchors: vec![None; self.series.len()],
                primary_domain_display_override: None,
            });
        }

        if primary_scale_mode.uses_series_percent_anchors(self.series.len()) {
            let anchors =
                self.compute_series_percent_anchors(bar_units, x_axis, min_x_unit, max_x_unit);
            let domain_override = self.compute_primary_percent_domain(
                bar_units,
                x_axis,
                min_x_unit,
                max_x_unit,
                primary_mark,
                &anchors,
            )?;

            let base_anchor = anchors.first().copied().flatten();
            return Some(PrimaryPercentScaleState {
                primary_scale_anchor: base_anchor,
                series_percent_anchors: anchors,
                primary_domain_display_override: Some(domain_override),
            });
        }

        let base_anchor =
            self.compute_primary_scale_anchor(bar_units, x_axis, min_x_unit, max_x_unit);
        Some(PrimaryPercentScaleState {
            primary_scale_anchor: base_anchor,
            series_percent_anchors: vec![base_anchor; self.series.len()],
            primary_domain_display_override: None,
        })
    }

    fn resolve_x_axis(&self) -> Option<XAxis> {
        match self.basis {
            data::chart::Basis::Time(timeframe) => {
                let anchor = self
                    .series
                    .iter()
                    .filter_map(|series| series.latest_point())
                    .map(|point| point.kline.time)
                    .max()?;

                Some(XAxis::Time { timeframe, anchor })
            }
            data::chart::Basis::Tick(_) => {
                if self.max_points_available() == 0 {
                    None
                } else {
                    Some(XAxis::Tick {
                        anchor: TickIndex::ZERO,
                    })
                }
            }
        }
    }

    fn max_points_available(&self) -> usize {
        self.series
            .iter()
            .map(KlineSeries::point_count)
            .max()
            .unwrap_or_default()
    }

    fn build_bar_units(
        &self,
        x_axis: XAxis,
        min_x_unit: i64,
        max_x_unit: i64,
    ) -> Vec<BarUnitWindow> {
        self.series
            .iter()
            .map(|series| {
                let (start_index, end_index) = match x_axis {
                    XAxis::Time { .. } => {
                        let Some((earliest, latest)) =
                            time_bucket_start_for_unit(x_axis, min_x_unit)
                                .zip(time_bucket_end_for_unit(x_axis, max_x_unit))
                        else {
                            return BarUnitWindow {
                                values: Vec::new(),
                                start_index: 0,
                            };
                        };
                        let range = series.time_index_range(earliest, latest);
                        (range.start, range.end)
                    }
                    XAxis::Tick { .. } => {
                        let point_count = series.point_count();
                        let point_count_i64 = i64::try_from(point_count).unwrap_or(i64::MAX);
                        let start = min_x_unit
                            .saturating_add(point_count_i64.saturating_sub(1))
                            .max(0);
                        let end = max_x_unit
                            .saturating_add(point_count_i64)
                            .min(point_count_i64);
                        (
                            usize::try_from(start)
                                .unwrap_or(point_count)
                                .min(point_count),
                            usize::try_from(end).unwrap_or(point_count).min(point_count),
                        )
                    }
                };

                let values = (start_index..end_index)
                    .filter_map(|index| match x_axis {
                        XAxis::Time { .. } => series
                            .time_at_index(index)
                            .map(|time| x_axis.unit_from_time(time)),
                        XAxis::Tick { .. } => {
                            let point_count = series.point_count();
                            let from_latest =
                                point_count.saturating_sub(1).saturating_sub(index) as u64;
                            Some(x_axis.unit_from_tick(TickIndex::from_u64(from_latest)))
                        }
                    })
                    .collect();

                BarUnitWindow {
                    values,
                    start_index,
                }
            })
            .collect()
    }

    fn indicator_source_offsets(&self) -> Vec<i64> {
        let mut offsets = vec![0];
        for instance in self.indicator_instances.instances() {
            let kind = &instance.kind;
            offsets.extend(
                kind.output_channels()
                    .iter()
                    .map(|output| i64::from(output.offset_bars)),
            );

            for fill in kind.fill_specs() {
                let channel_offset = |source: &data::chart::kline::IndicatorOverlaySource| {
                    let data::chart::kline::IndicatorOverlaySource::Channel(channel) = source;
                    kind.output_channels()
                        .iter()
                        .find(|output| output.id == *channel)
                        .map(|output| i64::from(output.offset_bars))
                        .unwrap_or_default()
                };
                offsets.push(if fill.upper_offset_bars == 0 {
                    channel_offset(&fill.upper)
                } else {
                    i64::from(fill.upper_offset_bars)
                });
                offsets.push(if fill.lower_offset_bars == 0 {
                    channel_offset(&fill.lower)
                } else {
                    i64::from(fill.lower_offset_bars)
                });
            }
        }
        offsets
    }

    fn for_each_bar_unit_index_with_units(
        &self,
        x_axis: XAxis,
        bar_units: &[BarUnitWindow],
        min_x_unit: i64,
        max_x_unit: i64,
        mut f: impl FnMut(usize, usize, &KlineSeries, i64, KlineSeriesPoint<'_>),
    ) {
        for (series_index, series) in self.series.iter().enumerate() {
            let Some(units) = bar_units.get(series_index) else {
                continue;
            };
            let (start, end) = bar_index_range_for_units(&units.values, min_x_unit, max_x_unit);
            let end = end.min(series.point_count().saturating_sub(units.start_index));
            if start >= end {
                continue;
            }

            let time_range = match x_axis {
                XAxis::Time { .. } => time_bucket_start_for_unit(x_axis, units.values[start])
                    .zip(time_bucket_end_for_unit(x_axis, units.values[end - 1])),
                XAxis::Tick { .. } => None,
            };
            let global_range = units.global_range(start..end);
            series.for_each_point_in_range(global_range, time_range, |point| {
                let Some(local_index) = point.index.checked_sub(units.start_index) else {
                    return;
                };
                if (start..end).contains(&local_index) {
                    f(
                        series_index,
                        point.index,
                        series,
                        units.values[local_index],
                        point,
                    );
                }
            });
        }
    }

    pub(super) fn for_each_scene_bar_unit_index(
        &self,
        scene: &Scene,
        f: impl FnMut(usize, usize, &KlineSeries, i64, KlineSeriesPoint<'_>),
    ) {
        self.for_each_bar_unit_index_with_units(
            scene.x.axis,
            &scene.bar_units,
            scene.x.min_unit,
            scene.x.max_unit,
            f,
        );
    }

    fn for_each_indicator_output_index_with_units(
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
        mut f: impl FnMut(&KlineSeries, usize, i64),
    ) {
        let Some(series) = self.series.first() else {
            return;
        };
        let Some(units) = bar_units.first() else {
            return;
        };
        let offset = self
            .indicator_output_spec_for_channel(value_id, channel_id)
            .map(|spec| i64::from(spec.offset_bars))
            .unwrap_or_default();
        let end = series
            .point_count()
            .saturating_sub(units.start_index)
            .min(units.values.len());
        let (start, end) = indicator_output_source_range(
            &units.values[..end],
            x_axis,
            min_x_unit,
            max_x_unit,
            offset,
        );
        for (source_index, &unit) in units.values.iter().enumerate().take(end).skip(start) {
            let Some(source_index) = units.global_index(source_index) else {
                continue;
            };
            f(
                series,
                source_index,
                shifted_output_unit(x_axis, unit, offset),
            );
        }
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

    pub(super) fn for_each_scene_indicator_line_output_index(
        &self,
        scene: &Scene,
        output: &IndicatorOutputRef,
        traversal: IndicatorLineTraversal,
        f: impl FnMut(&KlineSeries, usize, i64) -> IndicatorLinePointStatus,
    ) {
        self.for_each_scene_indicator_line_output_index_for_channel(
            scene,
            output.instance_id,
            &output.channel_id,
            traversal,
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
            .saturating_sub(units.start_index)
            .min(units.values.len());
        let Some((start, end)) = indicator_line_source_range(
            &units.values,
            scene.x.axis,
            scene.x.min_unit,
            scene.x.max_unit,
            offset,
            end,
            traversal,
        ) else {
            return;
        };

        if traversal != IndicatorLineTraversal::SegmentWithNeighbors {
            for local_index in start..end {
                let Some(source_index) = units.global_index(local_index) else {
                    continue;
                };
                let output_unit =
                    shifted_output_unit(scene.x.axis, units.values[local_index], offset);
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
            let Some(unit) = self.bar_unit_at_series_index(scene, 0, source_index) else {
                break;
            };
            match f(
                series,
                source_index,
                shifted_output_unit(scene.x.axis, unit, offset),
            ) {
                IndicatorLinePointStatus::Rendered | IndicatorLinePointStatus::Break => break,
                IndicatorLinePointStatus::Skipped => {}
            }
        }

        for local_index in start..end {
            let Some(source_index) = units.global_index(local_index) else {
                continue;
            };
            let output_unit = shifted_output_unit(scene.x.axis, units.values[local_index], offset);
            let _ = f(series, source_index, output_unit);
        }

        for source_index in visible_end..series.point_count() {
            let Some(unit) = self.bar_unit_at_series_index(scene, 0, source_index) else {
                break;
            };
            match f(
                series,
                source_index,
                shifted_output_unit(scene.x.axis, unit, offset),
            ) {
                IndicatorLinePointStatus::Rendered | IndicatorLinePointStatus::Break => break,
                IndicatorLinePointStatus::Skipped => {}
            }
        }
    }

    fn data_x_bounds(&self, x_axis: XAxis) -> Option<(i64, i64)> {
        match x_axis {
            XAxis::Time { .. } => self
                .series
                .iter()
                .filter_map(|series| {
                    let first = series.first_point()?.kline.time;
                    let last = series.latest_point()?.kline.time;
                    Some((x_axis.unit_from_time(first), x_axis.unit_from_time(last)))
                })
                .reduce(|(min_unit, max_unit), (first, last)| {
                    (min_unit.min(first), max_unit.max(last))
                }),
            XAxis::Tick { .. } => {
                let point_count = self.max_points_available();
                let last = x_axis.unit_from_tick(TickIndex::ZERO);
                let first = x_axis
                    .unit_from_tick(TickIndex::from_u64(point_count.saturating_sub(1) as u64));
                Some((first, last))
            }
        }
    }

    fn compute_x_window(
        &self,
        data_bounds: (i64, i64),
        plot_width: f32,
        horizontal_world: HorizontalWorld,
    ) -> (i64, i64, f32) {
        let (data_min_x, data_max_x) = data_bounds;

        let span = horizontal_world.spacing();
        let visible_units = (plot_width.max(1.0) / span).max(1.0);
        let right_cap = data_max_x as f32 + visible_units;
        let right_floor = data_min_x as f32;
        let right = (data_max_x as f32 + horizontal_world.offset()).clamp(right_floor, right_cap);
        let left = right - visible_units + 1.0;
        let left_unit = left.floor() as i64;
        let right_unit = right.ceil() as i64;

        if right_unit <= left_unit {
            return (left_unit, left_unit.saturating_add(1), right);
        }

        (left_unit, right_unit, right)
    }

    fn compute_primary_domain(
        &self,
        bar_units: &[BarUnitWindow],
        x_axis: XAxis,
        min_x_unit: i64,
        max_x_unit: i64,
        primary_value_precision: Option<PanelValuePrecision>,
        primary_mark: MarkKind,
        primary_plot_height: f32,
    ) -> Option<(YUnit, YUnit)> {
        let value_scale = PanelValueScale::new(primary_value_precision, self.base_ticker_info());
        let mut primary_extrema = PrimaryDomainExtrema::default();
        let primary_overlay_layers = self.visible_primary_overlay_layers();
        let include_primary_ohlc_range = self.uses_primary_ohlc_range(primary_mark);
        let include_footprint_range = self.trade_data_enabled && self.footprint.visible;
        let mut has_footprint_rows = false;

        self.for_each_bar_unit_index_with_units(
            x_axis,
            bar_units,
            min_x_unit,
            max_x_unit,
            |series_index, _bar_index, _series, _unit, point| {
                let (low, high) = if series_index == 0 && include_primary_ohlc_range {
                    (point.kline.low, point.kline.high)
                } else {
                    let close = point.kline.close;
                    (close, close)
                };

                primary_extrema.record_price(low);
                primary_extrema.record_price(high);

                if series_index == 0 && include_footprint_range {
                    for price in point.footprint.trades.keys() {
                        has_footprint_rows = true;
                        primary_extrema.record_price(*price);
                    }
                }
            },
        );

        for layer in &primary_overlay_layers {
            let output = &layer.presentation_output;
            let Some(spec) = self.indicator_output_spec(output) else {
                continue;
            };
            self.for_each_indicator_output_index_with_units(
                bar_units,
                x_axis,
                min_x_unit,
                max_x_unit,
                output,
                |series, source_index, _output_unit| {
                    if !self.indicator_input_available_at(
                        series,
                        source_index,
                        layer.required_input,
                    ) {
                        return;
                    }
                    match spec.value_type {
                        data::chart::kline::OutputValueType::Number => {
                            if matches!(layer.mark, MarkKind::Candle | MarkKind::Bar(_)) {
                                let Some(values) = self.indicator_layer_values_for_source_at(
                                    series,
                                    layer,
                                    source_index,
                                ) else {
                                    return;
                                };
                                match values {
                                    super::IndicatorBarValues::Plain { value } => {
                                        primary_extrema.record_scalar(value);
                                    }
                                    super::IndicatorBarValues::SignedOverlay {
                                        background_value,
                                        signed_value,
                                    } => {
                                        primary_extrema.record_scalar(background_value);
                                        primary_extrema.record_scalar(signed_value.abs());
                                    }
                                }
                            } else if let Some(value) = self
                                .indicator_raw_numeric_output_value_for_series_at(
                                    series,
                                    output,
                                    source_index,
                                )
                            {
                                primary_extrema.record_scalar(value);
                            }
                        }
                        data::chart::kline::OutputValueType::Ohlc => {
                            let Some(values) = self.indicator_ohlc_output_for_series_at(
                                series,
                                output,
                                source_index,
                            ) else {
                                return;
                            };
                            for value in [values.open, values.high, values.low, values.close]
                                .into_iter()
                                .flatten()
                            {
                                primary_extrema.record_scalar(value);
                            }
                        }
                        data::chart::kline::OutputValueType::Range => {
                            let Some(values) = self.indicator_range_output_for_series_at(
                                series,
                                output,
                                source_index,
                            ) else {
                                return;
                            };
                            if let Some(upper) = values.upper {
                                primary_extrema.record_scalar(upper);
                            }
                            if let Some(lower) = values.lower {
                                primary_extrema.record_scalar(lower);
                            }
                        }
                        data::chart::kline::OutputValueType::Boolean => {}
                    }
                },
            );
        }

        let min_primary_unit = match primary_extrema.min? {
            PrimaryDomainValue::Price(value) => value_scale.price_to_unit(value).0,
            PrimaryDomainValue::Scalar(value) => value_scale.value_to_unit(value).0,
        };
        let max_primary_unit = match primary_extrema.max? {
            PrimaryDomainValue::Price(value) => value_scale.price_to_unit(value).0,
            PrimaryDomainValue::Scalar(value) => value_scale.value_to_unit(value).0,
        };

        let footprint_tick_units = has_footprint_rows.then(|| {
            let zero_unit = value_scale.price_to_unit(Price::from_units(0));
            let tick_value = self
                .series
                .first()
                .map(|series| Price::from_units(series.footprint_tick_size().units))
                .unwrap_or_else(|| Price::from_units(0));
            let tick_unit = value_scale.price_to_unit(tick_value);
            i64::try_from(tick_unit.0.abs_diff(zero_unit.0))
                .unwrap_or(i64::MAX)
                .max(1)
        });
        let (padded_min, padded_max) = padded_primary_domain_units(
            min_primary_unit,
            max_primary_unit,
            primary_plot_height,
            footprint_tick_units,
        );

        Some((YUnit(padded_min), YUnit(padded_max)))
    }

    fn compute_indicator_domain(
        &self,
        bar_units: &[BarUnitWindow],
        x_axis: XAxis,
        min_x_unit: i64,
        max_x_unit: i64,
        output_layers: &[IndicatorOutputLayer],
        panel_value_precision: Option<PanelValuePrecision>,
        independent_scale: bool,
        scale_mode: PanelScaleMode,
    ) -> Option<(YUnit, YUnit)> {
        let mut min_value = f64::INFINITY;
        let mut max_value = f64::NEG_INFINITY;

        for layer in output_layers {
            let Some(spec) = self.indicator_output_spec(&layer.presentation_output) else {
                continue;
            };
            if spec.independent_scale != independent_scale {
                continue;
            }
            self.for_each_indicator_output_index_with_units(
                bar_units,
                x_axis,
                min_x_unit,
                max_x_unit,
                &layer.presentation_output,
                |series, source_index, _output_unit| {
                    let mut record_value = |value: f64| {
                        if !value.is_finite() {
                            return;
                        }
                        min_value = min_value.min(value);
                        max_value = max_value.max(value);
                    };

                    match spec.value_type {
                        data::chart::kline::OutputValueType::Number => {
                            if matches!(layer.mark, MarkKind::Candle | MarkKind::Bar(_)) {
                                let Some(values) = self.indicator_layer_values_for_source_at(
                                    series,
                                    layer,
                                    source_index,
                                ) else {
                                    return;
                                };
                                match values {
                                    super::IndicatorBarValues::Plain { value } => {
                                        record_value(value);
                                    }
                                    super::IndicatorBarValues::SignedOverlay {
                                        background_value,
                                        signed_value,
                                    } => {
                                        record_value(background_value);
                                        record_value(signed_value.abs());
                                    }
                                }
                            } else if let Some(value) = self
                                .indicator_raw_numeric_output_value_for_series_at(
                                    series,
                                    &layer.presentation_output,
                                    source_index,
                                )
                            {
                                record_value(value);
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
                                record_value(value);
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
                                record_value(upper);
                            }
                            if let Some(lower) = values.lower {
                                record_value(lower);
                            }
                        }
                        data::chart::kline::OutputValueType::Boolean => {}
                    }
                },
            );
        }

        if !min_value.is_finite() {
            return None;
        }

        let value_scale = PanelValueScale::new(panel_value_precision, self.base_ticker_info());
        let min_unit = value_scale.value_to_unit(min_value).0;
        let max_unit = value_scale.value_to_unit(max_value).0;
        let scale_mode = if independent_scale {
            PanelScaleMode::FitVisibleIncludeZero
        } else {
            scale_mode
        };
        Some(scale_mode.indicator_domain_units(min_unit, max_unit))
    }

    fn apply_cursor_interaction(
        &self,
        scene: &mut Scene,
        cursor_root_local: Option<Point>,
        primary_panel: usize,
        primary_plot: Rectangle,
        show_legend_values: bool,
        drag_active: bool,
        pan_active: bool,
    ) {
        let Some(local) = cursor_root_local else {
            return;
        };

        let hovered_control =
            Self::hit_panel_control(&scene.layout, &scene.chrome.panel_controls, local);
        let hovered_corner_control = Self::hit_corner_control(&scene.chrome.corner_controls, local);

        let mut ticker_hit = scene
            .chrome
            .ticker_legend
            .as_ref()
            .and_then(|legend| Self::hit_ticker_legend(&scene.layout, legend, local));

        if ticker_hit.is_some() {
            scene.chrome.ticker_legend = self.build_ticker_legend_layout(
                &scene.layout,
                primary_panel,
                self.legend_visibility.legend_values_always_visible(),
                true,
            );
            ticker_hit = scene
                .chrome
                .ticker_legend
                .as_ref()
                .and_then(|legend| Self::hit_ticker_legend(&scene.layout, legend, local));
        }

        let footprint_hit = scene
            .chrome
            .footprint_legend
            .as_ref()
            .and_then(|legend| Self::hit_footprint_legend(&scene.layout, legend, local));

        let study_hit = scene
            .chrome
            .study_legend
            .as_ref()
            .and_then(|legend| Self::hit_study_legend(&scene.layout, legend, local));

        let mut indicator_hit = scene
            .chrome
            .indicator_legend
            .as_ref()
            .and_then(|legend| Self::hit_indicator_legend(&scene.layout, legend, local));

        if indicator_hit.is_some() {
            scene.chrome.indicator_legend = self.build_indicator_legend_layout(
                &scene.layout,
                primary_panel,
                self.tooltip_rows_before_indicators(
                    scene.chrome.ticker_legend.as_ref(),
                    scene.chrome.footprint_legend.as_ref(),
                    scene.chrome.study_legend.as_ref(),
                    show_legend_values,
                ),
            );
            indicator_hit = scene
                .chrome
                .indicator_legend
                .as_ref()
                .and_then(|legend| Self::hit_indicator_legend(&scene.layout, legend, local));
        }

        scene.interaction.controls_visible_for_panel = hovered_control
            .map(|hit| hit.panel_index)
            .or_else(|| self.control_visibility_panel(&scene.layout, local));

        let zone = scene.layout.hit_test(local);

        let drawing_target =
            if self.drawings.allows_panning() && matches!(zone, LayoutHitZone::PanelPlot(_)) {
                self.hit_test_selected_drawing_handle(scene, local)
                    .map(|(id, kind)| CursorTarget::DrawingHandle { id, kind })
                    .or_else(|| {
                        self.hit_test_drawings(scene, local)
                            .map(CursorTarget::Drawing)
                    })
            } else {
                None
            };

        let target = if drag_active {
            if pan_active {
                CursorTarget::ActivePan
            } else {
                CursorTarget::ActiveDrag
            }
        } else if let Some(hit) = ticker_hit {
            CursorTarget::Legend(LegendTarget::from(hit))
        } else if let Some(hit) = footprint_hit {
            CursorTarget::Legend(LegendTarget::from(hit))
        } else if let Some(hit) = study_hit {
            CursorTarget::Legend(LegendTarget::from(hit))
        } else if let Some(hit) = indicator_hit {
            CursorTarget::Legend(LegendTarget::from(hit))
        } else if let Some(control) = hovered_corner_control {
            CursorTarget::CornerControl(control.kind)
        } else if let Some(control) = hovered_control {
            CursorTarget::PanelControl {
                panel_index: control.panel_index,
                kind: control.kind,
            }
        } else if let Some(target) = drawing_target {
            target
        } else {
            match zone {
                LayoutHitZone::PanelPlot(panel_index) => CursorTarget::Plot(panel_index),
                LayoutHitZone::Splitter(index) => CursorTarget::Splitter(index),
                LayoutHitZone::YAxis(index) => CursorTarget::YAxis(index),
                LayoutHitZone::PanelXAxis(index) => CursorTarget::PanelXAxis(index),
                LayoutHitZone::BottomXAxis => CursorTarget::BottomXAxis,
                LayoutHitZone::Outside => CursorTarget::Outside,
            }
        };
        scene.interaction.target = target;

        if !matches!(zone, LayoutHitZone::PanelPlot(_)) {
            return;
        }

        let Some(plot_local) = scene.layout.plot_local_point(local) else {
            return;
        };

        let x_plot = plot_local.x.clamp(0.0, primary_plot.width.max(1.0));
        let snapped_x_unit = scene.unit_from_plot_x(x_plot);
        let snapped_x_plot = scene
            .map_x_plot(snapped_x_unit)
            .clamp(0.0, scene.x_axis_plot_width().max(1.0));

        let LayoutHitZone::PanelPlot(panel_index) = zone else {
            return;
        };
        let Some(panel) = scene.layout.panel(panel_index) else {
            return;
        };

        let value_scale = self.panel_value_scale(panel_index);
        let y_in_panel = (plot_local.y - panel.plot.y).clamp(0.0, panel.plot.height.max(1.0));

        let (y_primary_unit, y_indicator_unit, snapped_y_plot) = match panel.kind {
            KlinePanelKind::PrimaryChart => {
                let value_ratio = scene.primary_value_ratio_at_plot_y(y_in_panel);
                let uses_display_transform = scene
                    .primary
                    .mode
                    .uses_display_transform(scene.can_use_log_primary_scale());

                let (y_primary_unit, snapped_y_plot) = if uses_display_transform {
                    let (min_display, max_display) = scene.primary_domain_display_values();
                    let y_display_value = min_display + ((max_display - min_display) * value_ratio);
                    let y_primary_unit = value_scale
                        .value_to_unit_f32(scene.primary_display_to_value(y_display_value));
                    let y_primary_value = value_scale.unit_to_value_f32(y_primary_unit);
                    let y_plot = scene
                        .map_primary_plot_with_anchor_f32(y_primary_value, scene.primary.anchor)
                        .clamp(panel.plot.y, panel.plot.y + panel.plot.height);

                    (y_primary_unit, y_plot)
                } else {
                    let y_primary_unit = Scene::lerp_unit(
                        scene.primary.domain.active().min(),
                        scene.primary.domain.active().max(),
                        value_ratio,
                    );
                    let y_plot = scene
                        .map_primary_plot_unit(y_primary_unit)
                        .clamp(panel.plot.y, panel.plot.y + panel.plot.height);

                    (y_primary_unit, y_plot)
                };

                (Some(y_primary_unit), None, snapped_y_plot)
            }
            KlinePanelKind::Indicator => {
                if self.indicator_is_unavailable(self.indicator_panel_value_id(panel_index)) {
                    (None, None, panel.plot.y + y_in_panel)
                } else {
                    let indicator_ratio = 1.0 - (y_in_panel / panel.plot.height.max(1.0));
                    let y_indicator_unit = scene
                        .indicator_panel_config(panel_index)
                        .map(|indicator| {
                            Scene::lerp_unit(
                                indicator.domain.active().min(),
                                indicator.domain.active().max(),
                                indicator_ratio,
                            )
                        })
                        .unwrap_or_else(|| Scene::lerp_unit(YUnit(0), YUnit(1), indicator_ratio));
                    let snapped_y_plot = scene
                        .map_indicator_plot_unit(panel_index, y_indicator_unit)
                        .map(|y| y.clamp(panel.plot.y, panel.plot.y + panel.plot.height))
                        .unwrap_or(panel.plot.y + y_in_panel);

                    (None, Some(y_indicator_unit), snapped_y_plot)
                }
            }
        };

        scene.interaction.cursor = Some(CursorInfo {
            x_unit: snapped_x_unit,
            panel_index,
            y_primary_unit,
            y_indicator_unit,
            x_plot: snapped_x_plot,
            y_plot: snapped_y_plot,
        });
    }

    pub(super) fn compute_scene_uncached(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        drag_active: bool,
        pan_active: bool,
    ) -> Option<Scene> {
        let panel_layout = PanelLayoutTree::from_layout(layout, &self.composition.panels)?;
        let base = self.compute_scene_base(panel_layout)?;
        self.build_scene_from_base(&base, layout.bounds(), cursor, drag_active, pan_active)
    }

    pub(super) fn ensure_scene_base(&self, state: &mut State, layout: Layout<'_>) {
        let Some(panel_layout) = PanelLayoutTree::from_layout(layout, &self.composition.panels)
        else {
            return;
        };

        let _ = state
            .scene_cache
            .get_or_build(self.version, panel_layout, |panel_layout| {
                self.compute_scene_base(panel_layout)
            });
    }

    pub(super) fn cached_scene(
        &self,
        state: &State,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) -> Option<Scene> {
        let root_bounds = layout.bounds();
        let panel_layout = PanelLayoutTree::from_layout(layout, &self.composition.panels)?;
        let drag_active = !matches!(state.drag_mode, DragMode::None);
        let pan_active = matches!(state.drag_mode, DragMode::Pan { .. });
        let Some(base) = state.scene_cache.get(self.version, &panel_layout) else {
            return self.compute_scene_uncached(layout, cursor, drag_active, pan_active);
        };

        self.build_scene_from_base(base, root_bounds, cursor, drag_active, pan_active)
    }

    fn compute_scene_base(&self, panel_layout: PanelLayoutTree) -> Option<SceneBase> {
        let primary_panel = panel_layout
            .panels
            .iter()
            .position(|panel| panel.kind == KlinePanelKind::PrimaryChart)?;
        let primary_mark = self.resolved_panel_mark(primary_panel, KlinePanelKind::PrimaryChart);
        let primary_scale_mode = self.resolved_panel_scale_mode(primary_panel);
        let primary_plot = panel_layout.panel(primary_panel)?.plot;

        let x_axis_plot_width = panel_layout.primary_plot_width()?;
        let horizontal_world = self.horizontal_world;
        let autoscale_plot_width = (x_axis_plot_width + self.autoscale_plot_width_offset).max(1.0);

        let x_axis = self.resolve_x_axis()?;
        let (data_min_x, data_max_x) = self.data_x_bounds(x_axis)?;
        let (domain_min_extension, domain_max_extension) =
            self.indicator_x_domain_expansion(x_axis);
        let data_bounds = (
            data_min_x.saturating_add(domain_min_extension),
            data_max_x.saturating_add(domain_max_extension),
        );
        let (min_x_unit, max_x_unit, right_x_unit) =
            self.compute_x_window(data_bounds, x_axis_plot_width, horizontal_world);
        let (autoscale_min_x_unit, autoscale_max_x_unit, _) =
            self.compute_x_window(data_bounds, autoscale_plot_width, horizontal_world);
        let (source_min_x_unit, source_max_x_unit) = source_bar_window_for_offsets(
            x_axis,
            autoscale_min_x_unit,
            autoscale_max_x_unit,
            &self.indicator_source_offsets(),
        );
        let bar_units: Arc<[BarUnitWindow]> =
            Arc::from(self.build_bar_units(x_axis, source_min_x_unit, source_max_x_unit));
        let x_world = horizontal_world.axis_at_right_unit(right_x_unit);
        let fractional_x_offset = right_x_unit - max_x_unit as f32;
        let primary_value_precision = self.panel_value_precision(primary_panel);
        let (min_primary_unit, max_primary_unit) = self.compute_primary_domain(
            bar_units.as_ref(),
            x_axis,
            autoscale_min_x_unit,
            autoscale_max_x_unit,
            primary_value_precision,
            primary_mark,
            primary_plot.height,
        )?;
        let primary_value_scale = self.panel_value_scale(primary_panel);

        let indicator_panels: Vec<IndicatorPanelScene> = panel_layout
            .panels
            .iter()
            .enumerate()
            .filter_map(|(panel_index, panel)| {
                if panel.kind != KlinePanelKind::Indicator {
                    return None;
                }

                let scale_mode = self.resolved_panel_scale_mode(panel_index);

                let value_id = self.indicator_panel_value_id(panel_index);
                let output_layers = self
                    .panel_output_layers(panel_index)
                    .into_iter()
                    .filter(|layer| layer.visible && layer.purpose.is_plot())
                    .collect::<Vec<_>>();
                let value_precision = self.panel_value_precision(panel_index);
                let value_scale = self.panel_value_scale(panel_index);
                let independent_domain_units = self.compute_indicator_domain(
                    bar_units.as_ref(),
                    x_axis,
                    autoscale_min_x_unit,
                    autoscale_max_x_unit,
                    &output_layers,
                    value_precision,
                    true,
                    scale_mode,
                );
                let independent_domain = independent_domain_units
                    .map(|(min_unit, max_unit)| YUnitRange::new(min_unit, max_unit));
                let (min_unit, max_unit) = self
                    .compute_indicator_domain(
                        bar_units.as_ref(),
                        x_axis,
                        autoscale_min_x_unit,
                        autoscale_max_x_unit,
                        &output_layers,
                        value_precision,
                        false,
                        scale_mode,
                    )
                    .or(independent_domain_units)
                    .unwrap_or((YUnit(0), YUnit(1)));
                let domain = PanelDomainState::new(YUnitRange::new(min_unit, max_unit));

                Some(IndicatorPanelScene {
                    panel_index,
                    value_id,
                    value_scale,
                    domain,
                    independent_domain,
                    presentation: self
                        .composition
                        .panels
                        .get(panel_index)
                        .map(|panel| panel.presentation)
                        .unwrap_or_default(),
                })
            })
            .collect();

        let PrimaryPercentScaleState {
            primary_scale_anchor,
            series_percent_anchors,
            primary_domain_display_override,
        } = self.compute_primary_percent_scale_state(
            bar_units.as_ref(),
            x_axis,
            autoscale_min_x_unit,
            autoscale_max_x_unit,
            primary_mark,
            primary_scale_mode,
        )?;

        let panel_controls = self.build_panel_control_hits(&panel_layout, primary_panel);
        let corner_controls = self.build_corner_control_hits(&panel_layout, primary_scale_mode);
        let primary_value_step = primary_value_scale.quantization_step_f32();
        let primary_value_decimals = primary_value_precision
            .and_then(|precision| precision.decimals(self.base_ticker_info()));
        Some(SceneBase {
            layout: panel_layout,
            x: XViewport {
                axis: x_axis,
                world: x_world,
                fractional_offset: fractional_x_offset,
                min_unit: min_x_unit,
                max_unit: max_x_unit,
            },
            bar_units,
            primary: PrimaryScale {
                panel_index: primary_panel,
                domain: PanelDomainState::new(YUnitRange::new(min_primary_unit, max_primary_unit)),
                display_domain_override: primary_domain_display_override,
                value_scale: primary_value_scale,
                mark: primary_mark,
                mode: primary_scale_mode,
                inverted: self.primary_scale_inverted,
                anchor: primary_scale_anchor,
                value_step: primary_value_step,
                value_decimals: primary_value_decimals,
                series_percent_anchors,
            },
            indicator_panels,
            panel_controls,
            corner_controls,
        })
    }

    /// Return the x-domain expansion required by displaced outputs. Time and
    /// tick axes use opposite bar-index directions, so the same positive
    /// displacement belongs on different sides of the domain.
    fn indicator_x_domain_expansion(&self, x_axis: XAxis) -> (i64, i64) {
        let output_offset = self
            .indicator_instances
            .instances()
            .iter()
            .filter_map(|instance| {
                instance
                    .kind
                    .output_channels()
                    .iter()
                    .map(|output| output.offset_bars)
                    .filter(|offset| *offset > 0)
                    .max()
            })
            .map(i64::from)
            .max()
            .unwrap_or_default();
        match x_axis {
            XAxis::Time { .. } => (0, output_offset),
            XAxis::Tick { .. } => (output_offset.saturating_neg(), 0),
        }
    }

    fn build_scene_from_base(
        &self,
        base: &SceneBase,
        root_bounds: Rectangle,
        cursor: mouse::Cursor,
        drag_active: bool,
        pan_active: bool,
    ) -> Option<Scene> {
        let primary_panel = base.primary.panel_index;
        let primary_plot = base.layout.panel(primary_panel)?.plot;
        let cursor_root_local = cursor.position_in(root_bounds);
        let hovering_plot = cursor_root_local
            .map(|local| matches!(base.layout.hit_test(local), LayoutHitZone::PanelPlot(_)))
            .unwrap_or(false);
        let show_legend_values = self.legend_visibility.legend_values_always_visible()
            || (!drag_active && hovering_plot);
        let show_legends = self.legend_visibility.legend_always_visible() || hovering_plot;
        let keep_ticker_names = self.series.len() > 1;

        let mut scene = Scene::from_base(base);
        self.apply_panel_viewports(&mut scene);
        scene.chrome.ticker_legend = if show_legends || keep_ticker_names {
            self.build_ticker_legend_layout(&base.layout, primary_panel, show_legend_values, false)
        } else {
            None
        };

        if self.trade_data_enabled && show_legends {
            scene.chrome.footprint_legend = self.build_footprint_legend_layout(
                &base.layout,
                primary_panel,
                self.tooltip_rows_before_studies(
                    scene.chrome.ticker_legend.as_ref(),
                    None,
                    show_legend_values,
                ),
                self.footprint.cluster_kind,
            );
        }
        if show_legends {
            scene.chrome.study_legend = self.build_study_legend_layout(
                &base.layout,
                primary_panel,
                self.tooltip_rows_before_studies(
                    scene.chrome.ticker_legend.as_ref(),
                    scene.chrome.footprint_legend.as_ref(),
                    show_legend_values,
                ),
                self.studies_legend_expanded,
            );
            scene.chrome.indicator_legend = self.build_indicator_legend_layout(
                &base.layout,
                primary_panel,
                self.tooltip_rows_before_indicators(
                    scene.chrome.ticker_legend.as_ref(),
                    scene.chrome.footprint_legend.as_ref(),
                    scene.chrome.study_legend.as_ref(),
                    show_legend_values,
                ),
            );
        }

        self.apply_cursor_interaction(
            &mut scene,
            cursor_root_local,
            primary_panel,
            primary_plot,
            show_legend_values,
            drag_active,
            pan_active,
        );
        Some(scene)
    }

    pub(super) fn tick_time_for_unit(&self, x_axis: XAxis, unit: i64) -> Option<UnixMs> {
        let from_latest = usize::try_from(x_axis.tick_from_unit(unit)?.as_u64()).ok()?;
        let series = self.series.first()?;
        let index = series
            .point_count()
            .checked_sub(from_latest.checked_add(1)?)?;
        series.time_at_index(index)
    }

    pub(super) fn format_x_label(&self, x_axis: XAxis, unit: i64, step_units: i64) -> String {
        match x_axis {
            XAxis::Time { .. } => x_axis.time_from_unit(unit).map_or_else(
                || unit.to_string(),
                |ts| {
                    let ts_ms = ts.as_u64();
                    let step_ms = x_axis.step_ms(step_units);

                    self.timezone
                        .format_with_kind(
                            ts_ms as i64,
                            data::config::timezone::TimeLabelKind::AxisStepMs { step_ms },
                        )
                        .unwrap_or_else(|| ts_ms.to_string())
                },
            ),
            XAxis::Tick { .. } => self.tick_time_for_unit(x_axis, unit).map_or_else(
                || unit.to_string(),
                |time| {
                    self.timezone
                        .format_with_kind(
                            time.as_u64() as i64,
                            data::config::timezone::TimeLabelKind::Axis {
                                timeframe: Timeframe::MS100,
                            },
                        )
                        .unwrap_or_else(|| time.as_u64().to_string())
                },
            ),
        }
    }

    pub(super) fn format_x_crosshair_label(&self, x_axis: XAxis, unit: i64) -> String {
        match x_axis {
            XAxis::Time { .. } => x_axis.time_from_unit(unit).map_or_else(
                || unit.to_string(),
                |ts| {
                    self.timezone
                        .format_with_kind(
                            ts.as_u64() as i64,
                            data::config::timezone::TimeLabelKind::Crosshair { show_millis: false },
                        )
                        .unwrap_or_else(|| ts.as_u64().to_string())
                },
            ),
            XAxis::Tick { .. } => self
                .tick_time_for_unit(x_axis, unit)
                .and_then(|time| {
                    self.timezone.format_with_kind(
                        time.as_u64() as i64,
                        data::config::timezone::TimeLabelKind::Crosshair { show_millis: true },
                    )
                })
                .unwrap_or_else(|| self.format_x_label(x_axis, unit, 1)),
        }
    }
}

fn source_bar_window_for_offsets(
    x_axis: XAxis,
    min_x_unit: i64,
    max_x_unit: i64,
    offsets: &[i64],
) -> (i64, i64) {
    let mut source_min = min_x_unit;
    let mut source_max = max_x_unit;

    for &offset in offsets {
        let (offset_min, offset_max) = match x_axis {
            XAxis::Time { .. } => (
                min_x_unit.saturating_sub(offset),
                max_x_unit.saturating_sub(offset),
            ),
            XAxis::Tick { .. } => (
                min_x_unit.saturating_add(offset),
                max_x_unit.saturating_add(offset),
            ),
        };
        source_min = source_min.min(offset_min);
        source_max = source_max.max(offset_max);
    }

    (source_min.saturating_sub(1), source_max.saturating_add(1))
}

pub(super) fn shifted_output_unit(axis: XAxis, source_unit: i64, offset_bars: i64) -> i64 {
    match axis {
        XAxis::Time { .. } => source_unit.saturating_add(offset_bars),
        XAxis::Tick { .. } => source_unit.saturating_sub(offset_bars),
    }
}

fn indicator_output_source_range(
    units: &[i64],
    axis: XAxis,
    min_unit: i64,
    max_unit: i64,
    offset: i64,
) -> (usize, usize) {
    (
        units.partition_point(|&unit| shifted_output_unit(axis, unit, offset) < min_unit),
        units.partition_point(|&unit| shifted_output_unit(axis, unit, offset) <= max_unit),
    )
}

fn first_unit_index(units: &[i64], mut matches: impl FnMut(i64) -> bool) -> usize {
    let mut low = 0;
    let mut high = units.len();
    while low < high {
        let middle = low + (high - low) / 2;
        if matches(units[middle]) {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    low
}

fn last_index_at_or_before(
    len: usize,
    target_unit: i64,
    mut unit_at: impl FnMut(usize) -> i64,
) -> Option<usize> {
    let mut low = 0;
    let mut high = len;
    while low < high {
        let middle = low + (high - low) / 2;
        if unit_at(middle) <= target_unit {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    low.checked_sub(1)
}

pub(super) fn bar_index_range_for_units(
    units: &[i64],
    min_x_unit: i64,
    max_x_unit: i64,
) -> (usize, usize) {
    if units.is_empty() {
        return (0, 0);
    }

    (
        first_unit_index(units, |unit| unit >= min_x_unit),
        last_index_at_or_before(units.len(), max_x_unit, |index| units[index])
            .map_or(0, |index| index.saturating_add(1)),
    )
}

fn time_bucket_end_for_unit(axis: XAxis, target_unit: i64) -> Option<UnixMs> {
    let XAxis::Time { timeframe, .. } = axis else {
        return None;
    };
    let bucket_start = axis.time_from_unit(target_unit)?.floor_to(timeframe);
    Some(bucket_start.saturating_add(timeframe.to_milliseconds().max(1).saturating_sub(1)))
}

fn time_bucket_start_for_unit(axis: XAxis, target_unit: i64) -> Option<UnixMs> {
    let XAxis::Time { timeframe, .. } = axis else {
        return None;
    };
    Some(axis.time_from_unit(target_unit)?.floor_to(timeframe))
}

fn padded_primary_domain_units(
    min_unit: i64,
    max_unit: i64,
    plot_height: f32,
    footprint_tick_units: Option<i64>,
) -> (i64, i64) {
    let mut padded_min = min_unit.min(max_unit);
    let mut padded_max = min_unit.max(max_unit);

    if let Some(tick_units) = footprint_tick_units {
        let half_tick_units = tick_units.saturating_add(1) / 2;
        padded_min = padded_min.saturating_sub(half_tick_units);
        padded_max = padded_max.saturating_add(half_tick_units);
    }

    let span = padded_min.abs_diff(padded_max);
    let base_padding = if span == 0 {
        padded_max.unsigned_abs().max(1).saturating_add(199) / 200
    } else {
        span.saturating_mul(5).saturating_add(99) / 100
    };
    let base_padding = i64::try_from(base_padding).unwrap_or(i64::MAX).max(1);

    let outer_padding = if footprint_tick_units.is_some() && plot_height > f32::EPSILON {
        let provisional_span = span.saturating_add((base_padding as u64).saturating_mul(2));
        (4.0 * provisional_span as f32 / plot_height).ceil() as i64
    } else {
        0
    };
    let padding = base_padding.saturating_add(outer_padding);

    padded_min = padded_min.saturating_sub(padding);
    padded_max = padded_max.saturating_add(padding);
    (padded_min, padded_max)
}
