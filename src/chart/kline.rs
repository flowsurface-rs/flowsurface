// FILE-SIZE-OK: upstream file, splitting out of scope for this fork
// GitHub Issue: https://github.com/terrylica/rangebar-py/issues/91
use super::{
    Action, Basis, Chart, Interaction, Message, PlotConstants, PlotData, ViewState, indicator,
    request_fetch, scale::linear::PriceInfoLabel,
};
use crate::chart::indicator::kline::KlineIndicatorImpl;
use crate::connector::fetcher::{FetchRange, RequestHandler, is_trade_fetch_enabled};
use crate::{modal::pane::settings::study, style};
use data::aggr::ticks::{OdbMicrostructure, TickAggr};
use data::aggr::time::TimeSeries;
use data::chart::indicator::{Indicator, KlineIndicator};
use data::chart::kline::{
    ClusterKind, ClusterScaling, FootprintStudy, KlineDataPoint, KlineTrades, NPoc, PointOfControl,
};
use data::chart::{Autoscale, KlineChartKind, ViewConfig};

use data::util::{abbr_large_numbers, count_decimals};
use exchange::unit::{Price, PriceStep, Qty};
use exchange::{
    Kline, OpenInterest as OIData, TickerInfo, Trade,
    adapter::clickhouse::{
        OpenDeviationBarProcessor, odb_to_kline, odb_to_microstructure, sse_connected, sse_enabled,
        trade_to_agg_trade,
    },
};

use std::cell::RefCell;
use std::collections::VecDeque;

use iced::task::Handle;
use iced::theme::palette::Extended;
use iced::widget::canvas::{self, Event, Geometry, Path, Stroke};
use iced::{Alignment, Element, Point, Rectangle, Renderer, Size, Theme, Vector, keyboard, mouse};

use enum_map::EnumMap;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Request for the dashboard to trigger an ODB gap-fill via the sidecar.
/// Returned from `insert_trades()` when agg_trade_id continuity gaps are detected.
#[derive(Debug, Clone)]
pub struct GapFillRequest {
    pub symbol: String,
    pub threshold_dbps: u32,
}

/// Classification of agg_trade_id anomalies between consecutive ODB bars.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarGapKind {
    /// Sequential gap: curr_first > prev_last + 1. Missing trades between bars.
    Gap,
    /// Day-boundary gap: same as Gap but bars span different UTC days.
    /// These are structural in ouroboros_mode=day (orphaned midnight bars)
    /// and cannot be healed by CH re-fetch alone — kintsugi must repair first.
    DayBoundary,
    /// Overlap: curr_first <= prev_last. Bars share agg_trade_ids (CH reconciliation artifact).
    Overlap,
}

/// A detected agg_trade_id anomaly between consecutive ODB bars.
/// Part of the Sentinel subsystem (bar-level continuity auditor).
#[derive(Debug, Clone)]
pub struct BarGap {
    /// Classification of this anomaly.
    pub kind: BarGapKind,
    /// last_agg_trade_id of bar[i-1].
    pub prev_last_id: u64,
    /// first_agg_trade_id of bar[i].
    pub curr_first_id: u64,
    /// Number of missing agg_trade_ids (curr_first - prev_last - 1), or overlap count.
    pub missing_count: u64,
    /// Timestamp (ms) of bar[i-1] — the older side of the gap.
    pub prev_bar_time_ms: u64,
    /// Timestamp (ms) of bar[i] (for log correlation).
    pub bar_time_ms: u64,
}

/// State for interactive bar-range selection on ODB charts.
/// Shift+Left Click: 1st = set anchor, 2nd = set end, 3rd = reset anchor.
#[derive(Default)]
struct BarSelectionState {
    /// Visual index of the anchor bar (0 = newest/rightmost).
    anchor: Option<usize>,
    /// Visual index of the end bar (set on second Shift+Click).
    end: Option<usize>,
    /// Whether the Shift key is currently held (tracked via ModifiersChanged).
    shift_held: bool,
}

/// Buffered CH/SSE bar with metadata, applied after gap-fill completion.
type BufferedChKline = (
    Kline,
    Option<(u64, u64)>,
    Option<exchange::adapter::clickhouse::ChMicrostructure>,
    Option<u64>, // open_time_ms
);

impl Chart for KlineChart {
    type IndicatorKind = KlineIndicator;

    fn state(&self) -> &ViewState {
        &self.chart
    }

    fn mut_state(&mut self) -> &mut ViewState {
        &mut self.chart
    }

    fn invalidate_crosshair(&mut self) {
        self.chart.cache.clear_crosshair();
        self.indicators
            .values_mut()
            .filter_map(Option::as_mut)
            .for_each(|indi| indi.clear_crosshair_caches());
    }

    fn invalidate_all(&mut self) {
        self.invalidate(None);
    }

    fn view_indicators(&'_ self, enabled: &[Self::IndicatorKind]) -> Vec<Element<'_, Message>> {
        let chart_state = self.state();
        let visible_region = chart_state.visible_region(chart_state.bounds.size());
        let (earliest, latest) = chart_state.interval_range(&visible_region);
        if earliest > latest {
            return vec![];
        }

        let market = chart_state.ticker_info.market_type();
        let mut elements = vec![];

        for selected_indicator in enabled {
            if !KlineIndicator::for_market(market).contains(selected_indicator) {
                continue;
            }
            if !selected_indicator.has_subplot() {
                continue;
            }
            if let Some(indi) = self.indicators[*selected_indicator].as_ref() {
                elements.push(indi.element(chart_state, earliest..=latest));
            }
        }
        elements
    }

    fn visible_timerange(&self) -> Option<(u64, u64)> {
        let chart = self.state();
        let region = chart.visible_region(chart.bounds.size());

        if region.width == 0.0 {
            return None;
        }

        match &chart.basis {
            Basis::Time(timeframe) => {
                let interval = timeframe.to_milliseconds();

                let (earliest, latest) = (
                    chart.x_to_interval(region.x) - (interval / 2),
                    chart.x_to_interval(region.x + region.width) + (interval / 2),
                );

                Some((earliest, latest))
            }
            Basis::Tick(_) => {
                unimplemented!()
            }
            Basis::Odb(_) => {
                // ODB bars use TickBased storage (Vec, oldest-first).
                // Return the full timestamp range of loaded data.
                if let PlotData::TickBased(tick_aggr) = &self.data_source {
                    if tick_aggr.datapoints.is_empty() {
                        return None;
                    }
                    // oldest is at index 0, newest at end
                    let earliest = tick_aggr.datapoints.first()?;
                    let latest = tick_aggr.datapoints.last()?;
                    Some((earliest.kline.time, latest.kline.time))
                } else {
                    None
                }
            }
        }
    }

    fn interval_keys(&self) -> Option<Vec<u64>> {
        match &self.data_source {
            PlotData::TimeBased(_) => None,
            PlotData::TickBased(tick_aggr) => Some(
                tick_aggr
                    .datapoints
                    .iter()
                    .map(|dp| dp.kline.time)
                    .collect(),
            ),
        }
    }

    fn autoscaled_coords(&self) -> Vector {
        let chart = self.state();
        let x_translation = match &self.kind {
            KlineChartKind::Footprint { .. } => {
                0.5 * (chart.bounds.width / chart.scaling) - (chart.cell_width / chart.scaling)
            }
            KlineChartKind::Candles | KlineChartKind::Odb => {
                0.5 * (chart.bounds.width / chart.scaling)
                    - (8.0 * chart.cell_width / chart.scaling)
            }
        };
        Vector::new(x_translation, chart.translation.y)
    }

    fn supports_fit_autoscaling(&self) -> bool {
        true
    }

    fn is_empty(&self) -> bool {
        match &self.data_source {
            PlotData::TimeBased(timeseries) => timeseries.datapoints.is_empty(),
            PlotData::TickBased(tick_aggr) => tick_aggr.datapoints.is_empty(),
        }
    }
}

impl PlotConstants for KlineChart {
    fn min_scaling(&self) -> f32 {
        self.kind.min_scaling()
    }

    fn max_scaling(&self) -> f32 {
        self.kind.max_scaling()
    }

    fn max_cell_width(&self) -> f32 {
        self.kind.max_cell_width()
    }

    fn min_cell_width(&self) -> f32 {
        self.kind.min_cell_width()
    }

    fn max_cell_height(&self) -> f32 {
        self.kind.max_cell_height()
    }

    fn min_cell_height(&self) -> f32 {
        self.kind.min_cell_height()
    }

    fn default_cell_width(&self) -> f32 {
        self.kind.default_cell_width()
    }
}

/// Create an indicator with configuration-aware params.
///
/// OFI-family indicators use `ofi_ema_period`; `TradeIntensityHeatmap` uses
/// `intensity_lookback`. All others use default construction.
// GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97
fn make_indicator_with_config(
    which: KlineIndicator,
    cfg: &data::chart::kline::Config,
) -> Box<dyn KlineIndicatorImpl> {
    match which {
        KlineIndicator::OFI => Box::new(
            indicator::kline::ofi::OFIIndicator::with_ema_period(cfg.ofi_ema_period),
        ),
        KlineIndicator::OFICumulativeEma => Box::new(
            indicator::kline::ofi_cumulative_ema::OFICumulativeEmaIndicator::with_ema_period(
                cfg.ofi_ema_period,
            ),
        ),
        KlineIndicator::TradeIntensityHeatmap => Box::new(
            indicator::kline::trade_intensity_heatmap::TradeIntensityHeatmapIndicator::with_lookback(
                cfg.intensity_lookback,
            ),
        ),
        other => indicator::kline::make_empty(other),
    }
}

pub struct KlineChart {
    chart: ViewState,
    data_source: PlotData<KlineDataPoint>,
    raw_trades: Vec<Trade>,
    indicators: EnumMap<KlineIndicator, Option<Box<dyn KlineIndicatorImpl>>>,
    fetching_trades: (bool, Option<Handle>),
    pub(crate) kind: KlineChartKind,
    request_handler: RequestHandler,
    study_configurator: study::Configurator<FootprintStudy>,
    last_tick: Instant,
    /// Separate timer for telemetry ChartSnapshot (not reset by per-frame ticks).
    #[cfg(feature = "telemetry")]
    last_snapshot: Instant,
    /// In-process ODB processor (opendeviationbar-core). Produces completed bars
    /// from raw WebSocket trades, eliminating the ClickHouse live polling path.
    odb_processor: Option<OpenDeviationBarProcessor>,
    /// Monotonic counter for AggTrade IDs fed to the ODB processor.
    next_agg_id: i64,
    /// Total completed bars from the in-process processor (diagnostic).
    odb_completed_count: u32,
    /// Locally-completed ODB bars appended while SSE is active. These have
    /// approximate boundaries and are popped when the authoritative SSE/CH
    /// bar arrives via `update_latest_kline()`.
    pending_local_bars: u32,
    /// Last agg_trade_id from gap-fill. WS trades with id <= this are skipped.
    gap_fill_fence_agg_id: Option<u64>,
    /// CH/SSE bars received during gap-fill, applied after completion.
    buffered_ch_klines: Vec<BufferedChKline>,
    /// Ring buffer of recent WS trades for bar-boundary replay.
    /// When a SSE/CH bar arrives and the processor resets, trades with
    /// `agg_trade_id > bar.last_agg_trade_id` are replayed into the new processor
    /// to eliminate the forming-bar price gap. VecDeque for O(1) eviction.
    ws_trade_ring: VecDeque<Trade>,
    /// Post-reset fence: after SSE/CH bar resets the processor, WS trades with
    /// `agg_trade_id <= this` are skipped. Prevents stale trades from the
    /// completed bar leaking into the new forming bar.
    sse_reset_fence_agg_id: Option<u64>,
    /// Kline chart configuration (e.g. OFI EMA period).
    // GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97
    pub(crate) kline_config: data::chart::kline::Config,
    // ── Production telemetry fields ──
    /// WS trade count since last throughput log (reset every 30s).
    ws_trade_count_window: u64,
    /// Timestamp (ms) of last throughput log.
    ws_throughput_last_log_ms: u64,
    /// Last seen agg_trade_id from WS trades (for continuity checks).
    last_ws_agg_trade_id: Option<u64>,
    /// Count of WS trades deduped by fence since gap-fill.
    dedup_total_skipped: u64,
    /// Max observed trade latency (wall_clock - trade_time) in ms, reset each log window.
    max_trade_latency_ms: i64,
    /// Count of CH bar reconciliation events since startup.
    ch_reconcile_count: u32,
    /// Watchdog: millisecond timestamp of last WS trade received.
    last_trade_received_ms: u64,
    /// Watchdog: whether we've already sent a dead-feed alert.
    trade_feed_dead_alerted: bool,
    /// Set when gap detection fires; cleared by finalize_gap_fill() + insert_raw_trades(is_batches_done).
    gap_fill_requested: bool,
    /// Cooldown: ms timestamp of last gap-fill trigger (prevents rapid re-triggering).
    last_gap_fill_trigger_ms: u64,
    /// Sentinel: timer for periodic bar-level continuity audit.
    last_sentinel_audit: Instant,
    /// Sentinel: number of bar-level gaps found in last audit (avoids re-alerting).
    sentinel_gap_count: usize,
    /// Sentinel: whether a kline re-fetch has been triggered to heal detected bar gaps.
    sentinel_refetch_pending: bool,
    /// Sentinel: earliest bar_time_ms among healable gaps from the last audit.
    /// Used to distinguish live-session gaps (not in CH yet) from historical gaps.
    sentinel_healable_gap_min_time_ms: Option<u64>,
    /// Bar range selection state (ODB charts only).
    /// Right-click: 1st = set anchor, 2nd = set end, 3rd = clear.
    /// RefCell: `canvas::Program::update()` takes `&self`, interior mutability needed.
    bar_selection: RefCell<BarSelectionState>,
}

impl KlineChart {
    pub fn new(
        layout: ViewConfig,
        basis: Basis,
        tick_size: f32,
        klines_raw: &[Kline],
        raw_trades: Vec<Trade>,
        enabled_indicators: &[KlineIndicator],
        ticker_info: TickerInfo,
        kind: &KlineChartKind,
        // GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97
        kline_config: data::chart::kline::Config,
    ) -> Self {
        match basis {
            Basis::Time(interval) => {
                let step = PriceStep::from_f32(tick_size);

                let timeseries = TimeSeries::<KlineDataPoint>::new(interval, step, klines_raw)
                    .with_trades(&raw_trades);

                let base_price_y = timeseries.base_price();
                let latest_x = timeseries.latest_timestamp().unwrap_or(0);
                let (scale_high, scale_low) = timeseries.price_scale({
                    match kind {
                        KlineChartKind::Footprint { .. } => 12,
                        KlineChartKind::Candles | KlineChartKind::Odb => 60,
                    }
                });

                let low_rounded = scale_low.round_to_side_step(true, step);
                let high_rounded = scale_high.round_to_side_step(false, step);

                let y_ticks = Price::steps_between_inclusive(low_rounded, high_rounded, step)
                    .map(|n| n.saturating_sub(1))
                    .unwrap_or(1)
                    .max(1) as f32;

                let cell_width = match kind {
                    KlineChartKind::Footprint { .. } => 80.0,
                    KlineChartKind::Candles | KlineChartKind::Odb => 4.0,
                };
                let cell_height = match kind {
                    KlineChartKind::Footprint { .. } => 800.0 / y_ticks,
                    KlineChartKind::Candles | KlineChartKind::Odb => 200.0 / y_ticks,
                };

                let mut chart = ViewState::new(
                    basis,
                    step,
                    count_decimals(tick_size),
                    ticker_info,
                    ViewConfig {
                        splits: layout.splits,
                        autoscale: Some(Autoscale::FitToVisible),
                        include_forming: true,
                    },
                    cell_width,
                    cell_height,
                );
                chart.base_price_y = base_price_y;
                chart.latest_x = latest_x;

                let x_translation = match &kind {
                    KlineChartKind::Footprint { .. } => {
                        0.5 * (chart.bounds.width / chart.scaling)
                            - (chart.cell_width / chart.scaling)
                    }
                    KlineChartKind::Candles | KlineChartKind::Odb => {
                        0.5 * (chart.bounds.width / chart.scaling)
                            - (8.0 * chart.cell_width / chart.scaling)
                    }
                };
                chart.translation.x = x_translation;

                let data_source = PlotData::TimeBased(timeseries);

                let mut indicators = EnumMap::default();
                for &i in enabled_indicators {
                    let mut indi = make_indicator_with_config(i, &kline_config);
                    indi.rebuild_from_source(&data_source);
                    indicators[i] = Some(indi);
                }

                KlineChart {
                    chart,
                    data_source,
                    raw_trades,
                    indicators,
                    fetching_trades: (false, None),
                    request_handler: RequestHandler::default(),
                    kind: kind.clone(),
                    study_configurator: study::Configurator::new(),
                    last_tick: Instant::now(),
                    #[cfg(feature = "telemetry")]
                    last_snapshot: Instant::now(),
                    odb_processor: None,
                    next_agg_id: 0,
                    odb_completed_count: 0,
                    pending_local_bars: 0,
                    gap_fill_fence_agg_id: None,
                    buffered_ch_klines: Vec::new(),
                    ws_trade_ring: VecDeque::new(),
                    sse_reset_fence_agg_id: None,
                    kline_config,
                    ws_trade_count_window: 0,
                    ws_throughput_last_log_ms: 0,
                    last_ws_agg_trade_id: None,
                    dedup_total_skipped: 0,
                    max_trade_latency_ms: 0,
                    ch_reconcile_count: 0,
                    last_trade_received_ms: 0,
                    trade_feed_dead_alerted: false,
                    gap_fill_requested: false,
                    last_gap_fill_trigger_ms: 0,
                    last_sentinel_audit: Instant::now(),
                    sentinel_gap_count: 0,
                    sentinel_refetch_pending: false,
                    sentinel_healable_gap_min_time_ms: None,
                    bar_selection: Default::default(),
                }
            }
            Basis::Tick(interval) => {
                let step = PriceStep::from_f32(tick_size);

                let cell_width = match kind {
                    KlineChartKind::Footprint { .. } => 80.0,
                    KlineChartKind::Candles | KlineChartKind::Odb => 4.0,
                };
                let cell_height = match kind {
                    KlineChartKind::Footprint { .. } => 90.0,
                    KlineChartKind::Candles | KlineChartKind::Odb => 8.0,
                };

                let mut chart = ViewState::new(
                    basis,
                    step,
                    count_decimals(tick_size),
                    ticker_info,
                    ViewConfig {
                        splits: layout.splits,
                        autoscale: Some(Autoscale::FitToVisible),
                        include_forming: true,
                    },
                    cell_width,
                    cell_height,
                );

                let x_translation = match &kind {
                    KlineChartKind::Footprint { .. } => {
                        0.5 * (chart.bounds.width / chart.scaling)
                            - (chart.cell_width / chart.scaling)
                    }
                    KlineChartKind::Candles | KlineChartKind::Odb => {
                        0.5 * (chart.bounds.width / chart.scaling)
                            - (8.0 * chart.cell_width / chart.scaling)
                    }
                };
                chart.translation.x = x_translation;

                let data_source = PlotData::TickBased(TickAggr::new(interval, step, &raw_trades));

                let mut indicators = EnumMap::default();
                for &i in enabled_indicators {
                    let mut indi = make_indicator_with_config(i, &kline_config);
                    indi.rebuild_from_source(&data_source);
                    indicators[i] = Some(indi);
                }

                KlineChart {
                    chart,
                    data_source,
                    raw_trades,
                    indicators,
                    fetching_trades: (false, None),
                    request_handler: RequestHandler::default(),
                    kind: kind.clone(),
                    study_configurator: study::Configurator::new(),
                    last_tick: Instant::now(),
                    #[cfg(feature = "telemetry")]
                    last_snapshot: Instant::now(),
                    odb_processor: None,
                    next_agg_id: 0,
                    odb_completed_count: 0,
                    pending_local_bars: 0,
                    gap_fill_fence_agg_id: None,
                    buffered_ch_klines: Vec::new(),
                    ws_trade_ring: VecDeque::new(),
                    sse_reset_fence_agg_id: None,
                    kline_config,
                    ws_trade_count_window: 0,
                    ws_throughput_last_log_ms: 0,
                    last_ws_agg_trade_id: None,
                    dedup_total_skipped: 0,
                    max_trade_latency_ms: 0,
                    ch_reconcile_count: 0,
                    last_trade_received_ms: 0,
                    trade_feed_dead_alerted: false,
                    gap_fill_requested: false,
                    last_gap_fill_trigger_ms: 0,
                    last_sentinel_audit: Instant::now(),
                    sentinel_gap_count: 0,
                    sentinel_refetch_pending: false,
                    sentinel_healable_gap_min_time_ms: None,
                    bar_selection: Default::default(),
                }
            }
            Basis::Odb(threshold_dbps) => {
                // ODB bars use TickBased storage (Vec indexed by position) with
                // index-based rendering, matching the Tick coordinate system.
                // Data comes from ClickHouse as precomputed klines.
                let step = PriceStep::from_f32(tick_size);

                let mut tick_aggr = TickAggr::from_klines(step, klines_raw);
                tick_aggr.odb_threshold_dbps = Some(threshold_dbps);

                // Scale cell width with threshold: larger thresholds have fewer bars
                // covering the same time span, so each bar deserves more horizontal space.
                // Reference: 250 dbps → 4.0 px. 500 → 8.0, 1000 → 16.0.
                let cell_width = 4.0_f32 * (threshold_dbps as f32 / 250.0);
                let cell_height = 8.0;

                let mut chart = ViewState::new(
                    basis,
                    step,
                    count_decimals(tick_size),
                    ticker_info,
                    ViewConfig {
                        splits: layout.splits,
                        autoscale: Some(Autoscale::FitToVisible),
                        include_forming: true,
                    },
                    cell_width,
                    cell_height,
                );

                let x_translation = 0.5 * (chart.bounds.width / chart.scaling)
                    - (8.0 * chart.cell_width / chart.scaling);
                chart.translation.x = x_translation;

                // Set last price line from newest kline so the dashed line
                // appears immediately, before any WebSocket trades arrive.
                // Color = last bar's close vs previous bar's close (market direction).
                if let Some(last_kline) = klines_raw.last() {
                    let prev_close = klines_raw
                        .iter()
                        .rev()
                        .nth(1)
                        .map(|k| k.close)
                        .unwrap_or(last_kline.close);
                    chart.last_price = Some(PriceInfoLabel::new(last_kline.close, prev_close));
                }

                let data_source = PlotData::TickBased(tick_aggr);

                let mut indicators = EnumMap::default();
                for &i in enabled_indicators {
                    let mut indi = make_indicator_with_config(i, &kline_config);
                    indi.rebuild_from_source(&data_source);
                    indicators[i] = Some(indi);
                }

                let odb_processor = OpenDeviationBarProcessor::new(threshold_dbps)
                    .map_err(|e| {
                        log::warn!("failed to create OpenDeviationBarProcessor: {e}");
                        exchange::tg_alert!(
                            exchange::telegram::Severity::Critical,
                            "odb-processor",
                            "ODB processor creation failed: {e}"
                        );
                    })
                    .ok();

                // Fix stale splits: saved states may have more splits than current
                // subplot panels (e.g. TradeIntensityHeatmap was reclassified from
                // subplot → candle colouring). Recalculate only when count mismatches.
                let subplot_count = indicators
                    .iter()
                    .filter(|(k, v)| v.is_some() && k.has_subplot())
                    .count();
                if let Some(&main_split) = chart.layout.splits.first()
                    && chart.layout.splits.len() != subplot_count
                {
                    chart.layout.splits =
                        data::util::calc_panel_splits(main_split, subplot_count, None);
                }

                KlineChart {
                    chart,
                    data_source,
                    raw_trades,
                    indicators,
                    fetching_trades: (false, None),
                    request_handler: RequestHandler::default(),
                    kind: kind.clone(),
                    study_configurator: study::Configurator::new(),
                    last_tick: Instant::now(),
                    #[cfg(feature = "telemetry")]
                    last_snapshot: Instant::now(),
                    odb_processor,
                    next_agg_id: 0,
                    odb_completed_count: 0,
                    pending_local_bars: 0,
                    gap_fill_fence_agg_id: None,
                    buffered_ch_klines: Vec::new(),
                    ws_trade_ring: VecDeque::new(),
                    sse_reset_fence_agg_id: None,
                    kline_config,
                    ws_trade_count_window: 0,
                    ws_throughput_last_log_ms: 0,
                    last_ws_agg_trade_id: None,
                    dedup_total_skipped: 0,
                    max_trade_latency_ms: 0,
                    ch_reconcile_count: 0,
                    last_trade_received_ms: 0,
                    trade_feed_dead_alerted: false,
                    gap_fill_requested: false,
                    last_gap_fill_trigger_ms: 0,
                    last_sentinel_audit: Instant::now(),
                    sentinel_gap_count: 0,
                    sentinel_refetch_pending: false,
                    sentinel_healable_gap_min_time_ms: None,
                    bar_selection: Default::default(),
                }
            }
        }
    }

