use super::fetch::FetchCoordinator;
use super::layout::{IndicatorRebuildReason, IndicatorState};
use data::chart::Basis;
use data::chart::kline::{
    HistoricalInputUpdate, IndicatorUpdateScope, KlineData, KlineDataChange, KlineIndexChange,
    OpenInterestSeries,
};
use exchange::{Kline, TickerInfo, Timeframe, Trade};

pub enum DataUpdate<'a> {
    HistoricalKlines {
        req_id: Option<uuid::Uuid>,
        timeframe: Timeframe,
        ticker_info: TickerInfo,
        klines: &'a [Kline],
        is_batches_done: bool,
        has_data: bool,
    },
    /// Refreshes only the chart's visual tail. The indicator engine consumes
    /// committed historical snapshots, not this provisional websocket value.
    LiveKline {
        timeframe: Timeframe,
        ticker_info: &'a TickerInfo,
        kline: &'a Kline,
    },
    /// Updates a selected series' live trades and invalidates the open preview.
    LiveTrades {
        ticker_info: &'a TickerInfo,
        trades: &'a [Trade],
    },
    HistoricalTrades {
        ticker_info: TickerInfo,
        trades: &'a [Trade],
        is_batches_done: bool,
        req_id: Option<uuid::Uuid>,
    },
    HistoricalOpenInterest {
        req_id: Option<uuid::Uuid>,
        ticker_info: TickerInfo,
        series: &'a OpenInterestSeries,
    },
}

pub(super) enum DataUpdateEffect {
    Unchanged,
    HistoricalChanged,
    LiveChanged,
}

