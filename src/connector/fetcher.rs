use data::chart::kline::OpenInterestSeries;
use data::chart::kline::fetch_plan::supports_open_interest;
use exchange::adapter::{AdapterError, AdapterHandles, Exchange, KlinePage, StreamKind};
use exchange::{Kline, TickerInfo, Timeframe, Trade, UnixMs};
use iced::{
    Task,
    task::{Handle, Straw, sipper},
};
use rustc_hash::FxHashMap;
use std::path::PathBuf;
use std::sync::RwLock;
use uuid::Uuid;

use crate::connector::client::{DataSources, ServerClient};

pub use data::TradeFetchMode;
pub use data::chart::kline::FetchRange;

static TRADE_FETCH_MODE: RwLock<TradeFetchMode> = RwLock::new(TradeFetchMode::Off);

/// Maximum trades to request per Arrow IPC call to server.
const ARROW_LIMIT: usize = 400_000;

/// Override the global trade-fetch mode at runtime.
pub fn set_trade_fetch_mode(mode: TradeFetchMode) {
    if let Ok(mut guard) = TRADE_FETCH_MODE.write() {
        *guard = mode;
    } else {
        log::error!("Trade fetch mode lock poisoned — resetting to Off");
    }
}

/// Return the current global trade-fetch mode.
pub fn trade_fetch_mode() -> TradeFetchMode {
    TRADE_FETCH_MODE
        .read()
        .map(|g| g.clone())
        .unwrap_or(TradeFetchMode::Off)
}

/// Returns `true` when a trade-fetch source (server or exchange API) is
/// active.
pub fn is_trade_fetch_enabled() -> bool {
    trade_fetch_mode() != TradeFetchMode::Off
}

#[derive(Debug, Clone)]
pub enum FetchedData {
    Trades {
        batch: Vec<Trade>,
        /// Upper bound of the fetch gap — trades beyond this timestamp
        /// already exist on the chart and must be filtered out.
        until_time: UnixMs,
        /// Whether the paging task has finished producing batches.
        is_batches_done: bool,
        /// Whether the source returned any trade data for the request.
        has_data: bool,
        /// Optional request ID to mark completion on the chart's
        /// [`RequestHandler`] once the batch is processed.
        req_id: Option<Uuid>,
    },
    Klines {
        data: Vec<Kline>,
        req_id: Option<uuid::Uuid>,
        is_batches_done: bool,
        has_data: bool,
    },
    OpenInterest {
        series: OpenInterestSeries,
        req_id: Option<uuid::Uuid>,
    },
}

#[derive(thiserror::Error, Debug, Clone)]
pub enum ReqError {
    #[error("Request overlaps with an existing request")]
    Overlaps,
    #[error("Source has no data for the requested range")]
    NoData,
    /// A previous attempt for this range failed and the retry cooldown
    /// has not yet elapsed.
    #[error("Previous request failed, retry not yet allowed")]
    Failed,
}

/// Lifecycle of a fetch request.
///
/// * `Failed` is retried after the cooldown (transient errors may resolve).
/// * `Completed` and `NoData` retain coverage evidence for non-profile inputs.
/// * Profile coverage lives in `ProfileKlineSource`, so completed profile
///   requests are removed.
#[derive(PartialEq, Clone, Debug)]
enum RequestStatus {
    Pending,
    /// Fetch succeeded and data was inserted.
    Completed,
    /// The source returned an empty result for this range.
    NoData,
    /// The fetch failed (network error, parse error, etc.).
    Failed(u64),
}

#[derive(Default)]
pub struct RequestHandler {
    requests: FxHashMap<Uuid, FetchRequest>,
}

impl RequestHandler {
    const RETRY_AFTER_MS: u64 = 30_000;