    /// Like `new()` but accepts optional microstructure sidecar from ClickHouse.
    /// Converts `ChMicrostructure` → `OdbMicrostructure` at the crate boundary.
    pub fn new_with_microstructure(
        layout: ViewConfig,
        basis: Basis,
        tick_size: f32,
        klines_raw: &[Kline],
        raw_trades: Vec<Trade>,
        enabled_indicators: &[KlineIndicator],
        ticker_info: TickerInfo,
        kind: &KlineChartKind,
        microstructure: Option<&[Option<exchange::adapter::clickhouse::ChMicrostructure>]>,
        agg_trade_id_ranges: Option<&[Option<(u64, u64)>]>,
        open_time_ms_list: Option<&[Option<u64>]>,
        // GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97
        kline_config: data::chart::kline::Config,
    ) -> Self {
        // For non-Odb bases or missing microstructure, delegate to plain new()
        if !matches!(basis, Basis::Odb(_)) || microstructure.is_none() {
            return Self::new(
                layout,
                basis,
                tick_size,
                klines_raw,
                raw_trades,
                enabled_indicators,
                ticker_info,
                kind,
                kline_config,
            );
        }

        // Safety: guarded by is_none() check above, but use expect() to document invariant
        let micro_slice = microstructure.expect("microstructure checked above");
        let step = PriceStep::from_f32(tick_size);

        // Convert ChMicrostructure → OdbMicrostructure
        let micro: Vec<Option<OdbMicrostructure>> = micro_slice
            .iter()
            .map(|m| {
                m.map(|cm| OdbMicrostructure {
                    trade_count: cm.trade_count,
                    ofi: cm.ofi,
                    trade_intensity: cm.trade_intensity,
                })
            })
            .collect();

        let empty_ids: Vec<Option<(u64, u64)>> = vec![None; klines_raw.len()];
        let ids = agg_trade_id_ranges.unwrap_or(&empty_ids);
        let empty_open_times: Vec<Option<u64>> = vec![None; klines_raw.len()];
        let open_times = open_time_ms_list.unwrap_or(&empty_open_times);
        let mut tick_aggr =
            TickAggr::from_klines_with_microstructure(step, klines_raw, &micro, ids, open_times);

        // Scale cell width with threshold (see non-microstructure constructor)
        let threshold_dbps = match basis {
            Basis::Odb(t) => t,
            _ => 250,
        };
        tick_aggr.odb_threshold_dbps = Some(threshold_dbps);
        let cell_width = 4.0_f32 * (threshold_dbps as f32 / 250.0);
        let cell_height = 8.0;

        let mut chart = ViewState::new(
            basis,
            step,
            count_decimals(tick_size),
            ticker_info,
            ViewConfig {
                splits: layout.splits,
                autoscale: Some(Autoscale::FitToVisible),
                include_forming: true,
            },
            cell_width,
            cell_height,
        );

        let x_translation =
            0.5 * (chart.bounds.width / chart.scaling) - (8.0 * chart.cell_width / chart.scaling);
        chart.translation.x = x_translation;

        // Set last price line from newest kline so the dashed line
        // appears immediately, before any WebSocket trades arrive.
        // Color = last bar's close vs previous bar's close (market direction).
        if let Some(last_kline) = klines_raw.last() {
            let prev_close = klines_raw
                .iter()
                .rev()
                .nth(1)
                .map(|k| k.close)
                .unwrap_or(last_kline.close);
            chart.last_price = Some(PriceInfoLabel::new(last_kline.close, prev_close));
        }

        let data_source = PlotData::TickBased(tick_aggr);

        let mut indicators = EnumMap::default();
        for &i in enabled_indicators {
            let mut indi = make_indicator_with_config(i, &kline_config);
            indi.rebuild_from_source(&data_source);
            indicators[i] = Some(indi);
        }

        let odb_processor = OpenDeviationBarProcessor::new(threshold_dbps)
            .map_err(|e| {
                log::warn!("failed to create OpenDeviationBarProcessor: {e}");
                exchange::tg_alert!(
                    exchange::telegram::Severity::Critical,
                    "odb-processor",
                    "ODB processor creation failed: {e}"
                );
            })
            .ok();

        // Fix stale splits (same as in new() Odb path above).
        let subplot_count = indicators
            .iter()
            .filter(|(k, v)| v.is_some() && k.has_subplot())
            .count();
        if let Some(&main_split) = chart.layout.splits.first()
            && chart.layout.splits.len() != subplot_count
        {
            chart.layout.splits = data::util::calc_panel_splits(main_split, subplot_count, None);
        }

        #[cfg(feature = "telemetry")]
        {
            use data::telemetry::{self, TelemetryEvent};
            let micro_count = micro.iter().filter(|m| m.is_some()).count();
            let oldest_ts = klines_raw.first().map(|k| k.time).unwrap_or(0);
            let newest_ts = klines_raw.last().map(|k| k.time).unwrap_or(0);
            let now = telemetry::now_ms();
            telemetry::emit(TelemetryEvent::ChInitialFetch {
                ts_ms: now,
                symbol: ticker_info.ticker.to_string(),
                threshold_dbps,
                bar_count: klines_raw.len(),
                oldest_ts,
                newest_ts,
                micro_count,
            });
            telemetry::emit(TelemetryEvent::ChartOpen {
                ts_ms: now,
                symbol: ticker_info.ticker.to_string(),
                threshold_dbps,
                bar_count: klines_raw.len(),
                micro_coverage: micro_count,
            });
        }

        KlineChart {
            chart,
            data_source,
            raw_trades,
            indicators,
            fetching_trades: (false, None),
            request_handler: RequestHandler::default(),
            kind: kind.clone(),
            study_configurator: study::Configurator::new(),
            last_tick: Instant::now(),
            #[cfg(feature = "telemetry")]
            last_snapshot: Instant::now(),
            odb_processor,
            next_agg_id: 0,
            odb_completed_count: 0,
            pending_local_bars: 0,
            gap_fill_fence_agg_id: None,
            buffered_ch_klines: Vec::new(),
            ws_trade_ring: VecDeque::new(),
            sse_reset_fence_agg_id: None,
            kline_config,
            ws_trade_count_window: 0,
            ws_throughput_last_log_ms: 0,
            last_ws_agg_trade_id: None,
            dedup_total_skipped: 0,
            max_trade_latency_ms: 0,
            ch_reconcile_count: 0,
            last_trade_received_ms: 0,
            trade_feed_dead_alerted: false,
            gap_fill_requested: false,
            last_gap_fill_trigger_ms: 0,
            last_sentinel_audit: Instant::now(),
            sentinel_gap_count: 0,
            sentinel_refetch_pending: false,
            sentinel_healable_gap_min_time_ms: None,
            bar_selection: Default::default(),
        }
    }

    pub fn update_latest_kline(
        &mut self,
        kline: &Kline,
        bar_agg_id_range: Option<(u64, u64)>,
        micro: Option<exchange::adapter::clickhouse::ChMicrostructure>,
        bar_open_time_ms: Option<u64>,
    ) {
        let bar_last_agg_id = bar_agg_id_range.map(|(_, last)| last);
        if self.chart.basis.is_odb() {
            log::debug!(
                "[SSE-dispatch] update_latest_kline: ts={} bar_agg_id_range={:?} \
                 basis={:?} fetching_trades={} pending_local_bars={}",
                kline.time,
                bar_agg_id_range,
                self.chart.basis,
                self.fetching_trades.0,
                self.pending_local_bars,
            );
        }
        match self.data_source {
            PlotData::TimeBased(ref mut timeseries) => {
                timeseries.insert_klines(&[*kline]);

                self.indicators
                    .values_mut()
                    .filter_map(Option::as_mut)
                    .for_each(|indi| indi.on_insert_klines(&[*kline]));

                let chart = self.mut_state();

                if (kline.time) > chart.latest_x {
                    chart.latest_x = kline.time;
                }

                chart.last_price = Some(PriceInfoLabel::new(kline.close, kline.open));
            }
            PlotData::TickBased(ref mut tick_aggr) => {
                if self.chart.basis.is_odb() {
                    // Buffer CH/SSE bars during gap-fill to prevent temporal inversions.
                    // They'll be applied in order after gap-fill completes.
                    if self.fetching_trades.0 {
                        self.buffered_ch_klines
                            .push((*kline, bar_agg_id_range, micro, bar_open_time_ms));
                        log::debug!(
                            "[gap-fill] buffered CH bar ts={} bar_agg_id_range={:?} during gap-fill",
                            kline.time,
                            bar_agg_id_range,
                        );
                        return;
                    }

                    // Oracle: capture last bar's microstructure BEFORE pop destroys it.
                    // When had_provisional=true, this IS the locally-built provisional bar.
                    // When had_provisional=false, this is the previous completed bar (less useful).
                    let had_provisional = self.pending_local_bars > 0;
                    let provisional_micro =
                        tick_aggr.datapoints.last().and_then(|dp| dp.microstructure);

                    // Pop locally-completed bars before reconciling with authoritative
                    // SSE/CH bars. Local bars have approximate boundaries (arbitrary WS
                    // start point) and are replaced by the authoritative version.
                    if self.pending_local_bars > 0 {
                        let to_pop =
                            (self.pending_local_bars as usize).min(tick_aggr.datapoints.len());
                        tick_aggr
                            .datapoints
                            .truncate(tick_aggr.datapoints.len() - to_pop);
                        log::info!(
                            "[SSE] popped {} pending local bar(s), appending authoritative bar ts={}",
                            to_pop,
                            kline.time,
                        );
                        self.pending_local_bars = 0;
                    }

                    // Get previous bar's close for color direction.
                    // If this kline replaces the last bar (same timestamp), use second-to-last.
                    // If this kline appends (new bar), use the current last bar.
                    let prev_close = if tick_aggr
                        .datapoints
                        .last()
                        .is_some_and(|dp| dp.kline.time == kline.time)
                    {
                        // Replace case: second-to-last bar
                        tick_aggr
                            .datapoints
                            .iter()
                            .rev()
                            .nth(1)
                            .map(|dp| dp.kline.close)
                    } else {
                        // Append case: current last bar
                        tick_aggr.datapoints.last().map(|dp| dp.kline.close)
                    };

                    // ODB streaming update — reconcile ClickHouse completed bar
                    // with locally-constructed forming bar. ClickHouse is authoritative.
                    let was_replace = tick_aggr
                        .datapoints
                        .last()
                        .is_some_and(|dp| dp.kline.time == kline.time);

                    let odb_micro = micro.map(|m| OdbMicrostructure {
                        trade_count: m.trade_count,
                        ofi: m.ofi,
                        trade_intensity: m.trade_intensity,
                    });
                    tick_aggr.replace_or_append_kline(kline, odb_micro);

                    // Attach agg_trade_id_range and open_time_ms from SSE/CH bar data.
                    // These are set after replace_or_append_kline (two-phase pattern)
                    // because replace_or_append_kline doesn't carry them.
                    if let Some(last_dp) = tick_aggr.datapoints.last_mut() {
                        if let Some(range) = bar_agg_id_range {
                            last_dp.agg_trade_id_range = Some(range);
                        }
                        if let Some(ts) = bar_open_time_ms {
                            last_dp.open_time_ms = Some(ts);
                        }
                    }

                    self.ch_reconcile_count += 1;
                    log::info!(
                        "[CH-reconcile] #{}: {} bar ts={} close={:.2} dp_count={}",
                        self.ch_reconcile_count,
                        if was_replace { "REPLACE" } else { "APPEND" },
                        kline.time,
                        kline.close.to_f32(),
                        tick_aggr.datapoints.len(),
                    );

                    // Oracle: the CORRECT assertion — after store, does the bar have microstructure?
                    let stored_has_micro = tick_aggr
                        .datapoints
                        .last()
                        .and_then(|dp| dp.microstructure)
                        .is_some();
                    let stored_ti = tick_aggr
                        .datapoints
                        .last()
                        .and_then(|dp| dp.microstructure)
                        .map(|m| m.trade_intensity);

                    log::info!(
                        "[oracle-micro] bar_ts={} ch_ti={} ch_ofi={} ch_tc={} \
                         provisional_ti={} provisional_ofi={} provisional_tc={} \
                         had_provisional={} stored_has_micro={} stored_ti={:?} action={}",
                        kline.time,
                        odb_micro.map(|m| m.trade_intensity).unwrap_or(-1.0),
                        odb_micro.map(|m| m.ofi).unwrap_or(-999.0),
                        odb_micro.map(|m| m.trade_count).unwrap_or(0),
                        provisional_micro.map(|m| m.trade_intensity).unwrap_or(-1.0),
                        provisional_micro.map(|m| m.ofi).unwrap_or(-999.0),
                        provisional_micro.map(|m| m.trade_count).unwrap_or(0),
                        had_provisional,
                        stored_has_micro,
                        stored_ti,
                        if was_replace { "REPLACE" } else { "APPEND" },
                    );

                    // Oracle assertion: if CH sent microstructure, stored bar MUST have it
                    if odb_micro.is_some() && !stored_has_micro {
                        log::error!(
                            "[oracle-FAIL] bar_ts={} CH sent micro but stored bar has None! \
                             This is the original bug — microstructure lost in pipeline.",
                            kline.time,
                        );
                        exchange::tg_alert!(
                            exchange::telegram::Severity::Critical,
                            "oracle",
                            "Oracle FAIL: CH sent micro but stored bar has None, bar_ts={}",
                            kline.time
                        );
                    }

                    self.indicators
                        .values_mut()
                        .filter_map(Option::as_mut)
                        .for_each(|indi| indi.on_insert_klines(&[*kline]));

                    // SSE/CH bars change datapoints but on_insert_klines is a no-op
                    // for the heatmap indicator. Rebuild to keep data in sync.
                    // Without this, heatmap.data.len() diverges from datapoints.len()
                    // and thermal_body_color maps to wrong bars.
                    self.indicators
                        .values_mut()
                        .filter_map(Option::as_mut)
                        .for_each(|indi| indi.rebuild_from_source(&self.data_source));

                    // When SSE delivers a bar, reset the local RBP processor and
                    // replay buffered WS trades past the bar's last_agg_trade_id.
                    // Without replay, the forming bar opens at whatever trade the WS
                    // delivers next — potentially $30+ away from the bar's close.
                    if sse_enabled()
                        && sse_connected()
                        && let Basis::Odb(threshold_dbps) = self.chart.basis
                    {
                        self.odb_processor = OpenDeviationBarProcessor::new(threshold_dbps)
                            .map_err(|e| {
                                log::warn!("failed to reset ODB processor: {e}");
                                exchange::tg_alert!(
                                    exchange::telegram::Severity::Critical,
                                    "odb-processor",
                                    "ODB processor creation failed: {e}"
                                );
                            })
                            .ok();
                        self.next_agg_id = 0;
                        // Post-reset fence: skip WS trades from the completed bar
                        self.sse_reset_fence_agg_id = bar_last_agg_id;

                        // Replay buffered trades past the bar boundary into the
                        // fresh processor so the forming bar starts from the correct
                        // trade (the one immediately after the completed bar's last).
                        let replayed = if let (Some(fence_id), Some(proc)) =
                            (bar_last_agg_id, &mut self.odb_processor)
                        {
                            let overflow: Vec<_> = self
                                .ws_trade_ring
                                .iter()
                                .filter(|t| t.agg_trade_id.is_none_or(|id| id > fence_id))
                                .cloned()
                                .collect();
                            let count = overflow.len();
                            for trade in &overflow {
                                let agg = trade_to_agg_trade(trade, self.next_agg_id);
                                self.next_agg_id += 1;
                                let _ = proc.process_single_trade(&agg);
                            }
                            if count > 0 {
                                let first_price = overflow.first().map(|t| t.price.to_f32());
                                log::info!(
                                    "[SSE] replayed {} trades past fence_id={} into new processor \
                                     (first_price={:?})",
                                    count,
                                    fence_id,
                                    first_price,
                                );
                            }
                            count
                        } else {
                            0
                        };

                        log::info!(
                            "[SSE] reset ODB processor after bar ts={}, close={:?}, \
                             bar_last_agg_id={:?}, replayed={}",
                            kline.time,
                            kline.close,
                            bar_last_agg_id,
                            replayed,
                        );
                    }

                    // Check forming bar existence before taking &mut self via mut_state().
                    let has_forming = self
                        .odb_processor
                        .as_ref()
                        .and_then(|p| p.get_incomplete_bar())
                        .is_some();

                    let chart = self.mut_state();

                    if kline.time > chart.latest_x {
                        chart.latest_x = kline.time;
                    }

                    // Set last_price from the CH/SSE bar only when no WS trades
                    // have arrived yet (startup).  Once live trades flow,
                    // insert_trades() owns the price line — overwriting
                    // it here with the completed bar's close would show a stale
                    // price (the bar close, not the current market price).
                    if !has_forming && chart.last_trade_time.is_none() {
                        let reference = prev_close.unwrap_or(kline.close);
                        chart.last_price = Some(PriceInfoLabel::new(kline.close, reference));
                    }
                }
            }
        }
    }

