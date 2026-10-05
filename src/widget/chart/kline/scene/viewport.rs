use super::{ChartCoordinates, Scene};
use crate::widget::chart::kline::composition::{PanelId, PanelScaleMode};
use crate::widget::chart::kline::layout::KlinePanelKind;
use crate::window::PixelRatio;
use data::chart::kline::viewport::{
    PanelYValueRange, PanelYViewportLimits, PrimaryViewportState, Y_UNIT_STEP_FALLBACK, YUnitRange,
    ZOOM_SENSITIVITY, autoscale_span_limits,
};

pub(crate) struct PanelViewportView<'a> {
    pub(crate) coordinates: ChartCoordinates<'a>,
    pub(crate) pixel_ratio: PixelRatio,
    pub(crate) primary_viewport: PrimaryViewportState,
    pub(crate) indicator_y_ranges: &'a [(PanelId, YUnitRange)],
}

impl PanelViewportView<'_> {
    fn panel_y_range(&self, panel_id: PanelId) -> Option<YUnitRange> {
        self.indicator_y_ranges
            .iter()
            .find(|(id, _)| *id == panel_id)
            .map(|(_, viewport)| *viewport)
    }

    fn panel_y_range_for_index(&self, panel_index: usize) -> Option<YUnitRange> {
        self.coordinates
            .panel_id(panel_index)
            .and_then(|panel_id| self.panel_y_range(panel_id))
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
                    self.coordinates
                        .panel_value_scale(scene.primary.panel_index)
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
                let value_scale = self.coordinates.panel_value_scale(panel_index);
                let bounds = self
                    .coordinates
                    .panel_value_precision(panel_index)
                    .and_then(|precision| {
                        precision.value_bounds(self.coordinates.base_ticker_info())
                    })
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
        let scale = self.coordinates.panel_value_scale(panel_index);
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
                let scale = self.coordinates.panel_value_scale(panel_index);
                let (min_value, max_value) = scale.values(range);
                let viewport = limits.apply(PanelYValueRange::normalized(min_value, max_value)?)?;
                self.value_range_to_unit_range(panel_index, viewport)
            }
        }
    }

    pub(crate) fn apply_panel_viewports(&self, scene: &mut Scene) {
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

    pub(crate) fn reconciled_y_ranges(&self, scene: &Scene) -> Vec<(PanelId, YUnitRange)> {
        let mut changes = Vec::new();

        if !scene.primary.mode.uses_percent_base()
            && let Some(range) = self.primary_viewport.range()
            && let Some(effective) =
                self.resolve_panel_viewport(scene, scene.primary.panel_index, range)
            && effective != range
            && let Some(panel_id) = self.coordinates.panel_id(scene.primary.panel_index)
        {
            changes.push((panel_id, effective));
        }

        for (panel_id, range) in self.indicator_y_ranges.iter().copied() {
            let Some(panel_index) = self.coordinates.panel_index_for_id(panel_id) else {
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

    pub(crate) fn panel_viewport_after_y_zoom(
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

    pub(crate) fn panel_viewport_after_primary_y_pan(
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
}
