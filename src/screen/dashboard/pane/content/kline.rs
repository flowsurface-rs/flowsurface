mod drawing;
mod fetch;
mod history;
mod indicator_browser;
mod layout;
mod presentation;
mod settings;
mod studies;
mod toolbar;
mod updates;
mod viewport;

pub use data::chart::kline::IndicatorId;
use data::chart::kline::LegendVisibility;
pub use data::chart::kline::series::KlineSeries;
pub use fetch::FetchUpdate;
pub use layout::{IndicatorCommand, IndicatorInputCommand, IndicatorStyleCommand};
pub use studies::StudyCommand;
pub use updates::DataUpdate;

use data::chart::kline::KlineData;
use data::chart::kline::indicators::IndicatorHost;
use drawing::{DrawingEditState, DrawingMessage, DrawingState, DrawingTools, DrawingUpdate};
use fetch::FetchCoordinator;
use history::DrawingHistory;
use layout::{IndicatorChange, IndicatorRebuildReason, IndicatorState};
use presentation::IndicatorPresentationResolver;
use settings::{ChartOptions, InspectorEffect, InspectorState, SettingsContext};
use studies::StudyState;
use updates::DataUpdateEffect;
use viewport::ViewportState;

use crate::connector::fetcher::FetchSpec;
use crate::screen::dashboard::pane::ChartAction;
use crate::style;
use crate::theme::ThemeAccents;
use crate::widget::chart::kline::{
    IndicatorPresentationContext, KlineWidget, KlineWidgetEvent, MIN_FULL_CACHE_REFRESH_INTERVAL,
    PanelInteraction,
};
use crate::widget::inspector::{InspectorBody, InspectorFooter};
use crate::widget::series_editor::{self, SeriesStyleEntry};
use crate::widget::study_configurator as study;
use crate::window::PixelRatio;
use data::chart::kline::composition::{
    DEFAULT_MIN_PANEL_RATIO, MarkKind, PanelRole, PanelScaleMode,
};

use data::chart::Basis;
use data::chart::kline::{
    self as persisted, CandlestickConfig, ChartSettings, DEFAULT_HORIZONTAL_SPACING_PX, FetchBatch,
    FetchPlanInput, FetchStage, FootprintImbalanceConfig, FootprintPocConfig, HorizontalWorld,
    IndicatorInputSource, IndicatorInstanceId, IndicatorInstanceStore, IndicatorLayout,
    IndicatorOutputRef, KlineStudy, LineStrokeStyle, SeriesColor, SettingsSnapshot, visible_window,
};
use data::chart::kline_data::{ClusterKind, ClusterScaling};
use exchange::adapter::StreamKind;
use exchange::{TickMultiplier, TickerInfo, UnixMs};
use indicator_engine::IndicatorFrame;

use std::sync::Arc;
use std::time::Instant;
use tokio::sync::oneshot;

const DEFAULT_HORIZONTAL_OFFSET_UNITS: f32 = 8.0;

const DEFAULT_FOOTPRINT_TICK_MULTIPLIER: TickMultiplier = TickMultiplier(25);

const FLOATING_TOOLBAR_MARGIN: f32 = 8.0;
const FLOATING_TOOLBAR_HANDLE_WIDTH: f32 = 28.0;
const FLOATING_TOOLBAR_HEIGHT: f32 = 36.0;
const SIDEBAR_WIDTH: f32 = 228.0;
const SIDEBAR_RETURN_BUTTON_WIDTH: f32 = 32.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Sidebar {
    #[default]
    Drawings,
    Settings,
}

struct IndicatorRebuildState {
    job_id: u64,
    frame_revision: u64,
    dirty_epochs: Vec<(IndicatorInstanceId, u64)>,
    dirty_ids: Vec<IndicatorInstanceId>,
    replay_ids: Vec<IndicatorInstanceId>,
    active_ids: Vec<IndicatorInstanceId>,
    frames: Arc<[IndicatorFrame]>,
    result: oneshot::Receiver<Result<IndicatorHost, String>>,
}

