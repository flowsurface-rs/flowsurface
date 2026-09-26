mod axes;
mod chrome;
pub(crate) mod composition;
pub(crate) mod drawing;
mod footprint;
mod helpers;
mod layout;
mod overlay;
mod presentation;
mod primary_render;
mod scene;
mod scene_builder;

use crate::{style, theme::ThemeAccents, window::PixelRatio};
use composition::{
    ChartComposition, DEFAULT_MIN_PANEL_RATIO, LayerDataKind, MarkKind, PanelId, PanelRole,
    PanelScaleMode, PanelValueId,
};
use data::UserTimezone;
use data::chart::Basis;
use data::chart::kline::poc::NPoc;
use data::chart::kline::series::KlineSeries;
use data::chart::kline::{
    CandlestickColorSlot, CandlestickConfig, DEFAULT_HORIZONTAL_SPACING_PX, FootprintConfig,
    FootprintPocColorSlot, KlineStudy, KlineStudyId, LegendVisibility, TpoBoundaryEdits,
    TpoProfile, VolumeProfileColorConfig, VolumeProfileColorSlot, VolumeProfileSchedule,
};
use data::chart::kline::{HorizontalWorld, IndicatorHost, IndicatorInstanceStore};
pub(crate) use data::chart::kline::{PanelYValueRange, YUnit, YUnitRange};
use data::chart::ticks::{
    Y_LABEL_DENSITY,
    x::{TimeTickLabel, TimeTickTier},
    x_labels_that_fit,
    y::{TickValue, YAxisScale},
    y_labels_that_fit,
};
use data::config::theme::ThemeColorRole;
use drawing::{
    AxisBadgeFilter, DrawingDragOperation, DrawingDragTarget, DrawingId, DrawingMeasure,
    DrawingSnapshot, DrawingTool, KlineWidgetDrawingEvent,
};
use exchange::{Kline, TickerInfo, UnixMs, unit::PriceStep};
use helpers::PixelGeometry;
use layout::{KlinePanelKind, LayoutHitZone, PanelLayoutNode, PanelLayoutTree};
pub(crate) use presentation::IndicatorPresentationStatus;
pub(crate) use presentation::{
    IndicatorBarValues, IndicatorPresentationContext, ResolvedIndicatorLayer,
    ResolvedIndicatorPresentation, ResolvedIndicatorStyleSlot, resolve_indicator_bar_values,
    resolve_indicator_presentation_status,
};
use primary_render::PrimaryPlotLayer;
use scene::{CursorTarget, LegendTarget, Scene, SceneCache, TickIndex, XAxis};

use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{self, Clipboard, Layout, Shell, Widget, layout as iced_layout, renderer};
use iced::theme::palette::Extended;
use iced::widget::canvas;
use iced::{
    Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector, keyboard, mouse,
    window,
};
use iced_core::renderer::Quad;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const Y_AXIS_GUTTER: f32 = 66.0;
const X_AXIS_HEIGHT: f32 = 24.0;
pub(crate) const MIN_FULL_CACHE_REFRESH_INTERVAL: Duration = Duration::from_micros(33_333);

const TEXT_SIZE: f32 = 12.0;
const CHAR_W: f32 = TEXT_SIZE * 0.64;
const Y_AXIS_LABEL_LEFT_INSET: f32 = 4.0;
const PANEL_SPLITTER_HEIGHT: f32 = 1.0;
const PIXELS_PER_SCROLL_LINE: f32 = 40.0;
const HORIZONTAL_DRAG_SCALE_DELTA_PER_PX: f32 = 0.3 / 9.0;
const DRAG_THRESHOLD_PX: f32 = 5.0;
const TPO_BOUNDARY_HIT_PX: f32 = 7.0;
pub(super) const TPO_ACTION_BUTTON_WIDTH_PX: f32 = 62.0;
pub(super) const TPO_ACTION_BUTTON_HEIGHT_PX: f32 = 20.0;
pub(super) const TPO_RESET_BUTTON_WIDTH_PX: f32 = 48.0;
const TPO_ACTION_BUTTON_GAP_PX: f32 = 3.0;

const PANEL_TITLE_TOP_PAD: f32 = 4.0;
const Y_AXIS_DRAG_SCALE_DELTA_PER_PX: f32 = 0.1;

const TICKER_LEGEND_PADDING: f32 = 4.0;
const TOOLTIP_ROW_LEFT_PAD: f32 = TICKER_LEGEND_PADDING * 2.0;
const TICKER_LEGEND_ROW_H: f32 = TEXT_SIZE + 6.0;

