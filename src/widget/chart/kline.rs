mod chrome;
pub mod composition;
mod layout;
mod scene;

use crate::style;
use composition::{MarkKind, PanelScaleMode};

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
const MIN_PANEL_HEIGHT: f32 = 40.0;
const PANEL_TITLE_LEFT_PAD: f32 = 6.0;
const PANEL_TITLE_TOP_PAD: f32 = 4.0;
const PANEL_TITLE_TO_CONTROLS_GAP: f32 = 8.0;
const PANEL_CONTROL_BOX: f32 = TEXT_SIZE + 5.0;
const PANEL_CONTROL_GAP: f32 = 4.0;
const PANEL_CONTROL_ICON_SIZE: f32 = TEXT_SIZE - 1.0;

const TICKER_LEGEND_PADDING: f32 = 4.0;
const TICKER_LEGEND_ROW_H: f32 = TEXT_SIZE + 6.0;
const TICKER_LEGEND_ICON_BOX: f32 = TEXT_SIZE + 8.0;
const TICKER_LEGEND_ICON_GAP: f32 = 4.0;
const TICKER_LEGEND_TOP_OFFSET: f32 = 0.0;

const DEFAULT_PANEL_KINDS: [KlinePanelKind; 2] =
    [KlinePanelKind::PrimaryChart, KlinePanelKind::Indicator];
const DEFAULT_PANEL_SPLITS: [f32; 1] = [0.75];
const DEFAULT_PANEL_MARKS: [MarkKind; 2] = [MarkKind::Candle, MarkKind::Bar];
const DEFAULT_PANEL_SCALE_MODES: [PanelScaleMode; 2] =
    [PanelScaleMode::Absolute, PanelScaleMode::Absolute];

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

use chrome::TickerLegendHit;
use layout::{LayoutHitZone, PanelLayoutTree};
use scene::Scene;

pub trait KlineSeriesLike {
    fn ticker_info(&self) -> &TickerInfo;
    fn bars(&self) -> &[Kline];
    fn indicator_value(&self, bar: &Kline) -> f32;

    fn indicator_value_for_panel(&self, _panel_index: usize, bar: &Kline) -> f32 {
        self.indicator_value(bar)
    }