    pub fn kind(&self) -> &KlineChartKind {
        &self.kind
    }

    fn missing_data_task(&mut self) -> Option<Action> {
        // Sentinel refetch: clear existing bars so the fresh CH fetch fully replaces
        // the display (not just prepends). Without clearing, prepend_klines skips all
        // bars that are newer than the current oldest — which is all of them since we
        // fetch the N most recent bars. Clearing forces a full reload.
        //
        // Guard: live-session gaps (bars built after UTC midnight) are not yet committed
        // to CH. Clearing datapoints for them would wipe all live bars with no CH
        // replacement. Detect this by comparing the gap's bar_time against today's UTC
        // midnight; skip the destructive clear and let OdbCatchup (already triggered by
        // insert_trades_inner gap detection) handle intra-session gaps instead.
        if self.sentinel_refetch_pending && self.chart.basis.is_odb() {
            self.sentinel_refetch_pending = false;
            let now_ms = chrono::Utc::now().timestamp_millis() as u64;
            let today_midnight_ms = (now_ms / 86_400_000) * 86_400_000;
            let gap_is_live_session = self
                .sentinel_healable_gap_min_time_ms
                .map(|t| t >= today_midnight_ms)
                .unwrap_or(false);
            self.sentinel_healable_gap_min_time_ms = None;

            if gap_is_live_session {
                // Gap is in the current ouroboros session — CH has no coverage yet.
                // OdbCatchup (fired by insert_trades_inner) is the correct repair path.
                log::warn!(
                    "[sentinel] live-session gap (post-midnight) — skipping CH refetch to \
                     avoid wiping live bars; OdbCatchup handles this"
                );
                return None;
            }

            if let PlotData::TickBased(tick_aggr) = &mut self.data_source {
                tick_aggr.datapoints.clear();
            }
            self.request_handler = RequestHandler::default();
            // u64::MAX signals "full reload — no time constraint" to build_odb_sql,
            // which uses the adaptive limit (20K/13K) instead of LIMIT 2000.
            let range = FetchRange::Kline(0, u64::MAX);
            return request_fetch(&mut self.request_handler, range);
        }

        match &self.data_source {
            PlotData::TimeBased(timeseries) => {
                let timeframe_ms = timeseries.interval.to_milliseconds();

                if timeseries.datapoints.is_empty() {
                    let latest = chrono::Utc::now().timestamp_millis() as u64;
                    let earliest = latest.saturating_sub(450 * timeframe_ms);

                    let range = FetchRange::Kline(earliest, latest);
                    if let Some(action) = request_fetch(&mut self.request_handler, range) {
                        return Some(action);
                    }
                }

                let (visible_earliest, visible_latest) = self.visible_timerange()?;
                let (kline_earliest, kline_latest) = timeseries.timerange();
                let earliest = visible_earliest.saturating_sub(visible_latest - visible_earliest);

                // priority 1, basic kline data fetch
                if visible_earliest < kline_earliest {
                    let range = FetchRange::Kline(earliest, kline_earliest);

                    if let Some(action) = request_fetch(&mut self.request_handler, range) {
                        return Some(action);
                    }
                }

                // priority 2, trades fetch
                if !self.fetching_trades.0
                    && is_trade_fetch_enabled()
                    && let Some((fetch_from, fetch_to)) =
                        timeseries.suggest_trade_fetch_range(visible_earliest, visible_latest)
                {
                    let range = FetchRange::Trades(fetch_from, fetch_to);
                    if let Some(action) = request_fetch(&mut self.request_handler, range) {
                        self.fetching_trades = (true, None);
                        return Some(action);
                    }
                }

                // priority 3, Open Interest data
                let ctx = indicator::kline::FetchCtx {
                    main_chart: &self.chart,
                    timeframe: timeseries.interval,
                    visible_earliest,
                    kline_latest,
                    prefetch_earliest: earliest,
                };
                for indi in self.indicators.values_mut().filter_map(Option::as_mut) {
                    if let Some(range) = indi.fetch_range(&ctx)
                        && let Some(action) = request_fetch(&mut self.request_handler, range)
                    {
                        return Some(action);
                    }
                }

                // priority 4, missing klines & integrity check
                if let Some(missing_keys) =
                    timeseries.check_kline_integrity(kline_earliest, kline_latest, timeframe_ms)
                {
                    let latest =
                        missing_keys.iter().max().unwrap_or(&visible_latest) + timeframe_ms;
                    let earliest =
                        missing_keys.iter().min().unwrap_or(&visible_earliest) - timeframe_ms;

                    let range = FetchRange::Kline(earliest, latest);
                    if let Some(action) = request_fetch(&mut self.request_handler, range) {
                        return Some(action);
                    }
                }
            }
            PlotData::TickBased(tick_aggr) => {
                if self.chart.basis.is_odb() {
                    if tick_aggr.datapoints.is_empty() {
                        // Initial fetch — u64::MAX signals "no time constraint, use adaptive
                        // limit" in build_odb_sql (20K for BPR25, 13K floor for others).
                        let range = FetchRange::Kline(0, u64::MAX);
                        return request_fetch(&mut self.request_handler, range);
                    }

                    // Request older data when scrolling left.
                    // TickAggr stores oldest-first; render iterates .rev().enumerate()
                    // so index 0 = newest (rightmost), index N-1 = oldest (leftmost).
                    let oldest_ts = tick_aggr.datapoints.first().unwrap().kline.time;

                    let visible_region = self.chart.visible_region(self.chart.bounds.size());
                    let (_earliest_idx, latest_idx) = self.chart.interval_range(&visible_region);
                    let total_bars = tick_aggr.datapoints.len() as u64;

                    // latest_idx is the left edge (oldest visible bar index).
                    // Fetch when it reaches 80% of loaded bars for smooth scrolling.
                    let fetch_threshold = total_bars.saturating_sub(total_bars / 5);
                    if latest_idx >= fetch_threshold {
                        let range = FetchRange::Kline(0, oldest_ts);
                        return request_fetch(&mut self.request_handler, range);
                    }
                }
            }
        }

        None
    }

    pub fn reset_request_handler(&mut self) {
        self.request_handler = RequestHandler::default();
        self.fetching_trades = (false, None);
    }

    pub fn raw_trades(&self) -> Vec<Trade> {
        self.raw_trades.clone()
    }

    pub fn set_handle(&mut self, handle: Handle) {
        self.fetching_trades.1 = Some(handle);
    }

    pub fn set_fetching_trades(&mut self, active: bool) {
        self.fetching_trades.0 = active;
    }

    pub fn clear_fetching_trades(&mut self) {
        self.fetching_trades = (false, None);
    }

    /// Complete gap-fill lifecycle: set dedup fence, flush buffered CH bars,
    /// clear fetching_trades flag, and invalidate canvas.
    ///
    /// Called from `ChangePaneStatus(Ready)` when the gap-fill sip completes.
    /// The sip's single batch arrives with `is_batches_done = false` (because
    /// `until_time: u64::MAX` means `last_trade_time < until_time` is always
    /// true), so the completion block in `insert_raw_trades` never fires.
    /// This method fills that gap.
    pub fn finalize_gap_fill(&mut self) {
        if !self.fetching_trades.0 {
            return;
        }

        // Set dedup fence from the last gap-fill trade's agg_trade_id.
        if let Some(last_id) = self.raw_trades.iter().rev().find_map(|t| t.agg_trade_id) {
            self.gap_fill_fence_agg_id = Some(last_id);
            // Advance telemetry tracker so we don't report a false-positive
            // gap when the first WS trade past the fence arrives.
            self.last_ws_agg_trade_id = Some(last_id);
            log::info!("[gap-fill] finalize: fence_agg_id={last_id}");
        }

        // Flush buffered CH/SSE bars that arrived during gap-fill.
        let buffered = std::mem::take(&mut self.buffered_ch_klines);
        if !buffered.is_empty() {
            log::info!(
                "[gap-fill] finalize: flushing {} buffered CH bars",
                buffered.len()
            );
        }
        self.fetching_trades = (false, None);
        self.gap_fill_requested = false;
        for (kline, bar_agg_id_range, micro, open_time_ms) in buffered {
            self.update_latest_kline(&kline, bar_agg_id_range, micro, open_time_ms);
        }

        // Startup anchor: seed the RBP processor with the last CH bar's close
        // price so the forming bar opens at the correct level instead of jumping
        // to the first WS trade (which may be $100+ away after a gap).
        if let PlotData::TickBased(ref tick_aggr) = self.data_source
            && let Some(ref mut processor) = self.odb_processor
            && processor.get_incomplete_bar().is_none()
            && let Some(last_dp) = tick_aggr.datapoints.last()
        {
            let anchor_price = last_dp.kline.close;
            let anchor_trade = Trade {
                time: last_dp.kline.time,
                is_sell: false,
                price: anchor_price,
                qty: Qty::ZERO,
                agg_trade_id: None,
            };
            let anchor = trade_to_agg_trade(&anchor_trade, 0);
            match processor.process_single_trade(&anchor) {
                Ok(_) => {
                    log::info!(
                        "[startup-anchor] seeded forming bar at close={:.2} ts={}",
                        anchor_price.to_f32(),
                        last_dp.kline.time,
                    );
                }
                Err(e) => {
                    log::warn!("[startup-anchor] failed to seed: {e}");
                    exchange::tg_alert!(
                        exchange::telegram::Severity::Warning,
                        "startup-anchor",
                        "Startup anchor failed to seed"
                    );
                }
            }
        }

        self.invalidate(None);

        // Sentinel: verify bar continuity after gap-fill completion
        let anomalies = self.audit_bar_continuity();
        let healable: Vec<_> = anomalies
            .iter()
            .filter(|g| g.kind == BarGapKind::Gap)
            .collect();
        let day_boundary_count = anomalies
            .iter()
            .filter(|g| g.kind == BarGapKind::DayBoundary)
            .count();
        let overlap_count = anomalies
            .iter()
            .filter(|g| g.kind == BarGapKind::Overlap)
            .count();
        if anomalies.is_empty() {
            log::info!("[sentinel] post-gap-fill: all bars continuous");
        } else {
            log::warn!(
                "[sentinel] post-gap-fill: {} anomalies remain ({} healable, {} day-boundary, {} overlaps)",
                anomalies.len(),
                healable.len(),
                day_boundary_count,
                overlap_count,
            );
            for (i, gap) in healable.iter().take(3).enumerate() {
                log::warn!(
                    "[sentinel]   remaining gap {}: prev_last={} curr_first={} missing={}",
                    i + 1,
                    gap.prev_last_id,
                    gap.curr_first_id,
                    gap.missing_count,
                );
            }
            // Only send Telegram for healable gaps (day-boundary are structural)
            if exchange::telegram::is_configured() && !healable.is_empty() {
                let total_missing: u64 = healable.iter().map(|g| g.missing_count).sum();
                let msg = format!(
                    "Post-gap-fill: {} healable gaps remain ({} missing IDs)\nKintsugi repair needed on bigblack",
                    healable.len(),
                    total_missing,
                );
                tokio::spawn(async move {
                    exchange::telegram::alert(
                        exchange::telegram::Severity::Warning,
                        "sentinel",
                        &msg,
                    )
                    .await;
                });
            }
        }
    }

    /// Sentinel: scan all datapoints for agg_trade_id anomalies between consecutive bars.
    /// Detects gaps (missing IDs), day-boundary gaps (structural), and overlaps.
    fn audit_bar_continuity(&self) -> Vec<BarGap> {
        let tick_aggr = match &self.data_source {
            PlotData::TickBased(ta) => ta,
            _ => return vec![],
        };

        let mut anomalies = Vec::new();
        const MS_PER_DAY: u64 = 86_400_000;

        for window in tick_aggr.datapoints.windows(2) {
            let (prev, curr) = (&window[0], &window[1]);

            let (Some((_prev_first, prev_last)), Some((curr_first, _curr_last))) =
                (prev.agg_trade_id_range, curr.agg_trade_id_range)
            else {
                continue;
            };

            if curr_first <= prev_last {
                // Overlap: bars share agg_trade_ids (or equal boundary)
                if curr_first == prev_last + 1 {
                    continue; // Perfect continuity — not an anomaly
                }
                let overlap_count = prev_last - curr_first + 1;
                anomalies.push(BarGap {
                    kind: BarGapKind::Overlap,
                    prev_last_id: prev_last,
                    curr_first_id: curr_first,
                    missing_count: overlap_count,
                    prev_bar_time_ms: prev.kline.time,
                    bar_time_ms: curr.kline.time,
                });
                continue;
            }

            let missing = curr_first - prev_last - 1;
            if missing == 0 {
                continue;
            }

            // Classify: single-day-boundary = structural ouroboros midnight orphan (1 day apart).
            // Multi-day = kintsugi-repairable outage (pipeline was down) — treat as healable Gap.
            let prev_day = prev.kline.time / MS_PER_DAY;
            let curr_day = curr.kline.time / MS_PER_DAY;
            let days_spanned = curr_day.saturating_sub(prev_day);
            let kind = if days_spanned == 1 {
                BarGapKind::DayBoundary
            } else {
                BarGapKind::Gap
            };

            anomalies.push(BarGap {
                kind,
                prev_last_id: prev_last,
                curr_first_id: curr_first,
                missing_count: missing,
                prev_bar_time_ms: prev.kline.time,
                bar_time_ms: curr.kline.time,
            });
        }

        anomalies
    }

    pub fn tick_size(&self) -> f32 {
        self.chart.tick_size.to_f32_lossy()
    }

    pub fn study_configurator(&self) -> &study::Configurator<FootprintStudy> {
        &self.study_configurator
    }

    pub fn update_study_configurator(&mut self, message: study::Message<FootprintStudy>) {
        let KlineChartKind::Footprint {
            ref mut studies, ..
        } = self.kind
        else {
            return;
        };

        match self.study_configurator.update(message) {
            Some(study::Action::ToggleStudy(study, is_selected)) => {
                if is_selected {
                    let already_exists = studies.iter().any(|s| s.is_same_type(&study));
                    if !already_exists {
                        studies.push(study);
                    }
                } else {
                    studies.retain(|s| !s.is_same_type(&study));
                }
            }
            Some(study::Action::ConfigureStudy(study)) => {
                if let Some(existing_study) = studies.iter_mut().find(|s| s.is_same_type(&study)) {
                    *existing_study = study;
                }
            }
            None => {}
        }

        self.invalidate(None);
    }

    pub fn chart_layout(&self) -> ViewConfig {
        self.chart.layout()
    }

    pub fn set_autoscale(&mut self, autoscale: Option<Autoscale>) {
        self.chart.layout.autoscale = autoscale;
    }

    pub fn set_include_forming(&mut self, include: bool) {
        self.chart.layout.include_forming = include;
    }

    pub fn set_cluster_kind(&mut self, new_kind: ClusterKind) {
        if let KlineChartKind::Footprint {
            ref mut clusters, ..
        } = self.kind
        {
            *clusters = new_kind;
        }

        self.invalidate(None);
    }

    pub fn set_cluster_scaling(&mut self, new_scaling: ClusterScaling) {
        if let KlineChartKind::Footprint {
            ref mut scaling, ..
        } = self.kind
        {
            *scaling = new_scaling;
        }

        self.invalidate(None);
    }

    pub fn basis(&self) -> Basis {
        self.chart.basis
    }

    pub fn change_tick_size(&mut self, new_tick_size: f32) {
        let chart = self.mut_state();

        let step = PriceStep::from_f32(new_tick_size);

        chart.cell_height *= new_tick_size / chart.tick_size.to_f32_lossy();
        chart.tick_size = step;

        match self.data_source {
            PlotData::TickBased(ref mut tick_aggr) => {
                tick_aggr.change_tick_size(new_tick_size, &self.raw_trades);
            }
            PlotData::TimeBased(ref mut timeseries) => {
                timeseries.change_tick_size(new_tick_size, &self.raw_trades);
            }
        }

        self.indicators
            .values_mut()
            .filter_map(Option::as_mut)
            .for_each(|indi| indi.on_ticksize_change(&self.data_source));

        self.invalidate(None);
    }

    pub fn set_basis(&mut self, new_basis: Basis) -> Option<Action> {
        self.chart.last_price = None;
        self.chart.basis = new_basis;

        match new_basis {
            Basis::Time(interval) => {
                let step = self.chart.tick_size;
                let timeseries = TimeSeries::<KlineDataPoint>::new(interval, step, &[]);
                self.data_source = PlotData::TimeBased(timeseries);
            }
            Basis::Tick(tick_count) => {
                let step = self.chart.tick_size;
                let tick_aggr = TickAggr::new(tick_count, step, &self.raw_trades);
                self.data_source = PlotData::TickBased(tick_aggr);
            }
            Basis::Odb(threshold_dbps) => {
                let step = self.chart.tick_size;
                let mut tick_aggr = TickAggr::from_klines(step, &[]);
                tick_aggr.odb_threshold_dbps = Some(threshold_dbps);
                self.data_source = PlotData::TickBased(tick_aggr);

                // Recreate the processor for the new threshold so live trades
                // produce bars at the correct range.
                self.odb_processor = OpenDeviationBarProcessor::new(threshold_dbps)
                    .map_err(|e| {
                        log::warn!("failed to create OpenDeviationBarProcessor: {e}");
                        exchange::tg_alert!(
                            exchange::telegram::Severity::Critical,
                            "odb-processor",
                            "ODB processor creation failed: {e}"
                        );
                    })
                    .ok();
                self.next_agg_id = 0;
                self.odb_completed_count = 0;
            }
        }

        // Clear processor when switching away from ODB bars.
        if !matches!(new_basis, Basis::Odb(_)) {
            self.odb_processor = None;
            self.next_agg_id = 0;
            self.odb_completed_count = 0;
        }

        self.indicators
            .values_mut()
            .filter_map(Option::as_mut)
            .for_each(|indi| indi.on_basis_change(&self.data_source));

        self.reset_request_handler();
        self.invalidate(Some(Instant::now()))
    }

