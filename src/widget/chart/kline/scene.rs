use super::chrome::{PanelControlHit, TickerLegendHit, TickerLegendIconKind, TickerLegendLayout};
use super::layout::{LayoutHitZone, PanelLayoutTree};
use super::{
    HorizontalScale, KlinePanelKind, KlineSeriesLike, KlineWidget, MAX_BAR_SPACING_PX,
    MIN_BAR_SPACING_PX, SCALE_STEP_PCT,
};
use crate::widget::chart::kline::composition::{MarkKind, PanelScaleMode};

use exchange::{Kline, Timeframe, UnixMs};

use iced::advanced::Layout;
use iced::{Rectangle, mouse};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct TickIndex(u64);

impl TickIndex {
    const ZERO: Self = Self(0);

    #[inline]
    fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum XAxis {
    Time {
        timeframe: Timeframe,
        anchor: UnixMs,
    },
    Tick {
        anchor: TickIndex,
    },
}

impl XAxis {
    #[inline]
    fn unit_from_time(self, value: UnixMs) -> i64 {
        match self {
            Self::Time { timeframe, anchor } => {
                let aligned = value.floor_to(timeframe).as_u64() as i128;
                let anchor = anchor.as_u64() as i128;
                let step = timeframe.to_milliseconds().max(1) as i128;
                ((aligned - anchor) / step).clamp(i64::MIN as i128, i64::MAX as i128) as i64
            }
            Self::Tick { .. } => 0,
        }
    }

    #[inline]
    fn unit_from_tick(self, value: TickIndex) -> i64 {
        match self {
            Self::Tick { anchor } => {
                let anchor = anchor.as_u64() as i128;
                let index = value.as_u64() as i128;
                (anchor - index).clamp(i64::MIN as i128, i64::MAX as i128) as i64
            }
            Self::Time { .. } => 0,
        }
    }

    #[inline]
    fn time_from_unit(self, unit: i64) -> Option<UnixMs> {
        match self {
            Self::Time { timeframe, anchor } => {
                let step = i64::try_from(timeframe.to_milliseconds()).unwrap_or(i64::MAX);
                Some(anchor.saturating_add_signed(unit.saturating_mul(step)))
            }
            Self::Tick { .. } => None,
        }
    }

    #[inline]
    fn tick_from_unit(self, unit: i64) -> Option<TickIndex> {
        match self {
            Self::Tick { anchor } => {
                let value = (anchor.as_u64() as i128) - (unit as i128);
                if value < 0 {
                    None
                } else {
                    Some(TickIndex(value as u64))
                }
            }
            Self::Time { .. } => None,
        }
    }

