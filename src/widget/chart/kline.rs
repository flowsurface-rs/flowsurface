use super::{
    LEGEND_CHAR_WIDTH as CHAR_W, LEGEND_LEFT_PAD as TOOLTIP_ROW_LEFT_PAD,
    LEGEND_PADDING as TICKER_LEGEND_PADDING, LEGEND_ROW_HEIGHT as TICKER_LEGEND_ROW_H,
    LEGEND_TEXT_SIZE as TEXT_SIZE, LEGEND_TOP_PAD as PANEL_TITLE_TOP_PAD,
};
mod axes;
mod chrome;
pub mod drawing;
mod footprint;
mod interaction;
mod layout;
mod overlay;
mod painting;
mod presentation;
mod scene;
pub mod studies;

use axes::AxisInputs;

use iced::widget::canvas;
use scene::{
    BarUnitWindow, CursorInfo, IndicatorPanelScene, LegendTarget, PanelDomainState,
    PanelValueScale, PrimaryPercentScaleState, PrimaryScale, SceneBase, SceneCacheEntry, XViewport,
};

pub use chrome::{StudiesLegend, StudyLegendRow};

use crate::widget::chart::kline::chrome::PreparedPanelControls;
use crate::widget::chart::{PANEL_SPLITTER_HEIGHT, Y_AXIS_LABEL_LEFT_INSET};
use crate::{style, theme::ThemeState, window::PixelRatio};
use composition::{
    BarMode, ChartComposition, DEFAULT_MIN_PANEL_RATIO, HistogramMode, MarkKind, PanelId,
    PanelRole, PanelScaleMode, PanelValueId, PanelValuePrecision,
};
use data::UserTimezone;
use data::chart::Basis;
use data::chart::kline::composition;
use data::chart::kline::series::KlineSeries;
pub use data::chart::kline::viewport::PrimaryViewportState;
use data::chart::kline::{
    CandlestickConfig, DEFAULT_HORIZONTAL_SPACING_PX, FootprintConfig, KlineStudyId,
    LegendVisibility,
};
use data::chart::kline::{HorizontalWorld, IndicatorHost, IndicatorInstanceStore};
pub use data::chart::kline::{YUnit, YUnitRange};

use drawing::{
    DrawingMeasure, DrawingPointerInput, DrawingSnapshot, DrawingTool, DrawingView,
    KlineWidgetDrawingEvent, WidgetDrawingState,
};
use exchange::TickerInfo;
pub(super) use interaction::SceneActivity;
pub use interaction::SelectionDisposition;
use interaction::{
    DragAction, GestureSource, LayerContext, PointerMove, PressAction, ReleasedGesture,
    UserInteraction,
};
use layout::{KlinePanelKind, LayoutHitZone, PanelLayoutTree};
use painting::{
    CanvasLayer, CanvasLayers, LayerPlacement, PaintInputs, PixelGeometry, RenderRequest,
    RenderScheduler, ResolvedLayer,
};
pub use presentation::IndicatorPresentationStatus;
pub use presentation::{
    IndicatorPresentationContext, ResolvedIndicatorLayer, ResolvedIndicatorPresentation,
    ResolvedIndicatorStyleSlot, resolve_indicator_presentation_status,
};
use scene::{ChartCoordinates, CursorTarget, Scene, SceneCache, XAxis};
use studies::{
    LayerPaintPhase, LayerPointerInput, StudyObject, StudyObjectEvent, StudyObjectKey, StudyStates,
};

use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{self, Clipboard, Layout, Shell, Widget, layout as iced_layout, renderer};
use iced::theme::palette::Extended;
use iced::{
    Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector, keyboard, mouse,
    window,
};
use iced_core::renderer::Quad;
use std::{sync::Arc, time::Duration};

const Y_AXIS_GUTTER: f32 = 66.0;
const X_AXIS_HEIGHT: f32 = 24.0;
pub const MIN_FULL_CACHE_REFRESH_INTERVAL: Duration = Duration::from_micros(33_333);

/// The submission order used by [`KlineWidget::draw`]; Iced may merge compatible layers.
///
/// # Note
///
/// Keep plot, widget, and legend passes separate because `iced_wgpu` renders
/// quads before meshes and text within each renderer layer.
const PAINT_ORDER: [PaintStep; 14] = [
    PaintStep::Canvas(CanvasLayer::PlotUnderlay),
    PaintStep::Canvas(CanvasLayer::StudyUnderlay),
    PaintStep::Canvas(CanvasLayer::SeriesPlots),
    PaintStep::Canvas(CanvasLayer::SavedDrawings),
    PaintStep::Canvas(CanvasLayer::PriceBars),
    PaintStep::Canvas(CanvasLayer::YAxisLabels),
    PaintStep::Canvas(CanvasLayer::XAxisLabels),
    PaintStep::Canvas(CanvasLayer::Overlay),
    PaintStep::Canvas(CanvasLayer::SelectionMarquee),
    PaintStep::AxisBadges,
    PaintStep::StudyChrome,
    PaintStep::Legends,
    PaintStep::ChartSeparators,
    PaintStep::PanelControls,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaintStep {
    /// Crosshair, last-price/countdown, and drawing readouts above the selection marquee.
    AxisBadges,
    /// Ticker, footprint, study, and indicator legend rows, including their actions.
    Legends,
    /// Panel splitters and the plot's axis boundaries.
    ChartSeparators,
    /// Panel move/close buttons and the primary autoscale/scale-mode buttons.
    PanelControls,
    /// Study-specific controls and markers, such as TPO merge/reset and handles.
    StudyChrome,
    Canvas(CanvasLayer),
}

struct MarqueeGesture {
    panel_index: usize,
    start: Point,
    current: Point,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct GutterCacheKey {
    version: u64,
    width: f32,
    height: f32,
    panel_count: usize,
}

struct State {
    layers: CanvasLayers,
    scheduler: RenderScheduler,
    axis_badge_inputs: Option<overlay::AxisBadgeInputs>,
    axis_badges: Option<chrome::PreparedChrome>,
    panel_controls: Option<chrome::PreparedPanelControls>,
    study_chrome: Option<chrome::PreparedChrome>,
    legends: Option<chrome::PreparedChrome>,
    scene_cache: SceneCache,
    interaction: UserInteraction,
    study_objects: StudyStates,
    drawing: WidgetDrawingState,
    y_axis_gutter_width: f32,
    gutter_cache_key: Option<GutterCacheKey>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            layers: CanvasLayers::default(),
            scheduler: RenderScheduler::default(),
            axis_badge_inputs: None,
            axis_badges: None,
            panel_controls: None,
            study_chrome: None,
            legends: None,
            interaction: UserInteraction::default(),
            study_objects: StudyStates::default(),
            drawing: WidgetDrawingState::default(),
            y_axis_gutter_width: Y_AXIS_GUTTER,
            gutter_cache_key: None,
            scene_cache: SceneCache::default(),
        }
    }
}

impl State {
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
    TickerVisibilityToggled(TickerInfo),
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
    study_objects: studies::StudyObjects<'a>,
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
    theme: &'a ThemeState,
    should_capture_escape: bool,
}

