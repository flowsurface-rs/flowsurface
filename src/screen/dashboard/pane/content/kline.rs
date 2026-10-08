mod drawing;
mod fetch;
mod history;
mod indicator_browser;
mod layout;
mod presentation;
mod profile;
mod rebuild;
mod settings;
mod studies;
mod toolbar;
mod updates;
mod viewport;

use crate::connector::fetcher::FetchSpec;
use crate::screen::dashboard::pane::ChartAction;
use crate::style;
use crate::theme::{ThemeAccents, ThemeState};
use crate::widget::chart::kline::studies::StudyObjectEvent;
use crate::widget::chart::kline::{
    IndicatorPresentationContext, KlineWidget, KlineWidgetEvent, MIN_FULL_CACHE_REFRESH_INTERVAL,
    PanelInteraction,
};
use crate::widget::inspector::{InspectorBody, InspectorContent, InspectorFooter};
use crate::widget::series_editor;
use crate::window::PixelRatio;

use drawing::{DrawingMessage, DrawingState, DrawingTools, DrawingUpdate};
use fetch::FetchCoordinator;
use layout::{IndicatorRebuildReason, IndicatorState};
use settings::{ChartOptions, InspectorEffect, InspectorState, SettingsContext};
use studies::{StudyChange, StudyCommand, StudyState};
use updates::DataUpdateEffect;
use viewport::ViewportState;

pub(crate) use fetch::FetchUpdate;
pub(crate) use layout::{IndicatorCommand, IndicatorInputCommand, IndicatorStyleCommand};
pub(crate) use settings::Message as SettingsMessage;
pub(crate) use updates::DataUpdate;

use data::chart::Basis;
use data::chart::kline::composition::{
    ChartComposition, DEFAULT_MIN_PANEL_RATIO, MarkKind, PanelRole, PanelScaleMode,
};
use data::chart::kline::footprint::{ClusterKind, ClusterScaling};
use data::chart::kline::series::KlineSeries;
use data::chart::kline::{
    self as persisted, CandlestickConfig, ChartSettings, DEFAULT_HORIZONTAL_SPACING_PX, FetchBatch,
    FetchPlanInput, FetchStage, FootprintImbalanceConfig, FootprintPocConfig, HorizontalWorld,
    IndicatorId, IndicatorInputSource, IndicatorInstanceId, IndicatorInstanceStore,
    IndicatorOutputRef, KlineData, KlineStudy, LegendVisibility, LineStrokeStyle, SeriesColor,
    SettingsSnapshot, visible_window,
};

use exchange::adapter::StreamKind;
use exchange::{TickMultiplier, TickerInfo, UnixMs};

use std::time::Instant;

const DEFAULT_HORIZONTAL_OFFSET_UNITS: f32 = 8.0;
const DEFAULT_TICK_MULTIPLIER: TickMultiplier = TickMultiplier(25);
const FLOATING_TOOLBAR_MARGIN: f32 = 8.0;
const FLOATING_TOOLBAR_HANDLE_WIDTH: f32 = 28.0;
const FLOATING_TOOLBAR_HEIGHT: f32 = 36.0;
const SETTINGS_SIDEBAR_WIDTH: f32 = 240.0;

pub enum Action {
    RemoveSeries(TickerInfo),
    RequestFetch(Vec<FetchSpec>),
    OpenUrl(String),
    OpenNetworkSettings,
}

#[derive(Debug, Clone)]
pub enum Message {
    Chart(KlineWidgetEvent),
    Drawing(drawing::Event),
    ToggleDrawingSidebar,
    CloseContextualSettings,
    SeriesEditor(series_editor::Message),
    IndicatorBrowser(indicator_browser::Message),
    IndicatorRebuildCompleted { job_id: u64 },
    Settings(SettingsMessage),
    OpenNetworkSettings,
}

pub struct KlineChart {
    basis: Basis,
    viewport: ViewportState,
    options: ChartOptions,
    data: KlineData,
    indicators: IndicatorState,
    fetch: FetchCoordinator,
    drawings: DrawingState,
    studies: StudyState,
    inspector: InspectorState,
    indicator_browser: indicator_browser::State,
    drawing_sidebar_visible: bool,
    settings_reconfigure_pending: bool,
    render_invalidation: RenderInvalidation,
    last_maintenance_at: Instant,
}

impl KlineChart {
    pub(crate) fn new(basis: Basis, tickers: &[TickerInfo]) -> Self {
        let base_ticker = tickers
            .first()
            .copied()
            .expect("Kline chart requires a base ticker");
        let tick_multiplier = DEFAULT_TICK_MULTIPLIER;
        let indicators = IndicatorState::new();

        let mut chart = Self {
            basis,
            viewport: ViewportState::default(),
            options: ChartOptions::default(),
            data: KlineData::new(base_ticker, tick_multiplier),
            indicators,
            fetch: FetchCoordinator::new(&[]),
            drawings: DrawingState::default(),
            studies: StudyState::default(),
            inspector: InspectorState::default(),
            indicator_browser: indicator_browser::State::default(),
            drawing_sidebar_visible: true,
            last_maintenance_at: Instant::now(),
            render_invalidation: RenderInvalidation::new(),
            settings_reconfigure_pending: false,
        };

        let _ = chart.data.add_ticker(base_ticker, chart.basis);
        for ticker in tickers.iter().copied() {
            if ticker != base_ticker {
                let _ = chart.data.add_ticker(ticker, chart.basis);
            }
        }

        let selected_tickers = chart.data.selected_tickers();
        chart.fetch.rebuild_handlers(&selected_tickers);
        chart.data.configure_series_for_basis(chart.basis);
        chart.drawings.tools.set_basis(basis);
        chart.sync_tick_drawing_sequence();
        chart
            .indicators
            .composition
            .sync_comparison_sources(chart.data.base_ticker(), &selected_tickers);
        chart.refresh_indicator_presentation();
        chart
    }

    pub(crate) fn new_with_config(
        basis: Basis,
        tickers: &[TickerInfo],
        config: &persisted::Config,
    ) -> Self {
        let mut chart = Self::new(basis, tickers);
        chart.apply_persisted_config(config);
        chart.viewport.reset_autoscale();
        chart
    }