    #[inline]
    fn step_ms(self, step_units: i64) -> u64 {
        match self {
            Self::Time { timeframe, .. } => {
                let step = step_units.max(1) as u64;
                timeframe.to_milliseconds().max(1).saturating_mul(step)
            }
            Self::Tick { .. } => 1,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct CursorInfo {
    pub(super) x_unit: i64,
    pub(super) panel_index: usize,
    pub(super) y_primary_value: Option<f32>,
    pub(super) y_indicator_value: Option<f32>,
    pub(super) x_plot: f32,
    pub(super) y_plot: f32,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct IndicatorPanelScene {
    pub(super) panel_index: usize,
    pub(super) mark: MarkKind,
    pub(super) min_value: f32,
    pub(super) max_value: f32,
}

#[derive(Debug, Clone)]
pub(super) struct Scene {
    pub(super) layout: PanelLayoutTree,
    pub(super) x_axis: XAxis,
    pub(super) bar_spacing_px: f32,
    pub(super) min_x_unit: i64,
    pub(super) max_x_unit: i64,
    pub(super) min_primary_value: f32,
    pub(super) max_primary_value: f32,
    pub(super) primary_panel: usize,
    pub(super) primary_mark: MarkKind,
    pub(super) primary_scale_mode: PanelScaleMode,
    pub(super) primary_scale_anchor: Option<f32>,
    pub(super) series_percent_anchors: Vec<Option<f32>>,
    pub(super) indicator_panels: Vec<IndicatorPanelScene>,
    pub(super) panel_controls: Vec<PanelControlHit>,
    pub(super) ticker_legend: Option<TickerLegendLayout>,
    pub(super) controls_visible_for_panel: Option<usize>,
    pub(super) hovered_control: Option<PanelControlHit>,
    pub(super) hovering_ticker_legend: bool,
    pub(super) hovered_ticker_row: Option<usize>,
    pub(super) hovered_ticker_icon: Option<(usize, TickerLegendIconKind)>,
    pub(super) cursor: Option<CursorInfo>,
}

impl Scene {
    pub(super) fn plot_rect(&self) -> Rectangle {
        self.layout.regions.plot
    }

    pub(super) fn primary_plot(&self) -> &Rectangle {
        &self
            .layout
            .panel(self.primary_panel)
            .expect("primary panel should exist")
            .plot
    }

    fn indicator_panel_config(&self, panel_index: usize) -> Option<&IndicatorPanelScene> {
        self.indicator_panels
            .iter()
            .find(|indicator| indicator.panel_index == panel_index)
    }

    fn indicator_plot(&self, panel_index: usize) -> Option<&Rectangle> {
        self.layout.panel(panel_index).map(|panel| &panel.plot)
    }

    pub(super) fn indicator_panel_bottom(&self, panel_index: usize) -> Option<f32> {
        self.indicator_plot(panel_index)
            .map(|rect| rect.y + rect.height)
    }

    pub(super) fn map_x_plot(&self, x_unit: i64) -> f32 {
        let steps_from_right = (self.max_x_unit - x_unit) as f32;
        self.primary_plot().width - (steps_from_right * self.bar_spacing_px)
    }

    fn unit_from_plot_x(&self, x_plot: f32) -> i64 {
        let width = self.primary_plot().width.max(1.0);
        let clamped_x = x_plot.clamp(0.0, width);
        let spacing = self.bar_spacing_px.max(1e-3);
        let steps_from_right = ((width - clamped_x) / spacing).round() as i64;

        self.max_x_unit
            .saturating_sub(steps_from_right)
            .clamp(self.min_x_unit, self.max_x_unit)
    }

    fn primary_value_to_display_with_anchor(&self, value: f32, anchor: Option<f32>) -> f32 {
        if self.can_use_log_primary_scale()
            && matches!(self.primary_scale_mode, PanelScaleMode::Logarithmic)
        {
            value.max(f32::MIN_POSITIVE).log10()
        } else {
            match (self.primary_scale_mode, anchor) {
                (PanelScaleMode::PercentFromBase, Some(base)) if base.abs() > f32::EPSILON => {
                    ((value / base) - 1.0) * 100.0
                }
                _ => value,
            }
        }
    }

    pub(super) fn primary_domain_display_values(&self) -> (f32, f32) {
        let min_primary_value = self.min_primary_value;
        let max_primary_value = self.max_primary_value;

        if self.can_use_log_primary_scale()
            && matches!(self.primary_scale_mode, PanelScaleMode::Logarithmic)
        {
            (
                min_primary_value.max(f32::MIN_POSITIVE).log10(),
                max_primary_value.max(f32::MIN_POSITIVE).log10(),
            )
        } else {
            match (self.primary_scale_mode, self.primary_scale_anchor) {
                (PanelScaleMode::PercentFromBase, Some(base)) if base.abs() > f32::EPSILON => (
                    ((min_primary_value / base) - 1.0) * 100.0,
                    ((max_primary_value / base) - 1.0) * 100.0,
                ),
                _ => (min_primary_value, max_primary_value),
            }
        }
    }

    fn primary_to_display_value(&self, value: f32) -> f32 {
        if self.can_use_log_primary_scale()
            && matches!(self.primary_scale_mode, PanelScaleMode::Logarithmic)
        {
            value.max(f32::MIN_POSITIVE).log10()
        } else {
            match (self.primary_scale_mode, self.primary_scale_anchor) {
                (PanelScaleMode::PercentFromBase, Some(base)) if base.abs() > f32::EPSILON => {
                    ((value / base) - 1.0) * 100.0
                }
                _ => value,
            }
        }
    }

    pub(super) fn primary_display_to_value(&self, display_value: f32) -> f32 {
        if self.can_use_log_primary_scale()
            && matches!(self.primary_scale_mode, PanelScaleMode::Logarithmic)
        {
            10_f32.powf(display_value).max(f32::MIN_POSITIVE)
        } else {
            match (self.primary_scale_mode, self.primary_scale_anchor) {
                (PanelScaleMode::PercentFromBase, Some(base)) if base.abs() > f32::EPSILON => {
                    base * (1.0 + display_value / 100.0)
                }
                _ => display_value,
            }
        }
    }

    fn can_use_log_primary_scale(&self) -> bool {
        self.min_primary_value > f32::EPSILON && self.max_primary_value > f32::EPSILON
    }

    pub(super) fn format_primary_axis_label(
        &self,
        display_value: f32,
        display_step: f32,
    ) -> String {
        match self.primary_scale_mode {
            PanelScaleMode::PercentFromBase => format!("{display_value:.2}%"),
            PanelScaleMode::Logarithmic if self.can_use_log_primary_scale() => {
                let value = self.primary_display_to_value(display_value);
                let next_value =
                    self.primary_display_to_value(display_value + display_step.abs().max(1e-3));
                let value_step = (next_value - value).abs().max(1e-6);
                super::super::format_value(value, value_step)
            }
            _ => super::super::format_value(display_value, display_step),
        }
    }

    pub(super) fn format_primary_cursor_label(&self, raw_value: f32) -> String {
        match self.primary_scale_mode {
            PanelScaleMode::PercentFromBase => {
                format!("{:.2}%", self.primary_to_display_value(raw_value))
            }
            _ => super::super::format_value(raw_value, 0.01),
        }
    }

    pub(super) fn map_primary_plot_with_anchor(&self, value: f32, anchor: Option<f32>) -> f32 {
        let (min_display, max_display) = self.primary_domain_display_values();
        let range = (max_display - min_display).abs().max(1e-6);
        let display_value = if matches!(self.primary_scale_mode, PanelScaleMode::PercentFromBase) {
            self.primary_value_to_display_with_anchor(value, anchor)
        } else {
            self.primary_to_display_value(value)
        };
        let ratio = ((display_value - min_display) / range).clamp(0.0, 1.0);
        let panel = self.primary_plot();
        panel.y + (1.0 - ratio) * panel.height
    }

    pub(super) fn map_indicator_plot(
        &self,
        panel_index: usize,
        indicator_value: f32,
    ) -> Option<f32> {
        let panel = self.indicator_plot(panel_index)?;
        let (min_value, max_value) = self
            .indicator_panel_config(panel_index)
            .map(|indicator| (indicator.min_value, indicator.max_value))
            .unwrap_or((0.0, 1.0));
        let range = (max_value - min_value).abs().max(1e-6);
        let ratio = ((indicator_value - min_value) / range).clamp(0.0, 1.0);
        Some(panel.y + (1.0 - ratio) * panel.height)
    }
}

impl<'a, S> KlineWidget<'a, S>
where
    S: KlineSeriesLike,
{
    pub(super) fn bar_at_or_before_unit<'b>(
        &self,
        series: &'b S,
        x_axis: XAxis,
        target_unit: i64,
    ) -> Option<&'b Kline> {
        let mut best: Option<(i64, &'b Kline)> = None;

        match x_axis {
            XAxis::Time { .. } => {
                for bar in series.bars() {
                    let unit = x_axis.unit_from_time(bar.time);
                    if unit == target_unit {
                        return Some(bar);
                    }

                    if unit <= target_unit
                        && best.map(|(best_unit, _)| unit > best_unit).unwrap_or(true)
                    {
                        best = Some((unit, bar));
                    }
                }
            }
            XAxis::Tick { .. } => {
                let len = series.bars().len();
                for (index, bar) in series.bars().iter().enumerate() {
                    let from_latest = len.saturating_sub(1).saturating_sub(index) as u64;
                    let unit = x_axis.unit_from_tick(TickIndex(from_latest));

                    if unit == target_unit {
                        return Some(bar);
                    }

                    if unit <= target_unit
                        && best.map(|(best_unit, _)| unit > best_unit).unwrap_or(true)
                    {
                        best = Some((unit, bar));
                    }
                }
            }
        }

        best.map(|(_, bar)| bar)
    }

    fn compute_primary_scale_anchor(
        &self,
        x_axis: XAxis,
        min_x_unit: i64,
        max_x_unit: i64,
    ) -> Option<f32> {
        let mut first_unit = i64::MAX;
        let mut base_close = None;

        self.for_each_bar_unit_index(x_axis, |series_index, _, unit, bar| {
            if series_index != 0 {
                return;
            }

            if unit < min_x_unit || unit > max_x_unit {
                return;
            }

            if unit < first_unit {
                first_unit = unit;
                base_close = Some(bar.close.to_f32());
            }
        });

        base_close
    }

    fn compute_series_percent_anchors(
        &self,
        x_axis: XAxis,
        min_x_unit: i64,
        max_x_unit: i64,
    ) -> Vec<Option<f32>> {
        let mut anchors = vec![None; self.series.len()];
        let mut first_units = vec![i64::MAX; self.series.len()];

        self.for_each_bar_unit_index(x_axis, |series_index, _, unit, bar| {
            if unit < min_x_unit || unit > max_x_unit {
                return;
            }

            if unit < first_units[series_index] {
                first_units[series_index] = unit;
                anchors[series_index] = Some(bar.close.to_f32());
            }
        });

        anchors
    }

    fn compute_primary_percent_domain(
        &self,
        x_axis: XAxis,
        min_x_unit: i64,
        max_x_unit: i64,
        primary_mark: MarkKind,
        anchors: &[Option<f32>],
    ) -> Option<(f32, f32)> {
        let mut min_pct: Option<f32> = None;
        let mut max_pct: Option<f32> = None;

        self.for_each_bar_unit_index(x_axis, |series_index, _, unit, bar| {
            if unit < min_x_unit || unit > max_x_unit {
                return;
            }

            let Some(anchor) = anchors
                .get(series_index)
                .copied()
                .flatten()
                .filter(|anchor| anchor.abs() > f32::EPSILON)
            else {
                return;
            };

            let mut emit_pct = |value: f32| {
                let pct = ((value / anchor) - 1.0) * 100.0;
                min_pct = Some(min_pct.map_or(pct, |current| current.min(pct)));
                max_pct = Some(max_pct.map_or(pct, |current| current.max(pct)));
            };

            if series_index == 0 && !matches!(primary_mark, MarkKind::Line) {
                emit_pct(bar.low.to_f32());
                emit_pct(bar.high.to_f32());
            } else {
                emit_pct(bar.close.to_f32());
            }
        });

        let min_pct = min_pct?;
        let max_pct = max_pct?;

        let pad = if (max_pct - min_pct).abs() <= f32::EPSILON {
            max_pct.abs().max(1.0)
        } else {
            ((max_pct - min_pct).abs() * 0.05).max(0.25)
        };

        Some((min_pct - pad, max_pct + pad))
    }

    fn resolve_x_axis(&self) -> Option<XAxis> {
        match self.basis {
            data::chart::Basis::Time(timeframe) => {
                let anchor = self
                    .series
                    .iter()
                    .flat_map(|s| s.bars().iter())
                    .map(|bar| bar.time.floor_to(timeframe))
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

    pub(super) fn normalize_horizontal_scale(&self, scale: HorizontalScale) -> HorizontalScale {
        HorizontalScale::pixels_per_bar(
            scale
                .as_pixels_per_bar()
                .clamp(MIN_BAR_SPACING_PX, MAX_BAR_SPACING_PX),
        )
    }

    fn max_points_available(&self) -> usize {
        self.series
            .iter()
            .map(|s| s.bars().len())
            .max()
            .unwrap_or_default()
    }

    pub(super) fn step_horizontal_scale_percent(
        &self,
        current: HorizontalScale,
        zoom_in: bool,
    ) -> HorizontalScale {
        let base = self.normalize_horizontal_scale(current).as_pixels_per_bar();
        let factor = if zoom_in {
            1.0 + SCALE_STEP_PCT
        } else {
            1.0 - SCALE_STEP_PCT
        };

        HorizontalScale::pixels_per_bar(
            (base * factor).clamp(MIN_BAR_SPACING_PX, MAX_BAR_SPACING_PX),
        )
    }

    pub(super) fn for_each_bar_unit_index(
        &self,
        x_axis: XAxis,
        mut f: impl FnMut(usize, &S, i64, &Kline),
    ) {
        for (series_index, series) in self.series.iter().enumerate() {
            match x_axis {
                XAxis::Time { .. } => {
                    for bar in series.bars() {
                        f(series_index, series, x_axis.unit_from_time(bar.time), bar);
                    }
                }
                XAxis::Tick { .. } => {
                    let len = series.bars().len();
                    for (index, bar) in series.bars().iter().enumerate() {
                        let from_latest = len.saturating_sub(1).saturating_sub(index) as u64;
                        f(
                            series_index,
                            series,
                            x_axis.unit_from_tick(TickIndex(from_latest)),
                            bar,
                        );
                    }
                }
            }
        }
    }

    fn for_each_bar_unit(&self, x_axis: XAxis, mut f: impl FnMut(&S, i64, &Kline)) {
        self.for_each_bar_unit_index(x_axis, |_, series, unit, bar| f(series, unit, bar));
    }

    fn data_x_bounds(&self, x_axis: XAxis) -> Option<(i64, i64)> {
        let mut any = false;
        let mut min_unit = i64::MAX;
        let mut max_unit = i64::MIN;

        self.for_each_bar_unit(x_axis, |_, unit, _| {
            any = true;
            min_unit = min_unit.min(unit);
            max_unit = max_unit.max(unit);
        });

        any.then_some((min_unit, max_unit))
    }

    fn visible_x_span_units_for_width(&self, plot_width: f32) -> i64 {
        let spacing = self
            .normalize_horizontal_scale(self.horizontal_scale)
            .as_pixels_per_bar()
            .max(1e-3);
        ((plot_width.max(1.0) / spacing).floor() as i64).max(1)
    }

    fn compute_x_window(&self, plot_width: f32) -> Option<(XAxis, i64, i64)> {
        let x_axis = self.resolve_x_axis()?;
        let (data_min_x, mut data_max_x) = self.data_x_bounds(x_axis)?;

        if data_max_x == data_min_x {
            data_max_x = data_max_x.saturating_add(1);
        }

        let span = self.visible_x_span_units_for_width(plot_width);

        let pan_units = self.horizontal_offset.round() as i64;
        let mut right = data_max_x.saturating_add(pan_units);
        let right_cap = data_max_x.saturating_add(span);
        if right > right_cap {
            right = right_cap;
        }

        let mut left = right.saturating_sub(span);

        if left < data_min_x {
            let shift = data_min_x.saturating_sub(left);
            left = left.saturating_add(shift);
            right = right.saturating_add(shift);
        }

        if right <= left {
            right = left.saturating_add(1);
        }

        Some((x_axis, left, right))
    }

    fn compute_primary_domain(
        &self,
        x_axis: XAxis,
        min_x_unit: i64,
        max_x_unit: i64,
    ) -> Option<(f32, f32)> {
        let mut min_primary_value: Option<f32> = None;
        let mut max_primary_value: Option<f32> = None;

        self.for_each_bar_unit(x_axis, |_, unit, bar| {
            if unit < min_x_unit || unit > max_x_unit {
                return;
            }

            let low = bar.low.to_f32();
            let high = bar.high.to_f32();

            min_primary_value = Some(min_primary_value.map_or(low, |value| value.min(low)));
            max_primary_value = Some(max_primary_value.map_or(high, |value| value.max(high)));
        });

        let min_primary_value = min_primary_value?;
        let max_primary_value = max_primary_value?;

        let pad = if (max_primary_value - min_primary_value).abs() <= f32::EPSILON {
            (max_primary_value.abs() / 200.0).max(1e-6)
        } else {
            ((max_primary_value - min_primary_value).abs() * 0.05).max(1e-6)
        };

        Some((min_primary_value - pad, max_primary_value + pad))
    }

    fn compute_indicator_domain(
        &self,
        x_axis: XAxis,
        min_x_unit: i64,
        max_x_unit: i64,
        indicator_panel_index: usize,
        scale_mode: PanelScaleMode,
    ) -> Option<(f32, f32)> {
        let mut any = false;
        let mut min_value = f32::INFINITY;
        let mut max_value = f32::NEG_INFINITY;

        self.for_each_bar_unit_index(x_axis, |series_index, series, unit, bar| {
            if series_index != 0 {
                return;
            }

            if unit < min_x_unit || unit > max_x_unit {
                return;
            }

            let Some(value) = series.indicator_value_for_panel_opt(indicator_panel_index, bar)
            else {
                return;
            };

            any = true;
            min_value = min_value.min(value);
            max_value = max_value.max(value);
        });

        if !any || !min_value.is_finite() || !max_value.is_finite() {
            return None;
        }

        match scale_mode {
            PanelScaleMode::FitVisible => {
                let span = (max_value - min_value).abs();
                let pad = if span <= f32::EPSILON {
                    max_value.abs().max(1.0) * 0.02
                } else {
                    span * 0.05
                };

                Some((min_value - pad, max_value + pad))
            }
            PanelScaleMode::FitVisibleIncludeZero => {
                let min_including_zero = min_value.min(0.0);
                let max_including_zero = max_value.max(0.0);
                let span = (max_including_zero - min_including_zero).abs();
                let pad = if span <= f32::EPSILON {
                    max_including_zero.abs().max(1.0) * 0.02
                } else {
                    span * 0.05
                };

                Some((min_including_zero - pad, max_including_zero + pad))
            }
            _ => Some((0.0, max_value.max(1.0))),
        }
    }

    pub(super) fn compute_scene(&self, layout: Layout<'_>, cursor: mouse::Cursor) -> Option<Scene> {
        let panel_kinds = self.resolved_panel_kinds();
        let panel_layout = PanelLayoutTree::from_layout(layout, panel_kinds)?;
        let primary_panel = panel_layout
            .panels
            .iter()
            .position(|panel| panel.kind == KlinePanelKind::PrimaryChart)?;
        let primary_mark = self.resolved_panel_mark(primary_panel, KlinePanelKind::PrimaryChart);
        let primary_scale_mode = self.resolved_panel_scale_mode(primary_panel);
        let primary_plot = panel_layout.panel(primary_panel)?.plot;
        let bar_spacing_px = self
            .normalize_horizontal_scale(self.horizontal_scale)
            .as_pixels_per_bar();

        let (x_axis, min_x_unit, max_x_unit) = self.compute_x_window(primary_plot.width)?;

        let indicator_panels: Vec<IndicatorPanelScene> = panel_layout
            .panels
            .iter()
            .enumerate()
            .filter_map(|(panel_index, panel)| {
                if panel.kind != KlinePanelKind::Indicator {
                    return None;
                }

                let mark = self.resolved_panel_mark(panel_index, KlinePanelKind::Indicator);
                let scale_mode = self.resolved_panel_scale_mode(panel_index);
                let (min_value, max_value) = self
                    .compute_indicator_domain(
                        x_axis,
                        min_x_unit,
                        max_x_unit,
                        panel_index,
                        scale_mode,
                    )
                    .unwrap_or((0.0, 1.0));

                Some(IndicatorPanelScene {
                    panel_index,
                    mark,
                    min_value,
                    max_value,
                })
            })
            .collect();

        let mut series_percent_anchors = Vec::new();

        let (min_primary_value, max_primary_value, primary_scale_anchor) =
            if matches!(primary_scale_mode, PanelScaleMode::PercentFromBase)
                && self.series.len() > 1
            {
                let anchors = self.compute_series_percent_anchors(x_axis, min_x_unit, max_x_unit);
                let (min_value, max_value) = self.compute_primary_percent_domain(
                    x_axis,
                    min_x_unit,
                    max_x_unit,
                    primary_mark,
                    &anchors,
                )?;

                series_percent_anchors = anchors;
                (min_value, max_value, None)
            } else {
                let (min_value, max_value) =
                    self.compute_primary_domain(x_axis, min_x_unit, max_x_unit)?;
                let scale_anchor = if matches!(primary_scale_mode, PanelScaleMode::PercentFromBase)
                {
                    self.compute_primary_scale_anchor(x_axis, min_x_unit, max_x_unit)
                } else {
                    None
                };

                (min_value, max_value, scale_anchor)
            };

        if series_percent_anchors.is_empty() {
            series_percent_anchors = vec![primary_scale_anchor; self.series.len()];
        }

        let cursor_root_local = cursor.position_in(layout.bounds());
        let show_legend_values = cursor_root_local
            .map(|local| matches!(panel_layout.hit_test(local), LayoutHitZone::PanelPlot(_)))
            .unwrap_or(false);

        let panel_controls = self.build_panel_control_hits(&panel_layout, primary_panel);
        let ticker_legend = self.build_ticker_legend_layout(
            &panel_layout,
            primary_panel,
            show_legend_values,
            false,
        );

        let mut scene = Scene {
            layout: panel_layout,
            x_axis,
            bar_spacing_px,
            min_x_unit,
            max_x_unit,
            min_primary_value,
            max_primary_value,
            primary_panel,
            primary_mark,
            primary_scale_mode,
            primary_scale_anchor,
            series_percent_anchors,
            indicator_panels,
            panel_controls,
            ticker_legend,
            controls_visible_for_panel: None,
            hovered_control: None,
            hovering_ticker_legend: false,
            hovered_ticker_row: None,
            hovered_ticker_icon: None,
            cursor: None,
        };

        if let Some(local) = cursor_root_local {
            scene.hovered_control =
                Self::hit_panel_control(&scene.layout, &scene.panel_controls, local);

            let mut legend_hit = scene
                .ticker_legend
                .as_ref()
                .and_then(|legend| Self::hit_ticker_legend(&scene.layout, legend, local));

            if legend_hit.is_some() {
                scene.ticker_legend =
                    self.build_ticker_legend_layout(&scene.layout, primary_panel, false, true);
                legend_hit = scene
                    .ticker_legend
                    .as_ref()
                    .and_then(|legend| Self::hit_ticker_legend(&scene.layout, legend, local));
            }

            if let Some(hit) = legend_hit {
                scene.hovering_ticker_legend = true;
                match hit {
                    TickerLegendHit::Background => {}
                    TickerLegendHit::Row(index) => {
                        scene.hovered_ticker_row = Some(index);
                    }
                    TickerLegendHit::Icon(index, kind) => {
                        scene.hovered_ticker_row = Some(index);
                        scene.hovered_ticker_icon = Some((index, kind));
                    }
                }
            }

            scene.controls_visible_for_panel = scene
                .hovered_control
                .map(|hit| hit.panel_index)
                .or_else(|| self.control_visibility_panel(&scene.layout, local));
        }

        let mut cursor_info = None;

        if let Some(local) = cursor_root_local {
            let zone = scene.layout.hit_test(local);

            if matches!(zone, LayoutHitZone::PanelPlot(_))
                && let Some(plot_local) = scene.layout.plot_local_point(local)
            {
                let x_plot = plot_local.x.clamp(0.0, primary_plot.width.max(1.0));
                let snapped_x_unit = scene.unit_from_plot_x(x_plot);
                let snapped_x_plot = scene
                    .map_x_plot(snapped_x_unit)
                    .round()
                    .clamp(0.0, primary_plot.width.max(1.0));

                if let LayoutHitZone::PanelPlot(panel_index) = zone
                    && let Some(panel) = scene.layout.panel(panel_index)
                {
                    let y_in_panel =
                        (plot_local.y - panel.plot.y).clamp(0.0, panel.plot.height.max(1.0));

                    let (y_primary_value, y_indicator_value) = match panel.kind {
                        KlinePanelKind::PrimaryChart => {
                            let value_ratio = 1.0 - (y_in_panel / panel.plot.height.max(1.0));
                            let (min_display, max_display) = scene.primary_domain_display_values();
                            let y_display_value =
                                min_display + ((max_display - min_display) * value_ratio);
                            let y_primary_value = scene.primary_display_to_value(y_display_value);

                            (Some(y_primary_value), None)
                        }
                        KlinePanelKind::Indicator => {
                            let indicator_ratio = 1.0 - (y_in_panel / panel.plot.height.max(1.0));
                            let (min_value, max_value) = scene
                                .indicator_panel_config(panel_index)
                                .map(|indicator| (indicator.min_value, indicator.max_value))
                                .unwrap_or((0.0, 1.0));
                            let y_indicator_value =
                                min_value + (max_value - min_value) * indicator_ratio;

                            (None, Some(y_indicator_value))
                        }
                    };

                    cursor_info = Some(CursorInfo {
                        x_unit: snapped_x_unit,
                        panel_index,
                        y_primary_value,
                        y_indicator_value,
                        x_plot: snapped_x_plot,
                        y_plot: panel.plot.y + y_in_panel,
                    });
                }
            }
        }

        scene.cursor = cursor_info;
        Some(scene)
    }

    pub(super) fn format_x_label(&self, x_axis: XAxis, unit: i64, step_units: i64) -> String {
        match x_axis {
            XAxis::Time { .. } => x_axis.time_from_unit(unit).map_or_else(
                || unit.to_string(),
                |ts| {
                    super::super::format_time_label(
                        ts.as_u64(),
                        x_axis.step_ms(step_units),
                        self.timezone,
                    )
                },
            ),
            XAxis::Tick { .. } => x_axis
                .tick_from_unit(unit)
                .map_or_else(|| unit.to_string(), |index| index.as_u64().to_string()),
        }
    }
}
