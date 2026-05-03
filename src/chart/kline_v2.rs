use crate::connector::fetcher::{FetchRange, FetchSpec, RequestHandler};
use crate::widget::chart::kline::composition::{
    BarMode, ChartComposition, DEFAULT_MIN_PANEL_RATIO, DataSourceId, HistogramMode, LayerDataKind,
    LayerId, LayerSource, MarkKind, PanelId, PanelRole, PanelScaleMode, PanelValueId,
};
use crate::widget::chart::kline::{
    BOLLINGER_LOWER_FIELD_KEY, BOLLINGER_UPPER_FIELD_KEY, DEFAULT_BAR_SPACING_PX, HorizontalScale,
    IndicatorData, KlineSeriesLike, KlineWidget, KlineWidgetEvent, OverlayChannelColorRole,
    OverlayChannelSpec, PanelYViewport, RSI_LOWER_BAND_FIELD_KEY, RSI_SIGNAL_FIELD_KEY,
    RSI_UPPER_BAND_FIELD_KEY,
    coord::{ChartCoord, ChartStepMs, RoundedOffsetUnits},
};

use data::chart::Basis;
use exchange::adapter::{MarketKind, StreamKind};
use exchange::{Kline, OpenInterest, TickerInfo, Timeframe, UnixMs};

use enum_map::{Enum, EnumMap};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::time::Instant;

const DEFAULT_HORIZONTAL_OFFSET_UNITS: f32 = 8.0;
const FETCH_VIEWPORT_WIDTH_ESTIMATE_PX: f32 = 1200.0;
const DEFAULT_FETCH_BARS: u64 = 500;
const MAX_AUTO_INDICATOR_WARMUP_BARS: u64 = 2000;
const SERIES_MAX_BARS: usize = 5000;
const COMPARISON_SOURCE_ID: DataSourceId = DataSourceId::Synthetic("Comparison");

const BOLLINGER_OVERLAY_CHANNELS: [OverlayChannelSpec; 3] = [
    OverlayChannelSpec {
        label: "BB",
        key: None,
        line_width: 1.1,
        color_role: OverlayChannelColorRole::Neutral,
    },
    OverlayChannelSpec {
        label: "U",
        key: Some(BOLLINGER_UPPER_FIELD_KEY),
        line_width: 1.0,
        color_role: OverlayChannelColorRole::Success,
    },
    OverlayChannelSpec {
        label: "L",
        key: Some(BOLLINGER_LOWER_FIELD_KEY),
        line_width: 1.0,
        color_role: OverlayChannelColorRole::Danger,
    },
];

const RSI_OVERLAY_CHANNELS: [OverlayChannelSpec; 4] = [
    OverlayChannelSpec {
        label: "RSI",
        key: None,
        line_width: 1.2,
        color_role: OverlayChannelColorRole::Neutral,
    },
    OverlayChannelSpec {
        label: "S",
        key: Some(RSI_SIGNAL_FIELD_KEY),
        line_width: 1.0,
        color_role: OverlayChannelColorRole::Primary,
    },
    OverlayChannelSpec {
        label: "OB",
        key: Some(RSI_UPPER_BAND_FIELD_KEY),
        line_width: 0.9,
        color_role: OverlayChannelColorRole::Success,
    },
    OverlayChannelSpec {
        label: "OS",
        key: Some(RSI_LOWER_BAND_FIELD_KEY),
        line_width: 0.9,
        color_role: OverlayChannelColorRole::Danger,
    },
];

mod drawing;
mod indicator;

use drawing::{DrawingMessage, DrawingTools, DrawingUpdate};
use indicator::{AvailabilityContext, IndicatorPanelRecipe, SeriesIndicatorData};
pub use indicator::{IndicatorAvailability, RsiConfig};

pub enum Action {
    SeriesColorChanged(TickerInfo, iced::Color),
    SeriesNameChanged(TickerInfo, String),
    RemoveSeries(TickerInfo),
    OpenSeriesEditor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IndicatorPanelBinding {
    AuxPanel {
        panel_id: PanelId,
    },
    PrimaryLayer {
        panel_id: PanelId,
        layer_id: LayerId,
    },
}

#[derive(Debug, Clone)]
pub enum Message {
    Chart(KlineWidgetEvent),
    Sidebar(DrawingMessage),
}

#[derive(Debug, Clone)]
pub struct KlineSeries {
    pub ticker_info: TickerInfo,
    pub name: Option<String>,
    pub bars: Vec<Kline>,
    indicators: SeriesIndicatorData,
}

impl KlineSeries {
    fn new(ticker_info: TickerInfo) -> Self {
        Self {
            ticker_info,
            name: None,
            bars: Vec::new(),
            indicators: SeriesIndicatorData::default(),
        }
    }

    fn oi_timerange(&self) -> Option<(UnixMs, UnixMs)> {
        self.indicators.oi_timerange()
    }

    fn refresh_indicator_inputs(&mut self) {
        self.indicators.refresh_from_bars(&self.bars);
    }
}

impl KlineSeriesLike for KlineSeries {
    fn ticker_info(&self) -> &TickerInfo {
        &self.ticker_info
    }

    fn bars(&self) -> &[Kline] {
        &self.bars
    }

    fn indicator_value(&self, bar: &Kline) -> f32 {
        f32::from(bar.volume.total())
    }