    pub fn studies(&self) -> Option<Vec<FootprintStudy>> {
        match &self.kind {
            KlineChartKind::Footprint { studies, .. } => Some(studies.clone()),
            _ => None,
        }
    }

    pub fn set_studies(&mut self, new_studies: Vec<FootprintStudy>) {
        if let KlineChartKind::Footprint {
            ref mut studies, ..
        } = self.kind
        {
            *studies = new_studies;
        }

        self.invalidate(None);
    }

    /// Update the OFI EMA period: rebuild the indicator with the new period.
    // GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97
    pub fn set_ofi_ema_period(&mut self, period: usize) {
        self.kline_config.ofi_ema_period = period;
        if self.indicators[KlineIndicator::OFI].is_some() {
            let mut new_indi: Box<dyn KlineIndicatorImpl> =
                Box::new(indicator::kline::ofi::OFIIndicator::with_ema_period(period));
            new_indi.rebuild_from_source(&self.data_source);
            self.indicators[KlineIndicator::OFI] = Some(new_indi);
        }
        if self.indicators[KlineIndicator::OFICumulativeEma].is_some() {
            let mut new_indi: Box<dyn KlineIndicatorImpl> = Box::new(
                indicator::kline::ofi_cumulative_ema::OFICumulativeEmaIndicator::with_ema_period(
                    period,
                ),
            );
            new_indi.rebuild_from_source(&self.data_source);
            self.indicators[KlineIndicator::OFICumulativeEma] = Some(new_indi);
        }
        self.invalidate(None);
    }

    /// Update intensity heatmap lookback window: rebuild the indicator with new params.
    // GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97
    pub fn set_intensity_lookback(&mut self, lookback: usize) {
        self.kline_config.intensity_lookback = lookback;
        if self.indicators[KlineIndicator::TradeIntensityHeatmap].is_some() {
            let mut new_indi: Box<dyn KlineIndicatorImpl> = Box::new(
                indicator::kline::trade_intensity_heatmap::TradeIntensityHeatmapIndicator::with_lookback(lookback),
            );
            new_indi.rebuild_from_source(&self.data_source);
            self.indicators[KlineIndicator::TradeIntensityHeatmap] = Some(new_indi);
        }
        self.invalidate(None);
    }

    pub fn set_thermal_wicks(&mut self, enabled: bool) {
        self.kline_config.thermal_wicks = enabled;
        self.invalidate(None);
    }

    pub fn set_show_sessions(&mut self, show: bool) {
        self.kline_config.show_sessions = show;
        self.invalidate(None);
    }

    /// NOTE(fork): Compute a keyboard navigation message using this chart's current state.
    /// Called from the app-level `keyboard::listen()` subscription to navigate without canvas focus.
    /// GitHub Issue: https://github.com/terrylica/rangebar-py/issues/100
    pub fn keyboard_nav_msg(&self, event: &iced::keyboard::Event) -> Option<super::Message> {
        super::keyboard_nav::process(event, self.state())
    }

    pub fn insert_trades(&mut self, trades_buffer: &[Trade]) -> Option<GapFillRequest> {
        self.insert_trades_inner(trades_buffer, false)
    }

    fn insert_trades_inner(
        &mut self,
        trades_buffer: &[Trade],
        is_gap_fill: bool,
    ) -> Option<GapFillRequest> {
        self.raw_trades.extend_from_slice(trades_buffer);

        match self.data_source {
            PlotData::TickBased(ref mut tick_aggr) => {
                if self.chart.basis.is_odb() {
                    // While gap-fill is active, skip RBP for WebSocket trades
                    // to avoid interleaving current-price trades with historical
                    // gap-fill data.  Gap-fill batches pass is_gap_fill=true to
                    // bypass this guard.
                    if self.fetching_trades.0 && !is_gap_fill {
                        log::trace!(
                            "[gap-fill] blocking {} WS trades during gap-fill",
                            trades_buffer.len(),
                        );
                        // Still update the live price line from the latest trade
                        // so the chart stays in sync with the widget during gap-fill.
                        if let Some(last_trade) = trades_buffer.last() {
                            let prev_close = tick_aggr.datapoints.last().map(|dp| dp.kline.close);
                            let reference = prev_close.unwrap_or(last_trade.price);
                            self.chart.last_price =
                                Some(PriceInfoLabel::new(last_trade.price, reference));
                            self.chart.last_trade_time = Some(last_trade.time);
                        }
                        return None;
                    }

                    // Dedup fence: skip WS trades that overlap with gap-fill data.
                    // Trades with agg_trade_id <= fence are duplicates. Once we see
                    // a trade past the fence, clear it (single transition).
                    if !is_gap_fill && let Some(fence_id) = self.gap_fill_fence_agg_id {
                        let before = trades_buffer.len();
                        let filtered: Vec<_> = trades_buffer
                            .iter()
                            .filter(|t| t.agg_trade_id.is_none_or(|id| id > fence_id))
                            .copied()
                            .collect();
                        let skipped = before - filtered.len();
                        if skipped > 0 {
                            self.dedup_total_skipped += skipped as u64;
                            log::info!(
                                "[dedup] skipped {skipped} WS trades <= fence {fence_id} \
                                 (total_skipped={})",
                                self.dedup_total_skipped,
                            );
                        }
                        if !filtered.is_empty() {
                            self.gap_fill_fence_agg_id = None;
                        }
                        if filtered.is_empty() {
                            return None;
                        }
                        // Continue with filtered trades — re-enter via recursive call
                        // to avoid duplicating the processor logic below.
                        return self.insert_trades_inner(&filtered, false);
                    }

                    // ── Production telemetry: throughput, latency, continuity ──
                    {
                        let now_ms = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64;

                        // Watchdog: record trade arrival + recovery alert
                        self.last_trade_received_ms = now_ms;
                        if self.trade_feed_dead_alerted {
                            self.trade_feed_dead_alerted = false;
                            log::info!("[watchdog] Trade feed recovered");
                            if exchange::telegram::is_configured() {
                                tokio::spawn(async move {
                                    exchange::telegram::alert(
                                        exchange::telegram::Severity::Recovery,
                                        "trade-watchdog",
                                        "Trade feed recovered — WS trades flowing again",
                                    )
                                    .await;
                                });
                            }
                        }

                        // Initialize throughput window on first call
                        if self.ws_throughput_last_log_ms == 0 {
                            self.ws_throughput_last_log_ms = now_ms;
                        }

                        self.ws_trade_count_window += trades_buffer.len() as u64;

                        // Track agg_trade_id continuity + latency (live WS only,
                        // skip for gap-fill which has stale timestamps by design)
                        if !is_gap_fill {
                            for trade in trades_buffer {
                                if let Some(id) = trade.agg_trade_id {
                                    if let Some(prev_id) = self.last_ws_agg_trade_id {
                                        let gap = id.saturating_sub(prev_id);
                                        if gap > 1 {
                                            log::warn!(
                                                "[telemetry] agg_trade_id GAP: prev={prev_id} \
                                                 curr={id} missing={} trades",
                                                gap - 1,
                                            );
                                            // Level 3 guard: alert on trade ID gap
                                            if exchange::telegram::is_configured() {
                                                let detail = format!(
                                                    "agg_trade_id gap: prev={prev_id} curr={id} \
                                                     missing={} trades",
                                                    gap - 1
                                                );
                                                tokio::spawn(async move {
                                                    exchange::telegram::alert(
                                                        exchange::telegram::Severity::Warning,
                                                        "trade continuity",
                                                        &detail,
                                                    )
                                                    .await;
                                                });
                                            }
                                            // Wire gap → automatic OdbCatchup recovery
                                            if !self.fetching_trades.0
                                                && !self.gap_fill_requested
                                                && self.chart.basis.is_odb()
                                                && now_ms
                                                    .saturating_sub(self.last_gap_fill_trigger_ms)
                                                    > 30_000
                                            {
                                                self.gap_fill_requested = true;
                                                self.last_gap_fill_trigger_ms = now_ms;
                                                log::info!(
                                                    "[gap-recovery] triggering: prev={prev_id} curr={id} missing={}",
                                                    gap - 1
                                                );
                                            }
                                        }
                                    }
                                    self.last_ws_agg_trade_id = Some(id);
                                }
                                // Trade latency: wall_clock - exchange trade_time
                                let latency = now_ms as i64 - trade.time as i64;
                                if latency > self.max_trade_latency_ms {
                                    self.max_trade_latency_ms = latency;
                                }
                            }
                        }

                        // Log throughput + latency summary every 30 seconds
                        let elapsed = now_ms.saturating_sub(self.ws_throughput_last_log_ms);
                        if elapsed >= 30_000 {
                            let tps = if elapsed > 0 {
                                (self.ws_trade_count_window as f64 / elapsed as f64) * 1000.0
                            } else {
                                0.0
                            };
                            log::info!(
                                "[telemetry] WS throughput: {:.1} trades/sec ({} trades in {:.1}s) \
                                 max_latency={}ms last_agg_id={:?} dedup_skipped={} \
                                 ch_reconcile={}",
                                tps,
                                self.ws_trade_count_window,
                                elapsed as f64 / 1000.0,
                                self.max_trade_latency_ms,
                                self.last_ws_agg_trade_id,
                                self.dedup_total_skipped,
                                self.ch_reconcile_count,
                            );
                            // Level 3 guard: alert on zero-throughput window
                            // (trade subscription likely lost — issue #104)
                            if self.ws_trade_count_window == 0
                                && exchange::telegram::is_configured()
                            {
                                tokio::spawn(async {
                                    exchange::telegram::alert(
                                        exchange::telegram::Severity::Critical,
                                        "trade feed",
                                        "Zero WS trades in 30s window — \
                                         trade subscription may be lost",
                                    )
                                    .await;
                                });
                            }
                            self.ws_trade_count_window = 0;
                            self.ws_throughput_last_log_ms = now_ms;
                            self.max_trade_latency_ms = 0;
                        }
                    }

                    // Buffer recent WS trades for bar-boundary replay.
                    // When a SSE/CH bar completes, we replay trades past the bar's
                    // last_agg_trade_id into the fresh processor to eliminate the
                    // forming-bar price gap.
                    if !is_gap_fill {
                        const RING_CAP: usize = 10_000;
                        for trade in trades_buffer {
                            if self.ws_trade_ring.len() >= RING_CAP {
                                self.ws_trade_ring.pop_front(); // O(1) eviction
                            }
                            self.ws_trade_ring.push_back(*trade);
                        }
                    }

                    // In-process ODB computation via opendeviationbar-core.
                    // Feed each WebSocket trade into the processor; completed
                    // bars are appended to the chart, replacing ClickHouse
                    // polling as the live data source.
                    if let Some(ref mut processor) = self.odb_processor {
                        let min_tick = self.chart.ticker_info.min_ticksize;
                        let old_dp_len = tick_aggr.datapoints.len();
                        let mut new_bars = 0u32;

                        for trade in trades_buffer {
                            // Post-reset fence: skip stale WS trades that belong
                            // to the completed bar (delivered after SSE reset).
                            if let Some(fence) = self.sse_reset_fence_agg_id
                                && let Some(id) = trade.agg_trade_id
                            {
                                if id <= fence {
                                    continue;
                                }
                                // First trade past fence — clear it and log
                                log::info!(
                                    "[post-reset-fence] cleared: first trade past fence \
                                     id={id} > fence={fence}, price={:.2}",
                                    trade.price.to_f32(),
                                );
                                self.sse_reset_fence_agg_id = None;
                            }

                            let agg = trade_to_agg_trade(trade, self.next_agg_id);

                            // Telemetry: sample every 500th WebSocket trade
                            #[cfg(feature = "telemetry")]
                            if self.next_agg_id % 500 == 0 {
                                use data::telemetry::{self, TelemetryEvent};
                                telemetry::emit(TelemetryEvent::WsTradeSample {
                                    ts_ms: telemetry::now_ms(),
                                    trade_time_ms: trade.time,
                                    price_units: trade.price.units,
                                    price_f32: trade.price.to_f32(),
                                    qty_units: trade.qty.units,
                                    is_sell: trade.is_sell,
                                    seq_id: self.next_agg_id,
                                });
                            }

                            // Diagnostic: log trade details every 2000 trades
                            if self.next_agg_id % 2000 == 0 {
                                log::info!(
                                    "[RBP] seq={} price={:.2} ts_us={} trade_time_ms={}",
                                    self.next_agg_id,
                                    trade.price.to_f32(),
                                    agg.timestamp,
                                    trade.time,
                                );
                                if let Some(forming) = processor.get_incomplete_bar() {
                                    log::info!(
                                        "[RBP]   forming: open={:.2} close={:.2} high={:.2} low={:.2} open_time={} trades={}",
                                        forming.open.to_f64(),
                                        forming.close.to_f64(),
                                        forming.high.to_f64(),
                                        forming.low.to_f64(),
                                        forming.open_time,
                                        forming.agg_record_count,
                                    );
                                    let open = forming.open.to_f64();
                                    let high_excursion = forming.high.to_f64() - open;
                                    let low_excursion = open - forming.low.to_f64();
                                    let threshold_pct =
                                        processor.threshold_decimal_bps() as f64 / 100_000.0;
                                    let expected_delta = open * threshold_pct;
                                    log::info!(
                                        "[RBP]   dbps={} delta={:.2} up={:.2} dn={:.2} breach={}",
                                        processor.threshold_decimal_bps(),
                                        expected_delta,
                                        high_excursion,
                                        low_excursion,
                                        high_excursion >= expected_delta
                                            || low_excursion >= expected_delta,
                                    );
                                }
                            }
                            self.next_agg_id += 1;

                            match processor.process_single_trade(&agg) {
                                Ok(Some(completed)) => {
                                    log::info!(
                                        "[RBP] BAR COMPLETED: open={:.2} close={:.2} high={:.2} low={:.2} trades={}",
                                        completed.open.to_f64(),
                                        completed.close.to_f64(),
                                        completed.high.to_f64(),
                                        completed.low.to_f64(),
                                        completed.agg_record_count,
                                    );
                                    let kline = odb_to_kline(&completed, min_tick);
                                    let micro = odb_to_microstructure(&completed);

                                    #[cfg(feature = "telemetry")]
                                    {
                                        use data::telemetry::{
                                            self, KlineSnapshot, TelemetryEvent,
                                        };
                                        let telem_dbps =
                                            if let data::chart::Basis::Odb(d) = self.chart.basis {
                                                d
                                            } else {
                                                0
                                            };
                                        telemetry::emit(TelemetryEvent::RbpBarComplete {
                                            ts_ms: telemetry::now_ms(),
                                            symbol: self.chart.ticker_info.ticker.to_string(),
                                            threshold_dbps: telem_dbps,
                                            kline: KlineSnapshot::from_kline(&kline),
                                            trade_count: micro.trade_count,
                                            ofi: micro.ofi,
                                            trade_intensity: micro.trade_intensity,
                                            completed_bar_index: self.odb_completed_count,
                                        });
                                    }

                                    let last_time =
                                        tick_aggr.datapoints.last().map(|dp| dp.kline.time);

                                    // Always append locally-completed bars to avoid
                                    // visual gaps. In SSE mode these are provisional
                                    // (approximate boundaries) and will be popped when
                                    // the authoritative SSE/CH bar arrives.
                                    let action = if sse_enabled() && sse_connected() {
                                        "APPEND(local-provisional)"
                                    } else if sse_enabled() && !sse_connected() {
                                        "APPEND(sse-fallback)"
                                    } else {
                                        match last_time {
                                            Some(t) if kline.time == t => "REPLACE",
                                            Some(t) if kline.time > t => "APPEND",
                                            Some(_) => "DROPPED!",
                                            None => "APPEND(empty)",
                                        }
                                    };
                                    log::info!(
                                        "[RBP]   kline.time={} last_dp_time={:?} action={}",
                                        kline.time,
                                        last_time,
                                        action,
                                    );

                                    tick_aggr.replace_or_append_kline(&kline, None);
                                    // Attach microstructure + agg_trade_id range
                                    if let Some(last_dp) = tick_aggr.datapoints.last_mut() {
                                        last_dp.microstructure = Some(OdbMicrostructure {
                                            trade_count: micro.trade_count,
                                            ofi: micro.ofi,
                                            trade_intensity: micro.trade_intensity,
                                        });
                                        // Guard: skip synthetic anchor IDs (0 from startup anchor).
                                        // Without this, the first bar's tooltip would show
                                        // "ID 0 → {real}" instead of a valid Binance range.
                                        if completed.first_agg_trade_id > 0
                                            && completed.last_agg_trade_id > 0
                                        {
                                            last_dp.agg_trade_id_range = Some((
                                                completed.first_agg_trade_id as u64,
                                                completed.last_agg_trade_id as u64,
                                            ));
                                        }
                                    }

                                    // Oracle: verify locally-completed bar has microstructure
                                    let rbp_stored_micro = tick_aggr
                                        .datapoints
                                        .last()
                                        .and_then(|dp| dp.microstructure);
                                    log::info!(
                                        "[oracle-rbp] bar_ts={} ti={:.4} ofi={:.4} tc={} \
                                         stored_has_micro={} action={}",
                                        kline.time,
                                        micro.trade_intensity,
                                        micro.ofi,
                                        micro.trade_count,
                                        rbp_stored_micro.is_some(),
                                        action,
                                    );
                                    if rbp_stored_micro.is_none() {
                                        log::error!(
                                            "[oracle-FAIL] RBP bar_ts={} completed with micro \
                                             but stored bar has None! Manual attachment failed.",
                                            kline.time,
                                        );
                                        exchange::tg_alert!(
                                            exchange::telegram::Severity::Critical,
                                            "oracle",
                                            "Oracle FAIL: RBP bar completed with micro but stored None, bar_ts={}",
                                            kline.time
                                        );
                                    }
                                    // Track provisional bars for cleanup on SSE/CH delivery
                                    if sse_enabled() && sse_connected() {
                                        self.pending_local_bars += 1;
                                    }
                                    new_bars += 1;
                                }
                                Ok(None) => {}
                                Err(e) => {
                                    log::warn!("OpenDeviationBarProcessor error: {e}");
                                }
                            }
                        }

                        // Update live price line from the raw WS trade (exchange-
                        // reported price).  Using the trade directly — rather than
                        // the processor's forming-bar close — keeps the chart in
                        // sync with the widget regardless of processor resets or
                        // bar-boundary divergence.
                        if let Some(last_trade) = trades_buffer.last() {
                            self.chart.last_trade_time = Some(last_trade.time);
                            let prev_close = tick_aggr.datapoints.last().map(|dp| dp.kline.close);
                            let reference = prev_close.unwrap_or(last_trade.price);
                            self.chart.last_price =
                                Some(PriceInfoLabel::new(last_trade.price, reference));
                            log::trace!(
                                "[PRICE/chart] trade_time={} price={:.2} trades_in_batch={}",
                                last_trade.time,
                                last_trade.price.to_f32(),
                                trades_buffer.len(),
                            );
                        }

                        if new_bars > 0 {
                            self.odb_completed_count += new_bars;
                            log::info!(
                                "[RBP] batch: {} new bars, total_completed={}",
                                new_bars,
                                self.odb_completed_count,
                            );
                            self.indicators
                                .values_mut()
                                .filter_map(Option::as_mut)
                                .for_each(|indi| {
                                    indi.on_insert_trades(
                                        trades_buffer,
                                        old_dp_len,
                                        &self.data_source,
                                    )
                                });
                        }
                    } else {
                        // Fallback: no processor, just update price line
                        if let Some(last_trade) = trades_buffer.last() {
                            let prev_close = tick_aggr
                                .datapoints
                                .last()
                                .map(|dp| dp.kline.close)
                                .unwrap_or(last_trade.price);
                            self.chart.last_price =
                                Some(PriceInfoLabel::new(last_trade.price, prev_close));
                        }
                    }
                    // During gap-fill, skip per-batch invalidation to avoid
                    // ~1800 redundant canvas redraws. A single invalidate
                    // fires when the gap-fill completes in insert_raw_trades().
                    if !is_gap_fill {
                        self.invalidate(None);
                    }
                } else {
                    let old_dp_len = tick_aggr.datapoints.len();
                    tick_aggr.insert_trades(trades_buffer);

                    if let Some(last_dp) = tick_aggr.datapoints.last() {
                        self.chart.last_price =
                            Some(PriceInfoLabel::new(last_dp.kline.close, last_dp.kline.open));
                    } else {
                        self.chart.last_price = None;
                    }

                    self.indicators
                        .values_mut()
                        .filter_map(Option::as_mut)
                        .for_each(|indi| {
                            indi.on_insert_trades(trades_buffer, old_dp_len, &self.data_source)
                        });

                    self.invalidate(None);
                }
            }
            PlotData::TimeBased(ref mut timeseries) => {
                timeseries.insert_trades_existing_buckets(trades_buffer);
            }
        }

        // Return gap-fill request if triggered during this batch
        if self.gap_fill_requested
            && !self.fetching_trades.0
            && let Basis::Odb(threshold_dbps) = self.chart.basis
        {
            let symbol = exchange::adapter::clickhouse::bare_symbol(&self.chart.ticker_info);
            return Some(GapFillRequest {
                symbol,
                threshold_dbps,
            });
        }
        None
    }

