use crate::chart::composition::{MarkKind, PanelScaleMode};
use crate::style;
use crate::widget::chart::Regions;
use crate::widget::chart::Zoom;

use data::UserTimezone;
use data::chart::Basis;
use exchange::{Kline, TickerInfo, Timeframe, UnixMs};

use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{self, Clipboard, Layout, Shell, Widget, layout, renderer};
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
const ZOOM_STEP_PCT: f32 = 0.05;
const PANEL_X_AXIS_HEIGHT: f32 = 18.0;
const PANEL_SPLITTER_HEIGHT: f32 = 1.0;
const PANEL_SPLITTER_HIT_PX: f32 = 8.0;
const MIN_PANEL_HEIGHT: f32 = 40.0;
const PANEL_TITLE_LEFT_PAD: f32 = 6.0;
const PANEL_TITLE_TOP_PAD: f32 = 4.0;
const PANEL_TITLE_TO_CONTROLS_GAP: f32 = 8.0;
const PANEL_CONTROL_BOX: f32 = TEXT_SIZE + 5.0;
const PANEL_CONTROL_GAP: f32 = 4.0;
const PANEL_CONTROL_ICON_SIZE: f32 = TEXT_SIZE - 1.0;
const PANEL_CONTROL_TOOLTIP_GAP: f32 = 3.0;

const TICKER_LEGEND_PADDING: f32 = 4.0;
const TICKER_LEGEND_ROW_H: f32 = TEXT_SIZE + 6.0;
const TICKER_LEGEND_ICON_BOX: f32 = TEXT_SIZE + 6.0;
const TICKER_LEGEND_ICON_GAP: f32 = 4.0;
const TICKER_LEGEND_TOP_OFFSET: f32 = TEXT_SIZE + 10.0;

const DEFAULT_PANEL_KINDS: [KlinePanelKind; 2] =
    [KlinePanelKind::PrimaryChart, KlinePanelKind::Indicator];
const DEFAULT_PANEL_SPLITS: [f32; 1] = [0.75];
const DEFAULT_PANEL_MARKS: [MarkKind; 2] = [MarkKind::Candle, MarkKind::Bar];
const DEFAULT_PANEL_SCALE_MODES: [PanelScaleMode; 2] =
    [PanelScaleMode::Absolute, PanelScaleMode::Absolute];

pub const DEFAULT_ZOOM_POINTS: usize = 150;
pub const MIN_ZOOM_POINTS: usize = 2;
pub const MAX_ZOOM_POINTS: usize = 5000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct TickIndex(u64);

impl TickIndex {
    const ZERO: Self = Self(0);

    #[inline]
    fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy)]
enum XAxis {
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

pub trait KlineSeriesLike {
    fn ticker_info(&self) -> &TickerInfo;
    fn bars(&self) -> &[Kline];
    fn indicator_value(&self, bar: &Kline) -> f32;