    fn indicator_data_for_panel_value_opt(
        &self,
        panel_value: Option<PanelValueId>,
        bar: &Kline,
    ) -> Option<IndicatorData> {
        let indicator = Self::indicator_for_panel_value(panel_value);
        let value = self.indicators.value_for_indicator(indicator, bar)?;
        let mut data = IndicatorData::scalar(value);

        if matches!(indicator, Some(KlineIndicator::Volume))
            && let Some(signed_overlay) = self.indicators.volume_overlay_for_bar(bar)
        {
            data = data.with_signed_overlay(signed_overlay);
        }

        if matches!(indicator, Some(KlineIndicator::BollingerBands))
            && let Some((upper, lower)) = self.indicators.bollinger_bands_for_bar(bar)
        {
            data = data
                .with_field(BOLLINGER_UPPER_FIELD_KEY, upper)
                .with_field(BOLLINGER_LOWER_FIELD_KEY, lower);
        }

        if matches!(indicator, Some(KlineIndicator::Rsi))
            && let Some(rsi_point) = self.indicators.rsi_fields_for_bar(bar)
        {
            data = data
                .with_field(RSI_UPPER_BAND_FIELD_KEY, rsi_point.upper_band)
                .with_field(RSI_LOWER_BAND_FIELD_KEY, rsi_point.lower_band);

            if let Some(signal) = rsi_point.signal {
                data = data.with_field(RSI_SIGNAL_FIELD_KEY, signal);
            }
        }

        Some(data)
    }

    fn indicator_overlay_channels_for_panel_value(
        &self,
        panel_value: PanelValueId,
    ) -> &'static [OverlayChannelSpec] {
        match panel_value {
            PanelValueId::BollingerBands => &BOLLINGER_OVERLAY_CHANNELS,
            PanelValueId::Rsi => &RSI_OVERLAY_CHANNELS,
            _ => &[],
        }
    }
}

impl KlineSeries {
    fn indicator_for_panel_value(panel_value: Option<PanelValueId>) -> Option<KlineIndicator> {
        match panel_value {
            Some(PanelValueId::Volume) => Some(KlineIndicator::Volume),
            Some(PanelValueId::BollingerBands) => Some(KlineIndicator::BollingerBands),
            Some(PanelValueId::Rsi) => Some(KlineIndicator::Rsi),
            Some(PanelValueId::OpenInterest) => Some(KlineIndicator::OpenInterest),
            Some(PanelValueId::CumulativeVolumeDelta) => {
                Some(KlineIndicator::CumulativeVolumeDelta)
            }
            None => None,
        }
    }
}

pub struct KlineChartV2 {
    basis: Basis,
    timeframe: Timeframe,
    horizontal_scale: HorizontalScale,
    horizontal_offset: f32,
    primary_autoscale: bool,
    panel_y_viewports: Vec<(PanelId, PanelYViewport)>,
    composition: ChartComposition,
    indicator_panels: EnumMap<KlineIndicator, Option<IndicatorPanelBinding>>,
    last_tick: Instant,
    cache_rev: u64,
    base_ticker: TickerInfo,
    selected_tickers: Vec<TickerInfo>,
    series_index: FxHashMap<TickerInfo, usize>,
    comparison_layers: FxHashMap<TickerInfo, LayerId>,
    request_handlers: FxHashMap<TickerInfo, RequestHandler>,
    rsi_config: RsiConfig,
    drawing: DrawingTools,
    pub series: Vec<KlineSeries>,
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
        let timeframe = Self::timeframe_for_basis(basis);
        let composition = ChartComposition::prototype_kline();
        let rsi_config = RsiConfig::default();

        let mut chart = Self {
            basis,
            timeframe,
            horizontal_scale: HorizontalScale::pixels_per_bar(DEFAULT_BAR_SPACING_PX),
            horizontal_offset: DEFAULT_HORIZONTAL_OFFSET_UNITS,
            primary_autoscale: true,
            panel_y_viewports: Vec::new(),
            composition,
            cache_rev: 0,
            base_ticker,
            indicator_panels: EnumMap::default(),
            last_tick: Instant::now(),
            selected_tickers: Vec::new(),
            series_index: FxHashMap::default(),
            comparison_layers: FxHashMap::default(),
            request_handlers: FxHashMap::default(),
            rsi_config,
            drawing: DrawingTools::default(),
            series: Vec::new(),
        };

        let _ = chart.add_ticker_state(base_ticker);

        for ticker in tickers.iter().copied() {
            if ticker != base_ticker {
                let _ = chart.add_ticker_state(ticker);
            }
        }

        chart.sync_primary_panel_comparison_sources();