impl DataUpdate<'_> {
    pub(super) fn kline_timeframe(&self) -> Option<Timeframe> {
        match self {
            DataUpdate::HistoricalKlines { timeframe, .. }
            | DataUpdate::LiveKline { timeframe, .. } => Some(*timeframe),
            _ => None,
        }
    }

    pub(super) fn apply(
        &self,
        basis: Basis,
        data: &mut KlineData,
        fetch: &mut FetchCoordinator,
        indicators: &mut IndicatorState,
        trade_data_enabled: bool,
    ) -> DataUpdateEffect {
        let indicator_ids = indicators.incremental_ids();
        let change = self.ingest(
            basis,
            data,
            fetch,
            indicators,
            &indicator_ids,
            trade_data_enabled,
        );
        if matches!(&change, KlineDataChange::Unchanged)
            || matches!(
                &change,
                KlineDataChange::HistoricalInput { update, .. } if !update.needs_rebuild()
            )
        {
            return DataUpdateEffect::Unchanged;
        }

        match (self, change) {
            (
                DataUpdate::HistoricalKlines { ticker_info, .. },
                KlineDataChange::HistoricalKlines { update },
            ) => {
                if *ticker_info == data.base_ticker() {
                    remap_indicator_outputs(data, indicators, update.index_change);
                }
                indicators.mark_for_candle_ticker(
                    data,
                    *ticker_info,
                    if basis.is_time() {
                        IndicatorRebuildReason::HistoricalDataChanged(update.scope)
                    } else {
                        IndicatorRebuildReason::BasisChanged
                    },
                );
                DataUpdateEffect::HistoricalChanged
            }
            (DataUpdate::LiveKline { ticker_info, .. }, KlineDataChange::LiveKline { update }) => {
                if **ticker_info == data.base_ticker() {
                    remap_indicator_outputs(data, indicators, update.index_change);
                }
                if let Some(scope) = update.rebuild_scope {
                    indicators.mark_for_candle_ticker(
                        data,
                        **ticker_info,
                        if !basis.is_time() && scope.is_full() {
                            IndicatorRebuildReason::BasisChanged
                        } else {
                            IndicatorRebuildReason::HistoricalDataChanged(scope)
                        },
                    );
                }
                apply_live_state(
                    data,
                    indicators,
                    update.coverage_frames,
                    update.committed_bar,
                    trade_data_enabled,
                );
                DataUpdateEffect::LiveChanged
            }
            (
                DataUpdate::LiveTrades { ticker_info, .. },
                KlineDataChange::LiveTrades { update },
            ) => {
                if update.rebuild_scope.is_some() || update.closed_trade_correction {
                    let scope = update.rebuild_scope.unwrap_or(IndicatorUpdateScope::Full);
                    let reason = if !basis.is_time() && scope.is_full() {
                        IndicatorRebuildReason::BasisChanged
                    } else {
                        IndicatorRebuildReason::HistoricalDataChanged(scope)
                    };
                    if basis.is_time() {
                        indicators.mark_for_trade_ticker(data, **ticker_info, reason);
                    } else {
                        indicators.mark_for_candle_ticker(data, **ticker_info, reason);
                    }
                }
                apply_live_state(
                    data,
                    indicators,
                    update.coverage_frames,
                    update.committed_bar,
                    trade_data_enabled,
                );
                DataUpdateEffect::LiveChanged
            }
            (
                DataUpdate::HistoricalTrades { ticker_info, .. },
                KlineDataChange::HistoricalTrades,
            ) => {
                if basis.is_time() {
                    indicators.mark_for_trade_ticker(
                        data,
                        *ticker_info,
                        IndicatorRebuildReason::InputFetch,
                    );
                } else {
                    indicators.mark_for_candle_ticker(
                        data,
                        *ticker_info,
                        IndicatorRebuildReason::BasisChanged,
                    );
                }
                DataUpdateEffect::HistoricalChanged
            }
            (_, KlineDataChange::HistoricalInput { source, update }) => {
                debug_assert!(update.needs_rebuild());
                indicators.mark_for_inputs(data, &[source], IndicatorRebuildReason::InputFetch);
                DataUpdateEffect::HistoricalChanged
            }
            _ => unreachable!("data update outcome does not match its input"),
        }
    }

    fn ingest(
        &self,
        basis: Basis,
        data: &mut KlineData,
        fetch: &mut FetchCoordinator,
        indicators: &IndicatorState,
        indicator_ids: &[data::chart::kline::IndicatorInstanceId],
        trade_data_enabled: bool,
    ) -> KlineDataChange {
        match self {
            DataUpdate::HistoricalKlines {
                req_id,
                ticker_info,
                klines,
                ..
            } => {
                if req_id.is_some_and(|req_id| !fetch.accepts_data(req_id)) {
                    return KlineDataChange::Unchanged;
                }

                let change = data.insert_historical_klines(*ticker_info, klines, basis);
                if let Some(req_id) = req_id {
                    if matches!(&change, KlineDataChange::HistoricalKlines { .. }) {
                        fetch.mark_completed(*ticker_info, *req_id);
                    } else {
                        fetch.mark_failed_for(*ticker_info, *req_id);
                    }
                }
                change
            }
            DataUpdate::LiveKline {
                ticker_info, kline, ..
            } => {
                data.update_live_kline(ticker_info, kline, basis, indicator_ids, trade_data_enabled)
            }
            DataUpdate::LiveTrades {
                ticker_info,
                trades,
            } => {
                let update_indicator_ids = if data.has_closed_trade_correction(trades, basis) {
                    indicator_ids
                        .iter()
                        .copied()
                        .filter(|id| {
                            !indicators.trade_request_affects_indicator(*id, data, **ticker_info)
                        })
                        .collect::<Vec<_>>()
                } else {
                    indicator_ids.to_vec()
                };
                data.insert_live_trades(
                    ticker_info,
                    trades,
                    basis,
                    &update_indicator_ids,
                    trade_data_enabled,
                )
            }
            DataUpdate::HistoricalTrades {
                ticker_info,
                trades,
                is_batches_done,
                req_id,
            } => {
                if req_id.is_some_and(|req_id| !fetch.accepts_data(req_id)) {
                    return KlineDataChange::Unchanged;
                }

                let change =
                    data.insert_historical_trades(*ticker_info, trades, *is_batches_done, basis);
                if let Some(req_id) = req_id {
                    fetch.record_trade_batch(*req_id, trades.len());
                }
                fetch.complete_trade_fetch(*ticker_info, *req_id, *is_batches_done);
                change
            }
            DataUpdate::HistoricalOpenInterest {
                req_id,
                ticker_info,
                series,
            } => {
                if req_id.is_some_and(|req_id| !fetch.accepts_data(req_id)) {
                    return KlineDataChange::Unchanged;
                }

                let change = data.insert_historical_open_interest(
                    *ticker_info,
                    series,
                    &indicators.instances,
                );
                if let Some(req_id) = req_id {
                    let KlineDataChange::HistoricalInput { update, .. } = &change else {
                        unreachable!("open-interest update returned another outcome")
                    };
                    match update {
                        HistoricalInputUpdate::Rejected => {
                            fetch.mark_failed_for(*ticker_info, *req_id)
                        }
                        HistoricalInputUpdate::NoData { .. } => {
                            fetch.mark_no_data(*ticker_info, *req_id);
                        }
                        HistoricalInputUpdate::Ignored | HistoricalInputUpdate::Applied => {
                            fetch.mark_completed(*ticker_info, *req_id)
                        }
                    }
                }
                change
            }
        }
    }
}

fn remap_indicator_outputs(
    data: &mut KlineData,
    indicators: &mut IndicatorState,
    index_change: KlineIndexChange,
) {
    match index_change {
        KlineIndexChange::None => {}
        KlineIndexChange::Prefix(prefix) => {
            data.remap_indicator_outputs(KlineIndexChange::Prefix(prefix));
            indicators.remap_coverage(KlineIndexChange::Prefix(prefix));
        }
        KlineIndexChange::Explicit(previous_indices) => {
            data.remap_indicator_outputs(KlineIndexChange::Explicit(previous_indices.clone()));
            indicators.remap_coverage(KlineIndexChange::Explicit(previous_indices));
        }
    }
}

fn apply_live_state(
    data: &KlineData,
    indicators: &mut IndicatorState,
    mut coverage_frames: Vec<indicator_engine::IndicatorFrame>,
    committed_bar: bool,
    trade_data_enabled: bool,
) {
    if let Some(frame) = data.latest_indicator_frame(trade_data_enabled) {
        coverage_frames.push(frame);
    }
    indicators.update_coverage_frames(&coverage_frames);

    if committed_bar {
        indicators.set_preview_dirty(false);
    } else {
        indicators.set_preview_dirty(true);
    }
}