    pub(crate) fn update(
        &mut self,
        message: Message,
        chart_settings: &mut ChartSettings,
    ) -> Option<Action> {
        match message {
            Message::IndicatorRebuildCompleted { job_id } => {
                self.complete_indicator_rebuild(job_id);
            }
            Message::ToggleDrawingSidebar => {
                if !self.close_contextual_settings() {
                    self.drawing_sidebar_visible = !self.drawing_sidebar_visible;
                    self.render_invalidation.invalidate();
                }
            }
            Message::CloseContextualSettings => {
                self.close_contextual_settings();
            }
            Message::IndicatorBrowser(message) => match message {
                indicator_browser::Message::SearchChanged(query) => {
                    self.indicator_browser.set_search_query(query);
                }
                indicator_browser::Message::ToggleIndicator(indicator) => {
                    return self.apply_browser_action(ChartCommand::AddIndicator(indicator));
                }
                indicator_browser::Message::ToggleStudy(study) => {
                    return self
                        .apply_browser_action(ChartCommand::Study(StudyCommand::Toggle(study)));
                }
            },
            Message::OpenNetworkSettings => return Some(Action::OpenNetworkSettings),
            Message::Settings(message) => {
                let effect = self
                    .with_settings_context(|inspector, context| inspector.update(message, context));
                let action = self.apply_inspector_effect(effect);
                self.reconcile_settings_target();
                return action;
            }
            Message::SeriesEditor(message) => {
                let effect = self.with_settings_context(|inspector, context| {
                    inspector.update(SettingsMessage::SeriesEditor(message), context)
                });
                let action = self.apply_inspector_effect(effect);
                self.reconcile_settings_target();
                return action;
            }
            Message::Drawing(message) => {
                let update = self.drawings.update(message, &mut chart_settings.drawing);
                self.apply_drawing_update(update);
            }
            Message::Chart(event) => {
                match event {
                    KlineWidgetEvent::Drawing(event) => {
                        let update = self
                            .drawings
                            .handle_widget_drawing_event(event, &chart_settings.drawing);
                        self.apply_drawing_update(update);
                    }
                    KlineWidgetEvent::Study(event) => {
                        let _ = self.apply_command(ChartCommand::StudyInteraction(event));
                    }
                    KlineWidgetEvent::StudySettings(id) => {
                        if self.studies.study(id).is_some() {
                            self.change_settings_target(|inspector| inspector.open_study(id));
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
                    KlineWidgetEvent::UndoRequested => return self.undo_edit(),
                    KlineWidgetEvent::RedoRequested => return self.redo_edit(),
                    KlineWidgetEvent::EscapePressed => {
                        let _ = self.handle_escape();
                    }
                    KlineWidgetEvent::PrimaryScaleInversionRequested => {
                        self.drawings.finish_pending_edit();
                        self.viewport.primary_scale_inverted =
                            !self.viewport.primary_scale_inverted;
                        self.render_invalidation.invalidate();
                    }
                    KlineWidgetEvent::PanelInteraction(interaction) => match interaction {
                        PanelInteraction::YViewportChanged { panel_id, range } => {
                            let composition = &self.indicators.composition;
                            if !viewport::allows_panel_range(composition, panel_id) {
                                return None;
                            }
                            if self.viewport.set_panel_range(composition, panel_id, range) {
                                self.render_invalidation.invalidate();
                            }
                        }
                        PanelInteraction::YViewportReset { panel_id } => {
                            let composition = &self.indicators.composition;
                            if !viewport::allows_panel_range(composition, panel_id) {
                                return None;
                            }
                            if self.viewport.reset_panel_range(composition, panel_id) {
                                self.render_invalidation.invalidate();
                            }
                        }
                        PanelInteraction::SplitChanged { index, split } => {
                            self.drawings.finish_pending_edit();
                            if self.indicators.composition.set_split(
                                index,
                                split,
                                DEFAULT_MIN_PANEL_RATIO,
                            ) {
                                self.drawings.clear_history();
                                self.render_invalidation.invalidate();
                            }
                        }
                        PanelInteraction::MoveUp { index } => {
                            self.drawings.finish_pending_edit();
                            if index > 0 && self.indicators.composition.move_panel(index, index - 1)
                            {
                                self.refresh_indicator_presentation();
                                self.render_invalidation.invalidate();
                            }
                        }
                        PanelInteraction::MoveDown { index } => {
                            self.drawings.finish_pending_edit();
                            let target = index.saturating_add(1);
                            if target < self.indicators.composition.panels.len()
                                && self.indicators.composition.move_panel(index, target)
                            {
                                self.refresh_indicator_presentation();
                                self.render_invalidation.invalidate();
                            }
                        }
                        PanelInteraction::Close { index } => {
                            self.close_panel_at_index(index);
                        }
                    },
                    KlineWidgetEvent::HorizontalWorldChanged(world) => {
                        self.viewport.horizontal_world = world;
                        self.render_invalidation.invalidate();
                    }
                    KlineWidgetEvent::HorizontalOffsetChanged(offset) => {
                        self.viewport.horizontal_world =
                            self.viewport.horizontal_world.with_offset(offset);
                        self.render_invalidation.invalidate();
                    }
                    KlineWidgetEvent::PlotWidthChanged(plot_width) => {
                        if self.viewport.plot_width == Some(plot_width) {
                            return None;
                        }

                        self.viewport.plot_width = Some(plot_width);
                        self.render_invalidation.invalidate();
                    }
                    KlineWidgetEvent::PrimaryAutoscaleDisabled => {
                        if self.viewport.primary.disable() {
                            self.render_invalidation.invalidate();
                        }
                    }
                    KlineWidgetEvent::PrimaryAutoscaleToggled => {
                        self.indicators.composition.primary_panel_id()?;
                        if self
                            .viewport
                            .toggle_primary_autoscale(&self.indicators.composition)
                        {
                            self.render_invalidation.invalidate();
                        }
                    }
                    KlineWidgetEvent::PrimaryScaleModeCycleRequested => {
                        self.drawings.finish_pending_edit();
                        let primary_panel_id = self.indicators.composition.primary_panel_id()?;
                        let panel = self.indicators.composition.panel(primary_panel_id)?;

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
                            .indicators
                            .composition
                            .set_panel_preferred_scale(primary_panel_id, next_scale)
                        {
                            self.viewport.primary.reset();

                            self.refresh_indicator_presentation();
                            self.render_invalidation.invalidate();
                        }
                    }
                    KlineWidgetEvent::TickerSettings(ticker) => {
                        let is_base_ticker = ticker == self.data.base_ticker();
                        self.change_settings_target(|inspector| {
                            if is_base_ticker {
                                inspector.open_candlestick();
                            } else {
                                inspector.open_series(ticker);
                            }
                        });
                    }
                    KlineWidgetEvent::FootprintSettings => {
                        self.change_settings_target(InspectorState::open_footprint);
                    }
                    KlineWidgetEvent::TickerVisibilityToggled(ticker_info) => {
                        let command = if ticker_info == self.data.base_ticker() {
                            ChartCommand::SetCandlestickVisible {
                                visible: !self.options.candlestick.visible,
                            }
                        } else {
                            ChartCommand::ToggleComparisonVisibility { ticker_info }
                        };
                        let _ = self.apply_command(command);
                    }
                    KlineWidgetEvent::FootprintsVisibilityToggled => {
                        let _ = self.apply_command(ChartCommand::SetFootprintVisible {
                            visible: !self.options.footprint.visible,
                        });
                    }
                    KlineWidgetEvent::IndicatorSettings(id) => {
                        if let Some(indicator) = self.indicator_kind(id) {
                            self.change_settings_target(|inspector| {
                                inspector.open_indicator(id, indicator);
                            });
                        }
                    }
                    KlineWidgetEvent::IndicatorVisibilityToggled(id) => {
                        let _ = self.apply_command(ChartCommand::indicator_style(
                            IndicatorStyleCommand::ToggleVisibility(id),
                        ));
                    }
                    KlineWidgetEvent::IndicatorRemove(id) => {
                        let _ = self.apply_command(ChartCommand::RemoveIndicator(id));
                    }
                    KlineWidgetEvent::TickerRemove(ticker) if ticker != self.data.base_ticker() => {
                        return Some(Action::RemoveSeries(ticker));
                    }
                    KlineWidgetEvent::TickerRemove(_ticker) => {}
                    KlineWidgetEvent::XAxisDoubleClick => {
                        let world = HorizontalWorld::new(
                            DEFAULT_HORIZONTAL_SPACING_PX,
                            DEFAULT_HORIZONTAL_OFFSET_UNITS,
                        );
                        self.viewport.horizontal_world = world;
                        self.render_invalidation.invalidate();
                    }
                    KlineWidgetEvent::DeletePressed => {
                        let update = self.drawings.update(
                            drawing::Event::Tools(DrawingMessage::DeleteSelectedDrawing),
                            &mut chart_settings.drawing,
                        );
                        self.apply_drawing_update(update);
                    }
                }
                self.reconcile_settings_target();
            }
        }
        None
    }

    pub(crate) fn view<'a>(
        &'a self,
        timezone: data::UserTimezone,
        pixel_ratio: PixelRatio,
        theme: &'a ThemeState,
        inspector_open: bool,
    ) -> iced::Element<'a, Message> {
        let theme_accents = theme.accents();
        let chart: iced::Element<'_, Message> =
            if self.data.series().iter().all(KlineSeries::is_empty) {
                iced::widget::center(iced::widget::text("Waiting for data...").size(16)).into()
            } else {
                let indicator_presentation = self
                    .indicator_presentation_context()
                    .with_rebuild_in_flight_ids(self.indicators.rebuild_in_flight_ids())
                    .with_pending_input_ids(
                        self.indicators.pending_input_ids(&self.data, &self.fetch),
                    );
                let trade_data_enabled = self.trade_data_enabled();
                let drawing_snapshot = self.drawings.tools.snapshot();
                let chart = KlineWidget::new(
                    self.data.series(),
                    self.data.indicator_host(),
                    self.basis,
                    &self.indicators.composition,
                    &self.indicators.instances,
                    indicator_presentation,
                    theme,
                )
                .with_drawing_snapshot(drawing_snapshot)
                .with_candlestick_config(self.options.candlestick)
                .with_footprint_config(self.options.footprint)
                .with_trade_data_enabled(trade_data_enabled);
                let chart = self
                    .studies
                    .configure_widget(
                        chart,
                        self.basis,
                        &self.data,
                        trade_data_enabled,
                        self.render_invalidation.revision(),
                    )
                    .with_legend_visibility(self.options.legend_visibility);

                let chart: iced::Element<'_, Message> = chart
                    .with_coordinate_state(self.viewport.horizontal_world, pixel_ratio)
                    .with_primary_viewport(self.viewport.primary)
                    .with_primary_scale_inverted(self.viewport.primary_scale_inverted)
                    .with_known_plot_width(self.viewport.plot_width)
                    .with_autoscale_plot_width_offset(self.autoscale_plot_width_offset())
                    .with_indicator_ranges(self.viewport.indicator_y_ranges.as_slice())
                    .with_timezone(timezone)
                    .with_version(
                        self.render_invalidation
                            .revision()
                            .wrapping_add(self.studies.runtime_revision()),
                    )
                    .with_should_capture_escape(inspector_open || self.can_handle_escape())
                    .into();

                iced::widget::container(chart)
                    .width(iced::Length::Fill)
                    .height(iced::Length::Fill)
                    .padding(1)
                    .into()
            };

        let chart = {
            let chart_stack = iced::widget::Stack::new()
                .width(iced::Length::Fill)
                .height(iced::Length::Fill)
                .push(chart)
                .push(self.drawings.view(theme_accents).map(Message::Drawing));

            let fetch_activity = self.fetch.pending_activity();
            let fetch_activity: Option<iced::Element<'_, Message>> = if fetch_activity.is_empty() {
                None
            } else {
                let mut lines = iced::widget::column![].spacing(2);
                for activity in fetch_activity {
                    let line = match activity {
                        fetch::FetchActivity::Kline => "Fetching Klines...".to_string(),
                        fetch::FetchActivity::Trades { fetched } => {
                            format!("Fetching Trades... {fetched} fetched")
                        }
                        fetch::FetchActivity::OpenInterest => {
                            "Fetching Open Interest...".to_string()
                        }
                    };
                    lines = lines.push(iced::widget::text(line).size(12));
                }

                Some(
                    iced::widget::row![
                        iced::widget::space::horizontal(),
                        iced::widget::opaque(
                            iced::widget::container(lines)
                                .style(style::chart_info_tooltip)
                                .padding([4, 8])
                        ),
                    ]
                    .padding(8)
                    .into(),
                )
            };

            let chart_stack = if let Some(fetch_activity) = fetch_activity {
                chart_stack.push(fetch_activity)
            } else {
                chart_stack
            };

            chart_stack.clip(true).into()
        };