        chart.install_default_indicator_panels();
        chart
    }

    pub fn update(&mut self, message: Message) -> Option<Action> {
        match message {
            Message::Sidebar(message) => {
                let update = self.drawing.update(message);
                self.apply_drawing_update(update);
            }
            Message::Chart(event) => {
                if let Some(update) = self.drawing.handle_kline_widget_event(&event) {
                    self.apply_drawing_update(update);
                    return None;
                }

                match event {
                    KlineWidgetEvent::HorizontalScaleChanged(scale) => {
                        self.horizontal_scale = scale;
                        self.bump_rev();
                    }
                    KlineWidgetEvent::HorizontalOffsetChanged(offset) => {
                        self.horizontal_offset = offset;
                        self.bump_rev();
                    }
                    KlineWidgetEvent::PrimaryAutoscaleToggled => {
                        let primary_panel_id = self.composition.primary_panel_id()?;

                        let effective_scale = self
                            .composition
                            .panel_effective_scale_mode(primary_panel_id)
                            .unwrap_or(PanelScaleMode::Absolute);

                        if !matches!(effective_scale, PanelScaleMode::PercentFromBase) {
                            self.primary_autoscale = !self.primary_autoscale;

                            if self.primary_autoscale {
                                let _ = self.reset_panel_y_viewport(primary_panel_id);
                            }

                            self.bump_rev();
                        }
                    }
                    KlineWidgetEvent::PrimaryScaleModeCycleRequested => {
                        let primary_panel_id = self.composition.primary_panel_id()?;
                        let panel = self.composition.panel(primary_panel_id)?;

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
                            .composition
                            .set_panel_preferred_scale(primary_panel_id, next_scale)
                        {
                            let now_effective = self
                                .composition
                                .panel_effective_scale_mode(primary_panel_id)
                                .unwrap_or(next_scale);

                            if matches!(now_effective, PanelScaleMode::PercentFromBase)
                                || self.primary_autoscale
                            {
                                let _ = self.reset_panel_y_viewport(primary_panel_id);
                            }

                            self.bump_rev();
                        }
                    }
                    KlineWidgetEvent::PanelYViewportChanged { panel_id, viewport } => {
                        if let Some(primary_id) = self.composition.primary_panel_id()
                            && primary_id == panel_id
                        {
                            if matches!(
                                self.composition.panel_effective_scale_mode(primary_id),
                                Some(PanelScaleMode::PercentFromBase)
                            ) {
                                return None;
                            }

                            if self.primary_autoscale {
                                self.primary_autoscale = false;
                            }
                        }

                        if self.set_panel_y_viewport(panel_id, viewport) {
                            self.bump_rev();
                        }
                    }
                    KlineWidgetEvent::PanelYViewportReset { panel_id } => {
                        if let Some(primary_id) = self.composition.primary_panel_id()
                            && primary_id == panel_id
                        {
                            if matches!(
                                self.composition.panel_effective_scale_mode(primary_id),
                                Some(PanelScaleMode::PercentFromBase)
                            ) {
                                return None;
                            }

                            if !self.primary_autoscale {
                                self.primary_autoscale = true;
                            }
                        }

                        if self.reset_panel_y_viewport(panel_id) {
                            self.bump_rev();
                        }
                    }
                    KlineWidgetEvent::PanelSplitChanged { index, split }
                        if self
                            .composition
                            .set_split(index, split, DEFAULT_MIN_PANEL_RATIO) =>
                    {
                        self.bump_rev();
                    }
                    KlineWidgetEvent::PanelMoveUp { index }
                        if index > 0 && self.composition.move_panel(index, index - 1) =>
                    {
                        self.bump_rev();
                    }
                    KlineWidgetEvent::PanelMoveDown { index } => {
                        let target = index.saturating_add(1);
                        if target < self.composition.panels.len()
                            && self.composition.move_panel(index, target)
                        {
                            self.bump_rev();
                        }
                    }
                    KlineWidgetEvent::PanelSettings { .. } => {
                        // TODO: Hook for upcoming panel settings modal/workflow.
                    }
                    KlineWidgetEvent::PanelClose { index } => {
                        if let Some((panel_id, panel_role)) = self
                            .composition
                            .panels
                            .get(index)
                            .map(|panel| (panel.id, panel.role))
                            && !matches!(panel_role, PanelRole::Primary)
                        {
                            // Remove panel-local drawings while panel coordinates are still valid.
                            let _ = self.drawing.prune_panel_drawings(panel_id);

                            if self.composition.remove_panel(panel_id) {
                                let _ = self.reset_panel_y_viewport(panel_id);
                                self.prune_stale_indicator_panel_bindings();
                                self.bump_rev();
                            }
                        }
                    }
                    KlineWidgetEvent::TickerSettings(_ticker) => {
                        // Hook for ticker-specific settings editor.
                    }
                    KlineWidgetEvent::TickerRemove(ticker) if ticker != self.base_ticker => {
                        return Some(Action::RemoveSeries(ticker));
                    }
                    KlineWidgetEvent::XAxisDoubleClick => {
                        self.horizontal_scale =
                            HorizontalScale::pixels_per_bar(DEFAULT_BAR_SPACING_PX);
                        self.horizontal_offset = DEFAULT_HORIZONTAL_OFFSET_UNITS;
                        self.bump_rev();
                    }
                    _ => {}
                }
            }
        }
        None
    }

    pub fn view(&self, timezone: data::UserTimezone) -> iced::Element<'_, Message> {
        if self.series.iter().all(|series| series.bars.is_empty()) {
            return iced::widget::center(iced::widget::text("Waiting for data...").size(16)).into();
        }

        let drawing_sidebar = self.drawing.view(Message::Sidebar);
        let chart: iced::Element<_> =
            KlineWidget::new(&self.series, self.timeframe, &self.composition)
                .with_basis(self.basis)
                .with_horizontal_scale(self.horizontal_scale)
                .with_horizontal_offset(self.horizontal_offset)
                .with_primary_autoscale(self.primary_autoscale)
                .with_panel_y_viewports(&self.panel_y_viewports)
                .with_active_drawing_tool(self.drawing.active_tool())
                .with_drawings(self.drawing.drawings())
                .with_selected_drawing(self.drawing.selected_drawing())
                .with_drawing_draft(self.drawing.drawing_draft())
                .with_timezone(timezone)
                .version(self.cache_rev)
                .into();

        let chart = iced::widget::container(chart.map(Message::Chart))
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .padding(1);

        iced::widget::row![drawing_sidebar.padding(4), chart].into()
    }

    pub fn invalidate(&mut self, now: Option<Instant>) -> Option<super::Action> {
        if let Some(ts) = now {
            self.last_tick = ts;
        }

        self.bump_rev();

        let reqs = self.collect_fetch_reqs(self.desired_fetch_batches(self.horizontal_offset));
        self.fetch_action(reqs)
    }

    pub fn basis(&self) -> Basis {
        self.basis
    }

    pub fn set_primary_scale_mode(&mut self, scale: PanelScaleMode) -> bool {
        let Some(primary_panel_id) = self.composition.primary_panel_id() else {
            return false;
        };

        if self
            .composition
            .set_panel_preferred_scale(primary_panel_id, scale)
        {
            if matches!(
                self.composition
                    .panel_effective_scale_mode(primary_panel_id),
                Some(PanelScaleMode::PercentFromBase)
            ) {
                self.primary_autoscale = true;
                let _ = self.reset_panel_y_viewport(primary_panel_id);
            }
            self.bump_rev();
            true
        } else {
            false
        }
    }

    pub fn rsi_config(&self) -> RsiConfig {
        self.rsi_config
    }

    pub fn set_rsi_config(&mut self, config: RsiConfig) -> Option<super::Action> {
        let normalized = config.normalized();
        if self.rsi_config == normalized {
            return None;
        }

        self.rsi_config = normalized;

        for series in &mut self.series {
            if series.indicators.set_rsi_config(normalized) {
                series.refresh_indicator_inputs();
            }
        }

        self.enforce_indicator_availability();
        self.bump_rev();

        let reqs = self.collect_fetch_reqs(self.desired_fetch_batches(self.horizontal_offset));
        self.fetch_action(reqs)
    }

    pub fn set_panel_mark(&mut self, panel_index: usize, mark: MarkKind) -> bool {
        let Some((panel_id, layer_id)) = self.panel_base_layer_ids(panel_index) else {
            return false;
        };

        if self
            .composition
            .set_panel_layer_mark(panel_id, layer_id, mark)
        {
            self.bump_rev();
            true
        } else {
            false
        }
    }

    pub fn set_panel_data_kind(&mut self, panel_index: usize, data_kind: LayerDataKind) -> bool {
        let Some((panel_id, layer_id)) = self.panel_base_layer_ids(panel_index) else {
            return false;
        };

        if self
            .composition
            .set_panel_layer_data_kind(panel_id, layer_id, data_kind)
        {
            self.bump_rev();
            true
        } else {
            false
        }
    }

    pub fn set_panel_bar_mode(&mut self, panel_index: usize, mode: BarMode) -> bool {
        let Some((panel_id, layer_id)) = self.panel_base_layer_ids(panel_index) else {
            return false;
        };

        if self
            .composition
            .set_panel_layer_bar_mode(panel_id, layer_id, mode)
        {
            self.bump_rev();
            true
        } else {
            false
        }
    }

    pub fn set_panel_histogram_mode(&mut self, panel_index: usize, mode: HistogramMode) -> bool {
        self.set_panel_bar_mode(panel_index, BarMode::Histogram(mode))
    }

    pub fn toggle_indicator(&mut self, indicator: KlineIndicator) -> bool {
        let changed = if self.indicator_panels[indicator].is_some() {
            self.disable_indicator(indicator)
        } else {
            self.enable_indicator(indicator)
        };

        if changed {
            self.bump_rev();
        }

        changed
    }

    pub fn ticker_info(&self) -> TickerInfo {
        self.base_ticker
    }

    pub fn selected_tickers(&self) -> &[TickerInfo] {
        &self.selected_tickers
    }

    pub fn add_ticker(&mut self, ticker_info: &TickerInfo) -> Vec<StreamKind> {
        if *ticker_info == self.base_ticker {
            return self.streams_for_all();
        }

        if self.add_ticker_state(*ticker_info) {
            self.sync_primary_panel_comparison_sources();
            self.bump_rev();
        }

        self.streams_for_all()
    }

    pub fn remove_ticker(&mut self, ticker_info: &TickerInfo) -> Vec<StreamKind> {
        if *ticker_info == self.base_ticker {
            return self.streams_for_all();
        }

        if self.remove_ticker_state(*ticker_info) {
            self.sync_primary_panel_comparison_sources();
            self.bump_rev();
        }

        self.streams_for_all()
    }

    pub fn set_series_color(&mut self, _ticker_info: TickerInfo, _color: iced::Color) -> bool {
        todo!()
    }

    pub fn set_series_name(&mut self, ticker_info: TickerInfo, name: String) -> bool {
        if let Some(idx) = self.series_index.get(&ticker_info).copied() {
            self.series[idx].name = Some(name);
            self.bump_rev();
            true
        } else {
            false
        }
    }

    pub fn last_update(&self) -> Instant {
        self.last_tick
    }

    fn apply_drawing_update(&mut self, update: DrawingUpdate) {
        if update.should_bump() {
            self.bump_rev();
        }
    }

    pub fn insert_history(
        &mut self,
        req_id: uuid::Uuid,
        ticker_info: TickerInfo,
        klines: &[Kline],
    ) {
        let Some(idx) = self.series_index.get(&ticker_info).copied() else {
            if let Some(handler) = self.request_handlers.get_mut(&ticker_info) {
                handler.mark_failed(req_id, "ticker mismatch".to_string());
            }
            return;
        };

        let incoming = self.klines_to_bars(klines);

        if incoming.is_empty() {
            if let Some(handler) = self.request_handlers.get_mut(&ticker_info) {
                handler.mark_failed(req_id, "No data received".to_string());
            }
            return;
        }

        merge_bars(&mut self.series[idx].bars, incoming);
        trim_bars(&mut self.series[idx].bars);
        self.series[idx].refresh_indicator_inputs();
        self.enforce_indicator_availability();

        if let Some(handler) = self.request_handlers.get_mut(&ticker_info) {
            handler.mark_completed(req_id);
        }
        self.bump_rev();
    }

    pub fn insert_snapshot(&mut self, ticker_info: TickerInfo, klines: &[Kline]) {
        let Some(idx) = self.series_index.get(&ticker_info).copied() else {
            return;
        };

        let incoming = self.klines_to_bars(klines);

        if incoming.is_empty() {
            return;
        }

        merge_bars(&mut self.series[idx].bars, incoming);
        trim_bars(&mut self.series[idx].bars);
        self.series[idx].refresh_indicator_inputs();
        self.enforce_indicator_availability();
        self.bump_rev();
    }

    pub fn update_latest_kline(&mut self, ticker_info: &TickerInfo, kline: &Kline) {
        let Some(idx) = self.series_index.get(ticker_info).copied() else {
            return;
        };

        let new_bar = Self::kline_to_bar(kline, self.basis, self.timeframe);

        let series = &mut self.series[idx];

        if let Some(last) = series.bars.last_mut() {
            if last.time == new_bar.time {
                *last = new_bar;
            } else if new_bar.time > last.time {
                series.bars.push(new_bar);
            }
        } else {
            series.bars.push(new_bar);
        }

        trim_bars(&mut series.bars);
        series.refresh_indicator_inputs();
        self.enforce_indicator_availability();
        self.bump_rev();
    }

    pub fn set_basis(&mut self, basis: Basis) -> Option<super::Action> {
        self.basis = basis;
        self.timeframe = Self::timeframe_for_basis(basis);

        for series in &mut self.series {
            series.bars.clear();
            series.indicators.clear();
        }

        self.enforce_indicator_availability();

        self.rebuild_handlers();
        self.bump_rev();

        let reqs = self.collect_fetch_reqs(self.desired_fetch_batches(self.horizontal_offset));
        self.fetch_action(reqs)
    }

    fn panel_y_viewport_index(&self, panel_id: PanelId) -> Option<usize> {
        self.panel_y_viewports
            .iter()
            .position(|(id, _)| *id == panel_id)
    }

    fn set_panel_y_viewport(&mut self, panel_id: PanelId, viewport: PanelYViewport) -> bool {
        if let Some(index) = self.panel_y_viewport_index(panel_id) {
            if self.panel_y_viewports[index].1 == viewport {
                return false;
            }

            self.panel_y_viewports[index].1 = viewport;
            return true;
        }

        self.panel_y_viewports.push((panel_id, viewport));
        true
    }

    fn reset_panel_y_viewport(&mut self, panel_id: PanelId) -> bool {
        let Some(index) = self.panel_y_viewport_index(panel_id) else {
            return false;
        };

        self.panel_y_viewports.remove(index);
        true
    }

    fn prune_stale_panel_y_viewports(&mut self) {
        self.panel_y_viewports
            .retain(|(panel_id, _)| self.composition.panel(*panel_id).is_some());
    }

    fn bump_rev(&mut self) {
        self.prune_stale_panel_y_viewports();
        self.cache_rev = self.cache_rev.wrapping_add(1);
    }

    fn install_default_indicator_panels(&mut self) {
        for indicator in KlineIndicator::for_market(self.base_ticker.market_type()) {
            let _ = self.enable_indicator(*indicator);
        }
    }

    fn enable_indicator(&mut self, indicator: KlineIndicator) -> bool {
        if self.indicator_panels[indicator].is_some() {
            return false;
        }

        if !self.is_indicator_available(indicator) {
            return false;
        }

        let recipe = indicator::panel_recipe(indicator);

        let binding = match recipe {
            IndicatorPanelRecipe::AuxPanel {
                panel_title,
                layer_name,
                source,
                data_kind,
                mark,
                axis,
                value_precision,
                value_label_policy,
                preferred_scale,
            } => {
                let layer = self.composition.new_layer(
                    layer_name,
                    LayerSource::RawKline { source },
                    data_kind,
                    mark,
                    axis,
                );

                let panel_id = self.composition.add_aux_panel(
                    panel_title,
                    vec![layer],
                    Some(value_precision),
                    value_label_policy,
                );
                let _ = self
                    .composition
                    .set_panel_preferred_scale(panel_id, preferred_scale);
                let _ = self.composition.set_panel_value_id(
                    panel_id,
                    Some(Self::panel_value_id_for_indicator(indicator)),
                );
                IndicatorPanelBinding::AuxPanel { panel_id }
            }
            IndicatorPanelRecipe::PrimaryOverlay {
                layer_name,
                source,
                value_id,
                data_kind,
                mark,
                axis,
            } => {
                let Some(primary_panel_id) = self.composition.primary_panel_id() else {
                    return false;
                };

                let layer = self.composition.new_layer(
                    layer_name,
                    LayerSource::RawIndicator { source, value_id },
                    data_kind,
                    mark,
                    axis,
                );
                let layer_id = layer.id;

                if !self.composition.add_layer_to_panel(primary_panel_id, layer) {
                    return false;
                }

                IndicatorPanelBinding::PrimaryLayer {
                    panel_id: primary_panel_id,
                    layer_id,
                }
            }
        };

        self.indicator_panels[indicator] = Some(binding);
        true
    }

    fn is_indicator_available(&self, indicator: KlineIndicator) -> bool {
        !matches!(
            self.indicator_availability(indicator),
            IndicatorAvailability::Unsupported(_)
        )
    }

    fn indicator_availability(&self, indicator: KlineIndicator) -> IndicatorAvailability {
        indicator::availability(
            indicator,
            AvailabilityContext {
                basis: self.basis,
                timeframe: self.timeframe,
                base_ticker: self.base_ticker,
            },
            self.series.iter().map(|series| &series.indicators),
        )
    }

    fn disable_indicator(&mut self, indicator: KlineIndicator) -> bool {
        let Some(binding) = self.indicator_panels[indicator].take() else {
            return false;
        };

        match binding {
            IndicatorPanelBinding::AuxPanel { panel_id } => {
                if self.composition.remove_panel(panel_id) {
                    true
                } else {
                    self.indicator_panels[indicator] = Some(binding);
                    false
                }
            }
            IndicatorPanelBinding::PrimaryLayer { panel_id, layer_id } => {
                if self.composition.remove_layer_from_panel(panel_id, layer_id) {
                    true
                } else {
                    self.indicator_panels[indicator] = Some(binding);
                    false
                }
            }
        }
    }

    fn enforce_indicator_availability(&mut self) {
        let mut changed = false;

        for &indicator in indicator::all_indicators() {
            if self.indicator_panels[indicator].is_none() {
                continue;
            }

            if matches!(
                self.indicator_availability(indicator),
                IndicatorAvailability::Unsupported(_)
            ) {
                changed |= self.disable_indicator(indicator);
            }
        }

        if changed {
            self.bump_rev();
        }
    }

    fn prune_stale_indicator_panel_bindings(&mut self) {
        for indicator in indicator::all_indicators().iter().copied() {
            if let Some(binding) = self.indicator_panels[indicator] {
                let stale = match binding {
                    IndicatorPanelBinding::AuxPanel { panel_id } => {
                        self.composition.panel(panel_id).is_none()
                    }
                    IndicatorPanelBinding::PrimaryLayer { panel_id, layer_id } => self
                        .composition
                        .panel(panel_id)
                        .map(|panel| !panel.layers.iter().any(|layer| layer.id == layer_id))
                        .unwrap_or(true),
                };

                if stale {
                    self.indicator_panels[indicator] = None;
                }
            }
        }
    }

    fn panel_value_id_for_indicator(indicator: KlineIndicator) -> PanelValueId {
        match indicator {
            KlineIndicator::Volume => PanelValueId::Volume,
            KlineIndicator::BollingerBands => PanelValueId::BollingerBands,
            KlineIndicator::Rsi => PanelValueId::Rsi,
            KlineIndicator::OpenInterest => PanelValueId::OpenInterest,
            KlineIndicator::CumulativeVolumeDelta => PanelValueId::CumulativeVolumeDelta,
        }
    }

    fn panel_base_layer_ids(&self, panel_index: usize) -> Option<(PanelId, LayerId)> {
        let panel = self.composition.panels.get(panel_index)?;
        let layer_id = panel
            .base_layer
            .or_else(|| panel.layers.first().map(|layer| layer.id))?;

        Some((panel.id, layer_id))
    }

    fn timeframe_for_basis(basis: Basis) -> Timeframe {
        match basis {
            Basis::Time(tf) => tf,
            // Keep widget math operational until tick-domain widget path lands.
            Basis::Tick(_) => Timeframe::MS100,
        }
    }

    fn kline_to_bar(kline: &Kline, basis: Basis, timeframe: Timeframe) -> Kline {
        let mut adjusted = *kline;
        adjusted.time = match basis {
            Basis::Time(_) => kline.time.floor_to(timeframe),
            Basis::Tick(_) => kline.time,
        };
        adjusted
    }

    fn klines_to_bars(&self, klines: &[Kline]) -> Vec<Kline> {
        let mut incoming: Vec<Kline> = klines
            .iter()
            .map(|kline| Self::kline_to_bar(kline, self.basis, self.timeframe))
            .collect();

        incoming.sort_by_key(|bar| bar.time);
        incoming.dedup_by_key(|bar| bar.time);
        incoming
    }

    fn queue_kline_fetch(
        &mut self,
        ticker_info: TickerInfo,
        range: FetchRange,
        out: &mut Vec<(uuid::Uuid, FetchRange, Option<StreamKind>)>,
    ) {
        let handler = self.request_handlers.entry(ticker_info).or_default();

        if let Ok(Some(req_id)) = handler.add_request(range) {
            out.push((
                req_id,
                range,
                Some(StreamKind::Kline {
                    ticker_info,
                    timeframe: self.timeframe,
                }),
            ));
        }
    }

    fn collect_fetch_reqs(
        &mut self,
        batches: Vec<(FetchRange, Vec<TickerInfo>)>,
    ) -> Vec<(uuid::Uuid, FetchRange, Option<StreamKind>)> {
        let mut reqs = Vec::new();

        for (range, tickers) in batches {
            for ticker_info in tickers {
                self.queue_kline_fetch(ticker_info, range, &mut reqs);
            }
        }

        reqs
    }

    fn fetch_action(
        &self,
        reqs: Vec<(uuid::Uuid, FetchRange, Option<StreamKind>)>,
    ) -> Option<super::Action> {
        if reqs.is_empty() {
            None
        } else {
            let specs = reqs
                .into_iter()
                .map(FetchSpec::from)
                .collect::<Vec<FetchSpec>>();
            Some(super::Action::RequestFetch(specs))
        }
    }

    fn dt_ms_est(&self) -> u64 {
        self.timeframe.to_milliseconds().max(1)
    }

    fn active_indicator_kline_warmup_bars(&self) -> u64 {
        indicator::all_indicators()
            .iter()
            .copied()
            .filter(|indicator| self.indicator_panels[*indicator].is_some())
            .filter_map(|indicator| indicator::kline_warmup_bars(indicator, self.rsi_config))
            .filter(|bars| *bars <= MAX_AUTO_INDICATOR_WARMUP_BARS)
            .max()
            .unwrap_or(0)
    }

    fn align_floor(&self, ts: UnixMs) -> UnixMs {
        ts.floor_to(self.timeframe)
    }

    fn estimate_visible_points_for_fetch(&self) -> i64 {
        let spacing = self.horizontal_scale.as_pixels_per_bar().max(1e-3);
        ((FETCH_VIEWPORT_WIDTH_ESTIMATE_PX / spacing).floor() as i64).max(2)
    }

    fn compute_visible_window(&self, horizontal_offset: f32) -> Option<(UnixMs, UnixMs)> {
        let dt = ChartStepMs::from_u64(self.dt_ms_est());

        let max_seen = self
            .series
            .iter()
            .flat_map(|series| series.bars.iter())
            .map(|bar| bar.time)
            .max()?;

        let max_seen = ChartCoord::from_unix_ms(max_seen);
        let span_units = self.estimate_visible_points_for_fetch().saturating_sub(1);
        let right_units = RoundedOffsetUnits::from_f32(horizontal_offset)?;

        let right_offset = right_units.saturating_scale(dt);
        let right = max_seen.saturating_add_i64(right_offset);

        let span_offset = span_units.saturating_mul(dt.get());
        let left = right.saturating_sub_i64(span_offset);

        Some((
            left.to_unix_ms_non_negative(),
            right.to_unix_ms_non_negative(),
        ))
    }

    fn desired_fetch_batches(&self, horizontal_offset: f32) -> Vec<(FetchRange, Vec<TickerInfo>)> {
        let dt = self.dt_ms_est();
        let visible_points = self.estimate_visible_points_for_fetch().max(0) as u64;
        let indicator_warmup_bars = self.active_indicator_kline_warmup_bars();
        let fetch_bars =
            DEFAULT_FETCH_BARS.max(visible_points.saturating_add(indicator_warmup_bars));
        let span = fetch_bars.saturating_mul(dt);
        let last_closed = self.align_floor(UnixMs::now());

        let mut batches: Vec<(FetchRange, Vec<TickerInfo>)> = Vec::new();

        let mut empty_tickers: Vec<TickerInfo> = Vec::new();
        for series in &self.series {
            if series.bars.is_empty() {
                empty_tickers.push(series.ticker_info);
            }
        }

        if !empty_tickers.is_empty() {
            let end = last_closed;
            let start = end.saturating_sub(span);
            batches.push((FetchRange::Kline(start, end), empty_tickers));
        }

        if let Some((window_min, _window_max)) = self.compute_visible_window(horizontal_offset) {
            let warmup_start = if indicator_warmup_bars > 0 {
                self.align_floor(
                    window_min.saturating_sub(indicator_warmup_bars.saturating_mul(dt)),
                )
            } else {
                window_min
            };

            let target_min = warmup_start.min(window_min);

            for series in &self.series {
                if let Some(series_min) = series.bars.first().map(|bar| bar.time)
                    && target_min < series_min
                {
                    let end = self.align_floor(series_min);
                    let start = end.saturating_sub(span);
                    batches.push((FetchRange::Kline(start, end), vec![series.ticker_info]));
                }
            }

            if self.oi_enabled_for_base()
                && matches!(
                    self.indicator_availability(KlineIndicator::OpenInterest),
                    IndicatorAvailability::Available
                )
                && let Some(base_series) = self.base_series()
                && let Some(kline_latest) = base_series.bars.last().map(|bar| bar.time)
            {
                let visible_window = self.compute_visible_window(horizontal_offset);
                let visible_earliest = visible_window.map(|(start, _)| start).unwrap_or(window_min);
                let visible_span = visible_window
                    .map(|(start, end)| end.as_u64().saturating_sub(start.as_u64()))
                    .unwrap_or(span);
                let prefetch_earliest = visible_earliest.saturating_sub(visible_span);

                match base_series.oi_timerange() {
                    Some((oi_earliest, oi_latest)) => {
                        if visible_earliest < oi_earliest {
                            batches.push((
                                FetchRange::OpenInterest(prefetch_earliest, oi_earliest),
                                vec![self.base_ticker],
                            ));
                        }

                        if oi_latest < kline_latest {
                            let start = oi_latest.max(prefetch_earliest);
                            if start < kline_latest {
                                batches.push((
                                    FetchRange::OpenInterest(start, kline_latest),
                                    vec![self.base_ticker],
                                ));
                            }
                        }
                    }
                    None => {
                        if prefetch_earliest < kline_latest {
                            batches.push((
                                FetchRange::OpenInterest(prefetch_earliest, kline_latest),
                                vec![self.base_ticker],
                            ));
                        }
                    }
                }
            }
        }

        batches
    }

    fn oi_enabled_for_base(&self) -> bool {
        self.indicator_panels[KlineIndicator::OpenInterest].is_some()
    }

    fn base_series(&self) -> Option<&KlineSeries> {
        let idx = self.series_index.get(&self.base_ticker).copied()?;
        self.series.get(idx)
    }

    pub fn insert_open_interest(
        &mut self,
        req_id: Option<uuid::Uuid>,
        ticker_info: TickerInfo,
        data: &[OpenInterest],
    ) {
        let Some(idx) = self.series_index.get(&ticker_info).copied() else {
            if let Some(req_id) = req_id
                && let Some(handler) = self.request_handlers.get_mut(&ticker_info)
            {
                handler.mark_failed(req_id, "ticker mismatch".to_string());
            }
            return;
        };

        if let Some(req_id) = req_id
            && let Some(handler) = self.request_handlers.get_mut(&ticker_info)
        {
            if data.is_empty() {
                handler.mark_failed(req_id, "No data received".to_string());
            } else {
                handler.mark_completed(req_id);
            }
        }

        if data.is_empty() {
            return;
        }

        let series = &mut self.series[idx];
        series
            .indicators
            .insert_open_interest_batch(data, self.basis, self.timeframe);
        self.bump_rev();
    }

    fn streams_for_all(&self) -> Vec<StreamKind> {
        self.selected_tickers
            .iter()
            .copied()
            .map(|ticker_info| StreamKind::Kline {
                ticker_info,
                timeframe: self.timeframe,
            })
            .collect()
    }

    fn add_ticker_state(&mut self, ticker_info: TickerInfo) -> bool {
        if self.selected_tickers.contains(&ticker_info) {
            return false;
        }

        let idx = self.series.len();
        let mut series = KlineSeries::new(ticker_info);
        let _ = series.indicators.set_rsi_config(self.rsi_config);
        self.series.push(series);
        self.series_index.insert(ticker_info, idx);
        self.request_handlers
            .insert(ticker_info, RequestHandler::default());
        self.selected_tickers.push(ticker_info);

        true
    }

    fn remove_ticker_state(&mut self, ticker_info: TickerInfo) -> bool {
        let Some(idx) = self.series_index.remove(&ticker_info) else {
            return false;
        };

        self.series.remove(idx);
        self.selected_tickers
            .retain(|ticker| *ticker != ticker_info);
        self.request_handlers.remove(&ticker_info);
        self.rebuild_series_index();

        true
    }

    fn rebuild_series_index(&mut self) {
        self.series_index.clear();
        for (idx, series) in self.series.iter().enumerate() {
            self.series_index.insert(series.ticker_info, idx);
        }
    }

    fn rebuild_handlers(&mut self) {
        self.request_handlers.clear();

        for &ticker in &self.selected_tickers {
            self.request_handlers
                .insert(ticker, RequestHandler::default());
        }
    }

    fn sync_primary_panel_comparison_sources(&mut self) {
        let Some(primary_panel_id) = self.composition.primary_panel_id() else {
            return;
        };

        let stale_layers: Vec<(TickerInfo, LayerId)> = self
            .comparison_layers
            .iter()
            .filter_map(|(ticker, layer_id)| {
                (!self.selected_tickers.contains(ticker) || *ticker == self.base_ticker)
                    .then_some((*ticker, *layer_id))
            })
            .collect();

        for (ticker, layer_id) in stale_layers {
            let _ = self
                .composition
                .remove_layer_from_panel(primary_panel_id, layer_id);
            self.comparison_layers.remove(&ticker);
        }

        for &ticker in self
            .selected_tickers
            .iter()
            .filter(|ticker| **ticker != self.base_ticker)
        {
            if self.comparison_layers.contains_key(&ticker) {
                continue;
            }

            let layer_name = ticker.ticker.symbol_and_exchange_string();

            if let Some(layer_id) = self.composition.add_comparison_source_to_panel(
                primary_panel_id,
                COMPARISON_SOURCE_ID,
                layer_name,
            ) {
                self.comparison_layers.insert(ticker, layer_id);
            }
        }
    }
}