// Keep plot and widget passes separate: iced_wgpu renders meshes before text
// within each renderer layer.
const PAINT_ORDER: [PaintStep; 11] = [
    PaintStep::Root(RootPaintStep::Separators),
    PaintStep::Plot(PrimaryPlotLayer::Background),
    PaintStep::Plot(PrimaryPlotLayer::Tpo),
    PaintStep::Plot(PrimaryPlotLayer::Foreground),
    PaintStep::Root(RootPaintStep::Drawings),
    PaintStep::Plot(PrimaryPlotLayer::PriceBars),
    PaintStep::Root(RootPaintStep::YAxis),
    PaintStep::Root(RootPaintStep::XAxis),
    PaintStep::Widget(WidgetPaintStep::Overlay),
    PaintStep::Widget(WidgetPaintStep::InteractionText),
    PaintStep::Widget(WidgetPaintStep::Marquee),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaintStep {
    Root(RootPaintStep),
    Plot(PrimaryPlotLayer),
    Widget(WidgetPaintStep),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootPaintStep {
    Separators,
    Drawings,
    YAxis,
    XAxis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WidgetPaintStep {
    Overlay,
    InteractionText,
    Marquee,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OverlayChannelSpec<'a> {
    pub(crate) label: &'a str,
    pub(crate) source: &'a str,
    pub(crate) line_width: f32,
    pub(crate) color_role: ThemeColorRole,
    pub(crate) color_slot: &'a str,
    pub(crate) style_by: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrimaryViewportState {
    Auto,
    Manual { range: Option<YUnitRange> },
}

impl PrimaryViewportState {
    pub(crate) fn is_auto(self) -> bool {
        matches!(self, Self::Auto)
    }

    pub(crate) fn range(self) -> Option<YUnitRange> {
        match self {
            Self::Auto => None,
            Self::Manual { range } => range,
        }
    }

    pub(crate) fn disable(&mut self) -> bool {
        if !self.is_auto() {
            return false;
        }

        *self = Self::Manual { range: None };
        true
    }

    pub(crate) fn set_range(&mut self, range: YUnitRange) -> bool {
        let next = Self::Manual { range: Some(range) };
        if *self == next {
            return false;
        }

        *self = next;
        true
    }

    pub(crate) fn reset(&mut self) -> bool {
        if matches!(self, Self::Auto) {
            return false;
        }

        *self = Self::Auto;
        true
    }

    pub(crate) fn toggle(&mut self) -> bool {
        if self.is_auto() {
            *self = Self::Manual { range: None };
        } else {
            *self = Self::Auto;
        }
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Regions {
    pub plot: Rectangle,
    pub x_axis: Rectangle,
    pub y_axis: Rectangle,
}

impl Regions {
    pub fn from_layout(root: iced_core::Layout<'_>) -> Self {
        let root_bounds = root.bounds();

        let row = root.child(0);
        let x_abs = root.child(1).bounds();

        let plot_abs = row.child(0).bounds();
        let y_abs = row.child(1).bounds();

        let to_local = |r: Rectangle| Rectangle {
            x: r.x - root_bounds.x,
            y: r.y - root_bounds.y,
            width: r.width,
            height: r.height,
        };

        Regions {
            plot: to_local(plot_abs),
            y_axis: to_local(y_abs),
            x_axis: to_local(x_abs),
        }
    }
}

pub(super) enum DragMode {
    None,
    Split(usize),
    TpoBoundary {
        boundary: UnixMs,
        preview: UnixMs,
        minimum: Option<UnixMs>,
        maximum: Option<UnixMs>,
    },
    YAxisScale {
        panel_index: usize,
        anchor_y: f32,
    },
    XAxisScale {
        anchor_x: f32,
        initial_world: HorizontalWorld,
        initial_right_unit: f32,
    },
    Pan {
        panel_index: usize,
    },
    DrawingMove {
        id: DrawingId,
        target: DrawingDragTarget,
        panel_id: PanelId,
    },
    Marquee {
        panel_index: usize,
        start: Point,
        current: Point,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TpoInteraction {
    MergeLeft { boundary: UnixMs },
    MoveBoundary { from: UnixMs, to: UnixMs },
    Reset { start: UnixMs, end: UnixMs },
}

#[derive(Debug, Clone, Copy)]
pub(super) struct TpoProfileGeometry {
    pub(super) index: usize,
    pub(super) start: UnixMs,
    pub(super) end: UnixMs,
    pub(super) overlay_bounds: Rectangle,
    pub(super) button_bounds: Option<Rectangle>,
    pub(super) reset_button_bounds: Option<Rectangle>,
}

#[derive(Debug, Clone, Copy)]
enum TpoHit {
    Reset {
        start: UnixMs,
        end: UnixMs,
    },
    Profile {
        start: UnixMs,
    },
    Boundary {
        boundary: UnixMs,
        minimum: Option<UnixMs>,
        maximum: Option<UnixMs>,
    },
    MergeLeft {
        boundary: UnixMs,
        left_start: UnixMs,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HorizontalWheelZoomMode {
    LiveEdge,
    Cursor,
}

impl HorizontalWheelZoomMode {
    fn from_modifiers(modifiers: keyboard::Modifiers) -> Self {
        if modifiers.intersects(keyboard::Modifiers::COMMAND) {
            Self::Cursor
        } else {
            Self::LiveEdge
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct GutterCacheKey {
    version: u64,
    width: f32,
    height: f32,
    panel_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct GeometryCacheKey {
    origin: Point,
    size: Size,
    pixel_ratio: PixelRatio,
}

impl GeometryCacheKey {
    fn new(rect: Rectangle, origin: Point, pixel_ratio: PixelRatio) -> Self {
        Self {
            origin,
            size: rect.size(),
            pixel_ratio,
        }
    }
}

struct State {
    plot_background_cache: canvas::Cache,
    plot_cache: canvas::Cache,
    tpo_cache: canvas::Cache,
    price_bars_cache: canvas::Cache,
    y_axis_cache: canvas::Cache,
    x_axis_cache: canvas::Cache,
    overlay_cache: canvas::Cache,
    interaction_text_cache: canvas::Cache,
    drawing_cache: canvas::Cache,
    marquee_cache: canvas::Cache,
    drag_mode: DragMode,
    selected_tpo_profile: Option<UnixMs>,
    pending_tpo_hit: Option<TpoHit>,
    drag_start: Option<Point>,
    last_cursor: Option<Point>,
    modifiers: keyboard::Modifiers,
    y_axis_gutter_width: f32,
    gutter_cache_key: Option<GutterCacheKey>,
    plot_geometry_key: Option<GeometryCacheKey>,
    drawing_geometry_key: Option<GeometryCacheKey>,
    overlay_geometry_key: Option<GeometryCacheKey>,
    interaction_text_geometry_key: Option<GeometryCacheKey>,
    x_axis_geometry_key: Option<GeometryCacheKey>,
    last_cache_rev: u64,
    last_cache_clear_at: Option<Instant>,
    last_theme_generation: Option<u64>,
    last_countdown_second: Option<i64>,
    previous_click: Option<iced_core::mouse::Click>,
    scene_cache: SceneCache,
}

impl Default for State {
    fn default() -> Self {
        Self {
            plot_background_cache: canvas::Cache::new(),
            plot_cache: canvas::Cache::new(),
            tpo_cache: canvas::Cache::new(),
            price_bars_cache: canvas::Cache::new(),
            y_axis_cache: canvas::Cache::new(),
            x_axis_cache: canvas::Cache::new(),
            overlay_cache: canvas::Cache::new(),
            interaction_text_cache: canvas::Cache::new(),
            drawing_cache: canvas::Cache::new(),
            marquee_cache: canvas::Cache::new(),
            drag_mode: DragMode::None,
            selected_tpo_profile: None,
            pending_tpo_hit: None,
            drag_start: None,
            last_cursor: None,
            modifiers: keyboard::Modifiers::empty(),
            y_axis_gutter_width: Y_AXIS_GUTTER,
            gutter_cache_key: None,
            plot_geometry_key: None,
            drawing_geometry_key: None,
            overlay_geometry_key: None,
            interaction_text_geometry_key: None,
            x_axis_geometry_key: None,
            last_cache_rev: 0,
            last_cache_clear_at: None,
            last_theme_generation: None,
            last_countdown_second: None,
            previous_click: None,
            scene_cache: SceneCache::default(),
        }
    }
}

impl State {
    fn clear_all_caches(&mut self) {
        self.plot_background_cache.clear();
        self.plot_cache.clear();
        self.tpo_cache.clear();
        self.price_bars_cache.clear();
        self.y_axis_cache.clear();
        self.x_axis_cache.clear();
        self.drawing_cache.clear();
        self.clear_overlay_caches();
        self.marquee_cache.clear();
    }

    fn cache_refresh_at(
        &self,
        version: u64,
        now: Instant,
        minimum_interval: Duration,
    ) -> Option<Instant> {
        (self.last_cache_rev != version).then(|| {
            self.last_cache_clear_at
                .map_or(now, |last_clear| last_clear + minimum_interval)
        })
    }

    fn mark_cache_refreshed(&mut self, version: u64, now: Instant) {
        self.last_cache_rev = version;
        self.last_cache_clear_at = Some(now);
    }

    fn clear_overlay_caches(&mut self) {
        self.overlay_cache.clear();
        self.interaction_text_cache.clear();
    }

    fn clear_cursor_caches(&mut self) {
        self.clear_overlay_caches();

        if matches!(self.drag_mode, DragMode::Marquee { .. }) {
            self.marquee_cache.clear();
        }
    }

    fn sync_geometry_cache_keys(
        &mut self,
        bounds: Rectangle,
        regions: Regions,
        pixel_ratio: PixelRatio,
    ) {
        let root_origin = Point::new(bounds.x, bounds.y);
        let plot_origin = Point::new(bounds.x + regions.plot.x, bounds.y + regions.plot.y);
        let x_axis_origin = Point::new(bounds.x + regions.x_axis.x, bounds.y + regions.x_axis.y);

        let plot_geometry_key = GeometryCacheKey::new(regions.plot, plot_origin, pixel_ratio);
        if self.plot_geometry_key != Some(plot_geometry_key) {
            self.plot_background_cache.clear();
            self.plot_cache.clear();
            self.tpo_cache.clear();
            self.price_bars_cache.clear();
            self.plot_geometry_key = Some(plot_geometry_key);
        }
        Self::sync_geometry_cache(
            &self.drawing_cache,
            &mut self.drawing_geometry_key,
            GeometryCacheKey::new(bounds, root_origin, pixel_ratio),
        );
        Self::sync_geometry_cache(
            &self.overlay_cache,
            &mut self.overlay_geometry_key,
            GeometryCacheKey::new(bounds, root_origin, pixel_ratio),
        );
        Self::sync_geometry_cache(
            &self.interaction_text_cache,
            &mut self.interaction_text_geometry_key,
            GeometryCacheKey::new(bounds, root_origin, pixel_ratio),
        );
        Self::sync_geometry_cache(
            &self.x_axis_cache,
            &mut self.x_axis_geometry_key,
            GeometryCacheKey::new(regions.x_axis, x_axis_origin, pixel_ratio),
        );
    }

    fn sync_geometry_cache(
        cache: &canvas::Cache,
        previous: &mut Option<GeometryCacheKey>,
        next: GeometryCacheKey,
    ) {
        if *previous != Some(next) {
            cache.clear();
            *previous = Some(next);
        }
    }

    fn rounded_drag_delta(&self, cursor: Point) -> Vector {
        let Some(previous) = self.last_cursor else {
            return Vector::new(0.0, 0.0);
        };
        let start = self.drag_start.unwrap_or(previous);

        // Round cumulative movement so sub-pixel events accumulate instead of disappearing.
        Vector::new(
            (cursor.x - start.x).round() - (previous.x - start.x).round(),
            (cursor.y - start.y).round() - (previous.y - start.y).round(),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PanelInteraction {
    YViewportChanged {
        panel_id: PanelId,
        range: YUnitRange,
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
    Close {
        index: usize,
    },
}

#[derive(Debug, Clone)]
pub enum KlineWidgetEvent {
    UndoRequested,
    RedoRequested,
    EscapePressed,
    PrimaryScaleInversionRequested,
    HorizontalWorldChanged(HorizontalWorld),
    HorizontalOffsetChanged(f32),
    PlotWidthChanged(f32),
    PrimaryAutoscaleDisabled,
    PrimaryAutoscaleToggled,
    PrimaryScaleModeCycleRequested,
    PanelInteraction(PanelInteraction),
    Tpo(TpoInteraction),
    TickerSettings(TickerInfo),
    FootprintSettings,
    TickerRemove(TickerInfo),
    CandlesticksVisibilityToggled,
    FootprintsVisibilityToggled,
    IndicatorSettings(PanelValueId),
    IndicatorVisibilityToggled(PanelValueId),
    IndicatorRemove(PanelValueId),
    StudySettings(KlineStudyId),
    StudyVisibilityToggled(KlineStudyId),
    StudyRemove(KlineStudyId),
    StudiesVisibilityToggled,
    StudiesExpansionToggled,
    StudiesRemove,
    XAxisDoubleClick,
    DeletePressed,
    Drawing(KlineWidgetDrawingEvent),
}

enum ProfileBarSource<'a> {
    Empty,
    External(&'a BTreeMap<UnixMs, Kline>),
    Chart { retained: (UnixMs, UnixMs) },
}

impl ProfileBarSource<'_> {
    fn collect(&self, series: &[KlineSeries]) -> Vec<Kline> {
        match self {
            Self::Empty => Vec::new(),
            Self::External(bars) => bars.values().copied().collect(),
            Self::Chart { retained } => series
                .first()
                .into_iter()
                .flat_map(KlineSeries::points)
                .filter(|point| point.kline.time >= retained.0 && point.kline.time < retained.1)
                .map(|point| *point.kline)
                .collect(),
        }
    }
}

pub struct KlineWidget<'a> {
    series: &'a [KlineSeries],
    indicator_host: &'a IndicatorHost,
    composition: &'a ChartComposition,
    indicator_instances: &'a IndicatorInstanceStore,
    studies: &'a [KlineStudy],
    profile_bars: ProfileBarSource<'a>,
    tpo_profiles: &'a [TpoProfile],
    tpo_edits: &'a TpoBoundaryEdits,
    studies_legend_expanded: bool,
    legend_visibility: LegendVisibility,
    candlestick: CandlestickConfig,
    footprint: FootprintConfig,
    trade_data_enabled: bool,
    drawings: DrawingSnapshot<'a>,
    basis: Basis,
    horizontal_world: HorizontalWorld,
    known_plot_width: Option<f32>,
    autoscale_plot_width_offset: f32,
    pixel_ratio: PixelRatio,
    primary_viewport: PrimaryViewportState,
    primary_scale_inverted: bool,
    indicator_y_ranges: &'a [(PanelId, YUnitRange)],
    indicator_presentation: IndicatorPresentationContext,
    timezone: UserTimezone,
    version: u64,
    theme_accents: ThemeAccents,
    escape_enabled: bool,
}

impl<'a> KlineWidget<'a> {
    pub fn new(
        series: &'a [KlineSeries],
        indicator_host: &'a IndicatorHost,
        basis: Basis,
        composition: &'a ChartComposition,
        indicator_instances: &'a IndicatorInstanceStore,
        theme_accents: ThemeAccents,
        tpo_edits: &'a TpoBoundaryEdits,
    ) -> Self {
        Self {
            series,
            indicator_host,
            composition,
            indicator_instances,
            studies: &[],
            profile_bars: ProfileBarSource::Empty,
            tpo_profiles: &[],
            tpo_edits,
            studies_legend_expanded: true,
            legend_visibility: LegendVisibility::default(),
            candlestick: CandlestickConfig::default(),
            footprint: FootprintConfig::default(),
            trade_data_enabled: false,
            drawings: DrawingSnapshot::new(DrawingTool::Cursor, false, &[], &[], None, None),
            basis,
            horizontal_world: HorizontalWorld::new(DEFAULT_HORIZONTAL_SPACING_PX, 0.0),
            known_plot_width: None,
            autoscale_plot_width_offset: 0.0,
            pixel_ratio: PixelRatio::default(),
            primary_viewport: PrimaryViewportState::Auto,
            primary_scale_inverted: false,
            indicator_y_ranges: &[],
            indicator_presentation: IndicatorPresentationContext::default(),
            timezone: UserTimezone::Utc,
            version: 0,
            theme_accents,
            escape_enabled: false,
        }
    }

    pub fn with_studies(mut self, studies: &'a [KlineStudy]) -> Self {
        self.studies = studies;
        self
    }

    pub fn with_tpo_profiles(mut self, profiles: &'a [TpoProfile]) -> Self {
        self.tpo_profiles = profiles;
        self
    }

    pub fn with_external_profile_bars(mut self, bars: Option<&'a BTreeMap<UnixMs, Kline>>) -> Self {
        self.profile_bars = bars.map_or(ProfileBarSource::Empty, ProfileBarSource::External);
        self
    }

    pub fn with_chart_profile_range(mut self, retained: Option<(UnixMs, UnixMs)>) -> Self {
        self.profile_bars = retained.map_or(ProfileBarSource::Empty, |retained| {
            ProfileBarSource::Chart { retained }
        });
        self
    }

    fn tpo_hit_test(
        &self,
        scene: &Scene,
        point: Point,
        selected_profile: Option<UnixMs>,
    ) -> Option<TpoHit> {
        let geometries = self.tpo_profile_geometries(scene, selected_profile);

        for geometry in &geometries {
            if geometry
                .reset_button_bounds
                .is_some_and(|bounds| point_in_rectangle(point, bounds))
            {
                return Some(TpoHit::Reset {
                    start: geometry.start,
                    end: geometry.end,
                });
            }
            if geometry
                .button_bounds
                .is_some_and(|bounds| point_in_rectangle(point, bounds))
            {
                let left_start = self
                    .tpo_profiles
                    .get(geometry.index.saturating_sub(1))?
                    .start;
                return Some(TpoHit::MergeLeft {
                    boundary: geometry.start,
                    left_start,
                });
            }
        }

        if selected_profile.is_some() {
            let snap_timeframe = match self.basis {
                Basis::Time(timeframe) => timeframe,
                Basis::Tick(_) => return None,
            };
            let selected_index = selected_profile.and_then(|start| {
                self.tpo_profiles
                    .iter()
                    .position(|profile| profile.start == start)
            });
            let plot = scene.primary_plot();
            let plot_top = scene.layout.regions.plot.y + plot.y;
            let plot_bottom = plot_top + plot.height;
            for (index, profile) in self.tpo_profiles.iter().enumerate().skip(1) {
                let Some(boundary_unit) = scene.x.axis.unit_from_time_f32(profile.start) else {
                    continue;
                };
                let boundary_x = scene.layout.regions.plot.x + scene.map_x_plot_f32(boundary_unit);
                if (point.x - boundary_x).abs() <= TPO_BOUNDARY_HIT_PX
                    && point.y >= plot_top
                    && point.y <= plot_bottom
                {
                    let Some((minimum, maximum)) = selected_index.and_then(|selected_index| {
                        let selected = self.tpo_profiles.get(selected_index)?;
                        if index == selected_index {
                            Some((
                                None,
                                Some(selected.end.offset_by_timeframe(snap_timeframe, -1)),
                            ))
                        } else if index == selected_index + 1 {
                            Some((
                                Some(selected.start.offset_by_timeframe(snap_timeframe, 1)),
                                None,
                            ))
                        } else {
                            None
                        }
                    }) else {
                        continue;
                    };
                    return Some(TpoHit::Boundary {
                        boundary: profile.start,
                        minimum,
                        maximum,
                    });
                }
            }
        }

        geometries
            .iter()
            .rev()
            .find(|geometry| point_in_rectangle(point, geometry.overlay_bounds))
            .map(|geometry| TpoHit::Profile {
                start: geometry.start,
            })
    }

    fn tpo_boundary_from_cursor(
        &self,
        scene: &Scene,
        cursor: Point,
        boundary: UnixMs,
        minimum: Option<UnixMs>,
        maximum: Option<UnixMs>,
    ) -> Option<UnixMs> {
        let pair_index = self
            .tpo_profiles
            .windows(2)
            .position(|profiles| profiles[1].start == boundary)?;
        let left_profile = self.tpo_profiles.get(pair_index)?;
        let snap_timeframe = match self.basis {
            Basis::Time(timeframe) => timeframe,
            Basis::Tick(_) => return None,
        };
        let period_ms = snap_timeframe.to_milliseconds().max(1);
        let raw_time = scene.time_from_plot_x_f32(cursor.x - scene.layout.regions.plot.x)?;
        let (before_anchor, elapsed) = if raw_time >= left_profile.start {
            (false, raw_time.as_u64() - left_profile.start.as_u64())
        } else {
            (true, left_profile.start.as_u64() - raw_time.as_u64())
        };
        let whole_periods = elapsed / period_ms;
        let rounded_periods =
            whole_periods.saturating_add(u64::from(elapsed % period_ms >= period_ms / 2));
        let intervals = i64::try_from(rounded_periods).unwrap_or(i64::MAX);
        let candidate = left_profile.start.offset_by_timeframe(
            snap_timeframe,
            if before_anchor { -intervals } else { intervals },
        );
        let minimum = minimum.unwrap_or(self.tpo_profiles.first()?.start);
        let maximum = maximum.unwrap_or(self.tpo_profiles.last()?.end);
        (minimum <= maximum).then(|| candidate.clamp(minimum, maximum))
    }

    pub fn with_studies_legend_expanded(mut self, expanded: bool) -> Self {
        self.studies_legend_expanded = expanded;
        self
    }

    pub fn with_legend_visibility(mut self, visibility: LegendVisibility) -> Self {
        self.legend_visibility = visibility;
        self
    }

    pub fn with_candlestick_config(mut self, config: CandlestickConfig) -> Self {
        self.candlestick = config;
        self
    }

    pub fn with_footprint_config(mut self, config: FootprintConfig) -> Self {
        self.footprint = config;
        self
    }

    pub fn with_trade_data_enabled(mut self, enabled: bool) -> Self {
        self.trade_data_enabled = enabled;
        self
    }

    pub fn with_coordinate_state(
        mut self,
        horizontal_world: HorizontalWorld,
        pixel_ratio: PixelRatio,
    ) -> Self {
        self.horizontal_world = horizontal_world;
        self.pixel_ratio = pixel_ratio;
        self
    }

    pub(crate) fn with_primary_viewport(mut self, viewport: PrimaryViewportState) -> Self {
        self.primary_viewport = viewport;
        self
    }

    pub fn with_primary_scale_inverted(mut self, inverted: bool) -> Self {
        self.primary_scale_inverted = inverted;
        self
    }

    pub fn with_known_plot_width(mut self, plot_width: Option<f32>) -> Self {
        self.known_plot_width = plot_width;
        self
    }

    pub fn with_autoscale_plot_width_offset(mut self, offset: f32) -> Self {
        self.autoscale_plot_width_offset = if offset.is_finite() {
            offset.max(0.0)
        } else {
            0.0
        };
        self
    }

    pub fn with_drawings(mut self, snapshot: DrawingSnapshot<'a>) -> Self {
        self.drawings = snapshot;
        self
    }

    pub(crate) fn with_indicator_ranges(mut self, ranges: &'a [(PanelId, YUnitRange)]) -> Self {
        self.indicator_y_ranges = ranges;
        self
    }

    pub fn with_indicator_presentation(
        mut self,
        presentation: IndicatorPresentationContext,
    ) -> Self {
        self.indicator_presentation = presentation;
        self
    }

    pub fn with_timezone(mut self, tz: UserTimezone) -> Self {
        self.timezone = tz;
        self
    }

    fn indicator_is_unavailable(&self, value_id: Option<PanelValueId>) -> bool {
        value_id.is_some_and(|value_id| self.indicator_presentation.is_unavailable(value_id))
    }

    pub(super) fn indicator_unavailable_message(
        &self,
        value_id: Option<PanelValueId>,
    ) -> Option<&str> {
        let value_id = value_id?;
        self.indicator_presentation.unavailable_message(value_id)
    }

    pub(super) fn indicator_marker(&self, value_id: PanelValueId) -> Option<(&'static str, bool)> {
        if self.indicator_presentation.is_loading(value_id) {
            return Some(("...", true));
        }
        self.indicator_unavailable_message(Some(value_id))
            .map(|_| ("!", false))
    }

    pub(super) fn indicator_label_x(&self, row_x: f32, value_id: PanelValueId) -> f32 {
        let marker_width = self.indicator_marker(value_id).map_or(0.0, |(marker, _)| {
            marker.chars().count() as f32 * CHAR_W + TICKER_LEGEND_PADDING
        });
        row_x + TOOLTIP_ROW_LEFT_PAD + marker_width
    }

    pub(super) fn indicator_marker_x(&self, row_x: f32) -> f32 {
        row_x + TOOLTIP_ROW_LEFT_PAD
    }

    pub fn with_version(mut self, rev: u64) -> Self {
        self.version = rev;
        self
    }

    pub fn with_escape_enabled(mut self, enabled: bool) -> Self {
        self.escape_enabled = enabled;
        self
    }

    fn is_command_modifier(modifiers: keyboard::Modifiers) -> bool {
        modifiers.intersects(keyboard::Modifiers::COMMAND)
    }

    fn drawing_magnet_enabled(&self, modifiers: keyboard::Modifiers) -> bool {
        self.drawings.active_tool.supports_magnet()
            && (self.drawings.magnet_enabled || Self::is_command_modifier(modifiers))
    }

    fn paint_separators(renderer: &mut Renderer, scene: &Scene, theme: &Theme) {
        use advanced::Renderer as _;

        let plot_rect = scene.plot_rect();
        let splitter_color = style::split_ruler(theme).color;

        for splitter in &scene.layout.splitters {
            renderer.fill_quad(
                Quad {
                    bounds: Rectangle {
                        x: plot_rect.x + splitter.x,
                        y: plot_rect.y + splitter.y,
                        width: splitter.width + scene.layout.regions.y_axis.width,
                        height: splitter.height,
                    },
                    snap: true,
                    ..Default::default()
                },
                splitter_color,
            );
        }

        renderer.fill_quad(
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

        renderer.fill_quad(
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
    }
}

impl<'a, M> Widget<M, Theme, Renderer> for KlineWidget<'a>
where
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
        tree: &mut Tree,
        _renderer: &Renderer,
        limits: &iced_layout::Limits,
    ) -> iced_layout::Node {
        let panel_count = self.panel_count();

        let build_layout = |y_axis_gutter: f32| {
            let build_panel_stack = |stack_size: Size| {
                let plot_heights = self.panel_plot_heights(stack_size.height, panel_count);
                let mut children =
                    Vec::with_capacity(panel_count.saturating_mul(3).saturating_sub(1));

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
                            iced_layout::Node::new(Size::new(
                                stack_size.width,
                                PANEL_SPLITTER_HEIGHT,
                            ))
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
                        &l.shrink(Size::new(y_axis_gutter, 0.0)),
                        Length::Fill,
                        Length::Fill,
                    );

                    build_panel_stack(stack_node.size())
                },
                |l| iced_layout::atomic(l, y_axis_gutter, Length::Fill),
            );

            let x_axis_node = iced_layout::next_to_each_other(
                limits,
                0.0,
                |l| {
                    iced_layout::atomic(
                        &l.shrink(Size::new(y_axis_gutter, 0.0)),
                        Length::Fill,
                        X_AXIS_HEIGHT,
                    )
                },
                |l| iced_layout::atomic(l, y_axis_gutter, X_AXIS_HEIGHT),
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
        };

        let layout_size = limits.max();
        let gutter_cache_key = GutterCacheKey {
            version: self.version,
            width: layout_size.width,
            height: layout_size.height,
            panel_count,
        };
        let state = tree.state.downcast_mut::<State>();
        let preserve_y_axis_gutter =
            self.composition
                .panels
                .iter()
                .enumerate()
                .any(|(panel_index, panel)| {
                    panel.role == PanelRole::Auxiliary
                        && self
                            .indicator_panel_value_id(panel_index)
                            .is_some_and(|value_id| {
                                self.indicator_presentation.is_loading(value_id)
                            })
                });

        let y_axis_gutter = if preserve_y_axis_gutter && state.gutter_cache_key.is_some() {
            state.gutter_cache_key = Some(gutter_cache_key);
            state.y_axis_gutter_width
        } else if state.gutter_cache_key == Some(gutter_cache_key) {
            state.y_axis_gutter_width
        } else {
            let mut gutter = Y_AXIS_GUTTER;
            let mut node = build_layout(gutter);

            for _ in 0..2 {
                let required = {
                    let provisional_layout = iced_layout::Layout::new(&node);
                    self.compute_scene_uncached(
                        provisional_layout,
                        mouse::Cursor::Unavailable,
                        false,
                        false,
                    )
                    .map(|scene| self.required_y_axis_gutter(&scene))
                };

                let Some(required) = required else {
                    break;
                };

                if required <= gutter + f32::EPSILON {
                    break;
                }

                gutter = required;
                node = build_layout(gutter);
            }

            state.y_axis_gutter_width = gutter;
            state.gutter_cache_key = Some(gutter_cache_key);
            gutter
        };

        build_layout(y_axis_gutter)
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

        {
            let state = tree.state.downcast_mut::<State>();
            self.ensure_scene_base(state, layout);
        }

        match event {
            Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                physical_key,
                modifiers,
                ..
            }) => {
                let state = tree.state.downcast_mut::<State>();
                state.modifiers = *modifiers;
                state.clear_overlay_caches();

                if self.escape_enabled
                    && cursor.is_over(layout.bounds())
                    && matches!(key, keyboard::Key::Named(keyboard::key::Named::Escape))
                {
                    shell.publish(M::from(KlineWidgetEvent::EscapePressed));
                    shell.capture_event();
                    return;
                }

                if cursor.is_over(layout.bounds())
                    && let Some(event) = history_shortcut(key.as_ref(), *modifiers)
                {
                    shell.publish(M::from(event));
                    shell.capture_event();
                    return;
                }

                if cursor.is_over(layout.bounds())
                    && primary_scale_inversion_shortcut(*physical_key, *modifiers)
                {
                    shell.publish(M::from(KlineWidgetEvent::PrimaryScaleInversionRequested));
                    shell.capture_event();
                    return;
                }

                shell.publish(M::from(KlineWidgetEvent::Drawing(
                    KlineWidgetDrawingEvent::MagnetModifierChanged(Self::is_command_modifier(
                        *modifiers,
                    )),
                )));

                match key.as_ref() {
                    keyboard::Key::Named(keyboard::key::Named::Backspace)
                    | keyboard::Key::Named(keyboard::key::Named::Delete)
                        if cursor.is_over(layout.bounds()) =>
                    {
                        shell.publish(M::from(KlineWidgetEvent::DeletePressed));
                        shell.capture_event();
                    }
                    _ => {}
                }
            }
            Event::Keyboard(keyboard::Event::KeyReleased { modifiers, .. }) => {
                let state = tree.state.downcast_mut::<State>();
                state.modifiers = *modifiers;
                state.clear_overlay_caches();
                shell.publish(M::from(KlineWidgetEvent::Drawing(
                    KlineWidgetDrawingEvent::MagnetModifierChanged(Self::is_command_modifier(
                        *modifiers,
                    )),
                )));
            }
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                let state = tree.state.downcast_mut::<State>();
                state.modifiers = *modifiers;
                state.clear_overlay_caches();
                shell.publish(M::from(KlineWidgetEvent::Drawing(
                    KlineWidgetDrawingEvent::MagnetModifierChanged(Self::is_command_modifier(
                        *modifiers,
                    )),
                )));
            }
            Event::Mouse(mouse_event) => {
                let state = tree.state.downcast_mut::<State>();
                let bounds = layout.bounds();
                let Some(layout_tree) =
                    PanelLayoutTree::from_layout(layout, &self.composition.panels)
                else {
                    return;
                };

                if matches!(
                    mouse_event,
                    mouse::Event::ButtonReleased(mouse::Button::Left)
                ) && cursor.position_in(bounds).is_none()
                {
                    let was_dragging = !matches!(state.drag_mode, DragMode::None);
                    if let DragMode::DrawingMove { id, .. } = state.drag_mode {
                        shell.publish(M::from(KlineWidgetEvent::Drawing(
                            KlineWidgetDrawingEvent::DragFinished { id },
                        )));
                    }
                    if let DragMode::TpoBoundary {
                        boundary, preview, ..
                    } = state.drag_mode
                    {
                        state.selected_tpo_profile = Some(preview);
                        if boundary != preview {
                            shell.publish(M::from(KlineWidgetEvent::Tpo(
                                TpoInteraction::MoveBoundary {
                                    from: boundary,
                                    to: preview,
                                },
                            )));
                        }
                    }
                    state.pending_tpo_hit = None;
                    state.drag_mode = DragMode::None;
                    state.drag_start = None;
                    state.last_cursor = None;
                    state.clear_overlay_caches();
                    state.clear_all_caches();
                    if was_dragging {
                        shell.capture_event();
                    }
                    return;
                }

                let cursor_pos = if let Some(local) = cursor.position_in(bounds) {
                    local
                } else if matches!(
                    state.drag_mode,
                    DragMode::DrawingMove { .. }
                        | DragMode::YAxisScale { .. }
                        | DragMode::XAxisScale { .. }
                        | DragMode::TpoBoundary { .. }
                        | DragMode::Marquee { .. }
                ) {
                    if let Some(global) = cursor.position() {
                        Point::new(global.x - bounds.x, global.y - bounds.y)
                    } else {
                        state.clear_cursor_caches();
                        return;
                    }
                } else {
                    state.clear_cursor_caches();
                    return;
                };

                let zone = layout_tree.hit_test(cursor_pos);
                let scene = self.cached_scene(state, layout, cursor);
                let cursor_target = scene.as_ref().map(|scene| scene.interaction.target);
                let corner_controls = scene
                    .as_ref()
                    .map(|scene| scene.chrome.corner_controls.as_slice())
                    .unwrap_or(&[]);
                let ticker_legend = scene
                    .as_ref()
                    .and_then(|scene| scene.chrome.ticker_legend.as_ref());
                let study_legend = scene
                    .as_ref()
                    .and_then(|scene| scene.chrome.study_legend.as_ref());
                let indicator_legend = scene
                    .as_ref()
                    .and_then(|scene| scene.chrome.indicator_legend.as_ref());

                match mouse_event {
                    mouse::Event::WheelScrolled { delta } => {
                        let scroll_y = {
                            let delta = match delta {
                                mouse::ScrollDelta::Lines { y, .. } => *y,
                                mouse::ScrollDelta::Pixels { y, .. } => *y / PIXELS_PER_SCROLL_LINE,
                            };

                            if delta.is_finite() { delta } else { 0.0 }
                        };

                        if scroll_y.abs() <= f32::EPSILON {
                            return;
                        }

                        match zone {
                            LayoutHitZone::PanelPlot(_)
                            | LayoutHitZone::PanelXAxis(_)
                            | LayoutHitZone::BottomXAxis => {
                                if matches!(zone, LayoutHitZone::PanelPlot(_))
                                    && matches!(
                                        cursor_target,
                                        Some(CursorTarget::PanelControl { .. })
                                    )
                                {
                                    return;
                                }

                                if matches!(zone, LayoutHitZone::PanelPlot(_))
                                    && matches!(cursor_target, Some(CursorTarget::CornerControl(_)))
                                {
                                    return;
                                }

                                if matches!(zone, LayoutHitZone::PanelPlot(_))
                                    && matches!(cursor_target, Some(CursorTarget::Legend(_)))
                                {
                                    return;
                                }

                                let horizontal_zoom_mode = if matches!(
                                    zone,
                                    LayoutHitZone::PanelXAxis(_) | LayoutHitZone::BottomXAxis
                                ) {
                                    HorizontalWheelZoomMode::Cursor
                                } else {
                                    HorizontalWheelZoomMode::from_modifiers(state.modifiers)
                                };
                                let horizontal_zoom_delta = scroll_y / 2.0;

                                if horizontal_zoom_delta.abs() <= f32::EPSILON {
                                    return;
                                }

                                let scene = match zone {
                                    LayoutHitZone::PanelPlot(_)
                                    | LayoutHitZone::PanelXAxis(_)
                                    | LayoutHitZone::BottomXAxis => {
                                        self.cached_scene(state, layout, cursor)
                                    }
                                    _ => None,
                                };
                                let horizontal_world = scene.as_ref().map(|scene| {
                                    self.horizontal_world_for_cursor(
                                        scene,
                                        cursor_pos.x,
                                        horizontal_zoom_delta,
                                        horizontal_zoom_mode,
                                    )
                                });
                                let new_world = horizontal_world.unwrap_or(self.horizontal_world);
                                let horizontal_changed = new_world != self.horizontal_world;

                                if horizontal_changed {
                                    shell.publish(M::from(
                                        KlineWidgetEvent::HorizontalWorldChanged(new_world),
                                    ));
                                    state.clear_all_caches();
                                    shell.capture_event();
                                }
                            }
                            LayoutHitZone::YAxis(panel_index) => {
                                let Some(scene) = self.cached_scene(state, layout, cursor) else {
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

                                let interaction = PanelInteraction::YViewportChanged {
                                    panel_id,
                                    range: viewport,
                                };

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
                        state.drag_start = Some(cursor_pos);
                        state.pending_tpo_hit = None;

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

                        if let (
                            Some(legend),
                            Some(CursorTarget::Legend(LegendTarget::Ticker {
                                row: row_index,
                                icon: Some(icon_kind),
                            })),
                        ) = (ticker_legend, cursor_target)
                            && let Some(row) = legend.rows.get(row_index)
                        {
                            shell.publish(M::from(icon_kind.into_event(row.ticker)));
                            state.drag_mode = DragMode::None;
                            state.last_cursor = None;
                            state.clear_all_caches();
                            shell.capture_event();
                            return;
                        }

                        if let Some(CursorTarget::Legend(LegendTarget::Footprint {
                            icon: Some(icon_kind),
                        })) = cursor_target
                        {
                            shell.publish(M::from(icon_kind.into_event()));
                            state.drag_mode = DragMode::None;
                            state.last_cursor = None;
                            state.clear_all_caches();
                            shell.capture_event();
                            return;
                        }

                        if let Some(CursorTarget::Legend(LegendTarget::StudyCategory {
                            icon: Some(icon_kind),
                        })) = cursor_target
                        {
                            shell.publish(M::from(icon_kind.into_event()));
                            state.drag_mode = DragMode::None;
                            state.last_cursor = None;
                            state.clear_all_caches();
                            shell.capture_event();
                            return;
                        }

                        if let (
                            Some(legend),
                            Some(CursorTarget::Legend(LegendTarget::Study {
                                row: row_index,
                                icon: Some(icon_kind),
                            })),
                        ) = (study_legend, cursor_target)
                            && let Some(row) = legend.rows.get(row_index)
                        {
                            shell.publish(M::from(icon_kind.into_event(row.study_id)));
                            state.drag_mode = DragMode::None;
                            state.last_cursor = None;
                            state.clear_all_caches();
                            shell.capture_event();
                            return;
                        }

                        if let (
                            Some(legend),
                            Some(CursorTarget::Legend(LegendTarget::Indicator {
                                row: row_index,
                                icon: Some(icon_kind),
                            })),
                        ) = (indicator_legend, cursor_target)
                            && let Some(row) = legend.rows.get(row_index)
                        {
                            shell.publish(M::from(icon_kind.into_event(row.value_id)));
                            state.drag_mode = DragMode::None;
                            state.last_cursor = None;
                            state.clear_all_caches();
                            shell.capture_event();
                            return;
                        }

                        if let Some(CursorTarget::CornerControl(kind)) = cursor_target {
                            shell.publish(M::from(kind.into_event()));
                            state.drag_mode = DragMode::None;
                            state.last_cursor = None;
                            state.clear_all_caches();
                            shell.capture_event();
                            return;
                        }

                        if matches!(cursor_target, Some(CursorTarget::Legend(_))) {
                            state.drag_mode = DragMode::None;
                            state.last_cursor = None;
                            return;
                        }

                        if let Some(CursorTarget::PanelControl { panel_index, kind }) =
                            cursor_target
                        {
                            let interaction = kind.into_interaction_kind(panel_index);

                            shell.publish(M::from(KlineWidgetEvent::PanelInteraction(interaction)));
                            state.drag_mode = DragMode::None;
                            state.last_cursor = None;
                            state.clear_all_caches();
                            shell.capture_event();
                            return;
                        }

                        let tpo_hit = if self.drawings.allows_panning()
                            && let LayoutHitZone::PanelPlot(panel_index) = zone
                            && let Some(scene) = scene.as_ref()
                            && panel_index == scene.primary.panel_index
                        {
                            prioritized_tpo_hit(
                                self.tpo_hit_test(scene, cursor_pos, state.selected_tpo_profile),
                                matches!(
                                    cursor_target,
                                    Some(
                                        CursorTarget::Drawing(_)
                                            | CursorTarget::DrawingHandle { .. }
                                    )
                                ),
                            )
                        } else {
                            None
                        };

                        if let Some(hit) = tpo_hit {
                            match hit {
                                TpoHit::Reset { .. }
                                | TpoHit::MergeLeft { .. }
                                | TpoHit::Profile { .. } => {
                                    state.pending_tpo_hit = Some(hit);
                                    state.clear_overlay_caches();
                                    if let LayoutHitZone::PanelPlot(panel_index) = zone {
                                        state.drag_mode = if matches!(hit, TpoHit::Profile { .. })
                                            && Self::is_command_modifier(state.modifiers)
                                        {
                                            DragMode::Marquee {
                                                panel_index,
                                                start: cursor_pos,
                                                current: cursor_pos,
                                            }
                                        } else {
                                            DragMode::Pan { panel_index }
                                        };
                                        state.last_cursor = Some(cursor_pos);
                                        shell.capture_event();
                                        return;
                                    }
                                }
                                TpoHit::Boundary {
                                    boundary,
                                    minimum,
                                    maximum,
                                } => {
                                    state.selected_tpo_profile = Some(boundary);
                                    state.drag_mode = DragMode::TpoBoundary {
                                        boundary,
                                        preview: boundary,
                                        minimum,
                                        maximum,
                                    };
                                    state.last_cursor = Some(cursor_pos);
                                    state.clear_overlay_caches();
                                    shell.capture_event();
                                    return;
                                }
                            }
                        } else if self.drawings.allows_panning()
                            && matches!(zone, LayoutHitZone::PanelPlot(_))
                            && state.selected_tpo_profile.take().is_some()
                        {
                            state.clear_overlay_caches();
                        }

                        if (self.drawings.measure.is_some()
                            || state.modifiers.intersects(keyboard::Modifiers::SHIFT)
                            || self.drawings.active_tool == DrawingTool::Measure)
                            && let LayoutHitZone::PanelPlot(panel_index) = zone
                        {
                            let measure_panel = self.drawings.measure.map(|m| m.state().panel_id);
                            let panel_id = measure_panel.or_else(|| self.panel_id(panel_index));

                            if let Some(panel_id) = panel_id
                                && let Some(scene) = self.cached_scene(state, layout, cursor)
                                && let Some(anchor) = self
                                    .drawing_anchor_from_scene_point(&scene, panel_id, cursor_pos)
                            {
                                shell.publish(M::from(KlineWidgetEvent::Drawing(
                                    KlineWidgetDrawingEvent::MeasureClick(anchor),
                                )));
                                state.drag_mode = DragMode::None;
                                state.last_cursor = None;
                                state.clear_all_caches();
                                shell.capture_event();
                                return;
                            }
                        }

                        if self.drawings.allows_panning()
                            && matches!(zone, LayoutHitZone::PanelPlot(_))
                            && let Some(scene) = self.cached_scene(state, layout, cursor)
                        {
                            if matches!(cursor_target, Some(CursorTarget::CornerControl(_))) {
                                state.drag_mode = DragMode::None;
                                state.last_cursor = None;
                                return;
                            }

                            if !Self::is_command_modifier(state.modifiers)
                                && let Some((id, handle_kind)) =
                                    self.hit_test_selected_drawing_handle(&scene, cursor_pos)
                                && let Some(anchor) = self.drawing_anchor_from_scene_cursor(&scene)
                            {
                                let target = DrawingDragTarget::Handle(handle_kind);

                                shell.publish(M::from(KlineWidgetEvent::Drawing(
                                    KlineWidgetDrawingEvent::DragStarted {
                                        id,
                                        target,
                                        operation: DrawingDragOperation::Move,
                                        anchor,
                                    },
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
                                let was_selected = self.drawings.selected_drawings.contains(&id);

                                if !was_selected {
                                    shell.publish(M::from(KlineWidgetEvent::Drawing(
                                        KlineWidgetDrawingEvent::Selected(Some(id)),
                                    )));
                                }

                                if let Some(anchor) = self.drawing_anchor_from_scene_cursor(&scene)
                                {
                                    let target = DrawingDragTarget::Translate;
                                    let operation = if Self::is_command_modifier(state.modifiers) {
                                        DrawingDragOperation::Copy
                                    } else {
                                        DrawingDragOperation::Move
                                    };

                                    shell.publish(M::from(KlineWidgetEvent::Drawing(
                                        KlineWidgetDrawingEvent::DragStarted {
                                            id,
                                            target,
                                            operation,
                                            anchor,
                                        },
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

                            if !Self::is_command_modifier(state.modifiers)
                                && !self.drawings.selected_drawings.is_empty()
                            {
                                shell.publish(M::from(KlineWidgetEvent::Drawing(
                                    KlineWidgetDrawingEvent::Selected(None),
                                )));
                                state.clear_all_caches();
                            }
                        }

                        if self.drawings.allows_panning()
                            && Self::is_command_modifier(state.modifiers)
                            && let LayoutHitZone::PanelPlot(panel_index) = zone
                        {
                            state.drag_mode = DragMode::Marquee {
                                panel_index,
                                start: cursor_pos,
                                current: cursor_pos,
                            };
                            state.last_cursor = Some(cursor_pos);
                            state.clear_cursor_caches();
                            shell.capture_event();
                            return;
                        }

                        if let LayoutHitZone::Splitter(split_index) = zone {
                            state.drag_mode = DragMode::Split(split_index);
                            state.last_cursor = Some(cursor_pos);
                            shell.capture_event();
                        } else if let LayoutHitZone::YAxis(panel_index) = zone {
                            let primary_panel_id = self.composition.primary_panel_id();
                            let is_primary_axis = self.panel_id(panel_index) == primary_panel_id;
                            let allows_primary_manual_zoom = primary_panel_id.is_some_and(|id| {
                                !matches!(
                                    self.composition.panel_effective_scale_mode(id),
                                    Some(PanelScaleMode::PercentFromBase)
                                )
                            });
                            if is_primary_axis && allows_primary_manual_zoom {
                                shell.publish(M::from(KlineWidgetEvent::PrimaryAutoscaleDisabled));
                            }
                            let anchor_y = layout_tree
                                .panel(panel_index)
                                .map_or(cursor_pos.y, |panel| {
                                    layout_tree.regions.plot.y + panel.plot.center().y
                                });
                            state.drag_mode = DragMode::YAxisScale {
                                panel_index,
                                anchor_y,
                            };
                            state.last_cursor = Some(cursor_pos);
                            shell.capture_event();
                        } else if matches!(
                            zone,
                            LayoutHitZone::PanelXAxis(_) | LayoutHitZone::BottomXAxis
                        ) {
                            if let Some(scene) = self.cached_scene(state, layout, cursor) {
                                state.drag_mode = DragMode::XAxisScale {
                                    anchor_x: cursor_pos.x,
                                    initial_world: self.horizontal_world,
                                    initial_right_unit: scene.effective_right_x_unit(),
                                };
                                state.last_cursor = None;
                                shell.capture_event();
                            } else {
                                state.drag_mode = DragMode::None;
                                state.last_cursor = None;
                            }
                        } else if !self.drawings.allows_panning() {
                            let magnetic = self.drawing_magnet_enabled(state.modifiers);
                            let anchor = if let Some(panel_id) = self.drawings.draft_panel_id() {
                                self.cached_scene(state, layout, cursor).and_then(|scene| {
                                    self.drawing_draft_anchor_from_scene_point(
                                        &scene, panel_id, cursor_pos, magnetic,
                                    )
                                })
                            } else if let LayoutHitZone::PanelPlot(panel_index) = zone {
                                let panel_id = self.panel_id(panel_index);

                                panel_id.and_then(|panel_id| {
                                    self.cached_scene(state, layout, cursor).and_then(|scene| {
                                        self.drawing_draft_anchor_from_scene_point(
                                            &scene, panel_id, cursor_pos, magnetic,
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
                        state.drag_start = None;
                        state.last_cursor = None;
                        state.clear_overlay_caches();
                        state.clear_all_caches();
                        shell.capture_event();
                    }
                    mouse::Event::ButtonReleased(mouse::Button::Left) => {
                        if let Some(hit) = state.pending_tpo_hit.take()
                            && state
                                .drag_start
                                .is_some_and(|start| !drag_threshold_reached(start, cursor_pos))
                        {
                            match hit {
                                TpoHit::Reset { start, end } => {
                                    shell.publish(M::from(KlineWidgetEvent::Tpo(
                                        TpoInteraction::Reset { start, end },
                                    )));
                                    state.clear_all_caches();
                                    shell.capture_event();
                                }
                                TpoHit::MergeLeft {
                                    boundary,
                                    left_start,
                                } => {
                                    state.selected_tpo_profile = Some(left_start);
                                    shell.publish(M::from(KlineWidgetEvent::Tpo(
                                        TpoInteraction::MergeLeft { boundary },
                                    )));
                                    state.clear_all_caches();
                                    shell.capture_event();
                                }
                                TpoHit::Profile { start } => {
                                    state.selected_tpo_profile = (state.selected_tpo_profile
                                        != Some(start))
                                    .then_some(start);
                                    state.drag_mode = DragMode::None;
                                    state.clear_overlay_caches();
                                    shell.capture_event();
                                }
                                TpoHit::Boundary { .. } => {}
                            }
                        }

                        if let DragMode::DrawingMove { id, .. } = state.drag_mode {
                            shell.publish(M::from(KlineWidgetEvent::Drawing(
                                KlineWidgetDrawingEvent::DragFinished { id },
                            )));
                            state.clear_overlay_caches();
                            state.clear_all_caches();
                            shell.capture_event();
                        }

                        if let DragMode::TpoBoundary {
                            boundary, preview, ..
                        } = state.drag_mode
                        {
                            state.selected_tpo_profile = Some(preview);
                            if boundary != preview {
                                shell.publish(M::from(KlineWidgetEvent::Tpo(
                                    TpoInteraction::MoveBoundary {
                                        from: boundary,
                                        to: preview,
                                    },
                                )));
                            }
                            state.clear_overlay_caches();
                            state.clear_all_caches();
                            shell.capture_event();
                        }

                        if let DragMode::Marquee {
                            panel_index,
                            start,
                            current,
                        } = state.drag_mode
                        {
                            if let Some(scene) = self.cached_scene(state, layout, cursor)
                                && self
                                    .clipped_marquee_rect(&scene, panel_index, start, current)
                                    .is_some()
                            {
                                let selected = self.marquee_selected_drawings(
                                    &scene,
                                    panel_index,
                                    start,
                                    current,
                                );
                                shell.publish(M::from(KlineWidgetEvent::Drawing(
                                    KlineWidgetDrawingEvent::MarqueeSelected(selected),
                                )));
                            }
                            state.clear_overlay_caches();
                            state.clear_all_caches();
                            shell.capture_event();
                        }

                        state.drag_mode = DragMode::None;
                        state.drag_start = None;
                        state.last_cursor = None;
                    }
                    mouse::Event::CursorMoved { .. } => {
                        state.clear_cursor_caches();

                        if let Some(start) = state.drag_start
                            && !matches!(state.drag_mode, DragMode::None)
                        {
                            if !drag_threshold_reached(start, cursor_pos) {
                                shell.capture_event();
                                return;
                            }
                            state.pending_tpo_hit = None;
                        }

                        if let Some(DrawingMeasure::Pending(measure_state)) = self.drawings.measure
                            && let Some(scene) = self.cached_scene(state, layout, cursor)
                            && let Some(anchor) = self.drawing_anchor_from_scene_point(
                                &scene,
                                measure_state.panel_id,
                                cursor_pos,
                            )
                        {
                            shell.publish(M::from(KlineWidgetEvent::Drawing(
                                KlineWidgetDrawingEvent::MeasureMove(anchor),
                            )));
                        }

                        if let DragMode::TpoBoundary {
                            boundary,
                            minimum,
                            maximum,
                            ..
                        } = state.drag_mode
                        {
                            if let Some(scene) = self.cached_scene(state, layout, cursor)
                                && let Some(preview) = self.tpo_boundary_from_cursor(
                                    &scene, cursor_pos, boundary, minimum, maximum,
                                )
                            {
                                state.drag_mode = DragMode::TpoBoundary {
                                    boundary,
                                    preview,
                                    minimum,
                                    maximum,
                                };
                                state.last_cursor = Some(cursor_pos);
                                state.clear_overlay_caches();
                                shell.capture_event();
                            }
                        } else if let DragMode::XAxisScale {
                            anchor_x,
                            initial_world,
                            initial_right_unit,
                        } = state.drag_mode
                        {
                            let total_delta = (anchor_x - cursor_pos.x).round()
                                * HORIZONTAL_DRAG_SCALE_DELTA_PER_PX;
                            if total_delta.abs() > f32::EPSILON
                                && let Some(scene) = self.cached_scene(state, layout, cursor)
                            {
                                let right_edge = scene.x_axis_plot_width().max(1.0);
                                let anchor_x =
                                    (anchor_x - scene.layout.regions.plot.x).clamp(0.0, right_edge);
                                let world = initial_world.zoom_around_screen(
                                    initial_right_unit,
                                    anchor_x,
                                    right_edge,
                                    total_delta,
                                );

                                if world != self.horizontal_world {
                                    shell.publish(M::from(
                                        KlineWidgetEvent::HorizontalWorldChanged(world),
                                    ));
                                    state.clear_all_caches();
                                    shell.capture_event();
                                }
                            }
                        } else if let DragMode::Split(split_index) = state.drag_mode {
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
                        } else if let DragMode::YAxisScale {
                            panel_index,
                            anchor_y,
                        } = state.drag_mode
                        {
                            let dy_px = state.rounded_drag_delta(cursor_pos).y;
                            let scale_delta = (-dy_px) * Y_AXIS_DRAG_SCALE_DELTA_PER_PX;

                            if scale_delta.abs() > f32::EPSILON
                                && let Some(scene) = self.cached_scene(state, layout, cursor)
                                && let Some(panel_id) = self.panel_id(panel_index)
                                && let Some(viewport) = self.panel_viewport_after_y_zoom(
                                    &scene,
                                    panel_index,
                                    anchor_y,
                                    scale_delta,
                                )
                            {
                                let interaction = PanelInteraction::YViewportChanged {
                                    panel_id,
                                    range: viewport,
                                };

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
                            if let Some(scene) = self.cached_scene(state, layout, cursor)
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
                        } else if let DragMode::Marquee {
                            panel_index, start, ..
                        } = state.drag_mode
                        {
                            state.drag_mode = DragMode::Marquee {
                                panel_index,
                                start,
                                current: cursor_pos,
                            };
                            state.last_cursor = Some(cursor_pos);
                            state.clear_cursor_caches();
                            shell.capture_event();
                        } else if let Some(draft_panel_id) = self.drawings.draft_panel_id()
                            && !matches!(
                                cursor_target,
                                Some(
                                    CursorTarget::PanelControl { .. }
                                        | CursorTarget::CornerControl(_)
                                        | CursorTarget::Legend(_)
                                )
                            )
                            && let Some(scene) = self.cached_scene(state, layout, cursor)
                            && let Some(anchor) = self.drawing_draft_anchor_from_scene_point(
                                &scene,
                                draft_panel_id,
                                cursor_pos,
                                self.drawing_magnet_enabled(state.modifiers),
                            )
                        {
                            shell.publish(M::from(KlineWidgetEvent::Drawing(
                                KlineWidgetDrawingEvent::AnchorMoved(anchor),
                            )));
                            state.last_cursor = Some(cursor_pos);
                            state.clear_cursor_caches();
                            shell.capture_event();
                        } else if let DragMode::Pan { panel_index } = state.drag_mode {
                            if Self::hit_corner_control(corner_controls, cursor_pos).is_some() {
                                state.last_cursor = Some(cursor_pos);
                                return;
                            }

                            let drag_delta = state.rounded_drag_delta(cursor_pos);
                            let dx_px = drag_delta.x;
                            let dy_px = drag_delta.y;
                            let spacing = self.horizontal_world.spacing();
                            let offset = self.horizontal_world.offset() - dx_px / spacing;

                            if (offset - self.horizontal_world.offset()).abs() > f32::EPSILON {
                                shell.publish(M::from(KlineWidgetEvent::HorizontalOffsetChanged(
                                    offset,
                                )));
                                state.clear_all_caches();
                            }

                            if dy_px.abs() > f32::EPSILON
                                && let Some(scene) = self.cached_scene(state, layout, cursor)
                                && let Some(panel_id) = self.panel_id(panel_index)
                                && let Some(viewport) = self.panel_viewport_after_primary_y_pan(
                                    &scene,
                                    panel_index,
                                    dy_px,
                                )
                            {
                                let interaction = PanelInteraction::YViewportChanged {
                                    panel_id,
                                    range: viewport,
                                };

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
            Event::Window(window::Event::RedrawRequested(now)) => {
                let state = tree.state.downcast_mut::<State>();
                if let Some(layout_tree) =
                    PanelLayoutTree::from_layout(layout, &self.composition.panels)
                {
                    state.sync_geometry_cache_keys(
                        layout.bounds(),
                        layout_tree.regions,
                        self.pixel_ratio,
                    );
                }
                if state.last_theme_generation != Some(self.theme_accents.generation()) {
                    state.clear_all_caches();
                    state.last_theme_generation = Some(self.theme_accents.generation());
                }

                if let Some(refresh_at) =
                    state.cache_refresh_at(self.version, *now, MIN_FULL_CACHE_REFRESH_INTERVAL)
                {
                    if *now >= refresh_at {
                        state.clear_all_caches();
                        state.mark_cache_refreshed(self.version, *now);
                    } else {
                        shell.request_redraw_at(refresh_at);
                    }
                }

                if let Some(scene) = self.cached_scene(state, layout, cursor) {
                    for (panel_id, range) in self.reconciled_y_ranges(&scene) {
                        shell.publish(M::from(KlineWidgetEvent::PanelInteraction(
                            PanelInteraction::YViewportChanged { panel_id, range },
                        )));
                    }
                }

                if self.basis.is_time() {
                    let second = chrono::Utc::now().timestamp();
                    if state.last_countdown_second != Some(second) {
                        state.interaction_text_cache.clear();
                        state.last_countdown_second = Some(second);
                    }
                }

                if let Some(plot_width) =
                    PanelLayoutTree::from_layout(layout, &self.composition.panels)
                        .and_then(|layout| layout.primary_plot_width())
                    && self.known_plot_width != Some(plot_width)
                {
                    shell.publish(M::from(KlineWidgetEvent::PlotWidthChanged(plot_width)));
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
        let state = tree.state.downcast_ref::<State>();
        let Some(scene) = self.cached_scene(state, layout, cursor) else {
            return;
        };
        let bounds = layout.bounds();

        use advanced::Renderer as _;
        use iced::advanced::graphics::geometry::Renderer as _;
        renderer.with_translation(Vector::new(bounds.x, bounds.y), |renderer| {
            let plot_rect = scene.plot_rect();
            let palette = theme.extended_palette();
            let generated_accents = self.theme_accents.as_array();

            for step in PAINT_ORDER {
                match step {
                    PaintStep::Root(pass) => match pass {
                        RootPaintStep::Separators => {
                            Self::paint_separators(renderer, &scene, theme);
                        }
                        RootPaintStep::Drawings => {
                            let overlay_pixels = PixelGeometry::new(
                                self.pixel_ratio,
                                Point::new(bounds.x, bounds.y),
                            );
                            let geometry =
                                state.drawing_cache.draw(renderer, bounds.size(), |frame| {
                                    if self.drawings.has_state() {
                                        self.fill_drawing_objects(
                                            frame,
                                            &scene,
                                            palette,
                                            generated_accents,
                                            overlay_pixels,
                                        );
                                    }
                                });
                            renderer.draw_geometry(geometry);
                        }
                        RootPaintStep::YAxis => {
                            let rect = scene.layout.regions.y_axis;
                            let geometry =
                                state.y_axis_cache.draw(renderer, rect.size(), |frame| {
                                    self.fill_y_axis_labels(frame, &scene, palette)
                                });
                            renderer.with_translation(Vector::new(rect.x, rect.y), |renderer| {
                                renderer.draw_geometry(geometry);
                            });
                        }
                        RootPaintStep::XAxis => {
                            let rect = scene.layout.regions.x_axis;
                            let x_axis_pixels = PixelGeometry::new(
                                self.pixel_ratio,
                                Point::new(bounds.x + rect.x, bounds.y + rect.y),
                            );
                            let geometry =
                                state.x_axis_cache.draw(renderer, rect.size(), |frame| {
                                    self.fill_x_axis_labels(frame, &scene, palette, x_axis_pixels)
                                });
                            renderer.with_translation(Vector::new(rect.x, rect.y), |renderer| {
                                renderer.draw_geometry(geometry);
                            });
                        }
                    },
                    PaintStep::Plot(layer) => {
                        let cache = match layer {
                            PrimaryPlotLayer::Background => &state.plot_background_cache,
                            PrimaryPlotLayer::Tpo => &state.tpo_cache,
                            PrimaryPlotLayer::Foreground => &state.plot_cache,
                            PrimaryPlotLayer::PriceBars => &state.price_bars_cache,
                        };
                        let plot_pixels = PixelGeometry::new(
                            self.pixel_ratio,
                            Point::new(bounds.x + plot_rect.x, bounds.y + plot_rect.y),
                        );
                        let geometry = cache.draw(renderer, plot_rect.size(), |frame| {
                            self.fill_main_geometry(
                                frame,
                                &scene,
                                palette,
                                generated_accents,
                                plot_pixels,
                                layer,
                            );
                        });
                        renderer.with_layer(plot_rect, |renderer| {
                            renderer.with_translation(
                                Vector::new(plot_rect.x, plot_rect.y),
                                |renderer| {
                                    renderer.draw_geometry(geometry);
                                },
                            );
                        });
                    }
                    PaintStep::Widget(layer) => {
                        let overlay_pixels =
                            PixelGeometry::new(self.pixel_ratio, Point::new(bounds.x, bounds.y));
                        let drawing_magnet = self.drawing_magnet_enabled(state.modifiers);
                        let geometry = match layer {
                            WidgetPaintStep::Overlay => {
                                state.overlay_cache.draw(renderer, bounds.size(), |frame| {
                                    self.fill_overlay(
                                        frame,
                                        &scene,
                                        palette,
                                        generated_accents,
                                        overlay_pixels,
                                        drawing_magnet,
                                        state.selected_tpo_profile,
                                        match state.drag_mode {
                                            DragMode::TpoBoundary { preview, .. } => Some(preview),
                                            _ => None,
                                        },
                                    );
                                })
                            }
                            WidgetPaintStep::InteractionText => state.interaction_text_cache.draw(
                                renderer,
                                bounds.size(),
                                |frame| {
                                    self.fill_overlay_interaction_text(
                                        frame,
                                        &scene,
                                        palette,
                                        drawing_magnet,
                                        overlay_pixels,
                                    );
                                },
                            ),
                            WidgetPaintStep::Marquee => {
                                state.marquee_cache.draw(renderer, bounds.size(), |frame| {
                                    if let DragMode::Marquee {
                                        panel_index,
                                        start,
                                        current,
                                    } = &state.drag_mode
                                        && let Some(rect) = self.clipped_marquee_rect(
                                            &scene,
                                            *panel_index,
                                            *start,
                                            *current,
                                        )
                                    {
                                        self.fill_marquee_rectangle(frame, rect, palette);
                                    }
                                })
                            }
                        };
                        renderer.with_layer(Rectangle::with_size(bounds.size()), |renderer| {
                            renderer.draw_geometry(geometry);
                        });
                    }
                }
            }
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
        let state = tree.state.downcast_ref::<State>();

        if matches!(
            state.drag_mode,
            DragMode::Split(_) | DragMode::YAxisScale { .. }
        ) {
            return advanced::mouse::Interaction::ResizingVertically;
        }

        if matches!(state.drag_mode, DragMode::TpoBoundary { .. }) {
            return advanced::mouse::Interaction::ResizingHorizontally;
        }

        if matches!(state.drag_mode, DragMode::XAxisScale { .. }) {
            return advanced::mouse::Interaction::ResizingHorizontally;
        }

        if matches!(state.drag_mode, DragMode::Pan { .. }) {
            return advanced::mouse::Interaction::Grabbing;
        }

        if matches!(state.drag_mode, DragMode::DrawingMove { .. }) {
            return advanced::mouse::Interaction::Grabbing;
        }

        let scene = self.cached_scene(state, layout, cursor);
        if self.drawings.allows_panning()
            && let Some(scene) = scene.as_ref()
            && let Some(point) = cursor.position_in(layout.bounds())
            && let Some(hit) = prioritized_tpo_hit(
                self.tpo_hit_test(scene, point, state.selected_tpo_profile),
                matches!(
                    scene.interaction.target,
                    CursorTarget::Drawing(_) | CursorTarget::DrawingHandle { .. }
                ),
            )
        {
            return match hit {
                TpoHit::Boundary { .. } => advanced::mouse::Interaction::ResizingHorizontally,
                TpoHit::Reset { .. } | TpoHit::MergeLeft { .. } | TpoHit::Profile { .. } => {
                    advanced::mouse::Interaction::Pointer
                }
            };
        }
        let Some(target) = scene.map(|scene| scene.interaction.target) else {
            return advanced::mouse::Interaction::default();
        };

        match target {
            CursorTarget::Legend(target) if target.has_icon() => {
                advanced::mouse::Interaction::Pointer
            }
            CursorTarget::Legend(_)
            | CursorTarget::Outside
            | CursorTarget::ActiveDrag
            | CursorTarget::ActivePan => advanced::mouse::Interaction::default(),
            CursorTarget::PanelControl { .. } | CursorTarget::CornerControl(_) => {
                advanced::mouse::Interaction::Pointer
            }
            CursorTarget::DrawingHandle { .. } => advanced::mouse::Interaction::Grab,
            CursorTarget::Drawing(id) => {
                if self.drawings.selected_drawings.contains(&id) {
                    advanced::mouse::Interaction::Grab
                } else {
                    advanced::mouse::Interaction::Pointer
                }
            }
            CursorTarget::Splitter(_) | CursorTarget::YAxis(_) => {
                advanced::mouse::Interaction::ResizingVertically
            }
            CursorTarget::PanelXAxis(_) | CursorTarget::BottomXAxis => {
                advanced::mouse::Interaction::ResizingHorizontally
            }
            CursorTarget::Plot(_) => advanced::mouse::Interaction::Crosshair,
        }
    }
}

fn history_shortcut(
    key: keyboard::Key<&str>,
    modifiers: keyboard::Modifiers,
) -> Option<KlineWidgetEvent> {
    let keyboard::Key::Character(character) = key else {
        return None;
    };
    if !character.eq_ignore_ascii_case("z") || !modifiers.contains(keyboard::Modifiers::COMMAND) {
        return None;
    }

    Some(if modifiers.shift() {
        KlineWidgetEvent::RedoRequested
    } else {
        KlineWidgetEvent::UndoRequested
    })
}

fn prioritized_tpo_hit(hit: Option<TpoHit>, drawing_target: bool) -> Option<TpoHit> {
    hit.filter(|hit| !drawing_target || !matches!(hit, TpoHit::Profile { .. }))
}

fn drag_threshold_reached(start: Point, current: Point) -> bool {
    (start.x - current.x).abs() + (start.y - current.y).abs() >= DRAG_THRESHOLD_PX
}

fn primary_scale_inversion_shortcut(
    physical_key: keyboard::key::Physical,
    modifiers: keyboard::Modifiers,
) -> bool {
    modifiers.alt() && physical_key == keyboard::key::Code::KeyI
}

fn point_in_rectangle(point: Point, bounds: Rectangle) -> bool {
    point.x >= bounds.x
        && point.x <= bounds.x + bounds.width
        && point.y >= bounds.y
        && point.y <= bounds.y + bounds.height
}

impl<'a, M> From<KlineWidget<'a>> for Element<'a, M, Theme, Renderer>
where
    M: Clone + 'a + 'static + From<KlineWidgetEvent>,
{
    fn from(chart: KlineWidget<'a>) -> Self {
        Self::new(chart)
    }
}
