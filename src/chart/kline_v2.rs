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
    self as persisted, AnchoredVwapConfig, CandlestickConfig, FetchBatch, FetchPlanInput,
    FetchStage, HorizontalWorld, IndicatorColorConfigStore, IndicatorColorSlot, IndicatorConfig,
    IndicatorEngine, KlineStudy, SeriesColor, desired_fetch_batches as plan_fetch_batches,
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
    OpenIndicatorSettings(KlineIndicator),
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
    SetIndicatorDataConfig(IndicatorConfig),
    SetIndicatorColor {
        indicator: KlineIndicator,
        slot: IndicatorColorSlot,
        accent: Option<data::chart::kline_v2::ThemeAccent>,
        shade: data::config::theme::ThemeColorShade,
    },
    ResetIndicatorSettings(KlineIndicator),
    SetCandlestickConfig(CandlestickConfig),
    ResetCandlestickSettings,
    SetPrimaryMark(MarkKind),
    SetCandlesticksVisible(bool),
    SetFootprintsVisible(bool),
    SetTradeDataEnabled(bool),
    SetFootprintClusterKind(ClusterKind),
    SetIndicatorMark {
        indicator: KlineIndicator,
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
    ToggleIndicatorVisibility(KlineIndicator),
    ToggleIndicator(KlineIndicator),
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
    indicator_colors: IndicatorColorConfigStore,
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
            layout: IndicatorLayout::new(),
            last_tick: Instant::now(),
            cache_rev: 0,
            data: KlineData::new(base_ticker, footprint_tick_multiplier),
            indicators: IndicatorEngine::default(),
            indicator_colors: IndicatorColorConfigStore::default(),
            fetch: FetchCoordinator::new(&[]),
            drawing: DrawingTools::default(),
            studies: Vec::new(),
            study_configs: Vec::new(),
            study_configurator: study::Configurator::new(),
            series_editor: TickerSeriesEditor::default(),
        };

        let _ = chart
            .data
            .add_ticker(base_ticker, chart.basis, &chart.indicators);
        for ticker in tickers.iter().copied() {
            if ticker != base_ticker {
                let _ = chart
                    .data
                    .add_ticker(ticker, chart.basis, &chart.indicators);
            }
        }

        chart.fetch.rebuild_handlers(chart.data.selected_tickers());
        chart
            .data
            .configure_series_for_basis(chart.basis, &chart.indicators);
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
                    KlineWidgetEvent::IndicatorSettings(_value_id) => {
                        let indicator = _value_id;
                        return Some(Action::OpenIndicatorSettings(indicator));
                    }
                    KlineWidgetEvent::IndicatorVisibilityToggled(value_id) => {
                        let indicator = value_id;
                        let _ =
                            self.apply_command(ChartCommand::ToggleIndicatorVisibility(indicator));
                    }
                    KlineWidgetEvent::IndicatorRemove(value_id) => {
                        let indicator = value_id;
                        let _ = self.apply_command(ChartCommand::ToggleIndicator(indicator));
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
            self.indicators.configs(),
            &self.indicator_colors,
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
            self.anchored_vwap_history_requirement(&active_indicators, visible_points);
        let input = FetchPlanInput {
            basis: self.basis,
            horizontal_offset,
            base_ticker: self.data.base_ticker(),
            series: &self.data.series,
            active_indicators: &active_indicators,
            indicator_configs: self.indicators.configs(),
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

    fn anchored_vwap_history_requirement(
        &self,
        active_indicators: &[KlineIndicator],
        visible_points: u64,
    ) -> (Option<UnixMs>, u64) {
        if !active_indicators.contains(&KlineIndicator::AnchoredVwap) {
            return (None, 0);
        }

        let Basis::Time(timeframe) = self.basis else {
            return (None, 0);
        };

        let reference_time = visible_window(
            &self.data.series,
            self.horizontal_world.offset(),
            timeframe,
            visible_points,
        )
        .map(|(start, _)| start)
        .unwrap_or_else(UnixMs::now);
        let config = self.indicators.anchored_vwap_config();
        let history_start = config.history_start(reference_time, data::UserTimezone::Utc);
        let history_bars = config.required_history_bars(timeframe).unwrap_or_default();
        (history_start, history_bars)
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
        let normalized_vwap_config = self
            .indicators
            .anchored_vwap_config()
            .normalized_for_basis(basis);
        if self.indicators.anchored_vwap_config() != normalized_vwap_config {
            let _ = self
                .indicators
                .update_config(IndicatorConfig::AnchoredVwap(normalized_vwap_config));
        }
        self.studies = self.normalize_studies_for_basis(self.studies.iter().copied());
        self.study_configs = self.normalize_studies_for_basis(self.study_configs.iter().copied());
        self.remember_study_configs(&self.studies.clone());

        self.data
            .reset_for_basis(self.basis, preserve_raw_trades, &self.indicators);
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
                            let market = self.data.base_ticker().market_type();
                            for indicator in self.layout.selected_indicators() {
                                if indicator::requires_trade_data(indicator) {
                                    self.layout.toggle_indicator(indicator, market);
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
            ChartCommand::SetIndicatorDataConfig(config) => {
                let requests_history = matches!(config, IndicatorConfig::AnchoredVwap(_));
                let config = match config {
                    IndicatorConfig::AnchoredVwap(config) => {
                        IndicatorConfig::AnchoredVwap(config.normalized_for_basis(self.basis))
                    }
                    config => config,
                };
                let changed = self.indicators.update_config(config);
                if changed {
                    self.data.configure_indicators(self.basis, &self.indicators);
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
                indicator,
                slot,
                accent,
                shade,
            } => {
                let current = self.indicator_colors.config_for(indicator).color_for(slot);
                let changed = self.indicator_colors.update(
                    indicator,
                    slot,
                    data::config::theme::ThemeColor::with_accent(current.role, accent, shade),
                );
                if changed {
                    self.bump_rev();
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::ResetIndicatorSettings(indicator) => {
                let data_changed = self.indicators.reset_config(indicator);
                let color_changed = self.indicator_colors.reset(indicator);
                let mark_changed = self.layout.reset_indicator_mark(indicator);
                let presentation_changed = self.layout.reset_indicator_presentation(indicator);
                let changed = data_changed || color_changed || mark_changed || presentation_changed;

                if data_changed {
                    self.data.configure_indicators(self.basis, &self.indicators);
                }
                if changed {
                    self.bump_rev();
                }

                CommandResult::Changed(changed)
            }
            ChartCommand::SetIndicatorMark { indicator, mark } => {
                let changed = self.layout.set_indicator_mark(indicator, mark);
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
            ChartCommand::ToggleIndicatorVisibility(indicator) => {
                let changed = self.layout.toggle_indicator_visibility(indicator);
                if changed {
                    self.bump_rev();
                }
                CommandResult::Changed(changed)
            }
            ChartCommand::ToggleIndicator(indicator) => {
                let changed = self
                    .layout
                    .toggle_indicator(indicator, self.data.base_ticker().market_type());
                if changed {
                    self.bump_rev();
                }
                if changed && indicator == KlineIndicator::AnchoredVwap {
                    CommandResult::Action(
                        self.request_fetch_action().map(super::Action::RequestFetch),
                    )
                } else {
                    CommandResult::Changed(changed)
                }
            }
            ChartCommand::AddTicker(ticker_info) => {
                if ticker_info != self.data.base_ticker()
                    && self
                        .data
                        .add_ticker(ticker_info, self.basis, &self.indicators)
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
                    self.data
                        .configure_series_for_basis(self.basis, &self.indicators);
                    self.configure_profile_caches();
                    self.bump_rev();
                }
                CommandResult::None
            }
        }
    }

    fn apply_persisted_config(&mut self, config: &persisted::Config) {
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
            let market = self.data.base_ticker().market_type();
            for indicator in self.layout.selected_indicators() {
                if indicator::requires_trade_data(indicator) {
                    self.layout.toggle_indicator(indicator, market);
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

        let _ = self
            .indicators
            .update_config(IndicatorConfig::Rsi(config.rsi));
        let _ = self
            .indicators
            .update_config(IndicatorConfig::MovingAverages(
                data::chart::kline_v2::MovingAveragesCalculationConfig {
                    fast_period: config.moving_averages.fast_period,
                    medium_period: config.moving_averages.medium_period,
                    slow_period: config.moving_averages.slow_period,
                    method: config.moving_averages.method,
                },
            ));
        let _ = self
            .indicators
            .update_config(IndicatorConfig::BollingerBands(
                data::chart::kline_v2::BollingerBandsCalculationConfig {
                    period: config.bollinger_bands.period,
                    method: config.bollinger_bands.method,
                    std_dev_multiplier: config.bollinger_bands.std_dev_multiplier,
                },
            ));
        let _ = self.indicators.update_config(IndicatorConfig::AnchoredVwap(
            config
                .resolved_anchored_vwap_config()
                .normalized_for_basis(self.basis),
        ));
        self.indicator_colors = config.indicator_colors.clone();
        self.data.configure_indicators(self.basis, &self.indicators);
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

    pub fn selected_indicators(&self) -> Vec<KlineIndicator> {
        self.layout.selected_indicators()
    }

    pub fn indicator_is_available(&self, indicator: KlineIndicator) -> bool {
        self.layout.indicator_is_available(
            indicator,
            self.basis,
            self.data.base_ticker(),
            self.data.base_series(),
            self.indicators.configs(),
        )
    }

    pub fn indicator_mark(&self, indicator: KlineIndicator) -> Option<MarkKind> {
        let signed_overlay_input = matches!(indicator, KlineIndicator::Volume)
            && self.data.base_series().is_some_and(|series| {
                series
                    .bars
                    .iter()
                    .any(|bar| bar.volume.buy_sell().is_some())
            });
        self.layout.indicator_mark(indicator, signed_overlay_input)
    }

    pub fn indicator_data_config(&self, indicator: KlineIndicator) -> Option<IndicatorConfig> {
        self.indicators.config_for(indicator)
    }

    pub fn anchored_vwap_config(&self) -> AnchoredVwapConfig {
        self.indicators.anchored_vwap_config()
    }

    pub fn indicator_color_config(
        &self,
        indicator: KlineIndicator,
    ) -> data::chart::kline_v2::IndicatorColorConfig {
        self.indicator_colors.config_for(indicator)
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
                self.indicators.configs(),
            )
            .into_iter()
            .chain(trade_data_disabled)
            .map(|(indicator, reason)| UnavailableIndicator {
                value_id: indicator,
                message: self.unavailable_indicator_message(indicator, reason),
            })
            .collect()
    }

    fn trade_data_unavailable_indicators(
        &self,
    ) -> Vec<(KlineIndicator, IndicatorUnsupportedReason)> {
        self.layout
            .trade_data_unavailable_indicators(self.trade_data_enabled())
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
        self.layout.oi_enabled()
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
            version: 3,
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
            rsi: self.indicators.rsi_config(),
            moving_averages: persisted::MovingAveragesPersistenceConfig {
                fast_period: self
                    .indicators
                    .moving_averages_calculation_config()
                    .fast_period,
                medium_period: self
                    .indicators
                    .moving_averages_calculation_config()
                    .medium_period,
                slow_period: self
                    .indicators
                    .moving_averages_calculation_config()
                    .slow_period,
                method: self.indicators.moving_averages_calculation_config().method,
            },
            bollinger_bands: persisted::BollingerBandsPersistenceConfig {
                period: self.indicators.bollinger_bands_calculation_config().period,
                method: self.indicators.bollinger_bands_calculation_config().method,
                std_dev_multiplier: self
                    .indicators
                    .bollinger_bands_calculation_config()
                    .std_dev_multiplier,
            },
            anchored_vwap: self.indicators.anchored_vwap_config(),
            legacy_indicator_provider_configs: std::collections::BTreeMap::new(),
            indicator_colors: self.indicator_colors.clone(),
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
