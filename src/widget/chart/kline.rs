mod axes;
mod chrome;
pub mod composition;
pub mod coord;
pub mod drawing;
mod footprint;
mod helpers;
mod layout;
mod overlay;
mod primary_render;
mod scale;
mod scene;
mod scene_builder;

use crate::style;
use crate::widget::chart::kline::drawing::{
    AxisBadgeFilter, DrawingDragOperation, DrawingDragTarget, DrawingId, DrawingMeasure,
    DrawingSnapshot, DrawingTool, KlineWidgetDrawingEvent,
};
use axes::horizontal_world_for_drag;
use chrome::{FootprintLegendHit, IndicatorLegendHit, StudyLegendHit, TickerLegendHit};
use composition::{
    BarMode, ChartComposition, DEFAULT_MIN_PANEL_RATIO, HistogramMode, LayerDataKind, MarkKind,
    PanelId, PanelPresentation, PanelScaleMode, PanelValueId,
};
use data::chart::kline_v2::indicators::{
    IndicatorCellValue, TableColumnColor, TableColumnFormat, TableColumnValue, table_columns,
};
use data::chart::kline_v2::{
    HorizontalWorld, IndicatorInstanceStore, LineStrokeStyle, MIN_LINE_WIDTH, SeriesColor,
};
pub use data::chart::kline_v2::{
    IndicatorOverlayChannelSpec as OverlayChannelSpec,
    IndicatorOverlayColorRole as OverlayChannelColorRole,
    IndicatorOverlaySource as OverlayChannelSource,
};
use data::config::theme::generated_theme_accents;
use layout::{KlinePanelKind, LayoutHitZone, PanelLayoutNode, PanelLayoutTree};
pub use scale::DEFAULT_BAR_SPACING_PX;
pub use scale::HorizontalScale;
use scale::{BarSpacingPx, HORIZONTAL_DRAG_SCALE_DELTA_PER_PX, normalized_scroll_delta};
use scene::{Scene, TickIndex, XAxis};

