mod command;
#[path = "kline_v2/data.rs"]
mod data_state;
mod drawing;
mod fetch;
mod indicator;
mod layout;
mod series_adapter;
mod settings;
mod viewport;

pub use command::{
    CandlestickCommand, ChartCommand, CommandResult, FootprintCommand, IndicatorCommand,
    PanelCommand, SeriesCommand, StudyCommand, TickerCommand,
};
pub use data::chart::kline_v2::KlineIndicator;
pub use data_state::DataUpdate;
pub use fetch::FetchUpdate;
pub use series_adapter::KlineSeries;

use data_state::KlineData;
use drawing::{DrawingMessage, DrawingTools, DrawingUpdate};
use fetch::FetchCoordinator;
use layout::IndicatorLayout;
use viewport::PanelViewportStore;

use crate::connector::fetcher::FetchSpec;
use crate::style;
use crate::widget::chart::kline::PanelInteraction;
use crate::widget::chart::kline::composition::{
    DEFAULT_MIN_PANEL_RATIO, MarkKind, PanelId, PanelRole, PanelScaleMode,
};
use crate::widget::chart::kline::{
    DEFAULT_BAR_SPACING_PX, KlineWidget, KlineWidgetEvent, PanelYViewport, UnavailableIndicator,
};
use crate::widget::series_editor::{self, SeriesStyleEntry, TickerSeriesEditor};
use crate::widget::study_configurator as study;

use data::chart::Basis;
use data::chart::kline::ClusterKind;
use data::chart::kline_v2::{
    self as persisted, CandlestickConfig, FetchBatch, FetchPlanInput, FetchStage,
    FootprintPocConfig, HorizontalWorld, IndicatorConfig, IndicatorInstanceId,
    IndicatorInstanceStore, IndicatorStyleConfig, KlineStudy, KlineStudyId, SeriesColor,
    desired_fetch_batches as plan_fetch_batches,
    desired_trade_fetch_range as plan_trade_fetch_range, visible_window,
};
use exchange::adapter::StreamKind;
use exchange::{TickMultiplier, TickerInfo, UnixMs};

use std::time::Instant;

const DEFAULT_HORIZONTAL_OFFSET_UNITS: f32 = 8.0;
pub const DEFAULT_FOOTPRINT_TICK_MULTIPLIER: TickMultiplier = TickMultiplier(10);
const SETTINGS_SIDEBAR_HEADER_HEIGHT: f32 = 32.0;
const FLOATING_TOOLBAR_MARGIN: f32 = 8.0;
const FLOATING_TOOLBAR_HANDLE_WIDTH: f32 = 28.0;
const FLOATING_TOOLBAR_HEIGHT: f32 = 48.0;

pub enum Action {
    RemoveSeries(TickerInfo),
    RequestFetch(Vec<FetchSpec>),
}

#[derive(Debug, Clone)]
pub enum Message {
    Chart(KlineWidgetEvent),
    Sidebar(DrawingMessage),
    Settings(settings::Message),
    SeriesEditor(series_editor::Message),
    CloseSettingsSidebar,
    FloatingToolbarHandleMoved(iced::Point),
    FloatingToolbarDragStarted,
    FloatingToolbarDragMoved(iced::Point),
    FloatingToolbarDragFinished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SidebarMode {
    Drawing,
    Settings,
}

const DRAWING_SIDEBAR_PADDING: f32 = 4.0;

#[derive(Debug, Clone, Copy)]
struct FloatingToolbarState {
    position: iced::Point,
    handle_cursor: iced::Point,
    is_dragging: bool,
}

impl Default for FloatingToolbarState {
    fn default() -> Self {
        Self {
            position: iced::Point::new(FLOATING_TOOLBAR_MARGIN, FLOATING_TOOLBAR_MARGIN),
            handle_cursor: iced::Point::ORIGIN,
            is_dragging: false,
        }
    }
}

impl FloatingToolbarState {
    fn set_handle_cursor(&mut self, cursor: iced::Point) {
        self.handle_cursor = cursor;
    }

    fn start_drag(&mut self) {
        self.is_dragging = true;
    }

    fn drag_to(&mut self, cursor: iced::Point) {
        if !self.is_dragging {
            return;
        }

        self.position = iced::Point::new(
            (cursor.x - self.handle_cursor.x).max(FLOATING_TOOLBAR_MARGIN),
            (cursor.y - self.handle_cursor.y).max(FLOATING_TOOLBAR_MARGIN),
        );
    }

    fn finish_drag(&mut self) {
        self.is_dragging = false;
    }

    fn position_within(&self, overlay_size: iced::Size) -> iced::Point {
        let max_y = (overlay_size.height - FLOATING_TOOLBAR_HEIGHT - FLOATING_TOOLBAR_MARGIN)
            .max(FLOATING_TOOLBAR_MARGIN);

        iced::Point::new(
            self.position.x.max(FLOATING_TOOLBAR_MARGIN),
            self.position.y.clamp(FLOATING_TOOLBAR_MARGIN, max_y),
        )
    }
}

pub struct KlineChartV2 {
    basis: Basis,
    horizontal_world: HorizontalWorld,
    primary_autoscale: bool,
    candlesticks_visible: bool,
    footprints_visible: bool,
    trade_data_enabled: bool,
    candlestick_config: CandlestickConfig,
    footprint_cluster_kind: ClusterKind,
    footprint_poc_config: FootprintPocConfig,
    panel_y_viewports: PanelViewportStore,
    plot_width: Option<f32>,
    layout: IndicatorLayout,
    last_tick: Instant,
    cache_rev: u64,
    data: KlineData,
    indicators: IndicatorInstanceStore,
    fetch: FetchCoordinator,
    drawing: DrawingTools,
    studies: Vec<KlineStudy>,
    study_configs: Vec<KlineStudy>,
    study_configurator: study::Configurator<KlineStudy>,
    series_editor: TickerSeriesEditor,
    settings: settings::State,
    studies_legend_expanded: bool,
    sidebar_mode: SidebarMode,
    floating_toolbar: FloatingToolbarState,
}

impl KlineChartV2 {
    pub fn new(basis: Basis, ticker_info: TickerInfo) -> Self {
        Self::new_with_tickers(basis, &[ticker_info])
    }

    pub fn new_with_tickers(basis: Basis, tickers: &[TickerInfo]) -> Self {
        let base_ticker = tickers
            .first()
            .copied()
            .expect("Kline v2 requires a base ticker");
        let footprint_tick_multiplier = DEFAULT_FOOTPRINT_TICK_MULTIPLIER;
        let mut indicators = IndicatorInstanceStore::default();
        let default_volume = indicators.create(KlineIndicator::Volume);

        let mut chart = Self {
            basis,
            horizontal_world: HorizontalWorld::new(
                DEFAULT_BAR_SPACING_PX,
                1.0,
                DEFAULT_HORIZONTAL_OFFSET_UNITS,
            ),
            primary_autoscale: true,
            candlesticks_visible: true,
            footprints_visible: false,
            trade_data_enabled: false,
            candlestick_config: CandlestickConfig::default(),
            footprint_cluster_kind: ClusterKind::default(),
            footprint_poc_config: FootprintPocConfig::default(),
            panel_y_viewports: PanelViewportStore::default(),
            plot_width: None,
            layout: IndicatorLayout::new(default_volume),
            last_tick: Instant::now(),
            cache_rev: 0,
            data: KlineData::new(base_ticker, footprint_tick_multiplier),
            indicators,
            fetch: FetchCoordinator::new(&[]),
            drawing: DrawingTools::default(),
            studies: Vec::new(),
            study_configs: Vec::new(),
            study_configurator: study::Configurator::new(),
            series_editor: TickerSeriesEditor::default(),
            settings: settings::State::default(),
            studies_legend_expanded: true,
            sidebar_mode: SidebarMode::Drawing,
            floating_toolbar: FloatingToolbarState::default(),
        };

        let active_indicators = chart.layout.selected_indicators();
        let _ = chart.data.add_ticker(
            base_ticker,
            chart.basis,
            &chart.indicators,
            &active_indicators,
        );
        for ticker in tickers.iter().copied() {
            if ticker != base_ticker {
                let _ = chart.data.add_ticker(
                    ticker,
                    chart.basis,
                    &chart.indicators,
                    &active_indicators,
                );
            }
        }

        chart.fetch.rebuild_handlers(chart.data.selected_tickers());
        chart
            .data
            .configure_series_for_basis(chart.basis, &chart.indicators, &active_indicators);
        chart
            .layout
            .sync_comparison_sources(chart.data.base_ticker(), chart.data.selected_tickers());
        chart
    }