        let sidebar = if self.has_settings_target() {
            self.contextual_settings_sidebar(theme_accents)
        } else if self.drawing_sidebar_visible {
            Some(
                self.drawings
                    .sidebar_view(&self.indicators.composition)
                    .map(Message::Drawing),
            )
        } else {
            None
        };
        let Some(sidebar) = sidebar else {
            return chart;
        };

        iced::widget::row![
            sidebar,
            iced::widget::rule::vertical(1.0).style(style::split_ruler),
            chart,
        ]
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
        .into()
    }

    pub(crate) fn invalidate(&mut self, now: Option<Instant>) -> Option<ChartAction> {
        if let Some(ts) = now {
            self.last_maintenance_at = ts;

            if self.flush_settings_update() {
                self.render_invalidation.invalidate();
            }

            if self.studies.rebuild(self.basis, &self.data, UnixMs::now()) {
                self.render_invalidation.invalidate();
            }

            let action = self
                .request_fetch_action(true)
                .map(ChartAction::RequestFetch);
            self.recalculate_indicators();
            self.preview_clean_indicators();
            self.redraw_data_if_due(ts);
            return action;
        }

        self.render_invalidation.invalidate();
        None
    }

    pub(crate) fn rebuild_indicators(&mut self) -> Option<iced::Task<Message>> {
        if let Some(job_id) = self.indicators.pending_rebuild_job() {
            self.complete_indicator_rebuild(job_id);
        }
        if self.settings_reconfigure_pending {
            return None;
        }

        let trade_data_enabled = self.trade_data_enabled();
        let worker =
            self.indicators
                .begin_full_rebuild(&self.data, &self.fetch, trade_data_enabled)?;
        self.refresh_indicator_presentation();
        self.render_invalidation.invalidate();
        Some(iced::Task::perform(worker, |job_id| {
            Message::IndicatorRebuildCompleted { job_id }
        }))
    }

    fn complete_indicator_rebuild(&mut self, job_id: u64) {
        let trade_data_enabled = self.trade_data_enabled();
        if self
            .indicators
            .complete_full_rebuild(&mut self.data, job_id, trade_data_enabled)
        {
            self.render_invalidation.invalidate();
        }
        self.refresh_indicator_presentation();
    }

    fn request_settings_update(&mut self, policy: UpdatePolicy) {
        match policy {
            UpdatePolicy::Redraw => self.render_invalidation.invalidate(),
            UpdatePolicy::DeferredReconfigure => self.settings_reconfigure_pending = true,
            UpdatePolicy::ImmediateReconfigure => {
                self.settings_reconfigure_pending = true;
                self.flush_settings_update();
                self.render_invalidation.invalidate();
            }
        }
    }

    fn flush_settings_update(&mut self) -> bool {
        if !std::mem::take(&mut self.settings_reconfigure_pending) {
            return false;
        }

        self.data.configure_series_for_basis(self.basis);
        self.reconcile_indicator_layout(false);
        self.sync_tick_drawing_sequence();
        self.prune_stale_indicator_y_ranges();
        self.refresh_indicator_presentation();
        true
    }

    pub(crate) fn pane_settings_view(&self) -> InspectorContent<'static, Message> {
        super::settings::kline_cfg_view(
            self.trade_data_enabled(),
            self.options.legend_visibility,
            self.trade_data_setting_lock_reason(),
        )
    }

    pub(crate) fn indicator_browser_view(&self) -> iced::Element<'_, Message> {
        indicator_browser::view(self, &self.indicator_browser)
    }

    pub(crate) fn indicator_browser_focus_id(&self) -> iced::widget::Id {
        self.indicator_browser.search_box_id()
    }

    pub(crate) fn indicator_kind(&self, id: IndicatorInstanceId) -> Option<IndicatorId> {
        self.indicators.indicator_kind(id)
    }

    pub(super) fn preserve_indicator_browser_state_from(&mut self, source: &Self) {
        self.indicator_browser = source.indicator_browser.clone();
    }

    fn autoscale_plot_width_offset(&self) -> f32 {
        let width = if self.has_settings_target() {
            SETTINGS_SIDEBAR_WIDTH
        } else if self.drawing_sidebar_visible {
            self.drawings.tools.sidebar_width()
        } else {
            return 0.0;
        };
        (width - DrawingTools::compact_sidebar_width()).max(0.0)
    }

    fn has_settings_target(&self) -> bool {
        self.inspector.has_contextual_target()
    }

    pub(crate) fn base_ticker_info(&self) -> TickerInfo {
        self.data.base_ticker()
    }

    pub(crate) fn series(&self) -> &[KlineSeries] {
        self.data.series()
    }

    pub(crate) fn studies(&self) -> &[KlineStudy] {
        self.studies.active()
    }

    pub(crate) fn basis(&self) -> Basis {
        self.basis
    }

    pub(crate) fn selected_indicators(&self) -> Vec<IndicatorInstanceId> {
        self.indicators.selected_indicators()
    }

    pub(crate) fn last_maintenance_at(&self) -> Instant {
        self.last_maintenance_at
    }

    pub(crate) fn tick_multiplier(&self) -> TickMultiplier {
        self.data.tick_multiplier()
    }

    fn contextual_settings_sidebar(
        &self,
        theme_accents: ThemeAccents,
    ) -> Option<iced::Element<'_, Message>> {
        let title = self
            .inspector
            .settings()
            .contextual_title(self.settings_view_context())?;

        let back = iced::widget::button(style::icon_text(style::Icon::Return, 14))
            .on_press(Message::CloseContextualSettings)
            .style(|theme, status| style::button::transparent(theme, status, false));

        let header = iced::widget::row![
            crate::widget::text::section(title)
                .wrapping(iced::widget::text::Wrapping::Word)
                .width(iced::Length::Fill),
            back,
        ]
        .align_y(iced::Alignment::Center)
        .spacing(8);

        Some(
            iced::widget::container(iced::widget::column![
                header,
                iced::widget::rule::horizontal(1.0).style(style::split_ruler),
                iced::widget::responsive(move |size| {
                    let Some((tab, body)) = self
                        .inspector
                        .settings()
                        .contextual_content(self.settings_view_context(), theme_accents)
                    else {
                        return iced::widget::column![].into();
                    };
                    let footer = self
                        .inspector
                        .settings()
                        .footer_view(self.settings_view_context(), tab);
                    let footer_height = footer.as_ref().map_or(0.0, |(height, _)| *height);
                    let mut content = iced::widget::column![InspectorBody::view(
                        body,
                        (size.height - footer_height).max(0.0),
                    )];
                    if let Some((height, footer)) = footer {
                        content = content.push(InspectorFooter::view(footer, height));
                    }
                    content.into()
                }),
            ])
            .width(SETTINGS_SIDEBAR_WIDTH)
            .height(iced::Length::Fill)
            .padding([4, 8])
            .style(style::chart_sidebar_container)
            .into(),
        )
    }

    fn can_handle_escape(&self) -> bool {
        self.has_settings_target()
            || self.inspector.is_confirming_reset()
            || self.drawings.tools.can_handle_escape()
    }

    fn change_settings_target(&mut self, change: impl FnOnce(&mut InspectorState)) {
        self.flush_settings_update();
        change(&mut self.inspector);
        self.render_invalidation.invalidate();
    }

    fn close_contextual_settings(&mut self) -> bool {
        if !self.has_settings_target() {
            return false;
        }
        self.change_settings_target(InspectorState::close_contextual_settings);
        true
    }

    pub(crate) fn go_back(&mut self) -> bool {
        self.inspector.dismiss_reset_confirmation() || self.close_contextual_settings()
    }

    pub(crate) fn left_sidebar_visible(&self) -> bool {
        self.has_settings_target() || self.drawing_sidebar_visible
    }

    fn remove_ticker(&mut self, ticker_info: TickerInfo) -> bool {
        if ticker_info == self.data.base_ticker() || !self.data.remove_ticker(ticker_info) {
            return false;
        }

        let was_multi_source = self.indicators.composition.primary_uses_multi_source();
        self.fetch.remove_ticker(ticker_info);
        let selected_tickers = self.data.selected_tickers();
        self.indicators
            .composition
            .sync_comparison_sources(self.data.base_ticker(), &selected_tickers);
        if was_multi_source
            && !self.indicators.composition.primary_uses_multi_source()
            && self
                .indicators
                .composition
                .restore_primary_scale_after_comparisons()
        {
            self.viewport.primary.reset();
        }

        self.indicators.mark_for_candle_ticker(
            &self.data,
            ticker_info,
            IndicatorRebuildReason::IndicatorStructureChanged,
        );
        true
    }

    fn replace_indicator_configuration(
        &mut self,
        instances: IndicatorInstanceStore,
        panels: &[persisted::PanelConfig],
        splits: &[f32],
    ) {
        self.indicators.instances = instances;

        if !panels.is_empty() {
            let panels = panels
                .iter()
                .map(persisted::panel_spec_from_config)
                .collect();
            self.indicators.composition = ChartComposition::from_parts(panels, splits.to_vec());
        } else if let Some(volume_id) = self
            .indicators
            .instances
            .instances()
            .iter()
            .find(|instance| {
                instance.kind == IndicatorId::parse("core.volume").expect("static ID is valid")
            })
            .map(|instance| instance.id)
        {
            let mut composition = ChartComposition::prototype_kline();
            let volume = IndicatorId::parse("core.volume").expect("static ID is valid");
            let _ = composition.add_indicator(volume_id, volume);
            self.indicators.composition = composition;
        } else {
            self.indicators.composition = ChartComposition::prototype_kline();
        }
    }

    fn indicator_requires_trade_data(&self, indicator: &IndicatorId) -> bool {
        self.indicators
            .requires_trade_data(indicator, self.basis, self.data.base_ticker())
    }

    fn indicator_presentation_context(&self) -> IndicatorPresentationContext {
        self.indicators.presentation_context()
    }

    fn indicator_history_requirement(
        &self,
        active_indicators: &[IndicatorInstanceId],
        visible_points: u64,
    ) -> (Option<UnixMs>, u64) {
        let reference_time = match self.basis {
            Basis::Time(timeframe) => visible_window(
                self.data.series(),
                self.viewport.horizontal_world.offset(),
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

    fn recalculate_indicators(&mut self) -> bool {
        let trade_data_enabled = self.trade_data_enabled();
        let changed = self
            .indicators
            .recalculate(&mut self.data, &self.fetch, trade_data_enabled);
        if changed {
            self.refresh_indicator_presentation();
            self.render_invalidation.invalidate();
        }
        changed
    }

    fn preview_clean_indicators(&mut self) -> bool {
        let trade_data_enabled = self.trade_data_enabled();
        let changed = self
            .indicators
            .preview_clean(&mut self.data, trade_data_enabled);
        if changed {
            self.refresh_indicator_presentation();
            self.render_invalidation.invalidate();
        }
        changed
    }

    fn reconcile_indicator_layout(&mut self, rebuild_all: bool) {
        let selected_tickers = self.data.selected_tickers();
        self.indicators
            .composition
            .sync_comparison_sources(self.data.base_ticker(), &selected_tickers);
        let _ = self.indicators.reconcile_attached_indicator_placements();
        let active_indicators = self.indicators.selected_indicators();
        self.configure_indicator_runtimes(&active_indicators, rebuild_all);
        self.studies
            .configure_caches(self.basis, &mut self.data, &mut self.fetch);
    }

    fn configure_indicator_runtimes(
        &mut self,
        active_indicators: &[IndicatorInstanceId],
        rebuild_all: bool,
    ) {
        let changed_indicator_ids = self
            .data
            .configure_indicators(&self.indicators.instances, active_indicators);
        self.indicators.retain_dirty_for(active_indicators);
        if rebuild_all {
            self.indicators
                .mark_rebuild_pending(IndicatorRebuildReason::IndicatorStructureChanged);
        }
        for id in changed_indicator_ids {
            let reason = self
                .indicators
                .rebuild_reason(id)
                .unwrap_or(IndicatorRebuildReason::IndicatorStructureChanged);
            self.indicators
                .mark_with_dependents(&self.data, &[id], reason);
        }
    }

    fn refresh_indicator_presentation(&mut self) {
        let trade_data_enabled = self.trade_data_enabled();
        self.indicators
            .refresh_presentation(&self.data, self.basis, trade_data_enabled);
    }

    fn reconcile_settings_target(&mut self) {
        let was_open = self.has_settings_target();
        self.with_settings_context(|inspector, context| {
            inspector.reconcile_missing_target(context)
        });
        if was_open && !self.has_settings_target() {
            self.flush_settings_update();
            self.render_invalidation.invalidate();
        }
    }

    fn settings_view_context(&self) -> SettingsContext<'_> {
        SettingsContext {
            basis: self.basis,
            options: &self.options,
            data: &self.data,
            indicators: &self.indicators,
            studies: &self.studies,
            trade_data_enabled: self.trade_data_enabled(),
        }
    }

    fn with_settings_context<R>(
        &mut self,
        update: impl for<'a> FnOnce(&mut InspectorState, &SettingsContext<'a>) -> R,
    ) -> R {
        let trade_data_enabled = self.trade_data_enabled();
        let context = SettingsContext::new(
            self.basis,
            &self.options,
            &self.data,
            &self.indicators,
            &self.studies,
            trade_data_enabled,
        );
        update(&mut self.inspector, &context)
    }

    pub(crate) fn handle_escape(&mut self) -> bool {
        if self.go_back() {
            return true;
        }

        let update = self.drawings.handle_escape();
        if matches!(update, DrawingUpdate::None) {
            return false;
        }

        self.apply_drawing_update(update);
        true
    }

    fn reconcile_data_requirements(&mut self, previous: DataRequirements) {
        let current = self.data_requirements();
        if previous.trades != current.trades {
            self.indicators.mark_for_inputs(
                &self.data,
                &[
                    IndicatorInputSource::Trades,
                    IndicatorInputSource::DirectionalVolume,
                ],
                IndicatorRebuildReason::TradeDataModeChanged,
            );
        }
        if previous.trades && !current.trades {
            self.fetch.clear_pending_trade_requests();
        }
        if !current.open_interest {
            self.fetch.clear_open_interest_requests();
        }
    }

    fn sync_tick_drawing_sequence(&mut self) {
        let sequence_id = if self.basis.is_time() {
            None
        } else {
            self.data
                .base_series()
                .map(|series| series.trade_sequence_id())
        };
        if self.drawings.tools.sync_tick_sequence(sequence_id) {
            self.drawings.clear_history();
        }
    }

    pub(super) fn redraw_data_if_due(&mut self, now: Instant) {
        self.render_invalidation.flush_data_redraw_if_due(now);
    }

    fn request_fetch_action(&mut self, allow_trade: bool) -> Option<Vec<FetchSpec>> {
        let horizontal_offset = self.viewport.horizontal_world.offset();
        let visible_points = self.visible_points();
        let active_indicators = self.selected_indicators();
        let data_requirements = self.data_requirements();

        let (required_history_start, required_history_bars) =
            self.indicator_history_requirement(&active_indicators, visible_points);
        let base_ticker = self.data.base_ticker();

        let input = FetchPlanInput {
            basis: self.basis,
            horizontal_offset,
            base_ticker,
            series: self.data.series(),
            active_indicators: &active_indicators,
            indicator_instances: &self.indicators.instances,
            open_interest_required: data_requirements.open_interest,
            trade_data_required: self.data_requirements().trades,
            trade_fetch_enabled: crate::connector::fetcher::is_trade_fetch_enabled(),
            visible_points,
            required_history_start,
            required_history_bars,
            now: UnixMs::now(),
        };

        let mut batches = input.desired_fetch_batches();

        if allow_trade && let Some(range) = input.desired_trade_fetch_range(base_ticker) {
            batches.push(FetchBatch {
                stage: FetchStage::Trades,
                range,
                tickers: vec![base_ticker],
            });
        }

        let window = match self.basis {
            Basis::Time(timeframe) => visible_window(
                self.data.series(),
                horizontal_offset,
                timeframe,
                visible_points,
            ),
            Basis::Tick(_) => None,
        };
        let study_ranges = self.studies.fetch_ranges(
            self.basis,
            &self.data,
            window,
            UnixMs::now(),
            &mut self.fetch,
            &mut batches,
        );

        let mut requests = self.fetch.collect_requests(
            batches
                .into_iter()
                .map(|batch| (batch.range, batch.tickers)),
            self.basis,
        );

        requests.extend(self.fetch.collect_requests(study_ranges, self.basis));

        if requests.is_empty() {
            return None;
        }
        Some(requests)
    }

    fn visible_points(&self) -> u64 {
        let Some(plot_width) = self.viewport.plot_width else {
            return 2;
        };

        let spacing = self.viewport.horizontal_world.spacing();
        let autoscale_plot_width = plot_width + self.autoscale_plot_width_offset();
        let visible_units = (autoscale_plot_width / spacing).max(1.0).ceil() as u64;
        visible_units.saturating_add(1).max(2)
    }

    fn prune_stale_indicator_y_ranges(&mut self) {
        let composition = &self.indicators.composition;
        self.viewport
            .indicator_y_ranges
            .retain(|panel_id| composition.panel(panel_id).is_some());
    }

    fn set_basis(&mut self, basis: Basis) -> Option<ChartAction> {
        self.drawings.finish_pending_edit();
        let previous_basis = self.basis;
        let basis_changed = previous_basis != basis;
        if basis_changed {
            self.drawings.clear_history();
        }
        self.drawings.tools.set_basis(basis);
        if matches!(basis, Basis::Tick(_)) {
            let base_ticker = self.data.base_ticker();
            let secondary_tickers = self
                .data
                .selected_tickers()
                .into_iter()
                .filter(|ticker| *ticker != base_ticker)
                .collect::<Vec<_>>();
            for ticker in secondary_tickers {
                self.remove_ticker(ticker);
            }
        }
        let preserve_raw_trades = matches!(
            (previous_basis, basis),
            (Basis::Time(_), Basis::Time(_)) | (Basis::Tick(_), Basis::Tick(_))
        );
        self.basis = basis;
        self.studies.set_basis(basis);

        self.data.reset_for_basis(self.basis, preserve_raw_trades);
        self.sync_tick_drawing_sequence();
        self.indicators
            .mark_rebuild_pending(IndicatorRebuildReason::BasisChanged);
        if basis_changed {
            self.fetch.reset_for_basis_change();
        } else {
            let selected_tickers = self.data.selected_tickers();
            self.fetch.rebuild_handlers(&selected_tickers);
        }

        // The next chart tick builds the normal fetch stage from the settled basis.
        None
    }

    fn undo_edit(&mut self) -> Option<Action> {
        if self.drawings.undo() {
            self.render_invalidation.invalidate();
        }
        None
    }

    fn redo_edit(&mut self) -> Option<Action> {
        if self.drawings.redo() {
            self.render_invalidation.invalidate();
        }
        None
    }

    fn close_panel_at_index(&mut self, index: usize) -> bool {
        self.drawings.finish_pending_edit();
        let previous_requirements = self.data_requirements();

        let Some(panel) = self.indicators.composition.panels.get(index) else {
            return false;
        };
        if matches!(panel.role, PanelRole::Primary) {
            return false;
        }

        let panel_id = panel.id;
        let indicator_ids = panel
            .layers
            .iter()
            .filter_map(|layer| layer.source.indicator_output())
            .map(|output| output.instance_id)
            .fold(Vec::new(), |mut ids, id| {
                if !ids.contains(&id) {
                    ids.push(id);
                }
                ids
            });
        let before_drawings = self.drawings.tools.edit_state();

        let _ = self.drawings.tools.prune_panel_drawings(panel_id);

        let mut indicator_removed = false;
        for id in indicator_ids {
            indicator_removed |= self.indicators.remove_indicator_instance(id);
        }
        let panel_removed = self.indicators.composition.panel(panel_id).is_some()
            && self.indicators.composition.remove_panel(panel_id);
        let changed = indicator_removed || panel_removed;

        if !changed {
            self.drawings.tools.restore_edit_state(before_drawings);
            return false;
        }

        self.reconcile_data_requirements(previous_requirements);
        let _ = self.viewport.indicator_y_ranges.reset(panel_id);
        self.drawings.clear_history();
        self.request_settings_update(UpdatePolicy::ImmediateReconfigure);
        true
    }

    fn apply_browser_action(&mut self, command: ChartCommand) -> Option<Action> {
        match self.apply_command(command) {
            Some(ChartAction::RequestFetch(fetch)) => Some(Action::RequestFetch(fetch)),
            _ => None,
        }
    }

    fn apply_inspector_effect(&mut self, effect: InspectorEffect) -> Option<Action> {
        match effect {
            InspectorEffect::None => None,
            InspectorEffect::Command(command) => match self.apply_command_inner(command) {
                Some(ChartAction::RequestFetch(fetch)) => Some(Action::RequestFetch(fetch)),
                _ => None,
            },
            InspectorEffect::StudyConfigurator(message) => {
                let command = self.studies.update_configurator(
                    message,
                    self.basis,
                    self.trade_data_enabled(),
                )?;
                let _ = self.apply_command_inner(ChartCommand::Study(command));
                None
            }
            InspectorEffect::OpenUrl(url) => Some(Action::OpenUrl(url)),
        }
    }

    pub(crate) fn apply_fetch_update(&mut self, update: FetchUpdate) -> bool {
        match update {
            FetchUpdate::Failed(req_id) => self.fetch.mark_failed(req_id),
            FetchUpdate::TradeFetchNoData {
                ticker_info,
                req_id,
            } => self.fetch.mark_no_data(ticker_info, req_id),
            FetchUpdate::FetchHandle { req_id, handle } => self.fetch.set_handle(req_id, handle),
        }
    }

    pub(crate) fn apply_data(&mut self, update: DataUpdate<'_>) {
        if let Some(timeframe) = update.kline_timeframe()
            && self.basis != Basis::Time(timeframe)
        {
            self.apply_study_kline_update(&update);
            return;
        }

        let trade_data_enabled = self.trade_data_enabled();
        let effect = update.apply(
            self.basis,
            &mut self.data,
            &mut self.fetch,
            &mut self.indicators,
            trade_data_enabled,
        );
        self.sync_tick_drawing_sequence();
        match effect {
            DataUpdateEffect::Unchanged => {}
            DataUpdateEffect::HistoricalChanged => {
                self.render_invalidation.invalidate();
            }
            DataUpdateEffect::LiveChanged => {
                self.render_invalidation.request_data_redraw();
                self.redraw_data_if_due(Instant::now());
            }
        }
        self.studies
            .apply_data_update(&update, &effect, self.data.base_ticker());
        if !matches!(effect, DataUpdateEffect::Unchanged) {
            self.refresh_indicator_presentation();
        }
    }

    fn apply_study_kline_update(&mut self, update: &DataUpdate<'_>) {
        let applied = match update {
            DataUpdate::LiveKline {
                timeframe,
                ticker_info,
                kline,
            } => self.studies.apply_live_kline(
                self.basis,
                &self.data,
                *timeframe,
                **ticker_info,
                kline,
            ),
            DataUpdate::HistoricalKlines {
                req_id,
                timeframe,
                ticker_info,
                klines,
                is_batches_done,
                has_data,
            } => {
                let is_study_request =
                    req_id.is_some_and(|req_id| self.fetch.is_profile_request(req_id));
                let applied = is_study_request
                    && self.studies.insert_historical_klines(
                        self.basis,
                        &self.data,
                        &mut self.fetch,
                        *req_id,
                        *timeframe,
                        *ticker_info,
                        klines,
                        *is_batches_done,
                        *has_data,
                    );
                if !applied {
                    log::warn!(
                        "Ignoring stale kline fetch for timeframe {:?}; chart basis = {:?}",
                        timeframe,
                        self.basis
                    );
                }
                applied
            }
            _ => false,
        };
        if applied {
            self.render_invalidation.request_data_redraw();
        }
    }

    pub(crate) fn apply_command(&mut self, command: ChartCommand) -> Option<ChartAction> {
        let action = self.apply_command_inner(command);
        self.reconcile_settings_target();
        action
    }

    fn apply_command_inner(&mut self, command: ChartCommand) -> Option<ChartAction> {
        let changes_presentation = command.changes_indicator_presentation();
        let mut policy = command.update_policy();
        let mut request_fetch = false;
        let previous_requirements = self.data_requirements();
        let previous_panels = self
            .indicators
            .composition
            .panels
            .iter()
            .map(|panel| panel.id)
            .collect::<Vec<_>>();
        let mut dirty = false;

        let result = match command {
            ChartCommand::SetBasis(basis) => {
                dirty = true;
                self.set_basis(basis)
            }
            ChartCommand::SetTradeDataEnabled(enabled) => {
                self.apply_trade_data_enabled(enabled, &mut dirty);
                None
            }
            ChartCommand::SetLegendVisibility(visibility) => {
                self.apply_legend_visibility(visibility, &mut dirty);
                None
            }
            ChartCommand::SetCandlestickConfig { config } => {
                let changed = self.options.candlestick != config;
                if changed {
                    self.options.candlestick = config;
                    dirty = true;
                }
                None
            }
            ChartCommand::ResetCandlestickSettings => {
                let default_style = self.options.candlestick.with_default_style();
                let config_changed = self.options.candlestick != default_style;
                let mark_changed = self.indicators.composition.reset_primary_mark();
                let ticker_info = self.data.base_ticker();
                let series_changed = self.data.reset_series_style(ticker_info);
                if config_changed {
                    self.options.candlestick = default_style;
                }
                if config_changed || mark_changed || series_changed {
                    dirty = true;
                }
                None
            }
            ChartCommand::SetCandlestickMark { mark } => {
                let changed = self.indicators.composition.set_primary_mark(mark);
                if changed {
                    dirty = true;
                }
                None
            }
            ChartCommand::SetCandlestickVisible { visible } => {
                let changed = self.options.candlestick.visible != visible;
                if changed {
                    self.options.candlestick = self.options.candlestick.with_visible(visible);
                    dirty = true;
                }
                None
            }
            ChartCommand::SetFootprintVisible { visible } => {
                let changed = self.options.footprint.visible != visible;
                if changed {
                    self.options.footprint = self.options.footprint.with_visible(visible);
                    dirty = true;
                }
                None
            }
            ChartCommand::SetFootprintClusterKind { kind } => {
                let changed = self.options.footprint.cluster_kind != kind;
                if changed {
                    self.options.footprint = self.options.footprint.with_cluster_kind(kind);
                    dirty = true;
                }
                None
            }
            ChartCommand::SetFootprintClusterScaling { scaling } => {
                let changed = self.options.footprint.cluster_scaling != scaling;
                if changed {
                    self.options.footprint = self.options.footprint.with_cluster_scaling(scaling);
                    dirty = true;
                }
                None
            }
            ChartCommand::SetFootprintImbalanceConfig { config } => {
                let changed = self.options.footprint.imbalance != config;
                if changed {
                    self.options.footprint = self.options.footprint.with_imbalance(config);
                    dirty = true;
                }
                None
            }
            ChartCommand::SetFootprintPocConfig { config } => {
                let changed = self.options.footprint.poc != config;
                if changed {
                    self.options.footprint = self.options.footprint.with_poc(config);
                    dirty = true;
                }
                None
            }
            ChartCommand::ResetFootprintSettings => {
                let default_options = self.options.footprint.with_default_options();
                let changed = self.options.footprint != default_options;
                if changed {
                    self.options.footprint = default_options;
                    dirty = true;
                }
                None
            }
            ChartCommand::SetTickMultiplier { multiplier } => {
                if self.data.set_tick_multiplier(multiplier) {
                    dirty = true;
                }
                None
            }
            ChartCommand::AddIndicator(kind) => {
                let change = self.indicators.add_indicator(kind, self.basis, &self.data);
                request_fetch =
                    change.apply(&mut self.indicators, &self.data, &mut dirty, &mut policy);
                None
            }
            ChartCommand::AddIndicatorOnSource {
                kind,
                input_id,
                source,
            } => {
                let change = self
                    .indicators
                    .add_indicator_on_source(kind, input_id, source, self.basis, &self.data);
                request_fetch =
                    change.apply(&mut self.indicators, &self.data, &mut dirty, &mut policy);
                None
            }
            ChartCommand::RemoveIndicator(id) => {
                let change = self.indicators.remove_indicator(id);
                request_fetch =
                    change.apply(&mut self.indicators, &self.data, &mut dirty, &mut policy);
                None
            }
            ChartCommand::Indicator(command) => {
                let change = self.indicators.apply(command);
                request_fetch =
                    change.apply(&mut self.indicators, &self.data, &mut dirty, &mut policy);
                None
            }
            ChartCommand::Study(command) => {
                self.apply_study_command(command, &mut dirty, &mut policy);
                None
            }
            ChartCommand::StudyInteraction(event) => {
                self.studies
                    .apply_interaction(event, self.basis, &self.data);
                dirty = true;
                None
            }
            ChartCommand::ToggleComparisonVisibility { ticker_info } => {
                dirty = self
                    .indicators
                    .composition
                    .toggle_comparison_visibility(ticker_info);
                None
            }
            ChartCommand::SetSeriesColor { ticker_info, color } => {
                let changed = self.data.set_series_color(ticker_info, color);
                if changed {
                    dirty = true;
                }
                None
            }
            ChartCommand::SetSeriesLineWidth { ticker_info, width } => {
                let changed = self.data.set_series_line_width(ticker_info, width);
                if changed {
                    dirty = true;
                }
                None
            }
            ChartCommand::SetSeriesStrokeStyle {
                ticker_info,
                stroke_style,
            } => {
                let changed = self.data.set_series_stroke_style(ticker_info, stroke_style);
                if changed {
                    dirty = true;
                }
                None
            }
            ChartCommand::ResetSeriesSettings { ticker_info } => {
                let changed = self.data.reset_series_style(ticker_info);
                if changed {
                    dirty = true;
                }
                None
            }
            ChartCommand::AddTicker { ticker_info } => {
                let changed = if ticker_info != self.data.base_ticker()
                    && self.data.add_ticker(ticker_info, self.basis)
                {
                    self.fetch.add_ticker(ticker_info);
                    let selected_tickers = self.data.selected_tickers();
                    self.indicators
                        .composition
                        .sync_comparison_sources(self.data.base_ticker(), &selected_tickers);

                    self.indicators.mark_for_candle_ticker(
                        &self.data,
                        ticker_info,
                        IndicatorRebuildReason::IndicatorStructureChanged,
                    );
                    true
                } else {
                    false
                };
                if changed {
                    dirty = true;
                }
                None
            }
            ChartCommand::RemoveTicker { ticker_info } => {
                let changed = self.remove_ticker(ticker_info);
                if changed {
                    dirty = true;
                }
                None
            }
        };

        self.reconcile_data_requirements(previous_requirements);
        if previous_panels
            .iter()
            .any(|panel_id| self.indicators.composition.panel(*panel_id).is_none())
        {
            self.drawings.clear_history();
        }

        if dirty {
            self.request_settings_update(policy);
            if changes_presentation && policy != UpdatePolicy::ImmediateReconfigure {
                self.refresh_indicator_presentation();
            }
        }
        if request_fetch && policy == UpdatePolicy::ImmediateReconfigure {
            return self
                .request_fetch_action(false)
                .map(ChartAction::RequestFetch);
        }
        result
    }

    fn apply_drawing_update(&mut self, update: DrawingUpdate) {
        if update.should_bump() {
            self.render_invalidation.invalidate();
        }
    }

    fn apply_trade_data_enabled(&mut self, enabled: bool, dirty: &mut bool) {
        if !enabled && self.trade_data_setting_lock_reason().is_some() {
            return;
        }
        if !self.options.set_trade_data_requested(enabled, self.basis) {
            return;
        }
        *dirty = true;
    }

    fn apply_legend_visibility(&mut self, visibility: LegendVisibility, dirty: &mut bool) {
        if self.options.legend_visibility == visibility {
            return;
        }
        self.options.legend_visibility = visibility;
        *dirty = true;
    }

    fn apply_study_command(
        &mut self,
        command: StudyCommand,
        dirty: &mut bool,
        policy: &mut UpdatePolicy,
    ) {
        let previous_ids = self
            .studies
            .active()
            .iter()
            .map(|study| study.id())
            .collect::<Vec<_>>();
        let change = self
            .studies
            .apply(command, self.basis, self.trade_data_enabled());
        *policy = match change {
            StudyChange::Unchanged => return,
            StudyChange::Visual => UpdatePolicy::Redraw,
            StudyChange::Reconfigure => UpdatePolicy::DeferredReconfigure,
        };
        *dirty = true;
        if !previous_ids
            .into_iter()
            .eq(self.studies.active().iter().map(|study| study.id()))
        {
            *policy = UpdatePolicy::ImmediateReconfigure;
        }
    }

    fn apply_shared_settings(&mut self, snapshot: &SettingsSnapshot) {
        self.settings_reconfigure_pending = false;
        let previous_requirements = self.data_requirements();
        self.replace_indicator_configuration(
            snapshot.indicator_instance_store(),
            &snapshot.panels,
            &snapshot.splits,
        );

        self.viewport.primary_scale_inverted = snapshot.primary_scale_inverted;

        self.options.trade_data_requested = snapshot.trade_data_enabled;
        self.options.legend_visibility = snapshot.legend_visibility;
        self.options.candlestick = snapshot.candlestick;
        self.options.footprint = snapshot.footprint;

        self.studies.restore_settings(snapshot, self.basis);

        self.reconcile_indicator_layout(true);
        self.sync_tick_drawing_sequence();
        self.prune_stale_indicator_y_ranges();
        self.reconcile_data_requirements(previous_requirements);
    }

    fn apply_persisted_config(&mut self, config: &persisted::Config) {
        let snapshot = config.settings_snapshot();
        self.drawings.clear_history();
        self.apply_shared_settings(&snapshot);

        self.drawing_sidebar_visible = config.drawing_sidebar_visible;
        self.drawings.floating_toolbar =
            toolbar::FloatingToolbarState::from_position(config.floating_toolbar_position);
        let drawings = config
            .drawings
            .iter()
            .filter(|drawing| drawing.object.panels_exist(&self.indicators.composition))
            .cloned()
            .collect();
        self.drawings.tools = DrawingTools::from_parts(config.active_drawing_tool, drawings);
        self.drawings.tools.set_basis(self.basis);

        for series_config in &config.series {
            let Some(index) = self
                .data
                .series()
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
            let Some(ticker_info) = self
                .data
                .series()
                .get(index)
                .map(|series| series.ticker_info)
            else {
                continue;
            };

            let _ = self.data.set_series_color(ticker_info, series_color);
            let _ = self
                .data
                .set_series_line_width(ticker_info, series_config.line_width);
            let _ = self
                .data
                .set_series_stroke_style(ticker_info, series_config.stroke_style);
        }

        self.indicators.allow_partial_rebuilds();
        self.refresh_indicator_presentation();
        self.render_invalidation.invalidate();
    }

    pub(crate) fn apply_settings_snapshot(
        &mut self,
        snapshot: &SettingsSnapshot,
    ) -> Option<Action> {
        self.apply_shared_settings(snapshot);
        let selected_tickers = self.data.selected_tickers();
        self.fetch.rebuild_handlers(&selected_tickers);
        self.refresh_indicator_presentation();
        self.reconcile_settings_target();
        self.render_invalidation.invalidate();

        self.request_fetch_action(false).map(Action::RequestFetch)
    }

    pub(crate) fn trade_data_enabled(&self) -> bool {
        self.data_requirements().trades
    }

    fn trade_data_setting_lock_reason(&self) -> Option<&'static str> {
        if matches!(self.basis, Basis::Tick(_)) {
            return Some("Trade data is required for tick-based charts");
        }

        let indicator_requires_trade_data =
            self.indicators.selected_indicators().iter().any(|id| {
                self.indicators.indicator(*id).is_some_and(|instance| {
                    instance.kind.is_registered()
                        && self.indicator_requires_trade_data(&instance.kind)
                })
            });
        let study_requires_trade_data = self.studies.requires_trade_data();

        (indicator_requires_trade_data || study_requires_trade_data)
            .then_some("Trade data is required by an active indicator or study")
    }

    fn data_requirements(&self) -> DataRequirements {
        let mut requirements = DataRequirements {
            klines: !self.data.selected_tickers().is_empty(),
            trades: self.options.trade_data_requested || matches!(self.basis, Basis::Tick(_)),
            ..DataRequirements::default()
        };

        let indicator_requirements = self
            .indicators
            .data_requirements(self.basis, self.data.base_ticker());
        requirements.klines |= indicator_requirements.klines;
        requirements.directional_volume |= indicator_requirements.directional_volume;
        requirements.trades |= indicator_requirements.trades;
        requirements.open_interest |= indicator_requirements.open_interest;

        if self.options.footprint.visible || self.studies.requires_trade_data() {
            requirements.trades = true;
        }
        requirements
    }

    pub(crate) fn settings_snapshot(&self) -> SettingsSnapshot {
        self.serializable_config().settings_snapshot()
    }

    pub(crate) fn desired_streams(&self) -> Vec<StreamKind> {
        match self.basis {
            Basis::Time(timeframe) => {
                let mut streams = Vec::new();

                if self.data_requirements().trades {
                    streams.push(StreamKind::Trades {
                        ticker_info: self.data.base_ticker(),
                    });
                }
                streams.extend(self.data.selected_tickers().into_iter().map(|ticker_info| {
                    StreamKind::Kline {
                        ticker_info,
                        timeframe,
                    }
                }));

                streams.extend(self.studies.additional_streams(self.basis, &self.data));
                streams
            }
            Basis::Tick(_) => vec![StreamKind::Trades {
                ticker_info: self.data.base_ticker(),
            }],
        }
    }

    pub(crate) fn serializable_config(&self) -> persisted::Config {
        let studies = self.studies.snapshot();
        persisted::Config {
            primary_scale_inverted: self.viewport.primary_scale_inverted,
            trade_data_enabled: self.options.trade_data_requested,
            legend_visibility: self.options.legend_visibility,
            candlestick: self.options.candlestick,
            footprint: self.options.footprint,
            panels: self
                .indicators
                .composition
                .panels
                .iter()
                .map(persisted::panel_config_from_spec)
                .collect(),
            splits: self.indicators.composition.splits.clone(),
            drawings: self.drawings.tools.time_drawings().to_vec(),
            active_drawing_tool: self.drawings.tools.active_tool(),
            floating_toolbar_position: self.drawings.floating_toolbar.position,
            drawing_sidebar_visible: self.drawing_sidebar_visible,
            studies: studies.active,
            study_configs: studies.configs,
            tpo_profile_edits: studies.edits,
            indicator_instances: self.indicators.instances.instances().to_vec(),
            series: self
                .data
                .series()
                .iter()
                .filter(|series| {
                    series.ticker_info != self.data.base_ticker()
                        || series.has_custom_line_style()
                        || !series.color().is_default()
                })
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
        }
    }
}