    pub fn insert_raw_trades(&mut self, raw_trades: Vec<Trade>, is_batches_done: bool) {
        if self.chart.basis.is_odb() && self.odb_processor.is_some() {
            // Gap-fill path: feed REST-fetched trades through OpenDeviationBarProcessor.
            //
            // On the first batch, historical trades arrive AFTER the WebSocket has
            // already pushed a few bars at the current price.  We must:
            //   1. Remove any WS-sourced datapoints whose time > gap start
            //   2. Recreate the RBP so its forming-bar state is clean
            // After that, gap-fill trades build correct bars from the last CH bar.
            //
            // While gap-fill is active (`fetching_trades.0 == true`), WebSocket
            // trades in `insert_trades` skip RBP processing to avoid
            // interleaving current-price trades with historical gap-fill trades.
            if let Some(first_trade) = raw_trades.first() {
                // Check if the RBP's forming bar has state from WebSocket
                // trades that are newer than the incoming gap-fill trades.
                // The forming bar's open_time is in microseconds; convert
                // the trade's millisecond timestamp for comparison.
                let forming_is_newer = self
                    .odb_processor
                    .as_ref()
                    .and_then(|p| p.get_incomplete_bar())
                    .is_some_and(|bar| {
                        let forming_ms = (bar.open_time / 1000) as u64;
                        forming_ms > first_trade.time
                    });

                // Also check if any completed datapoints are newer.
                let dp_is_newer = matches!(
                    self.data_source,
                    PlotData::TickBased(ref tick_aggr)
                        if tick_aggr.datapoints.last()
                            .is_some_and(|dp| first_trade.time < dp.kline.time)
                );

                if forming_is_newer || dp_is_newer {
                    if let PlotData::TickBased(ref mut tick_aggr) = self.data_source {
                        let gap_start = first_trade.time;
                        let before = tick_aggr.datapoints.len();
                        tick_aggr.datapoints.retain(|dp| dp.kline.time <= gap_start);
                        let removed = before - tick_aggr.datapoints.len();
                        log::info!(
                            "[gap-fill] reset: removed {removed} WS-added bars, \
                             retained {} CH bars, recreating RBP \
                             (forming_newer={forming_is_newer}, dp_newer={dp_is_newer})",
                            tick_aggr.datapoints.len(),
                        );
                    }

                    // Recreate the processor with a clean forming-bar state.
                    if let Basis::Odb(threshold_dbps) = self.chart.basis {
                        self.odb_processor = OpenDeviationBarProcessor::new(threshold_dbps)
                            .map_err(|e| log::warn!("failed to recreate RBP: {e}"))
                            .ok();
                        self.next_agg_id = 0;
                    }

                    // Rebuild indicators from the trimmed source.
                    self.indicators
                        .values_mut()
                        .filter_map(Option::as_mut)
                        .for_each(|indi| indi.rebuild_from_source(&self.data_source));
                    self.invalidate(None);
                }
            }

            // Use the inner method with is_gap_fill=true so that:
            // 1. The fetching_trades guard is bypassed (gap-fill trades must be processed)
            // 2. Canvas invalidation is suppressed (single redraw at gap-fill end)
            // Gap-fill trades use is_gap_fill=true which skips gap detection,
            // so the return is always None — discard it.
            let _ = self.insert_trades_inner(&raw_trades, true);
        } else {
            match self.data_source {
                PlotData::TickBased(ref mut tick_aggr) => {
                    tick_aggr.insert_trades(&raw_trades);
                }
                PlotData::TimeBased(ref mut timeseries) => {
                    timeseries.insert_trades_existing_buckets(&raw_trades);
                }
            }

            self.raw_trades.extend(raw_trades);
        }

        if is_batches_done {
            // Set dedup fence from the last gap-fill trade's agg_trade_id.
            if let Some(last_id) = self.raw_trades.iter().rev().find_map(|t| t.agg_trade_id) {
                self.gap_fill_fence_agg_id = Some(last_id);
                // Advance telemetry tracker so we don't report a false-positive
                // gap when the first WS trade past the fence arrives.
                self.last_ws_agg_trade_id = Some(last_id);
                log::info!("[gap-fill] complete: fence_agg_id={last_id}");
            }
            // Flush buffered CH/SSE bars that arrived during gap-fill.
            let buffered = std::mem::take(&mut self.buffered_ch_klines);
            self.fetching_trades = (false, None);
            self.gap_fill_requested = false;
            for (kline, bar_agg_id_range, micro, open_time_ms) in buffered {
                self.update_latest_kline(&kline, bar_agg_id_range, micro, open_time_ms);
            }
            // Single canvas redraw now that all gap-fill batches are processed.
            self.invalidate(None);
        }
    }

    pub fn insert_hist_klines(&mut self, req_id: uuid::Uuid, klines_raw: &[Kline]) {
        match self.data_source {
            PlotData::TimeBased(ref mut timeseries) => {
                timeseries.insert_klines(klines_raw);
                timeseries.insert_trades_existing_buckets(&self.raw_trades);

                self.indicators
                    .values_mut()
                    .filter_map(Option::as_mut)
                    .for_each(|indi| indi.on_insert_klines(klines_raw));

                if klines_raw.is_empty() {
                    self.request_handler
                        .mark_failed(req_id, "No data received".to_string());
                } else {
                    self.request_handler.mark_completed(req_id);
                }
                self.invalidate(None);
            }
            PlotData::TickBased(_) => {}
        }
    }

    /// Insert older ODB klines into the TickBased data source (historical scroll-back).
    pub fn insert_odb_hist_klines(
        &mut self,
        req_id: uuid::Uuid,
        klines: &[Kline],
        microstructure: Option<&[Option<exchange::adapter::clickhouse::ChMicrostructure>]>,
        agg_trade_id_ranges: Option<&[Option<(u64, u64)>]>,
        open_time_ms_list: Option<&[Option<u64>]>,
    ) {
        log::info!(
            "[RB-HIST] insert_odb_hist_klines: {} klines, micro={}, datasource=TickBased?{}",
            klines.len(),
            microstructure.is_some(),
            matches!(self.data_source, PlotData::TickBased(_)),
        );
        match &mut self.data_source {
            PlotData::TickBased(tick_aggr) => {
                let before_len = tick_aggr.datapoints.len();
                if klines.is_empty() {
                    self.request_handler
                        .mark_failed(req_id, "No data received".to_string());
                } else {
                    let micro: Option<Vec<Option<OdbMicrostructure>>> = microstructure.map(|ms| {
                        ms.iter()
                            .map(|m| {
                                m.map(|cm| OdbMicrostructure {
                                    trade_count: cm.trade_count,
                                    ofi: cm.ofi,
                                    trade_intensity: cm.trade_intensity,
                                })
                            })
                            .collect()
                    });
                    tick_aggr.prepend_klines_with_microstructure(
                        klines,
                        micro.as_deref(),
                        agg_trade_id_ranges,
                        open_time_ms_list,
                    );
                    self.request_handler.mark_completed(req_id);
                }
                let after_len = tick_aggr.datapoints.len();
                let micro_count = tick_aggr
                    .datapoints
                    .iter()
                    .filter(|dp| dp.microstructure.is_some())
                    .count();
                log::info!(
                    "[RB-HIST] TickAggr: {} -> {} datapoints, {} with microstructure",
                    before_len,
                    after_len,
                    micro_count,
                );

                // Oracle: dump last 20 bars' microstructure for post-hoc comparison
                for dp in tick_aggr
                    .datapoints
                    .iter()
                    .rev()
                    .take(20)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                {
                    if let Some(m) = dp.microstructure {
                        log::info!(
                            "[oracle-hist] bar_ts={} ti={:.4} ofi={:.4} tc={}",
                            dp.kline.time,
                            m.trade_intensity,
                            m.ofi,
                            m.trade_count,
                        );
                    }
                }

                // Startup anchor: extract anchor price before indicator rebuild
                // (tick_aggr borrow must end before rebuild_from_source borrows self.data_source).
                let anchor_info = tick_aggr
                    .datapoints
                    .last()
                    .map(|dp| (dp.kline.close, dp.kline.time));

                // Rebuild all indicators from updated data source
                let indicator_count = self.indicators.values().filter(|v| v.is_some()).count();
                log::info!(
                    "[RB-HIST] Rebuilding {} indicators from source",
                    indicator_count
                );
                self.indicators
                    .values_mut()
                    .filter_map(Option::as_mut)
                    .for_each(|indi| indi.rebuild_from_source(&self.data_source));
                if let Some((anchor_price, anchor_time)) = anchor_info {
                    let had_premature = self
                        .odb_processor
                        .as_ref()
                        .is_some_and(|p| p.get_incomplete_bar().is_some());
                    if had_premature {
                        // Reset processor to discard premature forming bar
                        if let data::chart::Basis::Odb(threshold_dbps) = self.chart.basis {
                            self.odb_processor = OpenDeviationBarProcessor::new(threshold_dbps)
                                .map_err(|e| {
                                    log::warn!("[startup-anchor] processor reset failed: {e}")
                                })
                                .ok();
                        }
                    }
                    // Seed with last CH bar's close price
                    if let Some(ref mut processor) = self.odb_processor {
                        let anchor_trade = Trade {
                            time: anchor_time,
                            is_sell: false,
                            price: anchor_price,
                            qty: Qty::ZERO,
                            agg_trade_id: None,
                        };
                        let anchor = trade_to_agg_trade(&anchor_trade, 0);
                        match processor.process_single_trade(&anchor) {
                            Ok(_) => {
                                log::info!(
                                    "[startup-anchor] seeded forming bar at close={:.2} \
                                     ts={} had_premature={}",
                                    anchor_price.to_f32(),
                                    anchor_time,
                                    had_premature,
                                );
                            }
                            Err(e) => {
                                log::warn!("[startup-anchor] failed to seed: {e}");
                            }
                        }
                    }
                }

                self.invalidate(None);
            }
            PlotData::TimeBased(_) => {
                log::warn!("[RB-HIST] data_source is TimeBased — ODB klines ignored!");
                exchange::tg_alert!(
                    exchange::telegram::Severity::Info,
                    "odb",
                    "RB-HIST data_source is TimeBased — ODB klines ignored"
                );
            }
        }
    }

    pub fn insert_open_interest(&mut self, req_id: Option<uuid::Uuid>, oi_data: &[OIData]) {
        if let Some(req_id) = req_id {
            if oi_data.is_empty() {
                self.request_handler
                    .mark_failed(req_id, "No data received".to_string());
            } else {
                self.request_handler.mark_completed(req_id);
            }
        }

        if let Some(indi) = self.indicators[KlineIndicator::OpenInterest].as_mut() {
            indi.on_open_interest(oi_data);
        }
    }

    fn calc_qty_scales(
        &self,
        earliest: u64,
        latest: u64,
        highest: Price,
        lowest: Price,
        step: PriceStep,
        cluster_kind: ClusterKind,
    ) -> f32 {
        let rounded_highest = highest.round_to_side_step(false, step).add_steps(1, step);
        let rounded_lowest = lowest.round_to_side_step(true, step).add_steps(-1, step);

        match &self.data_source {
            PlotData::TimeBased(timeseries) => timeseries
                .max_qty_ts_range(
                    cluster_kind,
                    earliest,
                    latest,
                    rounded_highest,
                    rounded_lowest,
                )
                .into(),
            PlotData::TickBased(tick_aggr) => {
                let earliest = earliest as usize;
                let latest = latest as usize;

                tick_aggr
                    .max_qty_idx_range(
                        cluster_kind,
                        earliest,
                        latest,
                        rounded_highest,
                        rounded_lowest,
                    )
                    .into()
            }
        }
    }

    pub fn last_update(&self) -> Instant {
        self.last_tick
    }

    pub fn invalidate(&mut self, now: Option<Instant>) -> Option<Action> {
        let chart = &mut self.chart;

        if let Some(autoscale) = chart.layout.autoscale {
            match autoscale {
                super::Autoscale::CenterLatest => {
                    let x_translation = match &self.kind {
                        KlineChartKind::Footprint { .. } => {
                            0.5 * (chart.bounds.width / chart.scaling)
                                - (chart.cell_width / chart.scaling)
                        }
                        KlineChartKind::Candles | KlineChartKind::Odb => {
                            0.5 * (chart.bounds.width / chart.scaling)
                                - (8.0 * chart.cell_width / chart.scaling)
                        }
                    };
                    chart.translation.x = x_translation;

                    let calculate_target_y = |kline: exchange::Kline| -> f32 {
                        let y_low = chart.price_to_y(kline.low);
                        let y_high = chart.price_to_y(kline.high);
                        let y_close = chart.price_to_y(kline.close);

                        let mut target_y_translation = -(y_low + y_high) / 2.0;

                        if chart.bounds.height > f32::EPSILON && chart.scaling > f32::EPSILON {
                            let visible_half_height = (chart.bounds.height / chart.scaling) / 2.0;

                            let view_center_y_centered = -target_y_translation;

                            let visible_y_top = view_center_y_centered - visible_half_height;
                            let visible_y_bottom = view_center_y_centered + visible_half_height;

                            let padding = chart.cell_height;

                            if y_close < visible_y_top {
                                target_y_translation = -(y_close - padding + visible_half_height);
                            } else if y_close > visible_y_bottom {
                                target_y_translation = -(y_close + padding - visible_half_height);
                            }
                        }
                        target_y_translation
                    };

                    chart.translation.y = self.data_source.latest_y_midpoint(calculate_target_y);
                }
                super::Autoscale::FitToVisible => {
                    let visible_region = chart.visible_region(chart.bounds.size());
                    let (start_interval, end_interval) = chart.interval_range(&visible_region);

                    // For ODB bars, include the forming bar's price range only when
                    // the viewport includes the newest bar (index 0 = rightmost edge).
                    // Without this gate, scrolling to historical data (e.g., 2022 prices)
                    // and back causes the live price to stretch the Y-axis permanently.
                    let forming_price_range = if chart.basis.is_odb()
                        && start_interval == 0
                        && chart.layout.include_forming
                    {
                        self.odb_processor.as_ref().and_then(|p| {
                            p.get_incomplete_bar()
                                .map(|b| (b.low.to_f64() as f32, b.high.to_f64() as f32))
                        })
                    } else {
                        None
                    };

                    let price_range = self
                        .data_source
                        .visible_price_range(start_interval, end_interval)
                        .map(|(mut lo, mut hi)| {
                            if let Some((f_lo, f_hi)) = forming_price_range {
                                lo = lo.min(f_lo);
                                hi = hi.max(f_hi);
                            }
                            (lo, hi)
                        })
                        .or({
                            // No completed bars visible — scale to forming bar alone.
                            forming_price_range
                        });

                    if let Some((lowest, highest)) = price_range {
                        let padding = (highest - lowest) * 0.05;
                        let price_span = (highest - lowest) + (2.0 * padding);

                        if price_span > 0.0 && chart.bounds.height > f32::EPSILON {
                            let padded_highest = highest + padding;
                            let chart_height = chart.bounds.height;
                            let tick_size = chart.tick_size.to_f32_lossy();

                            if tick_size > 0.0 {
                                chart.cell_height = (chart_height * tick_size) / price_span;
                                chart.base_price_y = Price::from_f32(padded_highest);
                                chart.translation.y = -chart_height / 2.0;
                            }
                        }
                    }
                }
            }
        }

        chart.cache.clear_all();
        for indi in self.indicators.values_mut().filter_map(Option::as_mut) {
            indi.clear_all_caches();
        }

        // Trade feed liveness watchdog (dead-man's switch).
        // Fires in invalidate() which runs every frame, unlike the telemetry
        // window in insert_trades_inner() which never fires when trades stop.
        if self.last_trade_received_ms > 0 {
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let stale_ms = now_ms.saturating_sub(self.last_trade_received_ms);
            if stale_ms > 90_000 && !self.trade_feed_dead_alerted {
                self.trade_feed_dead_alerted = true;
                let stale_secs = stale_ms / 1000;
                log::error!(
                    "[watchdog] No WS trades for {stale_secs}s — feed may be dead. \
                     last_agg_id={:?}, ch_reconcile={}",
                    self.last_ws_agg_trade_id,
                    self.ch_reconcile_count,
                );
                if exchange::telegram::is_configured() {
                    let msg = format!(
                        "No WS trades for {stale_secs}s. last_agg_id={:?}, ch_reconcile={}",
                        self.last_ws_agg_trade_id, self.ch_reconcile_count,
                    );
                    tokio::spawn(async move {
                        exchange::telegram::alert(
                            exchange::telegram::Severity::Critical,
                            "trade-watchdog",
                            &msg,
                        )
                        .await;
                    });
                }
            }
        }

        // ── Sentinel: bar-level agg_trade_id continuity audit (every 60s, ODB only) ──
        if self.chart.basis.is_odb()
            && !self.fetching_trades.0
            && let Some(t) = now
            && t.duration_since(self.last_sentinel_audit) >= std::time::Duration::from_secs(60)
        {
            self.last_sentinel_audit = t;
            let anomalies = self.audit_bar_continuity();

            // Partition: healable gaps (re-fetch can fix) vs structural (day-boundary, overlaps)
            let healable: Vec<_> = anomalies
                .iter()
                .filter(|g| g.kind == BarGapKind::Gap)
                .collect();
            let day_boundary: Vec<_> = anomalies
                .iter()
                .filter(|g| g.kind == BarGapKind::DayBoundary)
                .collect();
            let overlaps: Vec<_> = anomalies
                .iter()
                .filter(|g| g.kind == BarGapKind::Overlap)
                .collect();

            let total_anomalies = anomalies.len();

            if total_anomalies > 0 && total_anomalies != self.sentinel_gap_count {
                self.sentinel_gap_count = total_anomalies;

                // Log all anomaly types
                if !healable.is_empty() {
                    let missing: u64 = healable.iter().map(|g| g.missing_count).sum();
                    log::warn!(
                        "[sentinel] {} healable gaps ({} missing agg_trade_ids)",
                        healable.len(),
                        missing,
                    );
                    for (i, gap) in healable.iter().take(3).enumerate() {
                        log::warn!(
                            "[sentinel]   gap {}: prev_last={} curr_first={} missing={}",
                            i + 1,
                            gap.prev_last_id,
                            gap.curr_first_id,
                            gap.missing_count,
                        );
                    }
                }
                if !day_boundary.is_empty() {
                    let missing: u64 = day_boundary.iter().map(|g| g.missing_count).sum();
                    log::info!(
                        "[sentinel] {} day-boundary gaps ({} missing IDs, structural — kintsugi domain)",
                        day_boundary.len(),
                        missing,
                    );
                }
                if !overlaps.is_empty() {
                    let overlap_total: u64 = overlaps.iter().map(|g| g.missing_count).sum();
                    log::warn!(
                        "[sentinel] {} overlapping bar pairs ({} shared agg_trade_ids)",
                        overlaps.len(),
                        overlap_total,
                    );
                    for (i, gap) in overlaps.iter().take(3).enumerate() {
                        log::warn!(
                            "[sentinel]   overlap {}: prev_last={} curr_first={} shared={}",
                            i + 1,
                            gap.prev_last_id,
                            gap.curr_first_id,
                            gap.missing_count,
                        );
                    }
                }

                // Telegram: only alert for healable gaps or overlaps (not day-boundary)
                if exchange::telegram::is_configured()
                    && (!healable.is_empty() || !overlaps.is_empty())
                {
                    let mut detail = String::new();
                    if !healable.is_empty() {
                        let missing: u64 = healable.iter().map(|g| g.missing_count).sum();
                        detail.push_str(&format!(
                            "{} healable gaps ({} missing IDs)\n",
                            healable.len(),
                            missing,
                        ));
                        for (i, gap) in healable.iter().take(5).enumerate() {
                            let secs = gap.bar_time_ms / 1000;
                            let nanos = ((gap.bar_time_ms % 1000) * 1_000_000) as u32;
                            let dt = chrono::DateTime::from_timestamp(secs as i64, nanos)
                                .map(|d| d.format("%Y-%m-%dT%H:%M UTC").to_string())
                                .unwrap_or_else(|| gap.bar_time_ms.to_string());
                            detail.push_str(&format!(
                                "\n{}. prev={} → curr={}\n   ({} missing, bar_time={})",
                                i + 1,
                                gap.prev_last_id,
                                gap.curr_first_id,
                                gap.missing_count,
                                dt,
                            ));
                        }
                    }
                    if !overlaps.is_empty() {
                        if !detail.is_empty() {
                            detail.push_str("\n\n");
                        }
                        let overlap_total: u64 = overlaps.iter().map(|g| g.missing_count).sum();
                        detail.push_str(&format!(
                            "{} overlapping bar pairs ({} shared IDs)",
                            overlaps.len(),
                            overlap_total,
                        ));
                    }
                    if !day_boundary.is_empty() {
                        detail.push_str(&format!(
                            "\n\n({} day-boundary gaps omitted — structural)",
                            day_boundary.len(),
                        ));
                    }
                    if !healable.is_empty() {
                        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
                        let today_midnight_ms = (now_ms / 86_400_000) * 86_400_000;
                        let min_gap_time =
                            healable.iter().map(|g| g.prev_bar_time_ms).min().unwrap_or(0);
                        if min_gap_time >= today_midnight_ms {
                            detail.push_str(
                                "\n\nLive-session gap — OdbCatchup handles (no CH refetch)",
                            );
                        } else {
                            detail.push_str("\n\nTriggering CH kline re-fetch...");
                        }
                    }

                    tokio::spawn(async move {
                        exchange::telegram::alert(
                            exchange::telegram::Severity::Warning,
                            "sentinel",
                            &detail,
                        )
                        .await;
                    });
                }

                // Only trigger re-fetch for healable gaps (not day-boundary or overlaps)
                if !healable.is_empty() && !self.sentinel_refetch_pending {
                    self.sentinel_refetch_pending = true;
                    // Use prev_bar_time_ms (the OLDER side of the gap) to determine if the
                    // gap has historical CH coverage. bar_time_ms (newer side) can be in
                    // today's live session even when the gap itself spans historical data.
                    self.sentinel_healable_gap_min_time_ms =
                        healable.iter().map(|g| g.prev_bar_time_ms).min();
                    log::info!(
                        "[sentinel] triggering kline re-fetch to heal {} gaps",
                        healable.len()
                    );
                }
            } else if total_anomalies == 0 && self.sentinel_gap_count > 0 {
                let prev_count = self.sentinel_gap_count;
                self.sentinel_gap_count = 0;
                self.sentinel_refetch_pending = false;
                self.sentinel_healable_gap_min_time_ms = None;

                log::info!("[sentinel] all {} previous anomalies healed", prev_count);

                if exchange::telegram::is_configured() {
                    let msg = format!(
                        "All {} inter-bar anomalies healed (kintsugi repair confirmed)",
                        prev_count,
                    );
                    tokio::spawn(async move {
                        exchange::telegram::alert(
                            exchange::telegram::Severity::Recovery,
                            "sentinel",
                            &msg,
                        )
                        .await;
                    });
                }
            }
        }

        #[cfg(feature = "telemetry")]
        if let Some(t) = now {
            // Emit ChartSnapshot every ~30s for ODB charts.
            // Uses `last_snapshot` (not `last_tick`) so per-frame updates don't reset the timer.
            if self.chart.basis.is_odb()
                && t.duration_since(self.last_snapshot) >= std::time::Duration::from_secs(30)
            {
                self.last_snapshot = t;
                use data::telemetry::{self, TelemetryEvent};
                if let PlotData::TickBased(ref tick_aggr) = self.data_source {
                    let telem_dbps = if let data::chart::Basis::Odb(d) = self.chart.basis {
                        d
                    } else {
                        0
                    };
                    let forming_ts = self
                        .odb_processor
                        .as_ref()
                        .and_then(|p| p.get_incomplete_bar())
                        .map(|b| (b.close_time / 1000) as u64); // µs → ms
                    telemetry::emit(TelemetryEvent::ChartSnapshot {
                        ts_ms: telemetry::now_ms(),
                        symbol: self.chart.ticker_info.ticker.to_string(),
                        threshold_dbps: telem_dbps,
                        total_bars: tick_aggr.datapoints.len(),
                        visible_bars: if self.chart.cell_width > 0.0 {
                            (self.chart.bounds.width / self.chart.cell_width).ceil() as usize
                        } else {
                            0
                        },
                        newest_bar_ts: tick_aggr
                            .datapoints
                            .last()
                            .map(|dp| dp.kline.time)
                            .unwrap_or(0),
                        oldest_bar_ts: tick_aggr
                            .datapoints
                            .first()
                            .map(|dp| dp.kline.time)
                            .unwrap_or(0),
                        forming_bar_ts: forming_ts,
                        rbp_completed_count: self.odb_completed_count,
                    });
                }
            }
        }

        if let Some(t) = now {
            self.last_tick = t;
            self.missing_data_task()
        } else {
            None
        }
    }

