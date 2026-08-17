use crate::connector::fetcher::{FetchRange, FetchSpec, RequestHandler, is_trade_fetch_enabled};
use crate::widget::chart::kline::PanelInteraction;
use crate::widget::chart::kline::composition::{
    AxisBinding, BarMode, ChartComposition, DEFAULT_MIN_PANEL_RATIO, DataSourceId, HistogramMode,
    LayerDataKind, LayerId, LayerSource, LayerStyle, MarkKind, PanelComparisonPolicy, PanelId,
    PanelRole, PanelScaleMode, PanelValueId, PanelValueLabelMode, PanelValueLabelPolicy,
    PanelValuePrecision,
};
use crate::widget::chart::kline::drawing::{
    DrawingAnchor, DrawingEntity, DrawingId, DrawingObject, DrawingStyle, DrawingTool,
};
use crate::widget::chart::kline::{
    BOLLINGER_LOWER_FIELD_KEY, BOLLINGER_UPPER_FIELD_KEY, DEFAULT_BAR_SPACING_PX, HorizontalWorld,
    IndicatorData, KlineSeriesLike, KlineWidget, KlineWidgetEvent, OverlayChannelColorRole,
    OverlayChannelSpec, PanelYViewport, RSI_LOWER_BAND_FIELD_KEY, RSI_SIGNAL_FIELD_KEY,
    RSI_UPPER_BAND_FIELD_KEY, UnavailableIndicator,
    coord::{ChartCoord, ChartStepMs},
};

use data::aggr::ticks::TickAggr;
use data::chart::Basis;
use data::chart::kline::KlineTrades;
use data::chart::kline_v2 as persisted;
use exchange::adapter::{MarketKind, StreamKind};
use exchange::{
    Kline, OpenInterest, TickMultiplier, TickerInfo, Timeframe, Trade, UnixMs, unit::PriceStep,
};

use enum_map::{Enum, EnumMap};
use iced::task::Handle;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Display;
use std::time::Instant;

const DEFAULT_HORIZONTAL_OFFSET_UNITS: f32 = 8.0;
const FETCH_VIEWPORT_WIDTH_ESTIMATE_PX: f32 = 1200.0;
const DEFAULT_FETCH_BARS: u64 = 500;
const MAX_AUTO_INDICATOR_WARMUP_BARS: u64 = 2000;
const SERIES_MAX_BARS: usize = 5000;
const COMPARISON_SOURCE_ID: DataSourceId = DataSourceId::Synthetic("Comparison");
pub const DEFAULT_FOOTPRINT_TICK_MULTIPLIER: TickMultiplier = TickMultiplier(10);

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
use indicator::{
    AvailabilityContext, IndicatorPanelRecipe, IndicatorUnsupportedReason, SeriesIndicatorData,
};
pub use indicator::{IndicatorAvailability, RsiConfig, RsiSmoothing};

pub enum Action {
    SeriesColorChanged(TickerInfo, iced::Color),
    SeriesNameChanged(TickerInfo, String),
    RemoveSeries(TickerInfo),
    OpenSeriesEditor,
    RequestFetch(Vec<FetchSpec>),
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
    indicators: Option<SeriesIndicatorData>,
    footprints: Option<BTreeMap<UnixMs, KlineTrades>>,
    footprint_tick_size: PriceStep,
    raw_trades: Vec<Trade>,
    tick_times: Vec<UnixMs>,
    tick_aggr: Option<TickAggr>,
}

impl KlineSeries {
    fn new(ticker_info: TickerInfo) -> Self {
        Self {
            ticker_info,
            name: None,
            bars: Vec::new(),
            indicators: None,
            footprints: None,
            footprint_tick_size: ticker_info.min_ticksize.into(),
            raw_trades: Vec::new(),
            tick_times: Vec::new(),
            tick_aggr: None,
        }
    }

    fn new_base(ticker_info: TickerInfo) -> Self {
        let mut series = Self::new(ticker_info);
        series.indicators = Some(SeriesIndicatorData::default());
        series.footprints = Some(BTreeMap::new());
        series
    }

    fn oi_timerange(&self) -> Option<(UnixMs, UnixMs)> {
        self.indicators.as_ref()?.oi_timerange()
    }

    fn refresh_indicator_inputs(&mut self) {
        if let Some(indicators) = self.indicators.as_mut() {
            indicators.refresh_from_bars(&self.bars);
        }
    }

    fn enable_tick_aggregation(
        &mut self,
        interval: data::aggr::TickCount,
        footprint_tick_size: PriceStep,
    ) {
        self.bars.clear();
        if let Some(indicators) = self.indicators.as_mut() {
            indicators.clear();
        }
        self.tick_aggr = Some(TickAggr::new(
            interval,
            footprint_tick_size,
            &self.raw_trades,
        ));
        self.rebuild_bars_from_tick_aggregation();
    }

    fn disable_tick_aggregation(&mut self) {
        self.tick_aggr = None;
        self.tick_times.clear();
    }

    fn reset_for_basis(&mut self, preserve_raw_trades: bool) {
        self.bars.clear();
        if let Some(indicators) = self.indicators.as_mut() {
            indicators.clear();
        }
        if let Some(footprints) = self.footprints.as_mut() {
            footprints.clear();
        }
        self.disable_tick_aggregation();

        if !preserve_raw_trades {
            self.raw_trades.clear();
        }
    }

    fn insert_tick_trades(&mut self, trades: &[Trade]) {
        if self.tick_aggr.is_none() {
            return;
        }

        self.raw_trades.extend_from_slice(trades);

        let tick_aggr = self
            .tick_aggr
            .as_mut()
            .expect("tick aggregation was checked above");

        tick_aggr.insert_trades(trades);
        self.rebuild_bars_from_tick_aggregation();
    }

    fn insert_time_trades(&mut self, trades: &[Trade], timeframe: Timeframe) {
        let Some(footprints) = self.footprints.as_mut() else {
            return;
        };

        let mut updated_times = Vec::new();

        for trade in trades {
            let bar_time = trade.time.floor_to(timeframe);
            if !updated_times.contains(&bar_time) {
                updated_times.push(bar_time);
            }

            footprints
                .entry(bar_time)
                .or_insert_with(KlineTrades::new)
                .add_trade_to_nearest_bin(trade, self.footprint_tick_size);
        }

        for time in updated_times {
            if let Some(footprint) = footprints.get_mut(&time) {
                footprint.calculate_poc();
            }
        }
    }

    fn set_footprint_tick_size(&mut self, tick_size: PriceStep) {
        self.footprint_tick_size = tick_size;
    }

    fn rebuild_time_footprints(&mut self, timeframe: Timeframe) {
        if let Some(footprints) = self.footprints.as_mut() {
            footprints.clear();
        } else {
            return;
        }

        let raw_trades = self.raw_trades.clone();
        self.insert_time_trades(&raw_trades, timeframe);
    }

    fn trade_time_range(&self) -> Option<(UnixMs, UnixMs)> {
        let earliest = self.raw_trades.iter().map(|trade| trade.time).min()?;
        let latest = self.raw_trades.iter().map(|trade| trade.time).max()?;
        Some((earliest, latest))
    }