    pub fn add_request(&mut self, fetch: FetchRange) -> Result<Option<Uuid>, ReqError> {
        let request = FetchRequest::new(fetch);
        let id = Uuid::new_v4();

        if let Some((existing_id, status)) = self.requests.iter().find_map(|(k, v)| {
            if v.same_with(&request) {
                Some((*k, v.status.clone()))
            } else {
                None
            }
        }) {
            match status {
                RequestStatus::Completed => return Ok(None),
                RequestStatus::NoData => return Err(ReqError::NoData),
                RequestStatus::Pending if !is_kline_fetch(&fetch) => {
                    return Err(ReqError::Overlaps);
                }
                RequestStatus::Failed(ts) if !is_kline_fetch(&fetch) => {
                    let now_ms = chrono::Utc::now().timestamp_millis() as u64;
                    if now_ms.saturating_sub(ts) > Self::RETRY_AFTER_MS {
                        if let Some(existing) = self.requests.get_mut(&existing_id) {
                            existing.status = RequestStatus::Pending;
                            existing.received_trades = 0;
                        }
                        return Ok(Some(existing_id));
                    }
                    return Err(ReqError::Failed);
                }
                RequestStatus::Pending | RequestStatus::Failed(_) => {}
            }
        }

        if is_kline_fetch(&fetch) {
            if self.requests.values().any(|existing| {
                existing.status == RequestStatus::Pending && existing.overlaps(&fetch)
            }) {
                return Err(ReqError::Overlaps);
            }

            let now_ms = chrono::Utc::now().timestamp_millis() as u64;
            let mut expired_failed = Vec::new();
            for (existing_id, existing) in &self.requests {
                let RequestStatus::Failed(failed_at) = existing.status else {
                    continue;
                };
                if !existing.overlaps(&fetch) {
                    continue;
                }
                if now_ms.saturating_sub(failed_at) <= Self::RETRY_AFTER_MS {
                    return Err(ReqError::Failed);
                }
                expired_failed.push(*existing_id);
            }
            for existing_id in expired_failed {
                self.requests.remove(&existing_id);
            }
        }

        self.requests.insert(id, request);
        Ok(Some(id))
    }

    pub(crate) fn is_pending(&self, id: Uuid) -> bool {
        self.requests
            .get(&id)
            .is_some_and(|request| request.status == RequestStatus::Pending)
    }

    pub(crate) fn is_profile_request(&self, id: Uuid) -> bool {
        self.requests.get(&id).is_some_and(|request| {
            request.status == RequestStatus::Pending
                && matches!(request.fetch_type, FetchRange::ProfileKline { .. })
        })
    }

