mod chrome;
pub mod composition;
mod layout;
mod scene;

use crate::style;
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

pub const DEFAULT_BAR_SPACING_PX: f32 = 8.0;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DrawingTool {
    #[default]
    Cursor,
    Trendline,
    Box,
    HorizontalLine,
    VerticalLine,
}

impl DrawingTool {
    pub fn allows_panning(self) -> bool {
        matches!(self, Self::Cursor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DrawingId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct YUnit(pub i64);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawingAnchor {
    pub panel_id: PanelId,
    pub time: UnixMs,
    pub y_unit: YUnit,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawingStyle {
    pub stroke_color: iced::Color,
    pub stroke_width: f32,
    pub fill_color: Option<iced::Color>,
}

impl Default for DrawingStyle {
    fn default() -> Self {
        Self {
            stroke_color: iced::Color::from_rgb(0.82, 0.84, 0.90),
            stroke_width: 1.2,
            fill_color: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DrawingObject {
    Trendline {
        start: DrawingAnchor,
        end: DrawingAnchor,
    },
    Box {
        start: DrawingAnchor,
        end: DrawingAnchor,
    },
    HorizontalLine {
        panel_id: PanelId,
        y_unit: YUnit,
    },
    VerticalLine {
        time: UnixMs,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct DrawingEntity {
    pub id: DrawingId,
    pub object: DrawingObject,
    pub style: DrawingStyle,
    pub locked: bool,
    pub visible: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DrawingDraft {
    Trendline {
        start: DrawingAnchor,
        current: DrawingAnchor,
        style: DrawingStyle,
    },
    Box {
        start: DrawingAnchor,
        current: DrawingAnchor,
        style: DrawingStyle,
    },
}

impl DrawingDraft {
    pub fn tool(&self) -> DrawingTool {
        match self {
            Self::Trendline { .. } => DrawingTool::Trendline,
            Self::Box { .. } => DrawingTool::Box,
        }
    }

    pub fn style(&self) -> DrawingStyle {
        match self {
            Self::Trendline { style, .. } | Self::Box { style, .. } => *style,
        }
    }

    pub fn preview_object(&self) -> DrawingObject {
        match self {
            Self::Trendline { start, current, .. } => DrawingObject::Trendline {
                start: *start,
                end: *current,
            },
            Self::Box { start, current, .. } => DrawingObject::Box {
                start: *start,
                end: *current,
            },
        }
    }
}

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
    DrawingSelected(Option<DrawingId>),
    DrawingAnchorPressed(DrawingAnchor),
    DrawingAnchorMoved(DrawingAnchor),
    DrawingDragStarted {
        id: DrawingId,
        anchor: DrawingAnchor,
    },
    DrawingDragMoved {
        id: DrawingId,
        anchor: DrawingAnchor,
    },
    DrawingDragFinished {
        id: DrawingId,
    },
    DrawingDraftCanceled,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DragMode {
    None,
    Split(usize),
    AxisScale { panel_index: usize, anchor_y: f32 },
    Pan { panel_index: usize },
    DrawingMove { id: DrawingId },
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
    drawings: &'a [DrawingEntity],
    selected_drawing: Option<DrawingId>,
    drawing_draft: Option<&'a DrawingDraft>,
    basis: Basis,
    horizontal_scale: HorizontalScale,
    horizontal_offset: f32,
    active_drawing_tool: DrawingTool,
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
            drawings: &[],
            selected_drawing: None,
            drawing_draft: None,
            basis: Basis::Time(timeframe),
            horizontal_scale: HorizontalScale::pixels_per_bar(DEFAULT_BAR_SPACING_PX),
            horizontal_offset: 0.0,
            active_drawing_tool: DrawingTool::Cursor,
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

    pub fn with_drawings(mut self, drawings: &'a [DrawingEntity]) -> Self {
        self.drawings = drawings;
        self
    }

    pub fn with_selected_drawing(mut self, selected: Option<DrawingId>) -> Self {
        self.selected_drawing = selected;
        self
    }

    pub fn with_drawing_draft(mut self, draft: Option<&'a DrawingDraft>) -> Self {
        self.drawing_draft = draft;
        self
    }

    pub fn with_active_drawing_tool(mut self, tool: DrawingTool) -> Self {
        self.active_drawing_tool = tool;
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

    fn drawing_tool_allows_panning(&self) -> bool {
        self.active_drawing_tool.allows_panning()
    }

    fn has_drawing_state(&self) -> bool {
        !self.drawings.is_empty() || self.selected_drawing.is_some() || self.drawing_draft.is_some()
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

    fn drawing_anchor_from_layout_cursor(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) -> Option<DrawingAnchor> {
        let scene = self.compute_scene(layout, cursor)?;
        self.drawing_anchor_from_scene_cursor(&scene)
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

    fn anchor_to_overlay_point(&self, scene: &Scene, anchor: DrawingAnchor) -> Option<Point> {
        let panel_index = self.panel_index_for_id(anchor.panel_id)?;
        let panel = scene.layout.panel(panel_index)?;
        let panel_precision = self.panel_value_precision(panel_index);
        let value = self.panel_unit_to_value(panel_precision, anchor.y_unit);
        let x_unit = self.x_unit_for_anchor_time(scene, anchor.time)?;
        let x_plot = scene
            .map_x_plot(x_unit)
            .clamp(0.0, scene.layout.regions.plot.width.max(1.0));
        let y_plot = match panel.kind {
            KlinePanelKind::PrimaryChart => {
                scene.map_primary_plot_with_anchor(value, scene.primary_scale_anchor)
            }
            KlinePanelKind::Indicator => scene.map_indicator_plot(panel_index, value)?,
        }
        .clamp(panel.plot.y, panel.plot.y + panel.plot.height);

        Some(Point::new(
            scene.layout.regions.plot.x + x_plot,
            scene.layout.regions.plot.y + y_plot,
        ))
    }

    fn draw_drawing_object(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        object: &DrawingObject,
        style: DrawingStyle,
        selected: bool,
    ) {
        let stroke = canvas::Stroke::default()
            .with_color(style.stroke_color)
            .with_width(if selected {
                (style.stroke_width + 0.6).max(1.0)
            } else {
                style.stroke_width.max(1.0)
            });

        match object {
            DrawingObject::Trendline { start, end } => {
                let Some(start) = self.anchor_to_overlay_point(scene, *start) else {
                    return;
                };
                let Some(end) = self.anchor_to_overlay_point(scene, *end) else {
                    return;
                };

                frame.stroke(&canvas::Path::line(start, end), stroke);
            }
            DrawingObject::Box { start, end } => {
                let Some(start) = self.anchor_to_overlay_point(scene, *start) else {
                    return;
                };
                let Some(end) = self.anchor_to_overlay_point(scene, *end) else {
                    return;
                };

                let left = start.x.min(end.x);
                let right = start.x.max(end.x);
                let top = start.y.min(end.y);
                let bottom = start.y.max(end.y);
                let size = Size::new((right - left).max(1.0), (bottom - top).max(1.0));
                let origin = Point::new(left, top);

                if let Some(fill) = style.fill_color {
                    frame.fill_rectangle(origin, size, fill);
                }

                frame.stroke(&canvas::Path::rectangle(origin, size), stroke);
            }
            DrawingObject::HorizontalLine { panel_id, y_unit } => {
                let Some(panel_index) = self.panel_index_for_id(*panel_id) else {
                    return;
                };
                let Some(panel) = scene.layout.panel(panel_index) else {
                    return;
                };
                let Some(bounds) = self.panel_plot_bounds_in_overlay(scene, panel_index) else {
                    return;
                };
                let panel_precision = self.panel_value_precision(panel_index);
                let value = self.panel_unit_to_value(panel_precision, *y_unit);

                let y_plot = match panel.kind {
                    KlinePanelKind::PrimaryChart => {
                        scene.map_primary_plot_with_anchor(value, scene.primary_scale_anchor)
                    }
                    KlinePanelKind::Indicator => match scene.map_indicator_plot(panel_index, value)
                    {
                        Some(y) => y,
                        None => return,
                    },
                }
                .clamp(panel.plot.y, panel.plot.y + panel.plot.height);

                let y = scene.layout.regions.plot.y + y_plot;

                frame.stroke(
                    &canvas::Path::line(
                        Point::new(bounds.x, y),
                        Point::new(bounds.x + bounds.width, y),
                    ),
                    stroke,
                );
            }
            DrawingObject::VerticalLine { time } => {
                let Some(x_unit) = self.x_unit_for_anchor_time(scene, *time) else {
                    return;
                };

                let x_plot = scene
                    .map_x_plot(x_unit)
                    .clamp(0.0, scene.layout.regions.plot.width.max(1.0));
                let x = scene.layout.regions.plot.x + x_plot;
                let plot = scene.layout.regions.plot;

                frame.stroke(
                    &canvas::Path::line(Point::new(x, plot.y), Point::new(x, plot.y + plot.height)),
                    stroke,
                );
            }
        }
    }

    fn fill_drawings(&self, frame: &mut canvas::Frame, scene: &Scene) {
        for drawing in self.drawings.iter().filter(|drawing| drawing.visible) {
            let selected = self.selected_drawing == Some(drawing.id);
            self.draw_drawing_object(frame, scene, &drawing.object, drawing.style, selected);
        }

        if let Some(draft) = self.drawing_draft {
            let mut style = draft.style();
            style.stroke_color = style.stroke_color.scale_alpha(0.9);
            if let Some(fill) = style.fill_color {
                style.fill_color = Some(fill.scale_alpha(0.5));
            }

            self.draw_drawing_object(frame, scene, &draft.preview_object(), style, false);
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

    fn drawing_hit_test_object(&self, scene: &Scene, object: &DrawingObject, point: Point) -> bool {
        let tolerance = DRAWING_HIT_TOLERANCE_PX;

        match object {
            DrawingObject::Trendline { start, end } => {
                let Some(start_point) = self.anchor_to_overlay_point(scene, *start) else {
                    return false;
                };
                let Some(end_point) = self.anchor_to_overlay_point(scene, *end) else {
                    return false;
                };

                Self::point_segment_distance(point, start_point, end_point) <= tolerance
            }
            DrawingObject::Box { start, end } => {
                let Some(start_point) = self.anchor_to_overlay_point(scene, *start) else {
                    return false;
                };
                let Some(end_point) = self.anchor_to_overlay_point(scene, *end) else {
                    return false;
                };

                let left = start_point.x.min(end_point.x);
                let right = start_point.x.max(end_point.x);
                let top = start_point.y.min(end_point.y);
                let bottom = start_point.y.max(end_point.y);

                let within_expanded = point.x >= (left - tolerance)
                    && point.x <= (right + tolerance)
                    && point.y >= (top - tolerance)
                    && point.y <= (bottom + tolerance);

                if !within_expanded {
                    return false;
                }

                let near_edge = (point.x - left).abs() <= tolerance
                    || (point.x - right).abs() <= tolerance
                    || (point.y - top).abs() <= tolerance
                    || (point.y - bottom).abs() <= tolerance;

                near_edge
                    || (point.x >= left && point.x <= right && point.y >= top && point.y <= bottom)
            }
            DrawingObject::HorizontalLine { panel_id, y_unit } => {
                let Some(panel_index) = self.panel_index_for_id(*panel_id) else {
                    return false;
                };
                let Some(panel) = scene.layout.panel(panel_index) else {
                    return false;
                };
                let Some(bounds) = self.panel_plot_bounds_in_overlay(scene, panel_index) else {
                    return false;
                };

                let value_precision = self.panel_value_precision(panel_index);
                let value = self.panel_unit_to_value(value_precision, *y_unit);
                let y_plot = match panel.kind {
                    KlinePanelKind::PrimaryChart => {
                        scene.map_primary_plot_with_anchor(value, scene.primary_scale_anchor)
                    }
                    KlinePanelKind::Indicator => match scene.map_indicator_plot(panel_index, value)
                    {
                        Some(y) => y,
                        None => return false,
                    },
                }
                .clamp(panel.plot.y, panel.plot.y + panel.plot.height);

                let y = scene.layout.regions.plot.y + y_plot;
                point.x >= (bounds.x - tolerance)
                    && point.x <= (bounds.x + bounds.width + tolerance)
                    && (point.y - y).abs() <= tolerance
            }
            DrawingObject::VerticalLine { time } => {
                let Some(x_unit) = self.x_unit_for_anchor_time(scene, *time) else {
                    return false;
                };

                let plot = scene.layout.regions.plot;
                let x = scene.layout.regions.plot.x
                    + scene
                        .map_x_plot(x_unit)
                        .clamp(0.0, scene.layout.regions.plot.width.max(1.0));

                (point.x - x).abs() <= tolerance
                    && point.y >= (plot.y - tolerance)
                    && point.y <= (plot.y + plot.height + tolerance)
            }
        }
    }

    fn hit_test_drawings(&self, scene: &Scene, point: Point) -> Option<DrawingId> {
        self.drawings
            .iter()
            .rev()
            .filter(|drawing| drawing.visible)
            .find(|drawing| self.drawing_hit_test_object(scene, &drawing.object, point))
            .map(|drawing| drawing.id)
    }

    fn active_axis_labeled_object(&self) -> Option<DrawingObject> {
        if let Some(draft) = self.drawing_draft {
            return Some(draft.preview_object());
        }

        let selected = self.selected_drawing?;
        self.drawings
            .iter()
            .find(|drawing| drawing.id == selected && drawing.visible)
            .map(|drawing| drawing.object.clone())
    }

    fn axis_label_anchors_for_object(
        object: &DrawingObject,
    ) -> Option<(DrawingAnchor, DrawingAnchor)> {
        match object {
            DrawingObject::Trendline { start, end } | DrawingObject::Box { start, end } => {
                Some((*start, *end))
            }
            _ => None,
        }
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
    ) {
        let y_label_w = (text.len() as f32 * TEXT_SIZE * 0.6).clamp(40.0, 96.0);
        let y_label_h = TEXT_SIZE + 6.0;
        let y_label_x = scene.layout.regions.y_axis.x + 2.0;
        let y_label_y = (y - (y_label_h / 2.0)).clamp(
            scene.layout.regions.plot.y,
            scene.layout.regions.plot.y + scene.layout.regions.plot.height - y_label_h,
        );

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
        let Some(object) = self.active_axis_labeled_object() else {
            return;
        };
        let Some((start, end)) = Self::axis_label_anchors_for_object(&object) else {
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
                self.draw_y_axis_badge(frame, scene, palette, point.y, &y_text);
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
            !matches!(scene.primary_scale_mode, PanelScaleMode::PercentFromBase)
        } else {
            true
        }
    }

    fn panel_value_domain_from_scene(
        &self,
        scene: &Scene,
        panel_index: usize,
    ) -> Option<(f32, f32)> {
        let panel = scene.layout.panel(panel_index)?;

        match panel.kind {
            KlinePanelKind::PrimaryChart => {
                if !self.panel_accepts_manual_y_from_scene(scene, panel_index) {
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

        if !self.panel_accepts_manual_y_from_scene(scene, panel_index) {
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

    pub(super) fn primary_overlay_value_ids(&self) -> Vec<PanelValueId> {
        let Some(primary_panel_id) = self.composition.primary_panel_id() else {
            return Vec::new();
        };

        let Some(primary_panel) = self.composition.panel(primary_panel_id) else {
            return Vec::new();
        };

        primary_panel
            .layers
            .iter()
            .filter_map(|layer| layer.source.indicator_value_id())
            .collect()
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
                        let units_i128 = i128::from(y_unit.0) * i128::from(unit_size);
                        let units =
                            units_i128.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64;
                        return Price::from_units(units).to_f32();
                    }
                }
            }
            Some(PanelValuePrecision::BaseTickerMinQty) => {
                if let Some(series) = self.series.first() {
                    let min_qty = series.ticker_info().min_qty;
                    let exp = Qty::QTY_SCALE + i32::from(min_qty.power);

                    if let Some(unit_size) = Self::pow10_i64(exp) {
                        let units_i128 = i128::from(y_unit.0) * i128::from(unit_size);
                        let units =
                            units_i128.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64;
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

    fn fill_main_geometry(&self, frame: &mut canvas::Frame, scene: &Scene, palette: &Extended) {
        let spacing = scene.bar_spacing_px();
        let px_per_unit = spacing.as_f32();
        let max_width = spacing.as_i32().max(1);
        let candle_width = ((px_per_unit * 0.7).round() as i32)
            .clamp(1, 22)
            .min(max_width);
        let indicator_width = ((px_per_unit * 0.8).round() as i32)
            .clamp(1, 24)
            .min(max_width);

        let mut primary_line_points: Vec<Vec<Point>> = vec![Vec::new(); self.series.len()];
        let indicator_panel_channels: Vec<&'static [OverlayChannelSpec]> = scene
            .indicator_panels
            .iter()
            .map(|panel| self.overlay_channels_for_panel_value(panel.value_id))
            .collect();
        let mut indicator_line_points: Vec<Vec<Vec<Point>>> = indicator_panel_channels
            .iter()
            .map(|channels| vec![Vec::new(); channels.len()])
            .collect();
        let primary_overlay_value_ids = self.primary_overlay_value_ids();
        let primary_overlay_channels: Vec<&'static [OverlayChannelSpec]> =
            primary_overlay_value_ids
                .iter()
                .map(|value_id| self.overlay_channels_for_panel_value(Some(*value_id)))
                .collect();
        let mut primary_overlay_points: Vec<Vec<Vec<Point>>> = primary_overlay_channels
            .iter()
            .map(|channels| vec![Vec::new(); channels.len()])
            .collect();
        let indicator_zero_baselines: Vec<Option<f32>> = scene
            .indicator_panels
            .iter()
            .map(|panel| {
                scene
                    .map_indicator_plot(panel.panel_index, 0.0)
                    .or_else(|| scene.indicator_panel_bottom(panel.panel_index))
            })
            .collect();

        self.for_each_bar_unit_index(scene.x_axis, |series_index, series, x_unit, bar| {
            if x_unit < scene.min_x_unit || x_unit > scene.max_x_unit {
                return;
            }

            let x_px = scene.map_x_plot(x_unit).round() as i32;
            let is_base_series = series_index == 0;
            let series_anchor = scene
                .series_percent_anchors
                .get(series_index)
                .copied()
                .flatten();

            let y_open = scene.map_primary_plot_with_anchor(bar.open.to_f32(), series_anchor);
            let y_high = scene.map_primary_plot_with_anchor(bar.high.to_f32(), series_anchor);
            let y_low = scene.map_primary_plot_with_anchor(bar.low.to_f32(), series_anchor);
            let y_close = scene.map_primary_plot_with_anchor(bar.close.to_f32(), series_anchor);

            let color = if bar.close >= bar.open {
                palette.success.base.color
            } else {
                palette.danger.base.color
            };

            let primary_mark = if is_base_series {
                scene.primary_mark
            } else {
                MarkKind::Line
            };

            match primary_mark {
                MarkKind::Line => {
                    if let Some(points) = primary_line_points.get_mut(series_index) {
                        points.push(Point::new(x_px as f32, y_close));
                    }
                }
                MarkKind::Candle | MarkKind::Bar(_) => {
                    let body_top = y_open.min(y_close);
                    let body_h = (y_open - y_close).abs().max(1.0);
                    let candle_left = x_px - (candle_width / 2);

                    frame.fill_rectangle(
                        Point::new(candle_left as f32, body_top),
                        Size::new(candle_width as f32, body_h),
                        color,
                    );

                    let wick_w = ((candle_width as f32 * 0.16).round() as i32).clamp(1, 2);
                    let wick_left = x_px - (wick_w / 2);
                    frame.fill_rectangle(
                        Point::new(wick_left as f32, y_high.min(y_low)),
                        Size::new(wick_w as f32, (y_high - y_low).abs().max(1.0)),
                        color.scale_alpha(0.85),
                    );
                }
            }

            if !is_base_series {
                return;
            }

            for (overlay_idx, value_id) in primary_overlay_value_ids.iter().enumerate() {
                let Some(data) = series.indicator_data_for_panel_value_opt(Some(*value_id), bar)
                else {
                    continue;
                };

                let Some(channels) = primary_overlay_channels.get(overlay_idx) else {
                    continue;
                };

                for (channel_idx, channel) in channels.iter().copied().enumerate() {
                    let Some(value) = Self::overlay_channel_value(data, channel) else {
                        continue;
                    };

                    let y = scene.map_primary_plot_with_anchor(value, series_anchor);
                    if let Some(points) = primary_overlay_points
                        .get_mut(overlay_idx)
                        .and_then(|overlay| overlay.get_mut(channel_idx))
                    {
                        points.push(Point::new(x_px as f32, y));
                    }
                }
            }

            for (indicator_slot, indicator_panel) in scene.indicator_panels.iter().enumerate() {
                let Some(indicator_data) =
                    series.indicator_data_for_panel_value_opt(indicator_panel.value_id, bar)
                else {
                    continue;
                };
                let indicator_value = indicator_data.value();
                let y_indicator_baseline = indicator_zero_baselines
                    .get(indicator_slot)
                    .copied()
                    .flatten();

                if let (Some(y_indicator_value), Some(y_indicator_baseline)) = (
                    scene.map_indicator_plot(indicator_panel.panel_index, indicator_value),
                    y_indicator_baseline,
                ) {
                    match indicator_panel.mark {
                        MarkKind::Line => {
                            let Some(channels) = indicator_panel_channels.get(indicator_slot)
                            else {
                                continue;
                            };

                            for (channel_idx, channel) in channels.iter().copied().enumerate() {
                                let Some(channel_value) =
                                    Self::overlay_channel_value(indicator_data, channel)
                                else {
                                    continue;
                                };

                                let Some(y_channel_value) = scene
                                    .map_indicator_plot(indicator_panel.panel_index, channel_value)
                                else {
                                    continue;
                                };

                                if let Some(points) = indicator_line_points
                                    .get_mut(indicator_slot)
                                    .and_then(|channel_points| channel_points.get_mut(channel_idx))
                                {
                                    points.push(Point::new(x_px as f32, y_channel_value));
                                }
                            }
                        }
                        MarkKind::Candle | MarkKind::Bar(_) => {
                            let indicator_left = x_px - (indicator_width / 2);
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
                                        Point::new(indicator_left as f32, indicator_top),
                                        Size::new(indicator_width as f32, indicator_height),
                                        base_color.scale_alpha(0.3),
                                    );

                                    let overlay_abs = overlay.abs();
                                    if overlay_abs > 0.0
                                        && let Some(y_overlay) = scene.map_indicator_plot(
                                            indicator_panel.panel_index,
                                            overlay_abs,
                                        )
                                    {
                                        frame.fill_rectangle(
                                            Point::new(
                                                indicator_left as f32,
                                                y_overlay.min(y_indicator_baseline),
                                            ),
                                            Size::new(
                                                indicator_width as f32,
                                                (y_indicator_baseline - y_overlay).abs().max(1.0),
                                            ),
                                            base_color,
                                        );
                                    }
                                } else {
                                    frame.fill_rectangle(
                                        Point::new(indicator_left as f32, indicator_top),
                                        Size::new(indicator_width as f32, indicator_height),
                                        palette.secondary.strong.color,
                                    );
                                }
                            } else {
                                frame.fill_rectangle(
                                    Point::new(indicator_left as f32, indicator_top),
                                    Size::new(indicator_width as f32, indicator_height),
                                    palette.secondary.strong.color.scale_alpha(0.5),
                                );
                            }
                        }
                    }
                }
            }
        });

        for (series_index, points) in primary_line_points.iter().enumerate() {
            if points.len() < 2 {
                continue;
            }

            let is_base_series = series_index == 0;
            let line_color = if is_base_series {
                palette.background.base.text.scale_alpha(0.85)
            } else {
                let ticker = self.series[series_index].ticker_info();
                Self::comparison_line_color(ticker).scale_alpha(0.96)
            };

            let line_width = if is_base_series { 1.5 } else { 1.3 };

            let path = canvas::Path::new(|builder| {
                builder.move_to(points[0]);
                for point in points.iter().skip(1) {
                    builder.line_to(*point);
                }
            });

            frame.stroke(
                &path,
                canvas::Stroke::default()
                    .with_width(line_width)
                    .with_color(line_color),
            );
        }

        for (panel_idx, channel_points) in indicator_line_points.iter().enumerate() {
            let Some(channels) = indicator_panel_channels.get(panel_idx) else {
                continue;
            };

            for (channel_idx, points) in channel_points.iter().enumerate() {
                if points.len() < 2 {
                    continue;
                }

                let Some(channel) = channels.get(channel_idx).copied() else {
                    continue;
                };

                let path = canvas::Path::new(|builder| {
                    builder.move_to(points[0]);
                    for point in points.iter().skip(1) {
                        builder.line_to(*point);
                    }
                });

                frame.stroke(
                    &path,
                    canvas::Stroke::default()
                        .with_width(channel.line_width)
                        .with_color(Self::overlay_channel_color(channel, palette)),
                );
            }
        }

        for (overlay_idx, channels) in primary_overlay_points.iter().enumerate() {
            let Some(channel_specs) = primary_overlay_channels.get(overlay_idx) else {
                continue;
            };

            for (channel_idx, points) in channels.iter().enumerate() {
                if points.len() < 2 {
                    continue;
                }

                let Some(channel) = channel_specs.get(channel_idx).copied() else {
                    continue;
                };

                let path = canvas::Path::new(|builder| {
                    builder.move_to(points[0]);
                    for point in points.iter().skip(1) {
                        builder.line_to(*point);
                    }
                });

                frame.stroke(
                    &path,
                    canvas::Stroke::default()
                        .with_width(channel.line_width)
                        .with_color(Self::overlay_channel_color(channel, palette)),
                );
            }
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
                let y = scene.map_primary_plot_unit(tick_unit);
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
                let Some(y) = scene.map_indicator_plot_unit(panel_index, tick_unit) else {
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
        let plot_width = scene.primary_plot().width;
        let (ticks, step_units) = super::unit_ticks(
            scene.min_x_unit,
            scene.max_x_unit,
            plot_width,
            MIN_X_TICK_PX.max(40.0),
        );

        for tick in ticks {
            let x = scene.map_x_plot(tick);
            if x < 0.0 || x > plot_width {
                continue;
            }

            frame.fill_text(canvas::Text {
                content: self.format_x_label(scene.x_axis, tick, step_units),
                position: Point::new(x + 2.0, scene.layout.regions.x_axis.height / 2.0),
                color: palette.background.base.text,
                size: TEXT_SIZE.into(),
                align_x: iced::Alignment::Start.into(),
                align_y: iced::Alignment::Center.into(),
                font: style::AZERET_MONO,
                ..Default::default()
            });
        }
    }

    fn fill_overlay(&self, frame: &mut canvas::Frame, scene: &Scene, palette: &Extended) {
        self.fill_panel_controls(frame, scene, palette);
        self.fill_primary_ticker_legend(frame, scene, palette);

        if !scene.hovering_ticker_legend {
            let show_primary_panel_values = scene.ticker_legend.is_none();
            self.fill_panel_header_values(frame, scene, palette, show_primary_panel_values);
        }

        if self.has_drawing_state() {
            self.fill_drawings(frame, scene);
        }

        if scene.hovered_control.is_some() || scene.hovering_ticker_legend {
            return;
        }

        let Some(cursor) = scene.cursor else {
            return;
        };

        let line_color = palette.background.base.text.scale_alpha(0.35);

        let gx = scene.layout.regions.plot.x + cursor.x_plot;
        let panel_plot = scene
            .layout
            .panel(cursor.panel_index)
            .map(|panel| panel.plot)
            .unwrap_or(*scene.primary_plot());
        let panel_bounds = (
            scene.layout.regions.plot.y + panel_plot.y,
            scene.layout.regions.plot.y + panel_plot.y + panel_plot.height,
        );
        let gy =
            (scene.layout.regions.plot.y + cursor.y_plot).clamp(panel_bounds.0, panel_bounds.1);

        let stroke = canvas::Stroke::default()
            .with_color(line_color)
            .with_width(1.0);

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
        if scene.hovered_control.is_some() || scene.hovering_ticker_legend {
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
            self.draw_y_axis_badge(frame, scene, palette, gy, &y_text);
        }

        let x_text = self.format_x_label(scene.x_axis, cursor.x_unit, 1);
        self.draw_x_axis_badge(
            frame,
            scene,
            palette,
            scene.layout.regions.plot.x + cursor.x_plot,
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

                let Some(cursor_pos) = cursor.position_in(bounds) else {
                    if let DragMode::DrawingMove { id, .. } = state.drag_mode {
                        shell.publish(M::from(KlineWidgetEvent::DrawingDragFinished { id }));
                    }

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

                        if self.drawing_tool_allows_panning()
                            && matches!(zone, LayoutHitZone::PanelPlot(_))
                            && let Some(scene) = self.compute_scene(layout, cursor)
                        {
                            let hit_drawing = self.hit_test_drawings(&scene, cursor_pos);

                            if let Some(id) = hit_drawing {
                                let was_selected = self.selected_drawing == Some(id);

                                if !was_selected {
                                    shell.publish(M::from(KlineWidgetEvent::DrawingSelected(
                                        Some(id),
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
                                    shell.publish(M::from(KlineWidgetEvent::DrawingDragStarted {
                                        id,
                                        anchor,
                                    }));
                                    state.drag_mode = DragMode::DrawingMove { id };
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

                            if self.selected_drawing.is_some() {
                                shell.publish(M::from(KlineWidgetEvent::DrawingSelected(None)));
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
                        } else if let LayoutHitZone::PanelPlot(_) = zone
                            && !self.drawing_tool_allows_panning()
                            && let Some(anchor) =
                                self.drawing_anchor_from_layout_cursor(layout, cursor)
                        {
                            shell.publish(M::from(KlineWidgetEvent::DrawingAnchorPressed(anchor)));
                            state.drag_mode = DragMode::None;
                            state.last_cursor = Some(cursor_pos);
                            state.clear_overlay_caches();
                            state.clear_all_caches();
                            shell.capture_event();
                        } else if self.drawing_tool_allows_panning()
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
                        if self.drawing_draft.is_some() =>
                    {
                        shell.publish(M::from(KlineWidgetEvent::DrawingDraftCanceled));
                        state.drag_mode = DragMode::None;
                        state.last_cursor = None;
                        state.clear_overlay_caches();
                        state.clear_all_caches();
                        shell.capture_event();
                    }
                    mouse::Event::ButtonReleased(mouse::Button::Left) => {
                        if let DragMode::DrawingMove { id, .. } = state.drag_mode {
                            shell.publish(M::from(KlineWidgetEvent::DrawingDragFinished { id }));
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
                        } else if let DragMode::DrawingMove { id, .. } = state.drag_mode {
                            if matches!(zone, LayoutHitZone::PanelPlot(_))
                                && Self::hit_panel_control(
                                    &layout_tree,
                                    &panel_controls,
                                    cursor_pos,
                                )
                                .is_none()
                                && ticker_legend_hit.is_none()
                                && let Some(scene) = self.compute_scene(layout, cursor)
                                && let Some(new_anchor) =
                                    self.drawing_anchor_from_scene_cursor(&scene)
                            {
                                shell.publish(M::from(KlineWidgetEvent::DrawingDragMoved {
                                    id,
                                    anchor: new_anchor,
                                }));
                                state.last_cursor = Some(cursor_pos);
                                state.clear_overlay_caches();
                                state.clear_all_caches();
                                shell.capture_event();
                            }
                        } else if self.drawing_draft.is_some()
                            && matches!(zone, LayoutHitZone::PanelPlot(_))
                            && Self::hit_panel_control(&layout_tree, &panel_controls, cursor_pos)
                                .is_none()
                            && ticker_legend_hit.is_none()
                            && let Some(anchor) =
                                self.drawing_anchor_from_layout_cursor(layout, cursor)
                        {
                            shell.publish(M::from(KlineWidgetEvent::DrawingAnchorMoved(anchor)));
                            state.last_cursor = Some(cursor_pos);
                            state.clear_overlay_caches();
                            shell.capture_event();
                        } else if let DragMode::Pan { panel_index } = state.drag_mode {
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

            let plot_geom = state.plot_cache.draw(r, plot_rect.size(), |frame| {
                self.fill_main_geometry(frame, &scene, palette);
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
                self.fill_overlay(frame, &scene, palette);
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

        if self.drawing_tool_allows_panning()
            && matches!(zone, LayoutHitZone::PanelPlot(_))
            && let Some(scene) = self.compute_scene(layout, cursor)
            && let Some(hit_id) = self.hit_test_drawings(&scene, cursor_local)
        {
            if self.selected_drawing == Some(hit_id) {
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