    pub fn toggle_indicator(&mut self, indicator: KlineIndicator) {
        // Count only panel indicators (TradeIntensityHeatmap colours candles, not a panel).
        let prev_panel_count = self
            .indicators
            .iter()
            .filter(|(k, v)| v.is_some() && k.has_subplot())
            .count();

        if self.indicators[indicator].is_some() {
            self.indicators[indicator] = None;
        } else {
            let mut box_indi = make_indicator_with_config(indicator, &self.kline_config);
            box_indi.rebuild_from_source(&self.data_source);
            self.indicators[indicator] = Some(box_indi);
        }

        if let Some(main_split) = self.chart.layout.splits.first() {
            let current_panel_count = self
                .indicators
                .iter()
                .filter(|(k, v)| v.is_some() && k.has_subplot())
                .count();
            self.chart.layout.splits = data::util::calc_panel_splits(
                *main_split,
                current_panel_count,
                Some(prev_panel_count),
            );
        }
    }
}

impl canvas::Program<Message> for KlineChart {
    type State = Interaction;

    fn update(
        &self,
        interaction: &mut Interaction,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        // Track Shift key state for bar selection (ODB-only).
        if let Event::Keyboard(keyboard::Event::ModifiersChanged(mods)) = event {
            self.bar_selection.borrow_mut().shift_held = mods.shift();
        }

        // ODB-only: Shift+Left Click cycles anchor → end → restart anchor.
        if self.chart.basis.is_odb() {
            let shift_held = self.bar_selection.borrow().shift_held;
            if shift_held
                && let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event
            {
                if let Some(cursor_pos) = cursor.position_in(bounds) {
                    let bounds_size = bounds.size();
                    let region = self.chart.visible_region(bounds_size);
                    let (visual_idx, _) =
                        self.chart.snap_x_to_index(cursor_pos.x, bounds_size, region);
                    if visual_idx != u64::MAX {
                        let mut sel = self.bar_selection.borrow_mut();
                        match (sel.anchor, sel.end) {
                            (None, _) => sel.anchor = Some(visual_idx as usize),
                            (Some(_), None) => sel.end = Some(visual_idx as usize),
                            // Third Shift+Click: restart from new anchor.
                            (Some(_), Some(_)) => {
                                sel.anchor = Some(visual_idx as usize);
                                sel.end = None;
                            }
                        }
                        self.chart.cache.clear_all();
                    }
                }
                return Some(canvas::Action::request_redraw().and_capture());
            }
        }
        super::canvas_interaction(self, interaction, event, bounds, cursor)
    }

    fn draw(
        &self,
        interaction: &Interaction,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let chart = self.state();

        if chart.bounds.width == 0.0 {
            return vec![];
        }

        let bounds_size = bounds.size();
        let palette = theme.extended_palette();

        let klines = chart.cache.main.draw(renderer, bounds_size, |frame| {
            let center = Vector::new(bounds.width / 2.0, bounds.height / 2.0);

            frame.translate(center);
            frame.scale(chart.scaling);
            frame.translate(chart.translation);

            let region = chart.visible_region(frame.size());
            let (earliest, latest) = chart.interval_range(&region);

            let price_to_y = |price| chart.price_to_y(price);
            let interval_to_x = |interval| chart.interval_to_x(interval);

            match &self.kind {
                KlineChartKind::Footprint {
                    clusters,
                    scaling,
                    studies,
                } => {
                    let (highest, lowest) = chart.price_range(&region);

                    let max_cluster_qty = self.calc_qty_scales(
                        earliest,
                        latest,
                        highest,
                        lowest,
                        chart.tick_size,
                        *clusters,
                    );

                    let cell_height_unscaled = chart.cell_height * chart.scaling;
                    let cell_width_unscaled = chart.cell_width * chart.scaling;

                    let text_size = {
                        let text_size_from_height = cell_height_unscaled.round().min(16.0) - 3.0;
                        let text_size_from_width =
                            (cell_width_unscaled * 0.1).round().min(16.0) - 3.0;

                        text_size_from_height.min(text_size_from_width)
                    };

                    let candle_width = 0.1 * chart.cell_width;
                    let content_spacing = ContentGaps::from_view(candle_width, chart.scaling);

                    let imbalance = studies.iter().find_map(|study| {
                        if let FootprintStudy::Imbalance {
                            threshold,
                            color_scale,
                            ignore_zeros,
                        } = study
                        {
                            Some((*threshold, *color_scale, *ignore_zeros))
                        } else {
                            None
                        }
                    });

                    let show_text = {
                        let min_w = match clusters {
                            ClusterKind::VolumeProfile | ClusterKind::DeltaProfile => 80.0,
                            ClusterKind::BidAsk => 120.0,
                        };
                        should_show_text(cell_height_unscaled, cell_width_unscaled, min_w)
                    };

                    draw_all_npocs(
                        &self.data_source,
                        frame,
                        price_to_y,
                        interval_to_x,
                        candle_width,
                        chart.cell_width,
                        chart.cell_height,
                        palette,
                        studies,
                        earliest,
                        latest,
                        *clusters,
                        content_spacing,
                        imbalance.is_some(),
                    );

                    render_data_source(
                        &self.data_source,
                        frame,
                        earliest,
                        latest,
                        interval_to_x,
                        |frame, _visual_idx, x_position, kline, trades| {
                            let cluster_scaling =
                                effective_cluster_qty(*scaling, max_cluster_qty, trades, *clusters);

                            draw_clusters(
                                frame,
                                price_to_y,
                                x_position,
                                chart.cell_width,
                                chart.cell_height,
                                candle_width,
                                cluster_scaling,
                                palette,
                                text_size,
                                self.tick_size(),
                                show_text,
                                imbalance,
                                kline,
                                trades,
                                *clusters,
                                content_spacing,
                            );
                        },
                    );
                }
                KlineChartKind::Candles | KlineChartKind::Odb => {
                    // Session lines (behind candles)
                    if self.kline_config.show_sessions {
                        super::session::draw_sessions(
                            frame,
                            &region,
                            &chart.basis,
                            chart.cell_width,
                            interval_to_x,
                            &self.data_source,
                            earliest,
                            latest,
                        );
                    }

                    // Bar range selection highlight (ODB only, behind candles).
                    if chart.basis.is_odb() {
                        let sel = self.bar_selection.borrow();
                        if let Some(anchor) = sel.anchor {
                            let end = sel.end.unwrap_or(anchor);
                            let (lo, hi) = (anchor.min(end), anchor.max(end));
                            let x_right = interval_to_x(lo as u64) + chart.cell_width / 2.0;
                            let x_left = interval_to_x(hi as u64) - chart.cell_width / 2.0;
                            frame.fill_rectangle(
                                Point::new(x_left, region.y),
                                Size::new((x_right - x_left).max(0.0), region.height),
                                iced::Color { r: 1.0, g: 1.0, b: 0.3, a: 0.10 },
                            );
                        }
                    }

                    // ODB bars represent continuous price action — use tighter
                    // spacing (95%) so bars visually connect. Candles keep 80%
                    // for temporal separation between time periods.
                    let candle_fill = if chart.basis.is_odb() { 0.95 } else { 0.8 };
                    let candle_width = chart.cell_width * candle_fill;
                    // Look up heatmap indicator once for thermal candle body colouring.
                    let heatmap_indi =
                        self.indicators[KlineIndicator::TradeIntensityHeatmap].as_deref();
                    let total_len = if let PlotData::TickBased(t) = &self.data_source {
                        t.datapoints.len()
                    } else {
                        0
                    };
                    // Divergence detection: heatmap data length vs datapoints length.
                    // delta=-1 is normal (forming bar has no completed microstructure yet).
                    // Only |delta| > 1 indicates a real sync issue.
                    if let Some(h) = heatmap_indi {
                        let heatmap_len = h.data_len();
                        let delta = heatmap_len as isize - total_len as isize;
                        if delta.unsigned_abs() > 1 {
                            log::warn!(
                                "[intensity-diverge] heatmap_data={} != dp_count={} \
                                 (delta={delta}) → colors may map to wrong bars",
                                heatmap_len,
                                total_len,
                            );
                            exchange::tg_alert!(
                                exchange::telegram::Severity::Warning,
                                "intensity",
                                "Intensity heatmap divergence"
                            );
                        }
                    }

                    let thermal_wicks = self.kline_config.thermal_wicks;
                    render_data_source(
                        &self.data_source,
                        frame,
                        earliest,
                        latest,
                        interval_to_x,
                        |frame, visual_idx, x_position, kline, _| {
                            // visual_idx 0 = newest = highest storage index
                            let thermal_color = heatmap_indi.and_then(|h| {
                                let storage_idx = total_len.saturating_sub(1 + visual_idx);
                                h.thermal_body_color(storage_idx as u64)
                            });
                            // Wick: same thermal colour as body when thermal_wicks=true,
                            // otherwise falls back to direction green/red (None → unwrap_or).
                            let wick_color = if thermal_wicks { thermal_color } else { None };
                            draw_candle_dp(
                                frame,
                                price_to_y,
                                candle_width,
                                palette,
                                x_position,
                                kline,
                                thermal_color,
                                wick_color,
                            );
                        },
                    );

                    // Render the in-process forming bar (ODB bars only).
                    // Drawn at x = +cell_width (one slot right of index-0 = newest completed bar).
                    // Semi-transparent to signal it is still accumulating.
                    if chart.basis.is_odb()
                        && let Some(ref processor) = self.odb_processor
                        && let Some(forming) = processor.get_incomplete_bar()
                    {
                        let x_forming = chart.cell_width;
                        let open_f32 = forming.open.to_f64() as f32;
                        let high_f32 = forming.high.to_f64() as f32;
                        let low_f32 = forming.low.to_f64() as f32;
                        let close_f32 = forming.close.to_f64() as f32;

                        let direction_color = if close_f32 >= open_f32 {
                            palette.success.base.color
                        } else {
                            palette.danger.base.color
                        };
                        let forming_color = iced::Color {
                            a: 0.4,
                            ..direction_color
                        };

                        let y_open = price_to_y(Price::from_f32(open_f32));
                        let y_high = price_to_y(Price::from_f32(high_f32));
                        let y_low = price_to_y(Price::from_f32(low_f32));
                        let y_close = price_to_y(Price::from_f32(close_f32));

                        // Body
                        frame.fill_rectangle(
                            Point::new(x_forming - candle_width / 2.0, y_open.min(y_close)),
                            Size::new(candle_width, (y_open - y_close).abs().max(1.0)),
                            forming_color,
                        );
                        // Wick
                        frame.fill_rectangle(
                            Point::new(x_forming - candle_width / 8.0, y_high),
                            Size::new(candle_width / 4.0, (y_high - y_low).abs()),
                            forming_color,
                        );
                    }

                    // Draw overlay indicators (e.g. ZigZag) on the main candle pane.
                    for (_kind, indi) in &self.indicators {
                        if let Some(indi) = indi.as_ref() {
                            indi.draw_overlay(
                                frame,
                                total_len,
                                earliest as usize,
                                latest as usize,
                                &price_to_y,
                                &interval_to_x,
                                palette,
                            );
                        }
                    }
                }
            }

            chart.draw_last_price_line(frame, palette, region);
        });

        let watermark =
            super::draw_watermark(&chart.cache.watermark, renderer, bounds_size, palette);

        // Screen-space legend overlay — drawn after watermark, before crosshair so
        // the crosshair tooltip always appears on top.
        let legend = chart.cache.legend.draw(renderer, bounds_size, |frame| {
            if let Some(heatmap) = self.indicators[KlineIndicator::TradeIntensityHeatmap].as_deref()
            {
                heatmap.draw_screen_legend(frame);
            }
            // Bar selection stats overlay (ODB only, shown when both anchor+end are set).
            if chart.basis.is_odb() {
                let sel = self.bar_selection.borrow();
                if let (Some(anchor), Some(end)) = (sel.anchor, sel.end)
                    && let PlotData::TickBased(tick_aggr) = &self.data_source
                {
                    draw_bar_selection_stats(frame, palette, tick_aggr, anchor, end);
                }
            }
        });

        let crosshair = chart.cache.crosshair.draw(renderer, bounds_size, |frame| {
            if let Some(cursor_position) = cursor.position_in(bounds) {
                let (_, rounded_aggregation) =
                    chart.draw_crosshair(frame, theme, bounds_size, cursor_position, interaction);

                // Build forming bar Kline from odb_processor for tooltip
                let forming_kline = if rounded_aggregation == u64::MAX {
                    let fk = self
                        .odb_processor
                        .as_ref()
                        .and_then(|p| p.get_incomplete_bar())
                        .map(|bar| odb_to_kline(&bar, chart.ticker_info.min_ticksize));
                    log::trace!("[XHAIR] forming bar zone: forming_kline={}", fk.is_some());
                    fk
                } else {
                    None
                };

                draw_crosshair_tooltip(
                    &self.data_source,
                    &chart.ticker_info,
                    frame,
                    palette,
                    rounded_aggregation,
                    chart.basis,
                    chart.timezone.get(),
                    forming_kline.as_ref(),
                );
            }
        });

        vec![klines, watermark, legend, crosshair]
    }