    fn suggest_trade_fetch_range(
        &self,
        visible_earliest: UnixMs,
        visible_latest: UnixMs,
        timeframe: Timeframe,
    ) -> Option<(UnixMs, UnixMs)> {
        let footprints = self.footprints.as_ref()?;
        let target_time = self
            .bars
            .iter()
            .rev()
            .find(|bar| {
                footprints
                    .get(&bar.time)
                    .map(|footprint| footprint.trades.is_empty())
                    .unwrap_or(true)
            })
            .map(|bar| bar.time)?;

        let last_trade_before_gap = self
            .raw_trades
            .iter()
            .filter(|trade| trade.time < target_time)
            .map(|trade| trade.time)
            .max();
        let first_trade_after_gap = self
            .raw_trades
            .iter()
            .filter(|trade| trade.time > target_time)
            .map(|trade| trade.time)
            .min();

        if last_trade_before_gap.is_none() && first_trade_after_gap.is_none() {
            return None;
        }

        let data_earliest = self.bars.first()?.time;
        let data_latest = self.bars.last()?.time;
        let gap_start = last_trade_before_gap.map_or(data_earliest, |time| time.saturating_add(1));
        let fetch_from = gap_start
            .max(visible_earliest)
            .floor_to(timeframe)
            .max(gap_start);
        let fetch_to = first_trade_after_gap.map_or(data_latest.min(visible_latest), |time| {
            time.saturating_sub(1)
        });

        if fetch_from >= fetch_to {
            return None;
        }

        if gap_start < visible_earliest {
            let interval_ms = timeframe.to_milliseconds();
            if fetch_to.as_u64().saturating_sub(fetch_from.as_u64()) < interval_ms {
                return None;
            }
        }

        Some((fetch_from, fetch_to))
    }

    fn footprint_for_bar(&self, bar: &Kline) -> Option<&KlineTrades> {
        if let Some(tick_aggr) = self.tick_aggr.as_ref() {
            return usize::try_from(bar.time.as_u64())
                .ok()
                .and_then(|index| tick_aggr.datapoints.get(index))
                .map(|point| &point.footprint);
        }

        self.footprints.as_ref()?.get(&bar.time)
    }

    fn rebuild_bars_from_tick_aggregation(&mut self) {
        let Some(tick_aggr) = self.tick_aggr.as_mut() else {
            self.bars.clear();
            return;
        };

        if tick_aggr.datapoints.len() > SERIES_MAX_BARS {
            let excess = tick_aggr.datapoints.len() - SERIES_MAX_BARS;
            tick_aggr.datapoints.drain(0..excess);
        }

        self.tick_times = tick_aggr
            .datapoints
            .iter()
            .map(|point| point.kline.time)
            .collect();
        self.bars = tick_aggr
            .datapoints
            .iter()
            .enumerate()
            .map(|(index, point)| {
                let mut bar = point.kline;
                // Tick indicators are keyed by bar time. Synthetic monotonic
                // timestamps preserve one-to-one identity even when trades
                // share an exchange timestamp.
                bar.time = UnixMs::new(index as u64);
                bar
            })
            .collect();
        self.refresh_indicator_inputs();
    }

    fn set_rsi_config(&mut self, config: RsiConfig) -> bool {
        self.indicators
            .as_mut()
            .map(|indicators| indicators.set_rsi_config(config))
            .unwrap_or(false)
    }

    fn insert_open_interest(&mut self, data: &[OpenInterest], basis: Basis, timeframe: Timeframe) {
        if let Some(indicators) = self.indicators.as_mut() {
            indicators.insert_open_interest_batch(data, basis, timeframe);
        }
    }
}

impl KlineSeriesLike for KlineSeries {
    fn ticker_info(&self) -> &TickerInfo {
        &self.ticker_info
    }

    fn bars(&self) -> &[Kline] {
        &self.bars
    }

    fn time_at_index(&self, index: usize) -> Option<UnixMs> {
        self.tick_times
            .get(index)
            .copied()
            .or_else(|| self.bars.get(index).map(|bar| bar.time))
    }

    fn indicator_value(&self, bar: &Kline) -> f32 {
        bar.volume.total().to_f32_lossy()
    }

