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

mod bar_selection;
use bar_selection::{
    BrimSide, BarSelectionState, STATS_BOX_W, STATS_BOX_H,
    draw_selection_highlight, stats_box_origin, draw_bar_selection_stats,
};

mod crosshair;
use crosshair::draw_crosshair_tooltip;

mod odb_core;
pub use odb_core::BarGapKind;
#[cfg(test)]
use odb_core::GapFillRequest;

mod rendering;
pub(crate) use rendering::draw_candle_dp;
use rendering::{
    ContentGaps,
    render_data_source, draw_all_npocs,
    effective_cluster_qty, draw_clusters,
    should_show_text,
};

use iced::task::Handle;
use iced::theme::palette::Extended;
use iced::widget::canvas::{self, Event, Geometry, Path, Stroke};
use iced::{Alignment, Element, Point, Rectangle, Renderer, Size, Theme, Vector, keyboard, mouse};

use enum_map::EnumMap;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

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
            _ => Some(chart.interval_range(&region)),
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

        if self.chart.basis.is_odb() {
            let bounds_size = bounds.size();
            let (shift_held, dragging_brim, sel_anchor, sel_end,
                 dragging_stats_box, stats_box_pos) = {
                let sel = self.bar_selection.borrow();
                (sel.shift_held, sel.dragging_brim.is_some(), sel.anchor, sel.end,
                 sel.dragging_stats_box, sel.stats_box_pos)
            };

            // ── Stats box drag: release + move ────────────────────────────
            if dragging_stats_box {
                if let Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) = event {
                    self.bar_selection.borrow_mut().dragging_stats_box = false;
                    return Some(canvas::Action::request_redraw().and_capture());
                }
                if let Event::Mouse(mouse::Event::CursorMoved { .. }) = event {
                    if let Some(cursor_pos) = cursor.position_in(bounds) {
                        let (dx, dy) = self.bar_selection.borrow().stats_drag_offset;
                        let nx = (cursor_pos.x - dx).clamp(0.0, bounds_size.width  - STATS_BOX_W);
                        let ny = (cursor_pos.y - dy).clamp(0.0, bounds_size.height - STATS_BOX_H);
                        self.bar_selection.borrow_mut().stats_box_pos = Some(Point::new(nx, ny));
                    }
                    self.chart.cache.legend.clear();
                    return Some(canvas::Action::request_redraw().and_capture());
                }
            }

            // ── Brim drag: release ──────────────────────────────────────────
            if dragging_brim {
                if let Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) = event {
                    self.bar_selection.borrow_mut().dragging_brim = None;
                    return Some(canvas::Action::request_redraw().and_capture());
                }
                // Update boundary on cursor move.
                if let Event::Mouse(mouse::Event::CursorMoved { .. }) = event {
                    if let Some(cursor_pos) = cursor.position_in(bounds) {
                        let region = self.chart.visible_region(bounds_size);
                        let (visual_idx, _) =
                            self.chart.snap_x_to_index(cursor_pos.x, bounds_size, region);
                        if visual_idx != u64::MAX {
                            let new_idx = visual_idx as usize;
                            let mut sel = self.bar_selection.borrow_mut();
                            match sel.dragging_brim {
                                // Lo = right (newer) brim — update whichever of anchor/end is smaller
                                Some(BrimSide::Lo) => match (sel.anchor, sel.end) {
                                    (Some(a), Some(e)) if a <= e => sel.anchor = Some(new_idx),
                                    (Some(_), Some(_)) => sel.end = Some(new_idx),
                                    _ => {}
                                },
                                // Hi = left (older) brim — update whichever is larger
                                Some(BrimSide::Hi) => match (sel.anchor, sel.end) {
                                    (Some(a), Some(e)) if a >= e => sel.anchor = Some(new_idx),
                                    (Some(_), Some(_)) => sel.end = Some(new_idx),
                                    _ => {}
                                },
                                None => {}
                            }
                        }
                    }
                    // Only clear lightweight caches so candles stay cached.
                    self.chart.cache.clear_crosshair();
                    self.chart.cache.legend.clear();
                    return Some(canvas::Action::request_redraw().and_capture());
                }
            }

            // ── Stats box drag: start (click anywhere inside box, no Shift) ──
            // Checked before brim drag so the box always wins when overlapping.
            if !shift_held
                && let (Some(_), Some(_)) = (sel_anchor, sel_end)
                && let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event
                && let Some(cursor_pos) = cursor.position_in(bounds)
            {
                let origin = stats_box_origin(stats_box_pos, bounds_size.width);
                if cursor_pos.x >= origin.x && cursor_pos.x <= origin.x + STATS_BOX_W
                    && cursor_pos.y >= origin.y && cursor_pos.y <= origin.y + STATS_BOX_H
                {
                    let mut sel = self.bar_selection.borrow_mut();
                    sel.dragging_stats_box = true;
                    sel.stats_drag_offset =
                        (cursor_pos.x - origin.x, cursor_pos.y - origin.y);
                    return Some(canvas::Action::request_redraw().and_capture());
                }
            }

            // ── Brim drag: start (click on outermost selected bar, no Shift) ───
            // Uses snap_x_to_index (same as Shift+Click) so hit detection is
            // guaranteed consistent with the actual bar grid.
            if !shift_held
                && let (Some(anchor), Some(end)) = (sel_anchor, sel_end)
                && anchor != end
                && let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event
                && let Some(cursor_pos) = cursor.position_in(bounds)
            {
                let lo = anchor.min(end);
                let hi = anchor.max(end);
                let region = self.chart.visible_region(bounds_size);
                let (visual_idx, _) =
                    self.chart.snap_x_to_index(cursor_pos.x, bounds_size, region);
                if visual_idx != u64::MAX {
                    let snapped = visual_idx as usize;
                    let lo_dist = snapped.abs_diff(lo);
                    let hi_dist = snapped.abs_diff(hi);
                    // Within ±1 bar of a brim → drag it. Ties go to Lo (right/newer brim).
                    let side = if lo_dist <= 1 && lo_dist <= hi_dist {
                        Some(BrimSide::Lo)
                    } else if hi_dist <= 1 {
                        Some(BrimSide::Hi)
                    } else {
                        None
                    };
                    if let Some(side) = side {
                        self.bar_selection.borrow_mut().dragging_brim = Some(side);
                        return Some(canvas::Action::request_redraw().and_capture());
                    }
                }
            }

            // ── Shift+Left Click: set anchor / end / restart ───────────────
            if shift_held
                && let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event
            {
                if let Some(cursor_pos) = cursor.position_in(bounds) {
                    let region = self.chart.visible_region(bounds_size);
                    let (visual_idx, _) =
                        self.chart.snap_x_to_index(cursor_pos.x, bounds_size, region);
                    if visual_idx != u64::MAX {
                        let mut sel = self.bar_selection.borrow_mut();
                        match (sel.anchor, sel.end) {
                            (None, _) => sel.anchor = Some(visual_idx as usize),
                            (Some(_), None) => sel.end = Some(visual_idx as usize),
                            // Third Shift+Click: restart from new anchor; reset box position.
                            (Some(_), Some(_)) => {
                                sel.anchor = Some(visual_idx as usize);
                                sel.end = None;
                                sel.stats_box_pos = None;
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
                    let pos = sel.stats_box_pos;
                    draw_bar_selection_stats(frame, palette, tick_aggr, anchor, end, pos);
                }
            }
        });

        let crosshair = chart.cache.crosshair.draw(renderer, bounds_size, |frame| {
            // Selection highlight drawn in screen-space so it updates cheaply
            // during brim drag (only crosshair + legend caches cleared, not klines).
            if chart.basis.is_odb() {
                let sel = self.bar_selection.borrow();
                if let Some(anchor) = sel.anchor {
                    let end = sel.end.unwrap_or(anchor);
                    let (lo, hi) = (anchor.min(end), anchor.max(end));
                    draw_selection_highlight(frame, chart, bounds_size, lo, hi);
                }
            }

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
        // Cursor feedback for bar selection interactions (ODB only).
        if self.chart.basis.is_odb() {
            let sel = self.bar_selection.borrow();
            // Active drag states take priority.
            if sel.dragging_stats_box {
                return mouse::Interaction::Grabbing;
            }
            if sel.dragging_brim.is_some() {
                return mouse::Interaction::ResizingHorizontally;
            }
            if let (Some(anchor), Some(end)) = (sel.anchor, sel.end)
                && let Some(cursor_pos) = cursor.position_in(bounds) {
                    // Stats box hover (checked before brim — box wins when they overlap).
                    let origin = stats_box_origin(sel.stats_box_pos, bounds.width);
                    if cursor_pos.x >= origin.x && cursor_pos.x <= origin.x + STATS_BOX_W
                        && cursor_pos.y >= origin.y && cursor_pos.y <= origin.y + STATS_BOX_H
                    {
                        return mouse::Interaction::Grab;
                    }
                    // Brim hover.
                    if anchor != end {
                        let lo = anchor.min(end);
                        let hi = anchor.max(end);
                        let region = self.chart.visible_region(bounds.size());
                        let (visual_idx, _) =
                            self.chart.snap_x_to_index(cursor_pos.x, bounds.size(), region);
                        if visual_idx != u64::MAX {
                            let snapped = visual_idx as usize;
                            if snapped.abs_diff(lo) <= 1 || snapped.abs_diff(hi) <= 1 {
                                return mouse::Interaction::ResizingHorizontally;
                            }
                        }
                    }
            }
        }
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


// GitHub Issue: https://github.com/terrylica/flowsurface/issues/2

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