    pub(crate) fn request_ids(&self) -> impl Iterator<Item = Uuid> + '_ {
        self.requests.keys().copied()
    }

    pub(crate) fn has_pending_trade_fetch(&self) -> bool {
        self.requests.values().any(|request| {
            request.status == RequestStatus::Pending
                && matches!(request.fetch_type, FetchRange::Trades(..))
        })
    }

    pub(crate) fn completed_trade_ranges(&self) -> impl Iterator<Item = (UnixMs, UnixMs)> + '_ {
        self.requests.values().filter_map(|request| {
            if !matches!(
                &request.status,
                RequestStatus::Completed | RequestStatus::NoData
            ) {
                return None;
            }
            match request.fetch_type {
                FetchRange::Trades(from, to) => Some((from, to)),
                _ => None,
            }
        })
    }

    pub(crate) fn has_pending_profile_fetch(&self) -> bool {
        self.requests.values().any(|request| {
            request.status == RequestStatus::Pending
                && matches!(request.fetch_type, FetchRange::ProfileKline { .. })
        })
    }

    pub(crate) fn pending_ranges(&self) -> impl Iterator<Item = FetchRange> + '_ {
        self.requests
            .values()
            .filter(|request| request.status == RequestStatus::Pending)
            .map(|request| request.fetch_type)
    }

    pub(crate) fn pending_trade_count(&self) -> Option<usize> {
        self.requests.values().find_map(|request| {
            (request.status == RequestStatus::Pending
                && matches!(request.fetch_type, FetchRange::Trades(..)))
            .then_some(request.received_trades)
        })
    }

    pub(crate) fn record_trade_batch(&mut self, id: Uuid, count: usize) -> bool {
        let Some(request) = self.requests.get_mut(&id) else {
            return false;
        };

        if request.status != RequestStatus::Pending
            || !matches!(request.fetch_type, FetchRange::Trades(..))
        {
            return false;
        }

        request.received_trades = request.received_trades.saturating_add(count);
        true
    }

    pub fn mark_completed(&mut self, id: Uuid) -> bool {
        if self.is_profile_request(id) {
            return self.mark_profile_completed(id).is_some();
        }

        let Some(request) = self.requests.get_mut(&id) else {
            return false;
        };
        if request.status != RequestStatus::Pending {
            return false;
        }

        request.status = RequestStatus::Completed;
        true
    }

    pub(crate) fn mark_profile_completed(&mut self, id: Uuid) -> Option<FetchRange> {
        let request = self.requests.get(&id)?;
        if request.status != RequestStatus::Pending
            || !matches!(request.fetch_type, FetchRange::ProfileKline { .. })
        {
            return None;
        }
        self.requests.remove(&id).map(|request| request.fetch_type)
    }

    /// Mark a request as completed with no data — the source returned an
    /// empty result.  The range will never be retried.
    pub fn mark_no_data(&mut self, id: Uuid) -> bool {
        if self.is_profile_request(id) {
            self.requests.remove(&id);
            return true;
        }

        let Some(request) = self.requests.get_mut(&id) else {
            return false;
        };
        if request.status != RequestStatus::Pending {
            return false;
        }
        request.status = RequestStatus::NoData;
        true
    }

    pub fn mark_failed(&mut self, id: Uuid) -> bool {
        if let Some(request) = self.requests.get_mut(&id) {
            if request.status != RequestStatus::Pending {
                return false;
            }
            let timestamp = chrono::Utc::now().timestamp_millis() as u64;
            request.status = RequestStatus::Failed(timestamp);
            true
        } else {
            false
        }
    }

    pub fn clear_open_interest_requests(&mut self) -> Vec<Uuid> {
        let removed = self
            .requests
            .iter()
            .filter(|(_, request)| matches!(request.fetch_type, FetchRange::OpenInterest { .. }))
            .map(|(id, _)| *id)
            .collect();
        self.requests
            .retain(|_, request| !matches!(request.fetch_type, FetchRange::OpenInterest { .. }));
        removed
    }

    pub(crate) fn clear_pending_profile_requests(&mut self) -> Vec<Uuid> {
        let removed = self
            .requests
            .iter()
            .filter(|(_, request)| {
                matches!(request.fetch_type, FetchRange::ProfileKline { .. })
                    && request.status == RequestStatus::Pending
            })
            .map(|(id, _)| *id)
            .collect();
        self.requests.retain(|_, request| {
            !matches!(request.fetch_type, FetchRange::ProfileKline { .. })
                || request.status != RequestStatus::Pending
        });
        removed
    }

    pub(crate) fn clear_pending_trade_requests(&mut self) -> Vec<Uuid> {
        let removed = self
            .requests
            .iter()
            .filter(|(_, request)| {
                matches!(request.fetch_type, FetchRange::Trades(..))
                    && request.status == RequestStatus::Pending
            })
            .map(|(id, _)| *id)
            .collect();
        self.requests.retain(|_, request| {
            !matches!(request.fetch_type, FetchRange::Trades(..))
                || request.status != RequestStatus::Pending
        });
        removed
    }

    pub(crate) fn reset_for_basis_change(&mut self) {
        self.requests.clear();
    }
}

#[derive(PartialEq, Debug)]
struct FetchRequest {
    fetch_type: FetchRange,
    status: RequestStatus,
    received_trades: usize,
}

impl FetchRequest {
    fn new(fetch_type: FetchRange) -> Self {
        FetchRequest {
            fetch_type,
            status: RequestStatus::Pending,
            received_trades: 0,
        }
    }

    fn same_with(&self, other: &FetchRequest) -> bool {
        self.same_with_range(&other.fetch_type)
    }