impl<'a> KlineWidget<'a> {
    pub fn new(
        series: &'a [KlineSeries],
        indicator_host: &'a IndicatorHost,
        basis: Basis,
        composition: &'a ChartComposition,
        indicator_instances: &'a IndicatorInstanceStore,
        indicator_presentation: IndicatorPresentationContext,
        theme: &'a ThemeState,
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
            study_objects: studies::StudyObjects::default(),
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
            indicator_presentation,
            timezone: UserTimezone::Utc,
            version: 0,
            theme,
            should_capture_escape: false,
        }
    }

    pub fn with_study_legend(mut self, legend: chrome::StudiesLegend) -> Self {
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

    pub fn with_primary_viewport(mut self, viewport: PrimaryViewportState) -> Self {
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

    pub fn with_indicator_ranges(mut self, ranges: &'a [(PanelId, YUnitRange)]) -> Self {
        self.indicator_y_ranges = ranges;
        self
    }

    pub fn with_timezone(mut self, tz: UserTimezone) -> Self {
        self.timezone = tz;
        self
    }

    fn indicator_is_unavailable(&self, value_id: Option<PanelValueId>) -> bool {
        self.indicators().indicator_is_unavailable(value_id)
    }

    pub fn with_version(mut self, rev: u64) -> Self {
        self.version = rev;
        self
    }

    pub fn with_should_capture_escape(mut self, should_capture_escape: bool) -> Self {
        self.should_capture_escape = should_capture_escape;
        self
    }

    fn plot_renderer(&self) -> painting::plot::PlotRenderer<'_> {
        painting::plot::PlotRenderer {
            coordinates: self.coordinates(),
            indicators: self.indicators(),
            footprints: footprint::FootprintRenderer {
                series: self.series,
                candlestick: &self.candlestick,
                footprint: &self.footprint,
            },
            study_objects: &self.study_objects,
            trade_data_enabled: self.trade_data_enabled,
        }
    }

    fn overlay_view<'b>(&'b self, scene: &'b Scene) -> overlay::OverlayView<'b> {
        overlay::OverlayView {
            coordinates: self.coordinates(),
            indicators: self.indicators(),
            drawing: self.drawing_view(scene),
        }
    }

    fn legend_view(&self) -> chrome::legend::LegendView<'_> {
        chrome::legend::LegendView {
            coordinates: self.coordinates(),
            indicators: self.indicators(),
            candlestick: &self.candlestick,
            footprint: &self.footprint,
            study_legend: self.study_legend.as_ref(),
            legend_visibility: self.legend_visibility,
        }
    }

    fn indicators(&self) -> presentation::IndicatorView<'_> {
        presentation::IndicatorView {
            series: self.series,
            composition: self.composition,
            host: self.indicator_host,
            instances: self.indicator_instances,
            presentation: &self.indicator_presentation,
        }
    }

    fn coordinates(&self) -> ChartCoordinates<'_> {
        ChartCoordinates {
            series: self.series,
            composition: self.composition,
            basis: self.basis,
            timezone: self.timezone,
        }
    }

    fn axes<'b>(&'b self, scene: &'b Scene) -> axes::AxisView<'b> {
        axes::AxisView {
            scene,
            coordinates: self.coordinates(),
            indicators: self.indicators(),
        }
    }

    fn navigation(&self) -> interaction::navigation::NavigationView<'_> {
        interaction::navigation::NavigationView {
            viewports: self.viewports(),
            horizontal_world: self.horizontal_world,
        }
    }

    fn viewports(&self) -> scene::viewport::PanelViewportView<'_> {
        scene::viewport::PanelViewportView {
            coordinates: self.coordinates(),
            pixel_ratio: self.pixel_ratio,
            primary_viewport: self.primary_viewport,
            indicator_y_ranges: self.indicator_y_ranges,
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

    fn prepare_render_inputs(
        &self,
        state: &mut State,
        scene: Option<&Scene>,
        bounds: Rectangle,
        request: RenderRequest,
        utc_ms: u64,
    ) {
        if request == RenderRequest::None {
            state.axis_badges = state
                .axis_badge_inputs
                .as_ref()
                .map(|prepared| prepared.resolve(utc_ms));
            return;
        }
        let Some(scene) = scene else {
            let root_origin = bounds.position();
            for (layer, cache) in &mut state.layers {
                if request == RenderRequest::All || layer.is_interaction() {
                    cache.prepare(ResolvedLayer {
                        root_origin,
                        placement: LayerPlacement::Widget(bounds.size()),
                        pixel_ratio: self.pixel_ratio,
                        inputs: PaintInputs::default(),
                    });
                }
            }
            state.axis_badge_inputs = None;
            state.axis_badges = None;
            state.panel_controls = None;
            state.study_chrome = None;
            state.legends = None;
            return;
        };
        let palette = self.theme.current().extended_palette();

        state.panel_controls = Some(PreparedPanelControls::new(
            scene,
            bounds,
            palette,
            self.primary_viewport.is_auto(),
        ));

        let generated_accents = self.theme.accents();
        let accents = generated_accents.as_array();

        let root_origin = Point::new(bounds.x, bounds.y);
        let root_pixels = PixelGeometry::new(self.pixel_ratio, root_origin);
        let drawing_magnet = self.drawing_magnet_enabled(state.interaction.modifiers());

        let mut legends = chrome::PreparedChrome::new(bounds);
        self.legend_view()
            .fill_legends(&mut legends, scene, palette, accents);
        state.legends = Some(legends);
        let mut study_chrome = chrome::PreparedChrome::new(bounds);
        self.study_objects.prepare_controls(
            self.layer_context(scene, None, state.interaction.modifiers()),
            &state.study_objects,
            &mut study_chrome,
            palette,
            root_pixels,
        );
        state.study_chrome = Some(study_chrome);

        let prepared = self.overlay_view(scene).axis_badge_inputs(
            scene,
            bounds,
            palette,
            drawing_magnet,
            root_pixels,
        );
        state.axis_badges = Some(prepared.resolve(utc_ms));
        state.axis_badge_inputs = Some(prepared);

        for step in PAINT_ORDER {
            let PaintStep::Canvas(layer) = step else {
                continue;
            };
            if request != RenderRequest::All
                && !(request == RenderRequest::Interaction && layer.is_interaction())
            {
                continue;
            }
            let placement = match layer {
                CanvasLayer::PlotUnderlay
                | CanvasLayer::StudyUnderlay
                | CanvasLayer::SeriesPlots
                | CanvasLayer::PriceBars => LayerPlacement::Plot(scene.plot_rect()),
                CanvasLayer::YAxisLabels => LayerPlacement::Axis(scene.layout.regions.y_axis),
                CanvasLayer::XAxisLabels => LayerPlacement::Axis(scene.layout.regions.x_axis),
                _ => LayerPlacement::Widget(bounds.size()),
            };
            let origin = placement.origin();
            let pixels = PixelGeometry::new(
                self.pixel_ratio,
                Point::new(bounds.x + origin.x, bounds.y + origin.y),
            );
            let mut inputs = PaintInputs::default();
            match layer {
                CanvasLayer::PlotUnderlay
                | CanvasLayer::StudyUnderlay
                | CanvasLayer::SeriesPlots
                | CanvasLayer::PriceBars => {
                    self.plot_renderer().paint(
                        &mut inputs,
                        scene,
                        &state.study_objects,
                        palette,
                        accents,
                        pixels,
                        layer,
                    );
                    if layer == CanvasLayer::SeriesPlots {
                        self.fill_panel_titles(&mut inputs, scene, palette);
                        self.overlay_view(scene)
                            .paint_last_price_line(&mut inputs, scene, palette);
                    }
                }
                CanvasLayer::SavedDrawings => self
                    .drawing_view_at(scene, None, state.interaction.modifiers())
                    .fill_drawing_objects(&mut inputs, palette, accents, pixels),
                CanvasLayer::YAxisLabels => self.fill_y_axis_labels(
                    &mut inputs,
                    scene,
                    state
                        .scene_cache
                        .get(self.version)
                        .map(|entry| &entry.y_axis_labels),
                    palette,
                ),
                CanvasLayer::XAxisLabels => {
                    self.axes(scene)
                        .x_labels(pixels)
                        .paint(&mut inputs, palette, &[])
                }
                CanvasLayer::Overlay => {
                    self.overlay_view(scene).paint(
                        &mut inputs,
                        scene,
                        palette,
                        accents,
                        root_pixels,
                        drawing_magnet,
                    );
                    self.study_objects.paint(
                        LayerPaintPhase::Overlay,
                        self.layer_context(scene, None, state.interaction.modifiers()),
                        &state.study_objects,
                        &mut inputs,
                        palette,
                        accents,
                        root_pixels,
                    );
                    if self.drawing_snapshot.has_state() {
                        self.drawing_view_at(scene, None, state.interaction.modifiers())
                            .fill_drawing_interaction(&mut inputs, palette, accents, root_pixels);
                    }
                }
                CanvasLayer::SelectionMarquee => {
                    if let Some(marquee) = state.marquee()
                        && let Some(rect) = self.drawing_view(scene).clipped_marquee_rect(
                            marquee.panel_index,
                            marquee.start,
                            marquee.current,
                        )
                    {
                        drawing::fill_marquee_rectangle(&mut inputs, rect, palette);
                    }
                }
            }
            state.layers[layer].prepare(ResolvedLayer {
                root_origin,
                placement,
                pixel_ratio: self.pixel_ratio,
                inputs,
            });
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
        let Some(effects) = self.study_objects.pointer(
            key,
            LayerPointerInput::Pressed,
            self.layer_context(scene, Some(position), state.interaction.modifiers()),
            &mut state.study_objects,
        ) else {
            return false;
        };
        let (_, gesture) = effects.apply(&mut state.scheduler, shell, |event| {
            M::from(KlineWidgetEvent::Study(event))
        });
        let Some(gesture) = gesture else { return false };
        state.interaction.begin_layer_gesture(
            gesture,
            GestureSource::Study(key),
            position,
            scene.layout.hit_test(position),
        )
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
        let (_, gesture) = effects.apply(&mut state.scheduler, shell, |event| {
            M::from(KlineWidgetEvent::Drawing(event))
        });
        let Some(gesture) = gesture else { return false };

        state.interaction.begin_layer_gesture(
            gesture,
            GestureSource::Drawing,
            position,
            scene.layout.hit_test(position),
        )
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
        self.study_objects
            .pointer(key, input, context, &mut state.study_objects)
            .is_some_and(|effects| {
                effects
                    .apply(&mut state.scheduler, shell, |event| {
                        M::from(KlineWidgetEvent::Study(event))
                    })
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
        let effects = self
            .study_objects
            .pointer_all(input, context, &mut state.study_objects);
        let mut captured = false;
        for effects in effects {
            captured |= effects
                .apply(&mut state.scheduler, shell, |event| {
                    M::from(KlineWidgetEvent::Study(event))
                })
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
        let effects = view.pointer(input, &mut state.drawing);
        effects
            .apply(&mut state.scheduler, shell, |event| {
                M::from(KlineWidgetEvent::Drawing(event))
            })
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
            ReleasedGesture::Idle => return,
            ReleasedGesture::Pressed { action, click } => (
                if click == interaction::ClickOutcome::Confirmed {
                    action.selection_disposition()
                } else {
                    SelectionDisposition::Preserve
                },
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

        if let ReleasedGesture::Pressed {
            action,
            click: interaction::ClickOutcome::Confirmed,
        } = outcome
            && let Some(context) = layer_context
            && position.is_some_and(|point| {
                matches!(
                    context.scene.layout.hit_test(point),
                    LayoutHitZone::PanelPlot(_)
                )
            })
        {
            capture |= self.broadcast_study_object_pointer(
                LayerPointerInput::PlotClicked {
                    target: action.layer_key(),
                },
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
                state.scheduler.request(RenderRequest::All);
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
            state.scheduler.request(RenderRequest::All);
        }
        if capture {
            shell.capture_event();
        }
    }

    fn panel_count(&self) -> usize {
        self.composition.panel_count().max(1)
    }

    fn panel_id(&self, panel_index: usize) -> Option<composition::PanelId> {
        self.coordinates().panel_id(panel_index)
    }

    fn panel_value_precision(&self, panel_index: usize) -> Option<PanelValuePrecision> {
        self.coordinates().panel_value_precision(panel_index)
    }

    fn default_mark_for_panel(kind: KlinePanelKind) -> MarkKind {
        match kind {
            KlinePanelKind::PrimaryChart => MarkKind::Candle,
            KlinePanelKind::Indicator => MarkKind::Bar(BarMode::Histogram(HistogramMode::Plain)),
        }
    }

    fn resolved_panel_mark(&self, panel_index: usize, panel_kind: KlinePanelKind) -> MarkKind {
        let Some(panel_id) = self.panel_id(panel_index) else {
            return Self::default_mark_for_panel(panel_kind);
        };

        self.composition
            .panel_effective_mark(panel_id)
            .unwrap_or_else(|| Self::default_mark_for_panel(panel_kind))
    }

    fn resolved_panel_scale_mode(&self, panel_index: usize) -> PanelScaleMode {
        let Some(panel) = self.composition.panels.get(panel_index) else {
            return PanelScaleMode::Absolute;
        };

        self.composition
            .panel_effective_scale_mode(panel.id)
            .unwrap_or(PanelScaleMode::Absolute)
    }

    fn base_ticker_info(&self) -> Option<exchange::TickerInfo> {
        self.coordinates().base_ticker_info()
    }

    fn panel_value_scale(&self, panel_index: usize) -> PanelValueScale {
        self.coordinates().panel_value_scale(panel_index)
    }

    fn build_bar_units(&self, axis: XAxis, min: i64, max: i64) -> Vec<BarUnitWindow> {
        self.series
            .iter()
            .map(|series| series.bar_unit_window(axis, min, max))
            .collect()
    }

    fn apply_cursor_interaction(
        &self,
        scene: &mut Scene,
        cursor_root_local: Option<Point>,
        primary_panel: usize,
        primary_plot: Rectangle,
        show_legend_values: bool,
        activity: SceneActivity,
        layers: &StudyStates,
    ) {
        let Some(local) = cursor_root_local else {
            return;
        };

        let hovered_control =
            chrome::hit_panel_control(&scene.layout, &scene.chrome.panel_controls, local);
        let hovered_corner_control =
            chrome::hit_corner_control(&scene.chrome.corner_controls, local);

        let mut ticker_hit = scene
            .chrome
            .ticker_legend
            .as_ref()
            .and_then(|legend| legend.hit_test(&scene.layout, local));

        if ticker_hit.is_some() {
            scene.chrome.ticker_legend = self.legend_view().build_ticker_legend_layout(
                &scene.layout,
                primary_panel,
                self.legend_visibility.legend_values_always_visible(),
                true,
            );
            ticker_hit = scene
                .chrome
                .ticker_legend
                .as_ref()
                .and_then(|legend| legend.hit_test(&scene.layout, local));
        }

        let footprint_hit = scene
            .chrome
            .footprint_legend
            .as_ref()
            .and_then(|legend| legend.hit_test(&scene.layout, local));

        let study_hit = scene
            .chrome
            .study_legend
            .as_ref()
            .and_then(|legend| legend.hit_test(&scene.layout, local));

        let mut indicator_hit = scene
            .chrome
            .indicator_legend
            .as_ref()
            .and_then(|legend| legend.hit_test(&scene.layout, local));

        if indicator_hit.is_some() {
            scene.chrome.indicator_legend = self.legend_view().build_indicator_legend_layout(
                &scene.layout,
                primary_panel,
                chrome::legend::tooltip_rows_before_indicators(
                    scene.chrome.ticker_legend.as_ref(),
                    scene.chrome.footprint_legend.as_ref(),
                    scene.chrome.study_legend.as_ref(),
                    show_legend_values,
                ),
            );
            indicator_hit = scene
                .chrome
                .indicator_legend
                .as_ref()
                .and_then(|legend| legend.hit_test(&scene.layout, local));
        }

        scene.interaction.controls_visible_for_panel = hovered_control
            .map(|hit| hit.panel_index)
            .or_else(|| match scene.layout.hit_test(local) {
                LayoutHitZone::PanelPlot(panel_index) | LayoutHitZone::PanelXAxis(panel_index) => {
                    Some(panel_index)
                }
                _ => None,
            });

        let zone = scene.layout.hit_test(local);

        let layer_target = if self.drawing_snapshot.allows_panning() {
            let drawing_hit = self.drawing_view(scene).hit(local);
            let study_hit = self.study_objects.hit_test(
                self.layer_context(scene, Some(local), iced::keyboard::Modifiers::empty()),
                layers,
            );
            match (drawing_hit, study_hit) {
                (Some(drawing), Some(study)) if drawing.priority() >= study.priority => {
                    Some(CursorTarget::Drawing(drawing))
                }
                (_, Some(study)) => Some(CursorTarget::StudyObject(study)),
                (Some(drawing), None) => Some(CursorTarget::Drawing(drawing)),
                (None, None) => None,
            }
        } else {
            None
        };

        let target = match activity {
            SceneActivity::Pan => CursorTarget::ActivePan,
            SceneActivity::Drag => CursorTarget::ActiveDrag,
            SceneActivity::Idle => {
                if let Some(hit) = ticker_hit {
                    CursorTarget::Legend(LegendTarget::from(hit))
                } else if let Some(hit) = footprint_hit {
                    CursorTarget::Legend(LegendTarget::from(hit))
                } else if let Some(hit) = study_hit {
                    CursorTarget::Legend(LegendTarget::from(hit))
                } else if let Some(hit) = indicator_hit {
                    CursorTarget::Legend(LegendTarget::from(hit))
                } else if let Some(control) = hovered_corner_control {
                    CursorTarget::CornerControl(control.kind)
                } else if let Some(control) = hovered_control {
                    CursorTarget::PanelControl {
                        panel_index: control.panel_index,
                        kind: control.kind,
                    }
                } else if let Some(target) = layer_target {
                    target
                } else {
                    match zone {
                        LayoutHitZone::PanelPlot(panel_index) => CursorTarget::Plot(panel_index),
                        LayoutHitZone::Splitter(index) => CursorTarget::Splitter(index),
                        LayoutHitZone::YAxis(index) => CursorTarget::YAxis(index),
                        LayoutHitZone::PanelXAxis(index) => CursorTarget::PanelXAxis(index),
                        LayoutHitZone::BottomXAxis => CursorTarget::BottomXAxis,
                        LayoutHitZone::Outside => CursorTarget::Outside,
                    }
                }
            }
        };
        scene.interaction.target = target;

        if !matches!(zone, LayoutHitZone::PanelPlot(_)) {
            return;
        }

        let Some(plot_local) = scene.layout.plot_local_point(local) else {
            return;
        };

        let x_plot = plot_local.x.clamp(0.0, primary_plot.width.max(1.0));
        let snapped_x_unit = scene.unit_from_plot_x(x_plot);
        let snapped_x_plot = scene
            .map_x_plot(snapped_x_unit)
            .clamp(0.0, scene.x_axis_plot_width().max(1.0));

        let LayoutHitZone::PanelPlot(panel_index) = zone else {
            return;
        };
        let Some(panel) = scene.layout.panel(panel_index) else {
            return;
        };

        let value_scale = self.panel_value_scale(panel_index);
        let y_in_panel = (plot_local.y - panel.plot.y).clamp(0.0, panel.plot.height.max(1.0));

        let (y_primary_unit, y_indicator_unit, snapped_y_plot) = match panel.kind {
            KlinePanelKind::PrimaryChart => {
                let value_ratio = scene.primary_value_ratio_at_plot_y(y_in_panel);
                let uses_display_transform = scene
                    .primary
                    .mode
                    .uses_display_transform(scene.can_use_log_primary_scale());

                let (y_primary_unit, snapped_y_plot) = if uses_display_transform {
                    let (min_display, max_display) = scene.primary_domain_display_values();
                    let y_display_value = min_display + ((max_display - min_display) * value_ratio);
                    let y_primary_unit = value_scale
                        .value_to_unit_f32(scene.primary_display_to_value(y_display_value));
                    let y_primary_value = value_scale.unit_to_value_f32(y_primary_unit);
                    let y_plot = scene
                        .map_primary_plot_with_anchor_f32(y_primary_value, scene.primary.anchor)
                        .clamp(panel.plot.y, panel.plot.y + panel.plot.height);

                    (y_primary_unit, y_plot)
                } else {
                    let y_primary_unit = scene.primary.domain.active().unit_at_ratio(value_ratio);
                    let y_plot = scene
                        .map_primary_plot_unit(y_primary_unit)
                        .clamp(panel.plot.y, panel.plot.y + panel.plot.height);

                    (y_primary_unit, y_plot)
                };

                (Some(y_primary_unit), None, snapped_y_plot)
            }
            KlinePanelKind::Indicator => {
                if self.indicator_is_unavailable(
                    self.indicators().indicator_panel_value_id(panel_index),
                ) {
                    (None, None, panel.plot.y + y_in_panel)
                } else {
                    let indicator_ratio = 1.0 - (y_in_panel / panel.plot.height.max(1.0));
                    let y_indicator_unit = scene
                        .indicator_panel_config(panel_index)
                        .map(|indicator| indicator.domain.active().unit_at_ratio(indicator_ratio))
                        .unwrap_or_else(|| {
                            YUnitRange::new(YUnit(0), YUnit(1)).unit_at_ratio(indicator_ratio)
                        });
                    let snapped_y_plot = scene
                        .map_indicator_plot_unit(panel_index, y_indicator_unit)
                        .map(|y| y.clamp(panel.plot.y, panel.plot.y + panel.plot.height))
                        .unwrap_or(panel.plot.y + y_in_panel);

                    (None, Some(y_indicator_unit), snapped_y_plot)
                }
            }
        };

        scene.interaction.cursor = Some(CursorInfo {
            x_unit: snapped_x_unit,
            panel_index,
            y_primary_unit,
            y_indicator_unit,
            x_plot: snapped_x_plot,
            y_plot: snapped_y_plot,
        });
    }

    fn ensure_scene_base<'s>(
        &self,
        state: &'s mut State,
        layout: Layout<'_>,
    ) -> Option<&'s SceneCacheEntry> {
        let Some(panel_layout) = PanelLayoutTree::from_layout(layout, &self.composition.panels)
        else {
            state.scene_cache = SceneCache::default();
            return None;
        };

        state
            .scene_cache
            .get_or_build(self.version, panel_layout, |panel_layout| {
                let base = self.compute_scene_base(panel_layout)?;
                let scene = self.build_scene_from_base(
                    &base,
                    layout.bounds(),
                    mouse::Cursor::Unavailable,
                    SceneActivity::Idle,
                    &state.study_objects,
                )?;
                let labels = self.axes(&scene).y_labels();
                Some((base, labels))
            })
    }

    fn cached_scene(
        &self,
        state: &State,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) -> Option<Scene> {
        let root_bounds = layout.bounds();
        let entry = state.scene_cache.get(self.version)?;
        let activity = state
            .interaction
            .drag_action()
            .map_or(SceneActivity::Idle, DragAction::scene_activity);

        let mut scene = self.build_scene_from_base(
            &entry.base,
            root_bounds,
            cursor,
            activity,
            &state.study_objects,
        )?;

        scene.interaction.target = state.interaction.resolve_cursor_target(
            scene.interaction.target,
            cursor
                .position_in(root_bounds)
                .map_or(LayoutHitZone::Outside, |local| scene.layout.hit_test(local)),
            &self.drawing_snapshot,
        );

        Some(scene)
    }

    fn compute_scene_base(&self, panel_layout: PanelLayoutTree) -> Option<SceneBase> {
        let primary_panel = panel_layout
            .panels
            .iter()
            .position(|panel| panel.kind == KlinePanelKind::PrimaryChart)?;
        let primary_mark = self.resolved_panel_mark(primary_panel, KlinePanelKind::PrimaryChart);
        let primary_scale_mode = self.resolved_panel_scale_mode(primary_panel);
        let primary_plot = panel_layout.panel(primary_panel)?.plot;

        let x_axis_plot_width = panel_layout.primary_plot_width()?;
        let horizontal_world = self.horizontal_world;
        let autoscale_plot_width = (x_axis_plot_width + self.autoscale_plot_width_offset).max(1.0);

        let x_axis = XAxis::for_series(self.basis, self.series)?;
        let data_bounds = x_axis.data_bounds(self.series)?;
        let data_max_x = data_bounds.1;
        let output_offset = self.indicator_instances.max_positive_output_offset();
        let (min_x_unit, max_x_unit, right_x_unit) = x_axis.visible_window(
            data_bounds,
            output_offset,
            horizontal_world,
            x_axis_plot_width,
        );
        let (autoscale_min_x_unit, autoscale_max_x_unit, _) = x_axis.visible_window(
            data_bounds,
            output_offset,
            horizontal_world,
            autoscale_plot_width,
        );
        let source_offsets = self.indicator_instances.source_offsets();
        let (source_min_x_unit, source_max_x_unit) =
            x_axis.source_window(autoscale_min_x_unit, autoscale_max_x_unit, &source_offsets);
        let bar_units: Arc<[BarUnitWindow]> =
            Arc::from(self.build_bar_units(x_axis, source_min_x_unit, source_max_x_unit));
        let x_world = horizontal_world.axis_at_right_unit(right_x_unit);
        let fractional_x_offset = right_x_unit - max_x_unit as f32;
        let domains = scene::domain::WindowDomains {
            windows: data::chart::kline::series::KlineWindows {
                series: self.series,
                windows: bar_units.as_ref(),
                axis: x_axis,
            },
            indicators: self.indicators(),
            coordinates: self.coordinates(),
            min_x_unit: autoscale_min_x_unit,
            max_x_unit: autoscale_max_x_unit,
            include_footprint_range: self.trade_data_enabled && self.footprint.visible,
        };
        let primary_value_precision = self.panel_value_precision(primary_panel);
        let primary_value_scale = self.panel_value_scale(primary_panel);

        let indicator_panels: Vec<IndicatorPanelScene> = panel_layout
            .panels
            .iter()
            .enumerate()
            .filter_map(|(panel_index, panel)| {
                if panel.kind != KlinePanelKind::Indicator {
                    return None;
                }

                let scale_mode = self.resolved_panel_scale_mode(panel_index);

                let value_id = self.indicators().indicator_panel_value_id(panel_index);
                let output_layers = self
                    .indicators()
                    .panel_output_layers(panel_index)
                    .into_iter()
                    .filter(|layer| layer.visible && layer.purpose.is_plot())
                    .collect::<Vec<_>>();
                let value_precision = self.panel_value_precision(panel_index);
                let value_scale = self.panel_value_scale(panel_index);
                let independent_domain_units =
                    domains.indicator_domain(&output_layers, value_precision, true, scale_mode);
                let independent_domain = independent_domain_units
                    .map(|(min_unit, max_unit)| YUnitRange::new(min_unit, max_unit));
                let (min_unit, max_unit) = domains
                    .indicator_domain(&output_layers, value_precision, false, scale_mode)
                    .or(independent_domain_units)
                    .unwrap_or((YUnit(0), YUnit(1)));
                let domain = PanelDomainState::new(YUnitRange::new(min_unit, max_unit));

                Some(IndicatorPanelScene {
                    panel_index,
                    value_id,
                    value_scale,
                    domain,
                    independent_domain,
                    presentation: self
                        .composition
                        .panels
                        .get(panel_index)
                        .map(|panel| panel.presentation)
                        .unwrap_or_default(),
                })
            })
            .collect();

        let resolve_primary_scale = |domains: &scene::domain::WindowDomains<'_>| {
            domains
                .primary_domain(primary_value_precision, primary_mark, primary_plot.height)
                .zip(domains.percent_scale_state(primary_mark, primary_scale_mode))
        };
        let (
            (min_primary_unit, max_primary_unit),
            PrimaryPercentScaleState {
                primary_scale_anchor,
                series_percent_anchors,
                primary_domain_display_override,
            },
        ) = resolve_primary_scale(&domains).or_else(|| {
            let fallback_min = data_max_x
                .saturating_sub(autoscale_max_x_unit.saturating_sub(autoscale_min_x_unit));
            let (source_min, source_max) =
                x_axis.source_window(fallback_min, data_max_x, &source_offsets);
            let fallback_bar_units = self.build_bar_units(x_axis, source_min, source_max);
            let fallback_domains = scene::domain::WindowDomains {
                windows: data::chart::kline::series::KlineWindows {
                    series: self.series,
                    windows: &fallback_bar_units,
                    axis: x_axis,
                },
                min_x_unit: fallback_min,
                max_x_unit: data_max_x,
                ..domains
            };
            resolve_primary_scale(&fallback_domains)
        })?;

        let panel_controls =
            chrome::build_panel_control_hits(self.coordinates(), &panel_layout, primary_panel);
        let corner_controls = chrome::build_corner_control_hits(&panel_layout, primary_scale_mode);
        let primary_value_step = primary_value_scale.quantization_step_f32();
        let primary_value_decimals = primary_value_precision
            .and_then(|precision| precision.decimals(self.base_ticker_info()));
        Some(SceneBase {
            layout: panel_layout,
            x: XViewport {
                axis: x_axis,
                world: x_world,
                fractional_offset: fractional_x_offset,
                min_unit: min_x_unit,
                max_unit: max_x_unit,
            },
            bar_units,
            primary: PrimaryScale {
                panel_index: primary_panel,
                domain: PanelDomainState::new(YUnitRange::new(min_primary_unit, max_primary_unit)),
                display_domain_override: primary_domain_display_override,
                value_scale: primary_value_scale,
                mark: primary_mark,
                mode: primary_scale_mode,
                inverted: self.primary_scale_inverted,
                anchor: primary_scale_anchor,
                value_step: primary_value_step,
                value_decimals: primary_value_decimals,
                series_percent_anchors,
            },
            indicator_panels,
            panel_controls,
            corner_controls,
        })
    }

    fn build_scene_from_base(
        &self,
        base: &SceneBase,
        root_bounds: Rectangle,
        cursor: mouse::Cursor,
        activity: SceneActivity,
        layers: &StudyStates,
    ) -> Option<Scene> {
        let primary_panel = base.primary.panel_index;
        let primary_plot = base.layout.panel(primary_panel)?.plot;
        let cursor_root_local = cursor.position_in(root_bounds);
        let hovering_plot = cursor_root_local
            .map(|local| matches!(base.layout.hit_test(local), LayoutHitZone::PanelPlot(_)))
            .unwrap_or(false);
        let show_legend_values = self.legend_visibility.legend_values_always_visible()
            || (activity == SceneActivity::Idle && hovering_plot);
        let show_legends = self.legend_visibility.legend_always_visible() || hovering_plot;
        let keep_ticker_names = self.series.len() > 1;

        let mut scene = Scene::from_base(base);
        self.viewports().apply_panel_viewports(&mut scene);
        scene.chrome.ticker_legend = if show_legends || keep_ticker_names {
            self.legend_view().build_ticker_legend_layout(
                &base.layout,
                primary_panel,
                show_legend_values,
                false,
            )
        } else {
            None
        };

        if self.trade_data_enabled && show_legends {
            scene.chrome.footprint_legend = chrome::legend::FootprintLegendLayout::new(
                &base.layout,
                primary_panel,
                chrome::legend::tooltip_rows_before_studies(
                    scene.chrome.ticker_legend.as_ref(),
                    None,
                    show_legend_values,
                ),
                self.footprint.cluster_kind,
            );
        }
        if show_legends {
            scene.chrome.study_legend = self.legend_view().build_study_legend_layout(
                &base.layout,
                primary_panel,
                chrome::legend::tooltip_rows_before_studies(
                    scene.chrome.ticker_legend.as_ref(),
                    scene.chrome.footprint_legend.as_ref(),
                    show_legend_values,
                ),
            );
            scene.chrome.indicator_legend = self.legend_view().build_indicator_legend_layout(
                &base.layout,
                primary_panel,
                chrome::legend::tooltip_rows_before_indicators(
                    scene.chrome.ticker_legend.as_ref(),
                    scene.chrome.footprint_legend.as_ref(),
                    scene.chrome.study_legend.as_ref(),
                    show_legend_values,
                ),
            );
        }

        self.apply_cursor_interaction(
            &mut scene,
            cursor_root_local,
            primary_panel,
            primary_plot,
            show_legend_values,
            activity,
            layers,
        );
        Some(scene)
    }

    fn fill_panel_titles(&self, frame: &mut PaintInputs, scene: &Scene, palette: &Extended) {
        for (panel_index, panel) in scene.layout.panels.iter().enumerate() {
            let Some(title) = self
                .coordinates()
                .resolved_panel_title(panel_index, panel.kind)
            else {
                continue;
            };

            if panel.kind == KlinePanelKind::Indicator
                && self
                    .indicators()
                    .indicator_panel_value_id(panel_index)
                    .is_some()
            {
                continue;
            }

            let value_id = self.indicators().indicator_panel_value_id(panel_index);
            let title_x = value_id.map_or(panel.plot.x + TOOLTIP_ROW_LEFT_PAD, |value_id| {
                self.legend_view().indicator_label_x(panel.plot.x, value_id)
            });
            let title_y = panel.plot.y + PANEL_TITLE_TOP_PAD + TICKER_LEGEND_ROW_H * 0.5;

            if let Some(value_id) = value_id
                && let Some((marker, loading)) = self.legend_view().indicator_marker(value_id)
            {
                frame.fill_text(canvas::Text {
                    content: marker.to_owned(),
                    position: Point::new(chrome::legend::indicator_marker_x(panel.plot.x), title_y),
                    color: if loading {
                        palette.background.base.text.scale_alpha(0.72)
                    } else {
                        palette.danger.base.color
                    },
                    size: TEXT_SIZE.into(),
                    align_x: iced::Alignment::Start.into(),
                    align_y: iced::Alignment::Center.into(),
                    font: style::AZERET_MONO,
                    ..Default::default()
                });
            }

            frame.fill_text(canvas::Text {
                content: title.to_string(),
                position: Point::new(title_x, title_y),
                color: palette.background.base.text,
                size: TEXT_SIZE.into(),
                align_x: iced::Alignment::Start.into(),
                align_y: iced::Alignment::Center.into(),
                font: style::AZERET_MONO,
                ..Default::default()
            });
        }
    }

    fn fill_y_axis_labels(
        &self,
        frame: &mut PaintInputs,
        scene: &Scene,
        measured: Option<&AxisInputs>,
        palette: &Extended,
    ) {
        let fallback;
        let labels = match measured {
            Some(labels) => labels,
            None => {
                fallback = self.axes(scene).y_labels();
                &fallback
            }
        };
        let centers: Vec<f32> = self
            .overlay_view(scene)
            .series_end_label_positions(scene)
            .into_iter()
            .map(|(_, center)| center)
            .collect();
        labels.paint(frame, palette, &centers);
    }

    fn normalized_panel_splits(&self) -> Vec<f32> {
        if self.panel_count() <= 1 {
            Vec::new()
        } else {
            self.composition.normalized_splits(DEFAULT_MIN_PANEL_RATIO)
        }
    }

    pub fn with_study_object(mut self, object: StudyObject<'a>) -> Self {
        self.study_objects.register(object);
        self
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
        self.study_objects
            .reconcile_states(&mut state.study_objects);
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
            let current_object = self.study_objects.get(key);
            let previous_state = state.study_objects.get(&key);
            let state_matches = match (current_object, previous_state) {
                (Some(object), Some(previous_state)) => object.matches_state(previous_state),
                (None, None) => true,
                _ => false,
            };
            if !state_matches {
                let _ = state.interaction.cancel_pointer();
                state.scheduler.request(RenderRequest::All);
            }
        }
        self.study_objects
            .reconcile_states(&mut state.study_objects);
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
            let row_node = iced_layout::next_to_each_other(
                &limits.shrink(Size::new(0.0, X_AXIS_HEIGHT)),
                0.0,
                |l| {
                    let stack_node = iced_layout::atomic(
                        &l.shrink(Size::new(y_axis_gutter, 0.0)),
                        Length::Fill,
                        Length::Fill,
                    );

                    layout::build_panel_stack(
                        panel_count,
                        &self.normalized_panel_splits(),
                        stack_node.size(),
                        PANEL_SPLITTER_HEIGHT,
                    )
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
                            .indicators()
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
            let mut gutter = state.y_axis_gutter_width;
            let mut node = build_layout(gutter);

            for _ in 0..2 {
                let required = self
                    .ensure_scene_base(state, iced_layout::Layout::new(&node))
                    .map(|entry| entry.y_axis_labels.required_gutter());

                let Some(required) = required else {
                    break;
                };

                if (required - gutter).abs() <= f32::EPSILON {
                    break;
                }

                gutter = required;
                node = build_layout(gutter);
            }

            state.y_axis_gutter_width = gutter;
            state.gutter_cache_key = Some(gutter_cache_key);
            gutter
        };

        let node = build_layout(y_axis_gutter);
        self.ensure_scene_base(state, iced_layout::Layout::new(&node));
        node
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

        let state = tree.state.downcast_mut::<State>();

        match event {
            Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                physical_key,
                modifiers,
                ..
            }) => {
                state.interaction.set_modifiers(*modifiers);
                state.scheduler.request(RenderRequest::Interaction);

                if self.should_capture_escape
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
            Event::Keyboard(
                keyboard::Event::KeyReleased { modifiers, .. }
                | keyboard::Event::ModifiersChanged(modifiers),
            ) => {
                state.interaction.set_modifiers(*modifiers);
                state.scheduler.request(RenderRequest::Interaction);
                shell.publish(M::from(KlineWidgetEvent::from(
                    KlineWidgetDrawingEvent::MagnetModifierChanged(Self::is_command_modifier(
                        *modifiers,
                    )),
                )));
            }
            Event::Mouse(mouse_event) => {
                if matches!(mouse_event, mouse::Event::WheelScrolled { .. })
                    && matches!(
                        state.interaction.drag_action(),
                        Some(DragAction::XAxisScale { .. } | DragAction::YAxisScale { .. })
                    )
                {
                    shell.capture_event();
                    return;
                }

                let bounds = layout.bounds();
                if matches!(
                    mouse_event,
                    mouse::Event::ButtonReleased(mouse::Button::Left)
                ) {
                    if state.interaction.motion().is_some() {
                        self.finish_interaction_release(
                            state,
                            layout,
                            cursor,
                            cursor.position_in(bounds).is_none(),
                            shell,
                        );
                    }
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
                        state.scheduler.request(RenderRequest::Interaction);
                        return;
                    }
                } else {
                    state.scheduler.request(RenderRequest::Interaction);
                    return;
                };

                let pointer_delta = match mouse_event {
                    mouse::Event::CursorMoved { .. } => {
                        state.scheduler.request(RenderRequest::Interaction);
                        match state.interaction.pointer_moved(cursor_pos) {
                            PointerMove::Pending => {
                                shell.capture_event();
                                return;
                            }
                            PointerMove::Moving { delta } => delta,
                            PointerMove::Idle => Vector::new(0.0, 0.0),
                        }
                    }
                    mouse::Event::ButtonPressed(mouse::Button::Right)
                        if self.drawing_snapshot.drawing_draft.is_some() =>
                    {
                        shell.publish(M::from(KlineWidgetEvent::from(
                            KlineWidgetDrawingEvent::DraftCanceled,
                        )));
                        self.cancel_pointer(state, layout, cursor, shell);
                        state.scheduler.request(RenderRequest::All);
                        shell.capture_event();
                        return;
                    }
                    mouse::Event::WheelScrolled { .. }
                    | mouse::Event::ButtonPressed(mouse::Button::Left) => Vector::new(0.0, 0.0),
                    _ => return,
                };

                let Some(layout_tree) = state
                    .scene_cache
                    .get(self.version)
                    .map(|entry| entry.base.layout.clone())
                    .or_else(|| PanelLayoutTree::from_layout(layout, &self.composition.panels))
                else {
                    return;
                };
                let zone = layout_tree.hit_test(cursor_pos);
                let scene = self.cached_scene(state, layout, cursor);
                let cursor_target = scene.as_ref().map(|scene| scene.interaction.target);

                match mouse_event {
                    mouse::Event::WheelScrolled { delta } => {
                        let effects = self.navigation().wheel(
                            *delta,
                            zone,
                            cursor_pos,
                            state.interaction.modifiers(),
                            scene.as_ref(),
                        );
                        effects.apply(&mut state.scheduler, shell, M::from);
                    }
                    mouse::Event::ButtonPressed(mouse::Button::Left) => {
                        let chrome_click =
                            scene
                                .as_ref()
                                .map_or(chrome::ChromeClick::Outside, |scene| {
                                    scene
                                        .chrome
                                        .resolve_click(cursor_target, self.study_legend.as_ref())
                                });
                        match chrome_click {
                            chrome::ChromeClick::Event(event) => {
                                shell.publish(M::from(event));
                                state.scheduler.request(RenderRequest::All);
                                shell.capture_event();
                                return;
                            }
                            chrome::ChromeClick::Blocked => return,
                            chrome::ChromeClick::Outside => {}
                        }

                        let click_kind = state.interaction.begin_press(
                            cursor_pos,
                            cursor.position(),
                            PressAction::Ordinary,
                        );

                        if let Some(effects) = self.navigation().axis_reset(zone, click_kind) {
                            effects.apply(&mut state.scheduler, shell, M::from);
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
                                && let Some(scene) = scene.as_ref()
                                && let Some(anchor) = self
                                    .drawing_view(scene)
                                    .drawing_anchor_from_scene_point(panel_id, cursor_pos)
                            {
                                shell.publish(M::from(KlineWidgetEvent::from(
                                    KlineWidgetDrawingEvent::MeasureClick(anchor),
                                )));
                                state.scheduler.request(RenderRequest::All);
                                shell.capture_event();
                                return;
                            }
                        }
                        if let Some(hit) = layer_hit.filter(|hit| hit.precedes_tools)
                            && let Some(scene) = scene.as_ref()
                            && self.begin_study_object_interaction(
                                hit.key, scene, cursor_pos, state, shell,
                            )
                        {
                            return;
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
                            state.scheduler.request(RenderRequest::Interaction);
                            shell.capture_event();
                            return;
                        }

                        if matches!(
                            zone,
                            LayoutHitZone::Splitter(_)
                                | LayoutHitZone::YAxis(_)
                                | LayoutHitZone::PanelXAxis(_)
                                | LayoutHitZone::BottomXAxis
                        ) {
                            if let Some(effects) = self.navigation().begin_axis_drag(
                                zone,
                                &layout_tree,
                                scene.as_ref(),
                                cursor_pos,
                                &mut state.interaction,
                            ) {
                                effects.apply(&mut state.scheduler, shell, M::from);
                            } else {
                                self.cancel_pointer(state, layout, cursor, shell);
                            }
                        } else if !self.drawing_snapshot.allows_panning() {
                            let magnetic =
                                self.drawing_magnet_enabled(state.interaction.modifiers());
                            let panel_id = self.drawing_snapshot.draft_panel_id().or_else(|| {
                                if let LayoutHitZone::PanelPlot(panel_index) = zone {
                                    self.panel_id(panel_index)
                                } else {
                                    None
                                }
                            });
                            let anchor = panel_id.and_then(|panel_id| {
                                self.drawing_view(scene.as_ref()?)
                                    .drawing_draft_anchor_from_scene_point(
                                        panel_id, cursor_pos, magnetic,
                                    )
                            });

                            if let Some(anchor) = anchor {
                                state
                                    .interaction
                                    .begin_click(cursor_pos, PressAction::DrawingPlacement);
                                shell.publish(M::from(KlineWidgetEvent::from(
                                    KlineWidgetDrawingEvent::AnchorPressed(anchor),
                                )));
                                state.scheduler.request(RenderRequest::All);
                                shell.capture_event();
                            } else {
                                self.cancel_pointer(state, layout, cursor, shell);
                            }
                        } else if self.drawing_snapshot.allows_panning()
                            && let LayoutHitZone::PanelPlot(panel_index) = zone
                        {
                            state.interaction.begin_click_or_drag(
                                cursor_pos,
                                PressAction::Ordinary,
                                DragAction::Pan {
                                    panel_index,
                                    source_layer: None,
                                },
                            );
                        }
                    }
                    mouse::Event::CursorMoved { .. } => {
                        if let Some(DrawingMeasure::Pending(measure_state)) =
                            self.drawing_snapshot.measure
                            && let Some(scene) = scene.as_ref()
                            && let Some(anchor) = self
                                .drawing_view(scene)
                                .drawing_anchor_from_scene_point(measure_state.panel_id, cursor_pos)
                        {
                            shell.publish(M::from(KlineWidgetEvent::from(
                                KlineWidgetDrawingEvent::MeasureMove(anchor),
                            )));
                        }

                        let drag_action = state.interaction.drag_action();
                        if let Some(DragAction::Layer { key, .. }) = drag_action {
                            if let Some(scene) = scene.as_ref()
                                && let Some(effects) = self.study_objects.pointer(
                                    key,
                                    LayerPointerInput::Moved,
                                    self.layer_context(
                                        scene,
                                        Some(cursor_pos),
                                        state.interaction.modifiers(),
                                    ),
                                    &mut state.study_objects,
                                )
                            {
                                effects.apply(&mut state.scheduler, shell, |event| {
                                    M::from(KlineWidgetEvent::Study(event))
                                });
                            }
                        } else if let Some(DragAction::Drawing { .. }) = drag_action {
                            if let Some(scene) = scene.as_ref() {
                                self.apply_drawing_pointer(
                                    DrawingPointerInput::Moved,
                                    self.layer_context(
                                        scene,
                                        Some(cursor_pos),
                                        state.interaction.modifiers(),
                                    ),
                                    state,
                                    shell,
                                );
                            }
                        } else if let Some(
                            action @ (DragAction::XAxisScale { .. }
                            | DragAction::Split(_)
                            | DragAction::YAxisScale { .. }),
                        ) = drag_action
                        {
                            if let Some(motion) = state.interaction.motion() {
                                let effects = self.navigation().drag(
                                    action,
                                    motion,
                                    pointer_delta,
                                    &layout_tree,
                                    scene.as_ref(),
                                );
                                effects.apply(&mut state.scheduler, shell, M::from);
                            }
                        } else if state.marquee().is_some() {
                            state.scheduler.request(RenderRequest::Interaction);
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
                            && let Some(scene) = scene.as_ref()
                            && let Some(anchor) = self
                                .drawing_view(scene)
                                .drawing_draft_anchor_from_scene_point(
                                    draft_panel_id,
                                    cursor_pos,
                                    self.drawing_magnet_enabled(state.interaction.modifiers()),
                                )
                        {
                            shell.publish(M::from(KlineWidgetEvent::from(
                                KlineWidgetDrawingEvent::AnchorMoved(anchor),
                            )));
                            state.scheduler.request(RenderRequest::Interaction);
                            shell.capture_event();
                        } else if let Some(action @ DragAction::Pan { .. }) = drag_action
                            && let Some(motion) = state.interaction.motion()
                        {
                            let effects = self.navigation().drag(
                                action,
                                motion,
                                pointer_delta,
                                &layout_tree,
                                scene.as_ref(),
                            );
                            effects.apply(&mut state.scheduler, shell, M::from);
                        }
                    }
                    _ => {}
                }
            }
            Event::Window(window::Event::RedrawRequested(now)) => {
                let layout_tree = state
                    .scene_cache
                    .get(self.version)
                    .map(|entry| entry.base.layout.clone())
                    .or_else(|| PanelLayoutTree::from_layout(layout, &self.composition.panels));
                if let Some(panel_layout) = layout_tree.as_ref()
                    && (state.scene_cache.get(self.version).is_some()
                        || state.scheduler.presentation().is_some())
                {
                    let request = state.scheduler.admit(
                        self.version,
                        *now,
                        layout.bounds(),
                        panel_layout,
                        self.pixel_ratio,
                        self.theme.accents().generation(),
                    );
                    let scene = (request != RenderRequest::None)
                        .then(|| self.cached_scene(state, layout, cursor))
                        .flatten();
                    if let Some(scene) = scene.as_ref() {
                        for (panel_id, range) in self.viewports().reconciled_y_ranges(scene) {
                            shell.publish(M::from(KlineWidgetEvent::PanelInteraction(
                                PanelInteraction::YViewportChanged { panel_id, range },
                            )));
                        }
                    }
                    let utc_ms = chrono::Utc::now().timestamp_millis() as u64;
                    self.prepare_render_inputs(
                        state,
                        scene.as_ref(),
                        layout.bounds(),
                        request,
                        utc_ms,
                    );
                    if let Some(deadline) = state.scheduler.data_deadline(self.version, *now) {
                        shell.request_redraw_at(deadline);
                    }
                }

                if let Some(plot_width) = layout_tree.and_then(|layout| layout.primary_plot_width())
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
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();

        use advanced::Renderer as _;
        use advanced::graphics::geometry::Renderer as _;
        use advanced::text::Renderer as _;
        for step in PAINT_ORDER {
            match step {
                PaintStep::AxisBadges => {
                    if let Some(chrome) = &state.axis_badges {
                        chrome.draw(renderer);
                    }
                }
                PaintStep::StudyChrome => {
                    if let Some(controls) = &state.study_chrome {
                        controls.draw(renderer);
                    }
                }
                PaintStep::Legends => {
                    if let Some(legends) = &state.legends {
                        legends.draw(renderer);
                    }
                }
                PaintStep::ChartSeparators => {
                    if let Some(environment) = state.scheduler.presentation() {
                        let origin = environment.bounds.position();

                        renderer.with_translation(Vector::new(origin.x, origin.y), |renderer| {
                            renderer.with_layer(
                                Rectangle::with_size(environment.bounds.size()),
                                |renderer| {
                                    let layout = &environment.layout;
                                    let plot_rect = layout.regions.plot;
                                    let splitter_color = style::split_ruler(theme).color;

                                    for splitter in &layout.splitters {
                                        renderer.fill_quad(
                                            Quad {
                                                bounds: Rectangle {
                                                    x: plot_rect.x + splitter.x,
                                                    y: plot_rect.y + splitter.y,
                                                    width: splitter.width
                                                        + layout.regions.y_axis.width,
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
                                                width: plot_rect.width
                                                    + layout.regions.y_axis.width,
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
                                },
                            );
                        });
                    }
                }
                PaintStep::PanelControls => {
                    if let Some(controls) = &state.panel_controls {
                        renderer.with_translation(
                            Vector::new(controls.bounds.x, controls.bounds.y),
                            |renderer| {
                                let clip = Rectangle::with_size(controls.bounds.size());
                                renderer.with_layer(clip, |renderer| {
                                    for control in &controls.buttons {
                                        renderer.fill_quad(
                                            Quad {
                                                bounds: control.bounds,
                                                snap: false,
                                                ..Default::default()
                                            },
                                            control.background,
                                        );
                                        renderer.fill_text(
                                            control.text.clone(),
                                            control.bounds.center(),
                                            control.text_color,
                                            clip,
                                        );
                                    }
                                });
                            },
                        );
                    }
                }
                PaintStep::Canvas(layer_kind) => {
                    let cache = &state.layers[layer_kind];
                    let Some(layer) = cache.resolved() else {
                        continue;
                    };
                    let geometry = cache
                        .geometry
                        .draw(renderer, layer.placement.size(), |frame| {
                            layer.inputs.replay(frame)
                        });

                    renderer.with_translation(
                        Vector::new(layer.root_origin.x, layer.root_origin.y),
                        |renderer| match layer.placement {
                            LayerPlacement::Axis(bounds) => renderer
                                .with_translation(Vector::new(bounds.x, bounds.y), |renderer| {
                                    renderer.draw_geometry(geometry)
                                }),
                            LayerPlacement::Plot(bounds) => {
                                renderer.with_layer(bounds, |renderer| {
                                    renderer.with_translation(
                                        Vector::new(bounds.x, bounds.y),
                                        |renderer| renderer.draw_geometry(geometry),
                                    );
                                })
                            }
                            LayerPlacement::Widget(size) => {
                                let clip = if layer_kind == CanvasLayer::SavedDrawings {
                                    let Some(environment) = state.scheduler.presentation() else {
                                        return;
                                    };
                                    environment.layout.regions.plot
                                } else {
                                    Rectangle::with_size(size)
                                };
                                renderer
                                    .with_layer(clip, |renderer| renderer.draw_geometry(geometry))
                            }
                        },
                    );
                }
            }
        }
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

        if state.interaction.motion().is_none() && !cursor.is_over(layout.bounds()) {
            return advanced::mouse::Interaction::default();
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
