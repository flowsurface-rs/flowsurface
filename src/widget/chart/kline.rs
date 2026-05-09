mod chrome;
pub mod composition;
pub mod coord;
pub mod drawing;
mod layout;
mod scene;

use crate::style;
use crate::widget::chart::kline::drawing::{
    DrawingAnchor, DrawingDragTarget, DrawingEntity, DrawingHandleKind, DrawingId, DrawingObject,
    DrawingSnapshot, DrawingStyle, DrawingTool, KlineWidgetDrawingEvent,
};
use chrome::TickerLegendHit;
use composition::{
    BarMode, ChartComposition, DEFAULT_MIN_PANEL_RATIO, HistogramMode, LayerDataKind, MarkKind,
    PanelId, PanelScaleMode, PanelValueId, PanelValueLabelMode, PanelValueLabelPolicy,
    PanelValuePrecision,
};
use layout::{LayoutHitZone, PanelLayoutTree};
use scene::Scene;

use data::UserTimezone;
use data::chart::Basis;
use exchange::unit::{Price, Qty};
use exchange::{Kline, TickerInfo, Timeframe, UnixMs};

use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{self, Clipboard, Layout, Shell, Widget, layout as iced_layout, renderer};
use iced::theme::palette::Extended;
use iced::widget::canvas;
use iced::{
    Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector, mouse, window,
};
use iced_core::renderer::Quad;

const Y_AXIS_GUTTER: f32 = 66.0;
const X_AXIS_HEIGHT: f32 = 24.0;

const MIN_X_TICK_PX: f32 = 80.0;
const TEXT_SIZE: f32 = 12.0;
const CHAR_W: f32 = TEXT_SIZE * 0.64;
const PANEL_SPLITTER_HEIGHT: f32 = 1.0;
const PANEL_SPLITTER_HIT_PX: f32 = 8.0;
const PANEL_TITLE_LEFT_PAD: f32 = 6.0;
const PANEL_TITLE_TOP_PAD: f32 = 4.0;
const PANEL_TITLE_TO_CONTROLS_GAP: f32 = 8.0;
const PANEL_CONTROL_BOX: f32 = TEXT_SIZE + 5.0;
const PANEL_CONTROL_GAP: f32 = 4.0;
const PANEL_CONTROL_ICON_SIZE: f32 = TEXT_SIZE - 1.0;
const Y_AXIS_DRAG_SCALE_DELTA_PER_PX: f32 = 0.1;

const TICKER_LEGEND_PADDING: f32 = 4.0;
const TICKER_LEGEND_ROW_H: f32 = TEXT_SIZE + 6.0;
const TICKER_LEGEND_ICON_BOX: f32 = TEXT_SIZE + 8.0;
const TICKER_LEGEND_ICON_GAP: f32 = 4.0;
const TICKER_LEGEND_TOP_OFFSET: f32 = 0.0;
const DRAWING_HIT_TOLERANCE_PX: f32 = 7.0;
const DRAWING_HANDLE_RADIUS_PX: f32 = 4.0;
const DRAWING_HANDLE_HIT_RADIUS_PX: f32 = 8.0;

pub const DEFAULT_BAR_SPACING_PX: f32 = 5.0;
pub const MIN_BAR_SPACING_PX: f32 = 2.0;
pub const MAX_BAR_SPACING_PX: f32 = 48.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HorizontalScale(pub f32);

impl HorizontalScale {
    pub fn pixels_per_bar(px: f32) -> Self {
        Self(px)
    }