use data::UserTimezone;
use data::chart::Basis;
use data::chart::kline::{ClusterKind, KlineTrades, NPoc};
use data::chart::kline_v2::series::{
    AnchoredPriceLevelTradeDistribution, FootprintPocLine, PriceLevelTradeDistribution,
};
use data::chart::kline_v2::{
    CandlestickColorSlot, CandlestickConfig, FootprintPocColorSlot, FootprintPocConfig, KlineStudy,
    KlineStudyId, VolumeProfileColorConfig, VolumeProfileColorSlot, VolumeProfileSchedule,
};
pub(crate) use data::chart::kline_v2::{PanelYViewport, YUnit};
use data::chart::ticks::{
    Y_LABEL_DENSITY,
    x::{TimeTickLabel, TimeTickTier},
    x_labels_that_fit,
    y::{TickValue, YAxisScale},
    y_labels_that_fit,
};
use exchange::{
    Kline, TickerInfo, Timeframe, UnixMs,
    unit::{Price, PriceStep},
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
const MIN_FULL_CACHE_REFRESH_INTERVAL: Duration = Duration::from_micros(33_333);

const TEXT_SIZE: f32 = 12.0;
const CHAR_W: f32 = TEXT_SIZE * 0.64;
const Y_AXIS_LABEL_LEFT_INSET: f32 = 4.0;
const PANEL_SPLITTER_HEIGHT: f32 = 1.0;
const PANEL_TITLE_TOP_PAD: f32 = 4.0;
const Y_AXIS_DRAG_SCALE_DELTA_PER_PX: f32 = 0.1;

const TICKER_LEGEND_PADDING: f32 = 4.0;
const TOOLTIP_ROW_LEFT_PAD: f32 = TICKER_LEGEND_PADDING * 2.0;
const TICKER_LEGEND_ROW_H: f32 = TEXT_SIZE + 6.0;

const FOOTPRINT_TEXT_MIN_SIZE_PX: f32 = 6.0;

const DEFAULT_OVERLAY_CHANNELS: [OverlayChannelSpec; 1] = [OverlayChannelSpec {
    label: "V",
    source: OverlayChannelSource::Primary,
    line_width: 1.1,
    color_role: OverlayChannelColorRole::Neutral,
    color_slot: data::chart::kline_v2::IndicatorColorSlot::Main,
}];

pub use data::chart::kline_v2::values::IndicatorData;

pub trait KlineSeriesLike {
    fn ticker_info(&self) -> &TickerInfo;

    fn series_color(&self) -> SeriesColor {
        SeriesColor::default()
    }

    fn series_line_width(&self) -> u8 {
        MIN_LINE_WIDTH
    }

    fn series_stroke_style(&self) -> LineStrokeStyle {
        LineStrokeStyle::Solid
    }

    fn bars(&self) -> &[Kline];

    fn footprint_tick_size(&self) -> PriceStep {
        self.ticker_info().min_ticksize.into()
    }

    fn time_at_index(&self, index: usize) -> Option<UnixMs> {
        self.bars().get(index).map(|bar| bar.time)
    }

    fn indicator_value(&self, bar: &Kline) -> f32;

    fn footprint_for_bar(&self, _bar: &Kline) -> Option<&KlineTrades> {
        None
    }

    fn trade_price_distribution(
        &self,
        _earliest: UnixMs,
        _latest: UnixMs,
        _lowest: Price,
        _highest: Price,
        _tick_size: PriceStep,
    ) -> Option<PriceLevelTradeDistribution> {
        None
    }

    fn recurring_trade_price_distributions(
        &self,
        _interval: Timeframe,
        _tick_size: PriceStep,
        _earliest: UnixMs,
        _latest: UnixMs,
    ) -> Vec<AnchoredPriceLevelTradeDistribution> {
        Vec::new()
    }

    fn footprint_poc_lines(&self, _earliest: UnixMs, _latest: UnixMs) -> Vec<FootprintPocLine> {
        Vec::new()
    }

    fn footprint_tick_index_offset(&self) -> u64 {
        0
    }

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
}

pub(super) enum DragMode {
    None,
    Split(usize),
    AxisScale {
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
        anchor_x: f32,
        initial_offset: f32,
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

#[derive(Debug, Clone, Copy, PartialEq)]
struct GutterCacheKey {
    version: u64,
    width: f32,
    height: f32,
    panel_count: usize,
}

struct State {
    plot_cache: canvas::Cache,
    y_axis_cache: canvas::Cache,
    x_axis_cache: canvas::Cache,
    overlay_cache: canvas::Cache,
    interaction_text_cache: canvas::Cache,
    marquee_cache: canvas::Cache,
    drag_mode: DragMode,
    last_cursor: Option<Point>,
    modifiers: keyboard::Modifiers,
    y_axis_gutter_width: f32,
    gutter_cache_key: Option<GutterCacheKey>,
    last_cache_rev: u64,
    last_cache_clear_at: Option<Instant>,
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
            marquee_cache: canvas::Cache::new(),
            drag_mode: DragMode::None,
            last_cursor: None,
            modifiers: keyboard::Modifiers::empty(),
            y_axis_gutter_width: Y_AXIS_GUTTER,
            gutter_cache_key: None,
            last_cache_rev: 0,
            last_cache_clear_at: None,
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

    fn cache_refresh_is_due(&self, version: u64, now: Instant, minimum_interval: Duration) -> bool {
        self.last_cache_rev != version
            && self.last_cache_clear_at.is_none_or(|last_clear| {
                now.saturating_duration_since(last_clear) >= minimum_interval
            })
    }

    fn mark_cache_refreshed(&mut self, version: u64, now: Instant) {
        self.last_cache_rev = version;
        self.last_cache_clear_at = Some(now);
    }

    fn clear_overlay_caches(&mut self) {
        self.overlay_cache.clear();
        self.interaction_text_cache.clear();
        self.marquee_cache.clear();
    }
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
    HorizontalWorldChanged {
        cell_width: f32,
        scaling: f32,
        offset: f32,
    },
    HorizontalOffsetChanged(f32),
    PlotWidthChanged(f32),
    ZoomChanged {
        cell_width: f32,
        scaling: f32,
        offset: f32,
        y_viewport: Option<(PanelId, PanelYViewport)>,
    },
    PrimaryAutoscaleDisabled,
    PrimaryAutoscaleToggled,
    PrimaryScaleModeCycleRequested,
    PanelInteraction(PanelInteraction),
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

pub struct KlineWidget<'a, S> {
    series: &'a [S],
    composition: &'a ChartComposition,
    indicator_instances: &'a IndicatorInstanceStore,
    studies: &'a [KlineStudy],
    studies_legend_expanded: bool,
    candlestick_config: CandlestickConfig,
    candlesticks_visible: bool,
    footprints_visible: bool,
    footprints_enabled: bool,
    footprint_cluster_kind: ClusterKind,
    footprint_poc_config: FootprintPocConfig,
    drawings: DrawingSnapshot<'a>,
    basis: Basis,
    horizontal_world: HorizontalWorld,
    known_plot_width: Option<f32>,
    autoscale_plot_width_offset: f32,
    pixel_ratio: f32,
    primary_autoscale: bool,
    panel_y_viewports: &'a [(PanelId, PanelYViewport)],
    unavailable_indicator_panels: Vec<UnavailableIndicator>,
    timezone: UserTimezone,
    version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnavailableIndicator {
    pub value_id: PanelValueId,
    pub message: String,
}

impl<'a, S> KlineWidget<'a, S>
where
    S: KlineSeriesLike,
{
    pub fn new(
        series: &'a [S],
        basis: Basis,
        composition: &'a ChartComposition,
        indicator_instances: &'a IndicatorInstanceStore,
    ) -> Self {
        Self {
            series,
            composition,
            indicator_instances,
            studies: &[],
            studies_legend_expanded: true,
            candlestick_config: CandlestickConfig::default(),
            candlesticks_visible: false,
            footprints_visible: false,
            footprints_enabled: false,
            footprint_cluster_kind: ClusterKind::default(),
            footprint_poc_config: FootprintPocConfig::default(),
            drawings: DrawingSnapshot::new(DrawingTool::Cursor, false, &[], &[], None, None),
            basis,
            horizontal_world: HorizontalWorld::new(DEFAULT_BAR_SPACING_PX, 1.0, 0.0),
            known_plot_width: None,
            autoscale_plot_width_offset: 0.0,
            pixel_ratio: 1.0,
            primary_autoscale: false,
            panel_y_viewports: &[],
            unavailable_indicator_panels: Vec::new(),
            timezone: UserTimezone::Utc,
            version: 0,
        }
    }

    pub fn with_studies(mut self, studies: &'a [KlineStudy]) -> Self {
        self.studies = studies;
        self
    }

    pub fn with_studies_legend_expanded(mut self, expanded: bool) -> Self {
        self.studies_legend_expanded = expanded;
        self
    }

    pub fn with_candlestick_display(mut self, config: CandlestickConfig) -> Self {
        self.candlestick_config = config;
        self.candlesticks_visible = true;
        self
    }

    pub fn with_footprint_display(mut self, cluster_kind: ClusterKind) -> Self {
        self.footprints_visible = true;
        self.footprint_cluster_kind = cluster_kind;
        self
    }

    pub fn with_footprint_poc_config(mut self, config: FootprintPocConfig) -> Self {
        self.footprint_poc_config = config;
        self
    }

    pub fn with_footprint_enabled(mut self, enabled: bool) -> Self {
        self.footprints_enabled = enabled;
        self
    }

    pub fn with_coordinate_state(
        mut self,
        horizontal_world: HorizontalWorld,
        pixel_ratio: f32,
        primary_autoscale: bool,
    ) -> Self {
        self.horizontal_world = horizontal_world;
        self.pixel_ratio = if pixel_ratio.is_finite() && pixel_ratio > 0.0 {
            pixel_ratio
        } else {
            1.0
        };
        self.primary_autoscale = primary_autoscale;
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

    pub fn with_panels(mut self, viewports: &'a [(PanelId, PanelYViewport)]) -> Self {
        self.panel_y_viewports = viewports;
        self
    }

    pub fn with_indicator_availability(mut self, panels: Vec<UnavailableIndicator>) -> Self {
        self.unavailable_indicator_panels = panels;
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

    fn indicator_is_unavailable(&self, value_id: Option<PanelValueId>) -> bool {
        self.indicator_unavailable_message(value_id).is_some()
    }

    pub(super) fn indicator_unavailable_message(
        &self,
        value_id: Option<PanelValueId>,
    ) -> Option<&str> {
        let value_id = value_id?;
        self.unavailable_indicator_panels
            .iter()
            .find(|indicator| indicator.value_id == value_id)
            .map(|indicator| indicator.message.as_str())
    }

    pub(super) fn indicator_label_x(&self, row_x: f32, value_id: PanelValueId) -> f32 {
        let marker_width = if self.indicator_is_unavailable(Some(value_id)) {
            CHAR_W + TICKER_LEGEND_PADDING
        } else {
            0.0
        };
        row_x + TOOLTIP_ROW_LEFT_PAD + marker_width
    }

    pub(super) fn indicator_marker_x(&self, row_x: f32) -> f32 {
        row_x + TOOLTIP_ROW_LEFT_PAD
    }

    pub fn with_version(mut self, rev: u64) -> Self {
        self.version = rev;
        self
    }

    fn is_command_modifier(modifiers: keyboard::Modifiers) -> bool {
        modifiers.intersects(keyboard::Modifiers::COMMAND)
    }

    fn drawing_magnet_enabled(&self, modifiers: keyboard::Modifiers) -> bool {
        self.drawings.active_tool.supports_magnet()
            && (self.drawings.magnet_enabled || Self::is_command_modifier(modifiers))
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

        let y_axis_gutter = if state.gutter_cache_key == Some(gutter_cache_key) {
            state.y_axis_gutter_width
        } else {
            let mut gutter = Y_AXIS_GUTTER;
            let mut node = build_layout(gutter);

            for _ in 0..2 {
                let required = {
                    let provisional_layout = iced_layout::Layout::new(&node);
                    self.compute_scene(provisional_layout, mouse::Cursor::Unavailable)
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

        match event {
            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                let state = tree.state.downcast_mut::<State>();
                state.modifiers = *modifiers;
                state.clear_overlay_caches();
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
                    keyboard::Key::Named(keyboard::key::Named::Escape)
                        if self.drawings.measure.is_some() =>
                    {
                        shell.publish(M::from(KlineWidgetEvent::Drawing(
                            KlineWidgetDrawingEvent::MeasureCanceled,
                        )));
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

                let cursor_pos = if let Some(local) = cursor.position_in(bounds) {
                    local
                } else if matches!(
                    state.drag_mode,
                    DragMode::DrawingMove { .. }
                        | DragMode::XAxisScale { .. }
                        | DragMode::Marquee { .. }
                ) {
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

                let footprint_legend = self
                    .compute_scene(layout, cursor)
                    .and_then(|scene| scene.chrome.footprint_legend);
                let footprint_legend_hit = footprint_legend.as_ref().and_then(|legend| {
                    Self::hit_footprint_legend(&layout_tree, legend, cursor_pos)
                });

                let study_legend = self.build_study_legend_layout(
                    &layout_tree,
                    primary_panel,
                    self.tooltip_rows_before_studies(
                        ticker_legend.as_ref(),
                        footprint_legend.as_ref(),
                        show_legend_values,
                    ),
                    self.studies_legend_expanded,
                );
                let study_legend_hit = study_legend
                    .as_ref()
                    .and_then(|legend| Self::hit_study_legend(&layout_tree, legend, cursor_pos));

                let mut indicator_legend = self.build_indicator_legend_layout(
                    &layout_tree,
                    primary_panel,
                    self.tooltip_rows_before_indicators(
                        ticker_legend.as_ref(),
                        footprint_legend.as_ref(),
                        study_legend.as_ref(),
                        show_legend_values,
                    ),
                );
                let mut indicator_legend_hit = indicator_legend.as_ref().and_then(|legend| {
                    Self::hit_indicator_legend(&layout_tree, legend, cursor_pos)
                });

                if indicator_legend_hit.is_some() {
                    indicator_legend = self.build_indicator_legend_layout(
                        &layout_tree,
                        primary_panel,
                        self.tooltip_rows_before_indicators(
                            ticker_legend.as_ref(),
                            footprint_legend.as_ref(),
                            study_legend.as_ref(),
                            show_legend_values,
                        ),
                    );
                    indicator_legend_hit = indicator_legend.as_ref().and_then(|legend| {
                        Self::hit_indicator_legend(&layout_tree, legend, cursor_pos)
                    });
                }

                match mouse_event {
                    mouse::Event::WheelScrolled { delta } => {
                        let scroll_y = normalized_scroll_delta(delta);

                        if scroll_y.abs() <= f32::EPSILON {
                            return;
                        }

                        match zone {
                            LayoutHitZone::PanelPlot(_)
                            | LayoutHitZone::PanelXAxis(_)
                            | LayoutHitZone::BottomXAxis => {
                                if matches!(zone, LayoutHitZone::PanelPlot(_))
                                    && Self::hit_panel_control(
                                        &layout_tree,
                                        &panel_controls,
                                        cursor_pos,
                                    )
                                    .is_some()
                                {
                                    return;
                                }

                                if matches!(zone, LayoutHitZone::PanelPlot(_))
                                    && Self::hit_corner_control(&corner_controls, cursor_pos)
                                        .is_some()
                                {
                                    return;
                                }

                                if matches!(zone, LayoutHitZone::PanelPlot(_))
                                    && ticker_legend_hit.is_some()
                                {
                                    return;
                                }

                                if matches!(zone, LayoutHitZone::PanelPlot(_))
                                    && footprint_legend_hit.is_some()
                                {
                                    return;
                                }

                                if matches!(zone, LayoutHitZone::PanelPlot(_))
                                    && indicator_legend_hit.is_some()
                                {
                                    return;
                                }

                                if matches!(zone, LayoutHitZone::PanelPlot(_))
                                    && study_legend_hit.is_some()
                                {
                                    return;
                                }

                                let horizontal_zoom_delta = if self.primary_autoscale {
                                    scroll_y / 2.0
                                } else {
                                    scroll_y
                                };

                                if horizontal_zoom_delta.abs() <= f32::EPSILON {
                                    return;
                                }

                                let scene = match zone {
                                    LayoutHitZone::PanelPlot(_)
                                    | LayoutHitZone::PanelXAxis(_)
                                    | LayoutHitZone::BottomXAxis => {
                                        self.compute_scene(layout, cursor)
                                    }
                                    _ => None,
                                };
                                let horizontal_world = scene.as_ref().map(|scene| {
                                    self.horizontal_world_for_cursor(
                                        scene,
                                        cursor_pos.x,
                                        horizontal_zoom_delta,
                                    )
                                });
                                let new_world = horizontal_world.unwrap_or(self.horizontal_world);
                                let horizontal_changed = new_world != self.horizontal_world;
                                let y_zoom_panel = match zone {
                                    LayoutHitZone::PanelPlot(panel_index)
                                        if !self.primary_autoscale =>
                                    {
                                        Some(panel_index)
                                    }
                                    _ => None,
                                };
                                let y_viewport = y_zoom_panel.and_then(|panel_index| {
                                    let scene = scene.as_ref()?;
                                    let panel_id = self.panel_id(panel_index)?;
                                    self.panel_viewport_after_y_zoom(
                                        scene,
                                        panel_index,
                                        cursor_pos.y,
                                        scroll_y,
                                    )
                                    .map(|viewport| (panel_id, viewport))
                                });
                                let mut interaction_changed = false;

                                if horizontal_changed || y_viewport.is_some() {
                                    if y_viewport.is_some() {
                                        shell.publish(M::from(KlineWidgetEvent::ZoomChanged {
                                            cell_width: new_world.cell_width(),
                                            scaling: new_world.scaling(),
                                            offset: new_world.offset(),
                                            y_viewport,
                                        }));
                                    } else {
                                        shell.publish(M::from(
                                            KlineWidgetEvent::HorizontalWorldChanged {
                                                cell_width: new_world.cell_width(),
                                                scaling: new_world.scaling(),
                                                offset: new_world.offset(),
                                            },
                                        ));
                                    }
                                    interaction_changed = true;
                                }

                                if interaction_changed {
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

                        if let Some(FootprintLegendHit::Icon(icon_kind)) = footprint_legend_hit {
                            shell.publish(M::from(icon_kind.into_event()));
                            state.drag_mode = DragMode::None;
                            state.last_cursor = None;
                            state.clear_all_caches();
                            shell.capture_event();
                            return;
                        }

                        if let Some(StudyLegendHit::CategoryIcon(icon_kind)) = study_legend_hit {
                            shell.publish(M::from(icon_kind.into_event()));
                            state.drag_mode = DragMode::None;
                            state.last_cursor = None;
                            state.clear_all_caches();
                            shell.capture_event();
                            return;
                        }

                        if let (Some(legend), Some(StudyLegendHit::StudyIcon(row_index, icon_kind))) =
                            (&study_legend, study_legend_hit)
                            && let Some(row) = legend.rows.get(row_index)
                        {
                            shell.publish(M::from(icon_kind.into_event(row.study_id)));
                            state.drag_mode = DragMode::None;
                            state.last_cursor = None;
                            state.clear_all_caches();
                            shell.capture_event();
                            return;
                        }

                        if let (Some(legend), Some(IndicatorLegendHit::Icon(row_index, icon_kind))) =
                            (&indicator_legend, indicator_legend_hit)
                            && let Some(row) = legend.rows.get(row_index)
                        {
                            shell.publish(M::from(icon_kind.into_event(row.value_id)));
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

                        if footprint_legend_hit.is_some() {
                            state.drag_mode = DragMode::None;
                            state.last_cursor = None;
                            return;
                        }

                        if study_legend_hit.is_some() {
                            state.drag_mode = DragMode::None;
                            state.last_cursor = None;
                            return;
                        }

                        if indicator_legend_hit.is_some() {
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

                        if (self.drawings.measure.is_some()
                            || state.modifiers.intersects(keyboard::Modifiers::SHIFT)
                            || self.drawings.active_tool == DrawingTool::Measure)
                            && let LayoutHitZone::PanelPlot(panel_index) = zone
                        {
                            let measure_panel = self.drawings.measure.map(|m| m.state().panel_id);
                            let panel_id = measure_panel.or_else(|| self.panel_id(panel_index));

                            if let Some(panel_id) = panel_id
                                && let Some(scene) = self.compute_scene(layout, cursor)
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
                            && let Some(scene) = self.compute_scene(layout, cursor)
                        {
                            if Self::hit_corner_control(&corner_controls, cursor_pos).is_some() {
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
                            state.clear_overlay_caches();
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
                            state.drag_mode = DragMode::AxisScale {
                                panel_index,
                                anchor_y: cursor_pos.y,
                            };
                            state.last_cursor = Some(cursor_pos);
                            shell.capture_event();
                        } else if matches!(
                            zone,
                            LayoutHitZone::PanelXAxis(_) | LayoutHitZone::BottomXAxis
                        ) {
                            if let Some(scene) = self.compute_scene(layout, cursor) {
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
                                self.compute_scene(layout, cursor).and_then(|scene| {
                                    self.drawing_draft_anchor_from_scene_point(
                                        &scene, panel_id, cursor_pos, magnetic,
                                    )
                                })
                            } else if let LayoutHitZone::PanelPlot(panel_index) = zone {
                                let panel_id = self.panel_id(panel_index);

                                panel_id.and_then(|panel_id| {
                                    self.compute_scene(layout, cursor).and_then(|scene| {
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
                            state.drag_mode = DragMode::Pan {
                                panel_index,
                                anchor_x: cursor.position().map_or(cursor_pos.x, |pos| pos.x),
                                initial_offset: self.horizontal_world.offset(),
                            };
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

                        if let DragMode::Marquee {
                            panel_index,
                            start,
                            current,
                        } = state.drag_mode
                        {
                            if let Some(scene) = self.compute_scene(layout, cursor)
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
                        state.last_cursor = None;
                    }
                    mouse::Event::CursorMoved { .. } => {
                        state.clear_overlay_caches();

                        if let Some(DrawingMeasure::Pending(state)) = self.drawings.measure
                            && let Some(scene) = self.compute_scene(layout, cursor)
                            && let Some(anchor) = self.drawing_anchor_from_scene_point(
                                &scene,
                                state.panel_id,
                                cursor_pos,
                            )
                        {
                            shell.publish(M::from(KlineWidgetEvent::Drawing(
                                KlineWidgetDrawingEvent::MeasureMove(anchor),
                            )));
                        }

                        if let DragMode::XAxisScale {
                            anchor_x,
                            initial_world,
                            initial_right_unit,
                        } = state.drag_mode
                        {
                            let total_delta =
                                (anchor_x - cursor_pos.x) * HORIZONTAL_DRAG_SCALE_DELTA_PER_PX;
                            if let Some(scene) = self.compute_scene(layout, cursor) {
                                let world = horizontal_world_for_drag(
                                    &scene,
                                    initial_world,
                                    initial_right_unit,
                                    anchor_x,
                                    total_delta,
                                );

                                if world != self.horizontal_world {
                                    shell.publish(M::from(
                                        KlineWidgetEvent::HorizontalWorldChanged {
                                            cell_width: world.cell_width(),
                                            scaling: world.scaling(),
                                            offset: world.offset(),
                                        },
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
                            state.clear_overlay_caches();
                            shell.capture_event();
                        } else if let Some(draft_panel_id) = self.drawings.draft_panel_id()
                            && Self::hit_panel_control(&layout_tree, &panel_controls, cursor_pos)
                                .is_none()
                            && Self::hit_corner_control(&corner_controls, cursor_pos).is_none()
                            && ticker_legend_hit.is_none()
                            && let Some(scene) = self.compute_scene(layout, cursor)
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
                            state.clear_overlay_caches();
                            shell.capture_event();
                        } else if let DragMode::Pan {
                            panel_index,
                            anchor_x,
                            initial_offset,
                        } = state.drag_mode
                        {
                            if Self::hit_corner_control(&corner_controls, cursor_pos).is_some() {
                                state.last_cursor = Some(cursor_pos);
                                return;
                            }

                            let prev_y = state.last_cursor.map_or(cursor_pos.y, |prev| prev.y);
                            let dy_px = cursor_pos.y - prev_y;
                            let spacing = self.horizontal_world.effective_spacing();
                            let cursor_x = cursor.position().map_or(cursor_pos.x, |pos| pos.x);
                            let offset = initial_offset + (anchor_x - cursor_x) / spacing;

                            if (offset - self.horizontal_world.offset()).abs() > f32::EPSILON {
                                shell.publish(M::from(KlineWidgetEvent::HorizontalOffsetChanged(
                                    offset,
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
            Event::Window(window::Event::RedrawRequested(now)) => {
                let state = tree.state.downcast_mut::<State>();
                if state.cache_refresh_is_due(self.version, *now, MIN_FULL_CACHE_REFRESH_INTERVAL) {
                    state.clear_all_caches();
                    state.mark_cache_refreshed(self.version, *now);
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
        use advanced::Renderer as _;

        let state = tree.state.downcast_ref::<State>();
        let Some(scene) = self.compute_scene(layout, cursor) else {
            return;
        };

        let bounds = layout.bounds();
        let palette = theme.extended_palette();
        let generated_accents = generated_theme_accents(palette);

        renderer.with_translation(Vector::new(bounds.x, bounds.y), |r| {
            let plot_rect = scene.plot_rect();
            let plot_origin_in_window = Point::new(bounds.x + plot_rect.x, bounds.y + plot_rect.y);
            let overlay_origin_in_window = Point::new(bounds.x, bounds.y);

            let plot_geom = state.plot_cache.draw(r, plot_rect.size(), |frame| {
                self.fill_main_geometry(
                    frame,
                    &scene,
                    palette,
                    &generated_accents,
                    plot_origin_in_window,
                );
            });

            let splitter_color = palette.background.strong.color.scale_alpha(0.25);

            for splitter in &scene.layout.splitters {
                r.fill_quad(
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

            let drawing_magnet = self.drawing_magnet_enabled(state.modifiers);
            let overlay_geom = state.overlay_cache.draw(r, bounds.size(), |frame| {
                self.fill_overlay(
                    frame,
                    &scene,
                    palette,
                    &generated_accents,
                    overlay_origin_in_window,
                    drawing_magnet,
                );
            });

            let interaction_text_geom =
                state
                    .interaction_text_cache
                    .draw(r, bounds.size(), |frame| {
                        self.fill_overlay_interaction_text(frame, &scene, palette, drawing_magnet);
                    });

            let marquee_geom = state.marquee_cache.draw(r, bounds.size(), |frame| {
                if let DragMode::Marquee {
                    panel_index,
                    start,
                    current,
                } = &state.drag_mode
                    && let Some(rect) =
                        self.clipped_marquee_rect(&scene, *panel_index, *start, *current)
                {
                    self.fill_marquee_rectangle(frame, rect, palette);
                }
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

            r.with_layer(layer_bounds, |r| {
                use iced::advanced::graphics::geometry::Renderer as _;
                r.draw_geometry(marquee_geom);
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

        if matches!(state.drag_mode, DragMode::XAxisScale { .. }) {
            return advanced::mouse::Interaction::ResizingHorizontally;
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

        if matches!(ticker_legend_hit, Some(TickerLegendHit::Icon(_, _))) {
            return advanced::mouse::Interaction::Pointer;
        }

        let footprint_legend = self
            .compute_scene(layout, cursor)
            .and_then(|scene| scene.chrome.footprint_legend);
        let footprint_legend_hit = footprint_legend
            .as_ref()
            .and_then(|legend| Self::hit_footprint_legend(&layout_tree, legend, cursor_local));
        if matches!(footprint_legend_hit, Some(FootprintLegendHit::Icon(_))) {
            return advanced::mouse::Interaction::Pointer;
        }

        let study_legend = self.build_study_legend_layout(
            &layout_tree,
            primary_panel,
            self.tooltip_rows_before_studies(
                ticker_legend.as_ref(),
                footprint_legend.as_ref(),
                show_legend_values,
            ),
            self.studies_legend_expanded,
        );
        let study_legend_hit = study_legend
            .as_ref()
            .and_then(|legend| Self::hit_study_legend(&layout_tree, legend, cursor_local));
        if matches!(
            study_legend_hit,
            Some(StudyLegendHit::CategoryIcon(_)) | Some(StudyLegendHit::StudyIcon(_, _))
        ) {
            return advanced::mouse::Interaction::Pointer;
        }

        let indicator_legend = self.build_indicator_legend_layout(
            &layout_tree,
            primary_panel,
            self.tooltip_rows_before_indicators(
                ticker_legend.as_ref(),
                footprint_legend.as_ref(),
                study_legend.as_ref(),
                show_legend_values,
            ),
        );
        let indicator_legend_hit = indicator_legend
            .as_ref()
            .and_then(|legend| Self::hit_indicator_legend(&layout_tree, legend, cursor_local));
        if matches!(indicator_legend_hit, Some(IndicatorLegendHit::Icon(_, _))) {
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
            if self.drawings.selected_drawings.contains(&hit_id) {
                return advanced::mouse::Interaction::Grab;
            }

            return advanced::mouse::Interaction::Pointer;
        }

        match zone {
            LayoutHitZone::Splitter(_) => advanced::mouse::Interaction::ResizingVertically,
            LayoutHitZone::PanelPlot(_) => advanced::mouse::Interaction::Crosshair,
            LayoutHitZone::YAxis(_) => advanced::mouse::Interaction::ResizingVertically,
            LayoutHitZone::PanelXAxis(_) | LayoutHitZone::BottomXAxis => {
                advanced::mouse::Interaction::ResizingHorizontally
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
