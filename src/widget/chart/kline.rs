mod axes;
mod chrome;
pub(crate) mod composition;
pub(crate) mod drawing;
mod footprint;
mod helpers;
mod interaction;
mod layout;
mod overlay;
mod presentation;
mod primary_render;
mod scene;
mod scene_builder;
pub(crate) mod studies;

pub(crate) use chrome::{StudiesLegend, StudyLegendRow};

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
    FootprintPocColorSlot, KlineStudyId, LegendVisibility,
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
    AxisBadgeFilter, DrawingDrag, DrawingMeasure, DrawingSnapshot, DrawingTool, DrawingView,
    KlineWidgetDrawingEvent, WidgetDrawingState,
};
use exchange::TickerInfo;
use helpers::{ChartCoordinates, PixelGeometry};
use interaction::{
    CacheInvalidation, LayerContext, LayerEffects, LayerGesture, NavigationFallback, PointerMove,
    ReleasedGesture, UserInteraction,
};
use layout::{KlinePanelKind, LayoutHitZone, PanelLayoutNode, PanelLayoutTree};
pub(crate) use presentation::IndicatorPresentationStatus;
pub(crate) use presentation::{
    IndicatorBarValues, IndicatorPresentationContext, ResolvedIndicatorLayer,
    ResolvedIndicatorPresentation, ResolvedIndicatorStyleSlot, resolve_indicator_bar_values,
    resolve_indicator_presentation_status,
};
use primary_render::PrimaryPlotLayer;
use scene::{CursorTarget, LegendTarget, Scene, SceneCache, TickIndex, XAxis};
use studies::{
    LayerPaintPhase, LayerPointerInput, StudyObject, StudyObjectEvent, StudyObjectKey, StudyStates,
};

use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{self, Clipboard, Layout, Shell, Widget, layout as iced_layout, renderer};
use iced::theme::palette::Extended;
use iced::widget::canvas;
use iced::{
    Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector, keyboard, mouse,
    window,
};
use iced_core::renderer::Quad;
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
const PANEL_TITLE_TOP_PAD: f32 = 4.0;
const Y_AXIS_DRAG_SCALE_DELTA_PER_PX: f32 = 0.1;

const TICKER_LEGEND_PADDING: f32 = 4.0;
const TOOLTIP_ROW_LEFT_PAD: f32 = TICKER_LEGEND_PADDING * 2.0;
const TICKER_LEGEND_ROW_H: f32 = TEXT_SIZE + 6.0;