    fn indicator_value_for_panel_opt(&self, panel_index: usize, bar: &Kline) -> Option<f32> {
        Some(self.indicator_value_for_panel(panel_index, bar))
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
    PanelSplitChanged { index: usize, split: f32 },
    PanelMoveUp { index: usize },
    PanelMoveDown { index: usize },
    PanelSettings { index: usize },
    PanelClose { index: usize },
    TickerSettings(TickerInfo),
    TickerRemove(TickerInfo),
    XAxisDoubleClick,
}

struct State {
    plot_cache: canvas::Cache,
    y_axis_cache: canvas::Cache,
    x_axis_cache: canvas::Cache,
    overlay_cache: canvas::Cache,
    interaction_text_cache: canvas::Cache,
    is_panning: bool,
    dragging_split: Option<usize>,
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
            is_panning: false,
            dragging_split: None,
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
    basis: Basis,
    horizontal_scale: HorizontalScale,
    horizontal_offset: f32,
    panel_kinds: &'a [KlinePanelKind],
    panel_splits: &'a [f32],
    panel_titles: &'a [Option<String>],
    panel_marks: &'a [MarkKind],
    panel_scale_modes: &'a [PanelScaleMode],
    timezone: UserTimezone,
    version: u64,
}

impl<'a, S> KlineWidget<'a, S>
where
    S: KlineSeriesLike,
{
    pub fn new(series: &'a [S], timeframe: Timeframe) -> Self {
        Self {
            series,
            basis: Basis::Time(timeframe),
            horizontal_scale: HorizontalScale::pixels_per_bar(DEFAULT_BAR_SPACING_PX),
            horizontal_offset: 0.0,
            panel_kinds: &DEFAULT_PANEL_KINDS,
            panel_splits: &DEFAULT_PANEL_SPLITS,
            panel_titles: &[],
            panel_marks: &DEFAULT_PANEL_MARKS,
            panel_scale_modes: &DEFAULT_PANEL_SCALE_MODES,
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

    pub fn with_panel_layout(
        mut self,
        panel_kinds: &'a [KlinePanelKind],
        panel_splits: &'a [f32],
    ) -> Self {
        self.panel_kinds = panel_kinds;
        self.panel_splits = panel_splits;
        self
    }

    pub fn with_panel_titles(mut self, panel_titles: &'a [Option<String>]) -> Self {
        self.panel_titles = panel_titles;
        self
    }

    pub fn with_panel_rendering(
        mut self,
        panel_marks: &'a [MarkKind],
        panel_scale_modes: &'a [PanelScaleMode],
    ) -> Self {
        self.panel_marks = panel_marks;
        self.panel_scale_modes = panel_scale_modes;
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

    fn resolved_panel_kinds(&self) -> &[KlinePanelKind] {
        if self.panel_kinds.is_empty() {
            &DEFAULT_PANEL_KINDS
        } else {
            self.panel_kinds
        }
    }

    fn default_mark_for_panel(kind: KlinePanelKind) -> MarkKind {
        match kind {
            KlinePanelKind::PrimaryChart => MarkKind::Candle,
            KlinePanelKind::Indicator => MarkKind::Bar,
        }
    }

    fn default_title_for_panel(kind: KlinePanelKind) -> Option<&'static str> {
        match kind {
            KlinePanelKind::PrimaryChart => None,
            KlinePanelKind::Indicator => Some("Indicator"),
        }
    }

    fn resolved_panel_title(&self, panel_index: usize, panel_kind: KlinePanelKind) -> Option<&str> {
        self.panel_titles
            .get(panel_index)
            .and_then(|title| title.as_deref())
            .filter(|title| !title.is_empty())
            .or_else(|| Self::default_title_for_panel(panel_kind))
    }

    fn resolved_panel_mark(&self, panel_index: usize, panel_kind: KlinePanelKind) -> MarkKind {
        self.panel_marks
            .get(panel_index)
            .copied()
            .unwrap_or_else(|| Self::default_mark_for_panel(panel_kind))
    }

    fn resolved_panel_scale_mode(&self, panel_index: usize) -> PanelScaleMode {
        self.panel_scale_modes
            .get(panel_index)
            .copied()
            .unwrap_or(PanelScaleMode::Absolute)
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
        let mut indicator_line_points: Vec<Vec<Point>> =
            vec![Vec::new(); scene.indicator_panels.len()];
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
                MarkKind::Candle | MarkKind::Bar => {
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

            for (indicator_slot, indicator_panel) in scene.indicator_panels.iter().enumerate() {
                let Some(indicator_value) =
                    series.indicator_value_for_panel_opt(indicator_panel.panel_index, bar)
                else {
                    continue;
                };
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
                            if let Some(points) = indicator_line_points.get_mut(indicator_slot) {
                                points.push(Point::new(x_px as f32, y_indicator_value));
                            }
                        }
                        MarkKind::Candle | MarkKind::Bar => {
                            let indicator_left = x_px - (indicator_width / 2);
                            frame.fill_rectangle(
                                Point::new(
                                    indicator_left as f32,
                                    y_indicator_value.min(y_indicator_baseline),
                                ),
                                Size::new(
                                    indicator_width as f32,
                                    (y_indicator_baseline - y_indicator_value).abs().max(1.0),
                                ),
                                color.scale_alpha(0.4),
                            );
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

        for points in &indicator_line_points {
            if points.len() < 2 {
                continue;
            }

            let path = canvas::Path::new(|builder| {
                builder.move_to(points[0]);
                for point in points.iter().skip(1) {
                    builder.line_to(*point);
                }
            });

            frame.stroke(
                &path,
                canvas::Stroke::default()
                    .with_width(1.2)
                    .with_color(palette.background.base.text.scale_alpha(0.55)),
            );
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
        let total_ticks = (scene.primary_plot().height / (TEXT_SIZE * 2.5)).floor() as usize;
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

        let Some(cursor) = scene.cursor else {
            return;
        };

        let gy = scene.layout.regions.plot.y + cursor.y_plot;

        if let Some(y_text) = cursor
            .y_primary_value
            .map(|primary_value| scene.format_primary_cursor_label(primary_value))
            .or_else(|| {
                cursor
                    .y_indicator_value
                    .map(|indicator_value| format!("{indicator_value:.2}"))
            })
        {
            let y_label_w = (y_text.len() as f32 * TEXT_SIZE * 0.6).clamp(40.0, 96.0);
            let y_label_h = TEXT_SIZE + 6.0;
            let y_label_x = scene.layout.regions.y_axis.x + 2.0;
            let y_label_y = (gy - (y_label_h / 2.0)).clamp(
                scene.layout.regions.plot.y,
                scene.layout.regions.plot.y + scene.layout.regions.plot.height - y_label_h,
            );

            frame.fill_rectangle(
                Point::new(y_label_x, y_label_y),
                Size::new(y_label_w, y_label_h),
                palette.background.strong.color,
            );

            frame.fill_text(canvas::Text {
                content: y_text,
                position: Point::new(y_label_x + y_label_w - 4.0, y_label_y + y_label_h / 2.0),
                color: palette.background.strong.text,
                size: TEXT_SIZE.into(),
                align_x: iced::Alignment::End.into(),
                align_y: iced::Alignment::Center.into(),
                font: style::AZERET_MONO,
                ..Default::default()
            });
        }

        let x_text = self.format_x_label(scene.x_axis, cursor.x_unit, 1);
        let x_label_w = (x_text.len() as f32 * TEXT_SIZE * 0.62).clamp(60.0, 180.0);
        let x_label_h = TEXT_SIZE + 6.0;
        let x_label_x = (scene.layout.regions.plot.x + cursor.x_plot - x_label_w / 2.0).clamp(
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
            content: x_text,
            position: Point::new(x_label_x + x_label_w / 2.0, x_label_y + x_label_h / 2.0),
            color: palette.background.strong.text,
            size: TEXT_SIZE.into(),
            align_x: iced::Alignment::Center.into(),
            align_y: iced::Alignment::Center.into(),
            font: style::AZERET_MONO,
            ..Default::default()
        });
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
        let panel_count = self.resolved_panel_kinds().len().max(1);

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
                    PanelLayoutTree::from_layout(layout, self.resolved_panel_kinds())
                else {
                    return;
                };

                let Some(cursor_pos) = cursor.position_in(bounds) else {
                    if state.is_panning || state.dragging_split.is_some() {
                        state.is_panning = false;
                        state.dragging_split = None;
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
                    mouse::Event::WheelScrolled {
                        delta: mouse::ScrollDelta::Lines { y, .. },
                    } => {
                        if !matches!(zone, LayoutHitZone::PanelPlot(_)) {
                            return;
                        }

                        if Self::hit_panel_control(&layout_tree, &panel_controls, cursor_pos)
                            .is_some()
                        {
                            return;
                        }

                        if ticker_legend_hit.is_some() {
                            return;
                        }

                        let zoom_in = *y > 0.0;
                        let new_scale =
                            self.step_horizontal_scale_percent(self.horizontal_scale, zoom_in);

                        if (new_scale.as_pixels_per_bar()
                            - self.horizontal_scale.as_pixels_per_bar())
                        .abs()
                            > f32::EPSILON
                        {
                            shell.publish(M::from(KlineWidgetEvent::HorizontalScaleChanged(
                                self.normalize_horizontal_scale(new_scale),
                            )));
                            state.clear_all_caches();
                        }
                    }
                    mouse::Event::ButtonPressed(mouse::Button::Left) => {
                        if let Some(global_pos) = cursor.position() {
                            let new_click = iced_core::mouse::Click::new(
                                global_pos,
                                mouse::Button::Left,
                                state.previous_click,
                            );

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
                            state.is_panning = false;
                            state.dragging_split = None;
                            state.last_cursor = None;
                            state.clear_all_caches();
                            shell.capture_event();
                            return;
                        }

                        if ticker_legend_hit.is_some() {
                            state.is_panning = false;
                            state.dragging_split = None;
                            state.last_cursor = None;
                            return;
                        }

                        if matches!(zone, LayoutHitZone::PanelPlot(_))
                            && let Some(control) =
                                Self::hit_panel_control(&layout_tree, &panel_controls, cursor_pos)
                        {
                            shell.publish(M::from(control.kind.into_event(control.panel_index)));
                            state.is_panning = false;
                            state.dragging_split = None;
                            state.last_cursor = None;
                            state.clear_all_caches();
                            shell.capture_event();
                            return;
                        }

                        if let LayoutHitZone::Splitter(split_index) = zone {
                            state.dragging_split = Some(split_index);
                            state.is_panning = false;
                            state.last_cursor = Some(cursor_pos);
                            shell.capture_event();
                        } else if matches!(zone, LayoutHitZone::PanelPlot(_)) {
                            state.is_panning = true;
                            state.last_cursor = Some(cursor_pos);
                        }
                    }
                    mouse::Event::ButtonReleased(mouse::Button::Left) => {
                        state.is_panning = false;
                        state.dragging_split = None;
                        state.last_cursor = None;
                    }
                    mouse::Event::CursorMoved { .. } => {
                        state.clear_overlay_caches();

                        if let Some(split_index) = state.dragging_split {
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
                        } else if state.is_panning {
                            let prev = state.last_cursor.unwrap_or(cursor_pos);
                            let dx_px = cursor_pos.x - prev.x;

                            if dx_px.abs() > 0.0 {
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

        let Some(layout_tree) = PanelLayoutTree::from_layout(layout, self.resolved_panel_kinds())
        else {
            return advanced::mouse::Interaction::default();
        };
        let state = tree.state.downcast_ref::<State>();

        if state.dragging_split.is_some() {
            return advanced::mouse::Interaction::ResizingVertically;
        }

        if state.is_panning {
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

        match zone {
            LayoutHitZone::Splitter(_) => advanced::mouse::Interaction::ResizingVertically,
            LayoutHitZone::PanelPlot(_) => advanced::mouse::Interaction::Crosshair,
            LayoutHitZone::PanelXAxis(_) | LayoutHitZone::BottomXAxis | LayoutHitZone::YAxis => {
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