    pub fn new_with_tickers_and_config(
        basis: Basis,
        tickers: &[TickerInfo],
        config: &persisted::Config,
    ) -> Self {
        let mut chart = Self::new_with_tickers(basis, tickers);
        chart.apply_persisted_config(config);
        chart.reset_autoscale();
        chart
    }

    pub fn update(&mut self, message: Message) -> Option<Action> {
        match message {
            Message::CloseSettingsSidebar => {
                self.close_settings_sidebar();
            }
            Message::FloatingToolbarHandleMoved(cursor) => {
                self.floating_toolbar.set_handle_cursor(cursor);
            }
            Message::FloatingToolbarDragStarted => {
                self.floating_toolbar.start_drag();
            }
            Message::FloatingToolbarDragMoved(cursor) => {
                self.floating_toolbar.drag_to(cursor);
            }
            Message::FloatingToolbarDragFinished => {
                self.floating_toolbar.finish_drag();
            }
            Message::Settings(message) => {
                let mut settings = std::mem::take(&mut self.settings);
                let action = settings.update(self, message);
                self.settings = settings;
                return action;
            }
            Message::SeriesEditor(message) => {
                if let Some(change) = self.series_editor.update(message) {
                    let command = match change {
                        series_editor::Change::ColorChanged(ticker_info, color) => {
                            ChartCommand::Series(SeriesCommand::SetColor { ticker_info, color })
                        }
                        series_editor::Change::LineWidthChanged(ticker_info, width) => {
                            ChartCommand::Series(SeriesCommand::SetLineWidth { ticker_info, width })
                        }
                        series_editor::Change::StrokeStyleChanged(ticker_info, stroke_style) => {
                            ChartCommand::Series(SeriesCommand::SetStrokeStyle {
                                ticker_info,
                                stroke_style,
                            })
                        }
                    };
                    let _ = self.apply_command(command);
                }
            }
            Message::Sidebar(message) => {
                let update = self.drawing.update(message);
                self.apply_drawing_update(update);
            }
            Message::Chart(event) => {
                match event {
                    KlineWidgetEvent::Drawing(event) => {
                        let update = self.drawing.handle_widget_drawing_event(event);
                        self.apply_drawing_update(update);
                    }
                    KlineWidgetEvent::PanelInteraction(interaction) => {
                        match interaction {
                            PanelInteraction::YViewportChanged { panel_id, viewport } => {
                                if let Some(primary_id) =
                                    self.layout.composition().primary_panel_id()
                                    && primary_id == panel_id
                                {
                                    if matches!(
                                        self.layout
                                            .composition()
                                            .panel_effective_scale_mode(primary_id),
                                        Some(PanelScaleMode::PercentFromBase)
                                    ) {
                                        return None;
                                    }

                                    if self.primary_autoscale {
                                        self.primary_autoscale = false;
                                    }
                                }

                                if self.panel_y_viewports.set(panel_id, viewport) {
                                    self.bump_rev();
                                }
                            }
                            PanelInteraction::YViewportReset { panel_id } => {
                                if let Some(primary_id) =
                                    self.layout.composition().primary_panel_id()
                                    && primary_id == panel_id
                                {
                                    if matches!(
                                        self.layout
                                            .composition()
                                            .panel_effective_scale_mode(primary_id),
                                        Some(PanelScaleMode::PercentFromBase)
                                    ) {
                                        return None;
                                    }

                                    if !self.primary_autoscale {
                                        self.primary_autoscale = true;
                                    }
                                }

                                if self.panel_y_viewports.reset(panel_id) {
                                    self.bump_rev();
                                }
                            }
                            PanelInteraction::SplitChanged { index, split } => {
                                if self.layout.composition_mut().set_split(
                                    index,
                                    split,
                                    DEFAULT_MIN_PANEL_RATIO,
                                ) {
                                    self.bump_rev();
                                }
                            }
                            PanelInteraction::MoveUp { index } => {
                                if index > 0
                                    && self.layout.composition_mut().move_panel(index, index - 1)
                                {
                                    self.bump_rev();
                                }
                            }
                            PanelInteraction::MoveDown { index } => {
                                let target = index.saturating_add(1);
                                if target < self.layout.composition().panels.len()
                                    && self.layout.composition_mut().move_panel(index, target)
                                {
                                    self.bump_rev();
                                }
                            }
                            PanelInteraction::Settings { .. } => {
                                // TODO: Hook for upcoming panel settings modal/workflow.
                            }
                            PanelInteraction::Close { index } => {
                                if let Some((panel_id, panel_role)) = self
                                    .layout
                                    .composition()
                                    .panels
                                    .get(index)
                                    .map(|panel| (panel.id, panel.role))
                                    && !matches!(panel_role, PanelRole::Primary)
                                {
                                    // Remove panel-local drawings while panel coordinates are still valid.
                                    let _ = self.drawing.prune_panel_drawings(panel_id);

                                    if self.layout.close_panel(panel_id) {
                                        let _ = self.panel_y_viewports.reset(panel_id);
                                        self.bump_rev();
                                    }
                                }
                            }
                        }
                    }
                    KlineWidgetEvent::HorizontalScaleChanged(scale) => {
                        self.horizontal_world = HorizontalWorld::new(
                            scale.as_pixels_per_bar(),
                            1.0,
                            self.horizontal_world.offset(),
                        );
                        self.bump_rev();
                    }
                    KlineWidgetEvent::HorizontalWorldChanged {
                        cell_width,
                        scaling,
                        offset,
                    } => {
                        self.horizontal_world = HorizontalWorld::new(cell_width, scaling, offset);
                        self.bump_rev();
                    }
                    KlineWidgetEvent::HorizontalOffsetChanged(offset) => {
                        self.horizontal_world = self.horizontal_world.with_offset(offset);
                        self.bump_rev();
                    }
                    KlineWidgetEvent::PlotWidthChanged(plot_width) => {
                        if self.plot_width == Some(plot_width) {
                            return None;
                        }

                        self.plot_width = Some(plot_width);
                        self.bump_rev();
                        return self.request_fetch_action().map(Action::RequestFetch);
                    }
                    KlineWidgetEvent::ZoomChanged {
                        cell_width,
                        scaling,
                        offset,
                        y_viewport,
                    } => {
                        self.horizontal_world = HorizontalWorld::new(cell_width, scaling, offset);

                        if let Some((panel_id, viewport)) = y_viewport
                            && !matches!(
                                self.layout
                                    .composition()
                                    .panel_effective_scale_mode(panel_id),
                                Some(PanelScaleMode::PercentFromBase)
                            )
                        {
                            if self
                                .layout
                                .composition()
                                .primary_panel_id()
                                .is_some_and(|primary_id| primary_id == panel_id)
                                && self.primary_autoscale
                            {
                                self.primary_autoscale = false;
                            }
                            self.panel_y_viewports.set(panel_id, viewport);
                        }

                        self.bump_rev();
                    }
                    KlineWidgetEvent::PrimaryAutoscaleDisabled => {
                        if self.primary_autoscale {
                            self.primary_autoscale = false;
                            self.bump_rev();
                        }
                    }
                    KlineWidgetEvent::PrimaryAutoscaleToggled => {
                        let primary_panel_id = self.layout.composition().primary_panel_id()?;

                        let effective_scale = self
                            .layout
                            .composition()
                            .panel_effective_scale_mode(primary_panel_id)
                            .unwrap_or(PanelScaleMode::Absolute);

                        if !matches!(effective_scale, PanelScaleMode::PercentFromBase) {
                            self.primary_autoscale = !self.primary_autoscale;

                            if self.primary_autoscale {
                                let _ = self.panel_y_viewports.reset(primary_panel_id);
                            }

                            self.bump_rev();
                        }
                    }
                    KlineWidgetEvent::PrimaryScaleModeCycleRequested => {
                        let primary_panel_id = self.layout.composition().primary_panel_id()?;
                        let panel = self.layout.composition().panel(primary_panel_id)?;

                        if panel.uses_multi_source() {
                            // Multi-source primary scale is forced to percent.
                            return None;
                        }

                        let next_scale = match panel.preferred_scale {
                            PanelScaleMode::Absolute => PanelScaleMode::Logarithmic,
                            PanelScaleMode::Logarithmic => PanelScaleMode::PercentFromBase,
                            PanelScaleMode::PercentFromBase => PanelScaleMode::Absolute,
                            PanelScaleMode::FitVisible | PanelScaleMode::FitVisibleIncludeZero => {
                                PanelScaleMode::Absolute
                            }
                        };

                        if self
                            .layout
                            .composition_mut()
                            .set_panel_preferred_scale(primary_panel_id, next_scale)
                        {
                            let now_effective = self
                                .layout
                                .composition()
                                .panel_effective_scale_mode(primary_panel_id)
                                .unwrap_or(next_scale);

                            if matches!(now_effective, PanelScaleMode::PercentFromBase)
                                || self.primary_autoscale
                            {
                                let _ = self.panel_y_viewports.reset(primary_panel_id);
                            }

                            self.bump_rev();
                        }
                    }
                    KlineWidgetEvent::TickerSettings(ticker) => {
                        if ticker == self.data.base_ticker() {
                            self.series_editor.close();
                            self.settings.open_candlestick();
                            self.open_settings_sidebar();
                            return None;
                        }
                        if let Some(series) = self
                            .data
                            .series
                            .iter()
                            .find(|series| series.ticker_info == ticker)
                        {
                            self.series_editor.open_for(
                                ticker,
                                series.color(),
                                series.line_width(),
                                series.stroke_style(),
                            );
                        }
                        self.settings.open_series();
                        self.open_settings_sidebar();
                    }
                    KlineWidgetEvent::FootprintSettings => {
                        self.settings.open_footprint();
                        self.open_settings_sidebar();
                    }
                    KlineWidgetEvent::CandlesticksVisibilityToggled => {
                        let _ = self.apply_command(ChartCommand::Candlestick(
                            CandlestickCommand::SetVisible(!self.candlesticks_visible),
                        ));
                    }
                    KlineWidgetEvent::FootprintsVisibilityToggled => {
                        let _ = self.apply_command(ChartCommand::Footprint(
                            FootprintCommand::SetVisible(!self.footprints_visible),
                        ));
                    }
                    KlineWidgetEvent::IndicatorSettings(id) => {
                        if let Some(indicator) = self.indicator_kind(id) {
                            self.settings.open_indicator(id, indicator);
                            self.open_settings_sidebar();
                        }
                    }
                    KlineWidgetEvent::IndicatorVisibilityToggled(id) => {
                        let _ = self.apply_command(ChartCommand::Indicator(
                            IndicatorCommand::ToggleVisibility(id),
                        ));
                    }
                    KlineWidgetEvent::StudySettings(id) => {
                        if self.study(id).is_some() {
                            self.settings.open_study(id);
                            self.open_settings_sidebar();
                        }
                    }
                    KlineWidgetEvent::StudyVisibilityToggled(id) => {
                        let _ = self
                            .apply_command(ChartCommand::Study(StudyCommand::ToggleVisibility(id)));
                    }
                    KlineWidgetEvent::StudyRemove(id) => {
                        let _ = self.apply_command(ChartCommand::Study(StudyCommand::Remove(id)));
                    }
                    KlineWidgetEvent::StudiesVisibilityToggled => {
                        let _ = self
                            .apply_command(ChartCommand::Study(StudyCommand::ToggleAllVisibility));
                    }
                    KlineWidgetEvent::StudiesExpansionToggled => {
                        let _ =
                            self.apply_command(ChartCommand::Study(StudyCommand::ToggleExpansion));
                    }
                    KlineWidgetEvent::StudiesRemove => {
                        let _ = self.apply_command(ChartCommand::Study(StudyCommand::RemoveAll));
                    }
                    KlineWidgetEvent::IndicatorRemove(id) => {
                        let _ = self
                            .apply_command(ChartCommand::Indicator(IndicatorCommand::Remove(id)));
                    }
                    KlineWidgetEvent::TickerRemove(ticker) if ticker != self.data.base_ticker() => {
                        return Some(Action::RemoveSeries(ticker));
                    }
                    KlineWidgetEvent::TickerRemove(_ticker) => {}
                    KlineWidgetEvent::XAxisDoubleClick => {
                        self.horizontal_world = HorizontalWorld::new(
                            DEFAULT_BAR_SPACING_PX,
                            1.0,
                            DEFAULT_HORIZONTAL_OFFSET_UNITS,
                        );
                        self.bump_rev();
                    }
                    KlineWidgetEvent::DeletePressed => {
                        let update = self.drawing.update(DrawingMessage::DeleteSelectedDrawing);
                        self.apply_drawing_update(update);
                    }
                }
            }
        }
        None
    }