    pub fn as_pixels_per_bar(self) -> f32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelYViewport {
    pub min_value: f32,
    pub max_value: f32,
}

impl PanelYViewport {
    pub fn normalized(min_value: f32, max_value: f32) -> Option<Self> {
        if !min_value.is_finite() || !max_value.is_finite() {
            return None;
        }

        let (mut min_value, mut max_value) = if min_value <= max_value {
            (min_value, max_value)
        } else {
            (max_value, min_value)
        };

        if (max_value - min_value).abs() <= 1e-8 {
            let pad = min_value.abs().max(1.0) * 0.01;
            min_value -= pad;
            max_value += pad;
        }

        Some(Self {
            min_value,
            max_value,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct YUnit(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BarSpacingPx(i32);

impl BarSpacingPx {
    fn from_logical(px: f32) -> Self {
        let snapped = px
            .clamp(MIN_BAR_SPACING_PX, MAX_BAR_SPACING_PX)
            .round()
            .max(1.0) as i32;

        Self(snapped.max(1))
    }

    fn as_i32(self) -> i32 {
        self.0
    }

    fn as_f32(self) -> f32 {
        self.0 as f32
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IndicatorDataFieldKey {
    SignedOverlay,
    Custom(&'static str),
}

pub const BOLLINGER_UPPER_FIELD_KEY: IndicatorDataFieldKey =
    IndicatorDataFieldKey::Custom("bollinger.upper");
pub const BOLLINGER_LOWER_FIELD_KEY: IndicatorDataFieldKey =
    IndicatorDataFieldKey::Custom("bollinger.lower");
pub const RSI_SIGNAL_FIELD_KEY: IndicatorDataFieldKey = IndicatorDataFieldKey::Custom("rsi.signal");
pub const RSI_UPPER_BAND_FIELD_KEY: IndicatorDataFieldKey =
    IndicatorDataFieldKey::Custom("rsi.upper_band");
pub const RSI_LOWER_BAND_FIELD_KEY: IndicatorDataFieldKey =
    IndicatorDataFieldKey::Custom("rsi.lower_band");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayChannelColorRole {
    Neutral,
    Success,
    Danger,
    Primary,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlayChannelSpec {
    pub label: &'static str,
    pub key: Option<IndicatorDataFieldKey>,
    pub line_width: f32,
    pub color_role: OverlayChannelColorRole,
}

const DEFAULT_OVERLAY_CHANNELS: [OverlayChannelSpec; 1] = [OverlayChannelSpec {
    label: "V",
    key: None,
    line_width: 1.1,
    color_role: OverlayChannelColorRole::Neutral,
}];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IndicatorDataField {
    pub key: IndicatorDataFieldKey,
    pub value: f32,
}

const MAX_INDICATOR_EXTRA_FIELDS: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IndicatorData {
    value: f32,
    extra_fields: [Option<IndicatorDataField>; MAX_INDICATOR_EXTRA_FIELDS],
}

impl IndicatorData {
    pub fn scalar(value: f32) -> Self {
        Self {
            value,
            extra_fields: [None; MAX_INDICATOR_EXTRA_FIELDS],
        }
    }

    pub fn value(self) -> f32 {
        self.value
    }

    pub fn with_field(mut self, key: IndicatorDataFieldKey, value: f32) -> Self {
        if let Some(slot) = self
            .extra_fields
            .iter_mut()
            .find(|slot| slot.as_ref().map(|field| field.key == key).unwrap_or(false))
        {
            *slot = Some(IndicatorDataField { key, value });
            return self;
        }

        if let Some(slot) = self.extra_fields.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(IndicatorDataField { key, value });
        } else {
            debug_assert!(
                false,
                "IndicatorData extra field capacity exceeded ({MAX_INDICATOR_EXTRA_FIELDS})"
            );
        }

        self
    }

    pub fn field(self, key: IndicatorDataFieldKey) -> Option<f32> {
        self.extra_fields
            .iter()
            .flatten()
            .find(|field| field.key == key)
            .map(|field| field.value)
    }

    pub fn with_signed_overlay(self, signed_overlay: f32) -> Self {
        self.with_field(IndicatorDataFieldKey::SignedOverlay, signed_overlay)
    }

    pub fn signed_overlay(self) -> Option<f32> {
        self.field(IndicatorDataFieldKey::SignedOverlay)
    }
}

pub trait KlineSeriesLike {
    fn ticker_info(&self) -> &TickerInfo;
    fn bars(&self) -> &[Kline];
    fn indicator_value(&self, bar: &Kline) -> f32;

    fn indicator_value_for_panel_value_opt(
        &self,
        panel_value: Option<PanelValueId>,
        bar: &Kline,
    ) -> Option<f32> {
        let _ = panel_value;
        Some(self.indicator_value(bar))
    }

    fn indicator_overlay_value_for_panel_value_opt(
        &self,
        panel_value: Option<PanelValueId>,
        _bar: &Kline,
    ) -> Option<f32> {
        let _ = panel_value;
        None
    }

    fn indicator_data_for_panel_value_opt(
        &self,
        panel_value: Option<PanelValueId>,
        bar: &Kline,
    ) -> Option<IndicatorData> {
        let value = self.indicator_value_for_panel_value_opt(panel_value, bar)?;
        let mut data = IndicatorData::scalar(value);

        if let Some(signed_overlay) =
            self.indicator_overlay_value_for_panel_value_opt(panel_value, bar)
        {
            data = data.with_signed_overlay(signed_overlay);
        }

        Some(data)
    }

    fn indicator_overlay_channels_for_panel_value(
        &self,
        panel_value: PanelValueId,
    ) -> &'static [OverlayChannelSpec] {
        let _ = panel_value;
        &[]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KlinePanelKind {
    PrimaryChart,
    Indicator,
}

#[derive(Debug, Clone)]
pub enum KlineWidgetEvent {
    HorizontalScaleChanged(HorizontalScale),
    HorizontalOffsetChanged(f32),
    PrimaryAutoscaleToggled,
    PrimaryScaleModeCycleRequested,
    PanelYViewportChanged {
        panel_id: PanelId,
        viewport: PanelYViewport,
    },
    PanelYViewportReset {
        panel_id: PanelId,
    },
    PanelSplitChanged {
        index: usize,
        split: f32,
    },
    PanelMoveUp {
        index: usize,
    },
    PanelMoveDown {
        index: usize,
    },
    PanelSettings {
        index: usize,
    },
    PanelClose {
        index: usize,
    },
    TickerSettings(TickerInfo),
    TickerRemove(TickerInfo),
    XAxisDoubleClick,
    Drawing(KlineWidgetDrawingEvent),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DragMode {
    None,
    Split(usize),
    AxisScale {
        panel_index: usize,
        anchor_y: f32,
    },
    Pan {
        panel_index: usize,
    },
    DrawingMove {
        id: DrawingId,
        target: DrawingDragTarget,
        panel_id: PanelId,
    },
}

struct State {
    plot_cache: canvas::Cache,
    y_axis_cache: canvas::Cache,
    x_axis_cache: canvas::Cache,
    overlay_cache: canvas::Cache,
    interaction_text_cache: canvas::Cache,
    drag_mode: DragMode,
    last_cursor: Option<Point>,
    last_cache_rev: u64,
    previous_click: Option<iced_core::mouse::Click>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            plot_cache: canvas::Cache::new(),
            y_axis_cache: canvas::Cache::new(),
            x_axis_cache: canvas::Cache::new(),
            overlay_cache: canvas::Cache::new(),
            interaction_text_cache: canvas::Cache::new(),
            drag_mode: DragMode::None,
            last_cursor: None,
            last_cache_rev: 0,
            previous_click: None,
        }
    }
}

impl State {
    fn clear_all_caches(&mut self) {
        self.plot_cache.clear();
        self.y_axis_cache.clear();
        self.x_axis_cache.clear();
        self.clear_overlay_caches();
    }

    fn clear_overlay_caches(&mut self) {
        self.overlay_cache.clear();
        self.interaction_text_cache.clear();
    }
}

pub struct KlineWidget<'a, S> {
    series: &'a [S],
    composition: &'a ChartComposition,
    drawings: DrawingSnapshot<'a>,
    basis: Basis,
    horizontal_scale: HorizontalScale,
    horizontal_offset: f32,
    horizontal_pixel_ratio: f32,
    primary_autoscale: bool,
    panel_y_viewports: &'a [(PanelId, PanelYViewport)],
    timezone: UserTimezone,
    version: u64,
}

impl<'a, S> KlineWidget<'a, S>
where
    S: KlineSeriesLike,
{
    pub fn new(series: &'a [S], timeframe: Timeframe, composition: &'a ChartComposition) -> Self {
        Self {
            series,
            composition,
            drawings: DrawingSnapshot::new(DrawingTool::Cursor, &[], None, None),
            basis: Basis::Time(timeframe),
            horizontal_scale: HorizontalScale::pixels_per_bar(DEFAULT_BAR_SPACING_PX),
            horizontal_offset: 0.0,
            horizontal_pixel_ratio: 1.0,
            primary_autoscale: false,
            panel_y_viewports: &[],
            timezone: UserTimezone::Utc,
            version: 0,
        }
    }

    pub fn with_horizontal_scale(mut self, scale: HorizontalScale) -> Self {
        self.horizontal_scale = scale;
        self
    }

    pub fn with_horizontal_offset(mut self, offset: f32) -> Self {
        self.horizontal_offset = offset;
        self
    }

    pub fn with_horizontal_pixel_ratio(mut self, ratio: f32) -> Self {
        self.horizontal_pixel_ratio = if ratio.is_finite() && ratio > 0.0 {
            ratio
        } else {
            1.0
        };
        self
    }

    pub fn with_primary_autoscale(mut self, autoscale: bool) -> Self {
        self.primary_autoscale = autoscale;
        self
    }

    pub fn with_drawing_snapshot(mut self, snapshot: DrawingSnapshot<'a>) -> Self {
        self.drawings = snapshot;
        self
    }

    pub fn with_panel_y_viewports(mut self, viewports: &'a [(PanelId, PanelYViewport)]) -> Self {
        self.panel_y_viewports = viewports;
        self
    }

    pub fn with_timezone(mut self, tz: UserTimezone) -> Self {
        self.timezone = tz;
        self
    }

    pub fn with_basis(mut self, basis: Basis) -> Self {
        self.basis = basis;
        self
    }

    pub fn version(mut self, rev: u64) -> Self {
        self.version = rev;
        self
    }

    fn panel_count(&self) -> usize {
        self.composition.panel_count().max(1)
    }

    fn panel_index_for_id(&self, panel_id: PanelId) -> Option<usize> {
        self.composition
            .panels
            .iter()
            .position(|panel| panel.id == panel_id)
    }

    fn x_unit_for_anchor_time(&self, scene: &Scene, time: UnixMs) -> Option<i64> {
        if let Some(unit) = scene.x_unit_for_time(time) {
            return Some(unit);
        }

        let mut best: Option<(u64, i64)> = None;

        self.for_each_bar_unit_index(scene.x_axis, |series_index, _, x_unit, bar| {
            if series_index != 0 {
                return;
            }

            let delta = bar.time.as_u64().abs_diff(time.as_u64());

            if best
                .map(|(best_delta, _)| delta < best_delta)
                .unwrap_or(true)
            {
                best = Some((delta, x_unit));
            }
        });

        best.map(|(_, unit)| unit)
    }

    fn anchor_time_for_cursor_unit(&self, scene: &Scene, x_unit: i64) -> Option<UnixMs> {
        if let Some(time) = scene.time_for_x_unit(x_unit) {
            return Some(time);
        }

        let series = self.series.first()?;
        self.bar_at_or_before_unit(series, scene.x_axis, x_unit)
            .map(|bar| bar.time)
    }

    fn drawing_anchor_from_scene_cursor(&self, scene: &Scene) -> Option<DrawingAnchor> {
        let cursor = scene.cursor?;
        let panel_id = self.panel_id(cursor.panel_index)?;
        let time = self.anchor_time_for_cursor_unit(scene, cursor.x_unit)?;
        let y_unit = cursor.y_primary_unit.or(cursor.y_indicator_unit)?;

        Some(DrawingAnchor {
            panel_id,
            time,
            y_unit,
        })
    }

    fn drawing_anchor_from_scene_point(
        &self,
        scene: &Scene,
        panel_id: PanelId,
        cursor_local: Point,
    ) -> Option<DrawingAnchor> {
        let panel_index = self.panel_index_for_id(panel_id)?;
        let panel = scene.layout.panel(panel_index)?;

        let x_plot = cursor_local.x - scene.layout.regions.plot.x;
        let right_edge_px = scene.x_axis_plot_width().floor().max(1.0);
        let spacing_px = scene.bar_spacing_px().as_f32().max(1.0);
        let steps_from_right = ((right_edge_px - x_plot) / spacing_px).round() as i64;
        let x_unit = scene.max_x_unit.saturating_sub(steps_from_right);
        let time = self.anchor_time_for_cursor_unit(scene, x_unit)?;

        let panel_h = panel.plot.height.max(1.0);
        let y_in_panel = cursor_local.y - scene.layout.regions.plot.y - panel.plot.y;
        let ratio = 1.0 - (y_in_panel / panel_h);
        let panel_precision = self.panel_value_precision(panel_index);

        let y_unit = match panel.kind {
            KlinePanelKind::PrimaryChart => {
                let uses_display_space = matches!(
                    scene.primary_scale_mode,
                    PanelScaleMode::PercentFromBase | PanelScaleMode::Logarithmic
                ) || scene.primary_domain_display_override.is_some();

                if uses_display_space {
                    let (min_display, max_display) = scene.primary_domain_display_values();
                    let display_value = min_display + (max_display - min_display) * ratio;
                    let value = scene.primary_display_to_value(display_value);
                    self.panel_value_to_unit(panel_precision, value)
                } else {
                    let min_value =
                        self.panel_unit_to_value(panel_precision, scene.min_primary_unit);
                    let max_value =
                        self.panel_unit_to_value(panel_precision, scene.max_primary_unit);
                    let value = min_value + (max_value - min_value) * ratio;
                    self.panel_value_to_unit(panel_precision, value)
                }
            }
            KlinePanelKind::Indicator => {
                let indicator = scene
                    .indicator_panels
                    .iter()
                    .find(|indicator| indicator.panel_index == panel_index)?;
                let min_value = self.panel_unit_to_value(panel_precision, indicator.min_unit);
                let max_value = self.panel_unit_to_value(panel_precision, indicator.max_unit);
                let value = min_value + (max_value - min_value) * ratio;
                self.panel_value_to_unit(panel_precision, value)
            }
        };

        Some(DrawingAnchor {
            panel_id,
            time,
            y_unit,
        })
    }

    fn panel_plot_bounds_in_overlay(&self, scene: &Scene, panel_index: usize) -> Option<Rectangle> {
        let panel = scene.layout.panel(panel_index)?;
        Some(Rectangle {
            x: scene.layout.regions.plot.x + panel.plot.x,
            y: scene.layout.regions.plot.y + panel.plot.y,
            width: panel.plot.width,
            height: panel.plot.height,
        })
    }

    fn plot_bounds_in_overlay(&self, scene: &Scene) -> Rectangle {
        scene.layout.regions.plot
    }

    fn drawing_object_clip_bounds(&self, scene: &Scene, panel_id: PanelId) -> Option<Rectangle> {
        let panel_index = self.panel_index_for_id(panel_id)?;
        self.panel_plot_bounds_in_overlay(scene, panel_index)
    }

    fn clip_segment_to_bounds(
        start: Point,
        end: Point,
        bounds: Rectangle,
    ) -> Option<(Point, Point)> {
        let left = bounds.x;
        let right = bounds.x + bounds.width;
        let top = bounds.y;
        let bottom = bounds.y + bounds.height;

        let dx = end.x - start.x;
        let dy = end.y - start.y;

        let mut t0 = 0.0_f32;
        let mut t1 = 1.0_f32;

        let mut clip = |p: f32, q: f32| -> bool {
            if p.abs() <= f32::EPSILON {
                return q >= 0.0;
            }

            let r = q / p;

            if p < 0.0 {
                if r > t1 {
                    return false;
                }
                if r > t0 {
                    t0 = r;
                }
            } else {
                if r < t0 {
                    return false;
                }
                if r < t1 {
                    t1 = r;
                }
            }

            true
        };

        if !clip(-dx, start.x - left)
            || !clip(dx, right - start.x)
            || !clip(-dy, start.y - top)
            || !clip(dy, bottom - start.y)
        {
            return None;
        }

        if t0 > t1 {
            return None;
        }

        Some((
            Point::new(start.x + t0 * dx, start.y + t0 * dy),
            Point::new(start.x + t1 * dx, start.y + t1 * dy),
        ))
    }

    fn intersect_bounds(a: Rectangle, b: Rectangle) -> Option<Rectangle> {
        let left = a.x.max(b.x);
        let right = (a.x + a.width).min(b.x + b.width);
        let top = a.y.max(b.y);
        let bottom = (a.y + a.height).min(b.y + b.height);

        if right <= left || bottom <= top {
            return None;
        }

        Some(Rectangle {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        })
    }

    fn anchor_to_overlay_point(&self, scene: &Scene, anchor: DrawingAnchor) -> Option<Point> {
        let panel_index = self.panel_index_for_id(anchor.panel_id)?;
        let panel = scene.layout.panel(panel_index)?;
        let panel_precision = self.panel_value_precision(panel_index);
        let value = self.panel_unit_to_value(panel_precision, anchor.y_unit);
        let x_unit = self.x_unit_for_anchor_time(scene, anchor.time)?;
        let x_plot = Self::snap_plot_x_to_cell(
            scene.map_x_plot(x_unit),
            self.resolved_horizontal_pixel_ratio(),
        );
        let y_plot = match panel.kind {
            KlinePanelKind::PrimaryChart => {
                scene.map_primary_plot_with_anchor_unclamped(value, scene.primary_scale_anchor)
            }
            KlinePanelKind::Indicator => scene.map_indicator_plot_unclamped(panel_index, value)?,
        };

        Some(Point::new(
            scene.layout.regions.plot.x + x_plot,
            scene.layout.regions.plot.y + y_plot,
        ))
    }

    fn drawing_trendline_geometry(
        &self,
        scene: &Scene,
        start: DrawingAnchor,
        end: DrawingAnchor,
    ) -> Option<(Point, Point, Point, Point, Rectangle)> {
        if start.panel_id != end.panel_id {
            return None;
        }

        let clip_bounds = self.drawing_object_clip_bounds(scene, start.panel_id)?;
        let start_point = self.anchor_to_overlay_point(scene, start)?;
        let end_point = self.anchor_to_overlay_point(scene, end)?;
        let (visible_start, visible_end) =
            Self::clip_segment_to_bounds(start_point, end_point, clip_bounds)?;

        Some((
            start_point,
            end_point,
            visible_start,
            visible_end,
            clip_bounds,
        ))
    }

    fn drawing_box_geometry(
        &self,
        scene: &Scene,
        start: DrawingAnchor,
        end: DrawingAnchor,
    ) -> Option<(Point, Size, Rectangle, Rectangle)> {
        if start.panel_id != end.panel_id {
            return None;
        }

        let clip_bounds = self.drawing_object_clip_bounds(scene, start.panel_id)?;
        let start_point = self.anchor_to_overlay_point(scene, start)?;
        let end_point = self.anchor_to_overlay_point(scene, end)?;

        let left = start_point.x.min(end_point.x);
        let right = start_point.x.max(end_point.x);
        let top = start_point.y.min(end_point.y);
        let bottom = start_point.y.max(end_point.y);

        let origin = Point::new(left, top);
        let size = Size::new((right - left).max(1.0), (bottom - top).max(1.0));
        let object_bounds = Rectangle {
            x: origin.x,
            y: origin.y,
            width: size.width,
            height: size.height,
        };
        let visible_bounds = Self::intersect_bounds(object_bounds, clip_bounds)?;

        Some((origin, size, clip_bounds, visible_bounds))
    }

    fn drawing_horizontal_line_geometry(
        &self,
        scene: &Scene,
        panel_id: PanelId,
        y_unit: YUnit,
    ) -> Option<(Rectangle, f32)> {
        let panel_index = self.panel_index_for_id(panel_id)?;
        let panel = scene.layout.panel(panel_index)?;
        let clip_bounds = self.panel_plot_bounds_in_overlay(scene, panel_index)?;
        let panel_precision = self.panel_value_precision(panel_index);
        let value = self.panel_unit_to_value(panel_precision, y_unit);

        let y_plot = match panel.kind {
            KlinePanelKind::PrimaryChart => {
                scene.map_primary_plot_with_anchor_unclamped(value, scene.primary_scale_anchor)
            }
            KlinePanelKind::Indicator => scene.map_indicator_plot_unclamped(panel_index, value)?,
        };

        let panel_top = panel.plot.y;
        let panel_bottom = panel.plot.y + panel.plot.height;
        if y_plot < panel_top || y_plot > panel_bottom {
            return None;
        }

        Some((clip_bounds, scene.layout.regions.plot.y + y_plot))
    }

    fn drawing_vertical_line_geometry(
        &self,
        scene: &Scene,
        time: UnixMs,
    ) -> Option<(Rectangle, f32)> {
        let x_unit = self.x_unit_for_anchor_time(scene, time)?;
        let clip_bounds = self.plot_bounds_in_overlay(scene);
        let x = scene.layout.regions.plot.x
            + Self::snap_plot_x_to_cell(
                scene.map_x_plot(x_unit),
                self.resolved_horizontal_pixel_ratio(),
            );

        if x < clip_bounds.x || x > (clip_bounds.x + clip_bounds.width) {
            return None;
        }

        Some((clip_bounds, x))
    }

    fn draw_drawing_object(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        object: &DrawingObject,
        style: DrawingStyle,
        selected: bool,
        overlay_origin_in_window: Point,
        horizontal_pixel_ratio: f32,
    ) {
        let raw_stroke_width = if selected {
            (style.stroke_width + 0.6).max(1.0)
        } else {
            style.stroke_width.max(1.0)
        };
        let (stroke_width, stroke_width_phys) =
            Self::quantized_stroke_width(raw_stroke_width, horizontal_pixel_ratio);
        let stroke = canvas::Stroke::default()
            .with_color(style.stroke_color)
            .with_width(stroke_width);

        match object {
            DrawingObject::Trendline { start, end } => {
                let Some((start, end, _, _, bounds)) =
                    self.drawing_trendline_geometry(scene, *start, *end)
                else {
                    return;
                };
                let start = Self::snap_point_for_stroke_with_origin(
                    start,
                    horizontal_pixel_ratio,
                    overlay_origin_in_window,
                    stroke_width_phys,
                );
                let end = Self::snap_point_for_stroke_with_origin(
                    end,
                    horizontal_pixel_ratio,
                    overlay_origin_in_window,
                    stroke_width_phys,
                );

                frame.with_clip(bounds, |frame| {
                    frame.stroke(&canvas::Path::line(start, end), stroke);
                });
            }
            DrawingObject::Box { start, end } => {
                let Some((origin, size, bounds, _)) =
                    self.drawing_box_geometry(scene, *start, *end)
                else {
                    return;
                };

                let left = origin.x;
                let right = origin.x + size.width;
                let top = origin.y;
                let bottom = origin.y + size.height;

                let (fill_left, fill_width) = Self::snapped_span_with_origin(
                    left,
                    right,
                    horizontal_pixel_ratio,
                    overlay_origin_in_window.x,
                );
                let (fill_top, fill_height) = Self::snapped_span_with_origin(
                    top,
                    bottom,
                    horizontal_pixel_ratio,
                    overlay_origin_in_window.y,
                );

                let left_stroke = Self::snap_stroke_center_with_origin(
                    left,
                    horizontal_pixel_ratio,
                    overlay_origin_in_window.x,
                    stroke_width_phys,
                );
                let right_stroke = Self::snap_stroke_center_with_origin(
                    right,
                    horizontal_pixel_ratio,
                    overlay_origin_in_window.x,
                    stroke_width_phys,
                );
                let top_stroke = Self::snap_stroke_center_with_origin(
                    top,
                    horizontal_pixel_ratio,
                    overlay_origin_in_window.y,
                    stroke_width_phys,
                );
                let bottom_stroke = Self::snap_stroke_center_with_origin(
                    bottom,
                    horizontal_pixel_ratio,
                    overlay_origin_in_window.y,
                    stroke_width_phys,
                );

                frame.with_clip(bounds, |frame| {
                    if let Some(fill) = style.fill_color {
                        frame.fill_rectangle(
                            Point::new(fill_left, fill_top),
                            Size::new(fill_width, fill_height),
                            fill,
                        );
                    }

                    frame.stroke(
                        &canvas::Path::line(
                            Point::new(left_stroke, top_stroke),
                            Point::new(right_stroke, top_stroke),
                        ),
                        stroke,
                    );
                    frame.stroke(
                        &canvas::Path::line(
                            Point::new(right_stroke, top_stroke),
                            Point::new(right_stroke, bottom_stroke),
                        ),
                        stroke,
                    );
                    frame.stroke(
                        &canvas::Path::line(
                            Point::new(right_stroke, bottom_stroke),
                            Point::new(left_stroke, bottom_stroke),
                        ),
                        stroke,
                    );
                    frame.stroke(
                        &canvas::Path::line(
                            Point::new(left_stroke, bottom_stroke),
                            Point::new(left_stroke, top_stroke),
                        ),
                        stroke,
                    );
                });
            }
            DrawingObject::HorizontalLine { panel_id, y_unit } => {
                let Some((bounds, y)) =
                    self.drawing_horizontal_line_geometry(scene, *panel_id, *y_unit)
                else {
                    return;
                };

                let y = Self::snap_stroke_center_with_origin(
                    y,
                    horizontal_pixel_ratio,
                    overlay_origin_in_window.y,
                    stroke_width_phys,
                );

                frame.with_clip(bounds, |frame| {
                    frame.stroke(
                        &canvas::Path::line(
                            Point::new(bounds.x, y),
                            Point::new(bounds.x + bounds.width, y),
                        ),
                        stroke,
                    );
                });
            }
            DrawingObject::VerticalLine { time } => {
                let Some((plot, x)) = self.drawing_vertical_line_geometry(scene, *time) else {
                    return;
                };

                let x = Self::snap_stroke_center_with_origin(
                    x,
                    horizontal_pixel_ratio,
                    overlay_origin_in_window.x,
                    stroke_width_phys,
                );

                frame.with_clip(plot, |frame| {
                    frame.stroke(
                        &canvas::Path::line(
                            Point::new(x, plot.y),
                            Point::new(x, plot.y + plot.height),
                        ),
                        stroke,
                    );
                });
            }
        }
    }

    fn drawing_handle_points(
        &self,
        scene: &Scene,
        object: &DrawingObject,
    ) -> Vec<(DrawingHandleKind, Point)> {
        match object {
            DrawingObject::Trendline { start, end } => {
                let mut points = Vec::with_capacity(2);

                if let Some(start_point) = self.anchor_to_overlay_point(scene, *start) {
                    points.push((DrawingHandleKind::TrendlineStart, start_point));
                }

                if let Some(end_point) = self.anchor_to_overlay_point(scene, *end) {
                    points.push((DrawingHandleKind::TrendlineEnd, end_point));
                }

                points
            }
            DrawingObject::Box { start, end } => {
                if start.panel_id != end.panel_id {
                    return Vec::new();
                }

                let Some(start_point) = self.anchor_to_overlay_point(scene, *start) else {
                    return Vec::new();
                };
                let Some(end_point) = self.anchor_to_overlay_point(scene, *end) else {
                    return Vec::new();
                };

                let left = start_point.x.min(end_point.x);
                let right = start_point.x.max(end_point.x);
                let top = start_point.y.min(end_point.y);
                let bottom = start_point.y.max(end_point.y);

                vec![
                    (DrawingHandleKind::BoxTopLeft, Point::new(left, top)),
                    (DrawingHandleKind::BoxTopRight, Point::new(right, top)),
                    (DrawingHandleKind::BoxBottomRight, Point::new(right, bottom)),
                    (DrawingHandleKind::BoxBottomLeft, Point::new(left, bottom)),
                ]
            }
            DrawingObject::HorizontalLine { .. } | DrawingObject::VerticalLine { .. } => Vec::new(),
        }
    }

    fn drawing_handle_clip_bounds(
        &self,
        scene: &Scene,
        object: &DrawingObject,
    ) -> Option<Rectangle> {
        let panel_id = object.handle_panel_id()?;

        let panel_index = self.panel_index_for_id(panel_id)?;
        self.panel_plot_bounds_in_overlay(scene, panel_index)
    }

    fn draw_selected_drawing_handles(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        horizontal_pixel_ratio: f32,
        drawing: &DrawingEntity,
    ) {
        let handles = self.drawing_handle_points(scene, &drawing.object);
        if handles.is_empty() {
            return;
        }

        let Some(clip_bounds) = self.drawing_handle_clip_bounds(scene, &drawing.object) else {
            return;
        };

        let fill_color = palette.background.strongest.color;
        let stroke_color = palette.primary.base.color.scale_alpha(0.92);
        let (handle_stroke_width, _) = Self::quantized_stroke_width(1.2, horizontal_pixel_ratio);

        frame.with_clip(clip_bounds, |frame| {
            for (_, point) in handles {
                let circle = canvas::Path::circle(point, DRAWING_HANDLE_RADIUS_PX);

                frame.fill(&circle, fill_color);
                frame.stroke(
                    &circle,
                    canvas::Stroke::default()
                        .with_color(stroke_color)
                        .with_width(handle_stroke_width),
                );
            }
        });
    }

    fn hit_test_selected_drawing_handle(
        &self,
        scene: &Scene,
        point: Point,
    ) -> Option<(DrawingId, DrawingHandleKind)> {
        let drawing = self.drawings.selected_visible_drawing()?;
        if drawing.locked {
            return None;
        }

        let hit_radius_sq = DRAWING_HANDLE_HIT_RADIUS_PX * DRAWING_HANDLE_HIT_RADIUS_PX;
        let mut best: Option<(f32, DrawingHandleKind)> = None;

        for (kind, handle_point) in self.drawing_handle_points(scene, &drawing.object) {
            let dx = point.x - handle_point.x;
            let dy = point.y - handle_point.y;
            let dist_sq = dx * dx + dy * dy;

            if dist_sq > hit_radius_sq {
                continue;
            }

            if best
                .map(|(best_dist_sq, _)| dist_sq < best_dist_sq)
                .unwrap_or(true)
            {
                best = Some((dist_sq, kind));
            }
        }

        best.map(|(_, kind)| (drawing.id, kind))
    }

    fn fill_drawings(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        overlay_origin_in_window: Point,
        horizontal_pixel_ratio: f32,
    ) {
        let drawing_state = &self.drawings;

        for drawing in drawing_state
            .entities
            .iter()
            .filter(|drawing| drawing.visible)
        {
            let selected = drawing_state.selected_drawing == Some(drawing.id);
            self.draw_drawing_object(
                frame,
                scene,
                &drawing.object,
                drawing.style,
                selected,
                overlay_origin_in_window,
                horizontal_pixel_ratio,
            );
        }

        if let Some(selected_drawing) = drawing_state.selected_visible_drawing() {
            self.draw_selected_drawing_handles(
                frame,
                scene,
                palette,
                horizontal_pixel_ratio,
                selected_drawing,
            );
        }

        if let Some(draft) = drawing_state.drawing_draft {
            let mut style = draft.style();
            style.stroke_color = style.stroke_color.scale_alpha(0.9);
            if let Some(fill) = style.fill_color {
                style.fill_color = Some(fill.scale_alpha(0.5));
            }

            self.draw_drawing_object(
                frame,
                scene,
                &draft.preview_object(),
                style,
                false,
                overlay_origin_in_window,
                horizontal_pixel_ratio,
            );
        }
    }

    fn point_segment_distance(point: Point, a: Point, b: Point) -> f32 {
        let vx = b.x - a.x;
        let vy = b.y - a.y;
        let wx = point.x - a.x;
        let wy = point.y - a.y;

        let len_sq = vx * vx + vy * vy;
        if len_sq <= f32::EPSILON {
            return ((point.x - a.x).powi(2) + (point.y - a.y).powi(2)).sqrt();
        }

        let t = ((wx * vx + wy * vy) / len_sq).clamp(0.0, 1.0);
        let proj_x = a.x + t * vx;
        let proj_y = a.y + t * vy;

        ((point.x - proj_x).powi(2) + (point.y - proj_y).powi(2)).sqrt()
    }

    fn drawing_hit_test_object(
        &self,
        scene: &Scene,
        object: &DrawingObject,
        style: DrawingStyle,
        point: Point,
    ) -> bool {
        let tolerance = DRAWING_HIT_TOLERANCE_PX;

        match object {
            DrawingObject::Trendline { start, end } => {
                let Some((_, _, visible_start, visible_end, _)) =
                    self.drawing_trendline_geometry(scene, *start, *end)
                else {
                    return false;
                };

                Self::point_segment_distance(point, visible_start, visible_end) <= tolerance
            }
            DrawingObject::Box { start, end } => {
                let Some((origin, size, bounds, visible_bounds)) =
                    self.drawing_box_geometry(scene, *start, *end)
                else {
                    return false;
                };

                let left = origin.x;
                let right = origin.x + size.width;
                let top = origin.y;
                let bottom = origin.y + size.height;

                let within_expanded = point.x >= (visible_bounds.x - tolerance)
                    && point.x <= (visible_bounds.x + visible_bounds.width + tolerance)
                    && point.y >= (visible_bounds.y - tolerance)
                    && point.y <= (visible_bounds.y + visible_bounds.height + tolerance);

                if !within_expanded {
                    return false;
                }

                let top_left = Point::new(left, top);
                let top_right = Point::new(right, top);
                let bottom_right = Point::new(right, bottom);
                let bottom_left = Point::new(left, bottom);

                let near_edge = [
                    (top_left, top_right),
                    (top_right, bottom_right),
                    (bottom_right, bottom_left),
                    (bottom_left, top_left),
                ]
                .into_iter()
                .filter_map(|(edge_start, edge_end)| {
                    Self::clip_segment_to_bounds(edge_start, edge_end, bounds)
                })
                .any(|(edge_start, edge_end)| {
                    Self::point_segment_distance(point, edge_start, edge_end) <= tolerance
                });

                let inside_visible_fill = style.fill_color.is_some()
                    && point.x >= visible_bounds.x
                    && point.x <= (visible_bounds.x + visible_bounds.width)
                    && point.y >= visible_bounds.y
                    && point.y <= (visible_bounds.y + visible_bounds.height);

                near_edge || inside_visible_fill
            }
            DrawingObject::HorizontalLine { panel_id, y_unit } => {
                let Some((bounds, y)) =
                    self.drawing_horizontal_line_geometry(scene, *panel_id, *y_unit)
                else {
                    return false;
                };

                point.x >= (bounds.x - tolerance)
                    && point.x <= (bounds.x + bounds.width + tolerance)
                    && (point.y - y).abs() <= tolerance
            }
            DrawingObject::VerticalLine { time } => {
                let Some((plot, x)) = self.drawing_vertical_line_geometry(scene, *time) else {
                    return false;
                };

                (point.x - x).abs() <= tolerance
                    && point.y >= (plot.y - tolerance)
                    && point.y <= (plot.y + plot.height + tolerance)
            }
        }
    }

    fn hit_test_drawings(&self, scene: &Scene, point: Point) -> Option<DrawingId> {
        self.drawings
            .entities
            .iter()
            .rev()
            .filter(|drawing| drawing.visible)
            .find(|drawing| {
                self.drawing_hit_test_object(scene, &drawing.object, drawing.style, point)
            })
            .map(|drawing| drawing.id)
    }

    fn format_anchor_y_label(&self, scene: &Scene, anchor: DrawingAnchor) -> Option<String> {
        let panel_index = self.panel_index_for_id(anchor.panel_id)?;
        let panel = scene.layout.panel(panel_index)?;
        let panel_precision = self.panel_value_precision(panel_index);
        let value = self.panel_unit_to_value(panel_precision, anchor.y_unit);

        match panel.kind {
            KlinePanelKind::PrimaryChart => Some(scene.format_primary_cursor_label(value)),
            KlinePanelKind::Indicator => {
                Some(self.format_panel_axis_value(panel_index, panel_precision, value, 0.01))
            }
        }
    }

    fn draw_x_axis_badge(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        x: f32,
        text: &str,
    ) {
        let x_label_w = (text.len() as f32 * TEXT_SIZE * 0.62).clamp(60.0, 180.0);
        let x_label_h = TEXT_SIZE + 6.0;
        let x_label_x = (x - x_label_w / 2.0).clamp(
            scene.layout.regions.plot.x,
            scene.layout.regions.plot.x + scene.layout.regions.plot.width - x_label_w,
        );
        let x_label_y = scene.layout.regions.x_axis.y + 2.0;

        frame.fill_rectangle(
            Point::new(x_label_x, x_label_y),
            Size::new(x_label_w, x_label_h),
            palette.background.strong.color,
        );

        frame.fill_text(canvas::Text {
            content: text.to_string(),
            position: Point::new(x_label_x + x_label_w / 2.0, x_label_y + x_label_h / 2.0),
            color: palette.background.strong.text,
            size: TEXT_SIZE.into(),
            align_x: iced::Alignment::Center.into(),
            align_y: iced::Alignment::Center.into(),
            font: style::AZERET_MONO,
            ..Default::default()
        });
    }

    fn draw_y_axis_badge(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        y: f32,
        text: &str,
        clamp_bounds: Rectangle,
    ) {
        let y_label_w = (text.len() as f32 * TEXT_SIZE * 0.6).clamp(40.0, 96.0);
        let y_label_h = TEXT_SIZE + 6.0;
        let y_label_x = scene.layout.regions.y_axis.x + 2.0;
        let y_min = clamp_bounds.y;
        let y_max = (clamp_bounds.y + clamp_bounds.height - y_label_h).max(y_min);
        let y_label_y = (y - (y_label_h / 2.0)).clamp(y_min, y_max);

        frame.fill_rectangle(
            Point::new(y_label_x, y_label_y),
            Size::new(y_label_w, y_label_h),
            palette.background.strong.color,
        );

        frame.fill_text(canvas::Text {
            content: text.to_string(),
            position: Point::new(y_label_x + y_label_w - 4.0, y_label_y + y_label_h / 2.0),
            color: palette.background.strong.text,
            size: TEXT_SIZE.into(),
            align_x: iced::Alignment::End.into(),
            align_y: iced::Alignment::Center.into(),
            font: style::AZERET_MONO,
            ..Default::default()
        });
    }

    fn fill_active_drawing_axis_labels(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
    ) {
        let Some(object) = self.drawings.active_axis_labeled_object() else {
            return;
        };
        let Some((start, end)) = object.axis_label_anchors() else {
            return;
        };

        for anchor in [start, end] {
            let Some(point) = self.anchor_to_overlay_point(scene, anchor) else {
                continue;
            };

            if let Some(x_unit) = self.x_unit_for_anchor_time(scene, anchor.time) {
                let x_text = self.format_x_label(scene.x_axis, x_unit, 1);
                self.draw_x_axis_badge(frame, scene, palette, point.x, &x_text);
            }

            if let Some(y_text) = self.format_anchor_y_label(scene, anchor) {
                let Some(panel_index) = self.panel_index_for_id(anchor.panel_id) else {
                    continue;
                };
                let Some(panel_bounds) = self.panel_plot_bounds_in_overlay(scene, panel_index)
                else {
                    continue;
                };

                self.draw_y_axis_badge(frame, scene, palette, point.y, &y_text, panel_bounds);
            }
        }
    }

    fn panel_id(&self, panel_index: usize) -> Option<composition::PanelId> {
        self.composition
            .panels
            .get(panel_index)
            .map(|panel| panel.id)
    }

    fn panel_value_id(&self, panel_index: usize) -> Option<PanelValueId> {
        self.composition
            .panels
            .get(panel_index)
            .and_then(|panel| panel.value_id)
    }

    fn panel_value_precision(&self, panel_index: usize) -> Option<PanelValuePrecision> {
        self.composition
            .panels
            .get(panel_index)
            .and_then(|panel| panel.value_precision)
    }

    fn panel_value_label_policy(&self, panel_index: usize) -> PanelValueLabelPolicy {
        self.composition
            .panels
            .get(panel_index)
            .map(|panel| panel.value_label_policy)
            .unwrap_or_default()
    }

    fn panel_y_viewport(&self, panel_id: PanelId) -> Option<PanelYViewport> {
        self.panel_y_viewports
            .iter()
            .find(|(id, _)| *id == panel_id)
            .map(|(_, viewport)| *viewport)
    }

    pub(super) fn panel_y_viewport_for_index(&self, panel_index: usize) -> Option<PanelYViewport> {
        self.panel_id(panel_index)
            .and_then(|panel_id| self.panel_y_viewport(panel_id))
    }

    fn panel_accepts_manual_y_from_scene(&self, scene: &Scene, panel_index: usize) -> bool {
        if panel_index == scene.primary_panel {
            !self.primary_autoscale_enabled_for_mode(scene.primary_scale_mode)
        } else {
            true
        }
    }

    fn primary_autoscale_enabled_for_mode(&self, mode: PanelScaleMode) -> bool {
        self.primary_autoscale || matches!(mode, PanelScaleMode::PercentFromBase)
    }

    fn panel_value_domain_from_scene(
        &self,
        scene: &Scene,
        panel_index: usize,
    ) -> Option<(f32, f32)> {
        let panel = scene.layout.panel(panel_index)?;

        match panel.kind {
            KlinePanelKind::PrimaryChart => {
                if panel_index == scene.primary_panel
                    && matches!(scene.primary_scale_mode, PanelScaleMode::PercentFromBase)
                {
                    return None;
                }

                let (min_display, max_display) = scene.primary_domain_display_values();
                PanelYViewport::normalized(
                    scene.primary_display_to_value(min_display),
                    scene.primary_display_to_value(max_display),
                )
                .map(|viewport| (viewport.min_value, viewport.max_value))
            }
            KlinePanelKind::Indicator => {
                let indicator = scene
                    .indicator_panels
                    .iter()
                    .find(|indicator| indicator.panel_index == panel_index)?;
                let precision = self.panel_value_precision(panel_index);

                PanelYViewport::normalized(
                    self.panel_unit_to_value(precision, indicator.min_unit),
                    self.panel_unit_to_value(precision, indicator.max_unit),
                )
                .map(|viewport| (viewport.min_value, viewport.max_value))
            }
        }
    }

    fn panel_viewport_after_y_zoom(
        &self,
        scene: &Scene,
        panel_index: usize,
        cursor_y: f32,
        scroll_delta: f32,
    ) -> Option<PanelYViewport> {
        if scroll_delta.abs() <= f32::EPSILON {
            return None;
        }

        if panel_index == scene.primary_panel
            && matches!(scene.primary_scale_mode, PanelScaleMode::PercentFromBase)
        {
            return None;
        }

        let panel = scene.layout.panel(panel_index)?;
        let panel_h = panel.plot.height.max(1.0);
        let y_in_panel =
            (cursor_y - scene.layout.regions.plot.y - panel.plot.y).clamp(0.0, panel_h);
        let anchor_ratio = (1.0 - (y_in_panel / panel_h)).clamp(0.0, 1.0);

        let zoom_amount = scroll_delta.abs().clamp(0.05, 6.0);
        let zoom_scale = (1.0 + zoom_amount * 0.14).clamp(1.02, 3.0);

        match panel.kind {
            KlinePanelKind::PrimaryChart => {
                let (min_display, max_display) = scene.primary_domain_display_values();
                let display_span = (max_display - min_display).abs().max(1e-6);
                let min_display_span =
                    if matches!(scene.primary_scale_mode, PanelScaleMode::Logarithmic) {
                        1e-3
                    } else {
                        let step = self
                            .panel_quantization_step(self.panel_value_precision(panel_index))
                            .unwrap_or(1e-4)
                            .abs()
                            .max(1e-8);
                        step * 8.0
                    };

                let new_display_span = if scroll_delta > 0.0 {
                    display_span / zoom_scale
                } else {
                    display_span * zoom_scale
                }
                .max(min_display_span);

                let anchor_display = min_display + anchor_ratio * display_span;
                let new_min_display = anchor_display - anchor_ratio * new_display_span;
                let new_max_display = new_min_display + new_display_span;

                PanelYViewport::normalized(
                    scene.primary_display_to_value(new_min_display),
                    scene.primary_display_to_value(new_max_display),
                )
            }
            KlinePanelKind::Indicator => {
                let (min_value, max_value) =
                    self.panel_value_domain_from_scene(scene, panel_index)?;
                let span = (max_value - min_value).abs().max(1e-8);
                let step = self
                    .panel_quantization_step(self.panel_value_precision(panel_index))
                    .unwrap_or(1e-4)
                    .abs()
                    .max(1e-8);
                let min_span = step * 8.0;

                let new_span = if scroll_delta > 0.0 {
                    span / zoom_scale
                } else {
                    span * zoom_scale
                }
                .max(min_span);

                let anchor_value = min_value + anchor_ratio * span;
                let new_min = anchor_value - anchor_ratio * new_span;
                let new_max = new_min + new_span;

                PanelYViewport::normalized(new_min, new_max)
            }
        }
    }

    fn panel_viewport_after_primary_y_pan(
        &self,
        scene: &Scene,
        panel_index: usize,
        dy_px: f32,
    ) -> Option<PanelYViewport> {
        if dy_px.abs() <= f32::EPSILON {
            return None;
        }

        if panel_index != scene.primary_panel
            || !self.panel_accepts_manual_y_from_scene(scene, panel_index)
        {
            return None;
        }

        let panel = scene.layout.panel(panel_index)?;
        let panel_h = panel.plot.height.max(1.0);
        let delta_ratio = dy_px / panel_h;

        let (min_display, max_display) = scene.primary_domain_display_values();
        let display_span = (max_display - min_display).abs().max(1e-6);
        let shift = delta_ratio * display_span;

        PanelYViewport::normalized(
            scene.primary_display_to_value(min_display + shift),
            scene.primary_display_to_value(max_display + shift),
        )
    }

    fn panel_uses_signed_overlay_input(&self, panel_index: usize) -> bool {
        let panel_value = self.panel_value_id(panel_index);

        let Some(base_series) = self.series.first() else {
            return false;
        };

        base_series.bars().iter().any(|bar| {
            base_series
                .indicator_data_for_panel_value_opt(panel_value, bar)
                .and_then(IndicatorData::signed_overlay)
                .is_some()
        })
    }

    fn normalized_panel_splits(&self) -> Vec<f32> {
        if self.panel_count() <= 1 {
            Vec::new()
        } else {
            self.composition.normalized_splits(DEFAULT_MIN_PANEL_RATIO)
        }
    }

    fn default_mark_for_panel(kind: KlinePanelKind) -> MarkKind {
        match kind {
            KlinePanelKind::PrimaryChart => MarkKind::Candle,
            KlinePanelKind::Indicator => MarkKind::Bar(BarMode::Histogram(HistogramMode::Plain)),
        }
    }

    fn default_title_for_panel(kind: KlinePanelKind) -> Option<&'static str> {
        match kind {
            KlinePanelKind::PrimaryChart => None,
            KlinePanelKind::Indicator => Some("Indicator"),
        }
    }

    fn resolved_panel_title(&self, panel_index: usize, panel_kind: KlinePanelKind) -> Option<&str> {
        self.composition
            .panels
            .get(panel_index)
            .and_then(|panel| panel.title.as_deref())
            .filter(|title| !title.is_empty())
            .or_else(|| Self::default_title_for_panel(panel_kind))
    }

    fn resolved_panel_mark(&self, panel_index: usize, panel_kind: KlinePanelKind) -> MarkKind {
        let Some(panel_id) = self.panel_id(panel_index) else {
            return Self::default_mark_for_panel(panel_kind);
        };

        self.composition
            .panel_effective_mark_with_runtime(
                panel_id,
                self.panel_uses_signed_overlay_input(panel_index),
            )
            .unwrap_or_else(|| Self::default_mark_for_panel(panel_kind))
    }

    fn resolved_panel_scale_mode(&self, panel_index: usize) -> PanelScaleMode {
        let Some(panel) = self.composition.panels.get(panel_index) else {
            return PanelScaleMode::Absolute;
        };

        let mut scale = self
            .composition
            .panel_effective_scale_mode(panel.id)
            .unwrap_or(PanelScaleMode::Absolute);

        if matches!(panel.value_id, Some(PanelValueId::Volume))
            && matches!(scale, PanelScaleMode::Absolute)
        {
            scale = PanelScaleMode::FitVisibleIncludeZero;
        }

        scale
    }

    fn default_data_kind_for_panel(kind: KlinePanelKind) -> LayerDataKind {
        match kind {
            KlinePanelKind::PrimaryChart => LayerDataKind::Ohlc,
            KlinePanelKind::Indicator => LayerDataKind::Scalar,
        }
    }

    fn resolved_panel_data_kind(
        &self,
        panel_index: usize,
        panel_kind: KlinePanelKind,
    ) -> LayerDataKind {
        let Some(panel_id) = self.panel_id(panel_index) else {
            return Self::default_data_kind_for_panel(panel_kind);
        };

        self.composition
            .panel_effective_data_kind(panel_id)
            .unwrap_or_else(|| Self::default_data_kind_for_panel(panel_kind))
    }

    fn comparison_line_color(ticker: &TickerInfo) -> iced::Color {
        use std::hash::{DefaultHasher, Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        ticker.hash(&mut hasher);
        let seed = hasher.finish();

        let golden = 0.618_034_f32;
        let base = ((seed as f32 / u64::MAX as f32) + 0.12345).fract();
        let hue = (base + golden).fract() * 360.0;

        let saturation = 0.62 + (((seed >> 8) & 0xFF) as f32 / 255.0) * 0.2;
        let value = 0.82 + (((seed >> 16) & 0x7F) as f32 / 127.0) * 0.12;

        data::config::theme::from_hsv_degrees(hue, saturation.min(1.0), value.min(1.0))
    }

    fn collect_primary_overlay_value_ids(&self, out: &mut Vec<PanelValueId>) {
        out.clear();

        let Some(primary_panel_id) = self.composition.primary_panel_id() else {
            return;
        };

        let Some(primary_panel) = self.composition.panel(primary_panel_id) else {
            return;
        };

        out.extend(
            primary_panel
                .layers
                .iter()
                .filter_map(|layer| layer.source.indicator_value_id()),
        );
    }

    pub(super) fn primary_overlay_value_ids(&self) -> Vec<PanelValueId> {
        let mut out = Vec::new();
        self.collect_primary_overlay_value_ids(&mut out);
        out
    }

    pub(super) fn overlay_channels_for_panel_value(
        &self,
        value_id: Option<PanelValueId>,
    ) -> &'static [OverlayChannelSpec] {
        if let Some(value_id) = value_id
            && let Some(series) = self.series.first()
        {
            let channels = series.indicator_overlay_channels_for_panel_value(value_id);
            if !channels.is_empty() {
                return channels;
            }
        }

        &DEFAULT_OVERLAY_CHANNELS
    }

    pub(super) fn overlay_channel_value(
        data: IndicatorData,
        channel: OverlayChannelSpec,
    ) -> Option<f32> {
        channel
            .key
            .map_or_else(|| Some(data.value()), |key| data.field(key))
    }

    pub(super) fn overlay_channel_color(
        channel: OverlayChannelSpec,
        palette: &Extended,
    ) -> iced::Color {
        match channel.color_role {
            OverlayChannelColorRole::Neutral => palette.background.base.text.scale_alpha(0.72),
            OverlayChannelColorRole::Success => palette.success.base.color.scale_alpha(0.78),
            OverlayChannelColorRole::Danger => palette.danger.base.color.scale_alpha(0.78),
            OverlayChannelColorRole::Primary => palette.primary.base.color.scale_alpha(0.78),
        }
    }

    fn panel_quantization_step(&self, value_precision: Option<PanelValuePrecision>) -> Option<f32> {
        match value_precision {
            Some(PanelValuePrecision::BaseTickerMinTick) => self
                .series
                .first()
                .map(|series| series.ticker_info().min_ticksize.as_f32()),
            Some(PanelValuePrecision::BaseTickerMinQty) => self
                .series
                .first()
                .map(|series| series.ticker_info().min_qty.as_f32()),
            Some(PanelValuePrecision::FixedPower10(step)) => Some(step.as_f32()),
            Some(PanelValuePrecision::FixedStep(step)) => Some(step),
            None => None,
        }
        .map(|step| step.abs().max(1e-8))
    }

    fn decimals_from_power10(power: i8) -> usize {
        if power < 0 { (-power) as usize } else { 0 }
    }

    fn decimals_for_step(step: f32) -> usize {
        let step = step.abs();
        if !step.is_finite() || step <= 0.0 {
            return 4;
        }

        for decimals in 0..=8 {
            let scaled = (step as f64) * 10_f64.powi(decimals as i32);
            let nearest = scaled.round();
            let tolerance = (scaled.abs() * 1e-9).max(1e-12);

            if (scaled - nearest).abs() <= tolerance {
                return decimals;
            }
        }

        8
    }

    fn panel_value_decimals(&self, value_precision: Option<PanelValuePrecision>) -> Option<usize> {
        match value_precision {
            Some(PanelValuePrecision::BaseTickerMinTick) => self
                .series
                .first()
                .map(|series| Self::decimals_from_power10(series.ticker_info().min_ticksize.power)),
            Some(PanelValuePrecision::BaseTickerMinQty) => self
                .series
                .first()
                .map(|series| Self::decimals_from_power10(series.ticker_info().min_qty.power)),
            Some(PanelValuePrecision::FixedPower10(step)) => {
                Some(Self::decimals_from_power10(step.power))
            }
            Some(PanelValuePrecision::FixedStep(step)) => Some(Self::decimals_for_step(step)),
            None => None,
        }
    }

    fn pow10_i64(exp: i32) -> Option<i64> {
        if exp < 0 {
            return None;
        }

        10_i64.checked_pow(exp as u32)
    }

    fn round_to_i64_saturating(value: f32) -> i64 {
        if !value.is_finite() {
            return 0;
        }

        let rounded = value.round();
        if rounded > i64::MAX as f32 {
            i64::MAX
        } else if rounded < i64::MIN as f32 {
            i64::MIN
        } else {
            rounded as i64
        }
    }

    fn panel_value_to_unit(
        &self,
        value_precision: Option<PanelValuePrecision>,
        value: f32,
    ) -> YUnit {
        if !value.is_finite() {
            return YUnit(0);
        }

        match value_precision {
            Some(PanelValuePrecision::BaseTickerMinTick) => {
                if let Some(series) = self.series.first() {
                    let min_tick = series.ticker_info().min_ticksize;
                    let exp = 8 + i32::from(min_tick.power);

                    if let Some(unit_size) = Self::pow10_i64(exp) {
                        let price_units = Price::from_f32(value).round_to_min_tick(min_tick).units;
                        return YUnit(price_units.div_euclid(unit_size));
                    }
                }
            }
            Some(PanelValuePrecision::BaseTickerMinQty) => {
                if let Some(series) = self.series.first() {
                    let min_qty = series.ticker_info().min_qty;
                    let exp = Qty::QTY_SCALE + i32::from(min_qty.power);

                    if let Some(unit_size) = Self::pow10_i64(exp) {
                        let qty_units = Qty::from_f32(value).round_to_min_qty(min_qty).units;
                        return YUnit(qty_units.div_euclid(unit_size));
                    }
                }
            }
            Some(PanelValuePrecision::FixedPower10(step)) => {
                let step = step.as_f32().abs().max(1e-8);
                return YUnit(Self::round_to_i64_saturating(value / step));
            }
            Some(PanelValuePrecision::FixedStep(step)) => {
                let step = step.abs().max(1e-8);
                return YUnit(Self::round_to_i64_saturating(value / step));
            }
            None => {}
        }

        let step = self
            .panel_quantization_step(value_precision)
            .unwrap_or(1e-4)
            .max(1e-8);
        YUnit(Self::round_to_i64_saturating(value / step))
    }

    fn panel_unit_to_value(
        &self,
        value_precision: Option<PanelValuePrecision>,
        y_unit: YUnit,
    ) -> f32 {
        match value_precision {
            Some(PanelValuePrecision::BaseTickerMinTick) => {
                if let Some(series) = self.series.first() {
                    let min_tick = series.ticker_info().min_ticksize;
                    let exp = 8 + i32::from(min_tick.power);

                    if let Some(unit_size) = Self::pow10_i64(exp) {
                        let units = y_unit.0.saturating_mul(unit_size);
                        return Price::from_units(units).to_f32();
                    }
                }
            }
            Some(PanelValuePrecision::BaseTickerMinQty) => {
                if let Some(series) = self.series.first() {
                    let min_qty = series.ticker_info().min_qty;
                    let exp = Qty::QTY_SCALE + i32::from(min_qty.power);

                    if let Some(unit_size) = Self::pow10_i64(exp) {
                        let units = y_unit.0.saturating_mul(unit_size);
                        return f32::from(Qty::from_units(units));
                    }
                }
            }
            Some(PanelValuePrecision::FixedPower10(step)) => {
                return (y_unit.0 as f32) * step.as_f32().abs().max(1e-8);
            }
            Some(PanelValuePrecision::FixedStep(step)) => {
                return (y_unit.0 as f32) * step.abs().max(1e-8);
            }
            None => {}
        }

        let step = self
            .panel_quantization_step(value_precision)
            .unwrap_or(1e-4)
            .max(1e-8);
        (y_unit.0 as f32) * step
    }

    fn quantize_panel_value(
        &self,
        value_precision: Option<PanelValuePrecision>,
        value: f32,
    ) -> f32 {
        let y_unit = self.panel_value_to_unit(value_precision, value);
        self.panel_unit_to_value(value_precision, y_unit)
    }

    fn format_panel_value_compact(
        &self,
        panel_index: usize,
        value_precision: Option<PanelValuePrecision>,
        value: f32,
        fallback_step: f32,
    ) -> String {
        let quantized = self.quantize_panel_value(value_precision, value);
        let fallback = fallback_step.abs().max(1e-6);
        let step = self
            .panel_quantization_step(value_precision)
            .map(|panel_step| panel_step.max(fallback))
            .unwrap_or(fallback);

        let decimals = self
            .panel_value_decimals(value_precision)
            .unwrap_or_else(|| Self::decimals_for_step(step));
        let decimals = self
            .panel_value_label_policy(panel_index)
            .max_decimals
            .map(|max| decimals.min(max as usize))
            .unwrap_or(decimals);
        format!("{quantized:.decimals$}")
    }

    fn format_panel_value_by_mode(
        &self,
        panel_index: usize,
        value_precision: Option<PanelValuePrecision>,
        value: f32,
        fallback_step: f32,
        mode: PanelValueLabelMode,
    ) -> String {
        match mode {
            PanelValueLabelMode::Compact => {
                self.format_panel_value_compact(panel_index, value_precision, value, fallback_step)
            }
            PanelValueLabelMode::Commas => {
                data::util::format_with_commas(self.quantize_panel_value(value_precision, value))
            }
            PanelValueLabelMode::Abbreviated => {
                data::util::abbr_large_numbers(self.quantize_panel_value(value_precision, value))
            }
        }
    }

    fn format_panel_axis_value(
        &self,
        panel_index: usize,
        value_precision: Option<PanelValuePrecision>,
        value: f32,
        fallback_step: f32,
    ) -> String {
        let mode = self.panel_value_label_policy(panel_index).axis_mode;
        self.format_panel_value_by_mode(panel_index, value_precision, value, fallback_step, mode)
    }

    pub(super) fn format_panel_header_value(
        &self,
        panel_index: usize,
        value_precision: Option<PanelValuePrecision>,
        value: f32,
    ) -> String {
        let mode = self.panel_value_label_policy(panel_index).header_mode;
        self.format_panel_value_by_mode(panel_index, value_precision, value, 0.01, mode)
    }

    fn optimal_candlestick_width(bar_spacing: f32, pixel_ratio: f32) -> i32 {
        let from = 2.5_f32;
        let to = 4.0_f32;
        let coeff_special = 3.0_f32;

        if bar_spacing >= from && bar_spacing <= to {
            return (coeff_special * pixel_ratio).floor() as i32;
        }

        let reducing_coeff = 0.2_f32;
        let coeff = 1.0
            - (reducing_coeff * (bar_spacing.max(to) - to).atan()) / (std::f32::consts::PI * 0.5);

        let res = (bar_spacing * coeff * pixel_ratio).floor() as i32;
        let scaled_bar_spacing = (bar_spacing * pixel_ratio).floor() as i32;
        let optimal = res.min(scaled_bar_spacing);

        optimal.max(pixel_ratio.floor() as i32)
    }

    fn candlestick_width(bar_spacing: f32, horizontal_pixel_ratio: f32) -> i32 {
        let mut width = Self::optimal_candlestick_width(bar_spacing, horizontal_pixel_ratio);
        if width >= 2 {
            let wick_width = horizontal_pixel_ratio.floor() as i32;
            if (wick_width & 1) != (width & 1) {
                width -= 1;
            }
        }
        width
    }

    fn resolved_horizontal_pixel_ratio(&self) -> f32 {
        if self.horizontal_pixel_ratio.is_finite() && self.horizontal_pixel_ratio > 0.0 {
            self.horizontal_pixel_ratio
        } else {
            1.0
        }
    }

    fn physical_px_to_logical(px: i32, horizontal_pixel_ratio: f32) -> f32 {
        px as f32 / horizontal_pixel_ratio
    }

    fn physical_px_to_logical_with_origin(
        px: i32,
        horizontal_pixel_ratio: f32,
        origin_global: f32,
    ) -> f32 {
        Self::physical_px_to_logical(px, horizontal_pixel_ratio) - origin_global
    }

    fn snap_axis_to_physical_with_origin(
        value_local: f32,
        horizontal_pixel_ratio: f32,
        origin_global: f32,
    ) -> i32 {
        ((value_local + origin_global) * horizontal_pixel_ratio).round() as i32
    }

    fn snap_plot_x_to_cell_with_origin(
        x_plot: f32,
        horizontal_pixel_ratio: f32,
        origin_x_global: f32,
    ) -> f32 {
        let x_phys = Self::snap_axis_to_physical_with_origin(
            x_plot,
            horizontal_pixel_ratio,
            origin_x_global,
        );
        Self::physical_px_to_logical_with_origin(x_phys, horizontal_pixel_ratio, origin_x_global)
    }

    fn centered_left_for_width_with_origin(
        x_plot: f32,
        width_phys: i32,
        horizontal_pixel_ratio: f32,
        origin_x_global: f32,
    ) -> f32 {
        let center_phys = Self::snap_axis_to_physical_with_origin(
            x_plot,
            horizontal_pixel_ratio,
            origin_x_global,
        );
        let left_phys = center_phys - (width_phys / 2);
        Self::physical_px_to_logical_with_origin(left_phys, horizontal_pixel_ratio, origin_x_global)
    }

    fn snapped_span_with_origin(
        start_local: f32,
        end_local: f32,
        horizontal_pixel_ratio: f32,
        origin_y_global: f32,
    ) -> (f32, f32) {
        let top = start_local.min(end_local);
        let bottom = start_local.max(end_local);

        let top_phys =
            Self::snap_axis_to_physical_with_origin(top, horizontal_pixel_ratio, origin_y_global);
        let mut bottom_phys = Self::snap_axis_to_physical_with_origin(
            bottom,
            horizontal_pixel_ratio,
            origin_y_global,
        );

        if bottom_phys <= top_phys {
            bottom_phys = top_phys + 1;
        }

        let top_snapped = Self::physical_px_to_logical_with_origin(
            top_phys,
            horizontal_pixel_ratio,
            origin_y_global,
        );
        let height_snapped = (bottom_phys - top_phys) as f32 / horizontal_pixel_ratio;

        (top_snapped, height_snapped)
    }

    fn quantized_stroke_width(width_logical: f32, horizontal_pixel_ratio: f32) -> (f32, i32) {
        let stroke_width_phys = (width_logical.max(0.0) * horizontal_pixel_ratio).round() as i32;
        let stroke_width_phys = stroke_width_phys.max(1);
        (
            Self::logical_width_from_physical(stroke_width_phys, horizontal_pixel_ratio),
            stroke_width_phys,
        )
    }

    fn snap_stroke_center_with_origin(
        value_local: f32,
        horizontal_pixel_ratio: f32,
        origin_global: f32,
        stroke_width_phys: i32,
    ) -> f32 {
        let axis_phys = (value_local + origin_global) * horizontal_pixel_ratio;
        let snapped_phys = if (stroke_width_phys & 1) == 0 {
            axis_phys.round()
        } else {
            (axis_phys - 0.5).round() + 0.5
        };

        (snapped_phys / horizontal_pixel_ratio) - origin_global
    }

    fn snap_plot_x_for_stroke_with_origin(
        x_plot: f32,
        horizontal_pixel_ratio: f32,
        origin_x_global: f32,
        stroke_width_phys: i32,
    ) -> f32 {
        let x_cell =
            Self::snap_plot_x_to_cell_with_origin(x_plot, horizontal_pixel_ratio, origin_x_global);

        Self::snap_stroke_center_with_origin(
            x_cell,
            horizontal_pixel_ratio,
            origin_x_global,
            stroke_width_phys,
        )
    }

    fn snap_point_for_stroke_with_origin(
        point: Point,
        horizontal_pixel_ratio: f32,
        origin_global: Point,
        stroke_width_phys: i32,
    ) -> Point {
        Point::new(
            Self::snap_stroke_center_with_origin(
                point.x,
                horizontal_pixel_ratio,
                origin_global.x,
                stroke_width_phys,
            ),
            Self::snap_stroke_center_with_origin(
                point.y,
                horizontal_pixel_ratio,
                origin_global.y,
                stroke_width_phys,
            ),
        )
    }

    fn snap_plot_x_to_cell(x_plot: f32, horizontal_pixel_ratio: f32) -> f32 {
        let x_phys = (x_plot * horizontal_pixel_ratio).round() as i32;
        Self::physical_px_to_logical(x_phys, horizontal_pixel_ratio)
    }

    fn logical_width_from_physical(width_phys: i32, horizontal_pixel_ratio: f32) -> f32 {
        Self::physical_px_to_logical(width_phys.max(1), horizontal_pixel_ratio)
    }

    fn primitive_width_for_spacing(
        bar_spacing: f32,
        width_factor: f32,
        horizontal_pixel_ratio: f32,
    ) -> i32 {
        let scaled_spacing = (bar_spacing * width_factor).max(1e-6);
        Self::optimal_candlestick_width(scaled_spacing, horizontal_pixel_ratio)
    }

    fn fill_main_geometry(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        plot_origin_in_window: Point,
    ) {
        let horizontal_pixel_ratio = self.resolved_horizontal_pixel_ratio();
        let plot_origin_x_global = plot_origin_in_window.x;
        let plot_origin_y_global = plot_origin_in_window.y;

        let spacing = scene.bar_spacing_px();
        let px_per_unit = spacing.as_f32();
        let max_width_phys = ((px_per_unit * horizontal_pixel_ratio).floor() as i32).max(1);

        let mut candle_body_width_phys =
            Self::candlestick_width(px_per_unit, horizontal_pixel_ratio).clamp(1, max_width_phys);
        let wick_width_phys = (horizontal_pixel_ratio.floor() as i32)
            .max(1)
            .min(candle_body_width_phys);

        if candle_body_width_phys >= 2 && ((wick_width_phys ^ candle_body_width_phys) & 1) != 0 {
            candle_body_width_phys -= 1;
        }

        let indicator_width_phys =
            Self::primitive_width_for_spacing(px_per_unit, 0.8, horizontal_pixel_ratio)
                .clamp(1, max_width_phys);
        let indicator_width =
            Self::logical_width_from_physical(indicator_width_phys, horizontal_pixel_ratio);
        let candle_body_width =
            Self::logical_width_from_physical(candle_body_width_phys, horizontal_pixel_ratio);
        let wick_width = Self::logical_width_from_physical(wick_width_phys, horizontal_pixel_ratio);
        let in_visible_range =
            |x_unit: i64| x_unit >= scene.min_x_unit && x_unit <= scene.max_x_unit;

        for indicator_panel in &scene.indicator_panels {
            if !matches!(indicator_panel.mark, MarkKind::Candle | MarkKind::Bar(_)) {
                continue;
            }

            let Some(panel_plot) = scene
                .layout
                .panel(indicator_panel.panel_index)
                .map(|panel| panel.plot)
            else {
                continue;
            };

            let Some(y_indicator_baseline) = scene
                .map_indicator_plot_unclamped(indicator_panel.panel_index, 0.0)
                .or_else(|| scene.indicator_panel_bottom(indicator_panel.panel_index))
            else {
                continue;
            };

            frame.with_clip(panel_plot, |frame| {
                self.for_each_bar_unit_index(scene.x_axis, |series_index, series, x_unit, bar| {
                    if series_index != 0 || !in_visible_range(x_unit) {
                        return;
                    }

                    let Some(indicator_data) =
                        series.indicator_data_for_panel_value_opt(indicator_panel.value_id, bar)
                    else {
                        return;
                    };

                    let indicator_value = indicator_data.value();
                    let Some(y_indicator_value) = scene
                        .map_indicator_plot_unclamped(indicator_panel.panel_index, indicator_value)
                    else {
                        return;
                    };

                    let x_plot = scene.map_x_plot(x_unit);
                    let indicator_left = Self::centered_left_for_width_with_origin(
                        x_plot,
                        indicator_width_phys,
                        horizontal_pixel_ratio,
                        plot_origin_x_global,
                    );
                    let indicator_top = y_indicator_value.min(y_indicator_baseline);
                    let indicator_height =
                        (y_indicator_baseline - y_indicator_value).abs().max(1.0);

                    if matches!(indicator_panel.data_kind, LayerDataKind::Histogram)
                        && matches!(
                            indicator_panel.mark,
                            MarkKind::Bar(BarMode::Histogram(HistogramMode::SignedOverlay))
                        )
                    {
                        if let Some(overlay) = indicator_data.signed_overlay() {
                            let base_color = if overlay >= 0.0 {
                                palette.success.base.color
                            } else {
                                palette.danger.base.color
                            };

                            frame.fill_rectangle(
                                Point::new(indicator_left, indicator_top),
                                Size::new(indicator_width, indicator_height),
                                base_color.scale_alpha(0.3),
                            );

                            let overlay_abs = overlay.abs();
                            if overlay_abs > 0.0
                                && let Some(y_overlay) = scene.map_indicator_plot_unclamped(
                                    indicator_panel.panel_index,
                                    overlay_abs,
                                )
                            {
                                frame.fill_rectangle(
                                    Point::new(indicator_left, y_overlay.min(y_indicator_baseline)),
                                    Size::new(
                                        indicator_width,
                                        (y_indicator_baseline - y_overlay).abs().max(1.0),
                                    ),
                                    base_color,
                                );
                            }
                        } else {
                            frame.fill_rectangle(
                                Point::new(indicator_left, indicator_top),
                                Size::new(indicator_width, indicator_height),
                                palette.secondary.strong.color,
                            );
                        }
                    } else {
                        frame.fill_rectangle(
                            Point::new(indicator_left, indicator_top),
                            Size::new(indicator_width, indicator_height),
                            palette.secondary.strong.color.scale_alpha(0.5),
                        );
                    }
                });
            });
        }

        if let Some(primary_panel_id) = self.composition.primary_panel_id()
            && let Some(primary_panel) = self.composition.panel(primary_panel_id)
        {
            let base_series_anchor = scene.series_percent_anchors.first().copied().flatten();
            let primary_plot = *scene.primary_plot();

            frame.with_clip(primary_plot, |frame| {
                for layer in &primary_panel.layers {
                    let Some(value_id) = layer.source.indicator_value_id() else {
                        continue;
                    };

                    for channel in self
                        .overlay_channels_for_panel_value(Some(value_id))
                        .iter()
                        .copied()
                    {
                        let (channel_line_width, channel_line_width_phys) =
                            Self::quantized_stroke_width(
                                channel.line_width,
                                horizontal_pixel_ratio,
                            );
                        let mut point_count = 0usize;
                        let path = canvas::Path::new(|builder| {
                            self.for_each_bar_unit_index(
                                scene.x_axis,
                                |series_index, series, x_unit, bar| {
                                    if series_index != 0 || !in_visible_range(x_unit) {
                                        return;
                                    }

                                    let Some(data) = series
                                        .indicator_data_for_panel_value_opt(Some(value_id), bar)
                                    else {
                                        return;
                                    };

                                    let Some(value) = Self::overlay_channel_value(data, channel)
                                    else {
                                        return;
                                    };

                                    let point = Point::new(
                                        Self::snap_plot_x_for_stroke_with_origin(
                                            scene.map_x_plot(x_unit),
                                            horizontal_pixel_ratio,
                                            plot_origin_x_global,
                                            channel_line_width_phys,
                                        ),
                                        Self::snap_stroke_center_with_origin(
                                            scene.map_primary_plot_with_anchor_unclamped(
                                                value,
                                                base_series_anchor,
                                            ),
                                            horizontal_pixel_ratio,
                                            plot_origin_y_global,
                                            channel_line_width_phys,
                                        ),
                                    );

                                    if point_count == 0 {
                                        builder.move_to(point);
                                    } else {
                                        builder.line_to(point);
                                    }
                                    point_count += 1;
                                },
                            );
                        });

                        if point_count < 2 {
                            continue;
                        }

                        frame.stroke(
                            &path,
                            canvas::Stroke::default()
                                .with_width(channel_line_width)
                                .with_color(Self::overlay_channel_color(channel, palette)),
                        );
                    }
                }
            });
        }

        if matches!(scene.primary_mark, MarkKind::Candle | MarkKind::Bar(_)) {
            let primary_plot = *scene.primary_plot();
            frame.with_clip(primary_plot, |frame| {
                self.for_each_bar_unit_index(scene.x_axis, |series_index, _series, x_unit, bar| {
                    if series_index != 0 || !in_visible_range(x_unit) {
                        return;
                    }

                    let series_anchor = scene.series_percent_anchors.first().copied().flatten();
                    let y_open = scene
                        .map_primary_plot_with_anchor_unclamped(bar.open.to_f32(), series_anchor);
                    let y_high = scene
                        .map_primary_plot_with_anchor_unclamped(bar.high.to_f32(), series_anchor);
                    let y_low = scene
                        .map_primary_plot_with_anchor_unclamped(bar.low.to_f32(), series_anchor);
                    let y_close = scene
                        .map_primary_plot_with_anchor_unclamped(bar.close.to_f32(), series_anchor);

                    let color = if bar.close >= bar.open {
                        palette.success.base.color
                    } else {
                        palette.danger.base.color
                    };

                    let (body_top, body_h) = Self::snapped_span_with_origin(
                        y_open,
                        y_close,
                        horizontal_pixel_ratio,
                        plot_origin_y_global,
                    );
                    let (wick_top, wick_h) = Self::snapped_span_with_origin(
                        y_high,
                        y_low,
                        horizontal_pixel_ratio,
                        plot_origin_y_global,
                    );

                    let x_plot = scene.map_x_plot(x_unit);
                    let candle_left = Self::centered_left_for_width_with_origin(
                        x_plot,
                        candle_body_width_phys,
                        horizontal_pixel_ratio,
                        plot_origin_x_global,
                    );
                    let wick_left = Self::centered_left_for_width_with_origin(
                        x_plot,
                        wick_width_phys,
                        horizontal_pixel_ratio,
                        plot_origin_x_global,
                    );

                    frame.fill_rectangle(
                        Point::new(candle_left, body_top),
                        Size::new(candle_body_width, body_h),
                        color,
                    );
                    frame.fill_rectangle(
                        Point::new(wick_left, wick_top),
                        Size::new(wick_width, wick_h),
                        color.scale_alpha(0.85),
                    );
                });
            });
        }

        let primary_plot = *scene.primary_plot();
        frame.with_clip(primary_plot, |frame| {
            for series_index in 0..self.series.len() {
                let is_base_series = series_index == 0;
                let requested_line_width = if is_base_series { 1.5 } else { 1.3 };
                let (line_width, line_width_phys) =
                    Self::quantized_stroke_width(requested_line_width, horizontal_pixel_ratio);
                let mut point_count = 0usize;

                let path = canvas::Path::new(|builder| {
                    self.for_each_bar_unit_index(
                        scene.x_axis,
                        |iter_series_index, _series, x_unit, bar| {
                            if iter_series_index != series_index || !in_visible_range(x_unit) {
                                return;
                            }

                            let is_base_series = iter_series_index == 0;
                            if is_base_series && !matches!(scene.primary_mark, MarkKind::Line) {
                                return;
                            }

                            let series_anchor = scene
                                .series_percent_anchors
                                .get(iter_series_index)
                                .copied()
                                .flatten();
                            let point = Point::new(
                                Self::snap_plot_x_for_stroke_with_origin(
                                    scene.map_x_plot(x_unit),
                                    horizontal_pixel_ratio,
                                    plot_origin_x_global,
                                    line_width_phys,
                                ),
                                Self::snap_stroke_center_with_origin(
                                    scene.map_primary_plot_with_anchor_unclamped(
                                        bar.close.to_f32(),
                                        series_anchor,
                                    ),
                                    horizontal_pixel_ratio,
                                    plot_origin_y_global,
                                    line_width_phys,
                                ),
                            );

                            if point_count == 0 {
                                builder.move_to(point);
                            } else {
                                builder.line_to(point);
                            }
                            point_count += 1;
                        },
                    );
                });

                if point_count < 2 {
                    continue;
                }

                let line_color = if is_base_series {
                    palette.background.base.text.scale_alpha(0.85)
                } else {
                    let ticker = self.series[series_index].ticker_info();
                    Self::comparison_line_color(ticker).scale_alpha(0.96)
                };

                frame.stroke(
                    &path,
                    canvas::Stroke::default()
                        .with_width(line_width)
                        .with_color(line_color),
                );
            }
        });

        for indicator_panel in &scene.indicator_panels {
            if !matches!(indicator_panel.mark, MarkKind::Line) {
                continue;
            }

            let Some(panel_plot) = scene
                .layout
                .panel(indicator_panel.panel_index)
                .map(|panel| panel.plot)
            else {
                continue;
            };

            frame.with_clip(panel_plot, |frame| {
                for channel in self
                    .overlay_channels_for_panel_value(indicator_panel.value_id)
                    .iter()
                    .copied()
                {
                    let (channel_line_width, channel_line_width_phys) =
                        Self::quantized_stroke_width(channel.line_width, horizontal_pixel_ratio);
                    let mut point_count = 0usize;

                    let path = canvas::Path::new(|builder| {
                        self.for_each_bar_unit_index(
                            scene.x_axis,
                            |series_index, series, x_unit, bar| {
                                if series_index != 0 || !in_visible_range(x_unit) {
                                    return;
                                }

                                let Some(indicator_data) = series
                                    .indicator_data_for_panel_value_opt(
                                        indicator_panel.value_id,
                                        bar,
                                    )
                                else {
                                    return;
                                };

                                let Some(channel_value) =
                                    Self::overlay_channel_value(indicator_data, channel)
                                else {
                                    return;
                                };

                                let Some(y_channel_value) = scene.map_indicator_plot_unclamped(
                                    indicator_panel.panel_index,
                                    channel_value,
                                ) else {
                                    return;
                                };

                                let point = Point::new(
                                    Self::snap_plot_x_for_stroke_with_origin(
                                        scene.map_x_plot(x_unit),
                                        horizontal_pixel_ratio,
                                        plot_origin_x_global,
                                        channel_line_width_phys,
                                    ),
                                    Self::snap_stroke_center_with_origin(
                                        y_channel_value,
                                        horizontal_pixel_ratio,
                                        plot_origin_y_global,
                                        channel_line_width_phys,
                                    ),
                                );
                                if point_count == 0 {
                                    builder.move_to(point);
                                } else {
                                    builder.line_to(point);
                                }
                                point_count += 1;
                            },
                        );
                    });

                    if point_count < 2 {
                        continue;
                    }

                    frame.stroke(
                        &path,
                        canvas::Stroke::default()
                            .with_width(channel_line_width)
                            .with_color(Self::overlay_channel_color(channel, palette)),
                    );
                }
            });
        }

        self.fill_panel_titles(frame, scene, palette);
    }

    fn fill_panel_titles(&self, frame: &mut canvas::Frame, scene: &Scene, palette: &Extended) {
        for (panel_index, panel) in scene.layout.panels.iter().enumerate() {
            let Some(title) = self.resolved_panel_title(panel_index, panel.kind) else {
                continue;
            };

            let title_x = panel.plot.x + PANEL_TITLE_LEFT_PAD;
            let title_y = panel.plot.y + PANEL_TITLE_TOP_PAD;

            frame.fill_text(canvas::Text {
                content: title.to_string(),
                position: Point::new(title_x, title_y),
                color: palette.background.base.text.scale_alpha(0.72),
                size: (TEXT_SIZE - 1.0).into(),
                align_x: iced::Alignment::Start.into(),
                align_y: iced::Alignment::Start.into(),
                font: style::AZERET_MONO,
                ..Default::default()
            });
        }
    }

    fn fill_y_axis_labels(&self, frame: &mut canvas::Frame, scene: &Scene, palette: &Extended) {
        let min_tick_px = (TEXT_SIZE * 2.5).max(20.0);
        let uses_display_space_ticks = matches!(
            scene.primary_scale_mode,
            PanelScaleMode::PercentFromBase | PanelScaleMode::Logarithmic
        ) || scene.primary_domain_display_override.is_some();

        if !uses_display_space_ticks {
            let (ticks, step_units) = super::unit_ticks(
                scene.min_primary_unit.0,
                scene.max_primary_unit.0,
                scene.primary_plot().height,
                min_tick_px,
            );

            let primary_precision = self.panel_value_precision(scene.primary_panel);
            let zero_value = self.panel_unit_to_value(primary_precision, YUnit(0));
            let step_value = (self.panel_unit_to_value(primary_precision, YUnit(step_units))
                - zero_value)
                .abs()
                .max(1e-8);

            for tick in ticks {
                let tick_unit = YUnit(tick);
                let value = self.panel_unit_to_value(primary_precision, tick_unit);
                let y = scene.map_primary_plot_unit_clamped(tick_unit);
                let text = scene.format_primary_axis_label(value, step_value);

                frame.fill_text(canvas::Text {
                    content: text,
                    position: Point::new(scene.layout.regions.y_axis.width - 4.0, y),
                    color: palette.background.base.text,
                    size: TEXT_SIZE.into(),
                    align_x: iced::Alignment::End.into(),
                    align_y: iced::Alignment::Center.into(),
                    font: style::AZERET_MONO,
                    ..Default::default()
                });
            }
        } else {
            let total_ticks = (scene.primary_plot().height / min_tick_px).floor() as usize;
            let (min_display, max_display) = scene.primary_domain_display_values();
            let (ticks, step) = super::ticks(min_display, max_display, total_ticks.max(2));
            let display_range = (max_display - min_display).abs().max(1e-6);

            for tick in ticks {
                if tick < min_display - f32::EPSILON || tick > max_display + f32::EPSILON {
                    continue;
                }

                let ratio = ((tick - min_display) / display_range).clamp(0.0, 1.0);
                let y = scene.primary_plot().y + (1.0 - ratio) * scene.primary_plot().height;
                let text = scene.format_primary_axis_label(tick, step);

                frame.fill_text(canvas::Text {
                    content: text,
                    position: Point::new(scene.layout.regions.y_axis.width - 4.0, y),
                    color: palette.background.base.text,
                    size: TEXT_SIZE.into(),
                    align_x: iced::Alignment::End.into(),
                    align_y: iced::Alignment::Center.into(),
                    font: style::AZERET_MONO,
                    ..Default::default()
                });
            }
        }

        for indicator in &scene.indicator_panels {
            let panel_index = indicator.panel_index;
            let Some(panel) = scene.layout.panel(panel_index) else {
                continue;
            };

            let (ticks, step_units) = super::unit_ticks(
                indicator.min_unit.0,
                indicator.max_unit.0,
                panel.plot.height,
                min_tick_px,
            );

            let panel_precision = self.panel_value_precision(panel_index);
            let zero_value = self.panel_unit_to_value(panel_precision, YUnit(0));
            let step_value = (self.panel_unit_to_value(panel_precision, YUnit(step_units))
                - zero_value)
                .abs()
                .max(1e-8);

            for tick in ticks {
                let tick_unit = YUnit(tick);
                let Some(y) = scene.map_indicator_plot_unit_clamped(panel_index, tick_unit) else {
                    continue;
                };

                let value = self.panel_unit_to_value(panel_precision, tick_unit);
                let text =
                    self.format_panel_axis_value(panel_index, panel_precision, value, step_value);

                frame.fill_text(canvas::Text {
                    content: text,
                    position: Point::new(scene.layout.regions.y_axis.width - 4.0, y),
                    color: palette.background.base.text,
                    size: TEXT_SIZE.into(),
                    align_x: iced::Alignment::End.into(),
                    align_y: iced::Alignment::Center.into(),
                    font: style::AZERET_MONO,
                    ..Default::default()
                });
            }
        }
    }

    fn fill_x_axis_labels(&self, frame: &mut canvas::Frame, scene: &Scene, palette: &Extended) {
        let horizontal_pixel_ratio = self.resolved_horizontal_pixel_ratio();
        let plot_width = scene.x_axis_plot_width();
        let (ticks, step_units) = super::unit_ticks(
            scene.min_x_unit,
            scene.max_x_unit,
            plot_width,
            MIN_X_TICK_PX.max(40.0),
        );

        let mut ticks_with_bounds = Vec::with_capacity(ticks.len().saturating_add(2));

        if let Some(first) = ticks.first().copied() {
            ticks_with_bounds.push(first.saturating_sub(step_units));
        }

        ticks_with_bounds.extend(ticks);

        if let Some(last) = ticks_with_bounds.last().copied() {
            ticks_with_bounds.push(last.saturating_add(step_units));
        }

        let clip_region = Rectangle {
            x: 0.0,
            y: 0.0,
            width: plot_width,
            height: scene.layout.regions.x_axis.height,
        };

        frame.with_clip(clip_region, |frame| {
            for tick in ticks_with_bounds {
                let x = Self::snap_plot_x_to_cell(scene.map_x_plot(tick), horizontal_pixel_ratio);
                let label = self.format_x_label(scene.x_axis, tick, step_units);

                frame.fill_text(canvas::Text {
                    content: label,
                    position: Point::new(x, scene.layout.regions.x_axis.height / 2.0),
                    color: palette.background.base.text,
                    size: TEXT_SIZE.into(),
                    align_x: iced::Alignment::Center.into(),
                    align_y: iced::Alignment::Center.into(),
                    font: style::AZERET_MONO,
                    ..Default::default()
                });
            }
        });
    }

    fn fill_overlay(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        overlay_origin_in_window: Point,
    ) {
        let horizontal_pixel_ratio = self.resolved_horizontal_pixel_ratio();
        self.fill_corner_controls(frame, scene, palette);
        self.fill_panel_controls(frame, scene, palette);
        self.fill_primary_ticker_legend(frame, scene, palette);

        if !scene.hovering_ticker_legend {
            let show_primary_panel_values = scene.ticker_legend.is_none();
            self.fill_panel_header_values(frame, scene, palette, show_primary_panel_values);
        }

        if self.drawings.has_state() {
            self.fill_drawings(
                frame,
                scene,
                palette,
                overlay_origin_in_window,
                horizontal_pixel_ratio,
            );
        }

        if scene.hovered_control.is_some()
            || scene.hovered_corner_control.is_some()
            || scene.hovering_ticker_legend
        {
            return;
        }

        let Some(cursor) = scene.cursor else {
            return;
        };

        let line_color = palette.background.base.text.scale_alpha(0.35);
        let (crosshair_width, crosshair_width_phys) =
            Self::quantized_stroke_width(1.0, horizontal_pixel_ratio);

        let plot_origin_x_global = overlay_origin_in_window.x + scene.layout.regions.plot.x;
        let gx = scene.layout.regions.plot.x
            + Self::snap_plot_x_for_stroke_with_origin(
                cursor.x_plot,
                horizontal_pixel_ratio,
                plot_origin_x_global,
                crosshair_width_phys,
            );
        let panel_plot = scene
            .layout
            .panel(cursor.panel_index)
            .map(|panel| panel.plot)
            .unwrap_or(*scene.primary_plot());
        let panel_bounds = (
            scene.layout.regions.plot.y + panel_plot.y,
            scene.layout.regions.plot.y + panel_plot.y + panel_plot.height,
        );
        let gy = Self::snap_stroke_center_with_origin(
            scene.layout.regions.plot.y + cursor.y_plot,
            horizontal_pixel_ratio,
            overlay_origin_in_window.y,
            crosshair_width_phys,
        )
        .clamp(panel_bounds.0, panel_bounds.1);

        let stroke = canvas::Stroke::default()
            .with_color(line_color)
            .with_width(crosshair_width);

        frame.stroke(
            &canvas::Path::line(
                Point::new(gx, scene.layout.regions.plot.y),
                Point::new(
                    gx,
                    scene.layout.regions.plot.y + scene.layout.regions.plot.height,
                ),
            ),
            stroke,
        );

        frame.stroke(
            &canvas::Path::line(
                Point::new(scene.layout.regions.plot.x, gy),
                Point::new(
                    scene.layout.regions.plot.x + scene.layout.regions.plot.width,
                    gy,
                ),
            ),
            stroke,
        );
    }

    fn fill_overlay_interaction_text(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
    ) {
        if scene.hovered_control.is_some()
            || scene.hovered_corner_control.is_some()
            || scene.hovering_ticker_legend
        {
            return;
        }

        self.fill_active_drawing_axis_labels(frame, scene, palette);

        let Some(cursor) = scene.cursor else {
            return;
        };

        let gy = scene.layout.regions.plot.y + cursor.y_plot;

        if let Some(y_text) = cursor
            .y_primary_unit
            .map(|primary_unit| {
                let panel_precision = self.panel_value_precision(cursor.panel_index);
                let primary_value = self.panel_unit_to_value(panel_precision, primary_unit);
                scene.format_primary_cursor_label(primary_value)
            })
            .or_else(|| {
                cursor.y_indicator_unit.map(|indicator_unit| {
                    let panel_precision = self.panel_value_precision(cursor.panel_index);
                    let indicator_value = self.panel_unit_to_value(panel_precision, indicator_unit);
                    self.format_panel_axis_value(
                        cursor.panel_index,
                        panel_precision,
                        indicator_value,
                        0.01,
                    )
                })
            })
        {
            let y_badge_bounds = self
                .panel_plot_bounds_in_overlay(scene, cursor.panel_index)
                .unwrap_or_else(|| self.plot_bounds_in_overlay(scene));
            self.draw_y_axis_badge(frame, scene, palette, gy, &y_text, y_badge_bounds);
        }

        let x_text = self.format_x_label(scene.x_axis, cursor.x_unit, 1);
        let horizontal_pixel_ratio = self.resolved_horizontal_pixel_ratio();
        self.draw_x_axis_badge(
            frame,
            scene,
            palette,
            scene.layout.regions.plot.x
                + Self::snap_plot_x_to_cell(cursor.x_plot, horizontal_pixel_ratio),
            &x_text,
        );
    }
}

impl<'a, S, M> Widget<M, Theme, Renderer> for KlineWidget<'a, S>
where
    S: KlineSeriesLike,
    M: Clone + 'static + From<KlineWidgetEvent>,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fill,
            height: Length::Fill,
        }
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &iced_layout::Limits,
    ) -> iced_layout::Node {
        let panel_count = self.panel_count();

        let build_panel_stack = |stack_size: Size| {
            let plot_heights = self.panel_plot_heights(stack_size.height, panel_count);
            let mut children = Vec::with_capacity(panel_count.saturating_mul(3).saturating_sub(1));

            let mut y = 0.0;

            for panel_index in 0..panel_count {
                let plot_h = plot_heights.get(panel_index).copied().unwrap_or_default();

                children.push(
                    iced_layout::Node::new(Size::new(stack_size.width, plot_h))
                        .move_to(Point::new(0.0, y)),
                );
                y += plot_h;

                let axis_h = if panel_index + 1 == panel_count {
                    (stack_size.height - y).max(0.0)
                } else {
                    0.0
                };

                children.push(
                    iced_layout::Node::new(Size::new(stack_size.width, axis_h))
                        .move_to(Point::new(0.0, y)),
                );
                y += axis_h;

                if panel_index + 1 < panel_count {
                    children.push(
                        iced_layout::Node::new(Size::new(stack_size.width, PANEL_SPLITTER_HEIGHT))
                            .move_to(Point::new(0.0, y)),
                    );
                    y += PANEL_SPLITTER_HEIGHT;
                }
            }

            iced_layout::Node::with_children(stack_size, children)
        };

        let row_node = iced_layout::next_to_each_other(
            &limits.shrink(Size::new(0.0, X_AXIS_HEIGHT)),
            0.0,
            |l| {
                let stack_node = iced_layout::atomic(
                    &l.shrink(Size::new(Y_AXIS_GUTTER, 0.0)),
                    Length::Fill,
                    Length::Fill,
                );

                build_panel_stack(stack_node.size())
            },
            |l| iced_layout::atomic(l, Y_AXIS_GUTTER, Length::Fill),
        );

        let x_axis_node = iced_layout::next_to_each_other(
            limits,
            0.0,
            |l| {
                iced_layout::atomic(
                    &l.shrink(Size::new(Y_AXIS_GUTTER, 0.0)),
                    Length::Fill,
                    X_AXIS_HEIGHT,
                )
            },
            |l| iced_layout::atomic(l, Y_AXIS_GUTTER, X_AXIS_HEIGHT),
        );

        let row_h = row_node.size().height;
        let total_w = row_node.size().width;
        let total_h = row_h + X_AXIS_HEIGHT;

        iced_layout::Node::with_children(
            Size::new(total_w, total_h),
            vec![
                row_node.move_to(Point::new(0.0, 0.0)),
                x_axis_node.move_to(Point::new(0.0, row_h)),
            ],
        )
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        _viewport: &Rectangle,
    ) {
        if shell.is_event_captured() {
            return;
        }

        match event {
            Event::Mouse(mouse_event) => {
                let state = tree.state.downcast_mut::<State>();
                let bounds = layout.bounds();
                let Some(layout_tree) =
                    PanelLayoutTree::from_layout(layout, &self.composition.panels)
                else {
                    return;
                };

                let cursor_pos = if let Some(local) = cursor.position_in(bounds) {
                    local
                } else if matches!(state.drag_mode, DragMode::DrawingMove { .. }) {
                    if let Some(global) = cursor.position() {
                        Point::new(global.x - bounds.x, global.y - bounds.y)
                    } else {
                        if let DragMode::DrawingMove { id, .. } = state.drag_mode {
                            shell.publish(M::from(KlineWidgetEvent::Drawing(
                                KlineWidgetDrawingEvent::DragFinished { id },
                            )));
                        }

                        state.drag_mode = DragMode::None;
                        state.last_cursor = None;
                        state.clear_overlay_caches();
                        return;
                    }
                } else {
                    if !matches!(state.drag_mode, DragMode::None) {
                        state.drag_mode = DragMode::None;
                        state.last_cursor = None;
                    }
                    state.clear_overlay_caches();
                    return;
                };

                let zone = layout_tree.hit_test(cursor_pos);
                let primary_panel = layout_tree
                    .panels
                    .iter()
                    .position(|panel| panel.kind == KlinePanelKind::PrimaryChart)
                    .unwrap_or(0);
                let panel_controls = self.build_panel_control_hits(&layout_tree, primary_panel);
                let primary_scale_mode = self.resolved_panel_scale_mode(primary_panel);
                let corner_controls =
                    self.build_corner_control_hits(&layout_tree, primary_scale_mode);
                let show_legend_values = matches!(zone, LayoutHitZone::PanelPlot(_));
                let mut ticker_legend = self.build_ticker_legend_layout(
                    &layout_tree,
                    primary_panel,
                    show_legend_values,
                    false,
                );
                let mut ticker_legend_hit = ticker_legend
                    .as_ref()
                    .and_then(|legend| Self::hit_ticker_legend(&layout_tree, legend, cursor_pos));

                if ticker_legend_hit.is_some() {
                    ticker_legend =
                        self.build_ticker_legend_layout(&layout_tree, primary_panel, false, true);
                    ticker_legend_hit = ticker_legend.as_ref().and_then(|legend| {
                        Self::hit_ticker_legend(&layout_tree, legend, cursor_pos)
                    });
                }

                match mouse_event {
                    mouse::Event::WheelScrolled { delta } => {
                        let scroll_y = match delta {
                            mouse::ScrollDelta::Lines { y, .. }
                            | mouse::ScrollDelta::Pixels { y, .. } => *y,
                        };

                        if scroll_y.abs() <= f32::EPSILON {
                            return;
                        }

                        match zone {
                            LayoutHitZone::PanelPlot(_) => {
                                if Self::hit_panel_control(
                                    &layout_tree,
                                    &panel_controls,
                                    cursor_pos,
                                )
                                .is_some()
                                {
                                    return;
                                }

                                if Self::hit_corner_control(&corner_controls, cursor_pos).is_some()
                                {
                                    return;
                                }

                                if ticker_legend_hit.is_some() {
                                    return;
                                }

                                let zoom_in = scroll_y > 0.0;
                                let new_scale = self
                                    .step_horizontal_scale_percent(self.horizontal_scale, zoom_in);

                                if (new_scale.as_pixels_per_bar()
                                    - self.horizontal_scale.as_pixels_per_bar())
                                .abs()
                                    > f32::EPSILON
                                {
                                    shell.publish(M::from(
                                        KlineWidgetEvent::HorizontalScaleChanged(
                                            self.normalize_horizontal_scale(new_scale),
                                        ),
                                    ));
                                    state.clear_all_caches();
                                    shell.capture_event();
                                }
                            }
                            LayoutHitZone::YAxis(panel_index) => {
                                let Some(scene) = self.compute_scene(layout, cursor) else {
                                    return;
                                };

                                let Some(panel_id) = self.panel_id(panel_index) else {
                                    return;
                                };

                                let Some(viewport) = self.panel_viewport_after_y_zoom(
                                    &scene,
                                    panel_index,
                                    cursor_pos.y,
                                    scroll_y,
                                ) else {
                                    return;
                                };

                                shell.publish(M::from(KlineWidgetEvent::PanelYViewportChanged {
                                    panel_id,
                                    viewport,
                                }));
                                state.clear_all_caches();
                                shell.capture_event();
                            }
                            _ => {}
                        }
                    }
                    mouse::Event::ButtonPressed(mouse::Button::Left) => {
                        if let Some(global_pos) = cursor.position() {
                            let new_click = iced_core::mouse::Click::new(
                                global_pos,
                                mouse::Button::Left,
                                state.previous_click,
                            );

                            if let LayoutHitZone::YAxis(panel_index) = zone
                                && new_click.kind() == iced_core::mouse::click::Kind::Double
                                && let Some(panel_id) = self.panel_id(panel_index)
                            {
                                shell.publish(M::from(KlineWidgetEvent::PanelYViewportReset {
                                    panel_id,
                                }));
                                state.clear_all_caches();
                                state.previous_click = Some(new_click);
                                return;
                            }

                            if matches!(
                                zone,
                                LayoutHitZone::BottomXAxis | LayoutHitZone::PanelXAxis(_)
                            ) && new_click.kind() == iced_core::mouse::click::Kind::Double
                            {
                                shell.publish(M::from(KlineWidgetEvent::XAxisDoubleClick));
                                state.clear_all_caches();
                                state.previous_click = Some(new_click);
                                return;
                            }

                            state.previous_click = Some(new_click);
                        } else {
                            state.previous_click = None;
                        }

                        if let (Some(legend), Some(TickerLegendHit::Icon(row_index, icon_kind))) =
                            (ticker_legend.as_ref(), ticker_legend_hit)
                            && let Some(row) = legend.rows.get(row_index)
                        {
                            shell.publish(M::from(icon_kind.into_event(row.ticker)));
                            state.drag_mode = DragMode::None;
                            state.last_cursor = None;
                            state.clear_all_caches();
                            shell.capture_event();
                            return;
                        }

                        if let Some(control) =
                            Self::hit_corner_control(&corner_controls, cursor_pos)
                        {
                            shell.publish(M::from(control.kind.into_event()));
                            state.drag_mode = DragMode::None;
                            state.last_cursor = None;
                            state.clear_all_caches();
                            shell.capture_event();
                            return;
                        }

                        if ticker_legend_hit.is_some() {
                            state.drag_mode = DragMode::None;
                            state.last_cursor = None;
                            return;
                        }

                        if matches!(zone, LayoutHitZone::PanelPlot(_))
                            && let Some(control) =
                                Self::hit_panel_control(&layout_tree, &panel_controls, cursor_pos)
                        {
                            shell.publish(M::from(control.kind.into_event(control.panel_index)));
                            state.drag_mode = DragMode::None;
                            state.last_cursor = None;
                            state.clear_all_caches();
                            shell.capture_event();
                            return;
                        }

                        if self.drawings.allows_panning()
                            && matches!(zone, LayoutHitZone::PanelPlot(_))
                            && let Some(scene) = self.compute_scene(layout, cursor)
                        {
                            if Self::hit_corner_control(&corner_controls, cursor_pos).is_some() {
                                state.drag_mode = DragMode::None;
                                state.last_cursor = None;
                                return;
                            }

                            if let Some((id, handle_kind)) =
                                self.hit_test_selected_drawing_handle(&scene, cursor_pos)
                                && let Some(anchor) = self.drawing_anchor_from_scene_cursor(&scene)
                            {
                                let target = DrawingDragTarget::Handle(handle_kind);

                                shell.publish(M::from(KlineWidgetEvent::Drawing(
                                    KlineWidgetDrawingEvent::DragStarted { id, target, anchor },
                                )));
                                state.drag_mode = DragMode::DrawingMove {
                                    id,
                                    target,
                                    panel_id: anchor.panel_id,
                                };
                                state.last_cursor = Some(cursor_pos);
                                state.clear_overlay_caches();
                                state.clear_all_caches();
                                shell.capture_event();
                                return;
                            }

                            let hit_drawing = self.hit_test_drawings(&scene, cursor_pos);

                            if let Some(id) = hit_drawing {
                                let was_selected = self.drawings.selected_drawing == Some(id);

                                if !was_selected {
                                    shell.publish(M::from(KlineWidgetEvent::Drawing(
                                        KlineWidgetDrawingEvent::Selected(Some(id)),
                                    )));
                                    state.drag_mode = DragMode::None;
                                    state.last_cursor = None;
                                    state.clear_overlay_caches();
                                    state.clear_all_caches();
                                    shell.capture_event();
                                    return;
                                }

                                if let Some(anchor) = self.drawing_anchor_from_scene_cursor(&scene)
                                {
                                    let target = DrawingDragTarget::Translate;

                                    shell.publish(M::from(KlineWidgetEvent::Drawing(
                                        KlineWidgetDrawingEvent::DragStarted { id, target, anchor },
                                    )));
                                    state.drag_mode = DragMode::DrawingMove {
                                        id,
                                        target,
                                        panel_id: anchor.panel_id,
                                    };
                                    state.last_cursor = Some(cursor_pos);
                                    state.clear_overlay_caches();
                                    state.clear_all_caches();
                                    shell.capture_event();
                                    return;
                                }

                                state.drag_mode = DragMode::None;
                                state.last_cursor = Some(cursor_pos);
                                state.clear_overlay_caches();
                                state.clear_all_caches();
                                shell.capture_event();
                                return;
                            }

                            if self.drawings.selected_drawing.is_some() {
                                shell.publish(M::from(KlineWidgetEvent::Drawing(
                                    KlineWidgetDrawingEvent::Selected(None),
                                )));
                                state.drag_mode = DragMode::None;
                                state.last_cursor = None;
                                state.clear_overlay_caches();
                                state.clear_all_caches();
                                shell.capture_event();
                                return;
                            }
                        }

                        if let LayoutHitZone::Splitter(split_index) = zone {
                            state.drag_mode = DragMode::Split(split_index);
                            state.last_cursor = Some(cursor_pos);
                            shell.capture_event();
                        } else if let LayoutHitZone::YAxis(panel_index) = zone {
                            state.drag_mode = DragMode::AxisScale {
                                panel_index,
                                anchor_y: cursor_pos.y,
                            };
                            state.last_cursor = Some(cursor_pos);
                            shell.capture_event();
                        } else if !self.drawings.allows_panning() {
                            let anchor = if let Some(panel_id) = self.drawings.draft_panel_id() {
                                self.compute_scene(layout, cursor).and_then(|scene| {
                                    self.drawing_anchor_from_scene_point(
                                        &scene, panel_id, cursor_pos,
                                    )
                                })
                            } else if let LayoutHitZone::PanelPlot(panel_index) = zone {
                                let panel_id = self.panel_id(panel_index);

                                panel_id.and_then(|panel_id| {
                                    self.compute_scene(layout, cursor).and_then(|scene| {
                                        self.drawing_anchor_from_scene_point(
                                            &scene, panel_id, cursor_pos,
                                        )
                                    })
                                })
                            } else {
                                None
                            };

                            if let Some(anchor) = anchor {
                                shell.publish(M::from(KlineWidgetEvent::Drawing(
                                    KlineWidgetDrawingEvent::AnchorPressed(anchor),
                                )));
                                state.drag_mode = DragMode::None;
                                state.last_cursor = Some(cursor_pos);
                                state.clear_overlay_caches();
                                state.clear_all_caches();
                                shell.capture_event();
                            } else {
                                state.drag_mode = DragMode::None;
                                state.last_cursor = None;
                            }
                        } else if self.drawings.allows_panning()
                            && let LayoutHitZone::PanelPlot(panel_index) = zone
                        {
                            state.drag_mode = DragMode::Pan { panel_index };
                            state.last_cursor = Some(cursor_pos);
                        } else {
                            state.drag_mode = DragMode::None;
                            state.last_cursor = None;
                        }
                    }
                    mouse::Event::ButtonPressed(mouse::Button::Right)
                        if self.drawings.drawing_draft.is_some() =>
                    {
                        shell.publish(M::from(KlineWidgetEvent::Drawing(
                            KlineWidgetDrawingEvent::DraftCanceled,
                        )));
                        state.drag_mode = DragMode::None;
                        state.last_cursor = None;
                        state.clear_overlay_caches();
                        state.clear_all_caches();
                        shell.capture_event();
                    }
                    mouse::Event::ButtonReleased(mouse::Button::Left) => {
                        if let DragMode::DrawingMove { id, .. } = state.drag_mode {
                            shell.publish(M::from(KlineWidgetEvent::Drawing(
                                KlineWidgetDrawingEvent::DragFinished { id },
                            )));
                            state.clear_overlay_caches();
                            state.clear_all_caches();
                            shell.capture_event();
                        }

                        state.drag_mode = DragMode::None;
                        state.last_cursor = None;
                    }
                    mouse::Event::CursorMoved { .. } => {
                        state.clear_overlay_caches();

                        if let DragMode::Split(split_index) = state.drag_mode {
                            if let Some(split) = self.split_ratio_from_cursor(
                                cursor_pos.y,
                                &layout_tree,
                                split_index,
                            ) {
                                shell.publish(M::from(KlineWidgetEvent::PanelSplitChanged {
                                    index: split_index,
                                    split,
                                }));
                                state.last_cursor = Some(cursor_pos);
                                state.clear_all_caches();
                                shell.capture_event();
                            }
                        } else if let DragMode::AxisScale {
                            panel_index,
                            anchor_y,
                        } = state.drag_mode
                        {
                            let prev = state.last_cursor.unwrap_or(cursor_pos);
                            let dy_px = cursor_pos.y - prev.y;
                            let scale_delta = (-dy_px) * Y_AXIS_DRAG_SCALE_DELTA_PER_PX;

                            if scale_delta.abs() > f32::EPSILON
                                && let Some(scene) = self.compute_scene(layout, cursor)
                                && let Some(panel_id) = self.panel_id(panel_index)
                                && let Some(viewport) = self.panel_viewport_after_y_zoom(
                                    &scene,
                                    panel_index,
                                    anchor_y,
                                    scale_delta,
                                )
                            {
                                shell.publish(M::from(KlineWidgetEvent::PanelYViewportChanged {
                                    panel_id,
                                    viewport,
                                }));
                                state.clear_all_caches();
                                shell.capture_event();
                            }

                            state.last_cursor = Some(cursor_pos);
                        } else if let DragMode::DrawingMove {
                            id,
                            target,
                            panel_id,
                        } = state.drag_mode
                        {
                            if let Some(scene) = self.compute_scene(layout, cursor)
                                && let Some(new_anchor) = self
                                    .drawing_anchor_from_scene_point(&scene, panel_id, cursor_pos)
                            {
                                shell.publish(M::from(KlineWidgetEvent::Drawing(
                                    KlineWidgetDrawingEvent::DragMoved {
                                        id,
                                        target,
                                        anchor: new_anchor,
                                    },
                                )));
                                state.last_cursor = Some(cursor_pos);
                                state.clear_overlay_caches();
                                state.clear_all_caches();
                                shell.capture_event();
                            }
                        } else if let Some(draft_panel_id) = self.drawings.draft_panel_id()
                            && Self::hit_panel_control(&layout_tree, &panel_controls, cursor_pos)
                                .is_none()
                            && Self::hit_corner_control(&corner_controls, cursor_pos).is_none()
                            && ticker_legend_hit.is_none()
                            && let Some(scene) = self.compute_scene(layout, cursor)
                            && let Some(anchor) = self.drawing_anchor_from_scene_point(
                                &scene,
                                draft_panel_id,
                                cursor_pos,
                            )
                        {
                            shell.publish(M::from(KlineWidgetEvent::Drawing(
                                KlineWidgetDrawingEvent::AnchorMoved(anchor),
                            )));
                            state.last_cursor = Some(cursor_pos);
                            state.clear_overlay_caches();
                            shell.capture_event();
                        } else if let DragMode::Pan { panel_index } = state.drag_mode {
                            if Self::hit_corner_control(&corner_controls, cursor_pos).is_some() {
                                state.last_cursor = Some(cursor_pos);
                                return;
                            }

                            let prev = state.last_cursor.unwrap_or(cursor_pos);
                            let dx_px = cursor_pos.x - prev.x;
                            let dy_px = cursor_pos.y - prev.y;

                            if dx_px.abs() > f32::EPSILON {
                                let spacing = BarSpacingPx::from_logical(
                                    self.normalize_horizontal_scale(self.horizontal_scale)
                                        .as_pixels_per_bar(),
                                )
                                .as_f32();
                                let dx_units = -(dx_px) / spacing;

                                shell.publish(M::from(KlineWidgetEvent::HorizontalOffsetChanged(
                                    self.horizontal_offset + dx_units,
                                )));
                                state.clear_all_caches();
                            }

                            if dy_px.abs() > f32::EPSILON
                                && let Some(scene) = self.compute_scene(layout, cursor)
                                && let Some(panel_id) = self.panel_id(panel_index)
                                && let Some(viewport) = self.panel_viewport_after_primary_y_pan(
                                    &scene,
                                    panel_index,
                                    dy_px,
                                )
                            {
                                shell.publish(M::from(KlineWidgetEvent::PanelYViewportChanged {
                                    panel_id,
                                    viewport,
                                }));
                                state.clear_all_caches();
                            }

                            state.last_cursor = Some(cursor_pos);
                        }
                    }
                    _ => {}
                }
            }
            Event::Window(window::Event::RedrawRequested(_)) => {
                let state = tree.state.downcast_mut::<State>();
                if state.last_cache_rev != self.version {
                    state.clear_all_caches();
                    state.last_cache_rev = self.version;
                }
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        use advanced::Renderer as _;

        let state = tree.state.downcast_ref::<State>();
        let Some(scene) = self.compute_scene(layout, cursor) else {
            return;
        };

        let bounds = layout.bounds();
        let palette = theme.extended_palette();

        renderer.with_translation(Vector::new(bounds.x, bounds.y), |r| {
            let plot_rect = scene.plot_rect();
            let plot_origin_in_window = Point::new(bounds.x + plot_rect.x, bounds.y + plot_rect.y);
            let overlay_origin_in_window = Point::new(bounds.x, bounds.y);

            let plot_geom = state.plot_cache.draw(r, plot_rect.size(), |frame| {
                self.fill_main_geometry(frame, &scene, palette, plot_origin_in_window);
            });

            let splitter_color = palette.background.strong.color.scale_alpha(0.25);

            for splitter in &scene.layout.splitters {
                r.fill_quad(
                    Quad {
                        bounds: Rectangle {
                            x: plot_rect.x + splitter.x,
                            y: plot_rect.y + splitter.y,
                            width: splitter.width,
                            height: splitter.height,
                        },
                        snap: true,
                        ..Default::default()
                    },
                    splitter_color,
                );
            }

            r.fill_quad(
                Quad {
                    bounds: Rectangle {
                        x: plot_rect.x,
                        y: plot_rect.y + plot_rect.height,
                        width: plot_rect.width + scene.layout.regions.y_axis.width,
                        height: 1.0,
                    },
                    snap: true,
                    ..Default::default()
                },
                splitter_color,
            );

            r.fill_quad(
                Quad {
                    bounds: Rectangle {
                        x: plot_rect.x + plot_rect.width,
                        y: plot_rect.y,
                        width: 1.0,
                        height: plot_rect.height,
                    },
                    snap: true,
                    ..Default::default()
                },
                splitter_color,
            );

            let y_rect = scene.layout.regions.y_axis;
            let y_geom = state.y_axis_cache.draw(r, y_rect.size(), |frame| {
                self.fill_y_axis_labels(frame, &scene, palette);
            });

            let x_rect = scene.layout.regions.x_axis;
            let x_geom = state.x_axis_cache.draw(r, x_rect.size(), |frame| {
                self.fill_x_axis_labels(frame, &scene, palette);
            });

            let overlay_geom = state.overlay_cache.draw(r, bounds.size(), |frame| {
                self.fill_overlay(frame, &scene, palette, overlay_origin_in_window);
            });

            let interaction_text_geom =
                state
                    .interaction_text_cache
                    .draw(r, bounds.size(), |frame| {
                        self.fill_overlay_interaction_text(frame, &scene, palette);
                    });

            r.with_translation(Vector::new(plot_rect.x, plot_rect.y), |r| {
                use iced::advanced::graphics::geometry::Renderer as _;
                r.draw_geometry(plot_geom);
            });

            r.with_translation(Vector::new(y_rect.x, y_rect.y), |r| {
                use iced::advanced::graphics::geometry::Renderer as _;
                r.draw_geometry(y_geom);
            });

            r.with_translation(Vector::new(x_rect.x, x_rect.y), |r| {
                use iced::advanced::graphics::geometry::Renderer as _;
                r.draw_geometry(x_geom);
            });

            let layer_bounds = Rectangle {
                x: 0.0,
                y: 0.0,
                width: bounds.width,
                height: bounds.height,
            };

            r.with_layer(layer_bounds, |r| {
                use iced::advanced::graphics::geometry::Renderer as _;
                r.draw_geometry(overlay_geom);
            });

            r.with_layer(layer_bounds, |r| {
                use iced::advanced::graphics::geometry::Renderer as _;
                r.draw_geometry(interaction_text_geom);
            });
        });
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: advanced::mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> advanced::mouse::Interaction {
        let Some(cursor_local) = cursor.position_in(layout.bounds()) else {
            return advanced::mouse::Interaction::default();
        };

        let Some(layout_tree) = PanelLayoutTree::from_layout(layout, &self.composition.panels)
        else {
            return advanced::mouse::Interaction::default();
        };
        let state = tree.state.downcast_ref::<State>();

        if matches!(
            state.drag_mode,
            DragMode::Split(_) | DragMode::AxisScale { .. }
        ) {
            return advanced::mouse::Interaction::ResizingVertically;
        }

        if matches!(state.drag_mode, DragMode::Pan { .. }) {
            return advanced::mouse::Interaction::Grabbing;
        }

        if matches!(state.drag_mode, DragMode::DrawingMove { .. }) {
            return advanced::mouse::Interaction::Grabbing;
        }

        let primary_panel = layout_tree
            .panels
            .iter()
            .position(|panel| panel.kind == KlinePanelKind::PrimaryChart)
            .unwrap_or(0);
        let zone = layout_tree.hit_test(cursor_local);
        let panel_controls = self.build_panel_control_hits(&layout_tree, primary_panel);
        let primary_scale_mode = self.resolved_panel_scale_mode(primary_panel);
        let corner_controls = self.build_corner_control_hits(&layout_tree, primary_scale_mode);
        let show_legend_values = matches!(zone, LayoutHitZone::PanelPlot(_));
        let mut ticker_legend =
            self.build_ticker_legend_layout(&layout_tree, primary_panel, show_legend_values, false);
        let mut ticker_legend_hit = ticker_legend
            .as_ref()
            .and_then(|legend| Self::hit_ticker_legend(&layout_tree, legend, cursor_local));

        if ticker_legend_hit.is_some() {
            ticker_legend =
                self.build_ticker_legend_layout(&layout_tree, primary_panel, false, true);
            ticker_legend_hit = ticker_legend
                .as_ref()
                .and_then(|legend| Self::hit_ticker_legend(&layout_tree, legend, cursor_local));
        }

        if ticker_legend_hit.is_some() {
            return advanced::mouse::Interaction::Pointer;
        }

        if Self::hit_panel_control(&layout_tree, &panel_controls, cursor_local).is_some() {
            return advanced::mouse::Interaction::Pointer;
        }

        if Self::hit_corner_control(&corner_controls, cursor_local).is_some() {
            return advanced::mouse::Interaction::Pointer;
        }

        if self.drawings.allows_panning()
            && matches!(zone, LayoutHitZone::PanelPlot(_))
            && let Some(scene) = self.compute_scene(layout, cursor)
            && self
                .hit_test_selected_drawing_handle(&scene, cursor_local)
                .is_some()
        {
            return advanced::mouse::Interaction::Grab;
        }

        if self.drawings.allows_panning()
            && matches!(zone, LayoutHitZone::PanelPlot(_))
            && let Some(scene) = self.compute_scene(layout, cursor)
            && let Some(hit_id) = self.hit_test_drawings(&scene, cursor_local)
        {
            if self.drawings.selected_drawing == Some(hit_id) {
                return advanced::mouse::Interaction::Grab;
            }

            return advanced::mouse::Interaction::Pointer;
        }

        match zone {
            LayoutHitZone::Splitter(_) => advanced::mouse::Interaction::ResizingVertically,
            LayoutHitZone::PanelPlot(_) => advanced::mouse::Interaction::Crosshair,
            LayoutHitZone::YAxis(_) => advanced::mouse::Interaction::ResizingVertically,
            LayoutHitZone::PanelXAxis(_) | LayoutHitZone::BottomXAxis => {
                advanced::mouse::Interaction::Pointer
            }
            LayoutHitZone::Outside => advanced::mouse::Interaction::default(),
        }
    }
}

impl<'a, S, M> From<KlineWidget<'a, S>> for Element<'a, M, Theme, Renderer>
where
    S: KlineSeriesLike,
    M: Clone + 'a + 'static + From<KlineWidgetEvent>,
{
    fn from(chart: KlineWidget<'a, S>) -> Self {
        Self::new(chart)
    }
}