pub enum Action {
    RemoveSeries(TickerInfo),
    RequestFetch(Vec<FetchSpec>),
    OpenUrl(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KlineDataRequirements {
    pub klines: bool,
    pub directional_volume: bool,
    pub trades: bool,
    pub open_interest: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    Chart(KlineWidgetEvent),
    IndicatorRebuildCompleted { job_id: u64 },
    Drawing(drawing::Event),
    ToggleDrawingSidebar,
    ShowDrawingsPage,
    IndicatorBrowser(indicator_browser::Message),
    Settings(settings::Message),
    SeriesEditor(series_editor::Message),
}

pub struct KlineChart {
    basis: Basis,
    viewport: ViewportState,
    options: ChartOptions,
    data: KlineData,
    indicators: IndicatorState,
    fetch: FetchCoordinator,
    history: DrawingHistory,
    drawings: DrawingState,
    studies: StudyState,
    inspector: InspectorState,
    indicator_browser: indicator_browser::State,
    sidebar: Sidebar,
    drawing_sidebar_visible: bool,
    last_tick: Instant,
    cache_rev: u64,
    last_revision_at: Instant,
    data_redraw_pending: bool,
    frame_revision: u64,
    next_indicator_rebuild_job: u64,
    indicator_rebuild: Option<IndicatorRebuildState>,
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
    SetFootprintTickMultiplier {
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

impl ChartCommand {
    fn indicator_input(command: IndicatorInputCommand) -> Self {
        Self::Indicator(IndicatorCommand::InputChanged(command))
    }

    fn indicator_style(command: IndicatorStyleCommand) -> Self {
        Self::Indicator(IndicatorCommand::StyleChanged(command))
    }

    fn reset_indicator_settings(id: IndicatorInstanceId) -> Self {
        Self::Indicator(IndicatorCommand::ResetSettings(id))
    }
}

pub enum CommandResult {
    None,
    Changed(bool),
    Action(Option<crate::screen::dashboard::pane::ChartAction>),
}

impl KlineChart {
    pub fn new_with_tickers(basis: Basis, tickers: &[TickerInfo]) -> Self {
        let base_ticker = tickers
            .first()
            .copied()
            .expect("Kline chart requires a base ticker");
        let footprint_tick_multiplier = DEFAULT_FOOTPRINT_TICK_MULTIPLIER;
        let indicators = IndicatorState::new(basis);

        let mut chart = Self {
            basis,
            viewport: ViewportState::default(),
            options: ChartOptions::default(),
            data: KlineData::new(base_ticker, footprint_tick_multiplier),
            indicators,
            fetch: FetchCoordinator::new(&[]),
            history: DrawingHistory::default(),
            drawings: DrawingState::default(),
            studies: StudyState::default(),
            inspector: InspectorState::default(),
            indicator_browser: indicator_browser::State::default(),
            sidebar: Sidebar::Drawings,
            drawing_sidebar_visible: true,
            last_tick: Instant::now(),
            cache_rev: 0,
            last_revision_at: Instant::now(),
            data_redraw_pending: false,
            frame_revision: 0,
            next_indicator_rebuild_job: 0,
            indicator_rebuild: None,
        };

        let _ = chart.data.add_ticker(base_ticker, chart.basis);
        for ticker in tickers.iter().copied() {
            if ticker != base_ticker {
                let _ = chart.data.add_ticker(ticker, chart.basis);
            }
        }

        chart.fetch.rebuild_handlers(chart.data.selected_tickers());
        chart.data.configure_series_for_basis(chart.basis);
        chart.drawings.tools.set_basis(basis);
        chart.sync_tick_drawing_sequence();
        chart
            .indicators
            .layout_mut()
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
        chart.viewport.reset_autoscale();
        chart
    }

    pub fn update(
        &mut self,
        message: Message,
        chart_settings: &mut ChartSettings,
    ) -> Option<Action> {
        match message {
            Message::IndicatorRebuildCompleted { job_id } => {
                self.complete_indicator_rebuild(job_id);
            }
            Message::ToggleDrawingSidebar => {
                if self.sidebar != Sidebar::Drawings {
                    self.show_sidebar(Sidebar::Drawings);
                } else {
                    self.drawing_sidebar_visible = !self.drawing_sidebar_visible;
                    self.bump_rev();
                }
            }
            Message::ShowDrawingsPage => {
                self.show_sidebar(Sidebar::Drawings);
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
            Message::Settings(message) => {
                let effect = self
                    .with_settings_context(|inspector, context| inspector.update(message, context));
                let action = self.apply_inspector_effect(effect);
                self.reconcile_local_sidebar();
                return action;
            }
            Message::SeriesEditor(message) => {
                let effect = self.with_settings_context(|inspector, context| {
                    inspector.update(settings::Message::SeriesEditor(message), context)
                });
                let action = self.apply_inspector_effect(effect);
                self.reconcile_local_sidebar();
                return action;
            }
            Message::Drawing(message) => {
                let before = self.drawings.tools.edit_state();
                let update = self.drawings.update(message, &mut chart_settings.drawing);
                self.apply_drawing_update(update, before);
            }
            Message::Chart(event) => {
                match event {
                    KlineWidgetEvent::UndoRequested => return self.undo_edit(),
                    KlineWidgetEvent::RedoRequested => return self.redo_edit(),
                    KlineWidgetEvent::EscapePressed => {
                        let _ = self.handle_escape();
                    }
                    KlineWidgetEvent::PrimaryScaleInversionRequested => {
                        self.finish_pending_drawing_edit();
                        self.viewport.primary_scale_inverted =
                            !self.viewport.primary_scale_inverted;
                        self.bump_rev();
                    }
                    KlineWidgetEvent::Drawing(event) => {
                        let before = self.drawings.tools.edit_state();
                        let update = self
                            .drawings
                            .tools
                            .handle_widget_drawing_event(event, &chart_settings.drawing);
                        self.apply_drawing_update(update, before);
                    }
                    KlineWidgetEvent::PanelInteraction(interaction) => match interaction {
                        PanelInteraction::YViewportChanged { panel_id, range } => {
                            if let Some(primary_id) =
                                self.indicators.composition().primary_panel_id()
                                && primary_id == panel_id
                            {
                                if matches!(
                                    self.indicators
                                        .composition()
                                        .panel_effective_scale_mode(primary_id),
                                    Some(PanelScaleMode::PercentFromBase)
                                ) {
                                    return None;
                                }

                                if self.viewport.primary.set_range(range) {
                                    self.bump_rev();
                                }
                            } else if self.viewport.indicator_y_ranges.set(panel_id, range) {
                                self.bump_rev();
                            }
                        }
                        PanelInteraction::YViewportReset { panel_id } => {
                            if let Some(primary_id) =
                                self.indicators.composition().primary_panel_id()
                                && primary_id == panel_id
                            {
                                if matches!(
                                    self.indicators
                                        .composition()
                                        .panel_effective_scale_mode(primary_id),
                                    Some(PanelScaleMode::PercentFromBase)
                                ) {
                                    return None;
                                }

                                if self.viewport.primary.reset() {
                                    self.bump_rev();
                                }
                            } else if self.viewport.indicator_y_ranges.reset(panel_id) {
                                self.bump_rev();
                            }
                        }
                        PanelInteraction::SplitChanged { index, split } => {
                            self.finish_pending_drawing_edit();
                            if self.indicators.composition_mut().set_split(
                                index,
                                split,
                                DEFAULT_MIN_PANEL_RATIO,
                            ) {
                                self.history.clear();
                                self.bump_rev();
                            }
                        }
                        PanelInteraction::MoveUp { index } => {
                            self.finish_pending_drawing_edit();
                            if index > 0
                                && self
                                    .indicators
                                    .composition_mut()
                                    .move_panel(index, index - 1)
                            {
                                self.bump_rev();
                            }
                        }
                        PanelInteraction::MoveDown { index } => {
                            self.finish_pending_drawing_edit();
                            let target = index.saturating_add(1);
                            if target < self.indicators.composition().panels.len()
                                && self.indicators.composition_mut().move_panel(index, target)
                            {
                                self.bump_rev();
                            }
                        }
                        PanelInteraction::Close { index } => {
                            self.close_panel_at_index(index);
                        }
                    },
                    KlineWidgetEvent::HorizontalWorldChanged(world) => {
                        self.viewport.horizontal_world = world;
                        self.bump_rev();
                    }
                    KlineWidgetEvent::HorizontalOffsetChanged(offset) => {
                        self.viewport.horizontal_world =
                            self.viewport.horizontal_world.with_offset(offset);
                        self.bump_rev();
                    }
                    KlineWidgetEvent::PlotWidthChanged(plot_width) => {
                        if self.viewport.plot_width == Some(plot_width) {
                            return None;
                        }

                        self.viewport.plot_width = Some(plot_width);
                        self.bump_rev();
                    }
                    KlineWidgetEvent::PrimaryAutoscaleDisabled => {
                        if self.viewport.primary.disable() {
                            self.bump_rev();
                        }
                    }
                    KlineWidgetEvent::PrimaryAutoscaleToggled => {
                        let primary_panel_id = self.indicators.composition().primary_panel_id()?;

                        let effective_scale = self
                            .indicators
                            .composition()
                            .panel_effective_scale_mode(primary_panel_id)
                            .unwrap_or(PanelScaleMode::Absolute);

                        if !matches!(effective_scale, PanelScaleMode::PercentFromBase)
                            && self.viewport.primary.toggle()
                        {
                            self.bump_rev();
                        }
                    }
                    KlineWidgetEvent::PrimaryScaleModeCycleRequested => {
                        self.finish_pending_drawing_edit();
                        let primary_panel_id = self.indicators.composition().primary_panel_id()?;
                        let panel = self.indicators.composition().panel(primary_panel_id)?;

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
                            .composition_mut()
                            .set_panel_preferred_scale(primary_panel_id, next_scale)
                        {
                            self.viewport.primary.reset();

                            self.bump_rev();
                        }
                    }
                    KlineWidgetEvent::TickerSettings(ticker) => {
                        self.open_series_editor_for(ticker);

                        if ticker == self.data.base_ticker() {
                            self.inspector.open_candlestick();
                        } else {
                            self.inspector.open_series(ticker);
                        }
                        self.show_sidebar(Sidebar::Settings);
                    }
                    KlineWidgetEvent::FootprintSettings => {
                        self.inspector.open_footprint();
                        self.show_sidebar(Sidebar::Settings);
                    }
                    KlineWidgetEvent::CandlesticksVisibilityToggled => {
                        let _ = self.apply_command(ChartCommand::SetCandlestickVisible {
                            visible: !self.options.candlestick.visible,
                        });
                    }
                    KlineWidgetEvent::FootprintsVisibilityToggled => {
                        let _ = self.apply_command(ChartCommand::SetFootprintVisible {
                            visible: !self.options.footprint.visible,
                        });
                    }
                    KlineWidgetEvent::IndicatorSettings(id) => {
                        if let Some(indicator) = self.indicator_kind(id) {
                            self.inspector.open_indicator(id, indicator);
                            self.show_sidebar(Sidebar::Settings);
                        }
                    }
                    KlineWidgetEvent::IndicatorVisibilityToggled(id) => {
                        let _ = self.apply_command(ChartCommand::indicator_style(
                            IndicatorStyleCommand::ToggleVisibility(id),
                        ));
                    }
                    KlineWidgetEvent::StudySettings(id) => {
                        if self.studies.study(id).is_some() {
                            self.inspector.open_study(id);
                            self.show_sidebar(Sidebar::Settings);
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
                        self.bump_rev();
                    }
                    KlineWidgetEvent::DeletePressed => {
                        let before = self.drawings.tools.edit_state();
                        let update = self.drawings.tools.update(
                            DrawingMessage::DeleteSelectedDrawing,
                            &mut chart_settings.drawing,
                        );
                        self.apply_drawing_update(update, before);
                    }
                }
                self.reconcile_local_sidebar();
            }
        }
        None
    }

    pub fn invalidate(&mut self, now: Option<Instant>) -> Option<ChartAction> {
        if let Some(ts) = now {
            self.last_tick = ts;

            let action = self
                .request_fetch_action(true)
                .map(ChartAction::RequestFetch);
            self.recalculate_indicators();
            self.preview_clean_indicators();
            self.redraw_data_if_due(ts);
            return action;
        }

        self.bump_rev();
        None
    }

    pub(crate) fn rebuild_indicators(&mut self) -> Option<iced::Task<Message>> {
        if let Some(job_id) = self
            .indicator_rebuild
            .as_ref()
            .map(|pending| pending.job_id)
        {
            self.complete_indicator_rebuild(job_id);
        }
        if self.indicator_rebuild.is_some() {
            return None;
        }

        let plan = self.indicators.full_rebuild_plan(&self.data, &self.fetch)?;
        let active_ids = self.indicators.selected_indicators();
        let trade_data_enabled = self.trade_data_enabled();
        let frames = self.data.committed_indicator_frames(trade_data_enabled);
        let instances = self.indicators.instances().clone();
        let frame_revision = self.frame_revision;
        let job_id = self.next_indicator_rebuild_job.wrapping_add(1);
        self.next_indicator_rebuild_job = job_id;

        let (result_tx, result_rx) = oneshot::channel();
        let worker_replay_ids = plan.replay_ids.clone();
        let worker_frames = Arc::clone(&frames);
        let worker = async move {
            let result = tokio::task::spawn_blocking(move || {
                let mut host = IndicatorHost::default();
                host.recalculate(&worker_replay_ids, &instances, &worker_frames);
                host
            })
            .await
            .map_err(|error| format!("indicator rebuild worker failed: {error}"));
            let _ = result_tx.send(result);
        };

        self.indicator_rebuild = Some(IndicatorRebuildState {
            job_id,
            frame_revision,
            dirty_epochs: plan.dirty_epochs,
            dirty_ids: plan.dirty_ids,
            replay_ids: plan.replay_ids,
            active_ids,
            frames,
            result: result_rx,
        });
        self.bump_rev();
        Some(iced::Task::perform(worker, move |_| {
            Message::IndicatorRebuildCompleted { job_id }
        }))
    }

    fn complete_indicator_rebuild(&mut self, job_id: u64) {
        let Some(pending) = self.indicator_rebuild.as_mut() else {
            return;
        };
        if pending.job_id != job_id {
            return;
        }

        let result = match pending.result.try_recv() {
            Ok(result) => result,
            Err(oneshot::error::TryRecvError::Empty) => return,
            Err(oneshot::error::TryRecvError::Closed) => {
                self.indicator_rebuild = None;
                log::error!("indicator rebuild result channel closed for job {job_id}");
                return;
            }
        };
        let pending = self
            .indicator_rebuild
            .take()
            .expect("indicator rebuild state exists");

        let host = match result {
            Ok(host) => host,
            Err(error) => {
                log::error!("indicator rebuild job {job_id} failed: {error}");
                return;
            }
        };

        if !(pending.frame_revision == self.frame_revision
            && self.indicators.dirty_epochs_match(&pending.dirty_epochs)
            && pending.active_ids == self.indicators.selected_indicators())
        {
            return;
        }

        let trade_data_enabled = self.trade_data_enabled();
        self.indicators.accept_full_rebuild(
            &mut self.data,
            host,
            &pending.frames,
            &pending.dirty_ids,
            &pending.replay_ids,
            trade_data_enabled,
        );
        self.data
            .preview_indicator_open_for(&pending.replay_ids, trade_data_enabled);
        self.bump_rev();
    }

    fn bump_rev(&mut self) {
        self.sync_tick_drawing_sequence();
        self.prune_stale_indicator_y_ranges();
        self.cache_rev = self.cache_rev.wrapping_add(1);
        self.last_revision_at = Instant::now();
        self.data_redraw_pending = false;
    }

    pub(crate) fn view(
        &self,
        timezone: data::UserTimezone,
        pixel_ratio: PixelRatio,
        theme_accents: ThemeAccents,
    ) -> iced::Element<'_, Message> {
        let chart: iced::Element<'_, Message> =
            if self.data.series().iter().all(KlineSeries::is_empty) {
                iced::widget::center(iced::widget::text("Waiting for data...").size(16)).into()
            } else {
                let indicator_presentation = self
                    .indicator_presentation_context()
                    .with_rebuild_in_flight_ids(
                        self.indicator_rebuild
                            .as_ref()
                            .map_or_else(Vec::new, |pending| pending.replay_ids.clone()),
                    )
                    .with_pending_input_ids(
                        self.indicators.pending_input_ids(&self.data, &self.fetch),
                    );
                let chart = KlineWidget::new(
                    self.data.series(),
                    self.data.indicator_host(),
                    self.basis,
                    self.indicators.composition(),
                    self.indicators.instances(),
                    theme_accents,
                )
                .with_candlestick_config(self.options.candlestick)
                .with_footprint_config(self.options.footprint)
                .with_trade_data_enabled(self.trade_data_enabled())
                .with_legend_visibility(self.legend_visibility());

                let chart: iced::Element<_> = chart
                    .with_studies(self.studies.active())
                    .with_studies_legend_expanded(self.studies.legend_expanded())
                    .with_coordinate_state(self.viewport.horizontal_world, pixel_ratio)
                    .with_primary_viewport(self.viewport.primary)
                    .with_primary_scale_inverted(self.viewport.primary_scale_inverted)
                    .with_known_plot_width(self.viewport.plot_width)
                    .with_autoscale_plot_width_offset(self.autoscale_plot_width_offset())
                    .with_indicator_ranges(self.viewport.indicator_y_ranges.as_slice())
                    .with_indicator_presentation(indicator_presentation)
                    .with_drawings(self.drawings.tools.snapshot())
                    .with_timezone(timezone)
                    .with_version(self.cache_rev)
                    .with_escape_enabled(self.can_handle_escape())
                    .into();

                iced::widget::container(chart.map(Message::Chart))
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

        let is_sidebar_visible = self.sidebar != Sidebar::Drawings || self.drawing_sidebar_visible;
        if !is_sidebar_visible {
            return chart;
        }

        let sidebar = match self.sidebar {
            Sidebar::Drawings => self
                .drawings
                .sidebar_view(self.indicators.composition())
                .map(Message::Drawing),
            Sidebar::Settings => {
                let Some((title, settings)) = self.contextual_settings_content(theme_accents)
                else {
                    return chart;
                };

                let header = sidebar_header(title);

                let footer = {
                    let context = self.settings_view_context();
                    self.inspector.settings().footer_view(context)
                };

                sidebar_panel(
                    header,
                    InspectorBody::view(settings),
                    footer.map(InspectorFooter::view),
                )
            }
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

    pub(crate) fn pane_settings_view(&self) -> iced::Element<'static, Message> {
        settings::data_settings_content(
            self.trade_data_enabled(),
            self.legend_visibility(),
            self.basis(),
            self.trade_data_setting_locked(),
        )
        .map(Message::Settings)
    }

    fn autoscale_plot_width_offset(&self) -> f32 {
        let sidebar_width = match self.sidebar {
            Sidebar::Drawings if self.drawing_sidebar_visible => {
                self.drawings.tools.sidebar_width()
            }
            Sidebar::Drawings => return 0.0,
            Sidebar::Settings => SIDEBAR_WIDTH,
        };

        (sidebar_width - DrawingTools::compact_sidebar_width()).max(0.0)
    }

    fn contextual_settings_content(
        &self,
        theme_accents: ThemeAccents,
    ) -> Option<(String, iced::Element<'_, Message>)> {
        let context = self.settings_view_context();
        self.inspector
            .settings()
            .contextual_content(context, theme_accents)
    }

    pub(crate) fn indicator_browser_view(&self) -> iced::Element<'_, Message> {
        indicator_browser::view(self, &self.indicator_browser)
    }

    pub(crate) fn indicator_browser_focus_id(&self) -> iced::widget::Id {
        self.indicator_browser.search_box_id()
    }

    pub(super) fn preserve_indicator_browser_state_from(&mut self, source: &Self) {
        self.indicator_browser = source.indicator_browser.clone();
    }

    fn can_handle_escape(&self) -> bool {
        self.inspector.is_confirming_reset()
            || self.sidebar != Sidebar::Drawings
            || self.drawings.tools.can_handle_escape()
    }

    fn show_sidebar(&mut self, page: Sidebar) {
        if self.sidebar == page {
            return;
        }

        self.sidebar = page;
        self.bump_rev();
    }

    fn reconcile_local_sidebar(&mut self) {
        let settings_sidebar_open = self.sidebar == Sidebar::Settings;
        let settings_target_missing = self.with_settings_context(|inspector, context| {
            inspector.reconcile_missing_target(context);
            settings_sidebar_open && !inspector.has_contextual_target(context)
        });
        if settings_target_missing {
            self.show_sidebar(Sidebar::Drawings);
        }
    }

    fn apply_browser_action(&mut self, command: ChartCommand) -> Option<Action> {
        match self.apply_command(command) {
            CommandResult::Action(Some(ChartAction::RequestFetch(fetch))) => {
                Some(Action::RequestFetch(fetch))
            }
            _ => None,
        }
    }

    fn apply_inspector_effect(&mut self, effect: InspectorEffect) -> Option<Action> {
        match effect {
            InspectorEffect::None => None,
            InspectorEffect::Command(command) => match self.apply_settings_action(command) {
                CommandResult::Action(Some(ChartAction::RequestFetch(fetch))) => {
                    Some(Action::RequestFetch(fetch))
                }
                _ => None,
            },
            InspectorEffect::StudyConfigurator(message) => {
                self.update_study_configurator(message);
                None
            }
            InspectorEffect::OpenUrl(url) => Some(Action::OpenUrl(url)),
        }
    }

    fn settings_view_context(&self) -> SettingsContext<'_> {
        SettingsContext {
            basis: self.basis,
            options: &self.options,
            data: &self.data,
            indicators: &self.indicators,
            studies: &self.studies,
            series_editor: Some(self.inspector.series_editor()),
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

    fn open_series_editor_for(&mut self, ticker: TickerInfo) {
        let series_style_entry = self
            .data
            .series()
            .iter()
            .find(|series| series.ticker_info == ticker)
            .map(|series| SeriesStyleEntry {
                ticker: series.ticker_info,
                color: series.color(),
                line_width: series.line_width(),
                stroke_style: series.stroke_style(),
            });

        let Some(series) = series_style_entry else {
            self.inspector.series_editor_mut().close();
            return;
        };

        self.inspector.series_editor_mut().open_for(
            series.ticker,
            series.color,
            series.line_width,
            series.stroke_style,
        );
    }

    fn refresh_open_series_editor(&mut self, ticker: TickerInfo) {
        if self.inspector.series_editor().is_open_for(ticker) {
            self.open_series_editor_for(ticker);
        }
    }

    pub fn update_study_configurator(&mut self, message: study::Message<KlineStudy>) {
        let Some(command) =
            self.studies
                .update_configurator(message, self.basis, self.trade_data_enabled())
        else {
            return;
        };
        let _ = self.apply_settings_action(ChartCommand::Study(command));
    }

    fn handle_escape(&mut self) -> bool {
        if self.inspector.dismiss_reset_confirmation() {
            return true;
        }

        if self.sidebar != Sidebar::Drawings {
            self.show_sidebar(Sidebar::Drawings);
            return true;
        }

        let before = self.drawings.tools.edit_state();
        let update = self.drawings.tools.handle_escape();
        if matches!(update, DrawingUpdate::None) {
            return false;
        }

        self.apply_drawing_update(update, before);
        true
    }

    pub(crate) fn go_back(&mut self) -> bool {
        if self.inspector.dismiss_reset_confirmation() {
            return true;
        }

        if self.sidebar != Sidebar::Drawings {
            self.show_sidebar(Sidebar::Drawings);
            return true;
        }

        false
    }

    fn mark_indicators_with_dependents(
        &mut self,
        roots: &[IndicatorInstanceId],
        reason: IndicatorRebuildReason,
    ) {
        self.indicators
            .mark_with_dependents(&self.data, roots, reason);
    }

    fn reconcile_data_requirements(&mut self, previous: KlineDataRequirements) {
        let current = self.data_requirements();
        if previous.trades != current.trades {
            self.indicators.mark_for_inputs(
                &self.data,
                &[
                    IndicatorInputSource::Trades,
                    IndicatorInputSource::DirectionalVolume,
                ],
                IndicatorRebuildReason::InputConfigurationChanged,
            );
        }
        if !current.open_interest {
            self.fetch.clear_open_interest_requests();
        }
    }

    fn mark_indicators_for_candle_ticker(
        &mut self,
        ticker: TickerInfo,
        reason: IndicatorRebuildReason,
    ) {
        self.indicators
            .mark_for_candle_ticker(&self.data, ticker, reason);
    }

    fn recalculate_indicators(&mut self) -> bool {
        let trade_data_enabled = self.trade_data_enabled();
        let changed = self
            .indicators
            .recalculate(&mut self.data, &self.fetch, trade_data_enabled);
        if changed {
            self.bump_rev();
        }
        changed
    }

    fn preview_clean_indicators(&mut self) -> bool {
        let trade_data_enabled = self.trade_data_enabled();
        let changed = self
            .indicators
            .preview_clean(&mut self.data, trade_data_enabled);
        if changed {
            self.bump_rev();
        }
        changed
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
            self.history.clear();
        }
    }

    pub(super) fn redraw_data_if_due(&mut self, now: Instant) {
        if self.data_redraw_pending
            && now.saturating_duration_since(self.last_revision_at)
                >= MIN_FULL_CACHE_REFRESH_INTERVAL
        {
            self.bump_rev();
        }
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
            indicator_instances: self.indicators.instances(),
            open_interest_required: data_requirements.open_interest,
            trade_data_required: self.trade_data_required(),
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

        let requests = self.fetch.collect_requests(
            batches
                .into_iter()
                .map(|batch| (batch.range, batch.tickers)),
            self.basis,
        );

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

    pub fn apply_data(&mut self, update: DataUpdate<'_>) {
        let trade_data_enabled = self.trade_data_enabled();
        match updates::apply(
            update,
            self.basis,
            &mut self.data,
            &mut self.fetch,
            &mut self.indicators,
            trade_data_enabled,
        ) {
            DataUpdateEffect::Unchanged => {}
            DataUpdateEffect::HistoricalChanged => {
                self.bump_rev();
            }
            DataUpdateEffect::LiveChanged { committed_bar } => {
                if committed_bar {
                    self.frame_revision = self.frame_revision.wrapping_add(1);
                }
                self.data_redraw_pending = true;
                self.redraw_data_if_due(Instant::now());
            }
        }
    }

    pub fn apply_fetch_update(&mut self, update: FetchUpdate) -> bool {
        match update {
            FetchUpdate::Failed(req_id) => self.fetch.mark_failed(req_id),
            FetchUpdate::TradeFetchNoData {
                ticker_info,
                req_id,
            } => self.fetch.mark_no_data(ticker_info, req_id),
            FetchUpdate::FetchHandle { req_id, handle } => self.fetch.set_handle(req_id, handle),
        }
    }

    fn prune_stale_indicator_y_ranges(&mut self) {
        let layout = self.indicators.layout();
        self.viewport
            .indicator_y_ranges
            .retain(|panel_id| layout.composition().panel(panel_id).is_some());
    }

    fn configure_profile_caches(&mut self) {
        self.data
            .configure_profile_caches(self.studies.active(), self.basis);
    }

    fn set_basis(&mut self, basis: Basis) -> Option<ChartAction> {
        self.finish_pending_drawing_edit();
        if self.basis != basis {
            self.history.clear();
        }
        self.drawings.tools.set_basis(basis);
        if matches!(basis, Basis::Tick(_)) {
            let base_ticker = self.data.base_ticker();
            let secondary_tickers = self
                .data
                .selected_tickers()
                .iter()
                .copied()
                .filter(|ticker| *ticker != base_ticker)
                .collect::<Vec<_>>();
            for ticker in secondary_tickers {
                self.remove_ticker(ticker);
            }
        }
        let preserve_raw_trades = matches!(
            (self.basis, basis),
            (Basis::Time(_), Basis::Time(_)) | (Basis::Tick(_), Basis::Tick(_))
        );
        self.basis = basis;
        self.indicators.normalize_for_basis(basis);
        self.studies.normalize_for_basis(basis);

        self.data.reset_for_basis(self.basis, preserve_raw_trades);
        self.sync_tick_drawing_sequence();
        self.frame_revision = self.frame_revision.wrapping_add(1);
        self.indicators
            .mark_rebuild_pending(IndicatorRebuildReason::BasisChanged);
        self.configure_profile_caches();

        self.fetch.rebuild_handlers(self.data.selected_tickers());

        // The next chart tick builds the plan from the settled basis and
        // launches all stages together. Old requests have already been
        // aborted and their request IDs invalidated above.
        None
    }

    fn reconcile_indicator_layout(&mut self, rebuild_all: bool) {
        self.indicators
            .sync_comparison_sources(self.data.base_ticker(), self.data.selected_tickers());
        let _ = self.indicators.reconcile_attached_indicator_placements();
        let active_indicators = self.indicators.selected_indicators();
        self.configure_indicator_runtimes(&active_indicators, rebuild_all);
        self.configure_profile_caches();
    }

    fn configure_indicator_runtimes(
        &mut self,
        active_indicators: &[IndicatorInstanceId],
        rebuild_all: bool,
    ) {
        let changed_indicator_ids = self
            .data
            .configure_indicators(self.indicators.instances(), active_indicators);
        self.indicators.retain_dirty_for(active_indicators);
        if rebuild_all {
            self.indicators
                .mark_rebuild_pending(IndicatorRebuildReason::IndicatorConfigurationChanged);
        }
        if !changed_indicator_ids.is_empty() {
            self.mark_indicators_with_dependents(
                &changed_indicator_ids,
                IndicatorRebuildReason::IndicatorConfigurationChanged,
            );
        }
    }

    fn apply_drawing_update(&mut self, update: DrawingUpdate, before: DrawingEditState) {
        let after = self.drawings.tools.edit_state();

        match update {
            DrawingUpdate::AtomicEdit => {
                self.history.finish_drawing(before.clone());
                if before != after {
                    self.history.record(before, after);
                }
            }
            DrawingUpdate::GestureStarted { .. } => {
                self.history.finish_drawing(before.clone());
                self.history.begin_drawing(before);
            }
            DrawingUpdate::GestureUpdated => {}
            DrawingUpdate::GestureFinished { .. } => {
                self.history.finish_drawing(after);
            }
            DrawingUpdate::None | DrawingUpdate::StateOnly | DrawingUpdate::Visual => {
                if self.history.has_pending_drawing() && !self.drawings.tools.is_dragging() {
                    self.history.finish_drawing(after);
                }
            }
        }

        if update.should_bump() {
            self.bump_rev();
        }
    }

    fn finish_pending_drawing_edit(&mut self) {
        self.history
            .finish_drawing(self.drawings.tools.edit_state());
    }

    fn undo_edit(&mut self) -> Option<Action> {
        self.finish_pending_drawing_edit();
        let entry = self.history.pop_undo()?;
        self.drawings.tools.restore_edit_state(entry.before.clone());
        self.history.push_redo(entry);
        self.bump_rev();
        None
    }

    fn redo_edit(&mut self) -> Option<Action> {
        self.finish_pending_drawing_edit();
        let entry = self.history.pop_redo()?;
        self.drawings.tools.restore_edit_state(entry.after.clone());
        self.history.push_undo(entry);
        self.bump_rev();
        None
    }

    pub fn apply_command(&mut self, command: ChartCommand) -> CommandResult {
        let result = self.apply_command_inner(command);
        self.reconcile_local_sidebar();
        result
    }

    fn apply_settings_action(&mut self, command: ChartCommand) -> CommandResult {
        self.apply_command_inner(command)
    }

    fn apply_command_inner(&mut self, command: ChartCommand) -> CommandResult {
        let previous_requirements = self.data_requirements();
        let previous_panels = self
            .indicators
            .composition()
            .panels
            .iter()
            .map(|panel| panel.id)
            .collect::<Vec<_>>();
        let mut dirty = false;

        let result = match command {
            ChartCommand::SetBasis(basis) => {
                dirty = true;
                CommandResult::Action(self.set_basis(basis))
            }
            ChartCommand::SetTradeDataEnabled(enabled) => {
                self.apply_trade_data_enabled(enabled, &mut dirty)
            }
            ChartCommand::SetLegendVisibility(visibility) => {
                self.apply_legend_visibility(visibility, &mut dirty)
            }
            ChartCommand::SetCandlestickConfig { config } => {
                let changed = self.options.candlestick != config;
                if changed {
                    self.options.candlestick = config;
                    dirty = true;
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::ResetCandlestickSettings => {
                let default_style = self.options.candlestick.with_default_style();
                let config_changed = self.options.candlestick != default_style;
                let mark_changed = self.indicators.layout_mut().reset_primary_mark();
                let ticker_info = self.data.base_ticker();
                let series_changed = self.data.reset_series_style(ticker_info);
                self.refresh_open_series_editor(ticker_info);
                if config_changed {
                    self.options.candlestick = default_style;
                }
                if config_changed || mark_changed || series_changed {
                    dirty = true;
                }
                CommandResult::Changed(config_changed || mark_changed || series_changed)
            }
            ChartCommand::SetCandlestickMark { mark } => {
                let changed = self.indicators.layout_mut().set_primary_mark(mark);
                if changed {
                    dirty = true;
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::SetCandlestickVisible { visible } => {
                let changed = self.options.candlestick.visible != visible;
                if changed {
                    self.options.candlestick = self.options.candlestick.with_visible(visible);
                    dirty = true;
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::SetFootprintVisible { visible } => {
                let changed = self.options.footprint.visible != visible;
                if changed {
                    self.options.footprint = self.options.footprint.with_visible(visible);
                    self.configure_profile_caches();
                    dirty = true;
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::SetFootprintClusterKind { kind } => {
                let changed = self.options.footprint.cluster_kind != kind;
                if changed {
                    self.options.footprint = self.options.footprint.with_cluster_kind(kind);
                    dirty = true;
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::SetFootprintClusterScaling { scaling } => {
                let changed = self.options.footprint.cluster_scaling != scaling;
                if changed {
                    self.options.footprint = self.options.footprint.with_cluster_scaling(scaling);
                    dirty = true;
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::SetFootprintImbalanceConfig { config } => {
                let changed = self.options.footprint.imbalance != config;
                if changed {
                    self.options.footprint = self.options.footprint.with_imbalance(config);
                    dirty = true;
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::SetFootprintPocConfig { config } => {
                let changed = self.options.footprint.poc != config;
                if changed {
                    self.options.footprint = self.options.footprint.with_poc(config);
                    self.configure_profile_caches();
                    dirty = true;
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::ResetFootprintSettings => {
                let default_options = self.options.footprint.with_default_options();
                let poc_changed = self.options.footprint.poc != default_options.poc;
                let changed = self.options.footprint != default_options;
                if changed {
                    self.options.footprint = default_options;
                    if poc_changed {
                        self.configure_profile_caches();
                    }
                    dirty = true;
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::SetFootprintTickMultiplier { multiplier } => {
                if self.data.set_footprint_tick_multiplier(multiplier) {
                    self.data.configure_series_for_basis(self.basis);
                    self.configure_profile_caches();
                    dirty = true;
                }
                CommandResult::None
            }
            ChartCommand::AddIndicator(kind) => {
                let change = self.indicators.add_indicator(kind, self.basis, &self.data);
                self.apply_indicator_change(change, &mut dirty)
            }
            ChartCommand::AddIndicatorOnSource {
                kind,
                input_id,
                source,
            } => {
                let change = self
                    .indicators
                    .add_indicator_on_source(kind, input_id, source, self.basis, &self.data);
                self.apply_indicator_change(change, &mut dirty)
            }
            ChartCommand::RemoveIndicator(id) => {
                let change = self.indicators.remove_indicator(id);
                self.apply_indicator_change(change, &mut dirty)
            }
            ChartCommand::Indicator(command) => self.apply_indicator_command(command, &mut dirty),
            ChartCommand::Study(command) => self.apply_study_command(command, &mut dirty),
            ChartCommand::SetSeriesColor { ticker_info, color } => {
                let changed = self.data.set_series_color(ticker_info, color);
                if changed {
                    dirty = true;
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::SetSeriesLineWidth { ticker_info, width } => {
                let changed = self.data.set_series_line_width(ticker_info, width);
                if changed {
                    dirty = true;
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::SetSeriesStrokeStyle {
                ticker_info,
                stroke_style,
            } => {
                let changed = self.data.set_series_stroke_style(ticker_info, stroke_style);
                if changed {
                    dirty = true;
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::ResetSeriesSettings { ticker_info } => {
                let changed = self.data.reset_series_style(ticker_info);
                self.refresh_open_series_editor(ticker_info);
                if changed {
                    dirty = true;
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::AddTicker { ticker_info } => {
                let changed = if ticker_info != self.data.base_ticker()
                    && self.data.add_ticker(ticker_info, self.basis)
                {
                    self.fetch.add_ticker(ticker_info);
                    self.indicators.sync_comparison_sources(
                        self.data.base_ticker(),
                        self.data.selected_tickers(),
                    );
                    let active_indicators = self.selected_indicators();
                    self.configure_indicator_runtimes(&active_indicators, false);
                    self.mark_indicators_for_candle_ticker(
                        ticker_info,
                        IndicatorRebuildReason::IndicatorConfigurationChanged,
                    );
                    true
                } else {
                    false
                };
                if changed {
                    dirty = true;
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::RemoveTicker { ticker_info } => {
                let changed = self.remove_ticker(ticker_info);
                if changed {
                    dirty = true;
                }
                CommandResult::Changed(changed)
            }
        };

        self.reconcile_data_requirements(previous_requirements);
        if previous_panels
            .iter()
            .any(|panel_id| self.indicators.composition().panel(*panel_id).is_none())
        {
            self.history.clear();
        }

        if dirty {
            self.bump_rev();
        }
        result
    }

    fn close_panel_at_index(&mut self, index: usize) -> bool {
        self.finish_pending_drawing_edit();
        let previous_requirements = self.data_requirements();

        let Some(panel) = self.indicators.composition().panels.get(index) else {
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
        let panel_removed = self.indicators.composition().panel(panel_id).is_some()
            && self.indicators.layout_mut().close_panel(panel_id);
        let changed = indicator_removed || panel_removed;

        if !changed {
            self.drawings.tools.restore_edit_state(before_drawings);
            return false;
        }

        if indicator_removed {
            self.reconcile_indicator_layout(false);
        }
        self.reconcile_data_requirements(previous_requirements);
        let _ = self.viewport.indicator_y_ranges.reset(panel_id);
        self.history.clear();
        self.bump_rev();
        true
    }

    fn apply_trade_data_enabled(&mut self, enabled: bool, dirty: &mut bool) -> CommandResult {
        if !enabled && self.trade_data_setting_locked() {
            return CommandResult::Changed(false);
        }
        if !self.options.set_trade_data_requested(enabled, self.basis) {
            return CommandResult::Changed(false);
        }
        self.configure_profile_caches();
        *dirty = true;
        CommandResult::Changed(true)
    }

    fn apply_legend_visibility(
        &mut self,
        visibility: LegendVisibility,
        dirty: &mut bool,
    ) -> CommandResult {
        if self.options.legend_visibility == visibility {
            return CommandResult::Changed(false);
        }
        self.options.legend_visibility = visibility;
        *dirty = true;
        CommandResult::Changed(true)
    }

    fn apply_indicator_command(
        &mut self,
        command: IndicatorCommand,
        dirty: &mut bool,
    ) -> CommandResult {
        let change = self.indicators.apply(command, self.basis, &self.data);
        self.apply_indicator_change(change, dirty)
    }

    fn apply_indicator_change(
        &mut self,
        change: IndicatorChange,
        dirty: &mut bool,
    ) -> CommandResult {
        match change {
            IndicatorChange::Unchanged => CommandResult::Changed(false),
            IndicatorChange::Visual => {
                *dirty = true;
                CommandResult::Changed(true)
            }
            IndicatorChange::Reconfigure { id, request_fetch } => {
                self.data
                    .reconfigure_indicator(id, self.indicators.instances());
                self.mark_indicators_with_dependents(
                    &[id],
                    IndicatorRebuildReason::IndicatorConfigurationChanged,
                );
                *dirty = true;
                if request_fetch {
                    CommandResult::Action(
                        self.request_fetch_action(false)
                            .map(ChartAction::RequestFetch),
                    )
                } else {
                    CommandResult::Changed(true)
                }
            }
            IndicatorChange::Reconcile { request_fetch } => {
                self.reconcile_indicator_layout(false);
                *dirty = true;
                if request_fetch {
                    CommandResult::Action(
                        self.request_fetch_action(false)
                            .map(ChartAction::RequestFetch),
                    )
                } else {
                    CommandResult::Changed(true)
                }
            }
        }
    }

    fn apply_study_command(&mut self, command: StudyCommand, dirty: &mut bool) -> CommandResult {
        let change = self
            .studies
            .apply(command, self.basis, self.trade_data_enabled());
        self.apply_study_change(change, dirty)
    }

    fn apply_study_change(
        &mut self,
        change: studies::StudyChange,
        dirty: &mut bool,
    ) -> CommandResult {
        if change.profile_data_changed {
            self.configure_profile_caches();
        }
        if change.changed {
            *dirty = true;
        }
        CommandResult::Changed(change.changed)
    }

    fn remove_ticker(&mut self, ticker_info: TickerInfo) -> bool {
        if ticker_info == self.data.base_ticker() || !self.data.remove_ticker(ticker_info) {
            return false;
        }

        let was_multi_source = self.indicators.layout().primary_uses_multi_source();
        if self.inspector.series_editor().show_config_for == Some(ticker_info) {
            self.inspector.series_editor_mut().close();
        }
        self.fetch.remove_ticker(ticker_info);
        self.indicators
            .sync_comparison_sources(self.data.base_ticker(), self.data.selected_tickers());
        if was_multi_source
            && !self.indicators.layout().primary_uses_multi_source()
            && self.indicators.restore_primary_scale_after_comparisons()
        {
            self.viewport.primary.reset();
        }
        let active_indicators = self.selected_indicators();
        self.configure_indicator_runtimes(&active_indicators, false);
        self.mark_indicators_for_candle_ticker(
            ticker_info,
            IndicatorRebuildReason::IndicatorConfigurationChanged,
        );
        true
    }

    fn replace_indicator_configuration(
        &mut self,
        instances: IndicatorInstanceStore,
        panels: &[persisted::PanelConfig],
        splits: &[f32],
    ) {
        self.indicators.replace_instances(instances);
        if !panels.is_empty() {
            let panels = panels
                .iter()
                .map(persisted::panel_spec_from_config)
                .collect();
            self.indicators
                .layout_mut()
                .replace_composition(panels, splits.to_vec());
        } else if let Some(volume_id) = self
            .indicators
            .instances()
            .instances()
            .iter()
            .find(|instance| {
                instance.kind == IndicatorId::parse("core.volume").expect("static ID is valid")
            })
            .map(|instance| instance.id)
        {
            self.indicators
                .replace_layout(IndicatorLayout::new(volume_id));
        } else {
            self.indicators.replace_layout(IndicatorLayout::empty());
        }
    }

    fn apply_shared_settings(&mut self, snapshot: &SettingsSnapshot) {
        let previous_requirements = self.data_requirements();
        self.replace_indicator_configuration(
            snapshot.indicator_instances_for(self.basis),
            &snapshot.panels,
            &snapshot.splits,
        );
        self.viewport.primary_scale_inverted = snapshot.primary_scale_inverted;
        self.options.trade_data_requested = snapshot.trade_data_enabled;
        self.options.legend_visibility = snapshot.legend_visibility;
        self.options.candlestick = snapshot.candlestick;
        self.options.footprint = snapshot.footprint;
        self.studies.restore(studies::StudySnapshot::new(
            snapshot.studies.clone(),
            snapshot.study_configs.clone(),
        ));
        self.studies.normalize_for_basis(self.basis);
        let required_trade_data = self.indicators.selected_indicators().iter().any(|id| {
            self.indicators.indicator(*id).is_some_and(|instance| {
                instance.kind.is_registered() && self.indicator_requires_trade_data(&instance.kind)
            })
        });
        if !self.options.trade_data_requested && !required_trade_data {
            self.options.footprint = self.options.footprint.with_visible(false);
        }
        self.reconcile_indicator_layout(true);
        self.reconcile_data_requirements(previous_requirements);
    }

    fn apply_persisted_config(&mut self, config: &persisted::Config) {
        let snapshot = config.settings_snapshot();
        self.history.clear();
        self.apply_shared_settings(&snapshot);
        self.drawing_sidebar_visible = config.drawing_sidebar_visible;
        self.sidebar = Sidebar::Drawings;
        self.drawings.floating_toolbar =
            toolbar::FloatingToolbarState::from_position(config.floating_toolbar_position);
        let drawings = config
            .drawings
            .iter()
            .filter(|drawing| {
                DrawingTools::drawing_panels_exist(self.indicators.composition(), &drawing.object)
            })
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
        self.bump_rev();
    }

    pub fn settings_snapshot(&self) -> SettingsSnapshot {
        self.serializable_config().settings_snapshot()
    }

    pub fn apply_settings_snapshot(&mut self, snapshot: &SettingsSnapshot) -> Option<Action> {
        self.apply_shared_settings(snapshot);
        self.fetch.rebuild_handlers(self.data.selected_tickers());
        self.reconcile_local_sidebar();
        self.bump_rev();

        self.request_fetch_action(false).map(Action::RequestFetch)
    }

    pub fn studies(&self) -> &[KlineStudy] {
        self.studies.active()
    }

    pub fn trade_data_enabled(&self) -> bool {
        self.data_requirements().trades
    }

    fn legend_visibility(&self) -> LegendVisibility {
        self.options.legend_visibility
    }

    fn trade_data_setting_locked(&self) -> bool {
        matches!(self.basis, Basis::Tick(_))
            || self.indicators.selected_indicators().iter().any(|id| {
                self.indicators.indicator(*id).is_some_and(|instance| {
                    instance.kind.is_registered()
                        && self.indicator_requires_trade_data(&instance.kind)
                })
            })
    }

    pub fn drawing_sidebar_visible(&self) -> bool {
        self.sidebar == Sidebar::Drawings && self.drawing_sidebar_visible
    }

    fn indicator_requires_trade_data(&self, indicator: &IndicatorId) -> bool {
        self.indicators
            .requires_trade_data(indicator, self.basis, self.data.base_ticker())
    }

    pub fn basis(&self) -> Basis {
        self.basis
    }

    pub fn selected_indicators(&self) -> Vec<IndicatorInstanceId> {
        self.indicators.selected_indicators()
    }

    pub fn data_requirements(&self) -> KlineDataRequirements {
        let mut requirements = KlineDataRequirements {
            klines: !self.data.selected_tickers().is_empty(),
            trades: self.options.trade_data_requested || matches!(self.basis, Basis::Tick(_)),
            ..KlineDataRequirements::default()
        };

        let indicator_requirements = self
            .indicators
            .data_requirements(self.basis, self.data.base_ticker());
        requirements.klines |= indicator_requirements.klines;
        requirements.directional_volume |= indicator_requirements.directional_volume;
        requirements.trades |= indicator_requirements.trades;
        requirements.open_interest |= indicator_requirements.open_interest;

        if self.options.footprint.visible
            || self
                .studies
                .active()
                .iter()
                .any(|study| study.requires_trade_data())
        {
            requirements.trades = true;
        }
        requirements
    }

    pub fn indicator_kind(&self, id: IndicatorInstanceId) -> Option<IndicatorId> {
        self.indicators.indicator_kind(id)
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

    fn indicator_presentation_context(&self) -> IndicatorPresentationContext {
        IndicatorPresentationResolver::new(
            &self.indicators,
            &self.data,
            self.basis,
            self.trade_data_enabled(),
        )
        .context()
    }

    fn trade_data_required(&self) -> bool {
        self.data_requirements().trades
    }

    pub(crate) fn desired_streams(&self) -> Vec<StreamKind> {
        match self.basis {
            Basis::Time(timeframe) => {
                let mut streams = Vec::new();
                if self.trade_data_required() {
                    streams.push(StreamKind::Trades {
                        ticker_info: self.data.base_ticker(),
                    });
                }
                streams.extend(
                    self.data
                        .selected_tickers()
                        .iter()
                        .copied()
                        .map(|ticker_info| StreamKind::Kline {
                            ticker_info,
                            timeframe,
                        }),
                );
                streams
            }
            Basis::Tick(_) => vec![StreamKind::Trades {
                ticker_info: self.data.base_ticker(),
            }],
        }
    }

    pub fn serializable_config(&self) -> persisted::Config {
        persisted::Config {
            primary_scale_inverted: self.viewport.primary_scale_inverted,
            trade_data_enabled: self.options.trade_data_requested,
            legend_visibility: self.options.legend_visibility,
            candlestick: self.options.candlestick,
            footprint: self.options.footprint,
            panels: self
                .indicators
                .composition()
                .panels
                .iter()
                .map(persisted::panel_config_from_spec)
                .collect(),
            splits: self.indicators.composition().splits.clone(),
            drawings: self.drawings.tools.time_drawings().to_vec(),
            active_drawing_tool: self.drawings.tools.active_tool(),
            floating_toolbar_position: self.drawings.floating_toolbar.position,
            drawing_sidebar_visible: self.drawing_sidebar_visible,
            studies: self.studies.active_snapshot(),
            study_configs: self.studies.configs_snapshot(),
            indicator_instances: self.indicators.instances().instances().to_vec(),
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

fn sidebar_header<'a>(title: String) -> iced::Element<'a, Message> {
    let title = iced::widget::container(
        crate::widget::text::emphasis(title).wrapping(iced::widget::text::Wrapping::Glyph),
    )
    .width(iced::Length::Fill)
    .height(iced::Length::Fill)
    .align_y(iced::Alignment::Center)
    .clip(true);

    let return_button = iced::widget::button(style::icon_text(style::Icon::Return, 12))
        .style(|theme, status| style::button::transparent(theme, status, false))
        .on_press(Message::ShowDrawingsPage)
        .width(iced::Length::Fixed(SIDEBAR_RETURN_BUTTON_WIDTH));

    iced::widget::row![title, return_button]
        .width(iced::Length::Fill)
        .align_y(iced::Alignment::Center)
        .height(iced::Length::Fixed(32.0))
        .spacing(8)
        .into()
}

fn sidebar_panel<'a>(
    header: iced::Element<'a, Message>,
    body: iced::Element<'a, Message>,
    footer: Option<iced::Element<'a, Message>>,
) -> iced::Element<'a, Message> {
    iced::widget::container(iced::widget::column![
        header,
        iced::widget::rule::horizontal(1.0),
        body,
        footer.unwrap_or_else(|| iced::widget::column![].into()),
    ])
    .width(iced::Length::Fixed(SIDEBAR_WIDTH))
    .height(iced::Length::Fill)
    .padding([4, 8])
    .style(style::chart_sidebar_container)
    .into()
}