/// The order used by [`KlineWidget::draw`] to compose chart render passes.
///
/// # Note
///
/// Keep plot, widget, and legend passes separate because `iced_wgpu` renders
/// meshes before text within each renderer layer.
const PAINT_ORDER: [PaintStep; 12] = [
    PaintStep::Root(RootPaintStep::Separators),
    PaintStep::Plot(PrimaryPlotLayer::Background),
    PaintStep::Plot(PrimaryPlotLayer::Objects),
    PaintStep::Plot(PrimaryPlotLayer::Foreground),
    PaintStep::Root(RootPaintStep::Objects),
    PaintStep::Plot(PrimaryPlotLayer::PriceBars),
    PaintStep::Root(RootPaintStep::YAxis),
    PaintStep::Root(RootPaintStep::XAxis),
    PaintStep::Widget(WidgetPaintStep::Overlay),
    PaintStep::Widget(WidgetPaintStep::InteractionText),
    PaintStep::Widget(WidgetPaintStep::Marquee),
    PaintStep::Widget(WidgetPaintStep::Legends),
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
    Objects,
    YAxis,
    XAxis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WidgetPaintStep {
    Overlay,
    InteractionText,
    Marquee,
    Legends,
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

#[derive(Debug, Clone, Copy)]
enum PressAction {
    Ordinary,
    DrawingPlacement,
    Drawing {
        selection: SelectionDisposition,
        captures_outside_release: bool,
    },
    Layer {
        key: StudyObjectKey,
        selection: SelectionDisposition,
        captures_outside_release: bool,
    },
}

impl PressAction {
    fn selection_disposition(self) -> SelectionDisposition {
        match self {
            Self::DrawingPlacement => SelectionDisposition::Preserve,
            Self::Drawing { selection, .. } => selection,
            Self::Ordinary => SelectionDisposition::Clear,
            Self::Layer { selection, .. } => selection,
        }
    }

    fn captures_outside_release(self) -> bool {
        match self {
            Self::DrawingPlacement => true,
            Self::Drawing {
                captures_outside_release,
                ..
            } => captures_outside_release,
            Self::Layer {
                captures_outside_release,
                ..
            } => captures_outside_release,
            Self::Ordinary => false,
        }
    }

    fn layer_key(self) -> Option<StudyObjectKey> {
        match self {
            Self::Layer { key, .. } => Some(key),
            Self::Ordinary | Self::DrawingPlacement | Self::Drawing { .. } => None,
        }
    }

    fn is_drawing(self) -> bool {
        matches!(self, Self::Drawing { .. })
    }
}

#[derive(Debug, Clone, Copy)]
enum DragAction {
    Split(usize),
    Layer {
        key: StudyObjectKey,
        selection: SelectionDisposition,
        active_outside_bounds: bool,
        cursor: advanced::mouse::Interaction,
    },
    Drawing {
        selection: SelectionDisposition,
        active_outside_bounds: bool,
        cursor: advanced::mouse::Interaction,
    },
    YAxisScale {
        panel_index: usize,
        anchor_y: f32,
    },
    XAxisScale {
        initial_world: HorizontalWorld,
        initial_right_unit: f32,
    },
    Pan {
        panel_index: usize,
        source_layer: Option<StudyObjectKey>,
    },
    Marquee {
        panel_index: usize,
        source_layer: Option<StudyObjectKey>,
    },
}

#[derive(Debug, Clone, Copy)]
enum DrawingPointerInput {
    Moved,
    DragReleased,
    Cancelled,
    SelectionCleared,
}

impl DragAction {
    fn scene_activity(self) -> SceneActivity {
        match self {
            Self::Pan { .. } => SceneActivity::Pan,
            _ => SceneActivity::Drag,
        }
    }

    fn active_outside_bounds(self) -> bool {
        match self {
            Self::Layer {
                active_outside_bounds,
                ..
            }
            | Self::Drawing {
                active_outside_bounds,
                ..
            } => active_outside_bounds,
            _ => matches!(
                self,
                Self::YAxisScale { .. } | Self::XAxisScale { .. } | Self::Marquee { .. }
            ),
        }
    }

    fn selection_disposition(self) -> SelectionDisposition {
        match self {
            Self::Layer { selection, .. } | Self::Drawing { selection, .. } => selection,
            Self::Marquee { .. } => SelectionDisposition::Preserve,
            _ => SelectionDisposition::Clear,
        }
    }

    fn cursor_appearance(self) -> Option<advanced::mouse::Interaction> {
        match self {
            Self::Split(_) | Self::YAxisScale { .. } => {
                Some(advanced::mouse::Interaction::ResizingVertically)
            }
            Self::Layer { cursor, .. } | Self::Drawing { cursor, .. } => Some(cursor),
            Self::XAxisScale { .. } => Some(advanced::mouse::Interaction::ResizingHorizontally),
            Self::Pan { .. } => Some(advanced::mouse::Interaction::Grabbing),
            Self::Marquee { .. } => None,
        }
    }

    fn layer_key(self) -> Option<StudyObjectKey> {
        match self {
            Self::Layer { key, .. } => Some(key),
            Self::Pan { source_layer, .. } | Self::Marquee { source_layer, .. } => source_layer,
            _ => None,
        }
    }

    fn is_drawing(self) -> bool {
        matches!(self, Self::Drawing { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SceneActivity {
    Idle,
    Pan,
    Drag,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectionDisposition {
    Preserve,
    Clear,
}

struct MarqueeGesture {
    panel_index: usize,
    start: Point,
    current: Point,
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
    price_bars_cache: canvas::Cache,
    y_axis_cache: canvas::Cache,
    x_axis_cache: canvas::Cache,
    overlay_cache: canvas::Cache,
    legend_cache: canvas::Cache,
    interaction_text_cache: canvas::Cache,
    marquee_cache: canvas::Cache,
    study_underlay_cache: canvas::Cache,
    drawing_cache: canvas::Cache,
    interaction: UserInteraction<PressAction, DragAction>,
    study_objects: StudyStates,
    drawing: WidgetDrawingState,
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
    scene_cache: SceneCache,
}

impl Default for State {
    fn default() -> Self {
        Self {
            plot_background_cache: canvas::Cache::new(),
            plot_cache: canvas::Cache::new(),
            price_bars_cache: canvas::Cache::new(),
            y_axis_cache: canvas::Cache::new(),
            x_axis_cache: canvas::Cache::new(),
            overlay_cache: canvas::Cache::new(),
            legend_cache: canvas::Cache::new(),
            interaction_text_cache: canvas::Cache::new(),
            marquee_cache: canvas::Cache::new(),
            study_underlay_cache: canvas::Cache::new(),
            drawing_cache: canvas::Cache::new(),
            interaction: UserInteraction::default(),
            study_objects: StudyStates::default(),
            drawing: WidgetDrawingState::default(),
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
            scene_cache: SceneCache::default(),
        }
    }
}

impl State {
    fn apply_interaction_effects<M, E>(
        &mut self,
        effects: LayerEffects<E>,
        shell: &mut Shell<'_, M>,
        map_event: impl Fn(E) -> KlineWidgetEvent,
    ) -> (bool, Option<LayerGesture>)
    where
        M: Clone + From<KlineWidgetEvent>,
    {
        for event in effects.events {
            shell.publish(M::from(map_event(event)));
        }
        match effects.invalidation {
            CacheInvalidation::None => {}
            CacheInvalidation::Overlay => self.clear_overlay_caches(),
            CacheInvalidation::All => self.clear_all_caches(),
        }
        let captured = effects.status == iced::event::Status::Captured;
        if captured {
            shell.capture_event();
        }
        (captured, effects.gesture)
    }

    fn scene_activity(&self) -> SceneActivity {
        self.interaction
            .drag_action()
            .map_or(SceneActivity::Idle, DragAction::scene_activity)
    }

    fn marquee(&self) -> Option<MarqueeGesture> {
        let DragAction::Marquee { panel_index, .. } = self.interaction.drag_action()? else {
            return None;
        };
        let motion = self.interaction.motion()?;
        Some(MarqueeGesture {
            panel_index,
            start: motion.origin,
            current: motion.current,
        })
    }

    fn clear_all_caches(&mut self) {
        self.plot_background_cache.clear();
        self.plot_cache.clear();
        self.study_underlay_cache.clear();
        self.drawing_cache.clear();
        self.price_bars_cache.clear();
        self.y_axis_cache.clear();
        self.x_axis_cache.clear();
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
        self.legend_cache.clear();
        self.interaction_text_cache.clear();
    }

    fn clear_cursor_caches(&mut self) {
        self.clear_overlay_caches();

        if self.marquee().is_some() {
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
            self.study_underlay_cache.clear();
            self.price_bars_cache.clear();
            self.plot_geometry_key = Some(plot_geometry_key);
        }
        if self.drawing_geometry_key
            != Some(GeometryCacheKey::new(bounds, root_origin, pixel_ratio))
        {
            self.drawing_cache.clear();
            self.drawing_geometry_key =
                Some(GeometryCacheKey::new(bounds, root_origin, pixel_ratio));
        }
        let overlay_geometry_key = GeometryCacheKey::new(bounds, root_origin, pixel_ratio);
        if self.overlay_geometry_key != Some(overlay_geometry_key) {
            self.clear_overlay_caches();
            self.overlay_geometry_key = Some(overlay_geometry_key);
        }
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
    Drawing(KlineWidgetDrawingEvent),
    Study(StudyObjectEvent),
    StudySettings(KlineStudyId),
    StudyVisibilityToggled(KlineStudyId),
    StudyRemove(KlineStudyId),
    StudiesVisibilityToggled,
    StudiesExpansionToggled,
    StudiesRemove,
    TickerSettings(TickerInfo),
    FootprintSettings,
    TickerRemove(TickerInfo),
    CandlesticksVisibilityToggled,
    FootprintsVisibilityToggled,
    IndicatorSettings(PanelValueId),
    IndicatorVisibilityToggled(PanelValueId),
    IndicatorRemove(PanelValueId),
    XAxisDoubleClick,
    DeletePressed,
}

impl From<KlineWidgetDrawingEvent> for KlineWidgetEvent {
    fn from(event: KlineWidgetDrawingEvent) -> Self {
        Self::Drawing(event)
    }
}

pub struct KlineWidget<'a> {
    series: &'a [KlineSeries],
    indicator_host: &'a IndicatorHost,
    composition: &'a ChartComposition,
    indicator_instances: &'a IndicatorInstanceStore,
    drawing_snapshot: DrawingSnapshot<'a>,
    study_objects: Vec<StudyObject<'a>>,
    study_legend: Option<chrome::StudiesLegend>,
    legend_visibility: LegendVisibility,
    candlestick: CandlestickConfig,
    footprint: FootprintConfig,
    trade_data_enabled: bool,
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
    ) -> Self {
        Self {
            series,
            indicator_host,
            composition,
            indicator_instances,
            drawing_snapshot: DrawingSnapshot::new(
                DrawingTool::Cursor,
                false,
                &[],
                &[],
                None,
                None,
            ),
            study_objects: Vec::new(),
            study_legend: None,
            legend_visibility: LegendVisibility::default(),
            candlestick: CandlestickConfig::default(),
            footprint: FootprintConfig::default(),
            trade_data_enabled: false,
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

    pub(crate) fn with_study_legend(mut self, legend: chrome::StudiesLegend) -> Self {
        self.study_legend = Some(legend);
        self
    }

    pub fn with_drawing_snapshot(mut self, drawing_snapshot: DrawingSnapshot<'a>) -> Self {
        self.drawing_snapshot = drawing_snapshot;
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

    fn coordinates(&self) -> ChartCoordinates<'_> {
        ChartCoordinates {
            series: self.series,
            composition: self.composition,
            basis: self.basis,
            timezone: self.timezone,
        }
    }

    fn layer_context<'b>(
        &'b self,
        scene: &'b Scene,
        position: Option<Point>,
        modifiers: keyboard::Modifiers,
    ) -> LayerContext<'b> {
        LayerContext {
            scene,
            coordinates: self.coordinates(),
            position,
            modifiers,
        }
    }

    fn drawing_view<'b>(&'b self, scene: &'b Scene) -> DrawingView<'b> {
        self.drawing_view_at(scene, None, keyboard::Modifiers::empty())
    }

    fn drawing_view_at<'b>(
        &'b self,
        scene: &'b Scene,
        position: Option<Point>,
        modifiers: keyboard::Modifiers,
    ) -> DrawingView<'b> {
        DrawingView::new(
            self.drawing_snapshot,
            self.layer_context(scene, position, modifiers),
        )
    }

    fn begin_study_object_interaction<M>(
        &self,
        key: StudyObjectKey,
        scene: &Scene,
        position: Point,
        state: &mut State,
        shell: &mut Shell<'_, M>,
    ) -> bool
    where
        M: Clone + From<KlineWidgetEvent>,
    {
        let Some(effects) = self.study_object_pointer(
            key,
            LayerPointerInput::Pressed,
            self.layer_context(scene, Some(position), state.interaction.modifiers()),
            &mut state.study_objects,
        ) else {
            return false;
        };
        let (_, gesture) = state.apply_interaction_effects(effects, shell, KlineWidgetEvent::Study);
        let Some(gesture) = gesture else { return false };
        match gesture {
            LayerGesture::Click {
                selection,
                captures_outside_release,
            } => state.interaction.begin_click(
                position,
                PressAction::Layer {
                    key,
                    selection,
                    captures_outside_release,
                },
            ),
            LayerGesture::Drag {
                selection,
                active_outside_bounds,
                cursor,
            } => state.interaction.begin_drag_candidate(
                position,
                DragAction::Layer {
                    key,
                    selection,
                    active_outside_bounds,
                    cursor,
                },
            ),
            LayerGesture::ClickOrNavigate {
                selection,
                captures_outside_release,
                fallback,
            } => {
                let LayoutHitZone::PanelPlot(panel_index) = scene.layout.hit_test(position) else {
                    return false;
                };
                let fallback = match fallback {
                    NavigationFallback::PanOrMarquee
                        if Self::is_command_modifier(state.interaction.modifiers()) =>
                    {
                        DragAction::Marquee {
                            panel_index,
                            source_layer: Some(key),
                        }
                    }
                    _ => DragAction::Pan {
                        panel_index,
                        source_layer: Some(key),
                    },
                };
                state.interaction.begin_click_or_drag(
                    position,
                    PressAction::Layer {
                        key,
                        selection,
                        captures_outside_release,
                    },
                    fallback,
                );
            }
        }
        true
    }

    fn begin_drawing_interaction<M>(
        &self,
        scene: &Scene,
        position: Point,
        state: &mut State,
        shell: &mut Shell<'_, M>,
    ) -> bool
    where
        M: Clone + From<KlineWidgetEvent>,
    {
        let view = self.drawing_view_at(scene, Some(position), state.interaction.modifiers());
        let Some(hit) = view.hit(position) else {
            return false;
        };
        let effects = view.begin(hit, &mut state.drawing);
        let (_, gesture) =
            state.apply_interaction_effects(effects, shell, KlineWidgetEvent::Drawing);
        let Some(gesture) = gesture else { return false };
        match gesture {
            LayerGesture::Click {
                selection,
                captures_outside_release,
            } => state.interaction.begin_click(
                position,
                PressAction::Drawing {
                    selection,
                    captures_outside_release,
                },
            ),
            LayerGesture::Drag {
                selection,
                active_outside_bounds,
                cursor,
            } => state.interaction.begin_drag_candidate(
                position,
                DragAction::Drawing {
                    selection,
                    active_outside_bounds,
                    cursor,
                },
            ),
            LayerGesture::ClickOrNavigate {
                selection,
                captures_outside_release,
                fallback,
            } => {
                let LayoutHitZone::PanelPlot(panel_index) = scene.layout.hit_test(position) else {
                    return false;
                };
                let fallback = match fallback {
                    NavigationFallback::PanOrMarquee
                        if Self::is_command_modifier(state.interaction.modifiers()) =>
                    {
                        DragAction::Marquee {
                            panel_index,
                            source_layer: None,
                        }
                    }
                    _ => DragAction::Pan {
                        panel_index,
                        source_layer: None,
                    },
                };
                state.interaction.begin_click_or_drag(
                    position,
                    PressAction::Drawing {
                        selection,
                        captures_outside_release,
                    },
                    fallback,
                );
            }
        }
        true
    }

    fn is_command_modifier(modifiers: keyboard::Modifiers) -> bool {
        modifiers.intersects(keyboard::Modifiers::COMMAND)
    }

    fn drawing_magnet_enabled(&self, modifiers: keyboard::Modifiers) -> bool {
        self.drawing_snapshot.active_tool.supports_magnet()
            && (self.drawing_snapshot.magnet_enabled || Self::is_command_modifier(modifiers))
    }

    fn apply_study_object_pointer<M>(
        &self,
        key: StudyObjectKey,
        input: LayerPointerInput,
        context: LayerContext<'_>,
        state: &mut State,
        shell: &mut Shell<'_, M>,
    ) -> bool
    where
        M: Clone + From<KlineWidgetEvent>,
    {
        self.study_object_pointer(key, input, context, &mut state.study_objects)
            .is_some_and(|effects| {
                state
                    .apply_interaction_effects(effects, shell, KlineWidgetEvent::Study)
                    .0
            })
    }

    fn broadcast_study_object_pointer<M>(
        &self,
        input: LayerPointerInput,
        context: LayerContext<'_>,
        state: &mut State,
        shell: &mut Shell<'_, M>,
    ) -> bool
    where
        M: Clone + From<KlineWidgetEvent>,
    {
        let effects = self.study_objects_pointer_all(input, context, &mut state.study_objects);
        let mut captured = false;
        for effects in effects {
            captured |= state
                .apply_interaction_effects(effects, shell, KlineWidgetEvent::Study)
                .0;
        }
        captured
    }

    fn apply_drawing_pointer<M>(
        &self,
        input: DrawingPointerInput,
        context: LayerContext<'_>,
        state: &mut State,
        shell: &mut Shell<'_, M>,
    ) -> bool
    where
        M: Clone + From<KlineWidgetEvent>,
    {
        let view = self.drawing_view_at(context.scene, context.position, context.modifiers);
        let effects = match input {
            DrawingPointerInput::Moved => state
                .drawing
                .drag
                .map_or_else(LayerEffects::default, |drag| view.update_drag(drag)),
            DrawingPointerInput::DragReleased => state
                .drawing
                .drag
                .take()
                .map_or_else(LayerEffects::default, DrawingDrag::release),
            DrawingPointerInput::Cancelled => {
                state.drawing.drag = None;
                LayerEffects::default()
            }
            DrawingPointerInput::SelectionCleared => view.deselect(),
        };
        state
            .apply_interaction_effects(effects, shell, KlineWidgetEvent::Drawing)
            .0
    }

    fn cancel_pointer<M>(
        &self,
        state: &mut State,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        shell: &mut Shell<'_, M>,
    ) where
        M: Clone + From<KlineWidgetEvent>,
    {
        let drag = state.interaction.drag_action();
        let press = state.interaction.cancel_pointer();
        let drawing_active =
            press.is_some_and(PressAction::is_drawing) || drag.is_some_and(DragAction::is_drawing);
        let key = press
            .and_then(PressAction::layer_key)
            .or_else(|| drag.and_then(DragAction::layer_key));
        if let (Some(key), Some(scene)) = (key, self.cached_scene(state, layout, cursor)) {
            self.apply_study_object_pointer(
                key,
                LayerPointerInput::Cancelled,
                self.layer_context(
                    &scene,
                    cursor.position_in(layout.bounds()),
                    state.interaction.modifiers(),
                ),
                state,
                shell,
            );
        }
        if drawing_active && let Some(scene) = self.cached_scene(state, layout, cursor) {
            self.apply_drawing_pointer(
                DrawingPointerInput::Cancelled,
                self.layer_context(
                    &scene,
                    cursor.position_in(layout.bounds()),
                    state.interaction.modifiers(),
                ),
                state,
                shell,
            );
        }
    }

    fn finish_interaction_release<M>(
        &self,
        state: &mut State,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        outside_bounds: bool,
        shell: &mut Shell<'_, M>,
    ) where
        M: Clone + 'static + From<KlineWidgetEvent>,
    {
        let position = (!outside_bounds)
            .then(|| cursor.position_in(layout.bounds()))
            .flatten();
        let outcome = state.interaction.release(position);
        let (selection, had_gesture) = match outcome {
            ReleasedGesture::Idle => (SelectionDisposition::Clear, false),
            ReleasedGesture::Pressed { action, .. } => (
                action.selection_disposition(),
                action.captures_outside_release(),
            ),
            ReleasedGesture::Drag { action, .. } => (action.selection_disposition(), true),
        };
        let mut capture = outside_bounds && had_gesture;

        let scene = self.cached_scene(state, layout, cursor);
        let layer_context = scene
            .as_ref()
            .map(|scene| self.layer_context(scene, position, state.interaction.modifiers()));

        if selection == SelectionDisposition::Clear
            && let Some(context) = layer_context
        {
            capture |= self.apply_drawing_pointer(
                DrawingPointerInput::SelectionCleared,
                context,
                state,
                shell,
            );
        }

        match outcome {
            ReleasedGesture::Pressed {
                action: PressAction::Layer { key, .. },
                click,
            } => {
                if let Some(context) = layer_context {
                    capture |= self.apply_study_object_pointer(
                        key,
                        LayerPointerInput::Clicked {
                            confirmed: click == interaction::ClickOutcome::Confirmed,
                        },
                        context,
                        state,
                        shell,
                    );
                }
            }
            ReleasedGesture::Drag {
                action: DragAction::Layer { key, .. },
                ..
            } => {
                if let Some(context) = layer_context {
                    capture |= self.apply_study_object_pointer(
                        key,
                        LayerPointerInput::DragReleased,
                        context,
                        state,
                        shell,
                    );
                }
            }
            ReleasedGesture::Drag {
                action: DragAction::Drawing { .. },
                ..
            } => {
                if let Some(context) = layer_context {
                    capture |= self.apply_drawing_pointer(
                        DrawingPointerInput::DragReleased,
                        context,
                        state,
                        shell,
                    );
                }
            }
            ReleasedGesture::Drag {
                action: DragAction::Marquee { panel_index, .. },
                origin,
                position: Some(current),
                press_action,
            } => {
                if let Some(scene) = scene.as_ref()
                    && self
                        .drawing_view(scene)
                        .clipped_marquee_rect(panel_index, origin, current)
                        .is_some()
                {
                    let selected = self.drawing_view(scene).marquee_selected_drawings(
                        panel_index,
                        origin,
                        current,
                    );
                    shell.publish(M::from(KlineWidgetEvent::from(
                        KlineWidgetDrawingEvent::MarqueeSelected(selected),
                    )));
                }
                state.clear_overlay_caches();
                state.clear_all_caches();
                capture = true;
                if let (Some(context), Some(key)) =
                    (layer_context, press_action.and_then(PressAction::layer_key))
                {
                    capture |= self.apply_study_object_pointer(
                        key,
                        LayerPointerInput::Cancelled,
                        context,
                        state,
                        shell,
                    );
                }
            }
            ReleasedGesture::Drag {
                action,
                press_action,
                ..
            } => {
                if let (Some(context), Some(key)) =
                    (layer_context, press_action.and_then(PressAction::layer_key))
                    && action.layer_key() != Some(key)
                {
                    capture |= self.apply_study_object_pointer(
                        key,
                        LayerPointerInput::Cancelled,
                        context,
                        state,
                        shell,
                    );
                }
            }
            _ => {}
        }

        if outside_bounds {
            state.clear_overlay_caches();
            state.clear_all_caches();
        }
        if capture {
            shell.capture_event();
        }
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
        let mut state = State::default();
        self.reconcile_study_states(&mut state.study_objects);
        tree::State::new(state)
    }

    fn diff(&self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<State>();
        let active_key = state
            .interaction
            .press_action()
            .and_then(PressAction::layer_key)
            .or_else(|| {
                state
                    .interaction
                    .drag_action()
                    .and_then(DragAction::layer_key)
            });
        if let Some(key) = active_key {
            let current_object = self.study_objects.iter().find(|object| object.key() == key);
            let previous_state = state.study_objects.get(&key);
            let state_matches = match (current_object, previous_state) {
                (Some(object), Some(previous_state)) => object.matches_state(previous_state),
                (None, None) => true,
                _ => false,
            };
            if !state_matches {
                let _ = state.interaction.cancel_pointer();
                state.clear_all_caches();
            }
        }
        self.reconcile_study_states(&mut state.study_objects);
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
                        SceneActivity::Idle,
                        &state.study_objects,
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
                state.interaction.set_modifiers(*modifiers);
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

                shell.publish(M::from(KlineWidgetEvent::from(
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
                state.interaction.set_modifiers(*modifiers);
                state.clear_overlay_caches();
                shell.publish(M::from(KlineWidgetEvent::from(
                    KlineWidgetDrawingEvent::MagnetModifierChanged(Self::is_command_modifier(
                        *modifiers,
                    )),
                )));
            }
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                let state = tree.state.downcast_mut::<State>();
                state.interaction.set_modifiers(*modifiers);
                state.clear_overlay_caches();
                shell.publish(M::from(KlineWidgetEvent::from(
                    KlineWidgetDrawingEvent::MagnetModifierChanged(Self::is_command_modifier(
                        *modifiers,
                    )),
                )));
            }
            Event::Mouse(mouse_event) => {
                let state = tree.state.downcast_mut::<State>();
                let bounds = layout.bounds();
                let left_release = matches!(
                    mouse_event,
                    mouse::Event::ButtonReleased(mouse::Button::Left)
                );
                let Some(layout_tree) =
                    PanelLayoutTree::from_layout(layout, &self.composition.panels)
                else {
                    return;
                };

                if left_release && cursor.position_in(bounds).is_none() {
                    self.finish_interaction_release(state, layout, cursor, true, shell);
                    return;
                }

                let cursor_pos = if let Some(local) = cursor.position_in(bounds) {
                    local
                } else if state
                    .interaction
                    .drag_action()
                    .is_some_and(DragAction::active_outside_bounds)
                {
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
                                    HorizontalWheelZoomMode::from_modifiers(
                                        state.interaction.modifiers(),
                                    )
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
                        let click_kind = state.interaction.begin_press(
                            cursor_pos,
                            cursor.position(),
                            PressAction::Ordinary,
                        );

                        if let LayoutHitZone::YAxis(panel_index) = zone
                            && click_kind == Some(iced_core::mouse::click::Kind::Double)
                            && let Some(panel_id) = self.panel_id(panel_index)
                        {
                            let interaction = PanelInteraction::YViewportReset { panel_id };

                            shell.publish(M::from(KlineWidgetEvent::PanelInteraction(interaction)));
                            state.clear_all_caches();
                            return;
                        }

                        if matches!(
                            zone,
                            LayoutHitZone::BottomXAxis | LayoutHitZone::PanelXAxis(_)
                        ) && click_kind == Some(iced_core::mouse::click::Kind::Double)
                        {
                            shell.publish(M::from(KlineWidgetEvent::XAxisDoubleClick));
                            state.clear_all_caches();
                            return;
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
                            state.clear_all_caches();
                            shell.capture_event();
                            return;
                        }

                        if let Some(CursorTarget::Legend(LegendTarget::Footprint {
                            icon: Some(icon_kind),
                        })) = cursor_target
                        {
                            shell.publish(M::from(icon_kind.into_event()));
                            state.clear_all_caches();
                            shell.capture_event();
                            return;
                        }

                        if let Some(CursorTarget::Legend(LegendTarget::StudyCategory {
                            icon: Some(icon_kind),
                        })) = cursor_target
                            && self.study_legend.is_some()
                        {
                            let event = match icon_kind {
                                chrome::StudiesLegendIconKind::ExpandCollapse => {
                                    KlineWidgetEvent::StudiesExpansionToggled
                                }
                                chrome::StudiesLegendIconKind::Visibility => {
                                    KlineWidgetEvent::StudiesVisibilityToggled
                                }
                                chrome::StudiesLegendIconKind::Close => {
                                    KlineWidgetEvent::StudiesRemove
                                }
                            };
                            shell.publish(M::from(event));
                            state.clear_all_caches();
                            shell.capture_event();
                            return;
                        }

                        if let Some(CursorTarget::Legend(LegendTarget::Study {
                            row: row_index,
                            icon: Some(icon_kind),
                        })) = cursor_target
                            && let Some(row) = self
                                .study_legend
                                .as_ref()
                                .and_then(|legend| legend.rows.get(row_index))
                        {
                            let event = match icon_kind {
                                chrome::StudyLegendIconKind::Visibility => {
                                    KlineWidgetEvent::StudyVisibilityToggled(row.id)
                                }
                                chrome::StudyLegendIconKind::Settings => {
                                    KlineWidgetEvent::StudySettings(row.id)
                                }
                                chrome::StudyLegendIconKind::Close => {
                                    KlineWidgetEvent::StudyRemove(row.id)
                                }
                            };
                            shell.publish(M::from(event));
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
                            state.clear_all_caches();
                            shell.capture_event();
                            return;
                        }

                        if let Some(CursorTarget::CornerControl(kind)) = cursor_target {
                            shell.publish(M::from(kind.into_event()));
                            state.clear_all_caches();
                            shell.capture_event();
                            return;
                        }

                        if matches!(cursor_target, Some(CursorTarget::Legend(_))) {
                            return;
                        }

                        if let Some(CursorTarget::PanelControl { panel_index, kind }) =
                            cursor_target
                        {
                            let interaction = kind.into_interaction_kind(panel_index);

                            shell.publish(M::from(KlineWidgetEvent::PanelInteraction(interaction)));
                            state.clear_all_caches();
                            shell.capture_event();
                            return;
                        }

                        let layer_hit = match cursor_target {
                            Some(CursorTarget::StudyObject(hit)) => Some(hit),
                            _ => None,
                        };
                        let drawing_hit = match cursor_target {
                            Some(CursorTarget::Drawing(hit)) => Some(hit),
                            _ => None,
                        };
                        if self.drawing_snapshot.allows_panning()
                            && matches!(zone, LayoutHitZone::PanelPlot(_))
                            && let Some(scene) = scene.as_ref()
                        {
                            self.broadcast_study_object_pointer(
                                LayerPointerInput::PlotPressed {
                                    target: layer_hit.map(|hit| hit.key),
                                },
                                self.layer_context(
                                    scene,
                                    Some(cursor_pos),
                                    state.interaction.modifiers(),
                                ),
                                state,
                                shell,
                            );
                        }
                        if let Some(hit) = layer_hit.filter(|hit| hit.precedes_tools)
                            && let Some(scene) = scene.as_ref()
                            && self.begin_study_object_interaction(
                                hit.key, scene, cursor_pos, state, shell,
                            )
                        {
                            return;
                        }

                        if (self.drawing_snapshot.measure.is_some()
                            || state
                                .interaction
                                .modifiers()
                                .intersects(keyboard::Modifiers::SHIFT)
                            || self.drawing_snapshot.active_tool == DrawingTool::Measure)
                            && let LayoutHitZone::PanelPlot(panel_index) = zone
                        {
                            let measure_panel =
                                self.drawing_snapshot.measure.map(|m| m.state().panel_id);
                            let panel_id = measure_panel.or_else(|| self.panel_id(panel_index));

                            if let Some(panel_id) = panel_id
                                && let Some(scene) = self.cached_scene(state, layout, cursor)
                                && let Some(anchor) = self
                                    .drawing_view(&scene)
                                    .drawing_anchor_from_scene_point(panel_id, cursor_pos)
                            {
                                shell.publish(M::from(KlineWidgetEvent::from(
                                    KlineWidgetDrawingEvent::MeasureClick(anchor),
                                )));
                                state.clear_all_caches();
                                shell.capture_event();
                                return;
                            }
                        }

                        if let Some(hit) = layer_hit.filter(|hit| !hit.precedes_tools)
                            && let Some(scene) = scene.as_ref()
                            && self.begin_study_object_interaction(
                                hit.key, scene, cursor_pos, state, shell,
                            )
                        {
                            return;
                        }

                        if drawing_hit.is_some()
                            && let Some(scene) = scene.as_ref()
                            && self.begin_drawing_interaction(scene, cursor_pos, state, shell)
                        {
                            return;
                        }

                        if self.drawing_snapshot.allows_panning()
                            && Self::is_command_modifier(state.interaction.modifiers())
                            && let LayoutHitZone::PanelPlot(panel_index) = zone
                        {
                            state.interaction.begin_drag_candidate(
                                cursor_pos,
                                DragAction::Marquee {
                                    panel_index,
                                    source_layer: None,
                                },
                            );
                            state.clear_cursor_caches();
                            shell.capture_event();
                            return;
                        }

                        if let LayoutHitZone::Splitter(split_index) = zone {
                            state
                                .interaction
                                .begin_drag_candidate(cursor_pos, DragAction::Split(split_index));
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
                            state.interaction.begin_drag_candidate(
                                cursor_pos,
                                DragAction::YAxisScale {
                                    panel_index,
                                    anchor_y,
                                },
                            );
                            shell.capture_event();
                        } else if matches!(
                            zone,
                            LayoutHitZone::PanelXAxis(_) | LayoutHitZone::BottomXAxis
                        ) {
                            if let Some(scene) = self.cached_scene(state, layout, cursor) {
                                state.interaction.begin_drag_candidate(
                                    cursor_pos,
                                    DragAction::XAxisScale {
                                        initial_world: self.horizontal_world,
                                        initial_right_unit: scene.effective_right_x_unit(),
                                    },
                                );
                                shell.capture_event();
                            } else {
                                self.cancel_pointer(state, layout, cursor, shell);
                            }
                        } else if !self.drawing_snapshot.allows_panning() {
                            let magnetic =
                                self.drawing_magnet_enabled(state.interaction.modifiers());
                            let anchor =
                                if let Some(panel_id) = self.drawing_snapshot.draft_panel_id() {
                                    self.cached_scene(state, layout, cursor).and_then(|scene| {
                                        self.drawing_view(&scene)
                                            .drawing_draft_anchor_from_scene_point(
                                                panel_id, cursor_pos, magnetic,
                                            )
                                    })
                                } else if let LayoutHitZone::PanelPlot(panel_index) = zone {
                                    let panel_id = self.panel_id(panel_index);

                                    panel_id.and_then(|panel_id| {
                                        self.cached_scene(state, layout, cursor).and_then(|scene| {
                                            self.drawing_view(&scene)
                                                .drawing_draft_anchor_from_scene_point(
                                                    panel_id, cursor_pos, magnetic,
                                                )
                                        })
                                    })
                                } else {
                                    None
                                };

                            if let Some(anchor) = anchor {
                                state
                                    .interaction
                                    .begin_click(cursor_pos, PressAction::DrawingPlacement);
                                shell.publish(M::from(KlineWidgetEvent::from(
                                    KlineWidgetDrawingEvent::AnchorPressed(anchor),
                                )));
                                state.clear_overlay_caches();
                                state.clear_all_caches();
                                shell.capture_event();
                            } else {
                                self.cancel_pointer(state, layout, cursor, shell);
                            }
                        } else if self.drawing_snapshot.allows_panning()
                            && let LayoutHitZone::PanelPlot(panel_index) = zone
                        {
                            state.interaction.begin_drag_candidate(
                                cursor_pos,
                                DragAction::Pan {
                                    panel_index,
                                    source_layer: None,
                                },
                            );
                        }
                    }
                    mouse::Event::ButtonPressed(mouse::Button::Right)
                        if self.drawing_snapshot.drawing_draft.is_some() =>
                    {
                        shell.publish(M::from(KlineWidgetEvent::from(
                            KlineWidgetDrawingEvent::DraftCanceled,
                        )));
                        self.cancel_pointer(state, layout, cursor, shell);
                        state.clear_overlay_caches();
                        state.clear_all_caches();
                        shell.capture_event();
                    }
                    mouse::Event::ButtonReleased(mouse::Button::Left) => {
                        self.finish_interaction_release(state, layout, cursor, false, shell);
                    }
                    mouse::Event::CursorMoved { .. } => {
                        state.clear_cursor_caches();
                        let pointer_move = state.interaction.pointer_moved(cursor_pos);
                        if matches!(pointer_move, PointerMove::Pending) {
                            shell.capture_event();
                            return;
                        }
                        let pointer_delta = match pointer_move {
                            PointerMove::Moving { delta } => delta,
                            PointerMove::Idle | PointerMove::Pending => Vector::new(0.0, 0.0),
                        };

                        if let Some(DrawingMeasure::Pending(measure_state)) =
                            self.drawing_snapshot.measure
                            && let Some(scene) = self.cached_scene(state, layout, cursor)
                            && let Some(anchor) = self
                                .drawing_view(&scene)
                                .drawing_anchor_from_scene_point(measure_state.panel_id, cursor_pos)
                        {
                            shell.publish(M::from(KlineWidgetEvent::from(
                                KlineWidgetDrawingEvent::MeasureMove(anchor),
                            )));
                        }

                        let drag_action = state.interaction.drag_action();
                        if let Some(DragAction::Layer { key, .. }) = drag_action {
                            if let Some(scene) = self.cached_scene(state, layout, cursor)
                                && let Some(effects) = self.study_object_pointer(
                                    key,
                                    LayerPointerInput::Moved,
                                    self.layer_context(
                                        &scene,
                                        Some(cursor_pos),
                                        state.interaction.modifiers(),
                                    ),
                                    &mut state.study_objects,
                                )
                            {
                                state.apply_interaction_effects(
                                    effects,
                                    shell,
                                    KlineWidgetEvent::Study,
                                );
                            }
                        } else if let Some(DragAction::Drawing { .. }) = drag_action {
                            if let Some(scene) = self.cached_scene(state, layout, cursor) {
                                self.apply_drawing_pointer(
                                    DrawingPointerInput::Moved,
                                    self.layer_context(
                                        &scene,
                                        Some(cursor_pos),
                                        state.interaction.modifiers(),
                                    ),
                                    state,
                                    shell,
                                );
                            }
                        } else if let Some(DragAction::XAxisScale {
                            initial_world,
                            initial_right_unit,
                        }) = drag_action
                        {
                            let Some(anchor_x) =
                                state.interaction.motion().map(|motion| motion.origin.x)
                            else {
                                return;
                            };
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
                        } else if let Some(DragAction::Split(split_index)) = drag_action {
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
                                state.clear_all_caches();
                                shell.capture_event();
                            }
                        } else if let Some(DragAction::YAxisScale {
                            panel_index,
                            anchor_y,
                        }) = drag_action
                        {
                            let dy_px = pointer_delta.y;
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
                        } else if state.marquee().is_some() {
                            state.clear_cursor_caches();
                            shell.capture_event();
                        } else if let Some(draft_panel_id) = self.drawing_snapshot.draft_panel_id()
                            && !matches!(
                                cursor_target,
                                Some(
                                    CursorTarget::PanelControl { .. }
                                        | CursorTarget::CornerControl(_)
                                        | CursorTarget::Legend(_)
                                )
                            )
                            && let Some(scene) = self.cached_scene(state, layout, cursor)
                            && let Some(anchor) = self
                                .drawing_view(&scene)
                                .drawing_draft_anchor_from_scene_point(
                                    draft_panel_id,
                                    cursor_pos,
                                    self.drawing_magnet_enabled(state.interaction.modifiers()),
                                )
                        {
                            shell.publish(M::from(KlineWidgetEvent::from(
                                KlineWidgetDrawingEvent::AnchorMoved(anchor),
                            )));
                            state.clear_cursor_caches();
                            shell.capture_event();
                        } else if let Some(DragAction::Pan { panel_index, .. }) = drag_action {
                            if Self::hit_corner_control(corner_controls, cursor_pos).is_some() {
                                return;
                            }

                            let dx_px = pointer_delta.x;
                            let dy_px = pointer_delta.y;
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
                        RootPaintStep::Objects => {
                            let overlay_pixels = PixelGeometry::new(
                                self.pixel_ratio,
                                Point::new(bounds.x, bounds.y),
                            );
                            let geometry =
                                state.drawing_cache.draw(renderer, bounds.size(), |frame| {
                                    self.drawing_view_at(
                                        &scene,
                                        None,
                                        state.interaction.modifiers(),
                                    )
                                    .fill_drawing_objects(
                                        frame,
                                        palette,
                                        generated_accents,
                                        overlay_pixels,
                                    );
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
                            PrimaryPlotLayer::Objects => &state.study_underlay_cache,
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
                                &state.study_objects,
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
                        let drawing_magnet =
                            self.drawing_magnet_enabled(state.interaction.modifiers());
                        let geometry = match layer {
                            WidgetPaintStep::Legends => {
                                state.legend_cache.draw(renderer, bounds.size(), |frame| {
                                    self.fill_legends(frame, &scene, palette, generated_accents);
                                })
                            }
                            WidgetPaintStep::Overlay => {
                                state.overlay_cache.draw(renderer, bounds.size(), |frame| {
                                    self.fill_overlay(
                                        frame,
                                        &scene,
                                        palette,
                                        generated_accents,
                                        overlay_pixels,
                                        drawing_magnet,
                                    );
                                    self.paint_study_objects(
                                        LayerPaintPhase::Overlay,
                                        self.layer_context(
                                            &scene,
                                            None,
                                            state.interaction.modifiers(),
                                        ),
                                        &state.study_objects,
                                        frame,
                                        palette,
                                        generated_accents,
                                        overlay_pixels,
                                    );
                                    if self.drawing_snapshot.has_state() {
                                        self.drawing_view_at(
                                            &scene,
                                            None,
                                            state.interaction.modifiers(),
                                        )
                                        .fill_drawing_interaction(
                                            frame,
                                            palette,
                                            generated_accents,
                                            overlay_pixels,
                                        );
                                    }
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
                                    if let Some(marquee) = state.marquee()
                                        && let Some(rect) =
                                            self.drawing_view(&scene).clipped_marquee_rect(
                                                marquee.panel_index,
                                                marquee.start,
                                                marquee.current,
                                            )
                                    {
                                        drawing::fill_marquee_rectangle(frame, rect, palette);
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

        if let Some(appearance) = state
            .interaction
            .drag_action()
            .and_then(DragAction::cursor_appearance)
        {
            return appearance;
        }

        let scene = self.cached_scene(state, layout, cursor);
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
            CursorTarget::Drawing(hit) => hit.cursor_appearance(),
            CursorTarget::StudyObject(hit) => hit.cursor,
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

fn primary_scale_inversion_shortcut(
    physical_key: keyboard::key::Physical,
    modifiers: keyboard::Modifiers,
) -> bool {
    modifiers.alt() && physical_key == keyboard::key::Code::KeyI
}

impl<'a, M> From<KlineWidget<'a>> for Element<'a, M, Theme, Renderer>
where
    M: Clone + 'a + 'static + From<KlineWidgetEvent>,
{
    fn from(chart: KlineWidget<'a>) -> Self {
        Self::new(chart)
    }
}