fn merge_bars(dst: &mut Vec<Kline>, mut incoming: Vec<Kline>) {
    if incoming.is_empty() {
        return;
    }

    if dst.is_empty() {
        *dst = incoming;
        return;
    }

    incoming.sort_by_key(|bar| bar.time);
    incoming.dedup_by_key(|bar| bar.time);

    let mut i = 0usize;
    let mut j = 0usize;
    let mut merged = Vec::with_capacity(dst.len() + incoming.len());

    while i < dst.len() && j < incoming.len() {
        let a = dst[i];
        let b = incoming[j];

        if a.time < b.time {
            merged.push(a);
            i += 1;
        } else if b.time < a.time {
            merged.push(b);
            j += 1;
        } else {
            merged.push(b);
            i += 1;
            j += 1;
        }
    }

    if i < dst.len() {
        merged.extend_from_slice(&dst[i..]);
    }

    if j < incoming.len() {
        merged.extend_from_slice(&incoming[j..]);
    }

    merged.dedup_by_key(|bar| bar.time);
    *dst = merged;
}

fn trim_bars(bars: &mut Vec<Kline>) {
    if bars.len() > SERIES_MAX_BARS {
        let to_drop = bars.len() - SERIES_MAX_BARS;
        bars.drain(0..to_drop);
    }
}

pub trait Indicator: PartialEq + Display + 'static {
    fn for_market(market: MarketKind) -> &'static [Self]
    where
        Self: Sized;
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize, Eq, Enum)]
pub enum KlineIndicator {
    Volume,
    BollingerBands,
    Rsi,
    OpenInterest,
    CumulativeVolumeDelta,
}

impl Indicator for KlineIndicator {
    fn for_market(market: MarketKind) -> &'static [Self] {
        indicator::indicators_for_market(market)
    }
}

impl Display for KlineIndicator {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", indicator::display_name(*self))
    }
}
