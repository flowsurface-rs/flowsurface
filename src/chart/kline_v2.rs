#[path = "kline_v2/data.rs"]
mod data_state;
mod drawing;
mod fetch;
mod indicator;
mod layout;
mod series_adapter;
mod viewport;

pub use data::chart::kline_v2::KlineIndicator;
pub use series_adapter::KlineSeries;

use data_state::KlineData;
use drawing::{DrawingMessage, DrawingTools, DrawingUpdate};
use fetch::FetchCoordinator;
use indicator::IndicatorUnsupportedReason;
use layout::IndicatorLayout;
use viewport::PanelViewportStore;

use crate::connector::fetcher::FetchSpec;
use crate::modal::pane::settings::study;
use crate::widget::chart::kline::PanelInteraction;
use crate::widget::chart::kline::composition::{
    BarMode, ChartComposition, DEFAULT_MIN_PANEL_RATIO, HistogramMode, LayerDataKind, MarkKind,
    PanelId, PanelRole, PanelScaleMode,
};
use crate::widget::chart::kline::drawing::DrawingObject;
use crate::widget::chart::kline::{
    DEFAULT_BAR_SPACING_PX, KlineWidget, KlineWidgetEvent, PanelYViewport, UnavailableIndicator,
};
use crate::widget::series_editor::{self, SeriesStyleEntry, TickerSeriesEditor};

use data::chart::Basis;
use data::chart::kline::ClusterKind;
use data::chart::kline_v2::{
    self as persisted, CandlestickConfig, FetchBatch, FetchPlanInput, FetchStage, HorizontalWorld,
    IndicatorColorSlot, IndicatorConfig, IndicatorEngine, IndicatorInstanceId, KlineStudy,
    SeriesColor, desired_fetch_batches as plan_fetch_batches,
    desired_trade_fetch_range as plan_trade_fetch_range, visible_window,
};
use exchange::adapter::StreamKind;
use exchange::{Kline, OpenInterest, TickMultiplier, TickerInfo, Trade, UnixMs};
use iced::task::Handle;

use std::time::Instant;

const DEFAULT_HORIZONTAL_OFFSET_UNITS: f32 = 8.0;
pub const DEFAULT_FOOTPRINT_TICK_MULTIPLIER: TickMultiplier = TickMultiplier(10);

pub enum Action {
    RemoveSeries(TickerInfo),
    OpenSeriesEditor,
    OpenSeriesSettings(TickerInfo),
    OpenIndicatorSettings(IndicatorInstanceId),
    OpenCandlestickSettings,
    RequestFetch(Vec<FetchSpec>),
}

#[derive(Debug, Clone)]
pub enum Message {
    Chart(KlineWidgetEvent),
    Sidebar(DrawingMessage),
    SeriesEditor(series_editor::Message),
}

pub enum ChartCommand {
    SetPrimaryScaleMode(PanelScaleMode),
    SetIndicatorDataConfig {
        id: IndicatorInstanceId,
        config: IndicatorConfig,
    },
    SetIndicatorColor {
        id: IndicatorInstanceId,
        slot: IndicatorColorSlot,
        accent: Option<data::chart::kline_v2::ThemeAccent>,
        shade: data::config::theme::ThemeColorShade,
    },
    ResetIndicatorSettings(IndicatorInstanceId),
    SetCandlestickConfig(CandlestickConfig),
    ResetCandlestickSettings,
    SetPrimaryMark(MarkKind),
    SetCandlesticksVisible(bool),
    SetFootprintsVisible(bool),
    SetTradeDataEnabled(bool),
    SetFootprintClusterKind(ClusterKind),
    SetIndicatorMark {
        id: IndicatorInstanceId,
        mark: MarkKind,
    },
    SetStudies(Vec<KlineStudy>),
    SetPanelMark {
        panel_index: usize,
        mark: MarkKind,
    },
    SetPanelDataKind {
        panel_index: usize,
        data_kind: LayerDataKind,
    },
    SetPanelBarMode {
        panel_index: usize,
        mode: BarMode,
    },
    SetPanelHistogramMode {
        panel_index: usize,
        mode: HistogramMode,
    },
    ToggleIndicatorVisibility(IndicatorInstanceId),
    AddIndicator(KlineIndicator),
    RemoveIndicator(IndicatorInstanceId),
    AddTicker(TickerInfo),
    RemoveTicker(TickerInfo),
    SetSeriesColor {
        ticker_info: TickerInfo,
        color: SeriesColor,
    },
    SetBasis(Basis),
    SetFootprintTickMultiplier(TickMultiplier),
}