    pub fn view(
        &self,
        timezone: data::UserTimezone,
        pixel_ratio: f32,
    ) -> iced::Element<'_, Message> {
        let chart: iced::Element<'_, Message> = if self
            .data
            .series
            .iter()
            .all(|series| series.bars.is_empty())
        {
            iced::widget::center(iced::widget::text("Waiting for data...").size(16)).into()
        } else {
            let unavailable_indicator_panels = self.unavailable_indicator_panels();
            let mut chart = KlineWidget::new(
                &self.data.series,
                self.basis,
                self.layout.composition(),
                &self.indicators,
            )
            .with_footprint_enabled(self.trade_data_enabled());

            if self.candlesticks_visible {
                chart = chart.with_candlestick_display(self.candlestick_config);
            }

            if self.footprints_visible {
                chart = chart.with_footprint_display(self.footprint_cluster_kind);
            }

            let chart: iced::Element<_> = chart
                .with_studies(&self.studies)
                .with_footprint_poc_config(self.footprint_poc_config)
                .with_studies_legend_expanded(self.studies_legend_expanded)
                .with_coordinate_state(self.horizontal_world, pixel_ratio, self.primary_autoscale)
                .with_known_plot_width(self.plot_width)
                .with_autoscale_plot_width_offset(self.autoscale_plot_width_offset())
                .with_panels(self.panel_y_viewports.as_slice())
                .with_indicator_availability(unavailable_indicator_panels)
                .with_drawings(self.drawing.snapshot())
                .with_timezone(timezone)
                .with_version(self.cache_rev)
                .into();

            iced::widget::container(chart.map(Message::Chart))
                .width(iced::Length::Fill)
                .height(iced::Length::Fill)
                .padding(1)
                .into()
        };

        match self.sidebar_mode {
            SidebarMode::Drawing => {
                let sidebar: iced::Element<'_, Message> = self
                    .drawing
                    .sidebar_view(self.layout.composition(), Message::Sidebar)
                    .padding(DRAWING_SIDEBAR_PADDING)
                    .into();

                let mut chart_stack = iced::widget::Stack::new()
                    .width(iced::Length::Fill)
                    .height(iced::Length::Fill)
                    .push(chart)
                    .push(self.floating_toolbar_overlay());

                if self.floating_toolbar.is_dragging {
                    chart_stack = chart_stack.push(self.floating_toolbar_drag_capture());
                }

                iced::widget::row![sidebar, chart_stack.clip(true)].into()
            }
            SidebarMode::Settings => {
                let sidebar: iced::Element<'_, Message> = self.settings.sidebar_view(self);
                iced::widget::row![sidebar, chart].into()
            }
        }
    }

