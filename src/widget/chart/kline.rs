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
const ZOOM_STEP_PCT: f32 = 0.05;
const PANEL_X_AXIS_HEIGHT: f32 = 18.0;
const PANEL_SPLITTER_HEIGHT: f32 = 1.0;
const PANEL_SPLITTER_HIT_PX: f32 = 8.0;
const MIN_PANEL_HEIGHT: f32 = 40.0;

const DEFAULT_PANEL_KINDS: [KlinePanelKind; 2] = [KlinePanelKind::Value, KlinePanelKind::Histogram];
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
    fn secondary_value(&self, bar: &Kline) -> f32;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KlinePanelKind {
    Value,
    Histogram,
}

#[derive(Debug, Clone)]
pub enum KlineWidgetEvent {
    ZoomChanged(Zoom),
    PanChanged(f32),
    PanelSplitChanged { index: usize, split: f32 },
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
    y_secondary_value: Option<f32>,
    x_plot: f32,
    y_plot: f32,
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
    secondary_panel: Option<usize>,
    secondary_mark: Option<MarkKind>,
    secondary_max_value: Option<f32>,
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

    fn secondary_plot(&self) -> Option<&Rectangle> {
        self.secondary_panel
            .and_then(|index| self.layout.panel(index).map(|panel| &panel.plot))
    }

    fn secondary_panel_bottom(&self) -> Option<f32> {
        self.secondary_plot().map(|rect| rect.y + rect.height)
    }