    /// Check whether the stored [`FetchRange`] matches a given range.
    fn same_with_range(&self, range: &FetchRange) -> bool {
        match (&self.fetch_type, range) {
            (FetchRange::Kline(s1, e1), FetchRange::Kline(s2, e2)) => e1 == e2 && s1 == s2,
            (
                FetchRange::ProfileKline {
                    timeframe: t1,
                    from: s1,
                    to: e1,
                },
                FetchRange::ProfileKline {
                    timeframe: t2,
                    from: s2,
                    to: e2,
                },
            ) => t1 == t2 && e1 == e2 && s1 == s2,
            (
                FetchRange::OpenInterest { from: s1, to: e1 },
                FetchRange::OpenInterest { from: s2, to: e2 },
            ) => e1 == e2 && s1 == s2,
            (FetchRange::Trades(s1, e1), FetchRange::Trades(s2, e2)) => e1 == e2 && s1 == s2,
            _ => false,
        }
    }

    fn overlaps(&self, range: &FetchRange) -> bool {
        match (&self.fetch_type, range) {
            (FetchRange::Kline(s1, e1), FetchRange::Kline(s2, e2)) => s1 <= e2 && s2 <= e1,
            (
                FetchRange::ProfileKline {
                    timeframe: t1,
                    from: s1,
                    to: e1,
                },
                FetchRange::ProfileKline {
                    timeframe: t2,
                    from: s2,
                    to: e2,
                },
            ) => t1 == t2 && s1 < e2 && s2 < e1,
            _ => false,
        }
    }
}

fn is_kline_fetch(range: &FetchRange) -> bool {
    matches!(
        range,
        FetchRange::Kline(..) | FetchRange::ProfileKline { .. }
    )
}

pub struct FetchSpec {
    pub req_id: uuid::Uuid,
    pub fetch: FetchRange,
    pub stream: Option<StreamKind>,
}

impl From<(uuid::Uuid, FetchRange, Option<StreamKind>)> for FetchSpec {
    fn from(t: (uuid::Uuid, FetchRange, Option<StreamKind>)) -> Self {
        FetchSpec {
            req_id: t.0,
            fetch: t.1,
            stream: t.2,
        }
    }
}

impl std::fmt::Debug for FetchSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FetchSpec")
            .field("req_id", &self.req_id)
            .field("fetch", &self.fetch)
            .field("stream", &self.stream)
            .finish()
    }
}

impl Clone for FetchSpec {
    fn clone(&self) -> Self {
        FetchSpec {
            req_id: self.req_id,
            fetch: self.fetch,
            stream: self.stream,
        }
    }
}

#[derive(Debug, Clone)]
pub enum FetchUpdate {
    Data {
        pane_id: Uuid,
        stream: StreamKind,
        data: FetchedData,
    },
    Error {
        pane_id: Uuid,
        error: String,
        req_id: Option<Uuid>,
    },
}

