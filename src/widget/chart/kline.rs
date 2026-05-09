mod chrome;
pub mod composition;
pub mod coord;
pub mod drawing;
mod helpers;
mod layout;
mod scene;

use crate::style;
use crate::widget::chart::kline::drawing::{
    DrawingDragTarget, DrawingId, DrawingSnapshot, DrawingTool, KlineWidgetDrawingEvent,
};
use chrome::TickerLegendHit;
use composition::{
    BarMode, ChartComposition, DEFAULT_MIN_PANEL_RATIO, HistogramMode, LayerDataKind, MarkKind,
    PanelId, PanelScaleMode, PanelValueId,
};
use layout::{LayoutHitZone, PanelLayoutTree};
use scene::Scene;

use data::UserTimezone;
use data::chart::Basis;
use exchange::{Kline, TickerInfo, Timeframe};

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
const BAR_SPACING_SUBPIXEL_UNITS_PER_PX: i32 = 128;
const BAR_SPACING_QUANTUM_PX: f32 = 1.0 / BAR_SPACING_SUBPIXEL_UNITS_PER_PX as f32;
const HORIZONTAL_ZOOM_LINE_DELTA_MODE_ADJUSTMENT: f32 = 32.0;
const HORIZONTAL_ZOOM_PIXEL_DELTA_MODE_ADJUSTMENT: f32 = 1.0;
const HORIZONTAL_ZOOM_DELTA_NORMALIZATION: f32 = 100.0;
const HORIZONTAL_ZOOM_MAX_ACCUMULATED_CHUNKS_PER_EVENT: usize = 8;
const HORIZONTAL_ZOOM_MAX_ACCUMULATED_DELTA: f32 = 8.0;

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

    pub fn span(self) -> f32 {
        (self.max_value - self.min_value).abs()
    }

    pub fn zoom_around_ratio(
        self,
        anchor_ratio: f32,
        zoom_scale: f32,
        zoom_in: bool,
        min_span: f32,
    ) -> Option<Self> {
        let anchor_ratio = anchor_ratio.clamp(0.0, 1.0);
        let span = self.span().max(1e-8);
        let zoom_scale = zoom_scale.abs().max(1.0);
        let min_span = min_span.abs().max(1e-8);

        let new_span = if zoom_in {
            span / zoom_scale
        } else {
            span * zoom_scale
        }
        .max(min_span);

        let anchor_value = self.min_value + anchor_ratio * span;
        let new_min = anchor_value - anchor_ratio * new_span;
        let new_max = new_min + new_span;
        Self::normalized(new_min, new_max)
    }

    pub fn pan_by_ratio(self, delta_ratio: f32) -> Option<Self> {
        if !delta_ratio.is_finite() {
            return None;
        }

        let span = self.span().max(1e-8);
        let shift = delta_ratio * span;
        Self::normalized(self.min_value + shift, self.max_value + shift)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct YUnit(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BarSpacingPx(i32);

impl BarSpacingPx {
    fn min_units() -> i32 {
        ((MIN_BAR_SPACING_PX / BAR_SPACING_QUANTUM_PX).ceil() as i32).max(1)
    }

    fn max_units() -> i32 {
        ((MAX_BAR_SPACING_PX / BAR_SPACING_QUANTUM_PX).floor() as i32).max(Self::min_units())
    }

    fn from_units(units: i32) -> Self {
        Self(units.clamp(Self::min_units(), Self::max_units()))
    }

    fn from_logical(px: f32) -> Self {
        let quantized_units = (px.clamp(MIN_BAR_SPACING_PX, MAX_BAR_SPACING_PX)
            / BAR_SPACING_QUANTUM_PX)
            .round() as i32;

        Self::from_units(quantized_units)
    }

    fn as_f32(self) -> f32 {
        self.0 as f32 * BAR_SPACING_QUANTUM_PX
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
enum KlinePanelKind {
    PrimaryChart,
    Indicator,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PanelInteraction {
    YViewportChanged {
        panel_id: PanelId,
        viewport: PanelYViewport,
    },
    YViewportReset {
        panel_id: PanelId,
    },
    SplitChanged {
        index: usize,
        split: f32,
    },
    MoveUp {
        index: usize,
    },
    MoveDown {
        index: usize,
    },
    Settings {
        index: usize,
    },
    Close {
        index: usize,
    },
}

#[derive(Debug, Clone)]
pub enum KlineWidgetEvent {
    HorizontalScaleChanged(HorizontalScale),
    HorizontalOffsetChanged(f32),
    PrimaryAutoscaleToggled,
    PrimaryScaleModeCycleRequested,
    PanelInteraction(PanelInteraction),
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
    horizontal_zoom_scroll_accum: f32,
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
            horizontal_zoom_scroll_accum: 0.0,
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
                .map_indicator_plot(indicator_panel.panel_index, 0.0)
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
                    let Some(y_indicator_value) =
                        scene.map_indicator_plot(indicator_panel.panel_index, indicator_value)
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
                                && let Some(y_overlay) = scene
                                    .map_indicator_plot(indicator_panel.panel_index, overlay_abs)
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
                                            scene.map_primary_plot_with_anchor(
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
                    let y_open =
                        scene.map_primary_plot_with_anchor(bar.open.to_f32(), series_anchor);
                    let y_high =
                        scene.map_primary_plot_with_anchor(bar.high.to_f32(), series_anchor);
                    let y_low = scene.map_primary_plot_with_anchor(bar.low.to_f32(), series_anchor);
                    let y_close =
                        scene.map_primary_plot_with_anchor(bar.close.to_f32(), series_anchor);

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
                                    scene.map_primary_plot_with_anchor(
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

                                let Some(y_channel_value) = scene
                                    .map_indicator_plot(indicator_panel.panel_index, channel_value)
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
                let y = scene.map_primary_plot_unit(tick_unit).clamp(
                    scene.primary_plot().y,
                    scene.primary_plot().y + scene.primary_plot().height,
                );
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
                let y = y.clamp(panel.plot.y, panel.plot.y + panel.plot.height);

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

                        if !matches!(zone, LayoutHitZone::PanelPlot(_)) {
                            state.horizontal_zoom_scroll_accum = 0.0;
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
                                    state.horizontal_zoom_scroll_accum = 0.0;
                                    return;
                                }

                                if Self::hit_corner_control(&corner_controls, cursor_pos).is_some()
                                {
                                    state.horizontal_zoom_scroll_accum = 0.0;
                                    return;
                                }

                                if ticker_legend_hit.is_some() {
                                    state.horizontal_zoom_scroll_accum = 0.0;
                                    return;
                                }

                                let adjustment = match delta {
                                    mouse::ScrollDelta::Lines { .. } => {
                                        HORIZONTAL_ZOOM_LINE_DELTA_MODE_ADJUSTMENT
                                    }
                                    mouse::ScrollDelta::Pixels { .. } => {
                                        #[cfg(target_os = "windows")]
                                        {
                                            HORIZONTAL_ZOOM_PIXEL_DELTA_MODE_ADJUSTMENT
                                                / self.resolved_horizontal_pixel_ratio().max(1.0)
                                        }

                                        #[cfg(not(target_os = "windows"))]
                                        {
                                            HORIZONTAL_ZOOM_PIXEL_DELTA_MODE_ADJUSTMENT
                                        }
                                    }
                                };

                                let normalized_zoom_delta =
                                    (scroll_y * adjustment) / HORIZONTAL_ZOOM_DELTA_NORMALIZATION;

                                if normalized_zoom_delta.abs() <= f32::EPSILON {
                                    return;
                                }

                                let is_pixel_delta =
                                    matches!(delta, mouse::ScrollDelta::Pixels { .. });

                                if !is_pixel_delta {
                                    state.horizontal_zoom_scroll_accum = 0.0;
                                }

                                let mut remaining_zoom_delta = if is_pixel_delta {
                                    state.horizontal_zoom_scroll_accum + normalized_zoom_delta
                                } else {
                                    normalized_zoom_delta
                                };

                                let current_scale =
                                    self.normalize_horizontal_scale(self.horizontal_scale);
                                let mut new_scale = current_scale;
                                let mut applied_chunks = 0usize;

                                while remaining_zoom_delta.abs() > f32::EPSILON
                                    && applied_chunks
                                        < HORIZONTAL_ZOOM_MAX_ACCUMULATED_CHUNKS_PER_EVENT
                                {
                                    let zoom_scale = remaining_zoom_delta.clamp(-1.0, 1.0);
                                    let stepped =
                                        self.step_horizontal_scale_percent(new_scale, zoom_scale);

                                    if (stepped.as_pixels_per_bar() - new_scale.as_pixels_per_bar())
                                        .abs()
                                        <= f32::EPSILON
                                    {
                                        remaining_zoom_delta = 0.0;
                                        break;
                                    }

                                    new_scale = stepped;
                                    applied_chunks = applied_chunks.saturating_add(1);

                                    if is_pixel_delta {
                                        remaining_zoom_delta -= zoom_scale;

                                        // Keep partial pixel delta for the next wheel event.
                                        if remaining_zoom_delta.abs() < 1.0 {
                                            break;
                                        }
                                    } else {
                                        remaining_zoom_delta = 0.0;
                                    }
                                }

                                if is_pixel_delta {
                                    state.horizontal_zoom_scroll_accum = remaining_zoom_delta
                                        .clamp(
                                            -HORIZONTAL_ZOOM_MAX_ACCUMULATED_DELTA,
                                            HORIZONTAL_ZOOM_MAX_ACCUMULATED_DELTA,
                                        );
                                } else {
                                    state.horizontal_zoom_scroll_accum = 0.0;
                                }

                                if (new_scale.as_pixels_per_bar()
                                    - current_scale.as_pixels_per_bar())
                                .abs()
                                    > f32::EPSILON
                                {
                                    shell.publish(M::from(
                                        KlineWidgetEvent::HorizontalScaleChanged(new_scale),
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

                                let interaction =
                                    PanelInteraction::YViewportChanged { panel_id, viewport };

                                shell.publish(M::from(KlineWidgetEvent::PanelInteraction(
                                    interaction,
                                )));
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
                                let interaction = PanelInteraction::YViewportReset { panel_id };

                                shell.publish(M::from(KlineWidgetEvent::PanelInteraction(
                                    interaction,
                                )));
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
                            let interaction =
                                control.kind.into_interaction_kind(control.panel_index);

                            shell.publish(M::from(KlineWidgetEvent::PanelInteraction(interaction)));
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
                                let interaction = PanelInteraction::SplitChanged {
                                    index: split_index,
                                    split,
                                };

                                shell.publish(M::from(KlineWidgetEvent::PanelInteraction(
                                    interaction,
                                )));
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
                                let interaction =
                                    PanelInteraction::YViewportChanged { panel_id, viewport };

                                shell.publish(M::from(KlineWidgetEvent::PanelInteraction(
                                    interaction,
                                )));
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
                                let interaction =
                                    PanelInteraction::YViewportChanged { panel_id, viewport };

                                shell.publish(M::from(KlineWidgetEvent::PanelInteraction(
                                    interaction,
                                )));
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