    fn indicator_data_for_panel_value_opt(
        &self,
        panel_value: Option<PanelValueId>,
        bar: &Kline,
    ) -> Option<IndicatorData> {
        let indicator = Self::indicator_for_panel_value(panel_value);
        let indicators = self.indicators.as_ref()?;
        let value = indicators.value_for_indicator(indicator, bar)?;
        let mut data = IndicatorData::scalar(value);

        if matches!(indicator, Some(KlineIndicator::Volume))
            && let Some(signed_overlay) = indicators.volume_overlay_for_bar(bar)
        {
            data = data.with_signed_overlay(signed_overlay);
        }

        if matches!(indicator, Some(KlineIndicator::BollingerBands))
            && let Some((upper, lower)) = indicators.bollinger_bands_for_bar(bar)
        {
            data = data
                .with_field(BOLLINGER_UPPER_FIELD_KEY, upper)
                .with_field(BOLLINGER_LOWER_FIELD_KEY, lower);
        }

        if matches!(indicator, Some(KlineIndicator::Rsi))
            && let Some(rsi_point) = indicators.rsi_fields_for_bar(bar)
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

    fn footprint_for_bar(&self, bar: &Kline) -> Option<&KlineTrades> {
        KlineSeries::footprint_for_bar(self, bar)
    }

    fn footprint_tick_size(&self) -> PriceStep {
        self.footprint_tick_size
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
    horizontal_world: HorizontalWorld,
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
    trade_fetch_handle: Option<Handle>,
    trade_fetch_in_flight: bool,
    rsi_config: RsiConfig,
    footprint_tick_multiplier: TickMultiplier,
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
            horizontal_world: HorizontalWorld::new(
                DEFAULT_BAR_SPACING_PX,
                1.0,
                DEFAULT_HORIZONTAL_OFFSET_UNITS,
            ),
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
            trade_fetch_handle: None,
            trade_fetch_in_flight: false,
            rsi_config,
            footprint_tick_multiplier: DEFAULT_FOOTPRINT_TICK_MULTIPLIER,
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
        chart.configure_series_for_basis();

        chart.install_default_indicator_panels();
        chart
    }

    pub fn new_with_tickers_and_config(
        basis: Basis,
        tickers: &[TickerInfo],
        config: &persisted::Config,
    ) -> Self {
        let mut chart = Self::new_with_tickers(basis, tickers);
        chart.apply_persisted_config(config);
        chart
    }

    pub fn serializable_config(&self) -> persisted::Config {
        persisted::Config {
            version: 1,
            horizontal: persisted::HorizontalConfig {
                cell_width: self.horizontal_world.cell_width(),
                scaling: self.horizontal_world.scaling(),
                offset: self.horizontal_world.offset(),
            },
            primary_autoscale: self.primary_autoscale,
            panels: self
                .composition
                .panels
                .iter()
                .map(Self::serializable_panel)
                .collect(),
            splits: self.composition.splits.clone(),
            y_viewports: self
                .panel_y_viewports
                .iter()
                .map(|(panel_id, viewport)| persisted::ViewportConfig {
                    panel_id: panel_id.0,
                    min_value: viewport.min_value,
                    max_value: viewport.max_value,
                })
                .collect(),
            drawings: self
                .drawing
                .drawings()
                .iter()
                .map(Self::serializable_drawing)
                .collect(),
            active_drawing_tool: Self::serializable_drawing_tool(self.drawing.active_tool()),
            rsi: Self::serializable_rsi_config(self.rsi_config),
            series: self
                .series
                .iter()
                .filter_map(|series| {
                    series.name.as_ref().map(|name| persisted::SeriesConfig {
                        ticker: series.ticker_info.ticker,
                        name: Some(name.clone()),
                    })
                })
                .collect(),
        }
    }

    fn apply_persisted_config(&mut self, config: &persisted::Config) {
        if !config.panels.is_empty() {
            let panels = config
                .panels
                .iter()
                .map(Self::panel_from_persisted)
                .collect();
            self.composition = ChartComposition::from_parts(panels, config.splits.clone());
            self.rebuild_indicator_panel_bindings();
            self.rebuild_comparison_layer_bindings();
            self.sync_primary_panel_comparison_sources();
        }

        self.horizontal_world = HorizontalWorld::new(
            config.horizontal.cell_width,
            config.horizontal.scaling,
            config.horizontal.offset,
        );
        self.primary_autoscale = config.primary_autoscale;
        self.panel_y_viewports = config
            .y_viewports
            .iter()
            .filter_map(|viewport| {
                let panel_id = PanelId(viewport.panel_id);
                self.composition.panel(panel_id)?;
                Some((
                    panel_id,
                    PanelYViewport::normalized(viewport.min_value, viewport.max_value)?,
                ))
            })
            .collect();
        let drawings = config
            .drawings
            .iter()
            .filter_map(Self::drawing_from_persisted)
            .filter(|drawing| Self::drawing_panels_exist(&self.composition, &drawing.object))
            .collect();
        self.drawing = DrawingTools::from_parts(
            Self::drawing_tool_from_persisted(config.active_drawing_tool),
            drawings,
        );

        self.rsi_config = Self::rsi_config_from_persisted(config.rsi).normalized();
        let rsi_config = self.rsi_config;
        if let Some(base_series) = self.base_series_mut() {
            let _ = base_series.set_rsi_config(rsi_config);
        }

        for series_config in &config.series {
            let Some(index) = self
                .series
                .iter()
                .position(|series| series.ticker_info.ticker == series_config.ticker)
            else {
                continue;
            };

            self.series[index].name = series_config.name.clone();
        }

        self.bump_rev();
    }

    fn serializable_panel(
        panel: &crate::widget::chart::kline::composition::PanelSpec,
    ) -> persisted::PanelConfig {
        persisted::PanelConfig {
            id: panel.id.0,
            role: Self::serializable_panel_role(panel.role),
            title: panel.title.clone(),
            value_id: panel.value_id.map(Self::serializable_panel_value_id),
            value_precision: panel
                .value_precision
                .map(Self::serializable_value_precision),
            value_label_policy: Self::serializable_value_label_policy(panel.value_label_policy),
            base_layer: panel.base_layer.map(|layer_id| layer_id.0),
            preferred_scale: Self::serializable_panel_scale(panel.preferred_scale),
            comparison_policy: persisted::ComparisonPolicy {
                force_percent_scale_on_multi_source: panel
                    .comparison_policy
                    .force_percent_scale_on_multi_source,
                force_line_for_non_base_sources: panel
                    .comparison_policy
                    .force_line_for_non_base_sources,
            },
            layers: panel.layers.iter().map(Self::serializable_layer).collect(),
        }
    }

    fn serializable_layer(
        layer: &crate::widget::chart::kline::composition::LayerSpec,
    ) -> persisted::LayerConfig {
        persisted::LayerConfig {
            id: layer.id.0,
            name: layer.name.clone(),
            source: Self::serializable_layer_source(&layer.source),
            data_kind: Self::serializable_layer_data_kind(layer.data_kind),
            mark: Self::serializable_mark(layer.mark),
            axis: Self::serializable_axis(layer.axis),
            visible: layer.visible,
            style: persisted::LayerStyle {
                line_width: layer.style.line_width,
                opacity: layer.style.opacity,
            },
        }
    }

    fn panel_from_persisted(
        panel: &persisted::PanelConfig,
    ) -> crate::widget::chart::kline::composition::PanelSpec {
        crate::widget::chart::kline::composition::PanelSpec {
            id: PanelId(panel.id),
            role: Self::panel_role_from_persisted(panel.role),
            title: panel.title.clone(),
            value_id: panel.value_id.map(Self::panel_value_id_from_persisted),
            value_precision: panel
                .value_precision
                .map(Self::value_precision_from_persisted),
            value_label_policy: Self::value_label_policy_from_persisted(panel.value_label_policy),
            base_layer: panel.base_layer.map(LayerId),
            preferred_scale: Self::panel_scale_from_persisted(panel.preferred_scale),
            comparison_policy: PanelComparisonPolicy {
                force_percent_scale_on_multi_source: panel
                    .comparison_policy
                    .force_percent_scale_on_multi_source,
                force_line_for_non_base_sources: panel
                    .comparison_policy
                    .force_line_for_non_base_sources,
            },
            layers: panel
                .layers
                .iter()
                .map(Self::layer_from_persisted)
                .collect(),
        }
    }

    fn layer_from_persisted(
        panel: &persisted::LayerConfig,
    ) -> crate::widget::chart::kline::composition::LayerSpec {
        crate::widget::chart::kline::composition::LayerSpec {
            id: LayerId(panel.id),
            name: panel.name.clone(),
            source: Self::layer_source_from_persisted(panel.source),
            data_kind: Self::layer_data_kind_from_persisted(panel.data_kind),
            mark: Self::mark_from_persisted(panel.mark),
            axis: Self::axis_from_persisted(panel.axis),
            visible: panel.visible,
            style: LayerStyle {
                line_width: panel.style.line_width,
                opacity: panel.style.opacity,
            },
        }
    }

    fn rebuild_indicator_panel_bindings(&mut self) {
        self.indicator_panels = EnumMap::default();

        for panel in &self.composition.panels {
            if let Some(value_id) = panel.value_id {
                let indicator = Self::indicator_for_panel_value(value_id);
                self.indicator_panels[indicator] =
                    Some(IndicatorPanelBinding::AuxPanel { panel_id: panel.id });
            }

            for layer in &panel.layers {
                let Some(value_id) = layer.source.indicator_value_id() else {
                    continue;
                };
                let indicator = Self::indicator_for_panel_value(value_id);
                self.indicator_panels[indicator] = Some(IndicatorPanelBinding::PrimaryLayer {
                    panel_id: panel.id,
                    layer_id: layer.id,
                });
            }
        }
    }

    fn rebuild_comparison_layer_bindings(&mut self) {
        self.comparison_layers.clear();
        let Some(primary_panel_id) = self.composition.primary_panel_id() else {
            return;
        };
        let Some(primary_panel) = self.composition.panel(primary_panel_id) else {
            return;
        };

        for layer in &primary_panel.layers {
            if !matches!(
                layer.source,
                LayerSource::RawKline {
                    source: DataSourceId::Synthetic("Comparison")
                }
            ) {
                continue;
            }

            if let Some(ticker) = self
                .selected_tickers
                .iter()
                .copied()
                .find(|ticker| ticker.ticker.symbol_and_exchange_string() == layer.name)
            {
                self.comparison_layers.insert(ticker, layer.id);
            }
        }
    }

    fn serializable_panel_role(role: PanelRole) -> persisted::PanelRole {
        match role {
            PanelRole::Primary => persisted::PanelRole::Primary,
            PanelRole::Auxiliary => persisted::PanelRole::Auxiliary,
        }
    }

    fn panel_role_from_persisted(role: persisted::PanelRole) -> PanelRole {
        match role {
            persisted::PanelRole::Primary => PanelRole::Primary,
            persisted::PanelRole::Auxiliary => PanelRole::Auxiliary,
        }
    }

    fn serializable_panel_scale(scale: PanelScaleMode) -> persisted::PanelScaleMode {
        match scale {
            PanelScaleMode::Absolute => persisted::PanelScaleMode::Absolute,
            PanelScaleMode::FitVisible => persisted::PanelScaleMode::FitVisible,
            PanelScaleMode::FitVisibleIncludeZero => {
                persisted::PanelScaleMode::FitVisibleIncludeZero
            }
            PanelScaleMode::Logarithmic => persisted::PanelScaleMode::Logarithmic,
            PanelScaleMode::PercentFromBase => persisted::PanelScaleMode::PercentFromBase,
        }
    }

    fn panel_scale_from_persisted(scale: persisted::PanelScaleMode) -> PanelScaleMode {
        match scale {
            persisted::PanelScaleMode::Absolute => PanelScaleMode::Absolute,
            persisted::PanelScaleMode::FitVisible => PanelScaleMode::FitVisible,
            persisted::PanelScaleMode::FitVisibleIncludeZero => {
                PanelScaleMode::FitVisibleIncludeZero
            }
            persisted::PanelScaleMode::Logarithmic => PanelScaleMode::Logarithmic,
            persisted::PanelScaleMode::PercentFromBase => PanelScaleMode::PercentFromBase,
        }
    }

    fn serializable_panel_value_id(value_id: PanelValueId) -> persisted::PanelValueId {
        match value_id {
            PanelValueId::Volume => persisted::PanelValueId::Volume,
            PanelValueId::BollingerBands => persisted::PanelValueId::BollingerBands,
            PanelValueId::Rsi => persisted::PanelValueId::Rsi,
            PanelValueId::OpenInterest => persisted::PanelValueId::OpenInterest,
            PanelValueId::CumulativeVolumeDelta => persisted::PanelValueId::CumulativeVolumeDelta,
        }
    }

    fn panel_value_id_from_persisted(value_id: persisted::PanelValueId) -> PanelValueId {
        match value_id {
            persisted::PanelValueId::Volume => PanelValueId::Volume,
            persisted::PanelValueId::BollingerBands => PanelValueId::BollingerBands,
            persisted::PanelValueId::Rsi => PanelValueId::Rsi,
            persisted::PanelValueId::OpenInterest => PanelValueId::OpenInterest,
            persisted::PanelValueId::CumulativeVolumeDelta => PanelValueId::CumulativeVolumeDelta,
        }
    }

    fn serializable_layer_source(source: &LayerSource) -> persisted::LayerSource {
        match source {
            LayerSource::RawKline {
                source: DataSourceId::Synthetic("Comparison"),
            } => persisted::LayerSource::Comparison,
            LayerSource::RawKline { .. } => persisted::LayerSource::Primary,
            LayerSource::RawIndicator { value_id, .. } => {
                persisted::LayerSource::Indicator(Self::serializable_panel_value_id(*value_id))
            }
        }
    }

    fn layer_source_from_persisted(source: persisted::LayerSource) -> LayerSource {
        match source {
            persisted::LayerSource::Primary => LayerSource::RawKline {
                source: DataSourceId::Primary,
            },
            persisted::LayerSource::Comparison => LayerSource::RawKline {
                source: COMPARISON_SOURCE_ID,
            },
            persisted::LayerSource::Indicator(value_id) => LayerSource::RawIndicator {
                source: DataSourceId::Primary,
                value_id: Self::panel_value_id_from_persisted(value_id),
            },
        }
    }

    fn serializable_layer_data_kind(kind: LayerDataKind) -> persisted::LayerDataKind {
        match kind {
            LayerDataKind::Ohlc => persisted::LayerDataKind::Ohlc,
            LayerDataKind::Scalar => persisted::LayerDataKind::Scalar,
            LayerDataKind::Histogram => persisted::LayerDataKind::Histogram,
        }
    }

    fn layer_data_kind_from_persisted(kind: persisted::LayerDataKind) -> LayerDataKind {
        match kind {
            persisted::LayerDataKind::Ohlc => LayerDataKind::Ohlc,
            persisted::LayerDataKind::Scalar => LayerDataKind::Scalar,
            persisted::LayerDataKind::Histogram => LayerDataKind::Histogram,
        }
    }

    fn serializable_axis(axis: AxisBinding) -> persisted::AxisBinding {
        match axis {
            AxisBinding::Primary => persisted::AxisBinding::Primary,
            AxisBinding::Secondary => persisted::AxisBinding::Secondary,
        }
    }

    fn axis_from_persisted(axis: persisted::AxisBinding) -> AxisBinding {
        match axis {
            persisted::AxisBinding::Primary => AxisBinding::Primary,
            persisted::AxisBinding::Secondary => AxisBinding::Secondary,
        }
    }

    fn serializable_mark(mark: MarkKind) -> persisted::MarkKind {
        match mark {
            MarkKind::Candle => persisted::MarkKind::Candle,
            MarkKind::Line => persisted::MarkKind::Line,
            MarkKind::Bar(mode) => persisted::MarkKind::Bar(Self::serializable_bar_mode(mode)),
        }
    }

    fn mark_from_persisted(mark: persisted::MarkKind) -> MarkKind {
        match mark {
            persisted::MarkKind::Candle => MarkKind::Candle,
            persisted::MarkKind::Line => MarkKind::Line,
            persisted::MarkKind::Bar(mode) => MarkKind::Bar(Self::bar_mode_from_persisted(mode)),
        }
    }

    fn serializable_bar_mode(mode: BarMode) -> persisted::BarMode {
        match mode {
            BarMode::Regular => persisted::BarMode::Regular,
            BarMode::Histogram(mode) => persisted::BarMode::Histogram(match mode {
                HistogramMode::Plain => persisted::HistogramMode::Plain,
                HistogramMode::SignedOverlay => persisted::HistogramMode::SignedOverlay,
            }),
        }
    }

    fn bar_mode_from_persisted(mode: persisted::BarMode) -> BarMode {
        match mode {
            persisted::BarMode::Regular => BarMode::Regular,
            persisted::BarMode::Histogram(mode) => BarMode::Histogram(match mode {
                persisted::HistogramMode::Plain => HistogramMode::Plain,
                persisted::HistogramMode::SignedOverlay => HistogramMode::SignedOverlay,
            }),
        }
    }

    fn serializable_value_precision(precision: PanelValuePrecision) -> persisted::ValuePrecision {
        match precision {
            PanelValuePrecision::BaseTickerMinTick => persisted::ValuePrecision::BaseTickerMinTick,
            PanelValuePrecision::BaseTickerMinQty => persisted::ValuePrecision::BaseTickerMinQty,
            PanelValuePrecision::FixedPower10(step) => {
                persisted::ValuePrecision::FixedPower10(step.power)
            }
            PanelValuePrecision::FixedStep(step) => persisted::ValuePrecision::FixedStep(step),
        }
    }

    fn value_precision_from_persisted(precision: persisted::ValuePrecision) -> PanelValuePrecision {
        match precision {
            persisted::ValuePrecision::BaseTickerMinTick => PanelValuePrecision::BaseTickerMinTick,
            persisted::ValuePrecision::BaseTickerMinQty => PanelValuePrecision::BaseTickerMinQty,
            persisted::ValuePrecision::FixedPower10(power) => {
                PanelValuePrecision::FixedPower10(exchange::unit::Power10::new(power))
            }
            persisted::ValuePrecision::FixedStep(step) => PanelValuePrecision::FixedStep(step),
        }
    }

    fn serializable_value_label_policy(
        policy: PanelValueLabelPolicy,
    ) -> persisted::ValueLabelPolicy {
        persisted::ValueLabelPolicy {
            axis_mode: Self::serializable_value_label_mode(policy.axis_mode),
            header_mode: Self::serializable_value_label_mode(policy.header_mode),
            max_decimals: policy.max_decimals,
        }
    }

    fn value_label_policy_from_persisted(
        policy: persisted::ValueLabelPolicy,
    ) -> PanelValueLabelPolicy {
        PanelValueLabelPolicy {
            axis_mode: Self::value_label_mode_from_persisted(policy.axis_mode),
            header_mode: Self::value_label_mode_from_persisted(policy.header_mode),
            max_decimals: policy.max_decimals,
        }
    }

    fn serializable_value_label_mode(mode: PanelValueLabelMode) -> persisted::ValueLabelMode {
        match mode {
            PanelValueLabelMode::Compact => persisted::ValueLabelMode::Compact,
            PanelValueLabelMode::Commas => persisted::ValueLabelMode::Commas,
            PanelValueLabelMode::Abbreviated => persisted::ValueLabelMode::Abbreviated,
        }
    }

    fn value_label_mode_from_persisted(mode: persisted::ValueLabelMode) -> PanelValueLabelMode {
        match mode {
            persisted::ValueLabelMode::Compact => PanelValueLabelMode::Compact,
            persisted::ValueLabelMode::Commas => PanelValueLabelMode::Commas,
            persisted::ValueLabelMode::Abbreviated => PanelValueLabelMode::Abbreviated,
        }
    }

    fn serializable_drawing_tool(tool: DrawingTool) -> persisted::DrawingTool {
        match tool {
            DrawingTool::Cursor => persisted::DrawingTool::Cursor,
            DrawingTool::Trendline => persisted::DrawingTool::Trendline,
            DrawingTool::Box => persisted::DrawingTool::Box,
            DrawingTool::HorizontalLine => persisted::DrawingTool::HorizontalLine,
            DrawingTool::VerticalLine => persisted::DrawingTool::VerticalLine,
        }
    }

    fn drawing_tool_from_persisted(tool: persisted::DrawingTool) -> DrawingTool {
        match tool {
            persisted::DrawingTool::Cursor => DrawingTool::Cursor,
            persisted::DrawingTool::Trendline => DrawingTool::Trendline,
            persisted::DrawingTool::Box => DrawingTool::Box,
            persisted::DrawingTool::HorizontalLine => DrawingTool::HorizontalLine,
            persisted::DrawingTool::VerticalLine => DrawingTool::VerticalLine,
        }
    }

    fn serializable_drawing(drawing: &DrawingEntity) -> persisted::DrawingConfig {
        persisted::DrawingConfig {
            id: drawing.id.0,
            object: Self::serializable_drawing_object(&drawing.object),
            style: persisted::DrawingStyle {
                stroke_color: drawing.style.stroke_color,
                stroke_width: drawing.style.stroke_width,
                fill_color: drawing.style.fill_color,
            },
            locked: drawing.locked,
            visible: drawing.visible,
        }
    }

    fn drawing_from_persisted(drawing: &persisted::DrawingConfig) -> Option<DrawingEntity> {
        Some(DrawingEntity {
            id: DrawingId(drawing.id),
            object: Self::drawing_object_from_persisted(&drawing.object)?,
            style: DrawingStyle {
                stroke_color: drawing.style.stroke_color,
                stroke_width: drawing.style.stroke_width,
                fill_color: drawing.style.fill_color,
            },
            locked: drawing.locked,
            visible: drawing.visible,
        })
    }

    fn serializable_drawing_object(object: &DrawingObject) -> persisted::DrawingObject {
        match object {
            DrawingObject::Trendline { start, end } => persisted::DrawingObject::Trendline {
                start: Self::serializable_anchor(*start),
                end: Self::serializable_anchor(*end),
            },
            DrawingObject::Box { start, end } => persisted::DrawingObject::Box {
                start: Self::serializable_anchor(*start),
                end: Self::serializable_anchor(*end),
            },
            DrawingObject::HorizontalLine { panel_id, y_unit } => {
                persisted::DrawingObject::HorizontalLine {
                    panel_id: panel_id.0,
                    y_unit: y_unit.0,
                }
            }
            DrawingObject::VerticalLine { time } => {
                persisted::DrawingObject::VerticalLine { time: *time }
            }
        }
    }

    fn drawing_object_from_persisted(object: &persisted::DrawingObject) -> Option<DrawingObject> {
        match object {
            persisted::DrawingObject::Trendline { start, end } => Some(DrawingObject::Trendline {
                start: Self::anchor_from_persisted(*start)?,
                end: Self::anchor_from_persisted(*end)?,
            }),
            persisted::DrawingObject::Box { start, end } => Some(DrawingObject::Box {
                start: Self::anchor_from_persisted(*start)?,
                end: Self::anchor_from_persisted(*end)?,
            }),
            persisted::DrawingObject::HorizontalLine { panel_id, y_unit } => {
                Some(DrawingObject::HorizontalLine {
                    panel_id: PanelId(*panel_id),
                    y_unit: crate::widget::chart::kline::YUnit(*y_unit),
                })
            }
            persisted::DrawingObject::VerticalLine { time } => {
                Some(DrawingObject::VerticalLine { time: *time })
            }
        }
    }

    fn serializable_anchor(anchor: DrawingAnchor) -> persisted::DrawingAnchor {
        persisted::DrawingAnchor {
            panel_id: anchor.panel_id.0,
            time: anchor.time,
            y_unit: anchor.y_unit.0,
        }
    }

    fn anchor_from_persisted(anchor: persisted::DrawingAnchor) -> Option<DrawingAnchor> {
        Some(DrawingAnchor {
            panel_id: PanelId(anchor.panel_id),
            time: anchor.time,
            y_unit: crate::widget::chart::kline::YUnit(anchor.y_unit),
        })
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

    fn serializable_rsi_config(config: RsiConfig) -> persisted::RsiConfig {
        persisted::RsiConfig {
            period: config.period,
            smoothing: match config.smoothing {
                RsiSmoothing::None => persisted::RsiSmoothing::None,
                RsiSmoothing::Ema(period) => persisted::RsiSmoothing::Ema(period),
            },
            upper_band: config.upper_band,
            lower_band: config.lower_band,
        }
    }

    fn rsi_config_from_persisted(config: persisted::RsiConfig) -> RsiConfig {
        RsiConfig {
            period: config.period,
            smoothing: match config.smoothing {
                persisted::RsiSmoothing::None => RsiSmoothing::None,
                persisted::RsiSmoothing::Ema(period) => RsiSmoothing::Ema(period),
            },
            upper_band: config.upper_band,
            lower_band: config.lower_band,
        }
    }

    pub fn update(&mut self, message: Message) -> Option<Action> {
        match message {
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
                            PanelInteraction::YViewportReset { panel_id } => {
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
                            PanelInteraction::SplitChanged { index, split } => {
                                if self
                                    .composition
                                    .set_split(index, split, DEFAULT_MIN_PANEL_RATIO)
                                {
                                    self.bump_rev();
                                }
                            }
                            PanelInteraction::MoveUp { index } => {
                                if index > 0 && self.composition.move_panel(index, index - 1) {
                                    self.bump_rev();
                                }
                            }
                            PanelInteraction::MoveDown { index } => {
                                let target = index.saturating_add(1);
                                if target < self.composition.panels.len()
                                    && self.composition.move_panel(index, target)
                                {
                                    self.bump_rev();
                                }
                            }
                            PanelInteraction::Settings { .. } => {
                                // TODO: Hook for upcoming panel settings modal/workflow.
                            }
                            PanelInteraction::Close { index } => {
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
                        }
                    }
                    KlineWidgetEvent::HorizontalScaleChanged(scale) => {
                        self.horizontal_world = HorizontalWorld::new(
                            scale.as_pixels_per_bar(),
                            1.0,
                            self.horizontal_world.offset(),
                        );
                        return self.fetch_action_from_invalidation();
                    }
                    KlineWidgetEvent::HorizontalWorldChanged {
                        cell_width,
                        scaling,
                        offset,
                    } => {
                        self.horizontal_world = HorizontalWorld::new(cell_width, scaling, offset);
                        return self.fetch_action_from_invalidation();
                    }
                    KlineWidgetEvent::HorizontalOffsetChanged(offset) => {
                        self.horizontal_world = self.horizontal_world.with_offset(offset);
                        return self.fetch_action_from_invalidation();
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
                                self.composition.panel_effective_scale_mode(panel_id),
                                Some(PanelScaleMode::PercentFromBase)
                            )
                        {
                            if self
                                .composition
                                .primary_panel_id()
                                .is_some_and(|primary_id| primary_id == panel_id)
                                && self.primary_autoscale
                            {
                                self.primary_autoscale = false;
                            }
                            self.set_panel_y_viewport(panel_id, viewport);
                        }

                        return self.fetch_action_from_invalidation();
                    }
                    KlineWidgetEvent::PrimaryAutoscaleDisabled => {
                        if self.primary_autoscale {
                            self.primary_autoscale = false;
                            self.bump_rev();
                        }
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
                    KlineWidgetEvent::TickerSettings(_ticker) => {
                        // Hook for ticker-specific settings editor.
                    }
                    KlineWidgetEvent::IndicatorSettings(_value_id) => {
                        // Hook for indicator-specific settings editor.
                    }
                    KlineWidgetEvent::IndicatorRemove(value_id) => {
                        let indicator = Self::indicator_for_panel_value(value_id);
                        self.toggle_indicator(indicator);
                    }
                    KlineWidgetEvent::TickerRemove(ticker) if ticker != self.base_ticker => {
                        return Some(Action::RemoveSeries(ticker));
                    }
                    KlineWidgetEvent::TickerRemove(_ticker) => {}
                    KlineWidgetEvent::XAxisDoubleClick => {
                        self.horizontal_world = HorizontalWorld::new(
                            DEFAULT_BAR_SPACING_PX,
                            1.0,
                            DEFAULT_HORIZONTAL_OFFSET_UNITS,
                        );
                        return self.fetch_action_from_invalidation();
                    }
                }
            }
        }
        None
    }