    fn mouse_interaction(
        &self,
        interaction: &Interaction,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        match interaction {
            Interaction::Panning { .. } => mouse::Interaction::Grabbing,
            Interaction::Zoomin { .. } => mouse::Interaction::ZoomIn,
            Interaction::None | Interaction::Ruler { .. } => {
                if cursor.is_over(bounds) {
                    mouse::Interaction::Crosshair
                } else {
                    mouse::Interaction::default()
                }
            }
        }
    }
}

fn draw_footprint_kline(
    frame: &mut canvas::Frame,
    price_to_y: impl Fn(Price) -> f32,
    x_position: f32,
    candle_width: f32,
    kline: &Kline,
    palette: &Extended,
) {
    let y_open = price_to_y(kline.open);
    let y_high = price_to_y(kline.high);
    let y_low = price_to_y(kline.low);
    let y_close = price_to_y(kline.close);

    let body_color = if kline.close >= kline.open {
        palette.success.weak.color
    } else {
        palette.danger.weak.color
    };
    frame.fill_rectangle(
        Point::new(x_position - (candle_width / 8.0), y_open.min(y_close)),
        Size::new(candle_width / 4.0, (y_open - y_close).abs()),
        body_color,
    );

    let wick_color = if kline.close >= kline.open {
        palette.success.weak.color
    } else {
        palette.danger.weak.color
    };
    let marker_line = Stroke::with_color(
        Stroke {
            width: 1.0,
            ..Default::default()
        },
        wick_color.scale_alpha(0.6),
    );
    frame.stroke(
        &Path::line(
            Point::new(x_position, y_high),
            Point::new(x_position, y_low),
        ),
        marker_line,
    );
}

pub(crate) fn draw_candle_dp(
    frame: &mut canvas::Frame,
    price_to_y: impl Fn(Price) -> f32,
    candle_width: f32,
    palette: &Extended,
    x_position: f32,
    kline: &Kline,
    thermal_body_color: Option<iced::Color>,
    thermal_wick_color: Option<iced::Color>,
) {
    let y_open = price_to_y(kline.open);
    let y_high = price_to_y(kline.high);
    let y_low = price_to_y(kline.low);
    let y_close = price_to_y(kline.close);

    let direction_color = if kline.close >= kline.open {
        palette.success.base.color
    } else {
        palette.danger.base.color
    };

    // Body: thermal colour when heatmap active, otherwise green/red direction.
    let body_color = thermal_body_color.unwrap_or(direction_color);
    frame.fill_rectangle(
        Point::new(x_position - (candle_width / 2.0), y_open.min(y_close)),
        Size::new(candle_width, (y_open - y_close).abs()),
        body_color,
    );

    // Wick: thermal colour (merged) or green/red direction, per "Thermal Wicks" setting.
    let wick_color = thermal_wick_color.unwrap_or(direction_color);
    frame.fill_rectangle(
        Point::new(x_position - (candle_width / 8.0), y_high),
        Size::new(candle_width / 4.0, (y_high - y_low).abs()),
        wick_color,
    );
}

fn render_data_source<F>(
    data_source: &PlotData<KlineDataPoint>,
    frame: &mut canvas::Frame,
    earliest: u64,
    latest: u64,
    interval_to_x: impl Fn(u64) -> f32,
    draw_fn: F,
) where
    F: Fn(&mut canvas::Frame, usize, f32, &Kline, &KlineTrades),
{
    match data_source {
        PlotData::TickBased(tick_aggr) => {
            let earliest = earliest as usize;
            let latest = latest as usize;

            tick_aggr
                .datapoints
                .iter()
                .rev()
                .enumerate()
                .filter(|(index, _)| *index <= latest && *index >= earliest)
                .for_each(|(index, tick_aggr)| {
                    let x_position = interval_to_x(index as u64);

                    draw_fn(
                        frame,
                        index,
                        x_position,
                        &tick_aggr.kline,
                        &tick_aggr.footprint,
                    );
                });
        }
        PlotData::TimeBased(timeseries) => {
            if latest < earliest {
                return;
            }

            timeseries
                .datapoints
                .range(earliest..=latest)
                .for_each(|(timestamp, dp)| {
                    let x_position = interval_to_x(*timestamp);

                    draw_fn(frame, 0, x_position, &dp.kline, &dp.footprint);
                });
        }
    }
}

fn draw_all_npocs(
    data_source: &PlotData<KlineDataPoint>,
    frame: &mut canvas::Frame,
    price_to_y: impl Fn(Price) -> f32,
    interval_to_x: impl Fn(u64) -> f32,
    candle_width: f32,
    cell_width: f32,
    cell_height: f32,
    palette: &Extended,
    studies: &[FootprintStudy],
    visible_earliest: u64,
    visible_latest: u64,
    cluster_kind: ClusterKind,
    spacing: ContentGaps,
    imb_study_on: bool,
) {
    let Some(lookback) = studies.iter().find_map(|study| {
        if let FootprintStudy::NPoC { lookback } = study {
            Some(*lookback)
        } else {
            None
        }
    }) else {
        return;
    };

    let (filled_color, naked_color) = (
        palette.background.strong.color,
        if palette.is_dark {
            palette.warning.weak.color.scale_alpha(0.5)
        } else {
            palette.warning.strong.color
        },
    );

    let line_height = cell_height.min(1.0);

    let bar_width_factor: f32 = 0.9;
    let inset = (cell_width * (1.0 - bar_width_factor)) / 2.0;

    let candle_lane_factor: f32 = match cluster_kind {
        ClusterKind::VolumeProfile | ClusterKind::DeltaProfile => 0.25,
        ClusterKind::BidAsk => 1.0,
    };

    let start_x_for = |cell_center_x: f32| -> f32 {
        match cluster_kind {
            ClusterKind::BidAsk => cell_center_x + (candle_width / 2.0) + spacing.candle_to_cluster,
            ClusterKind::VolumeProfile | ClusterKind::DeltaProfile => {
                let content_left = (cell_center_x - (cell_width / 2.0)) + inset;
                let candle_lane_left = content_left
                    + if imb_study_on {
                        candle_width + spacing.marker_to_candle
                    } else {
                        0.0
                    };
                candle_lane_left + candle_width * candle_lane_factor + spacing.candle_to_cluster
            }
        }
    };

    let wick_x_for = |cell_center_x: f32| -> f32 {
        match cluster_kind {
            ClusterKind::BidAsk => cell_center_x, // not used for BidAsk clustering
            ClusterKind::VolumeProfile | ClusterKind::DeltaProfile => {
                let content_left = (cell_center_x - (cell_width / 2.0)) + inset;
                let candle_lane_left = content_left
                    + if imb_study_on {
                        candle_width + spacing.marker_to_candle
                    } else {
                        0.0
                    };
                candle_lane_left + (candle_width * candle_lane_factor) / 2.0
                    - (spacing.candle_to_cluster * 0.5)
            }
        }
    };

    let end_x_for = |cell_center_x: f32| -> f32 {
        match cluster_kind {
            ClusterKind::BidAsk => cell_center_x - (candle_width / 2.0) - spacing.candle_to_cluster,
            ClusterKind::VolumeProfile | ClusterKind::DeltaProfile => wick_x_for(cell_center_x),
        }
    };

    let rightmost_cell_center_x = {
        let earliest_x = interval_to_x(visible_earliest);
        let latest_x = interval_to_x(visible_latest);
        if earliest_x > latest_x {
            earliest_x
        } else {
            latest_x
        }
    };

    let mut draw_the_line = |interval: u64, poc: &PointOfControl| {
        let start_x = start_x_for(interval_to_x(interval));

        let (line_width, color) = match poc.status {
            NPoc::Naked => {
                let end_x = end_x_for(rightmost_cell_center_x);
                let line_width = end_x - start_x;
                if line_width.abs() <= cell_width {
                    return;
                }
                (line_width, naked_color)
            }
            NPoc::Filled { at } => {
                let end_x = end_x_for(interval_to_x(at));
                let line_width = end_x - start_x;
                if line_width.abs() <= cell_width {
                    return;
                }
                (line_width, filled_color)
            }
            _ => return,
        };

        frame.fill_rectangle(
            Point::new(start_x, price_to_y(poc.price) - line_height / 2.0),
            Size::new(line_width, line_height),
            color,
        );
    };

    match data_source {
        PlotData::TickBased(tick_aggr) => {
            tick_aggr
                .datapoints
                .iter()
                .rev()
                .enumerate()
                .take(lookback)
                .filter_map(|(index, dp)| dp.footprint.poc.as_ref().map(|poc| (index as u64, poc)))
                .for_each(|(interval, poc)| draw_the_line(interval, poc));
        }
        PlotData::TimeBased(timeseries) => {
            timeseries
                .datapoints
                .iter()
                .rev()
                .take(lookback)
                .filter_map(|(timestamp, dp)| {
                    dp.footprint.poc.as_ref().map(|poc| (*timestamp, poc))
                })
                .for_each(|(interval, poc)| draw_the_line(interval, poc));
        }
    }
}

fn effective_cluster_qty(
    scaling: ClusterScaling,
    visible_max: f32,
    footprint: &KlineTrades,
    cluster_kind: ClusterKind,
) -> f32 {
    let individual_max = match cluster_kind {
        ClusterKind::BidAsk => footprint
            .trades
            .values()
            .map(|group| group.buy_qty.max(group.sell_qty))
            .max()
            .unwrap_or_default(),
        ClusterKind::DeltaProfile => footprint
            .trades
            .values()
            .map(|group| group.buy_qty.abs_diff(group.sell_qty))
            .max()
            .unwrap_or_default(),
        ClusterKind::VolumeProfile => footprint
            .trades
            .values()
            .map(|group| group.buy_qty + group.sell_qty)
            .max()
            .unwrap_or_default(),
    };
    let individual_max_f32 = f32::from(individual_max);

    match scaling {
        ClusterScaling::VisibleRange => Qty::scale_or_one(visible_max),
        ClusterScaling::Datapoint => individual_max.to_scale_or_one(),
        ClusterScaling::Hybrid { weight } => {
            let w = weight.clamp(0.0, 1.0);
            Qty::scale_or_one(visible_max * w + individual_max_f32 * (1.0 - w))
        }
    }
}

fn draw_clusters(
    frame: &mut canvas::Frame,
    price_to_y: impl Fn(Price) -> f32,
    x_position: f32,
    cell_width: f32,
    cell_height: f32,
    candle_width: f32,
    max_cluster_qty: f32,
    palette: &Extended,
    text_size: f32,
    tick_size: f32,
    show_text: bool,
    imbalance: Option<(usize, Option<usize>, bool)>,
    kline: &Kline,
    footprint: &KlineTrades,
    cluster_kind: ClusterKind,
    spacing: ContentGaps,
) {
    let text_color = palette.background.weakest.text;

    let bar_width_factor: f32 = 0.9;
    let inset = (cell_width * (1.0 - bar_width_factor)) / 2.0;

    let cell_left = x_position - (cell_width / 2.0);
    let content_left = cell_left + inset;
    let content_right = x_position + (cell_width / 2.0) - inset;

    match cluster_kind {
        ClusterKind::VolumeProfile | ClusterKind::DeltaProfile => {
            let area = ProfileArea::new(
                content_left,
                content_right,
                candle_width,
                spacing,
                imbalance.is_some(),
            );
            let bar_alpha = if show_text { 0.25 } else { 1.0 };

            for (price, group) in &footprint.trades {
                let buy_qty = f32::from(group.buy_qty);
                let sell_qty = f32::from(group.sell_qty);
                let y = price_to_y(*price);

                match cluster_kind {
                    ClusterKind::VolumeProfile => {
                        super::draw_volume_bar(
                            frame,
                            area.bars_left,
                            y,
                            buy_qty,
                            sell_qty,
                            max_cluster_qty,
                            area.bars_width,
                            cell_height,
                            palette.success.base.color,
                            palette.danger.base.color,
                            bar_alpha,
                            true,
                        );

                        if show_text {
                            draw_cluster_text(
                                frame,
                                &abbr_large_numbers(f32::from(group.total_qty())),
                                Point::new(area.bars_left, y),
                                text_size,
                                text_color,
                                Alignment::Start,
                                Alignment::Center,
                            );
                        }
                    }
                    ClusterKind::DeltaProfile => {
                        let delta = f32::from(group.delta_qty());
                        if show_text {
                            draw_cluster_text(
                                frame,
                                &abbr_large_numbers(delta),
                                Point::new(area.bars_left, y),
                                text_size,
                                text_color,
                                Alignment::Start,
                                Alignment::Center,
                            );
                        }

                        let bar_width = (delta.abs() / max_cluster_qty) * area.bars_width;
                        if bar_width > 0.0 {
                            let color = if delta >= 0.0 {
                                palette.success.base.color.scale_alpha(bar_alpha)
                            } else {
                                palette.danger.base.color.scale_alpha(bar_alpha)
                            };
                            frame.fill_rectangle(
                                Point::new(area.bars_left, y - (cell_height / 2.0)),
                                Size::new(bar_width, cell_height),
                                color,
                            );
                        }
                    }
                    _ => {}
                }

                if let Some((threshold, color_scale, ignore_zeros)) = imbalance {
                    let step = PriceStep::from_f32(tick_size);
                    let higher_price =
                        Price::from_f32(price.to_f32() + tick_size).round_to_step(step);

                    let rect_w = ((area.imb_marker_width - 1.0) / 2.0).max(1.0);
                    let buyside_x = area.imb_marker_left + area.imb_marker_width - rect_w;
                    let sellside_x =
                        area.imb_marker_left + area.imb_marker_width - (2.0 * rect_w) - 1.0;

                    draw_imbalance_markers(
                        frame,
                        &price_to_y,
                        footprint,
                        *price,
                        sell_qty,
                        higher_price,
                        threshold,
                        color_scale,
                        ignore_zeros,
                        cell_height,
                        palette,
                        buyside_x,
                        sellside_x,
                        rect_w,
                    );
                }
            }

            draw_footprint_kline(
                frame,
                &price_to_y,
                area.candle_center_x,
                candle_width,
                kline,
                palette,
            );
        }
        ClusterKind::BidAsk => {
            let area = BidAskArea::new(
                x_position,
                content_left,
                content_right,
                candle_width,
                spacing,
            );

            let bar_alpha = if show_text { 0.25 } else { 1.0 };

            let imb_marker_reserve = if imbalance.is_some() {
                ((area.imb_marker_width - 1.0) / 2.0).max(1.0)
            } else {
                0.0
            };

            let right_max_x =
                area.bid_area_right - imb_marker_reserve - (2.0 * spacing.marker_to_bars);
            let right_area_width = (right_max_x - area.bid_area_left).max(0.0);

            let left_min_x =
                area.ask_area_left + imb_marker_reserve + (2.0 * spacing.marker_to_bars);
            let left_area_width = (area.ask_area_right - left_min_x).max(0.0);

            for (price, group) in &footprint.trades {
                let buy_qty = f32::from(group.buy_qty);
                let sell_qty = f32::from(group.sell_qty);
                let y = price_to_y(*price);

                if buy_qty > 0.0 && right_area_width > 0.0 {
                    if show_text {
                        draw_cluster_text(
                            frame,
                            &abbr_large_numbers(buy_qty),
                            Point::new(area.bid_area_left, y),
                            text_size,
                            text_color,
                            Alignment::Start,
                            Alignment::Center,
                        );
                    }

                    let bar_width = (buy_qty / max_cluster_qty) * right_area_width;
                    if bar_width > 0.0 {
                        frame.fill_rectangle(
                            Point::new(area.bid_area_left, y - (cell_height / 2.0)),
                            Size::new(bar_width, cell_height),
                            palette.success.base.color.scale_alpha(bar_alpha),
                        );
                    }
                }
                if sell_qty > 0.0 && left_area_width > 0.0 {
                    if show_text {
                        draw_cluster_text(
                            frame,
                            &abbr_large_numbers(sell_qty),
                            Point::new(area.ask_area_right, y),
                            text_size,
                            text_color,
                            Alignment::End,
                            Alignment::Center,
                        );
                    }

                    let bar_width = (sell_qty / max_cluster_qty) * left_area_width;
                    if bar_width > 0.0 {
                        frame.fill_rectangle(
                            Point::new(area.ask_area_right, y - (cell_height / 2.0)),
                            Size::new(-bar_width, cell_height),
                            palette.danger.base.color.scale_alpha(bar_alpha),
                        );
                    }
                }

                if let Some((threshold, color_scale, ignore_zeros)) = imbalance
                    && area.imb_marker_width > 0.0
                {
                    let step = PriceStep::from_f32(tick_size);
                    let higher_price =
                        Price::from_f32(price.to_f32() + tick_size).round_to_step(step);

                    let rect_width = ((area.imb_marker_width - 1.0) / 2.0).max(1.0);

                    let buyside_x = area.bid_area_right - rect_width - spacing.marker_to_bars;
                    let sellside_x = area.ask_area_left + spacing.marker_to_bars;

                    draw_imbalance_markers(
                        frame,
                        &price_to_y,
                        footprint,
                        *price,
                        sell_qty,
                        higher_price,
                        threshold,
                        color_scale,
                        ignore_zeros,
                        cell_height,
                        palette,
                        buyside_x,
                        sellside_x,
                        rect_width,
                    );
                }
            }

            draw_footprint_kline(
                frame,
                &price_to_y,
                area.candle_center_x,
                candle_width,
                kline,
                palette,
            );
        }
    }
}

fn draw_imbalance_markers(
    frame: &mut canvas::Frame,
    price_to_y: &impl Fn(Price) -> f32,
    footprint: &KlineTrades,
    price: Price,
    sell_qty: f32,
    higher_price: Price,
    threshold: usize,
    color_scale: Option<usize>,
    ignore_zeros: bool,
    cell_height: f32,
    palette: &Extended,
    buyside_x: f32,
    sellside_x: f32,
    rect_width: f32,
) {
    if ignore_zeros && sell_qty <= 0.0 {
        return;
    }

    if let Some(group) = footprint.trades.get(&higher_price) {
        let diagonal_buy_qty = f32::from(group.buy_qty);

        if ignore_zeros && diagonal_buy_qty <= 0.0 {
            return;
        }

        let rect_height = cell_height / 2.0;

        let alpha_from_ratio = |ratio: f32| -> f32 {
            if let Some(scale) = color_scale {
                let divisor = (scale as f32 / 10.0) - 1.0;
                (0.2 + 0.8 * ((ratio - 1.0) / divisor).min(1.0)).min(1.0)
            } else {
                1.0
            }
        };

        if diagonal_buy_qty >= sell_qty {
            let required_qty = sell_qty * (100 + threshold) as f32 / 100.0;
            if diagonal_buy_qty > required_qty {
                let ratio = diagonal_buy_qty / required_qty;
                let alpha = alpha_from_ratio(ratio);

                let y = price_to_y(higher_price);
                frame.fill_rectangle(
                    Point::new(buyside_x, y - (rect_height / 2.0)),
                    Size::new(rect_width, rect_height),
                    palette.success.weak.color.scale_alpha(alpha),
                );
            }
        } else {
            let required_qty = diagonal_buy_qty * (100 + threshold) as f32 / 100.0;
            if sell_qty > required_qty {
                let ratio = sell_qty / required_qty;
                let alpha = alpha_from_ratio(ratio);

                let y = price_to_y(price);
                frame.fill_rectangle(
                    Point::new(sellside_x, y - (rect_height / 2.0)),
                    Size::new(rect_width, rect_height),
                    palette.danger.weak.color.scale_alpha(alpha),
                );
            }
        }
    }
}

impl ContentGaps {
    fn from_view(candle_width: f32, scaling: f32) -> Self {
        let px = |p: f32| p / scaling;
        let base = (candle_width * 0.2).max(px(2.0));
        Self {
            marker_to_candle: base,
            candle_to_cluster: base,
            marker_to_bars: px(2.0),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ContentGaps {
    /// Space between imb. markers candle body
    marker_to_candle: f32,
    /// Space between candle body and clusters
    candle_to_cluster: f32,
    /// Inner space reserved between imb. markers and clusters (used for BidAsk)
    marker_to_bars: f32,
}

fn draw_cluster_text(
    frame: &mut canvas::Frame,
    text: &str,
    position: Point,
    text_size: f32,
    color: iced::Color,
    align_x: Alignment,
    align_y: Alignment,
) {
    frame.fill_text(canvas::Text {
        content: text.to_string(),
        position,
        size: iced::Pixels(text_size),
        color,
        align_x: align_x.into(),
        align_y: align_y.into(),
        font: style::AZERET_MONO,
        ..canvas::Text::default()
    });
}

// GitHub Issue: https://github.com/terrylica/flowsurface/issues/2
/// Formats a duration given in milliseconds as a compact human-readable string.
/// Examples: 500 → "500ms", 45_234 → "45.234s", 63_555 → "1m 3.555s", 3_661_000 → "1h 1m 1s"
fn format_duration_ms(ms: u64) -> String {
    if ms >= 3_600_000 {
        let h = ms / 3_600_000;
        let rem = ms % 3_600_000;
        let m = rem / 60_000;
        let s = rem % 60_000 / 1_000;
        if m == 0 && s == 0 {
            format!("{h}h")
        } else if s == 0 {
            format!("{h}h {m}m")
        } else {
            format!("{h}h {m}m {s}s")
        }
    } else if ms >= 60_000 {
        let m = ms / 60_000;
        let rem_ms = ms % 60_000;
        if rem_ms == 0 {
            format!("{m}m")
        } else {
            format!("{m}m {:.3}s", rem_ms as f64 / 1000.0)
        }
    } else if ms >= 1_000 {
        format!("{:.3}s", ms as f64 / 1000.0)
    } else {
        format!("{ms}ms")
    }
}

/// Draws bar selection statistics overlay (screen-space, top-center of chart).
/// Called in the `legend` cache layer when both anchor and end are confirmed.
///
/// Display semantics:
/// - Distance: |end − anchor| bars (0 = same bar, 1 = adjacent bars)
/// - Up/Down: inclusive count over [min, max]; ratios from that same count.
fn draw_bar_selection_stats(
    frame: &mut canvas::Frame,
    palette: &Extended,
    tick_aggr: &data::aggr::ticks::TickAggr,
    anchor: usize,
    end: usize,
) {
    let len = tick_aggr.datapoints.len();
    if len == 0 {
        return;
    }
    let (lo, hi) = (anchor.min(end), anchor.max(end));
    let hi = hi.min(len - 1);
    let lo = lo.min(len - 1);

    // Distance: |end − anchor| (0 for same bar, 1 for adjacent).
    let distance = hi - lo;

    // Count up/down over the inclusive range [lo, hi].
    let examined = hi - lo + 1;
    let up = (lo..=hi)
        .filter(|&vi| {
            let si = len - 1 - vi;
            tick_aggr.datapoints[si].kline.close >= tick_aggr.datapoints[si].kline.open
        })
        .count();
    let down = examined - up;
    let up_pct = up as f32 / examined as f32 * 100.0;
    let down_pct = down as f32 / examined as f32 * 100.0;

    let text_size = 13.0_f32;
    let line_h = text_size + 5.0;
    let lines: &[(&str, String)] = &[
        ("neutral", format!("{distance} bars")),
        ("up", format!("↑ {up}  ({up_pct:.0}%)")),
        ("down", format!("↓ {down}  ({down_pct:.0}%)")),
    ];

    let box_w = 130.0_f32;
    let box_h = line_h * lines.len() as f32 + 10.0;
    let x = frame.width() / 2.0 - box_w / 2.0;
    let y = 10.0_f32;

    frame.fill_rectangle(
        Point::new(x - 6.0, y - 4.0),
        Size::new(box_w + 12.0, box_h),
        iced::Color { r: 0.08, g: 0.08, b: 0.08, a: 0.88 },
    );

    for (i, (kind, text)) in lines.iter().enumerate() {
        let color = match *kind {
            "up" => palette.success.base.color,
            "down" => palette.danger.base.color,
            _ => palette.background.strong.text,
        };
        frame.fill_text(canvas::Text {
            content: text.clone(),
            position: Point::new(x, y + i as f32 * line_h),
            size: iced::Pixels(text_size),
            color,
            ..Default::default()
        });
    }
}

fn draw_crosshair_tooltip(
    data: &PlotData<KlineDataPoint>,
    ticker_info: &TickerInfo,
    frame: &mut canvas::Frame,
    palette: &Extended,
    at_interval: u64,
    basis: Basis,
    timezone: data::UserTimezone,
    forming_kline: Option<&Kline>,
) {
    // Resolve both the kline and (for tick-based) the agg_trade_id_range
    let (kline_opt, agg_id_range): (Option<&Kline>, Option<(u64, u64)>) = match data {
        PlotData::TimeBased(timeseries) => {
            let kline = timeseries
                .datapoints
                .iter()
                .find(|(time, _)| **time == at_interval)
                .map(|(_, dp)| &dp.kline)
                .or_else(|| {
                    if timeseries.datapoints.is_empty() {
                        None
                    } else {
                        let (last_time, dp) = timeseries.datapoints.last_key_value()?;
                        if at_interval > *last_time {
                            Some(&dp.kline)
                        } else {
                            None
                        }
                    }
                });
            (kline, None)
        }
        PlotData::TickBased(tick_aggr) => {
            if at_interval == u64::MAX {
                log::trace!(
                    "[TOOLTIP] forming bar sentinel detected, forming_kline={}",
                    forming_kline.is_some()
                );
                // Forming bar: use last completed bar's agg_trade_id_range
                let ids = tick_aggr
                    .datapoints
                    .last()
                    .and_then(|dp| dp.agg_trade_id_range);
                (forming_kline, ids)
            } else {
                let index = (at_interval / u64::from(tick_aggr.interval.0)) as usize;
                if index < tick_aggr.datapoints.len() {
                    let dp = &tick_aggr.datapoints[tick_aggr.datapoints.len() - 1 - index];
                    (Some(&dp.kline), dp.agg_trade_id_range)
                } else {
                    (None, None)
                }
            }
        }
    };

    if let Some(kline) = kline_opt {
        let change_pct = ((kline.close - kline.open).to_f32() / kline.open.to_f32()) * 100.0;
        let change_color = if change_pct >= 0.0 {
            palette.success.base.color
        } else {
            palette.danger.base.color
        };

        let base_color = palette.background.base.text;
        let dim_color = base_color.scale_alpha(0.65);
        let precision = ticker_info.min_ticksize;

        let pct_str = format!("{change_pct:+.2}%");
        let open_str = kline.open.to_string(precision);
        let high_str = kline.high.to_string(precision);
        let low_str = kline.low.to_string(precision);
        let close_str = kline.close.to_string(precision);

        let segments: &[(&str, iced::Color, bool)] = &[
            ("O", base_color, false),
            (&open_str, change_color, true),
            ("H", base_color, false),
            (&high_str, change_color, true),
            ("L", base_color, false),
            (&low_str, change_color, true),
            ("C", base_color, false),
            (&close_str, change_color, true),
            (&pct_str, change_color, true),
        ];

        let ohlc_width: f32 = segments
            .iter()
            .map(|(s, _, is_val)| s.len() as f32 * 10.0 + if *is_val { 8.0 } else { 3.0 })
            .sum();

        // Timing rows: open time, close time, duration — only for index-based bases.
        // Shows both UTC and Local so the user always sees both at a glance.
        let timing_lines: Option<(String, String)> = match (basis, data) {
            (Basis::Odb(_) | Basis::Tick(_), PlotData::TickBased(tick_aggr)) => {
                let (open_ms, close_ms) = if at_interval == u64::MAX {
                    // Forming bar: open = last completed bar's close_time, close = forming kline's time
                    let open = tick_aggr.datapoints.last().map(|dp| dp.kline.time as i64);
                    let close = kline.time as i64;
                    log::trace!(
                        "[TOOLTIP] forming timing: open_ms={:?} close_ms={}",
                        open,
                        close
                    );
                    (open, close)
                } else {
                    let index = (at_interval / u64::from(tick_aggr.interval.0)) as usize;
                    let fwd = tick_aggr.datapoints.len().saturating_sub(1 + index);
                    let close = kline.time as i64;
                    // Open time: use open_time_ms from ClickHouse if available (correct for ODB
                    // bars — prev_bar.close_time ≠ this_bar.open_time due to gap between the
                    // trigger trade and the first trade of the new bar).
                    // Fallback: previous bar's close time (correct for Tick basis).
                    let open = tick_aggr
                        .datapoints
                        .get(fwd)
                        .and_then(|dp| dp.open_time_ms)
                        .map(|ms| ms as i64)
                        .or_else(|| {
                            (fwd > 0)
                                .then(|| tick_aggr.datapoints[fwd - 1].kline.time as i64)
                        });
                    (open, close)
                };

                let alt_tz = match timezone {
                    data::UserTimezone::Utc => data::UserTimezone::Local,
                    data::UserTimezone::Local => data::UserTimezone::Utc,
                };

                let dur_fmt = open_ms
                    .map(|open| format_duration_ms(close_ms.saturating_sub(open).max(0) as u64))
                    .unwrap_or_else(|| "—".into());

                let fmt_row = |tz: data::UserTimezone| {
                    let close_fmt = tz.format_bar_time_ms(close_ms).unwrap_or_default();
                    let open_fmt = open_ms
                        .and_then(|ms| tz.format_bar_time_ms(ms))
                        .unwrap_or_else(|| "—".into());
                    format!("{open_fmt}  →  {close_fmt}   ({dur_fmt})  {tz}")
                };

                Some((fmt_row(timezone), fmt_row(alt_tz)))
            }
            _ => None,
        };

        // Row 4: agg_trade_id range (ODB bars only)
        let agg_id_line: Option<String> = agg_id_range.map(|(first, last)| {
            let span = last.saturating_sub(first) + 1;
            format!("ID {first}  →  {last}   (n={span})")
        });

        let timing_width = timing_lines
            .as_ref()
            .map(|(a, b)| {
                let wa = a.len() as f32 * 9.0 + 16.0;
                let wb = b.len() as f32 * 9.0 + 16.0;
                wa.max(wb)
            })
            .unwrap_or(0.0);
        let agg_id_width = agg_id_line
            .as_ref()
            .map(|s| s.len() as f32 * 9.0 + 16.0)
            .unwrap_or(0.0);
        let bg_width = ohlc_width.max(timing_width).max(agg_id_width);
        let has_timing = timing_lines.is_some();
        let has_agg_id = agg_id_line.is_some();
        let bg_height = match (has_timing, has_agg_id) {
            (true, true) => 78.0,   // OHLC + 2 timing + agg_id
            (true, false) => 60.0,  // OHLC + 2 timing
            (false, true) => 38.0,  // OHLC + agg_id
            (false, false) => 20.0, // OHLC only
        };

        let position = Point::new(
            frame.width() - bg_width - 8.0,
            frame.height() - bg_height - 8.0,
        );

        frame.fill_rectangle(
            position,
            iced::Size::new(bg_width, bg_height),
            palette.background.weakest.color.scale_alpha(0.9),
        );

        // Row 1: O H L C %
        let mut x = position.x;
        for (text, seg_color, is_value) in segments {
            frame.fill_text(canvas::Text {
                content: text.to_string(),
                position: Point::new(x, position.y),
                size: iced::Pixels(15.0),
                color: *seg_color,
                font: style::AZERET_MONO,
                ..canvas::Text::default()
            });
            x += text.len() as f32 * 10.0;
            x += if *is_value { 8.0 } else { 3.0 };
        }

        let mut next_y = position.y + 22.0;

        // Row 2 + 3: open → close (duration) in both timezones
        if let Some((primary, alt)) = timing_lines {
            frame.fill_text(canvas::Text {
                content: primary,
                position: Point::new(position.x, next_y),
                size: iced::Pixels(13.0),
                color: dim_color,
                font: style::AZERET_MONO,
                ..canvas::Text::default()
            });
            next_y += 17.0;
            frame.fill_text(canvas::Text {
                content: alt,
                position: Point::new(position.x, next_y),
                size: iced::Pixels(13.0),
                color: dim_color,
                font: style::AZERET_MONO,
                ..canvas::Text::default()
            });
            next_y += 17.0;
        }

        // Row 4: agg_trade_id range
        if let Some(id_line) = agg_id_line {
            frame.fill_text(canvas::Text {
                content: id_line,
                position: Point::new(position.x, next_y),
                size: iced::Pixels(13.0),
                color: dim_color,
                font: style::AZERET_MONO,
                ..canvas::Text::default()
            });
        }
    }
}

struct ProfileArea {
    imb_marker_left: f32,
    imb_marker_width: f32,
    bars_left: f32,
    bars_width: f32,
    candle_center_x: f32,
}

impl ProfileArea {
    fn new(
        content_left: f32,
        content_right: f32,
        candle_width: f32,
        gaps: ContentGaps,
        has_imbalance: bool,
    ) -> Self {
        let candle_lane_left = if has_imbalance {
            content_left + candle_width + gaps.marker_to_candle
        } else {
            content_left
        };
        let candle_lane_width = candle_width * 0.25;

        let bars_left = candle_lane_left + candle_lane_width + gaps.candle_to_cluster;
        let bars_width = (content_right - bars_left).max(0.0);

        let candle_center_x = candle_lane_left + (candle_lane_width / 2.0);

        Self {
            imb_marker_left: content_left,
            imb_marker_width: if has_imbalance { candle_width } else { 0.0 },
            bars_left,
            bars_width,
            candle_center_x,
        }
    }
}

struct BidAskArea {
    bid_area_left: f32,
    bid_area_right: f32,
    ask_area_left: f32,
    ask_area_right: f32,
    candle_center_x: f32,
    imb_marker_width: f32,
}

impl BidAskArea {
    fn new(
        x_position: f32,
        content_left: f32,
        content_right: f32,
        candle_width: f32,
        spacing: ContentGaps,
    ) -> Self {
        let candle_body_width = candle_width * 0.25;

        let candle_left = x_position - (candle_body_width / 2.0);
        let candle_right = x_position + (candle_body_width / 2.0);

        let ask_area_right = candle_left - spacing.candle_to_cluster;
        let bid_area_left = candle_right + spacing.candle_to_cluster;

        Self {
            bid_area_left,
            bid_area_right: content_right,
            ask_area_left: content_left,
            ask_area_right,
            candle_center_x: x_position,
            imb_marker_width: candle_width,
        }
    }
}

#[inline]
fn should_show_text(cell_height_unscaled: f32, cell_width_unscaled: f32, min_w: f32) -> bool {
    cell_height_unscaled > 8.0 && cell_width_unscaled > min_w
}

#[cfg(test)]
mod tests {
    use super::GapFillRequest;
    use data::chart::Basis;
    use exchange::Trade;
    use exchange::unit::{Price, Qty};

    fn make_trade(id: u64, price: f32) -> Trade {
        Trade {
            time: 1000,
            is_sell: false,
            price: Price::from_f32(price),
            qty: Qty::from_f32(0.001),
            agg_trade_id: Some(id),
        }
    }

    fn is_gap(prev: u64, curr: u64) -> bool {
        curr.saturating_sub(prev) > 1
    }

    #[test]
    fn gap_of_one_is_not_a_gap() {
        assert!(!is_gap(100, 101));
    }

    #[test]
    fn gap_of_two_is_a_gap() {
        assert!(is_gap(100, 102));
    }

    #[test]
    fn saturating_sub_handles_reorder() {
        assert!(!is_gap(200, 100));
    }

    #[test]
    fn make_trade_has_correct_id() {
        let t = make_trade(42, 68500.0);
        assert_eq!(t.agg_trade_id, Some(42));
    }

    #[test]
    fn cooldown_arithmetic_blocks_within_30s() {
        let last: u64 = 1_000_000;
        let now: u64 = 1_020_000; // 20s later
        assert!(now.saturating_sub(last) <= 30_000);
    }

    #[test]
    fn cooldown_arithmetic_allows_after_30s() {
        let last: u64 = 1_000_000;
        let now: u64 = 1_031_000; // 31s later
        assert!(now.saturating_sub(last) > 30_000);
    }

    #[test]
    fn cooldown_arithmetic_exact_boundary() {
        let last: u64 = 1_000_000;
        let now: u64 = 1_030_000; // exactly 30s
        assert!(now.saturating_sub(last) <= 30_000);
    }

    #[test]
    fn gap_fill_request_fields() {
        let req = GapFillRequest {
            symbol: "BTCUSDT".into(),
            threshold_dbps: 250,
        };
        assert_eq!(req.symbol, "BTCUSDT");
        assert_eq!(req.threshold_dbps, 250);
    }

    #[test]
    fn dedup_fence_filters_stale_trades() {
        let fence_id: u64 = 100;
        let trades = [
            make_trade(99, 68000.0),
            make_trade(100, 68100.0),
            make_trade(101, 68200.0),
            make_trade(102, 68300.0),
        ];
        let passed: Vec<_> = trades
            .iter()
            .filter(|t| t.agg_trade_id.is_none_or(|id| id > fence_id))
            .collect();
        assert_eq!(passed.len(), 2);
        assert_eq!(passed[0].agg_trade_id, Some(101));
        assert_eq!(passed[1].agg_trade_id, Some(102));
    }

    #[test]
    fn dedup_fence_none_passes_all() {
        let fence: Option<u64> = None;
        let trades = [
            make_trade(1, 68000.0),
            make_trade(2, 68100.0),
            make_trade(3, 68200.0),
        ];
        let passed: Vec<_> = trades
            .iter()
            .filter(|t| match fence {
                None => true,
                Some(f) => t.agg_trade_id.is_none_or(|id| id > f),
            })
            .collect();
        assert_eq!(passed.len(), 3);
    }

    /// Guard logic: `!fetching_trades && !gap_fill_requested && basis.is_odb()
    ///               && now_ms.saturating_sub(last_trigger) > 30_000`
    fn guard_allows(
        fetching_trades: bool,
        gap_fill_requested: bool,
        basis: &Basis,
        now_ms: u64,
        last_trigger: u64,
    ) -> bool {
        !fetching_trades
            && !gap_fill_requested
            && basis.is_odb()
            && now_ms.saturating_sub(last_trigger) > 30_000
    }

    #[test]
    fn guard_composition_all_false() {
        // All guard conditions are "clear" → trigger allowed
        assert!(guard_allows(
            false,
            false,
            &Basis::Odb(250),
            100_000,
            60_000
        ));
    }

    #[test]
    fn guard_fetching_blocks() {
        assert!(!guard_allows(
            true,
            false,
            &Basis::Odb(250),
            100_000,
            60_000
        ));
    }

    #[test]
    fn guard_already_requested_blocks() {
        assert!(!guard_allows(
            false,
            true,
            &Basis::Odb(250),
            100_000,
            60_000
        ));
    }

    #[test]
    fn guard_cooldown_blocks() {
        // last_trigger 20s ago → within 30s cooldown
        assert!(!guard_allows(
            false,
            false,
            &Basis::Odb(250),
            100_000,
            80_000
        ));
    }

    #[test]
    fn guard_non_odb_blocks() {
        assert!(!guard_allows(
            false,
            false,
            &Basis::Time(exchange::Timeframe::M1),
            100_000,
            60_000
        ));
    }

    #[test]
    fn guard_all_clear_allows() {
        // All guards pass: not fetching, not requested, ODB basis, cooldown expired
        assert!(guard_allows(
            false,
            false,
            &Basis::Odb(500),
            200_000,
            100_000
        ));
    }

    #[test]
    fn gap_detection_skipped_for_gap_fill_trades() {
        let is_gap_fill = true;
        let prev_id: u64 = 100;
        let curr_id: u64 = 200; // big gap

        // When is_gap_fill is true, gap detection is skipped regardless of gap size
        let should_detect_gap = !is_gap_fill && is_gap(prev_id, curr_id);
        assert!(!should_detect_gap);

        // When is_gap_fill is false, the same gap IS detected
        let is_gap_fill = false;
        let should_detect_gap = !is_gap_fill && is_gap(prev_id, curr_id);
        assert!(should_detect_gap);
    }
}