impl From<KlineWidgetEvent> for Message {
    fn from(event: KlineWidgetEvent) -> Self {
        Self::Chart(event)
    }
}

struct RenderInvalidation {
    revision: u64,
    last_invalidated_at: Instant,
    data_redraw_pending: bool,
}

impl RenderInvalidation {
    fn new() -> Self {
        Self {
            revision: 0,
            last_invalidated_at: Instant::now(),
            data_redraw_pending: false,
        }
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn invalidate(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.last_invalidated_at = Instant::now();
        self.data_redraw_pending = false;
    }

    fn request_data_redraw(&mut self) {
        self.data_redraw_pending = true;
    }

    fn flush_data_redraw_if_due(&mut self, now: Instant) {
        if self.data_redraw_pending
            && now.saturating_duration_since(self.last_invalidated_at)
                >= MIN_FULL_CACHE_REFRESH_INTERVAL
        {
            self.invalidate();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct DataRequirements {
    klines: bool,
    directional_volume: bool,
    trades: bool,
    open_interest: bool,
}

pub enum ChartCommand {
    SetBasis(Basis),
    SetTradeDataEnabled(bool),
    SetLegendVisibility(LegendVisibility),

    SetCandlestickConfig {
        config: CandlestickConfig,
    },
    ResetCandlestickSettings,
    SetCandlestickMark {
        mark: MarkKind,
    },
    SetCandlestickVisible {
        visible: bool,
    },

    SetFootprintVisible {
        visible: bool,
    },
    SetFootprintClusterKind {
        kind: ClusterKind,
    },
    SetFootprintClusterScaling {
        scaling: ClusterScaling,
    },
    SetFootprintImbalanceConfig {
        config: FootprintImbalanceConfig,
    },
    SetFootprintPocConfig {
        config: FootprintPocConfig,
    },
    ResetFootprintSettings,
    SetTickMultiplier {
        multiplier: TickMultiplier,
    },

    AddIndicator(IndicatorId),
    AddIndicatorOnSource {
        kind: IndicatorId,
        input_id: String,
        source: IndicatorOutputRef,
    },
    RemoveIndicator(IndicatorInstanceId),
    Indicator(IndicatorCommand),
    Study(StudyCommand),
    StudyInteraction(StudyObjectEvent),

    ToggleComparisonVisibility {
        ticker_info: TickerInfo,
    },
    SetSeriesColor {
        ticker_info: TickerInfo,
        color: SeriesColor,
    },
    SetSeriesLineWidth {
        ticker_info: TickerInfo,
        width: u8,
    },
    SetSeriesStrokeStyle {
        ticker_info: TickerInfo,
        stroke_style: LineStrokeStyle,
    },
    ResetSeriesSettings {
        ticker_info: TickerInfo,
    },
    AddTicker {
        ticker_info: TickerInfo,
    },
    RemoveTicker {
        ticker_info: TickerInfo,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UpdatePolicy {
    Redraw,
    DeferredReconfigure,
    ImmediateReconfigure,
}

impl ChartCommand {
    fn update_policy(&self) -> UpdatePolicy {
        match self {
            Self::SetLegendVisibility(_)
            | Self::SetCandlestickConfig { .. }
            | Self::ResetCandlestickSettings
            | Self::SetCandlestickMark { .. }
            | Self::SetCandlestickVisible { .. }
            | Self::SetFootprintClusterKind { .. }
            | Self::SetFootprintClusterScaling { .. }
            | Self::SetFootprintImbalanceConfig { .. }
            | Self::SetFootprintPocConfig { .. }
            | Self::ResetFootprintSettings
            | Self::ToggleComparisonVisibility { .. }
            | Self::SetSeriesColor { .. }
            | Self::SetSeriesLineWidth { .. }
            | Self::SetSeriesStrokeStyle { .. }
            | Self::ResetSeriesSettings { .. }
            | Self::StudyInteraction(_) => UpdatePolicy::Redraw,
            Self::SetBasis(_)
            | Self::SetTradeDataEnabled(_)
            | Self::SetTickMultiplier { .. }
            | Self::AddIndicator(_)
            | Self::AddIndicatorOnSource { .. }
            | Self::RemoveIndicator(_)
            | Self::AddTicker { .. }
            | Self::RemoveTicker { .. }
            | Self::Indicator(IndicatorCommand::InputChanged(
                IndicatorInputCommand::SourceBinding { .. }
                | IndicatorInputCommand::BaseSource { .. },
            )) => UpdatePolicy::ImmediateReconfigure,
            Self::SetFootprintVisible { .. } | Self::Indicator(_) | Self::Study(_) => {
                UpdatePolicy::DeferredReconfigure
            }
        }
    }

    fn changes_indicator_presentation(&self) -> bool {
        match self {
            Self::SetBasis(_)
            | Self::SetTradeDataEnabled(_)
            | Self::ResetCandlestickSettings
            | Self::SetCandlestickMark { .. }
            | Self::SetFootprintVisible { .. }
            | Self::SetTickMultiplier { .. }
            | Self::AddIndicator(_)
            | Self::AddIndicatorOnSource { .. }
            | Self::RemoveIndicator(_)
            | Self::Indicator(_)
            | Self::Study(_)
            | Self::AddTicker { .. }
            | Self::RemoveTicker { .. } => true,
            Self::SetLegendVisibility(_)
            | Self::SetCandlestickConfig { .. }
            | Self::SetCandlestickVisible { .. }
            | Self::SetFootprintClusterKind { .. }
            | Self::SetFootprintClusterScaling { .. }
            | Self::SetFootprintImbalanceConfig { .. }
            | Self::SetFootprintPocConfig { .. }
            | Self::ResetFootprintSettings
            | Self::StudyInteraction(_)
            | Self::ToggleComparisonVisibility { .. }
            | Self::SetSeriesColor { .. }
            | Self::SetSeriesLineWidth { .. }
            | Self::SetSeriesStrokeStyle { .. }
            | Self::ResetSeriesSettings { .. } => false,
        }
    }

    fn indicator_input(command: IndicatorInputCommand) -> Self {
        Self::Indicator(IndicatorCommand::InputChanged(command))
    }

    fn indicator_style(command: IndicatorStyleCommand) -> Self {
        Self::Indicator(IndicatorCommand::StyleChanged(command))
    }
}