    pub fn settings_modal_view(&self) -> iced::Element<'static, Message> {
        settings::kline_v2_modal_view(self)
    }

    pub fn open_settings_modal(&mut self) {
        self.settings.open_main();
        self.close_settings_sidebar();
    }

    pub fn close_settings_sidebar(&mut self) {
        self.sidebar_mode = SidebarMode::Drawing;
    }

    pub fn settings_sidebar_visible(&self) -> bool {
        matches!(self.sidebar_mode, SidebarMode::Settings)
    }

    fn open_settings_sidebar(&mut self) {
        self.sidebar_mode = SidebarMode::Settings;
    }

    fn floating_toolbar_overlay<'a>(&'a self) -> iced::Element<'a, Message> {
        iced::widget::responsive(move |overlay_size| {
            self.drawing
                .selected_toolbar_view(Message::Sidebar)
                .map(|controls| self.floating_toolbar_content(controls, overlay_size))
                .unwrap_or_else(|| self.empty_floating_toolbar_content())
        })
        .into()
    }

    fn floating_toolbar_content<'a>(
        &'a self,
        controls: iced::Element<'a, Message>,
        overlay_size: iced::Size,
    ) -> iced::Element<'a, Message> {
        let drag_handle = iced::widget::mouse_area(
            iced::widget::container(style::icon_text(style::Icon::DragHandle, 12))
                .width(FLOATING_TOOLBAR_HANDLE_WIDTH)
                .height(iced::Length::Fill)
                .align_x(iced::alignment::Horizontal::Center)
                .align_y(iced::alignment::Vertical::Center),
        )
        .on_move(Message::FloatingToolbarHandleMoved)
        .on_press(Message::FloatingToolbarDragStarted)
        .interaction(iced::mouse::Interaction::Grab);

        let scrollable_controls = iced::widget::scrollable::Scrollable::with_direction(
            controls,
            iced::widget::scrollable::Direction::Horizontal(
                iced::widget::scrollable::Scrollbar::new()
                    .width(6)
                    .scroller_width(6),
            ),
        )
        .anchor_left()
        .width(iced::Length::Shrink)
        .height(iced::Length::Fill)
        .style(style::scroll_bar);
        let toolbar = iced::widget::mouse_area(
            iced::widget::container(iced::widget::row![drag_handle, scrollable_controls])
                .style(style::chart_toolbar_container)
                .width(iced::Length::Shrink)
                .height(FLOATING_TOOLBAR_HEIGHT),
        )
        .interaction(iced::mouse::Interaction::Idle);
        let position = self.floating_toolbar.position_within(overlay_size);

        iced::widget::container(
            iced::widget::column![
                iced::widget::space::vertical().height(position.y),
                iced::widget::row![iced::widget::space::horizontal().width(position.x), toolbar,],
            ]
            .width(iced::Length::Fill)
            .height(iced::Length::Fill),
        )
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
        .into()
    }

    fn empty_floating_toolbar_content<'a>(&self) -> iced::Element<'a, Message> {
        iced::widget::container(iced::widget::space::horizontal())
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .into()
    }

    fn floating_toolbar_drag_capture<'a>(&self) -> iced::Element<'a, Message> {
        iced::widget::mouse_area(
            iced::widget::space::horizontal()
                .width(iced::Length::Fill)
                .height(iced::Length::Fill),
        )
        .on_move(Message::FloatingToolbarDragMoved)
        .on_release(Message::FloatingToolbarDragFinished)
        .interaction(iced::mouse::Interaction::Grabbing)
        .into()
    }

    fn autoscale_plot_width_offset(&self) -> f32 {
        (self.sidebar_width() - self.compact_sidebar_width()).max(0.0)
    }

    fn sidebar_width(&self) -> f32 {
        match self.sidebar_mode {
            SidebarMode::Drawing => self.drawing.sidebar_width() + 2.0 * DRAWING_SIDEBAR_PADDING,
            SidebarMode::Settings => self.settings.sidebar_width(),
        }
    }

    fn compact_sidebar_width(&self) -> f32 {
        DrawingTools::compact_sidebar_width() + 2.0 * DRAWING_SIDEBAR_PADDING
    }

    pub fn invalidate(&mut self, now: Option<Instant>) -> Option<crate::chart::Action> {
        if let Some(ts) = now {
            self.last_tick = ts;
            self.bump_rev();
            return self.request_fetch_action().map(super::Action::RequestFetch);
        }

        self.bump_rev();
        None
    }

    fn bump_rev(&mut self) {
        self.prune_stale_panel_y_viewports();
        self.cache_rev = self.cache_rev.wrapping_add(1);
    }

    fn request_fetch_action(&mut self) -> Option<Vec<FetchSpec>> {
        let horizontal_offset = self.horizontal_world.offset();
        let visible_points = self.visible_points();
        let active_indicators = self.selected_indicators();
        let (required_history_start, required_history_bars) =
            self.indicator_history_requirement(&active_indicators, visible_points);
        let input = FetchPlanInput {
            basis: self.basis,
            horizontal_offset,
            base_ticker: self.data.base_ticker(),
            series: &self.data.series,
            active_indicators: &active_indicators,
            indicator_instances: &self.indicators,
            open_interest_enabled: self.oi_enabled_for_base(),
            trade_data_required: self.trade_data_required(),
            trade_fetch_enabled: crate::connector::fetcher::is_trade_fetch_enabled(),
            visible_points,
            required_history_start,
            required_history_bars,
            now: UnixMs::now(),
        };
        let mut batches = plan_fetch_batches(input);
        if let Some(range) = plan_trade_fetch_range(input) {
            batches.push(FetchBatch {
                stage: FetchStage::Trades,
                range,
                tickers: vec![self.data.base_ticker()],
            });
        }
        let stage = batches.iter().map(|batch| batch.stage).min()?;
        batches.retain(|batch| batch.stage == stage);
        let requests = self.fetch.collect_requests(
            batches
                .into_iter()
                .map(|batch| (batch.range, batch.tickers)),
            self.basis,
        );

        (!requests.is_empty()).then_some(requests)
    }

    fn visible_points(&self) -> u64 {
        let Some(plot_width) = self.plot_width else {
            return 2;
        };

        let spacing = self.horizontal_world.effective_spacing();
        let autoscale_plot_width = plot_width + self.autoscale_plot_width_offset();
        let visible_units = (autoscale_plot_width / spacing).max(1.0).ceil() as u64;
        visible_units.saturating_add(1).max(2)
    }

    fn indicator_history_requirement(
        &self,
        active_indicators: &[IndicatorInstanceId],
        visible_points: u64,
    ) -> (Option<UnixMs>, u64) {
        let reference_time = match self.basis {
            Basis::Time(timeframe) => visible_window(
                &self.data.series,
                self.horizontal_world.offset(),
                timeframe,
                visible_points,
            )
            .map(|(start, _)| start)
            .unwrap_or_else(UnixMs::now),
            Basis::Tick(_) => UnixMs::now(),
        };
        let requirement =
            self.indicators
                .history_requirement(active_indicators, self.basis, reference_time);
        (requirement.history_start(), requirement.history_bars())
    }

    pub fn apply_data(&mut self, update: DataUpdate<'_>) {
        let changed = match update {
            DataUpdate::HistoricalKlines {
                req_id,
                ticker_info,
                klines,
            } => self.data.insert_historical_klines(
                req_id,
                ticker_info,
                klines,
                self.basis,
                &mut self.fetch,
                &self.indicators,
            ),
            DataUpdate::LiveKline { ticker_info, kline } => {
                self.data
                    .update_live_kline(ticker_info, kline, self.basis, &self.indicators)
            }
            DataUpdate::LiveTrades {
                ticker_info,
                trades,
            } => self
                .data
                .insert_live_trades(ticker_info, trades, self.basis, &self.indicators),
            DataUpdate::HistoricalTrades {
                trades,
                is_batches_done,
                req_id,
            } => self.data.insert_historical_trades(
                trades,
                is_batches_done,
                req_id,
                self.basis,
                &mut self.fetch,
                &self.indicators,
            ),
            DataUpdate::HistoricalOpenInterest {
                req_id,
                ticker_info,
                data,
            } => self.data.insert_historical_open_interest(
                req_id,
                ticker_info,
                data,
                self.basis,
                &mut self.fetch,
                &self.indicators,
            ),
        };

        if changed {
            self.bump_rev();
        }
    }

    pub fn apply_fetch_update(&mut self, update: FetchUpdate) {
        match update {
            FetchUpdate::Failed(req_id) => self.fetch.mark_failed(req_id),
            FetchUpdate::TradeFetchNoData(req_id) => self
                .fetch
                .mark_trade_fetch_no_data(self.data.base_ticker(), req_id),
            FetchUpdate::TradeFetchHandle(handle) => self.fetch.set_handle(handle),
        }
    }

    fn prune_stale_panel_y_viewports(&mut self) {
        let layout = &self.layout;
        self.panel_y_viewports.retain(|panel_id| {
            layout.composition().panel(panel_id).is_some() || layout.retains_panel(panel_id)
        });
    }

    fn configure_profile_caches(&mut self) {
        self.data.configure_profile_caches(
            &self.studies,
            self.basis,
            self.footprints_visible,
            self.footprint_poc_config,
        );
    }

    fn set_basis(&mut self, basis: Basis) -> Option<super::Action> {
        let preserve_raw_trades = matches!(
            (self.basis, basis),
            (Basis::Time(_), Basis::Time(_)) | (Basis::Tick(_), Basis::Tick(_))
        );
        self.basis = basis;
        let normalized = self.indicators.normalized_for_basis(basis);
        self.indicators = normalized;
        self.studies = self.normalize_studies_for_basis(self.studies.iter().copied());
        self.study_configs = self.normalize_studies_for_basis(self.study_configs.iter().copied());
        self.remember_study_configs(&self.studies.clone());

        let active_indicators = self.layout.selected_indicators();
        self.data.reset_for_basis(
            self.basis,
            preserve_raw_trades,
            &self.indicators,
            &active_indicators,
        );
        self.configure_profile_caches();

        self.fetch.reset();
        self.fetch.rebuild_handlers(self.data.selected_tickers());

        self.request_fetch_action().map(super::Action::RequestFetch)
    }

    fn set_primary_scale_mode(&mut self, scale: PanelScaleMode) -> bool {
        let Some(primary_panel_id) = self.layout.composition().primary_panel_id() else {
            return false;
        };

        if self
            .layout
            .composition_mut()
            .set_panel_preferred_scale(primary_panel_id, scale)
        {
            if matches!(
                self.layout
                    .composition()
                    .panel_effective_scale_mode(primary_panel_id),
                Some(PanelScaleMode::PercentFromBase)
            ) {
                self.primary_autoscale = true;
                let _ = self.panel_y_viewports.reset(primary_panel_id);
            }
            true
        } else {
            false
        }
    }

    fn remember_study_configs(&mut self, studies: &[KlineStudy]) {
        for study in studies {
            if let Some(configured) = self
                .study_configs
                .iter_mut()
                .find(|configured| configured.is_same_type(*study))
            {
                *configured = *study;
            } else {
                self.study_configs.push(*study);
            }
        }
    }

    fn apply_drawing_update(&mut self, update: DrawingUpdate) {
        if update.should_bump() {
            self.bump_rev();
        }
    }

    pub fn apply_command(&mut self, command: ChartCommand) -> CommandResult {
        let mut dirty = false;

        let result = match command {
            ChartCommand::SetPrimaryScaleMode(scale) => {
                let changed = self.set_primary_scale_mode(scale);
                if changed {
                    dirty = true;
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::SetBasis(basis) => {
                dirty = true;
                CommandResult::Action(self.set_basis(basis))
            }
            ChartCommand::Candlestick(command) => {
                self.apply_candlestick_command(command, &mut dirty)
            }
            ChartCommand::Footprint(command) => self.apply_footprint_command(command, &mut dirty),
            ChartCommand::Indicator(command) => self.apply_indicator_command(command, &mut dirty),
            ChartCommand::Study(command) => self.apply_study_command(command, &mut dirty),
            ChartCommand::Panel(command) => self.apply_panel_command(command, &mut dirty),
            ChartCommand::Series(command) => self.apply_series_command(command, &mut dirty),
            ChartCommand::Ticker(command) => self.apply_ticker_command(command, &mut dirty),
        };

        if dirty {
            self.bump_rev();
        }
        result
    }

    fn apply_candlestick_command(
        &mut self,
        command: CandlestickCommand,
        dirty: &mut bool,
    ) -> CommandResult {
        match command {
            CandlestickCommand::SetConfig(config) => {
                let changed = self.candlestick_config != config;
                if changed {
                    self.candlestick_config = config;
                    *dirty = true;
                }
                CommandResult::Changed(changed)
            }
            CandlestickCommand::ResetSettings => {
                let config_changed = self.candlestick_config != CandlestickConfig::default();
                let mark_changed = self.layout.reset_primary_mark();
                let series_changed = self.data.reset_series_styles();
                self.series_editor.close();
                if config_changed {
                    self.candlestick_config = CandlestickConfig::default();
                }
                if config_changed || mark_changed || series_changed {
                    *dirty = true;
                }
                CommandResult::Changed(config_changed || mark_changed || series_changed)
            }
            CandlestickCommand::SetMark(mark) => {
                let changed = self.layout.set_primary_mark(mark);
                if changed {
                    *dirty = true;
                }
                CommandResult::Changed(changed)
            }
            CandlestickCommand::SetVisible(visible) => {
                let changed = self.candlesticks_visible != visible;
                if changed {
                    self.candlesticks_visible = visible;
                    *dirty = true;
                }
                CommandResult::Changed(changed)
            }
        }
    }

    fn apply_footprint_command(
        &mut self,
        command: FootprintCommand,
        dirty: &mut bool,
    ) -> CommandResult {
        match command {
            FootprintCommand::SetVisible(visible) => {
                let changed = self.footprints_visible != visible;
                if changed {
                    self.footprints_visible = visible;
                    self.configure_profile_caches();
                    *dirty = true;
                }
                CommandResult::Changed(changed)
            }
            FootprintCommand::SetTradeDataEnabled(enabled) => {
                let changed = self.trade_data_enabled != enabled;
                if changed {
                    self.trade_data_enabled = enabled;
                    if !matches!(self.basis, Basis::Tick(_)) {
                        self.footprints_visible = enabled;
                        if !enabled {
                            for id in self.layout.selected_indicators() {
                                if self.indicators.get(id).is_some_and(|instance| {
                                    indicator::requires_trade_data(instance.kind)
                                }) {
                                    let _ = self.layout.remove_indicator(id);
                                    let _ = self.indicators.remove(id);
                                }
                            }
                            self.studies.retain(|study| !study.requires_trade_data());
                        }
                    }
                    self.configure_profile_caches();
                    *dirty = true;
                    CommandResult::Changed(true)
                } else {
                    CommandResult::Changed(false)
                }
            }
            FootprintCommand::SetClusterKind(kind) => {
                let changed = self.footprint_cluster_kind != kind;
                if changed {
                    self.footprint_cluster_kind = kind;
                    *dirty = true;
                }
                CommandResult::Changed(changed)
            }
            FootprintCommand::SetPocConfig(config) => {
                let changed = self.footprint_poc_config != config;
                if changed {
                    self.footprint_poc_config = config;
                    self.configure_profile_caches();
                    *dirty = true;
                }
                CommandResult::Changed(changed)
            }
            FootprintCommand::ResetSettings => {
                let cluster_changed = self.footprint_cluster_kind != ClusterKind::default();
                let poc_changed = self.footprint_poc_config != FootprintPocConfig::default();
                if cluster_changed {
                    self.footprint_cluster_kind = ClusterKind::default();
                }
                if poc_changed {
                    self.footprint_poc_config = FootprintPocConfig::default();
                    self.configure_profile_caches();
                }
                let changed = cluster_changed || poc_changed;
                if changed {
                    *dirty = true;
                }
                CommandResult::Changed(changed)
            }
            FootprintCommand::SetTickMultiplier(multiplier) => {
                if self.data.set_footprint_tick_multiplier(multiplier) {
                    let active_indicators = self.layout.selected_indicators();
                    self.data.configure_series_for_basis(
                        self.basis,
                        &self.indicators,
                        &active_indicators,
                    );
                    self.configure_profile_caches();
                    *dirty = true;
                }
                CommandResult::None
            }
        }
    }

    fn apply_indicator_command(
        &mut self,
        command: IndicatorCommand,
        dirty: &mut bool,
    ) -> CommandResult {
        match command {
            IndicatorCommand::SetDataConfig { id, config } => {
                let requests_history = self.layout.selected_indicators().contains(&id);
                let changed = self
                    .indicators
                    .update_config(id, config.normalized_for_basis(self.basis));
                if changed {
                    let active_indicators = self.layout.selected_indicators();
                    self.data.configure_indicators(
                        self.basis,
                        &self.indicators,
                        &active_indicators,
                    );
                    *dirty = true;
                    if requests_history {
                        return CommandResult::Action(
                            self.request_fetch_action().map(super::Action::RequestFetch),
                        );
                    }
                }
                CommandResult::Changed(changed)
            }
            IndicatorCommand::SetColor {
                id,
                slot,
                accent,
                shade,
            } => {
                let Some(current) = self
                    .indicators
                    .get(id)
                    .map(|instance| instance.colors.color_for(slot))
                else {
                    return CommandResult::Changed(false);
                };
                let changed = self.indicators.update_color(
                    id,
                    slot,
                    data::config::theme::ThemeColor::with_accent(current.role, accent, shade),
                );
                if changed {
                    *dirty = true;
                }
                CommandResult::Changed(changed)
            }
            IndicatorCommand::SetFillVisibility { id, visible } => {
                let changed = self.indicators.update_fill_visibility(id, visible);
                if changed {
                    *dirty = true;
                }
                CommandResult::Changed(changed)
            }
            IndicatorCommand::SetLineWidth { id, slot, width } => {
                let changed = self.indicators.update_line_width(id, slot, width);
                if changed {
                    *dirty = true;
                }
                CommandResult::Changed(changed)
            }
            IndicatorCommand::SetStrokeStyle {
                id,
                slot,
                stroke_style,
            } => {
                let changed = self.indicators.update_stroke_style(id, slot, stroke_style);
                if changed {
                    *dirty = true;
                }
                CommandResult::Changed(changed)
            }
            IndicatorCommand::ResetSettings(id) => {
                let data_changed = self.indicators.reset_config(id);
                let color_changed = self.indicators.reset_colors(id);
                let style_changed = self.indicators.reset_style(id);
                let mark_changed = self.layout.reset_indicator_mark(id);
                let presentation_changed = self.layout.reset_indicator_presentation(id);
                let changed = data_changed
                    || color_changed
                    || style_changed
                    || mark_changed
                    || presentation_changed;

                if data_changed {
                    let active_indicators = self.layout.selected_indicators();
                    self.data.configure_indicators(
                        self.basis,
                        &self.indicators,
                        &active_indicators,
                    );
                }
                if changed {
                    *dirty = true;
                }

                CommandResult::Changed(changed)
            }
            IndicatorCommand::SetMark { id, mark } => {
                let changed = self.layout.set_indicator_mark(id, mark);
                if changed {
                    *dirty = true;
                }
                CommandResult::Changed(changed)
            }
            IndicatorCommand::ToggleVisibility(id) => {
                let changed = self.layout.toggle_indicator_visibility(id);
                if changed {
                    *dirty = true;
                }
                CommandResult::Changed(changed)
            }
            IndicatorCommand::Add(kind) => {
                if !self.indicator_is_supported_for_market(kind) {
                    return CommandResult::Changed(false);
                }
                let id = self.indicators.create(kind);
                let changed =
                    self.layout
                        .add_indicator(id, kind, self.data.base_ticker().market_type());
                if !changed {
                    let _ = self.indicators.remove(id);
                    return CommandResult::Changed(false);
                }
                let active_indicators = self.layout.selected_indicators();
                self.data
                    .configure_indicators(self.basis, &self.indicators, &active_indicators);
                *dirty = true;
                CommandResult::Action(self.request_fetch_action().map(super::Action::RequestFetch))
            }
            IndicatorCommand::Remove(id) => {
                let changed = self.layout.remove_indicator(id);
                if changed {
                    let _ = self.indicators.remove(id);
                    let active_indicators = self.layout.selected_indicators();
                    self.data.configure_indicators(
                        self.basis,
                        &self.indicators,
                        &active_indicators,
                    );
                    *dirty = true;
                }
                CommandResult::Changed(changed)
            }
        }
    }

    fn apply_study_command(&mut self, command: StudyCommand, dirty: &mut bool) -> CommandResult {
        match command {
            StudyCommand::Set(studies) => {
                let normalized = self.normalize_studies_for_basis(studies);

                self.remember_study_configs(&normalized);
                let changed = self.studies != normalized;
                if changed {
                    self.studies = normalized;
                    self.configure_profile_caches();
                    *dirty = true;
                }
                CommandResult::Changed(changed)
            }
            StudyCommand::Toggle(study) => {
                let is_active = self.studies.iter().any(|active| active.is_same_type(study));
                self.set_study_active(study, !is_active)
            }
            StudyCommand::ToggleVisibility(id) => {
                let changed = self
                    .studies
                    .iter()
                    .position(|study| study.id() == id)
                    .map(|index| {
                        let study = self.studies[index];
                        self.studies[index] = study.with_visibility(!study.visible());
                        true
                    })
                    .unwrap_or(false);

                if changed {
                    let studies = self.studies.clone();
                    self.remember_study_configs(&studies);
                    *dirty = true;
                }

                CommandResult::Changed(changed)
            }
            StudyCommand::ToggleAllVisibility => {
                let should_show = !self.studies.iter().all(|study| study.visible());
                let mut changed = false;

                for study in &mut self.studies {
                    if study.visible() != should_show {
                        *study = study.with_visibility(should_show);
                        changed = true;
                    }
                }

                if changed {
                    let studies = self.studies.clone();
                    self.remember_study_configs(&studies);
                    *dirty = true;
                }

                CommandResult::Changed(changed)
            }
            StudyCommand::ToggleExpansion => {
                self.studies_legend_expanded = !self.studies_legend_expanded;
                *dirty = true;
                CommandResult::Changed(true)
            }
            StudyCommand::Remove(id) => {
                let original_len = self.studies.len();
                self.studies.retain(|study| study.id() != id);
                let changed = self.studies.len() != original_len;
                if changed {
                    self.configure_profile_caches();
                    *dirty = true;
                }
                CommandResult::Changed(changed)
            }
            StudyCommand::RemoveAll => {
                let had_studies = !self.studies.is_empty();
                if had_studies {
                    self.studies.clear();
                    self.configure_profile_caches();
                    *dirty = true;
                }
                CommandResult::Changed(had_studies)
            }
        }
    }

    fn apply_panel_command(&mut self, command: PanelCommand, dirty: &mut bool) -> CommandResult {
        match command {
            PanelCommand::SetMark { panel_index, mark } => {
                let changed = self.layout.set_panel_mark(panel_index, mark);
                if changed {
                    *dirty = true;
                }
                CommandResult::Changed(changed)
            }
            PanelCommand::SetDataKind {
                panel_index,
                data_kind,
            } => {
                let changed = self.layout.set_panel_data_kind(panel_index, data_kind);
                if changed {
                    *dirty = true;
                }
                CommandResult::Changed(changed)
            }
            PanelCommand::SetBarMode { panel_index, mode } => {
                let changed = self.layout.set_panel_bar_mode(panel_index, mode);
                if changed {
                    *dirty = true;
                }
                CommandResult::Changed(changed)
            }
            PanelCommand::SetHistogramMode { panel_index, mode } => {
                let changed = self.layout.set_panel_histogram_mode(panel_index, mode);
                if changed {
                    *dirty = true;
                }
                CommandResult::Changed(changed)
            }
        }
    }

    fn apply_series_command(&mut self, command: SeriesCommand, dirty: &mut bool) -> CommandResult {
        match command {
            SeriesCommand::SetColor { ticker_info, color } => {
                let changed = self.data.set_series_color(ticker_info, color);
                if changed {
                    *dirty = true;
                }
                CommandResult::Changed(changed)
            }
            SeriesCommand::SetLineWidth { ticker_info, width } => {
                let changed = self.data.set_series_line_width(ticker_info, width);
                if changed {
                    *dirty = true;
                }
                CommandResult::Changed(changed)
            }
            SeriesCommand::SetStrokeStyle {
                ticker_info,
                stroke_style,
            } => {
                let changed = self.data.set_series_stroke_style(ticker_info, stroke_style);
                if changed {
                    *dirty = true;
                }
                CommandResult::Changed(changed)
            }
            SeriesCommand::ResetSettings => {
                let changed = self.data.reset_series_styles();
                self.series_editor.close();
                if changed {
                    *dirty = true;
                }
                CommandResult::Changed(changed)
            }
        }
    }

    fn apply_ticker_command(&mut self, command: TickerCommand, dirty: &mut bool) -> CommandResult {
        match command {
            TickerCommand::Add(ticker_info) => {
                let active_indicators = self.layout.selected_indicators();
                if ticker_info != self.data.base_ticker()
                    && self.data.add_ticker(
                        ticker_info,
                        self.basis,
                        &self.indicators,
                        &active_indicators,
                    )
                {
                    self.fetch.add_ticker(ticker_info);
                    self.layout.sync_comparison_sources(
                        self.data.base_ticker(),
                        self.data.selected_tickers(),
                    );
                    *dirty = true;
                }
                CommandResult::Streams(self.streams_for_all())
            }
            TickerCommand::Remove(ticker_info) => {
                let was_multi_source = self.layout.primary_uses_multi_source();
                if ticker_info != self.data.base_ticker() && self.data.remove_ticker(ticker_info) {
                    if self.series_editor.show_config_for == Some(ticker_info) {
                        self.series_editor.close();
                    }
                    self.fetch.remove_ticker(ticker_info);
                    self.layout.sync_comparison_sources(
                        self.data.base_ticker(),
                        self.data.selected_tickers(),
                    );
                    if was_multi_source
                        && !self.layout.primary_uses_multi_source()
                        && self.layout.restore_primary_scale_after_comparisons()
                    {
                        self.primary_autoscale = true;
                        if let Some(primary_panel_id) = self.layout.composition().primary_panel_id()
                        {
                            let _ = self.panel_y_viewports.reset(primary_panel_id);
                        }
                    }
                    *dirty = true;
                }
                CommandResult::Streams(self.streams_for_all())
            }
        }
    }

    fn apply_persisted_config(&mut self, config: &persisted::Config) {
        self.indicators = config.indicator_instances_for(self.basis);
        if !config.panels.is_empty() {
            let panels = config
                .panels
                .iter()
                .map(persisted::panel_spec_from_config)
                .collect();
            self.layout
                .replace_composition(panels, config.splits.clone());
            self.layout
                .sync_comparison_sources(self.data.base_ticker(), self.data.selected_tickers());
        } else if let Some(volume_id) = self
            .indicators
            .instances()
            .iter()
            .find(|instance| instance.kind == KlineIndicator::Volume)
            .map(|instance| instance.id)
        {
            self.layout = IndicatorLayout::new(volume_id);
        }

        self.horizontal_world = HorizontalWorld::new(
            config.horizontal.cell_width,
            config.horizontal.scaling,
            DEFAULT_HORIZONTAL_OFFSET_UNITS,
        );
        self.primary_autoscale = config.primary_autoscale;
        self.candlesticks_visible = config.candlesticks_visible;
        self.footprints_visible = config.footprints_visible;
        self.trade_data_enabled = if config.version < 2 {
            config.footprints_visible
        } else {
            config.trade_data_enabled
        };
        if !self.trade_data_enabled && !matches!(self.basis, Basis::Tick(_)) {
            self.footprints_visible = false;
            for id in self.layout.selected_indicators() {
                if self
                    .indicators
                    .get(id)
                    .is_some_and(|instance| indicator::requires_trade_data(instance.kind))
                {
                    let _ = self.layout.remove_indicator(id);
                    let _ = self.indicators.remove(id);
                }
            }
            self.studies.retain(|study| !study.requires_trade_data());
        }
        self.candlestick_config = config.candlestick;
        self.footprint_cluster_kind = config.footprint_cluster_kind;
        self.footprint_poc_config = config.footprint_poc;
        self.studies = self.normalize_studies_for_basis(config.studies.iter().copied());
        self.study_configs = if config.study_configs.is_empty() {
            self.normalize_studies_for_basis(config.studies.iter().copied())
        } else {
            self.normalize_studies_for_basis(config.study_configs.iter().copied())
        };
        self.remember_study_configs(&self.studies.clone());
        let panel_y_viewports: Vec<_> = config
            .y_viewports
            .iter()
            .filter_map(|viewport| {
                let panel_id = PanelId(viewport.panel_id);
                self.layout.composition().panel(panel_id)?;
                Some((
                    panel_id,
                    PanelYViewport::normalized(viewport.min_value, viewport.max_value)?,
                ))
            })
            .collect();
        self.panel_y_viewports.replace(panel_y_viewports);
        let drawings = config
            .drawings
            .iter()
            .filter(|drawing| {
                DrawingTools::drawing_panels_exist(self.layout.composition(), &drawing.object)
            })
            .cloned()
            .collect();
        self.drawing = DrawingTools::from_parts(
            config.active_drawing_tool,
            drawings,
            config.drawing_style_defaults,
            config.horizontal_line_ray,
            config.box_extend_right,
        );

        let active_indicators = self.layout.selected_indicators();
        self.data
            .configure_indicators(self.basis, &self.indicators, &active_indicators);
        self.configure_profile_caches();

        for series_config in &config.series {
            let Some(index) = self
                .data
                .series
                .iter()
                .position(|series| series.ticker_info.ticker == series_config.ticker)
            else {
                continue;
            };

            let series_color = series_config
                .accent
                .map(|accent| SeriesColor::theme(Some(accent), series_config.shade))
                .or_else(|| series_config.color.map(SeriesColor::Custom))
                .unwrap_or_else(|| SeriesColor::theme(None, series_config.shade));
            let _ = self.data.series[index].set_series_color(series_color);
            let _ = self.data.series[index].set_line_width(series_config.line_width);
            let _ = self.data.series[index].set_stroke_style(series_config.stroke_style);
        }

        self.bump_rev();
    }

    fn reset_autoscale(&mut self) {
        self.primary_autoscale = true;
        self.panel_y_viewports = PanelViewportStore::default();
    }

    pub fn studies(&self) -> &[KlineStudy] {
        &self.studies
    }

    pub fn study(&self, id: KlineStudyId) -> Option<KlineStudy> {
        self.studies.iter().copied().find(|study| study.id() == id)
    }

    pub fn candlestick_config(&self) -> CandlestickConfig {
        self.candlestick_config
    }

    pub fn primary_mark(&self) -> MarkKind {
        self.layout.primary_mark().unwrap_or(MarkKind::Candle)
    }

    pub fn series_editor(&self) -> &TickerSeriesEditor {
        &self.series_editor
    }

    pub fn series_style_entries(&self) -> Vec<SeriesStyleEntry> {
        self.data
            .series
            .iter()
            .map(|series| SeriesStyleEntry {
                ticker: series.ticker_info,
                color: series.color(),
                line_width: series.line_width(),
                stroke_style: series.stroke_style(),
            })
            .collect()
    }

    pub fn trade_data_enabled(&self) -> bool {
        self.trade_data_enabled || matches!(self.basis, Basis::Tick(_))
    }

    pub fn indicator_requires_trade_data(&self, indicator: KlineIndicator) -> bool {
        indicator::requires_trade_data(indicator)
    }

    pub fn indicator_produces_signed_overlay(&self, id: IndicatorInstanceId) -> bool {
        self.data
            .base_series()
            .map(|series| series.indicator_produces_signed_overlay(id))
            .unwrap_or(false)
    }

    pub fn study_is_available(&self, study: KlineStudy) -> bool {
        study.normalized_for_basis(self.basis).is_some()
            && (self.trade_data_enabled() || !study.requires_trade_data())
    }

    pub fn footprint_cluster_kind(&self) -> ClusterKind {
        self.footprint_cluster_kind
    }

    pub fn footprint_poc_config(&self) -> FootprintPocConfig {
        self.footprint_poc_config
    }

    pub fn study_configurator(&self) -> &study::Configurator<KlineStudy> {
        &self.study_configurator
    }

    pub fn update_study_configurator(&mut self, message: study::Message<KlineStudy>) {
        let Some(action) = self.study_configurator.update(message) else {
            return;
        };

        match action {
            study::Action::ToggleStudy(study, is_selected) => {
                let _ = self.set_study_active(study, is_selected);
            }
            study::Action::ConfigureStudy(study) => {
                let mut studies = self.studies.clone();
                if let Some(active) = studies.iter_mut().find(|active| active.is_same_type(study)) {
                    *active = study;
                }
                let _ = self.apply_command(ChartCommand::Study(StudyCommand::Set(studies)));
            }
        }
    }

    fn set_study_active(&mut self, study: KlineStudy, is_active: bool) -> CommandResult {
        if is_active && !self.study_is_available(study) {
            return CommandResult::Changed(false);
        }

        let mut studies = self.studies.clone();
        if is_active {
            let configured = self
                .study_configs
                .iter()
                .find(|saved| saved.is_same_type(study))
                .copied()
                .unwrap_or(study)
                .with_visibility(true);

            if studies.iter().any(|active| active.is_same_type(configured)) {
                return CommandResult::Changed(false);
            }

            studies.push(configured);
        } else {
            studies.retain(|active| !active.is_same_type(study));
        }

        self.apply_command(ChartCommand::Study(StudyCommand::Set(studies)))
    }

    fn normalize_studies_for_basis(
        &self,
        studies: impl IntoIterator<Item = KlineStudy>,
    ) -> Vec<KlineStudy> {
        let mut normalized = Vec::new();
        for study in studies {
            let Some(study) = study.normalized_for_basis(self.basis) else {
                continue;
            };

            if !normalized
                .iter()
                .any(|existing: &KlineStudy| existing.is_same_type(study))
            {
                normalized.push(study);
            }
        }
        normalized
    }

    pub fn basis(&self) -> Basis {
        self.basis
    }

    pub fn selected_indicators(&self) -> Vec<IndicatorInstanceId> {
        self.layout.selected_indicators()
    }

    pub fn indicator_is_available(&self, id: IndicatorInstanceId) -> bool {
        self.layout.indicator_is_available(
            id,
            self.basis,
            self.data.base_ticker(),
            self.data.base_series(),
            &self.indicators,
        )
    }

    pub fn indicator_mark(&self, id: IndicatorInstanceId) -> Option<MarkKind> {
        self.layout
            .indicator_mark(id, self.indicator_produces_signed_overlay(id))
    }

    pub fn indicator_data_config(&self, id: IndicatorInstanceId) -> Option<IndicatorConfig> {
        self.indicators.get(id).map(|instance| instance.config)
    }

    pub fn indicator_kind(&self, id: IndicatorInstanceId) -> Option<KlineIndicator> {
        self.indicators.get(id).map(|instance| instance.kind)
    }

    pub fn indicator_color_config(
        &self,
        id: IndicatorInstanceId,
    ) -> Option<data::chart::kline_v2::IndicatorColorConfig> {
        self.indicators.get(id).map(|instance| instance.colors)
    }

    pub fn indicator_style_config(&self, id: IndicatorInstanceId) -> Option<IndicatorStyleConfig> {
        self.indicators.get(id).map(|instance| instance.style)
    }

    pub fn indicator_is_supported_for_market(&self, indicator: KlineIndicator) -> bool {
        self.layout
            .indicator_is_supported_for_market(indicator, self.data.base_ticker())
    }

    pub fn ticker_info(&self) -> TickerInfo {
        self.data.base_ticker()
    }

    pub fn selected_tickers(&self) -> &[TickerInfo] {
        self.data.selected_tickers()
    }

    pub fn last_update(&self) -> Instant {
        self.last_tick
    }

    pub fn footprint_tick_multiplier(&self) -> TickMultiplier {
        self.data.footprint_tick_multiplier()
    }

    fn unavailable_indicator_panels(&self) -> Vec<UnavailableIndicator> {
        self.layout.unavailable_indicator_panels(
            self.basis,
            self.data.base_ticker(),
            self.data.base_series(),
            &self.indicators,
            self.trade_data_enabled(),
        )
    }

    fn oi_enabled_for_base(&self) -> bool {
        self.layout.oi_enabled(&self.indicators)
    }

    pub(crate) fn trade_data_required(&self) -> bool {
        self.trade_data_enabled()
    }

    fn streams_for_all(&self) -> Vec<StreamKind> {
        let mut streams = self.data.streams_for_all(self.basis);
        if matches!(self.basis, Basis::Time(_)) && !self.trade_data_required() {
            streams.retain(|stream| !matches!(stream, StreamKind::Trades { .. }));
        }
        streams
    }

    pub fn serializable_config(&self) -> persisted::Config {
        persisted::Config {
            version: 6,
            horizontal: persisted::HorizontalConfig {
                cell_width: self.horizontal_world.cell_width(),
                scaling: self.horizontal_world.scaling(),
                offset: self.horizontal_world.offset(),
            },
            primary_autoscale: self.primary_autoscale,
            candlesticks_visible: self.candlesticks_visible,
            footprints_visible: self.footprints_visible,
            trade_data_enabled: self.trade_data_enabled,
            candlestick: self.candlestick_config,
            footprint_cluster_kind: self.footprint_cluster_kind,
            footprint_poc: self.footprint_poc_config,
            panels: self
                .layout
                .composition()
                .panels
                .iter()
                .map(persisted::panel_config_from_spec)
                .collect(),
            splits: self.layout.composition().splits.clone(),
            y_viewports: self
                .panel_y_viewports
                .iter()
                .map(|(panel_id, viewport)| persisted::ViewportConfig {
                    panel_id: panel_id.0,
                    min_value: viewport.min_value,
                    max_value: viewport.max_value,
                })
                .collect(),
            drawings: self.drawing.drawings().to_vec(),
            active_drawing_tool: self.drawing.active_tool(),
            drawing_style_defaults: self.drawing.style_defaults(),
            horizontal_line_ray: self.drawing.horizontal_line_ray(),
            box_extend_right: self.drawing.box_extend_right(),
            studies: self.studies.clone(),
            study_configs: self.study_configs.clone(),
            indicator_instances: self.indicators.instances().to_vec(),
            indicator_configs: None,
            indicator_colors: Default::default(),
            series: self
                .data
                .series
                .iter()
                .filter(|series| !series.color().is_default() || series.has_custom_line_style())
                .map(|series| {
                    let color = series.color();
                    persisted::SeriesConfig {
                        ticker: series.ticker_info.ticker,
                        color: match color {
                            SeriesColor::Custom(color) => Some(color),
                            SeriesColor::Theme(_) => None,
                        },
                        accent: color.accent_value(),
                        shade: color
                            .theme_color()
                            .map_or(data::config::theme::ThemeColorShade::Base, |color| {
                                color.shade
                            }),
                        line_width: series.line_width(),
                        stroke_style: series.stroke_style(),
                    }
                })
                .collect(),
            ..persisted::Config::default()
        }
    }
}