    fn map_x_plot(&self, x_unit: i64) -> f32 {
        let span = self.span_units();
        let ratio = ((x_unit - self.min_x_unit) as f32 / span).clamp(0.0, 1.0);
        ratio * self.primary_plot().width
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

    fn map_primary_plot(&self, value: f32) -> f32 {
        let (min_display, max_display) = self.primary_domain_display_values();
        let range = (max_display - min_display).abs().max(1e-6);
        let ratio = ((self.primary_to_display_value(value) - min_display) / range).clamp(0.0, 1.0);
        let panel = self.primary_plot();
        panel.y + (1.0 - ratio) * panel.height
    }

    fn map_secondary_plot(&self, secondary_value: f32) -> Option<f32> {
        let panel = self.secondary_plot()?;
        let secondary_max_value = self.secondary_max_value.unwrap_or(1.0).max(1.0);
        let ratio = (secondary_value / secondary_max_value).clamp(0.0, 1.0);
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
            KlinePanelKind::Value => MarkKind::Candle,
            KlinePanelKind::Histogram => MarkKind::Bar,
        }
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

    fn compute_primary_scale_anchor(
        &self,
        x_axis: XAxis,
        min_x_unit: i64,
        max_x_unit: i64,
    ) -> Option<f32> {
        let mut first_unit = i64::MAX;
        let mut base_close = None;

        self.for_each_bar_unit(x_axis, |_, unit, bar| {
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

    fn for_each_bar_unit(&self, x_axis: XAxis, mut f: impl FnMut(&S, i64, &Kline)) {
        for series in self.series {
            match x_axis {
                XAxis::Time { .. } => {
                    for bar in series.bars() {
                        f(series, x_axis.unit_from_time(bar.time), bar);
                    }
                }
                XAxis::Tick { .. } => {
                    let len = series.bars().len();
                    for (index, bar) in series.bars().iter().enumerate() {
                        let from_latest = len.saturating_sub(1).saturating_sub(index) as u64;
                        f(series, x_axis.unit_from_tick(TickIndex(from_latest)), bar);
                    }
                }
            }
        }
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

    fn compute_secondary_max(
        &self,
        x_axis: XAxis,
        min_x_unit: i64,
        max_x_unit: i64,
    ) -> Option<f32> {
        let mut any = false;
        let mut secondary_max_value = 0.0f32;

        self.for_each_bar_unit(x_axis, |series, unit, bar| {
            if unit < min_x_unit || unit > max_x_unit {
                return;
            }

            any = true;
            secondary_max_value = secondary_max_value.max(series.secondary_value(bar));
        });

        any.then_some(secondary_max_value.max(1.0))
    }

    fn compute_scene(&self, layout: Layout<'_>, cursor: mouse::Cursor) -> Option<Scene> {
        let panel_kinds = self.resolved_panel_kinds();
        let panel_layout = PanelLayoutTree::from_layout(layout, panel_kinds)?;
        let primary_panel = panel_layout
            .panels
            .iter()
            .position(|panel| panel.kind == KlinePanelKind::Value)?;
        let primary_mark = self.resolved_panel_mark(primary_panel, KlinePanelKind::Value);
        let primary_scale_mode = self.resolved_panel_scale_mode(primary_panel);
        let secondary_panel = panel_layout
            .panels
            .iter()
            .position(|panel| panel.kind == KlinePanelKind::Histogram);
        let secondary_mark =
            secondary_panel.map(|index| self.resolved_panel_mark(index, KlinePanelKind::Histogram));

        let (x_axis, min_x_unit, max_x_unit) = self.compute_x_window()?;
        let (min_primary_value, max_primary_value) =
            self.compute_primary_domain(x_axis, min_x_unit, max_x_unit)?;
        let primary_scale_anchor = if matches!(primary_scale_mode, PanelScaleMode::PercentFromBase)
        {
            self.compute_primary_scale_anchor(x_axis, min_x_unit, max_x_unit)
        } else {
            None
        };
        let secondary_max_value = if secondary_panel.is_some() {
            self.compute_secondary_max(x_axis, min_x_unit, max_x_unit)
        } else {
            None
        };

        let primary_plot = panel_layout.panel(primary_panel)?.plot;

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
            secondary_panel,
            secondary_mark,
            secondary_max_value,
            cursor: None,
        };

        let mut cursor_info = None;

        if let Some(local) = cursor.position_in(layout.bounds()) {
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

                    let (y_primary_value, y_secondary_value) = match panel.kind {
                        KlinePanelKind::Value => {
                            let value_ratio = 1.0 - (y_in_panel / panel.plot.height.max(1.0));
                            let (min_display, max_display) = scene.primary_domain_display_values();
                            let y_display_value =
                                min_display + ((max_display - min_display) * value_ratio);
                            let y_primary_value = scene.primary_display_to_value(y_display_value);

                            (Some(y_primary_value), None)
                        }
                        KlinePanelKind::Histogram => {
                            let secondary_ratio = 1.0 - (y_in_panel / panel.plot.height.max(1.0));
                            let y_secondary_value =
                                scene.secondary_max_value.unwrap_or(1.0) * secondary_ratio;

                            (None, Some(y_secondary_value))
                        }
                    };

                    cursor_info = Some(CursorInfo {
                        x_unit: snapped_x_unit,
                        panel_index,
                        y_primary_value,
                        y_secondary_value,
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

    fn fill_main_geometry(&self, frame: &mut canvas::Frame, scene: &Scene, palette: &Extended) {
        let px_per_unit = scene.primary_plot().width / scene.span_units().max(1.0);
        let candle_width = (px_per_unit * 0.7).clamp(1.0, 22.0);
        let secondary_width = (px_per_unit * 0.8).clamp(1.0, 24.0);

        let mut primary_line_points: Vec<Point> = Vec::new();
        let mut secondary_line_points: Vec<Point> = Vec::new();

        self.for_each_bar_unit(scene.x_axis, |series, x_unit, bar| {
            if x_unit < scene.min_x_unit || x_unit > scene.max_x_unit {
                return;
            }

            let x = scene.map_x_plot(x_unit);
            let y_open = scene.map_primary_plot(bar.open.to_f32());
            let y_high = scene.map_primary_plot(bar.high.to_f32());
            let y_low = scene.map_primary_plot(bar.low.to_f32());
            let y_close = scene.map_primary_plot(bar.close.to_f32());

            let color = if bar.close >= bar.open {
                palette.success.base.color
            } else {
                palette.danger.base.color
            };

            match scene.primary_mark {
                MarkKind::Line => {
                    primary_line_points.push(Point::new(x, y_close));
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

            if let (Some(y_secondary_value), Some(y_secondary_bottom)) = (
                scene.map_secondary_plot(series.secondary_value(bar)),
                scene.secondary_panel_bottom(),
            ) {
                match scene.secondary_mark.unwrap_or(MarkKind::Bar) {
                    MarkKind::Line => {
                        secondary_line_points.push(Point::new(x, y_secondary_value));
                    }
                    MarkKind::Candle | MarkKind::Bar => {
                        frame.fill_rectangle(
                            Point::new(
                                x - (secondary_width / 2.0),
                                y_secondary_value.min(y_secondary_bottom),
                            ),
                            Size::new(
                                secondary_width,
                                (y_secondary_bottom - y_secondary_value).abs().max(1.0),
                            ),
                            color.scale_alpha(0.4),
                        );
                    }
                }
            }
        });

        if primary_line_points.len() >= 2 {
            let path = canvas::Path::new(|builder| {
                builder.move_to(primary_line_points[0]);
                for point in primary_line_points.iter().skip(1) {
                    builder.line_to(*point);
                }
            });

            frame.stroke(
                &path,
                canvas::Stroke::default()
                    .with_width(1.5)
                    .with_color(palette.background.base.text.scale_alpha(0.85)),
            );
        }

        if secondary_line_points.len() >= 2 {
            let path = canvas::Path::new(|builder| {
                builder.move_to(secondary_line_points[0]);
                for point in secondary_line_points.iter().skip(1) {
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
                    .y_secondary_value
                    .map(|secondary_value| format!("{secondary_value:.2}"))
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
                    return;
                };

                let zone = layout_tree.hit_test(cursor_pos);

                match mouse_event {
                    mouse::Event::WheelScrolled {
                        delta: mouse::ScrollDelta::Lines { y, .. },
                    } => {
                        if !matches!(zone, LayoutHitZone::PanelPlot(_)) {
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
                        } else if matches!(zone, LayoutHitZone::PanelPlot(_)) {
                            state.overlay_cache.clear();
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
