use super::*;

// ── Public types ──────────────────────────────────────────────────────────────

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

// ── ODB-specific impl KlineChart ─────────────────────────────────────────────

impl KlineChart {
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

    pub fn reset_request_handler(&mut self) {
        self.request_handler = RequestHandler::default();
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
    pub(super) fn audit_bar_continuity(&self) -> Vec<BarGap> {
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
}