    fn indicator_value_for_panel(&self, _panel_index: usize, bar: &Kline) -> f32 {
        self.indicator_value(bar)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KlinePanelKind {
    PrimaryChart,
    Indicator,
}

#[derive(Debug, Clone)]
pub enum KlineWidgetEvent {
    ZoomChanged(Zoom),
    PanChanged(f32),
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
    is_panning: bool,
    dragging_split: Option<usize>,
    last_cursor: Option<Point>,
    last_cache_rev: u64,
    previous_click: Option<iced_core::mouse::Click>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LayoutHitZone {
    PanelPlot(usize),
    PanelXAxis(usize),
    BottomXAxis,
    YAxis,
    Splitter(usize),
    Outside,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanelControlKind {
    MoveUp,
    MoveDown,
    Settings,
    Close,
}

impl PanelControlKind {
    fn icon(self) -> &'static str {
        match self {
            Self::MoveUp => "^",
            Self::MoveDown => "v",
            Self::Settings => "S",
            Self::Close => "X",
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            Self::MoveUp => "Move panel up",
            Self::MoveDown => "Move panel down",
            Self::Settings => "Panel settings",
            Self::Close => "Remove panel",
        }
    }

    fn into_event(self, index: usize) -> KlineWidgetEvent {
        match self {
            Self::MoveUp => KlineWidgetEvent::PanelMoveUp { index },
            Self::MoveDown => KlineWidgetEvent::PanelMoveDown { index },
            Self::Settings => KlineWidgetEvent::PanelSettings { index },
            Self::Close => KlineWidgetEvent::PanelClose { index },
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PanelControlHit {
    panel_index: usize,
    kind: PanelControlKind,
    rect: Rectangle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TickerLegendIconKind {
    Settings,
    Close,
}

impl TickerLegendIconKind {
    fn icon(self) -> &'static str {
        match self {
            Self::Settings => "S",
            Self::Close => "X",
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            Self::Settings => "Ticker settings",
            Self::Close => "Remove ticker",
        }
    }

    fn into_event(self, ticker: TickerInfo) -> KlineWidgetEvent {
        match self {
            Self::Settings => KlineWidgetEvent::TickerSettings(ticker),
            Self::Close => KlineWidgetEvent::TickerRemove(ticker),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct TickerLegendRowHit {
    ticker: TickerInfo,
    y_center: f32,
    row_rect: Rectangle,
    settings: Rectangle,
    close: Rectangle,
    has_close: bool,
}

#[derive(Debug, Clone)]
struct TickerLegendLayout {
    bg: Rectangle,
    rows: Vec<TickerLegendRowHit>,
}

#[derive(Debug, Clone, Copy)]
enum TickerLegendHit {
    Background,
    Row(usize),
    Icon(usize, TickerLegendIconKind),
}

#[derive(Debug, Clone, Copy)]
struct PanelLayoutNode {
    kind: KlinePanelKind,
    plot: Rectangle,
    x_axis: Rectangle,
}

#[derive(Debug, Clone)]
struct PanelLayoutTree {
    regions: Regions,
    panels: Vec<PanelLayoutNode>,
    splitters: Vec<Rectangle>,
}

impl PanelLayoutTree {
    fn child_at(layout: Layout<'_>, index: usize) -> Option<Layout<'_>> {
        (index < layout.children().len()).then(|| layout.child(index))
    }

    fn from_layout(root: Layout<'_>, panel_kinds: &[KlinePanelKind]) -> Option<Self> {
        if panel_kinds.is_empty() {
            return None;
        }

        let regions = Regions::from_layout(root);

        let row = Self::child_at(root, 0)?;
        let panels = Self::child_at(row, 0)?;

        let panels_bounds = panels.bounds();
        let to_plot_local = |r: Rectangle| Rectangle {
            x: r.x - panels_bounds.x,
            y: r.y - panels_bounds.y,
            width: r.width,
            height: r.height,
        };

        let mut cursor = 0usize;
        let mut panel_nodes = Vec::with_capacity(panel_kinds.len());
        let mut splitters = Vec::with_capacity(panel_kinds.len().saturating_sub(1));

        for (index, kind) in panel_kinds.iter().copied().enumerate() {
            let plot = to_plot_local(Self::child_at(panels, cursor)?.bounds());
            cursor += 1;

            let x_axis = to_plot_local(Self::child_at(panels, cursor)?.bounds());
            cursor += 1;

            panel_nodes.push(PanelLayoutNode { kind, plot, x_axis });

            if index + 1 < panel_kinds.len() {
                splitters.push(to_plot_local(Self::child_at(panels, cursor)?.bounds()));
                cursor += 1;
            }
        }

        Some(Self {
            regions,
            panels: panel_nodes,
            splitters,
        })
    }

    fn panel(&self, index: usize) -> Option<&PanelLayoutNode> {
        self.panels.get(index)
    }

    fn primary_plot_width(&self) -> f32 {
        self.panels
            .first()
            .map(|panel| panel.plot.width)
            .unwrap_or(self.regions.plot.width)
    }

    fn contains(rect: Rectangle, p: Point) -> bool {
        p.x >= rect.x && p.x <= rect.x + rect.width && p.y >= rect.y && p.y <= rect.y + rect.height
    }

    fn plot_local_point(&self, root_local: Point) -> Option<Point> {
        self.regions.is_in_plot(root_local).then_some(Point::new(
            root_local.x - self.regions.plot.x,
            root_local.y - self.regions.plot.y,
        ))
    }

    fn splitter_hit_rect(splitter: Rectangle) -> Rectangle {
        let hit_h = PANEL_SPLITTER_HIT_PX;
        let center_y = splitter.y + splitter.height * 0.5;

        Rectangle {
            x: splitter.x,
            y: center_y - hit_h * 0.5,
            width: splitter.width,
            height: hit_h,
        }
    }

    fn hit_test(&self, root_local: Point) -> LayoutHitZone {
        if self.regions.is_in_y_axis(root_local) {
            return LayoutHitZone::YAxis;
        }

        if self.regions.is_in_x_axis(root_local) {
            return LayoutHitZone::BottomXAxis;
        }

        let Some(plot_local) = self.plot_local_point(root_local) else {
            return LayoutHitZone::Outside;
        };

        for (index, splitter) in self.splitters.iter().copied().enumerate() {
            if Self::contains(Self::splitter_hit_rect(splitter), plot_local) {
                return LayoutHitZone::Splitter(index);
            }
        }

        for (index, panel) in self.panels.iter().enumerate() {
            if Self::contains(panel.plot, plot_local) {
                return LayoutHitZone::PanelPlot(index);
            }

            if Self::contains(panel.x_axis, plot_local) {
                return LayoutHitZone::PanelXAxis(index);
            }
        }

        LayoutHitZone::Outside
    }
}

impl Default for State {
    fn default() -> Self {
        Self {
            plot_cache: canvas::Cache::new(),
            y_axis_cache: canvas::Cache::new(),
            x_axis_cache: canvas::Cache::new(),
            overlay_cache: canvas::Cache::new(),
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
        self.overlay_cache.clear();
    }
}

pub struct KlineWidget<'a, S> {
    series: &'a [S],
    basis: Basis,
    zoom: Zoom,
    pan: f32,
    panel_kinds: &'a [KlinePanelKind],
    panel_splits: &'a [f32],
    panel_titles: &'a [String],
    panel_marks: &'a [MarkKind],
    panel_scale_modes: &'a [PanelScaleMode],
    timezone: UserTimezone,
    version: u64,
}

#[derive(Debug, Clone, Copy)]
struct CursorInfo {
    x_unit: i64,
    panel_index: usize,
    y_primary_value: Option<f32>,
    y_indicator_value: Option<f32>,
    x_plot: f32,
    y_plot: f32,
}

#[derive(Debug, Clone, Copy)]
struct IndicatorPanelScene {
    panel_index: usize,
    mark: MarkKind,
    min_value: f32,
    max_value: f32,
}

#[derive(Debug, Clone)]
struct Scene {
    layout: PanelLayoutTree,
    x_axis: XAxis,
    min_x_unit: i64,
    max_x_unit: i64,
    min_primary_value: f32,
    max_primary_value: f32,
    primary_panel: usize,
    primary_mark: MarkKind,
    primary_scale_mode: PanelScaleMode,
    primary_scale_anchor: Option<f32>,
    series_percent_anchors: Vec<Option<f32>>,
    indicator_panels: Vec<IndicatorPanelScene>,
    panel_controls: Vec<PanelControlHit>,
    ticker_legend: Option<TickerLegendLayout>,
    controls_visible_for_panel: Option<usize>,
    hovered_control: Option<PanelControlHit>,
    hovering_ticker_legend: bool,
    hovered_ticker_row: Option<usize>,
    hovered_ticker_icon: Option<(usize, TickerLegendIconKind)>,
    cursor: Option<CursorInfo>,
}

impl Scene {
    fn plot_rect(&self) -> Rectangle {
        self.layout.regions.plot
    }

    fn span_units(&self) -> f32 {
        (self.max_x_unit - self.min_x_unit).max(1) as f32
    }

    fn primary_plot(&self) -> &Rectangle {
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

    fn indicator_panel_bottom(&self, panel_index: usize) -> Option<f32> {
        self.indicator_plot(panel_index)
            .map(|rect| rect.y + rect.height)
    }

    fn map_x_plot(&self, x_unit: i64) -> f32 {
        let span = self.span_units();
        let ratio = ((x_unit - self.min_x_unit) as f32 / span).clamp(0.0, 1.0);
        ratio * self.primary_plot().width
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

    fn primary_domain_display_values(&self) -> (f32, f32) {
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

    fn primary_display_to_value(&self, display_value: f32) -> f32 {
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

    fn format_primary_axis_label(&self, display_value: f32, display_step: f32) -> String {
        match self.primary_scale_mode {
            PanelScaleMode::PercentFromBase => format!("{display_value:.2}%"),
            PanelScaleMode::Logarithmic if self.can_use_log_primary_scale() => {
                let value = self.primary_display_to_value(display_value);
                let next_value =
                    self.primary_display_to_value(display_value + display_step.abs().max(1e-3));
                let value_step = (next_value - value).abs().max(1e-6);
                super::format_value(value, value_step)
            }
            _ => super::format_value(display_value, display_step),
        }
    }

    fn format_primary_cursor_label(&self, raw_value: f32) -> String {
        match self.primary_scale_mode {
            PanelScaleMode::PercentFromBase => {
                format!("{:.2}%", self.primary_to_display_value(raw_value))
            }
            _ => super::format_value(raw_value, 0.01),
        }
    }

    fn map_primary_plot_with_anchor(&self, value: f32, anchor: Option<f32>) -> f32 {
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

    fn map_indicator_plot(&self, panel_index: usize, indicator_value: f32) -> Option<f32> {
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
    pub fn new(series: &'a [S], timeframe: Timeframe) -> Self {
        Self {
            series,
            basis: Basis::Time(timeframe),
            zoom: Zoom::points(DEFAULT_ZOOM_POINTS),
            pan: 0.0,
            panel_kinds: &DEFAULT_PANEL_KINDS,
            panel_splits: &DEFAULT_PANEL_SPLITS,
            panel_titles: &[],
            panel_marks: &DEFAULT_PANEL_MARKS,
            panel_scale_modes: &DEFAULT_PANEL_SCALE_MODES,
            timezone: UserTimezone::Utc,
            version: 0,
        }
    }

    pub fn with_zoom(mut self, zoom: Zoom) -> Self {
        self.zoom = zoom;
        self
    }

    pub fn with_pan(mut self, pan: f32) -> Self {
        self.pan = pan;
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

    pub fn with_panel_titles(mut self, panel_titles: &'a [String]) -> Self {
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

    fn default_title_for_panel(kind: KlinePanelKind) -> &'static str {
        match kind {
            KlinePanelKind::PrimaryChart => "Price",
            KlinePanelKind::Indicator => "Indicator",
        }
    }

    fn resolved_panel_title(&self, panel_index: usize, panel_kind: KlinePanelKind) -> &str {
        self.panel_titles
            .get(panel_index)
            .map(String::as_str)
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| Self::default_title_for_panel(panel_kind))
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

    fn panel_control_kinds(
        &self,
        panel_index: usize,
        panel_count: usize,
        primary_panel: usize,
    ) -> Vec<PanelControlKind> {
        let mut controls = Vec::with_capacity(4);
        let is_primary = panel_index == primary_panel;

        if panel_index > 0 {
            controls.push(PanelControlKind::MoveUp);
        }

        if panel_index + 1 < panel_count {
            controls.push(PanelControlKind::MoveDown);
        }

        controls.push(PanelControlKind::Settings);

        if !is_primary {
            controls.push(PanelControlKind::Close);
        }

        controls
    }

    fn build_panel_control_hits(
        &self,
        layout: &PanelLayoutTree,
        primary_panel: usize,
    ) -> Vec<PanelControlHit> {
        let panel_count = layout.panels.len();
        let mut hits = Vec::with_capacity(panel_count.saturating_mul(4));

        for (panel_index, panel) in layout.panels.iter().enumerate() {
            let title = self.resolved_panel_title(panel_index, panel.kind);
            let title_w = title.chars().count() as f32 * CHAR_W;
            let min_x = panel.plot.x + PANEL_TITLE_LEFT_PAD + title_w + PANEL_TITLE_TO_CONTROLS_GAP;

            let mut controls = self.panel_control_kinds(panel_index, panel_count, primary_panel);
            while !controls.is_empty() {
                let count = controls.len() as f32;
                let total_w =
                    count * PANEL_CONTROL_BOX + (count - 1.0).max(0.0) * PANEL_CONTROL_GAP;
                let x = panel.plot.x + panel.plot.width - PANEL_TITLE_LEFT_PAD - total_w;
                let y = panel.plot.y + PANEL_TITLE_TOP_PAD - 1.0;

                if x < min_x {
                    controls.remove(0);
                    continue;
                }

                let mut x_cursor = x;
                for kind in controls.into_iter() {
                    hits.push(PanelControlHit {
                        panel_index,
                        kind,
                        rect: Rectangle {
                            x: x_cursor,
                            y,
                            width: PANEL_CONTROL_BOX,
                            height: PANEL_CONTROL_BOX,
                        },
                    });

                    x_cursor += PANEL_CONTROL_BOX + PANEL_CONTROL_GAP;
                }
                break;
            }
        }

        hits
    }

    fn hit_panel_control(
        layout: &PanelLayoutTree,
        controls: &[PanelControlHit],
        root_local: Point,
    ) -> Option<PanelControlHit> {
        let plot_local = layout.plot_local_point(root_local)?;

        controls
            .iter()
            .copied()
            .find(|control| PanelLayoutTree::contains(control.rect, plot_local))
    }

    fn control_visibility_panel(
        &self,
        layout: &PanelLayoutTree,
        root_local: Point,
    ) -> Option<usize> {
        match layout.hit_test(root_local) {
            LayoutHitZone::PanelPlot(panel_index) | LayoutHitZone::PanelXAxis(panel_index) => {
                Some(panel_index)
            }
            _ => None,
        }
    }

    fn build_ticker_legend_layout(
        &self,
        layout: &PanelLayoutTree,
        primary_panel: usize,
    ) -> Option<TickerLegendLayout> {
        if self.series.is_empty() {
            return None;
        }

        let panel = layout.panel(primary_panel)?;
        let plot = panel.plot;

        let rows_count = self.series.len();
        let max_name_chars = self
            .series
            .iter()
            .map(|series| {
                series
                    .ticker_info()
                    .ticker
                    .symbol_and_exchange_string()
                    .len()
            })
            .max()
            .unwrap_or(0);

        let text_w = max_name_chars as f32 * CHAR_W;
        let icon_pack_w = (2.0 * TICKER_LEGEND_ICON_BOX) + TICKER_LEGEND_ICON_GAP;

        let bg_w = (text_w + icon_pack_w + TICKER_LEGEND_PADDING * 2.0 + 10.0)
            .clamp(120.0, (plot.width * 0.6).max(120.0));

        let max_bg_h = ((rows_count as f32) * TICKER_LEGEND_ROW_H + TICKER_LEGEND_PADDING * 2.0)
            .min(plot.height * 0.5)
            .max(TICKER_LEGEND_ROW_H + TICKER_LEGEND_PADDING * 2.0);
        let max_rows_fit =
            (((max_bg_h - TICKER_LEGEND_PADDING * 2.0) / TICKER_LEGEND_ROW_H).floor() as usize)
                .max(1);
        let visible_rows = rows_count.min(max_rows_fit);
        let bg_h = visible_rows as f32 * TICKER_LEGEND_ROW_H + TICKER_LEGEND_PADDING * 2.0;

        let bg = Rectangle {
            x: plot.x + PANEL_TITLE_LEFT_PAD,
            y: plot.y + PANEL_TITLE_TOP_PAD + TICKER_LEGEND_TOP_OFFSET,
            width: bg_w,
            height: bg_h,
        };

        let x_right = bg.x + bg.width - TICKER_LEGEND_PADDING;

        let mut rows = Vec::with_capacity(visible_rows);
        let mut row_top = bg.y + TICKER_LEGEND_PADDING;

        for (index, series) in self.series.iter().take(visible_rows).enumerate() {
            let has_close = index != 0;
            let y_center = row_top + TICKER_LEGEND_ROW_H * 0.5;
            let close = Rectangle {
                x: x_right - TICKER_LEGEND_ICON_BOX,
                y: y_center - TICKER_LEGEND_ICON_BOX * 0.5,
                width: TICKER_LEGEND_ICON_BOX,
                height: TICKER_LEGEND_ICON_BOX,
            };
            let settings = Rectangle {
                x: if has_close {
                    close.x - TICKER_LEGEND_ICON_GAP - TICKER_LEGEND_ICON_BOX
                } else {
                    close.x
                },
                y: close.y,
                width: TICKER_LEGEND_ICON_BOX,
                height: TICKER_LEGEND_ICON_BOX,
            };

            rows.push(TickerLegendRowHit {
                ticker: *series.ticker_info(),
                y_center,
                row_rect: Rectangle {
                    x: bg.x,
                    y: row_top,
                    width: bg.width,
                    height: TICKER_LEGEND_ROW_H,
                },
                settings,
                close,
                has_close,
            });

            row_top += TICKER_LEGEND_ROW_H;
        }

        Some(TickerLegendLayout { bg, rows })
    }

    fn hit_ticker_legend(
        layout: &PanelLayoutTree,
        legend: &TickerLegendLayout,
        root_local: Point,
    ) -> Option<TickerLegendHit> {
        let plot_local = layout.plot_local_point(root_local)?;
        if !legend.bg.contains(plot_local) {
            return None;
        }

        for (index, row) in legend.rows.iter().enumerate() {
            if !row.row_rect.contains(plot_local) {
                continue;
            }

            if row.settings.contains(plot_local) {
                return Some(TickerLegendHit::Icon(index, TickerLegendIconKind::Settings));
            }

            if row.has_close && row.close.contains(plot_local) {
                return Some(TickerLegendHit::Icon(index, TickerLegendIconKind::Close));
            }

            return Some(TickerLegendHit::Row(index));
        }

        Some(TickerLegendHit::Background)
    }

    fn bar_at_or_before_unit<'b>(
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

    fn panel_min_ratio(&self, panel_count: usize, usable_plot_height: f32) -> f32 {
        if panel_count <= 1 {
            return 0.0;
        }

        let usable = usable_plot_height.max(1.0);
        let geometric_min = MIN_PANEL_HEIGHT / usable;
        let feasible_cap = 1.0 / panel_count as f32;

        geometric_min.min(feasible_cap)
    }

    fn normalized_panel_splits(&self, panel_count: usize, usable_plot_height: f32) -> Vec<f32> {
        let split_count = panel_count.saturating_sub(1);
        if split_count == 0 {
            return Vec::new();
        }

        let mut splits = Vec::with_capacity(split_count);
        for index in 0..split_count {
            let fallback = (index + 1) as f32 / panel_count as f32;
            splits.push(self.panel_splits.get(index).copied().unwrap_or(fallback));
        }

        let min_ratio = self.panel_min_ratio(panel_count, usable_plot_height);

        for index in 0..split_count {
            let remaining_panels_after = panel_count.saturating_sub(index + 1);

            let lower = if index > 0 {
                splits[index - 1] + min_ratio
            } else {
                min_ratio
            };

            let upper = 1.0 - (remaining_panels_after as f32 * min_ratio);
            let (min_bound, max_bound) = if lower <= upper {
                (lower, upper)
            } else {
                (upper, lower)
            };

            splits[index] = splits[index].clamp(min_bound, max_bound);
        }

        splits
    }

    fn panel_plot_heights(&self, panel_stack_height: f32, panel_count: usize) -> Vec<f32> {
        if panel_count == 0 {
            return Vec::new();
        }

        let non_plot = (panel_count as f32 * PANEL_X_AXIS_HEIGHT)
            + (panel_count.saturating_sub(1) as f32 * PANEL_SPLITTER_HEIGHT);
        let usable = (panel_stack_height - non_plot).max(0.0);

        if panel_count == 1 {
            return vec![usable];
        }

        let splits = self.normalized_panel_splits(panel_count, usable.max(1.0));
        let mut heights = Vec::with_capacity(panel_count);
        let mut previous = 0.0;

        for split in splits {
            let boundary = split.clamp(0.0, 1.0) * usable;
            heights.push((boundary - previous).max(0.0));
            previous = boundary;
        }

        heights.push((usable - previous).max(0.0));
        heights
    }

    fn split_ratio_from_cursor(
        &self,
        cursor_y: f32,
        layout: &PanelLayoutTree,
        split_index: usize,
    ) -> Option<f32> {
        let panel_count = layout.panels.len();
        let split_count = panel_count.saturating_sub(1);

        if split_count == 0 || split_index >= split_count {
            return None;
        }

        let local_y = (cursor_y - layout.regions.plot.y).clamp(0.0, layout.regions.plot.height);
        let usable_plot_height: f32 = layout.panels.iter().map(|panel| panel.plot.height).sum();
        let usable = usable_plot_height.max(1.0);

        let fixed_before = ((split_index + 1) as f32 * PANEL_X_AXIS_HEIGHT)
            + (split_index as f32 * PANEL_SPLITTER_HEIGHT)
            + (PANEL_SPLITTER_HEIGHT * 0.5);
        let boundary = (local_y - fixed_before).clamp(0.0, usable);
        let ratio = (boundary / usable).clamp(0.0, 1.0);

        let splits = self.normalized_panel_splits(panel_count, usable);
        let min_ratio = self.panel_min_ratio(panel_count, usable);

        let lower = if split_index > 0 {
            splits[split_index - 1] + min_ratio
        } else {
            min_ratio
        };

        let upper = if split_index + 1 < splits.len() {
            splits[split_index + 1] - min_ratio
        } else {
            1.0 - min_ratio
        };

        let (min_bound, max_bound) = if lower <= upper {
            (lower, upper)
        } else {
            (upper, lower)
        };

        Some(ratio.clamp(min_bound, max_bound))
    }

    fn resolve_x_axis(&self) -> Option<XAxis> {
        match self.basis {
            Basis::Time(timeframe) => {
                let anchor = self
                    .series
                    .iter()
                    .flat_map(|s| s.bars().iter())
                    .map(|bar| bar.time.floor_to(timeframe))
                    .max()?;

                Some(XAxis::Time { timeframe, anchor })
            }
            Basis::Tick(_) => {
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

    fn normalize_zoom(&self, z: Zoom) -> Zoom {
        if z.is_all() {
            return Zoom::all();
        }

        Zoom::points(z.0.clamp(MIN_ZOOM_POINTS, MAX_ZOOM_POINTS))
    }

    fn max_points_available(&self) -> usize {
        self.series
            .iter()
            .map(|s| s.bars().len())
            .max()
            .unwrap_or_default()
    }

    fn step_zoom_percent(&self, current: Zoom, zoom_in: bool) -> Zoom {
        let len = self.max_points_available().max(MIN_ZOOM_POINTS);
        let base_n = if current.is_all() {
            len
        } else {
            current.0.clamp(MIN_ZOOM_POINTS, MAX_ZOOM_POINTS)
        };

        let step = ((base_n as f32) * ZOOM_STEP_PCT).ceil().max(1.0) as usize;

        let new_n = if zoom_in {
            base_n.saturating_sub(step).max(MIN_ZOOM_POINTS)
        } else {
            base_n.saturating_add(step).min(MAX_ZOOM_POINTS)
        };

        Zoom::points(new_n)
    }

    fn for_each_bar_unit_index(&self, x_axis: XAxis, mut f: impl FnMut(usize, &S, i64, &Kline)) {
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

    fn current_x_span_units(&self) -> f32 {
        let Some(x_axis) = self.resolve_x_axis() else {
            return 1.0;
        };
        let Some((data_min_x, data_max_x)) = self.data_x_bounds(x_axis) else {
            return 1.0;
        };

        if self.zoom.is_all() {
            (data_max_x - data_min_x).max(1) as f32
        } else {
            self.zoom
                .0
                .clamp(MIN_ZOOM_POINTS, MAX_ZOOM_POINTS)
                .saturating_sub(1) as f32
        }
    }

    fn compute_x_window(&self) -> Option<(XAxis, i64, i64)> {
        let x_axis = self.resolve_x_axis()?;
        let (data_min_x, mut data_max_x) = self.data_x_bounds(x_axis)?;

        if data_max_x == data_min_x {
            data_max_x = data_max_x.saturating_add(1);
        }

        let span = if self.zoom.is_all() {
            (data_max_x - data_min_x).max(1)
        } else {
            self.zoom
                .0
                .clamp(MIN_ZOOM_POINTS, MAX_ZOOM_POINTS)
                .saturating_sub(1) as i64
        };

        let pan_units = self.pan.round() as i64;
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
        let mut indicator_max_value = 0.0f32;

        self.for_each_bar_unit_index(x_axis, |series_index, series, unit, bar| {
            if series_index != 0 {
                return;
            }

            if unit < min_x_unit || unit > max_x_unit {
                return;
            }

            any = true;
            let value = series.indicator_value_for_panel(indicator_panel_index, bar);
            min_value = min_value.min(value);
            indicator_max_value = indicator_max_value.max(value);
        });

        if !any {
            return None;
        }

        match scale_mode {
            PanelScaleMode::FitVisible => {
                let span = (indicator_max_value - min_value).abs();
                let pad = if span <= f32::EPSILON {
                    indicator_max_value.abs().max(1.0) * 0.02
                } else {
                    span * 0.05
                };

                Some((min_value - pad, indicator_max_value + pad))
            }
            PanelScaleMode::FitVisibleIncludeZero => {
                let min_including_zero = min_value.min(0.0);
                let max_including_zero = indicator_max_value.max(0.0);
                let span = (max_including_zero - min_including_zero).abs();
                let pad = if span <= f32::EPSILON {
                    max_including_zero.abs().max(1.0) * 0.02
                } else {
                    span * 0.05
                };

                Some((min_including_zero - pad, max_including_zero + pad))
            }
            _ => Some((0.0, indicator_max_value.max(1.0))),
        }
    }

    fn compute_scene(&self, layout: Layout<'_>, cursor: mouse::Cursor) -> Option<Scene> {
        let panel_kinds = self.resolved_panel_kinds();
        let panel_layout = PanelLayoutTree::from_layout(layout, panel_kinds)?;
        let primary_panel = panel_layout
            .panels
            .iter()
            .position(|panel| panel.kind == KlinePanelKind::PrimaryChart)?;
        let primary_mark = self.resolved_panel_mark(primary_panel, KlinePanelKind::PrimaryChart);
        let primary_scale_mode = self.resolved_panel_scale_mode(primary_panel);

        let (x_axis, min_x_unit, max_x_unit) = self.compute_x_window()?;

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

        let primary_plot = panel_layout.panel(primary_panel)?.plot;

        let panel_controls = self.build_panel_control_hits(&panel_layout, primary_panel);
        let ticker_legend = self.build_ticker_legend_layout(&panel_layout, primary_panel);

        let mut scene = Scene {
            layout: panel_layout,
            x_axis,
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

        let cursor_root_local = cursor.position_in(layout.bounds());

        if let Some(local) = cursor_root_local {
            scene.hovered_control =
                Self::hit_panel_control(&scene.layout, &scene.panel_controls, local);

            if let Some(legend) = scene.ticker_legend.as_ref()
                && let Some(hit) = Self::hit_ticker_legend(&scene.layout, legend, local)
            {
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

                let span = (max_x_unit - min_x_unit).max(1) as f32;
                let ratio = (x_plot / primary_plot.width.max(1.0)).clamp(0.0, 1.0);
                let raw_x_unit = min_x_unit.saturating_add((ratio * span).round() as i64);
                let snapped_x_unit = raw_x_unit.clamp(min_x_unit, max_x_unit);
                let snapped_x_plot = (((snapped_x_unit - min_x_unit) as f32 / span)
                    .clamp(0.0, 1.0))
                    * primary_plot.width;

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

    fn format_x_label(&self, x_axis: XAxis, unit: i64, step_units: i64) -> String {
        match x_axis {
            XAxis::Time { .. } => x_axis.time_from_unit(unit).map_or_else(
                || unit.to_string(),
                |ts| {
                    super::format_time_label(ts.as_u64(), x_axis.step_ms(step_units), self.timezone)
                },
            ),
            XAxis::Tick { .. } => x_axis
                .tick_from_unit(unit)
                .map_or_else(|| unit.to_string(), |index| index.as_u64().to_string()),
        }
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
        let px_per_unit = scene.primary_plot().width / scene.span_units().max(1.0);
        let candle_width = (px_per_unit * 0.7).clamp(1.0, 22.0);
        let indicator_width = (px_per_unit * 0.8).clamp(1.0, 24.0);

        let mut primary_line_points: Vec<Vec<Point>> = vec![Vec::new(); self.series.len()];
        let mut indicator_line_points: Vec<Vec<Point>> =
            vec![Vec::new(); scene.indicator_panels.len()];

        self.for_each_bar_unit_index(scene.x_axis, |series_index, series, x_unit, bar| {
            if x_unit < scene.min_x_unit || x_unit > scene.max_x_unit {
                return;
            }

            let x = scene.map_x_plot(x_unit);
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
                        points.push(Point::new(x, y_close));
                    }
                }
                MarkKind::Candle | MarkKind::Bar => {
                    let body_top = y_open.min(y_close);
                    let body_h = (y_open - y_close).abs().max(1.0);

                    frame.fill_rectangle(
                        Point::new(x - (candle_width / 2.0), body_top),
                        Size::new(candle_width, body_h),
                        color,
                    );

                    let wick_w = (candle_width * 0.16).clamp(1.0, 2.0);
                    frame.fill_rectangle(
                        Point::new(x - (wick_w / 2.0), y_high.min(y_low)),
                        Size::new(wick_w, (y_high - y_low).abs().max(1.0)),
                        color.scale_alpha(0.85),
                    );
                }
            }

            if !is_base_series {
                return;
            }

            for (indicator_slot, indicator_panel) in scene.indicator_panels.iter().enumerate() {
                let indicator_value =
                    series.indicator_value_for_panel(indicator_panel.panel_index, bar);

                if let (Some(y_indicator_value), Some(y_indicator_bottom)) = (
                    scene.map_indicator_plot(indicator_panel.panel_index, indicator_value),
                    scene.indicator_panel_bottom(indicator_panel.panel_index),
                ) {
                    match indicator_panel.mark {
                        MarkKind::Line => {
                            if let Some(points) = indicator_line_points.get_mut(indicator_slot) {
                                points.push(Point::new(x, y_indicator_value));
                            }
                        }
                        MarkKind::Candle | MarkKind::Bar => {
                            frame.fill_rectangle(
                                Point::new(
                                    x - (indicator_width / 2.0),
                                    y_indicator_value.min(y_indicator_bottom),
                                ),
                                Size::new(
                                    indicator_width,
                                    (y_indicator_bottom - y_indicator_value).abs().max(1.0),
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
            let title = self.resolved_panel_title(panel_index, panel.kind);
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

    fn fill_panel_header_values(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
    ) {
        let Some(cursor) = scene.cursor else {
            return;
        };

        let Some(base_series) = self.series.first() else {
            return;
        };

        let Some(base_bar) = self.bar_at_or_before_unit(base_series, scene.x_axis, cursor.x_unit)
        else {
            return;
        };

        for (panel_index, panel) in scene.layout.panels.iter().enumerate() {
            let title = self.resolved_panel_title(panel_index, panel.kind);
            let title_w = title.chars().count() as f32 * CHAR_W;

            let mut x =
                scene.layout.regions.plot.x + panel.plot.x + PANEL_TITLE_LEFT_PAD + title_w + 8.0;
            let y = scene.layout.regions.plot.y + panel.plot.y + PANEL_TITLE_TOP_PAD;

            let mut max_x = scene.layout.regions.plot.x + panel.plot.x + panel.plot.width
                - PANEL_TITLE_LEFT_PAD;
            if scene.controls_visible_for_panel == Some(panel_index)
                && let Some(left_control_x) = scene
                    .panel_controls
                    .iter()
                    .filter(|control| control.panel_index == panel_index)
                    .map(|control| control.rect.x)
                    .reduce(f32::min)
            {
                max_x = max_x.min(scene.layout.regions.plot.x + left_control_x - 4.0);
            }

            if x >= max_x {
                continue;
            }

            if panel_index == scene.primary_panel {
                let precision = base_series.ticker_info().min_ticksize;

                if matches!(scene.primary_mark, MarkKind::Candle | MarkKind::Bar) {
                    let open_f = base_bar.open.to_f32();
                    let close_f = base_bar.close.to_f32();
                    let change_pct = if open_f.abs() > f32::EPSILON {
                        ((close_f - open_f) / open_f) * 100.0
                    } else {
                        0.0
                    };

                    let value_color = if change_pct >= 0.0 {
                        palette.success.base.color
                    } else {
                        palette.danger.base.color
                    };
                    let label_color = palette.background.base.text.scale_alpha(0.82);

                    let segments: Vec<(String, iced::Color, bool)> = vec![
                        ("O".to_string(), label_color, false),
                        (base_bar.open.to_string(precision), value_color, true),
                        ("H".to_string(), label_color, false),
                        (base_bar.high.to_string(precision), value_color, true),
                        ("L".to_string(), label_color, false),
                        (base_bar.low.to_string(precision), value_color, true),
                        ("C".to_string(), label_color, false),
                        (base_bar.close.to_string(precision), value_color, true),
                        (format!("{change_pct:+.2}%"), value_color, true),
                    ];

                    for (text, color, is_value) in segments {
                        if x >= max_x {
                            break;
                        }

                        frame.fill_text(canvas::Text {
                            content: text.clone(),
                            position: Point::new(x, y),
                            color,
                            size: (TEXT_SIZE - 1.0).into(),
                            align_x: iced::Alignment::Start.into(),
                            align_y: iced::Alignment::Start.into(),
                            font: style::AZERET_MONO,
                            ..Default::default()
                        });

                        x += text.chars().count() as f32 * CHAR_W;
                        x += if is_value { 6.0 } else { 2.0 };
                    }
                } else {
                    let text = format!("C {}", base_bar.close.to_string(precision));
                    frame.fill_text(canvas::Text {
                        content: text,
                        position: Point::new(x, y),
                        color: palette.background.base.text.scale_alpha(0.85),
                        size: (TEXT_SIZE - 1.0).into(),
                        align_x: iced::Alignment::Start.into(),
                        align_y: iced::Alignment::Start.into(),
                        font: style::AZERET_MONO,
                        ..Default::default()
                    });
                }
            } else {
                let value = base_series.indicator_value_for_panel(panel_index, base_bar);
                let text = super::format_value(value, 0.01);

                frame.fill_text(canvas::Text {
                    content: text,
                    position: Point::new(x, y),
                    color: palette.background.base.text.scale_alpha(0.82),
                    size: (TEXT_SIZE - 1.0).into(),
                    align_x: iced::Alignment::Start.into(),
                    align_y: iced::Alignment::Start.into(),
                    font: style::AZERET_MONO,
                    ..Default::default()
                });
            }
        }
    }

    fn draw_legend_icon_button(
        frame: &mut canvas::Frame,
        rect: Rectangle,
        icon: &str,
        hovered: bool,
        danger: bool,
        palette: &Extended,
    ) {
        let fill = if hovered && danger {
            palette.danger.base.color.scale_alpha(0.22)
        } else if hovered {
            palette.background.strong.color
        } else {
            palette.background.base.color.scale_alpha(0.72)
        };

        let text = if hovered && danger {
            palette.danger.base.text
        } else if hovered {
            palette.background.strong.text
        } else {
            palette.background.base.text.scale_alpha(0.86)
        };

        frame.fill_rectangle(rect.position(), rect.size(), fill);
        frame.fill_text(canvas::Text {
            content: icon.to_string(),
            position: Point::new(rect.x + rect.width * 0.5, rect.y + rect.height * 0.5),
            color: text,
            size: PANEL_CONTROL_ICON_SIZE.into(),
            align_x: iced::Alignment::Center.into(),
            align_y: iced::Alignment::Center.into(),
            font: style::AZERET_MONO,
            ..Default::default()
        });
    }

    fn fill_primary_ticker_legend(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
    ) {
        let Some(legend) = scene.ticker_legend.as_ref() else {
            return;
        };

        let to_root = |rect: Rectangle| Rectangle {
            x: scene.layout.regions.plot.x + rect.x,
            y: scene.layout.regions.plot.y + rect.y,
            width: rect.width,
            height: rect.height,
        };

        let bg = to_root(legend.bg);
        frame.fill_rectangle(
            bg.position(),
            bg.size(),
            palette.background.weak.color.scale_alpha(0.82),
        );

        for (index, row) in legend.rows.iter().enumerate() {
            let row_rect = to_root(row.row_rect);
            let hovered_row = scene.hovered_ticker_row == Some(index);

            if hovered_row {
                frame.fill_rectangle(
                    row_rect.position(),
                    row_rect.size(),
                    palette.background.strong.color.scale_alpha(0.2),
                );
            }

            let label = row.ticker.ticker.symbol_and_exchange_string();
            let label_color = if index == 0 {
                palette.background.base.text
            } else {
                Self::comparison_line_color(&row.ticker).scale_alpha(0.96)
            };

            frame.fill_text(canvas::Text {
                content: label,
                position: Point::new(
                    bg.x + TICKER_LEGEND_PADDING,
                    scene.layout.regions.plot.y + row.y_center,
                ),
                color: label_color,
                size: (TEXT_SIZE - 1.0).into(),
                align_x: iced::Alignment::Start.into(),
                align_y: iced::Alignment::Center.into(),
                font: style::AZERET_MONO,
                ..Default::default()
            });

            if hovered_row {
                let settings_root = to_root(row.settings);
                let close_root = to_root(row.close);

                let settings_hovered =
                    scene.hovered_ticker_icon == Some((index, TickerLegendIconKind::Settings));
                Self::draw_legend_icon_button(
                    frame,
                    settings_root,
                    TickerLegendIconKind::Settings.icon(),
                    settings_hovered,
                    false,
                    palette,
                );

                if row.has_close {
                    let close_hovered =
                        scene.hovered_ticker_icon == Some((index, TickerLegendIconKind::Close));
                    Self::draw_legend_icon_button(
                        frame,
                        close_root,
                        TickerLegendIconKind::Close.icon(),
                        close_hovered,
                        true,
                        palette,
                    );
                }
            }
        }

        if let Some((row_index, icon_kind)) = scene.hovered_ticker_icon
            && let Some(row) = legend.rows.get(row_index)
        {
            let target = match icon_kind {
                TickerLegendIconKind::Settings => to_root(row.settings),
                TickerLegendIconKind::Close => to_root(row.close),
            };

            let label = icon_kind.tooltip();
            let label_w = (label.chars().count() as f32 * CHAR_W + 10.0).clamp(90.0, 180.0);
            let label_h = TEXT_SIZE + 4.0;

            let tooltip_x = (target.x + target.width * 0.5 - label_w * 0.5).clamp(
                scene.layout.regions.plot.x,
                scene.layout.regions.plot.x + scene.layout.regions.plot.width - label_w,
            );
            let tooltip_y =
                (target.y - label_h - PANEL_CONTROL_TOOLTIP_GAP).max(scene.layout.regions.plot.y);

            frame.fill_rectangle(
                Point::new(tooltip_x, tooltip_y),
                Size::new(label_w, label_h),
                palette.background.strong.color,
            );

            frame.fill_text(canvas::Text {
                content: label.to_string(),
                position: Point::new(tooltip_x + label_w * 0.5, tooltip_y + label_h * 0.5),
                color: palette.background.strong.text,
                size: (TEXT_SIZE - 1.0).into(),
                align_x: iced::Alignment::Center.into(),
                align_y: iced::Alignment::Center.into(),
                font: style::AZERET_MONO,
                ..Default::default()
            });
        }
    }

    fn fill_panel_controls(&self, frame: &mut canvas::Frame, scene: &Scene, palette: &Extended) {
        let Some(visible_panel) = scene.controls_visible_for_panel else {
            return;
        };

        for control in scene
            .panel_controls
            .iter()
            .copied()
            .filter(|control| control.panel_index == visible_panel)
        {
            let hovered = scene
                .hovered_control
                .map(|hit| hit.panel_index == control.panel_index && hit.kind == control.kind)
                .unwrap_or(false);

            let is_close = matches!(control.kind, PanelControlKind::Close);

            let fill_color = if hovered && is_close {
                palette.danger.base.color.scale_alpha(0.22)
            } else if hovered {
                palette.background.strong.color
            } else {
                palette.background.base.color.scale_alpha(0.72)
            };

            let text_color = if hovered && is_close {
                palette.danger.base.text
            } else if hovered {
                palette.background.strong.text
            } else {
                palette.background.base.text.scale_alpha(0.86)
            };

            let x = scene.layout.regions.plot.x + control.rect.x;
            let y = scene.layout.regions.plot.y + control.rect.y;

            frame.fill_rectangle(
                Point::new(x, y),
                Size::new(control.rect.width, control.rect.height),
                fill_color,
            );

            frame.fill_text(canvas::Text {
                content: control.kind.icon().to_string(),
                position: Point::new(x + control.rect.width / 2.0, y + control.rect.height / 2.0),
                color: text_color,
                size: PANEL_CONTROL_ICON_SIZE.into(),
                align_x: iced::Alignment::Center.into(),
                align_y: iced::Alignment::Center.into(),
                font: style::AZERET_MONO,
                ..Default::default()
            });
        }

        if let Some(hovered) = scene.hovered_control {
            let label = hovered.kind.tooltip();
            let label_w = (label.chars().count() as f32 * CHAR_W + 10.0).clamp(68.0, 170.0);
            let label_h = TEXT_SIZE + 4.0;

            let hovered_x = scene.layout.regions.plot.x + hovered.rect.x;
            let hovered_y = scene.layout.regions.plot.y + hovered.rect.y;

            let tooltip_x = (hovered_x + hovered.rect.width * 0.5 - label_w * 0.5).clamp(
                scene.layout.regions.plot.x,
                scene.layout.regions.plot.x + scene.layout.regions.plot.width - label_w,
            );

            let tooltip_y =
                (hovered_y - label_h - PANEL_CONTROL_TOOLTIP_GAP).max(scene.layout.regions.plot.y);

            frame.fill_rectangle(
                Point::new(tooltip_x, tooltip_y),
                Size::new(label_w, label_h),
                palette.background.strong.color,
            );

            frame.fill_text(canvas::Text {
                content: label.to_string(),
                position: Point::new(tooltip_x + label_w * 0.5, tooltip_y + label_h * 0.5),
                color: palette.background.strong.text,
                size: (TEXT_SIZE - 1.0).into(),
                align_x: iced::Alignment::Center.into(),
                align_y: iced::Alignment::Center.into(),
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
        self.fill_panel_header_values(frame, scene, palette);

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
        limits: &layout::Limits,
    ) -> layout::Node {
        let panel_count = self.resolved_panel_kinds().len().max(1);

        let build_panel_stack = |stack_size: Size| {
            let plot_heights = self.panel_plot_heights(stack_size.height, panel_count);
            let mut children = Vec::with_capacity(panel_count.saturating_mul(3).saturating_sub(1));

            let mut y = 0.0;

            for panel_index in 0..panel_count {
                let plot_h = plot_heights.get(panel_index).copied().unwrap_or_default();

                children.push(
                    layout::Node::new(Size::new(stack_size.width, plot_h))
                        .move_to(Point::new(0.0, y)),
                );
                y += plot_h;

                let axis_h = if panel_index + 1 == panel_count {
                    (stack_size.height - y).max(0.0)
                } else {
                    PANEL_X_AXIS_HEIGHT
                };

                children.push(
                    layout::Node::new(Size::new(stack_size.width, axis_h))
                        .move_to(Point::new(0.0, y)),
                );
                y += axis_h;

                if panel_index + 1 < panel_count {
                    children.push(
                        layout::Node::new(Size::new(stack_size.width, PANEL_SPLITTER_HEIGHT))
                            .move_to(Point::new(0.0, y)),
                    );
                    y += PANEL_SPLITTER_HEIGHT;
                }
            }

            layout::Node::with_children(stack_size, children)
        };

        let row_node = layout::next_to_each_other(
            &limits.shrink(Size::new(0.0, X_AXIS_HEIGHT)),
            0.0,
            |l| {
                let stack_node = layout::atomic(
                    &l.shrink(Size::new(Y_AXIS_GUTTER, 0.0)),
                    Length::Fill,
                    Length::Fill,
                );

                build_panel_stack(stack_node.size())
            },
            |l| layout::atomic(l, Y_AXIS_GUTTER, Length::Fill),
        );

        let x_axis_node = layout::next_to_each_other(
            limits,
            0.0,
            |l| {
                layout::atomic(
                    &l.shrink(Size::new(Y_AXIS_GUTTER, 0.0)),
                    Length::Fill,
                    X_AXIS_HEIGHT,
                )
            },
            |l| layout::atomic(l, Y_AXIS_GUTTER, X_AXIS_HEIGHT),
        );

        let row_h = row_node.size().height;
        let total_w = row_node.size().width;
        let total_h = row_h + X_AXIS_HEIGHT;

        layout::Node::with_children(
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
                    state.overlay_cache.clear();
                    return;
                };

                let zone = layout_tree.hit_test(cursor_pos);
                let primary_panel = layout_tree
                    .panels
                    .iter()
                    .position(|panel| panel.kind == KlinePanelKind::PrimaryChart)
                    .unwrap_or(0);
                let panel_controls = self.build_panel_control_hits(&layout_tree, primary_panel);
                let ticker_legend = self.build_ticker_legend_layout(&layout_tree, primary_panel);
                let ticker_legend_hit = ticker_legend
                    .as_ref()
                    .and_then(|legend| Self::hit_ticker_legend(&layout_tree, legend, cursor_pos));

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
                        let new_zoom = self.step_zoom_percent(self.zoom, zoom_in);

                        if new_zoom != self.zoom {
                            shell.publish(M::from(KlineWidgetEvent::ZoomChanged(
                                self.normalize_zoom(new_zoom),
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
                        state.overlay_cache.clear();

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
                                let x_span = self.current_x_span_units();
                                let plot_w = layout_tree.primary_plot_width().max(1.0);
                                let dx_pts = -(dx_px) * (x_span / plot_w);

                                shell.publish(M::from(KlineWidgetEvent::PanChanged(
                                    self.pan + dx_pts,
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

            r.with_layer(
                Rectangle {
                    x: 0.0,
                    y: 0.0,
                    width: bounds.width,
                    height: bounds.height,
                },
                |r| {
                    use iced::advanced::graphics::geometry::Renderer as _;
                    r.draw_geometry(overlay_geom);
                },
            );
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
        let panel_controls = self.build_panel_control_hits(&layout_tree, primary_panel);
        let ticker_legend = self.build_ticker_legend_layout(&layout_tree, primary_panel);
        let ticker_legend_hit = ticker_legend
            .as_ref()
            .and_then(|legend| Self::hit_ticker_legend(&layout_tree, legend, cursor_local));

        if ticker_legend_hit.is_some() {
            return advanced::mouse::Interaction::Pointer;
        }

        if Self::hit_panel_control(&layout_tree, &panel_controls, cursor_local).is_some() {
            return advanced::mouse::Interaction::Pointer;
        }

        match layout_tree.hit_test(cursor_local) {
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