    fn fetch_action_from_invalidation(&mut self) -> Option<Action> {
        match self.invalidate(None)? {
            super::Action::RequestFetch(fetch) => Some(Action::RequestFetch(fetch)),
            _ => None,
        }
    }

    pub fn view(
        &self,
        timezone: data::UserTimezone,
        pixel_ratio: f32,
    ) -> iced::Element<'_, Message> {
        if self.series.iter().all(|series| series.bars.is_empty()) {
            return iced::widget::center(iced::widget::text("Waiting for data...").size(16)).into();
        }

        let drawing_sidebar = self.drawing.view(&self.composition, Message::Sidebar);
        let unavailable_indicator_panels = self.unavailable_indicator_panels();
        let chart: iced::Element<_> =
            KlineWidget::new(&self.series, self.timeframe, &self.composition)
                .with_basis(self.basis)
                .with_horizontal_world_state(self.horizontal_world)
                .with_pixel_ratio(pixel_ratio)
                .with_primary_autoscale(self.primary_autoscale)
                .with_panel_y_viewports(&self.panel_y_viewports)
                .with_unavailable_indicator_panels(unavailable_indicator_panels)
                .with_drawing_snapshot(self.drawing.snapshot())
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

        let reqs =
            self.collect_fetch_reqs(self.desired_fetch_batches(self.horizontal_world.offset()));
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