pub fn request_fetch(
    sources: &DataSources,
    pane_id: Uuid,
    ready_streams: &[StreamKind],
    req_id: Uuid,
    fetch: FetchRange,
    stream: Option<StreamKind>,
    on_fetch_handle: &mut impl FnMut(Uuid, Handle),
) -> Task<FetchUpdate> {
    let handles = sources.exchange.clone();

    match fetch {
        FetchRange::Kline(from, to) => {
            let kline_stream = if let Some(s) = stream {
                Some((s, pane_id))
            } else {
                ready_streams.iter().find_map(|stream| {
                    if let StreamKind::Kline { .. } = stream {
                        Some((*stream, pane_id))
                    } else {
                        None
                    }
                })
            };

            if let Some((stream, pane_uid)) = kline_stream {
                return kline_fetch_task(
                    handles,
                    pane_uid,
                    stream,
                    Some(req_id),
                    Some((from, to)),
                    on_fetch_handle,
                );
            }
        }
        FetchRange::ProfileKline {
            timeframe,
            from,
            to,
        } => {
            let kline_stream = if let Some(s) = stream {
                Some((s, pane_id))
            } else {
                ready_streams.iter().find_map(|stream| {
                    if let StreamKind::Kline { .. } = stream {
                        Some((*stream, pane_id))
                    } else {
                        None
                    }
                })
            };

            if let Some((StreamKind::Kline { ticker_info, .. }, pane_uid)) = kline_stream {
                let (task, handle) = Task::sip(
                    fetch_klines_paged(handles, ticker_info, timeframe, from, to),
                    move |batch| FetchUpdate::Data {
                        pane_id: pane_uid,
                        stream: StreamKind::Kline {
                            ticker_info,
                            timeframe,
                        },
                        data: FetchedData::Klines {
                            data: batch,
                            req_id: Some(req_id),
                            is_batches_done: false,
                            has_data: true,
                        },
                    },
                    move |result| match result {
                        Ok(has_data) => FetchUpdate::Data {
                            pane_id: pane_uid,
                            stream: StreamKind::Kline {
                                ticker_info,
                                timeframe,
                            },
                            data: FetchedData::Klines {
                                data: Vec::new(),
                                req_id: Some(req_id),
                                is_batches_done: true,
                                has_data,
                            },
                        },
                        Err(err) => {
                            log::error!("Kline fetch failed: {err}");
                            FetchUpdate::Error {
                                pane_id: pane_uid,
                                error: err.ui_message(),
                                req_id: Some(req_id),
                            }
                        }
                    },
                )
                .abortable();

                on_fetch_handle(req_id, handle.abort_on_drop());
                return task;
            }
        }
        FetchRange::OpenInterest { from, to } => {
            let kline_stream = if let Some(s) = stream {
                Some((s, pane_id))
            } else {
                ready_streams.iter().find_map(|stream| {
                    if let StreamKind::Kline { .. } = stream {
                        Some((*stream, pane_id))
                    } else {
                        None
                    }
                })
            };

            if let Some((stream, pane_uid)) = kline_stream {
                return open_interest_fetch_task(
                    handles,
                    pane_uid,
                    stream,
                    Some(req_id),
                    Some((from, to)),
                    on_fetch_handle,
                );
            }
        }
        FetchRange::Trades(from_time, to_time) => {
            let trade_stream = stream
                .filter(|stream| matches!(stream, StreamKind::Trades { .. }))
                .or_else(|| {
                    ready_streams.iter().find_map(|stream| {
                        if let StreamKind::Trades { .. } = stream {
                            Some(*stream)
                        } else {
                            None
                        }
                    })
                });

            if let Some(trade_stream @ StreamKind::Trades { ticker_info }) = trade_stream {
                let is_binance = matches!(
                    ticker_info.exchange(),
                    Exchange::BinanceSpot | Exchange::BinanceLinear | Exchange::BinanceInverse
                );
                let mode = trade_fetch_mode();
                let server = sources.server.clone();
                let data_path = data::data_path(Some("market_data/binance/"));

                if let Some(ref client) = server {
                    log::debug!(
                        "Trade fetch: using server at {} ({})",
                        client.base_url(),
                        ticker_info.exchange()
                    );
                } else if mode == TradeFetchMode::Server {
                    log::error!(
                        "Server mode selected but server URL is invalid, check Network Manager settings"
                    );
                    return Task::done(FetchUpdate::Error {
                        pane_id,
                        error: "Server mode selected but the server URL is invalid.".to_string(),
                        req_id: Some(req_id),
                    });
                } else if is_binance {
                    log::debug!(
                        "Trade fetch: using direct exchange API for {}",
                        ticker_info.exchange()
                    );
                } else {
                    log::warn!(
                        "Trade fetch via exchange API is only supported for Binance, got {}",
                        ticker_info.exchange()
                    );
                    return Task::done(FetchUpdate::Error {
                        pane_id,
                        error: format!(
                            "Trade fetch via exchange API is only supported for Binance, got {}",
                            ticker_info.exchange()
                        ),
                        req_id: Some(req_id),
                    });
                }

                let (task, handle) = Task::sip(
                    fetch_trades_paged(server, handles, ticker_info, from_time, to_time, data_path),
                    move |batch| {
                        let data = FetchedData::Trades {
                            batch,
                            req_id: Some(req_id),
                            until_time: to_time,
                            is_batches_done: false,
                            has_data: true,
                        };

                        FetchUpdate::Data {
                            pane_id,
                            data,
                            stream: trade_stream,
                        }
                    },
                    move |result| match result {
                        Ok(true) => FetchUpdate::Data {
                            pane_id,
                            data: FetchedData::Trades {
                                batch: Vec::new(),
                                req_id: Some(req_id),
                                until_time: to_time,
                                is_batches_done: true,
                                has_data: true,
                            },
                            stream: trade_stream,
                        },
                        Ok(false) => {
                            // Source returned no data for this range. Produce
                            // an empty batch so the dashboard calls mark_no_data.
                            FetchUpdate::Data {
                                pane_id,
                                data: FetchedData::Trades {
                                    batch: Vec::new(),
                                    req_id: Some(req_id),
                                    until_time: to_time,
                                    is_batches_done: true,
                                    has_data: false,
                                },
                                stream: trade_stream,
                            }
                        }
                        Err(err) => {
                            log::error!("Trade fetch failed: {err}");
                            FetchUpdate::Error {
                                pane_id,
                                error: err.ui_message(),
                                req_id: Some(req_id),
                            }
                        }
                    },
                )
                .abortable();

                on_fetch_handle(req_id, handle.abort_on_drop());

                return task;
            }
        }
    }

    Task::none()
}