pub enum CommandResult {
    None,
    Changed(bool),
    Streams(Vec<StreamKind>),
    Action(Option<crate::chart::Action>),
}

pub enum DataUpdate<'a> {
    HistoricalKlines {
        req_id: Option<uuid::Uuid>,
        ticker_info: TickerInfo,
        klines: &'a [Kline],
    },
    LiveKline {
        ticker_info: &'a TickerInfo,
        kline: &'a Kline,
    },
    LiveTrades {
        ticker_info: &'a TickerInfo,
        trades: &'a [Trade],
    },
    HistoricalTrades {
        trades: &'a [Trade],
        is_batches_done: bool,
        req_id: Option<uuid::Uuid>,
    },
    HistoricalOpenInterest {
        req_id: Option<uuid::Uuid>,
        ticker_info: TickerInfo,
        data: &'a [OpenInterest],
    },
}

pub enum FetchUpdate {
    Failed(uuid::Uuid),
    TradeFetchNoData(uuid::Uuid),
    TradeFetchHandle(Handle),
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
    panel_y_viewports: PanelViewportStore,
    plot_width: Option<f32>,
    layout: IndicatorLayout,
    last_tick: Instant,
    cache_rev: u64,
    data: KlineData,
    indicators: IndicatorEngine,
    fetch: FetchCoordinator,
    drawing: DrawingTools,
    studies: Vec<KlineStudy>,
    study_configs: Vec<KlineStudy>,
    study_configurator: study::Configurator<KlineStudy>,
    series_editor: TickerSeriesEditor,
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
        let mut indicators = IndicatorEngine::default();
        let default_volume = indicators.create_instance(KlineIndicator::Volume);

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
            Message::SeriesEditor(message) => {
                if let Some(change) = self.series_editor.update(message) {
                    let command = match change {
                        series_editor::Change::ColorChanged(ticker_info, color) => {
                            ChartCommand::SetSeriesColor { ticker_info, color }
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

                                    if self.layout.composition_mut().remove_panel(panel_id) {
                                        let _ = self.panel_y_viewports.reset(panel_id);
                                        self.layout.prune_stale_bindings();
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
                            return Some(Action::OpenCandlestickSettings);
                        }
                        if let Some(series) = self
                            .data
                            .series
                            .iter()
                            .find(|series| series.ticker_info == ticker)
                        {
                            self.series_editor.open_for(ticker, series.color());
                        }
                        return Some(Action::OpenSeriesSettings(ticker));
                    }
                    KlineWidgetEvent::FootprintSettings => {
                        return Some(Action::OpenSeriesEditor);
                    }
                    KlineWidgetEvent::CandlesticksVisibilityToggled => {
                        let _ = self.apply_command(ChartCommand::SetCandlesticksVisible(
                            !self.candlesticks_visible,
                        ));
                    }
                    KlineWidgetEvent::FootprintsVisibilityToggled => {
                        let _ = self.apply_command(ChartCommand::SetFootprintsVisible(
                            !self.footprints_visible,
                        ));
                    }
                    KlineWidgetEvent::IndicatorSettings(id) => {
                        return Some(Action::OpenIndicatorSettings(id));
                    }
                    KlineWidgetEvent::IndicatorVisibilityToggled(id) => {
                        let _ = self.apply_command(ChartCommand::ToggleIndicatorVisibility(id));
                    }
                    KlineWidgetEvent::IndicatorRemove(id) => {
                        let _ = self.apply_command(ChartCommand::RemoveIndicator(id));
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
        if self.data.series.iter().all(|series| series.bars.is_empty()) {
            return iced::widget::center(iced::widget::text("Waiting for data...").size(16)).into();
        }

        let drawing_sidebar = self
            .drawing
            .view(self.layout.composition(), Message::Sidebar);
        let unavailable_indicator_panels = self.unavailable_indicator_panels();
        let mut chart = KlineWidget::new(
            &self.data.series,
            self.basis,
            self.layout.composition(),
            self.indicators.instances(),
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
            .with_coordinate_state(self.horizontal_world, pixel_ratio, self.primary_autoscale)
            .with_known_plot_width(self.plot_width)
            .with_panels(self.panel_y_viewports.as_slice())
            .with_indicator_availability(unavailable_indicator_panels)
            .with_drawings(self.drawing.snapshot())
            .with_timezone(timezone)
            .with_version(self.cache_rev)
            .into();

        let chart = iced::widget::container(chart.map(Message::Chart))
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .padding(1);

        iced::widget::row![drawing_sidebar.padding(4), chart].into()
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
            indicator_instances: self.indicators.instances(),
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
        let visible_units = (plot_width / spacing).max(1.0).ceil() as u64;
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
        let recurring_intervals = self
            .studies
            .iter()
            .filter_map(|study| {
                study
                    .recurring_profile_interval()
                    .filter(|_| study.is_interval_available(self.basis))
            })
            .collect::<Vec<_>>();
        let footprint_poc_enabled = self
            .studies
            .iter()
            .any(|study| study.is_footprint_poc() && study.is_available(self.basis));
        self.data
            .configure_recurring_profile_intervals(&recurring_intervals);
        self.data
            .configure_footprint_poc_cache(footprint_poc_enabled);
    }

    fn set_basis(&mut self, basis: Basis) -> Option<super::Action> {
        let preserve_raw_trades =
            matches!(self.basis, Basis::Tick(_)) && matches!(basis, Basis::Tick(_));
        self.basis = basis;
        let _ = self.indicators.normalize_configs_for_basis(basis);
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
        self.bump_rev();

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
            self.bump_rev();
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
        match command {
            ChartCommand::SetPrimaryScaleMode(scale) => {
                CommandResult::Changed(self.set_primary_scale_mode(scale))
            }
            ChartCommand::SetCandlestickConfig(config) => {
                let changed = self.candlestick_config != config;
                if changed {
                    self.candlestick_config = config;
                    self.bump_rev();
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::ResetCandlestickSettings => {
                let config_changed = self.candlestick_config != CandlestickConfig::default();
                let mark_changed = self.layout.reset_primary_mark();
                if config_changed {
                    self.candlestick_config = CandlestickConfig::default();
                }
                if config_changed || mark_changed {
                    self.bump_rev();
                }
                CommandResult::Changed(config_changed || mark_changed)
            }
            ChartCommand::SetPrimaryMark(mark) => {
                let changed = self.layout.set_primary_mark(mark);
                if changed {
                    self.bump_rev();
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::SetCandlesticksVisible(visible) => {
                let changed = self.candlesticks_visible != visible;
                if changed {
                    self.candlesticks_visible = visible;
                    self.bump_rev();
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::SetFootprintsVisible(visible) => {
                let changed = self.footprints_visible != visible;
                if changed {
                    self.footprints_visible = visible;
                    self.bump_rev();
                    CommandResult::Changed(true)
                } else {
                    CommandResult::Changed(false)
                }
            }
            ChartCommand::SetTradeDataEnabled(enabled) => {
                let changed = self.trade_data_enabled != enabled;
                if changed {
                    self.trade_data_enabled = enabled;
                    if !matches!(self.basis, Basis::Tick(_)) {
                        self.footprints_visible = enabled;
                        if !enabled {
                            for id in self.layout.selected_indicators() {
                                if self.indicators.instances().get(id).is_some_and(|instance| {
                                    indicator::requires_trade_data(instance.kind)
                                }) {
                                    let _ = self.layout.remove_indicator(id);
                                    let _ = self.indicators.remove_instance(id);
                                }
                            }
                            self.studies.retain(|study| !study.requires_trade_data());
                        }
                    }
                    self.configure_profile_caches();
                    self.bump_rev();
                    CommandResult::Changed(true)
                } else {
                    CommandResult::Changed(false)
                }
            }
            ChartCommand::SetFootprintClusterKind(kind) => {
                let changed = self.footprint_cluster_kind != kind;
                if changed {
                    self.footprint_cluster_kind = kind;
                    self.bump_rev();
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::SetIndicatorDataConfig { id, config } => {
                let requests_history = self.layout.selected_indicators().contains(&id);
                let changed = self
                    .indicators
                    .update_config_for_basis(id, config, self.basis);
                if changed {
                    let active_indicators = self.layout.selected_indicators();
                    self.data.configure_indicators(
                        self.basis,
                        &self.indicators,
                        &active_indicators,
                    );
                    self.bump_rev();
                    if requests_history {
                        return CommandResult::Action(
                            self.request_fetch_action().map(super::Action::RequestFetch),
                        );
                    }
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::SetIndicatorColor {
                id,
                slot,
                accent,
                shade,
            } => {
                let Some(current) = self
                    .indicators
                    .instances()
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
                    self.bump_rev();
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::ResetIndicatorSettings(id) => {
                let data_changed = self.indicators.reset_config(id);
                let color_changed = self.indicators.reset_colors(id);
                let mark_changed = self.layout.reset_indicator_mark(id);
                let presentation_changed = self.layout.reset_indicator_presentation(id);
                let changed = data_changed || color_changed || mark_changed || presentation_changed;

                if data_changed {
                    let active_indicators = self.layout.selected_indicators();
                    self.data.configure_indicators(
                        self.basis,
                        &self.indicators,
                        &active_indicators,
                    );
                }
                if changed {
                    self.bump_rev();
                }

                CommandResult::Changed(changed)
            }
            ChartCommand::SetIndicatorMark { id, mark } => {
                let changed = self.layout.set_indicator_mark(id, mark);
                if changed {
                    self.bump_rev();
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::SetStudies(studies) => {
                let normalized = self.normalize_studies_for_basis(studies);

                self.remember_study_configs(&normalized);
                let changed = self.studies != normalized;
                if changed {
                    self.studies = normalized;
                    self.configure_profile_caches();
                    self.bump_rev();
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::SetPanelMark { panel_index, mark } => {
                let changed = self.layout.set_panel_mark(panel_index, mark);
                if changed {
                    self.bump_rev();
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::SetPanelDataKind {
                panel_index,
                data_kind,
            } => {
                let changed = self.layout.set_panel_data_kind(panel_index, data_kind);
                if changed {
                    self.bump_rev();
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::SetPanelBarMode { panel_index, mode } => {
                let changed = self.layout.set_panel_bar_mode(panel_index, mode);
                if changed {
                    self.bump_rev();
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::SetPanelHistogramMode { panel_index, mode } => {
                let changed = self.layout.set_panel_histogram_mode(panel_index, mode);
                if changed {
                    self.bump_rev();
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::ToggleIndicatorVisibility(id) => {
                let changed = self.layout.toggle_indicator_visibility(id);
                if changed {
                    self.bump_rev();
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::AddIndicator(kind) => {
                if !self.indicator_is_supported_for_market(kind) {
                    return CommandResult::Changed(false);
                }
                let id = self.indicators.create_instance(kind);
                let changed =
                    self.layout
                        .add_indicator(id, kind, self.data.base_ticker().market_type());
                if !changed {
                    let _ = self.indicators.remove_instance(id);
                    return CommandResult::Changed(false);
                }
                let active_indicators = self.layout.selected_indicators();
                self.data
                    .configure_indicators(self.basis, &self.indicators, &active_indicators);
                self.bump_rev();
                CommandResult::Action(self.request_fetch_action().map(super::Action::RequestFetch))
            }
            ChartCommand::RemoveIndicator(id) => {
                let changed = self.layout.remove_indicator(id);
                if changed {
                    let _ = self.indicators.remove_instance(id);
                    let active_indicators = self.layout.selected_indicators();
                    self.data.configure_indicators(
                        self.basis,
                        &self.indicators,
                        &active_indicators,
                    );
                    self.bump_rev();
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::AddTicker(ticker_info) => {
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
                    self.bump_rev();
                }
                CommandResult::Streams(self.streams_for_all())
            }
            ChartCommand::RemoveTicker(ticker_info) => {
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
                    self.bump_rev();
                }
                CommandResult::Streams(self.streams_for_all())
            }
            ChartCommand::SetSeriesColor { ticker_info, color } => {
                let changed = self.data.set_series_color(ticker_info, color);
                if changed {
                    self.bump_rev();
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::SetBasis(basis) => CommandResult::Action(self.set_basis(basis)),
            ChartCommand::SetFootprintTickMultiplier(multiplier) => {
                if self.data.set_footprint_tick_multiplier(multiplier) {
                    let active_indicators = self.layout.selected_indicators();
                    self.data.configure_series_for_basis(
                        self.basis,
                        &self.indicators,
                        &active_indicators,
                    );
                    self.configure_profile_caches();
                    self.bump_rev();
                }
                CommandResult::None
            }
        }
    }

    fn apply_persisted_config(&mut self, config: &persisted::Config) {
        let _ = self
            .indicators
            .replace_instances(config.indicator_instances_for(self.basis));
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
            config.horizontal.offset,
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
                    .instances()
                    .get(id)
                    .is_some_and(|instance| indicator::requires_trade_data(instance.kind))
                {
                    let _ = self.layout.remove_indicator(id);
                    let _ = self.indicators.remove_instance(id);
                }
            }
            self.studies.retain(|study| !study.requires_trade_data());
        }
        self.candlestick_config = config.candlestick;
        self.footprint_cluster_kind = config.footprint_cluster_kind;
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
                Self::drawing_panels_exist(self.layout.composition(), &drawing.object)
            })
            .cloned()
            .collect();
        self.drawing = DrawingTools::from_parts(config.active_drawing_tool, drawings);

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

            if let Some(accent) = series_config.accent {
                let _ = self.data.series[index].set_series_color(SeriesColor::Accent(accent));
            } else if let Some(color) = series_config.color {
                let _ = self.data.series[index].set_color(color);
            }
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
            })
            .collect()
    }

    pub fn trade_data_enabled(&self) -> bool {
        self.trade_data_enabled || matches!(self.basis, Basis::Tick(_))
    }

    pub fn indicator_requires_trade_data(&self, indicator: KlineIndicator) -> bool {
        indicator::requires_trade_data(indicator)
    }

    pub fn footprint_cluster_kind(&self) -> ClusterKind {
        self.footprint_cluster_kind
    }

    pub fn study_configurator(&self) -> &study::Configurator<KlineStudy> {
        &self.study_configurator
    }

    pub fn update_study_configurator(&mut self, message: study::Message<KlineStudy>) {
        let Some(action) = self.study_configurator.update(message) else {
            return;
        };

        let mut studies = self.studies.clone();
        match action {
            study::Action::ToggleStudy(study, is_selected) => {
                if is_selected {
                    let study = self
                        .study_configs
                        .iter()
                        .find(|configured| configured.is_same_type(study))
                        .copied()
                        .unwrap_or(study);
                    if !studies.iter().any(|active| active.is_same_type(study)) {
                        studies.push(study);
                    }
                } else {
                    studies.retain(|active| !active.is_same_type(study));
                }
            }
            study::Action::ConfigureStudy(study) => {
                if let Some(active) = studies.iter_mut().find(|active| active.is_same_type(study)) {
                    *active = study;
                }
            }
        }

        let _ = self.apply_command(ChartCommand::SetStudies(studies));
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
            self.indicators.instances(),
        )
    }

    pub fn indicator_mark(&self, id: IndicatorInstanceId) -> Option<MarkKind> {
        let kind = self.indicators.instances().get(id)?.kind;
        let signed_overlay_input = kind.uses_directional_volume()
            && self.data.base_series().is_some_and(|series| {
                series
                    .bars
                    .iter()
                    .any(|bar| bar.volume.buy_sell().is_some())
            });
        self.layout.indicator_mark(id, signed_overlay_input)
    }

    pub fn indicator_data_config(&self, id: IndicatorInstanceId) -> Option<IndicatorConfig> {
        self.indicators
            .instances()
            .get(id)
            .map(|instance| instance.config)
    }

    pub fn indicator_kind(&self, id: IndicatorInstanceId) -> Option<KlineIndicator> {
        self.indicators
            .instances()
            .get(id)
            .map(|instance| instance.kind)
    }

    pub fn indicator_color_config(
        &self,
        id: IndicatorInstanceId,
    ) -> Option<data::chart::kline_v2::IndicatorColorConfig> {
        self.indicators
            .instances()
            .get(id)
            .map(|instance| instance.colors)
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
        let trade_data_disabled = self.trade_data_unavailable_indicators();
        self.layout
            .unavailable_indicators(
                self.basis,
                self.data.base_ticker(),
                self.data.base_series(),
                self.indicators.instances(),
            )
            .into_iter()
            .chain(trade_data_disabled)
            .filter_map(|(id, reason)| {
                let kind = self.indicators.instances().get(id)?.kind;
                Some(UnavailableIndicator {
                    value_id: id,
                    message: self.unavailable_indicator_message(kind, reason),
                })
            })
            .collect()
    }

    fn trade_data_unavailable_indicators(
        &self,
    ) -> Vec<(IndicatorInstanceId, IndicatorUnsupportedReason)> {
        self.layout.trade_data_unavailable_indicators(
            self.trade_data_enabled(),
            self.indicators.instances(),
        )
    }

    fn unavailable_indicator_message(
        &self,
        indicator: KlineIndicator,
        reason: IndicatorUnsupportedReason,
    ) -> String {
        let name = indicator::display_name(indicator);
        match reason {
            IndicatorUnsupportedReason::BasisNotSupported => {
                format!("{name} is not available for {}.", self.basis)
            }
            IndicatorUnsupportedReason::SourceNotSupported => {
                format!(
                    "{name} is not available for {}.",
                    self.data.base_ticker().exchange()
                )
            }
            IndicatorUnsupportedReason::ResolutionNotSupported => match self.basis {
                Basis::Time(timeframe) => {
                    format!("{name} is not available on {timeframe} timeframe.")
                }
                Basis::Tick(_) => format!("{name} is not available for tick charts."),
            },
            IndicatorUnsupportedReason::InsufficientData {
                available,
                required,
            } => {
                format!("{name} needs {required} candles; only {available} available.")
            }
            IndicatorUnsupportedReason::MissingRequiredInput => {
                format!("{name} requires directional trade-volume data.")
            }
        }
    }

    fn oi_enabled_for_base(&self) -> bool {
        self.layout.oi_enabled(self.indicators.instances())
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
            version: 5,
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
            studies: self.studies.clone(),
            study_configs: self.study_configs.clone(),
            indicator_instances: self.indicators.instances().instances().to_vec(),
            indicator_configs: None,
            indicator_colors: Default::default(),
            series: self
                .data
                .series
                .iter()
                .filter(|series| !matches!(series.color(), SeriesColor::Base))
                .map(|series| persisted::SeriesConfig {
                    ticker: series.ticker_info.ticker,
                    color: match series.color() {
                        SeriesColor::Custom(color) => Some(color),
                        SeriesColor::Base | SeriesColor::Accent(_) => None,
                    },
                    accent: match series.color() {
                        SeriesColor::Accent(accent) => Some(accent),
                        SeriesColor::Base | SeriesColor::Custom(_) => None,
                    },
                })
                .collect(),
            ..persisted::Config::default()
        }
    }

    fn drawing_panels_exist(composition: &ChartComposition, object: &DrawingObject) -> bool {
        let panel_exists = |panel_id: PanelId| composition.panel(panel_id).is_some();

        match object {
            DrawingObject::Trendline { start, end } | DrawingObject::Box { start, end } => {
                panel_exists(start.panel_id) && panel_exists(end.panel_id)
            }
            DrawingObject::HorizontalLine { panel_id, .. } => panel_exists(*panel_id),
            DrawingObject::VerticalLine { .. } => true,
        }
    }
}