        if let Some(base_series) = self.base_series_mut()
            && base_series.set_rsi_config(normalized)
        {
            base_series.refresh_indicator_inputs();
        }

        self.bump_rev();

        let reqs =
            self.collect_fetch_reqs(self.desired_fetch_batches(self.horizontal_world.offset()));
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

    pub fn selected_indicators(&self) -> Vec<KlineIndicator> {
        indicator::all_indicators()
            .iter()
            .copied()
            .filter(|indicator| self.indicator_panels[*indicator].is_some())
            .collect()
    }

    pub fn indicator_is_available(&self, indicator: KlineIndicator) -> bool {
        self.is_indicator_available(indicator)
    }

    pub fn indicator_is_supported_for_market(&self, indicator: KlineIndicator) -> bool {
        indicator::is_supported_for_market(indicator, self.base_ticker.market_type())
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
                handler.mark_failed(req_id);
            }
            return;
        };

        let incoming = self.klines_to_bars(klines);

        if incoming.is_empty() {
            if let Some(handler) = self.request_handlers.get_mut(&ticker_info) {
                handler.mark_failed(req_id);
            }
            return;
        }

        merge_bars(&mut self.series[idx].bars, incoming);
        trim_bars(&mut self.series[idx].bars);
        self.series[idx].refresh_indicator_inputs();
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
        self.bump_rev();
    }

    pub fn insert_trades(&mut self, ticker_info: &TickerInfo, trades: &[Trade]) {
        if trades.is_empty() {
            return;
        }

        let Some(index) = self.series_index.get(ticker_info).copied() else {
            return;
        };

        match self.basis {
            Basis::Tick(_) => self.series[index].insert_tick_trades(trades),
            Basis::Time(timeframe) => {
                if *ticker_info != self.base_ticker {
                    return;
                }

                self.series[index].raw_trades.extend_from_slice(trades);
                self.series[index].insert_time_trades(trades, timeframe);
            }
        }
        self.last_tick = Instant::now();
        self.bump_rev();
    }

    pub fn insert_fetched_trades(
        &mut self,
        trades: &[Trade],
        is_batches_done: bool,
        req_id: Option<uuid::Uuid>,
    ) {
        let Basis::Time(timeframe) = self.basis else {
            return;
        };

        if let Some(base_series) = self.base_series_mut() {
            base_series.raw_trades.extend_from_slice(trades);
            base_series.insert_time_trades(trades, timeframe);
        }

        if is_batches_done
            && let Some(req_id) = req_id
            && let Some(handler) = self.request_handlers.get_mut(&self.base_ticker)
        {
            handler.mark_completed(req_id);
            self.trade_fetch_in_flight = false;
            self.trade_fetch_handle = None;
        }

        if !trades.is_empty() {
            self.bump_rev();
        }
    }

    pub fn mark_trade_fetch_no_data(&mut self, req_id: uuid::Uuid) {
        if let Some(handler) = self.request_handlers.get_mut(&self.base_ticker) {
            handler.mark_no_data(req_id);
        }
        self.trade_fetch_in_flight = false;
        self.trade_fetch_handle = None;
    }

    pub fn mark_fetch_failed(&mut self, req_id: uuid::Uuid) {
        let is_trade_fetch = self
            .request_handlers
            .values()
            .find_map(|handler| handler.request_range(req_id))
            .is_some_and(|range| matches!(range, FetchRange::Trades(..)));

        if self
            .request_handlers
            .values_mut()
            .any(|handler| handler.mark_failed(req_id))
        {
            if is_trade_fetch {
                self.trade_fetch_in_flight = false;
                self.trade_fetch_handle = None;
            }
            return;
        }

        log::warn!("KlineV2 request not found: {:?}", req_id);
    }

    pub fn set_trade_fetch_handle(&mut self, handle: Handle) {
        self.trade_fetch_handle = Some(handle);
    }

    pub fn set_basis(&mut self, basis: Basis) -> Option<super::Action> {
        let preserve_raw_trades =
            matches!(self.basis, Basis::Tick(_)) && matches!(basis, Basis::Tick(_));
        self.basis = basis;
        self.timeframe = Self::timeframe_for_basis(basis);

        for series in &mut self.series {
            series.reset_for_basis(preserve_raw_trades);
        }
        self.configure_series_for_basis();

        self.trade_fetch_handle = None;
        self.trade_fetch_in_flight = false;
        self.rebuild_handlers();
        self.bump_rev();

        let reqs =
            self.collect_fetch_reqs(self.desired_fetch_batches(self.horizontal_world.offset()));
        self.fetch_action(reqs)
    }

    pub fn footprint_tick_multiplier(&self) -> TickMultiplier {
        self.footprint_tick_multiplier
    }

    pub fn set_footprint_tick_multiplier(&mut self, multiplier: TickMultiplier) {
        if self.footprint_tick_multiplier == multiplier {
            return;
        }

        self.footprint_tick_multiplier = multiplier;
        self.configure_series_for_basis();
        self.bump_rev();
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

    fn configure_series_for_basis(&mut self) {
        match self.basis {
            Basis::Time(_) => {
                for series in &mut self.series {
                    series.set_footprint_tick_size(
                        self.footprint_tick_multiplier
                            .multiply_with_min_tick_step(series.ticker_info),
                    );
                    series.disable_tick_aggregation();
                    series.rebuild_time_footprints(self.timeframe);
                }
            }
            Basis::Tick(interval) => {
                for series in &mut self.series {
                    let tick_size = self
                        .footprint_tick_multiplier
                        .multiply_with_min_tick_step(series.ticker_info);
                    series.set_footprint_tick_size(tick_size);
                    series.enable_tick_aggregation(interval, tick_size);
                }
            }
        }
    }

    fn is_indicator_available(&self, indicator: KlineIndicator) -> bool {
        !matches!(
            self.indicator_availability(indicator),
            IndicatorAvailability::Unsupported(_)
        )
    }

    fn indicator_availability(&self, indicator: KlineIndicator) -> IndicatorAvailability {
        let base_series_data = self
            .base_series()
            .and_then(|series| series.indicators.as_ref());
        let availability = indicator::availability(
            indicator,
            AvailabilityContext {
                basis: self.basis,
                timeframe: self.timeframe,
                base_ticker: self.base_ticker,
            },
            base_series_data,
        );

        if !matches!(availability, IndicatorAvailability::Available) {
            return availability;
        }

        let Some(required) = indicator::kline_warmup_bars(indicator, self.rsi_config) else {
            return availability;
        };

        let available = self
            .base_series()
            .map(|series| series.bars.len())
            .unwrap_or(0);
        let required = required as usize;

        if available < required {
            IndicatorAvailability::Unsupported(IndicatorUnsupportedReason::InsufficientData {
                available,
                required,
            })
        } else {
            availability
        }
    }

    fn unavailable_indicator_panels(&self) -> Vec<UnavailableIndicator> {
        indicator::all_indicators()
            .iter()
            .copied()
            .filter_map(|indicator| {
                let IndicatorAvailability::Unsupported(reason) =
                    self.indicator_availability(indicator)
                else {
                    return None;
                };

                self.indicator_panels[indicator].as_ref()?;
                Some(UnavailableIndicator {
                    value_id: Self::panel_value_id_for_indicator(indicator),
                    message: self.unavailable_indicator_message(indicator, reason),
                })
            })
            .collect()
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
                    self.base_ticker.exchange()
                )
            }
            IndicatorUnsupportedReason::ResolutionNotSupported => {
                format!("{name} is not available on {} timeframe.", self.timeframe)
            }
            IndicatorUnsupportedReason::InsufficientData {
                available,
                required,
            } => {
                format!("{name} needs {required} candles; only {available} available.")
            }
            IndicatorUnsupportedReason::MissingRequiredInput => {
                format!("{name} requires directional trade-volume data.")
            }
            IndicatorUnsupportedReason::InconsistentInputCoverage => {
                format!("{name} requires consistent directional trade-volume data.")
            }
        }
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

    fn indicator_for_panel_value(value_id: PanelValueId) -> KlineIndicator {
        match value_id {
            PanelValueId::Volume => KlineIndicator::Volume,
            PanelValueId::BollingerBands => KlineIndicator::BollingerBands,
            PanelValueId::Rsi => KlineIndicator::Rsi,
            PanelValueId::OpenInterest => KlineIndicator::OpenInterest,
            PanelValueId::CumulativeVolumeDelta => KlineIndicator::CumulativeVolumeDelta,
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
            // Tick mode still carries a placeholder timeframe for APIs that
            // require one; x-axis positioning uses the tick basis directly.
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

    fn queue_trade_fetch(
        &mut self,
        range: FetchRange,
        out: &mut Vec<(uuid::Uuid, FetchRange, Option<StreamKind>)>,
    ) {
        if self.trade_fetch_in_flight {
            return;
        }

        let handler = self.request_handlers.entry(self.base_ticker).or_default();

        if let Ok(Some(req_id)) = handler.add_request(range) {
            self.trade_fetch_in_flight = true;
            out.push((
                req_id,
                range,
                Some(StreamKind::Trades {
                    ticker_info: self.base_ticker,
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

        if let Some(range) = self.desired_trade_fetch_range(self.horizontal_world.offset()) {
            self.queue_trade_fetch(range, &mut reqs);
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
        let spacing = self.horizontal_world.effective_spacing();
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
        let right_offset = (f64::from(horizontal_offset) * dt.get() as f64)
            .round()
            .clamp(i64::MIN as f64, i64::MAX as f64) as i64;
        let right = max_seen.saturating_add_i64(right_offset);

        let span_offset = span_units.saturating_mul(dt.get());
        let left = right.saturating_sub_i64(span_offset);

        Some((
            left.to_unix_ms_non_negative(),
            right.to_unix_ms_non_negative(),
        ))
    }

    fn desired_trade_fetch_range(&self, horizontal_offset: f32) -> Option<FetchRange> {
        if !matches!(self.basis, Basis::Time(_)) || !is_trade_fetch_enabled() {
            return None;
        }

        let timeframe = self.timeframe;
        let dt = self.dt_ms_est();
        let visible_points = self.estimate_visible_points_for_fetch().max(0) as u64;
        let (visible_earliest, visible_latest) = self
            .compute_visible_window(horizontal_offset)
            .unwrap_or_else(|| {
                let latest = self.align_floor(UnixMs::now());
                let span = DEFAULT_FETCH_BARS.max(visible_points).saturating_mul(dt);
                (latest.saturating_sub(span), latest)
            });
        let visible_latest = visible_latest.min(self.align_floor(UnixMs::now()));

        if visible_earliest >= visible_latest {
            return None;
        }

        let base_series = self.base_series()?;
        let Some((data_earliest, data_latest)) = base_series.trade_time_range() else {
            return Some(FetchRange::Trades(visible_earliest, visible_latest));
        };

        if visible_earliest < data_earliest {
            return Some(FetchRange::Trades(visible_earliest, data_earliest));
        }
        if visible_latest > data_latest {
            return Some(FetchRange::Trades(
                data_latest.saturating_add(1),
                visible_latest,
            ));
        }

        base_series
            .suggest_trade_fetch_range(visible_earliest, visible_latest, timeframe)
            .map(|(from, to)| FetchRange::Trades(from, to))
    }

    fn desired_fetch_batches(&self, horizontal_offset: f32) -> Vec<(FetchRange, Vec<TickerInfo>)> {
        if matches!(self.basis, Basis::Tick(_)) {
            return Vec::new();
        }

        let dt = self.dt_ms_est();
        let visible_points = self.estimate_visible_points_for_fetch().max(0) as u64;
        let indicator_warmup_bars = self.active_indicator_kline_warmup_bars();
        let base_fetch_bars =
            DEFAULT_FETCH_BARS.max(visible_points.saturating_add(indicator_warmup_bars));
        let comparison_fetch_bars = DEFAULT_FETCH_BARS.max(visible_points);
        let base_span = base_fetch_bars.saturating_mul(dt);
        let comparison_span = comparison_fetch_bars.saturating_mul(dt);
        let last_closed = self.align_floor(UnixMs::now());

        let mut batches: Vec<(FetchRange, Vec<TickerInfo>)> = Vec::new();

        let mut empty_base = false;
        let mut empty_comparisons: Vec<TickerInfo> = Vec::new();
        for series in &self.series {
            if series.bars.is_empty() {
                if series.ticker_info == self.base_ticker {
                    empty_base = true;
                } else {
                    empty_comparisons.push(series.ticker_info);
                }
            }
        }

        if empty_base {
            let end = last_closed;
            let start = end.saturating_sub(base_span);
            batches.push((FetchRange::Kline(start, end), vec![self.base_ticker]));
        }

        if !empty_comparisons.is_empty() {
            let end = last_closed;
            let start = end.saturating_sub(comparison_span);
            batches.push((FetchRange::Kline(start, end), empty_comparisons));
        }

        if let Some((window_min, _window_max)) = self.compute_visible_window(horizontal_offset) {
            let warmup_start = if indicator_warmup_bars > 0 {
                self.align_floor(
                    window_min.saturating_sub(indicator_warmup_bars.saturating_mul(dt)),
                )
            } else {
                window_min
            };

            for series in &self.series {
                let target_min = if series.ticker_info == self.base_ticker {
                    warmup_start.min(window_min)
                } else {
                    window_min
                };
                let fetch_span = if series.ticker_info == self.base_ticker {
                    base_span
                } else {
                    comparison_span
                };

                if let Some(series_min) = series.bars.first().map(|bar| bar.time)
                    && target_min < series_min
                {
                    let end = self.align_floor(series_min);
                    let start = end.saturating_sub(fetch_span);
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
                    .unwrap_or(base_span);
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

    fn base_series_mut(&mut self) -> Option<&mut KlineSeries> {
        let idx = self.series_index.get(&self.base_ticker).copied()?;
        self.series.get_mut(idx)
    }

    pub fn insert_open_interest(
        &mut self,
        req_id: Option<uuid::Uuid>,
        ticker_info: TickerInfo,
        data: &[OpenInterest],
    ) {
        if ticker_info != self.base_ticker {
            return;
        }

        let Some(idx) = self.series_index.get(&ticker_info).copied() else {
            if let Some(req_id) = req_id
                && let Some(handler) = self.request_handlers.get_mut(&ticker_info)
            {
                handler.mark_failed(req_id);
            }
            return;
        };

        if let Some(req_id) = req_id
            && let Some(handler) = self.request_handlers.get_mut(&ticker_info)
        {
            if data.is_empty() {
                handler.mark_failed(req_id);
            } else {
                handler.mark_completed(req_id);
            }
        }

        if data.is_empty() {
            return;
        }

        let series = &mut self.series[idx];
        series.insert_open_interest(data, self.basis, self.timeframe);
        self.bump_rev();
    }

    fn streams_for_all(&self) -> Vec<StreamKind> {
        match self.basis {
            Basis::Time(_) => {
                let mut streams = vec![StreamKind::Trades {
                    ticker_info: self.base_ticker,
                }];
                streams.extend(self.selected_tickers.iter().copied().map(|ticker_info| {
                    StreamKind::Kline {
                        ticker_info,
                        timeframe: self.timeframe,
                    }
                }));
                streams
            }
            Basis::Tick(_) => self
                .selected_tickers
                .iter()
                .copied()
                .map(|ticker_info| StreamKind::Trades { ticker_info })
                .collect(),
        }
    }

    fn add_ticker_state(&mut self, ticker_info: TickerInfo) -> bool {
        if self.selected_tickers.contains(&ticker_info) {
            return false;
        }

        let idx = self.series.len();
        let mut series = if ticker_info == self.base_ticker {
            KlineSeries::new_base(ticker_info)
        } else {
            KlineSeries::new(ticker_info)
        };
        let _ = series.set_rsi_config(self.rsi_config);
        if let Basis::Tick(interval) = self.basis {
            let tick_size = self
                .footprint_tick_multiplier
                .multiply_with_min_tick_step(series.ticker_info);
            series.set_footprint_tick_size(tick_size);
            series.enable_tick_aggregation(interval, tick_size);
        }
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