pub fn request_fetch_many(
    sources: &DataSources,
    pane_id: Uuid,
    ready_streams: &[StreamKind],
    reqs: impl IntoIterator<Item = (Uuid, FetchRange, Option<StreamKind>)>,
    mut on_fetch_handle: impl FnMut(Uuid, Handle),
) -> Task<FetchUpdate> {
    Task::batch(reqs.into_iter().map(|(req_id, fetch, stream)| {
        request_fetch(
            sources,
            pane_id,
            ready_streams,
            req_id,
            fetch,
            stream,
            &mut on_fetch_handle,
        )
    }))
}

pub fn open_interest_fetch_task(
    handles: AdapterHandles,
    pane_id: Uuid,
    stream: StreamKind,
    req_id: Option<Uuid>,
    range: Option<(UnixMs, UnixMs)>,
    on_fetch_handle: &mut impl FnMut(Uuid, Handle),
) -> Task<FetchUpdate> {
    let fetch_task = match stream {
        StreamKind::Kline {
            ticker_info,
            timeframe,
        } => {
            if !supports_open_interest(ticker_info, timeframe) {
                return Task::done(FetchUpdate::Error {
                    pane_id,
                    error: format!(
                        "Open Interest is not available for {}",
                        ticker_info.exchange()
                    ),
                    req_id,
                });
            }

            let fetch = async move {
                handles
                    .fetch_open_interest(ticker_info, timeframe, range)
                    .await
                    .map(|data| {
                        OpenInterestSeries::from_points(
                            data.into_iter().map(|point| (point.time, point.value)),
                        )
                    })
            };

            Task::perform(
                iced::futures::TryFutureExt::map_err(fetch, move |err| {
                    log::error!("Open Interest fetch failed: {err}");
                    err.ui_message()
                }),
                move |result| match result {
                    Ok(batch) => {
                        let data = FetchedData::OpenInterest {
                            series: batch,
                            req_id,
                        };
                        FetchUpdate::Data {
                            pane_id,
                            data,
                            stream,
                        }
                    }
                    Err(err) => FetchUpdate::Error {
                        pane_id,
                        error: err,
                        req_id,
                    },
                },
            )
        }
        _ => Task::none(),
    };

    let (task, handle) = fetch_task.abortable();
    if let Some(req_id) = req_id {
        on_fetch_handle(req_id, handle.abort_on_drop());
    }
    task
}

pub fn kline_fetch_task(
    handles: AdapterHandles,
    pane_id: Uuid,
    stream: StreamKind,
    req_id: Option<Uuid>,
    range: Option<(UnixMs, UnixMs)>,
    on_fetch_handle: &mut impl FnMut(Uuid, Handle),
) -> Task<FetchUpdate> {
    let fetch_task = match stream {
        StreamKind::Kline {
            ticker_info,
            timeframe,
        } => {
            let fetch = async move { handles.fetch_klines(ticker_info, timeframe, range).await };

            Task::perform(
                iced::futures::TryFutureExt::map_err(fetch, |err| {
                    log::error!("Kline fetch failed: {err}");
                    err.ui_message()
                }),
                move |result| match result {
                    Ok(klines) => {
                        let data = FetchedData::Klines {
                            has_data: !klines.is_empty(),
                            data: klines,
                            req_id,
                            is_batches_done: true,
                        };
                        FetchUpdate::Data {
                            pane_id,
                            data,
                            stream,
                        }
                    }
                    Err(err) => FetchUpdate::Error {
                        pane_id,
                        error: err,
                        req_id,
                    },
                },
            )
        }
        _ => Task::none(),
    };

    let (task, handle) = fetch_task.abortable();
    if let Some(req_id) = req_id {
        on_fetch_handle(req_id, handle.abort_on_drop());
    }
    task
}

fn fetch_klines_paged(
    handles: AdapterHandles,
    ticker_info: TickerInfo,
    timeframe: Timeframe,
    from_time: UnixMs,
    to_time: UnixMs,
) -> impl Straw<bool, Vec<Kline>, AdapterError> {
    sipper(async move |mut progress| {
        let end = to_time.min(UnixMs::now().floor_to(timeframe));
        let mut cursor = from_time;
        let mut had_data = false;

        while cursor < end {
            let page = handles
                .fetch_klines_page(ticker_info, timeframe, cursor, end)
                .await?;
            let next_cursor = advance_kline_page(cursor, end, &page)?;
            if !page.data.is_empty() {
                had_data = true;
                progress.send(page.data).await;
            }
            cursor = next_cursor;
        }

        Ok(had_data)
    })
}

fn advance_kline_page(
    cursor: UnixMs,
    end: UnixMs,
    page: &KlinePage,
) -> Result<UnixMs, AdapterError> {
    if page.next_from <= cursor {
        return Err(AdapterError::ParseError(
            "Kline paging range did not advance".to_string(),
        ));
    }
    if page.next_from > end {
        return Err(AdapterError::ParseError(
            "Kline paging cursor exceeded requested end".to_string(),
        ));
    }
    Ok(page.next_from)
}

/// Fetch trades from the configured source using a single forward-paging
/// loop (oldest → newest).
///
/// Returns `Ok(true)` when at least one trade was received, `Ok(false)`
/// when the source confirmed the range has no data.
pub fn fetch_trades_paged(
    server: Option<ServerClient>,
    handles: AdapterHandles,
    ticker_info: TickerInfo,
    from_time: UnixMs,
    to_time: UnixMs,
    data_path: PathBuf,
) -> impl Straw<bool, Vec<Trade>, AdapterError> {
    sipper(async move |mut progress| {
        let mut cursor = from_time;
        let mut had_data = false;

        while cursor < to_time {
            let prev_cursor = cursor;

            if let Some(ref client) = server {
                let parsed = client
                    .fetch_trades_arrow(ticker_info, cursor, to_time, ARROW_LIMIT)
                    .await?;

                let is_empty = parsed.raw_row_count == 0;
                if !is_empty {
                    had_data = had_data || !parsed.trades.is_empty();
                    cursor = parsed.last_ts.map_or(cursor, |t| t.saturating_add(1));
                }
                if !parsed.trades.is_empty() {
                    progress.send(parsed.trades).await;
                }
                if is_empty {
                    break;
                }
            } else {
                let batch = handles
                    .fetch_trades(ticker_info, cursor, Some(data_path.clone()))
                    .await?;

                if batch.is_empty() {
                    break;
                }

                had_data = true;
                cursor = batch.last().map_or(cursor, |t| t.time.saturating_add(1));
                progress.send(batch).await;
            }

            if cursor <= prev_cursor {
                log::error!(
                    "paging cursor did not advance past {prev_cursor:?} despite a non-empty response; \
             aborting to avoid an infinite loop (source may be malformed)"
                );
                return Err(AdapterError::ParseError(
                    "Source may be malformed. Check logs for details.".to_string(),
                ));
            }
        }

        Ok(had_data)
    })
}
