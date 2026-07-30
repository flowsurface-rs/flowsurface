use exchange::adapter::{AdapterError, AdapterHandles, Exchange, StreamKind};
use exchange::{Kline, OpenInterest, TickerInfo, Trade, UnixMs};
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
        /// Optional request ID to mark completion on the chart's
        /// [`RequestHandler`] once the batch is processed.
        req_id: Option<Uuid>,
    },
    Klines {
        data: Vec<Kline>,
        req_id: Option<uuid::Uuid>,
    },
    OI {
        data: Vec<OpenInterest>,
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
/// * `Completed` and `NoData` are never retried — the data is either
///   already present or the source confirmed the range is empty.
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

        if let Some((existing_id, existing_req)) = self.requests.iter_mut().find_map(|(k, v)| {
            if v.same_with(&request) {
                Some((*k, v))
            } else {
                None
            }
        }) {
            let now_ms = chrono::Utc::now().timestamp_millis() as u64;
            let retry_after_ms = Self::RETRY_AFTER_MS;
            let status = existing_req.status.clone();

            return match status {
                RequestStatus::Completed => Ok(None),
                RequestStatus::Pending => Err(ReqError::Overlaps),
                RequestStatus::NoData => Err(ReqError::NoData),
                RequestStatus::Failed(ts) => {
                    if now_ms - ts > retry_after_ms {
                        existing_req.status = RequestStatus::Pending;
                        Ok(Some(existing_id))
                    } else {
                        Err(ReqError::Failed)
                    }
                }
            };
        }

        self.requests.insert(id, request);
        Ok(Some(id))
    }

    /// Returns `true` when a request with the given range is currently
    /// pending (in-flight and not yet completed or failed).
    pub fn has_pending(&self, range: &FetchRange) -> bool {
        self.requests
            .values()
            .any(|r| r.status == RequestStatus::Pending && r.same_with_range(range))
    }

    pub fn mark_completed(&mut self, id: Uuid) {
        if let Some(request) = self.requests.get_mut(&id) {
            request.status = RequestStatus::Completed;
        } else {
            log::warn!("Request not found: {:?}", id);
        }
    }

    /// Mark a request as completed with no data — the source returned an
    /// empty result.  The range will never be retried.
    pub fn mark_no_data(&mut self, id: Uuid) {
        if let Some(request) = self.requests.get_mut(&id) {
            request.status = RequestStatus::NoData;
        } else {
            log::warn!("Request not found: {:?}", id);
        }
    }

    pub fn mark_failed(&mut self, id: Uuid) {
        if let Some(request) = self.requests.get_mut(&id) {
            let timestamp = chrono::Utc::now().timestamp_millis() as u64;
            request.status = RequestStatus::Failed(timestamp);
        } else {
            log::warn!("Request not found: {:?}", id);
        }
    }
}

#[derive(PartialEq, Debug, Clone, Copy)]
pub enum FetchRange {
    Kline(UnixMs, UnixMs),
    OpenInterest(UnixMs, UnixMs),
    Trades(UnixMs, UnixMs),
}

#[derive(PartialEq, Debug)]
struct FetchRequest {
    fetch_type: FetchRange,
    status: RequestStatus,
}

impl FetchRequest {
    fn new(fetch_type: FetchRange) -> Self {
        FetchRequest {
            fetch_type,
            status: RequestStatus::Pending,
        }
    }

    fn same_with(&self, other: &FetchRequest) -> bool {
        self.same_with_range(&other.fetch_type)
    }

    /// Check whether the stored [`FetchRange`] matches a given range.
    fn same_with_range(&self, range: &FetchRange) -> bool {
        match (&self.fetch_type, range) {
            (FetchRange::Kline(s1, e1), FetchRange::Kline(s2, e2)) => e1 == e2 && s1 == s2,
            (FetchRange::OpenInterest(s1, e1), FetchRange::OpenInterest(s2, e2)) => {
                e1 == e2 && s1 == s2
            }
            (FetchRange::Trades(s1, e1), FetchRange::Trades(s2, e2)) => e1 == e2 && s1 == s2,
            _ => false,
        }
    }
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

#[derive(Debug, Clone, Copy, PartialEq)]
/// Used for showing updates in pane title bar.
pub enum FetchTaskStatus {
    Loading(InfoKind),
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InfoKind {
    FetchingKlines,
    FetchingTrades(usize),
    FetchingOI,
}

#[derive(Debug, Clone)]
pub enum FetchUpdate {
    Status {
        pane_id: Uuid,
        status: FetchTaskStatus,
    },
    Data {
        layout_id: Uuid,
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
    layout_id: Uuid,
    req_id: Uuid,
    fetch: FetchRange,
    stream: Option<StreamKind>,
    on_trade_handle: &mut impl FnMut(Handle),
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
                    layout_id,
                    pane_uid,
                    stream,
                    Some(req_id),
                    Some((from, to)),
                );
            }
        }
        FetchRange::OpenInterest(from, to) => {
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
                return oi_fetch_task(
                    handles.clone(),
                    layout_id,
                    pane_uid,
                    stream,
                    Some(req_id),
                    Some((from, to)),
                );
            }
        }
        FetchRange::Trades(from_time, to_time) => {
            let trade_info = ready_streams.iter().find_map(|stream| {
                if let StreamKind::Trades { ticker_info } = stream {
                    Some((*ticker_info, pane_id, *stream))
                } else {
                    None
                }
            });

            if let Some((ticker_info, pane_id, stream)) = trade_info {
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
                        };

                        FetchUpdate::Data {
                            layout_id,
                            pane_id,
                            data,
                            stream,
                        }
                    },
                    move |result| match result {
                        Ok(true) => FetchUpdate::Status {
                            pane_id,
                            status: FetchTaskStatus::Completed,
                        },
                        Ok(false) => {
                            // Source returned no data for this range. Produce
                            // an empty batch so the dashboard calls mark_no_data.
                            FetchUpdate::Data {
                                layout_id,
                                pane_id,
                                data: FetchedData::Trades {
                                    batch: Vec::new(),
                                    req_id: Some(req_id),
                                    until_time: to_time,
                                },
                                stream,
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

                on_trade_handle(handle.abort_on_drop());

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
    layout_id: Uuid,
    reqs: impl IntoIterator<Item = (Uuid, FetchRange, Option<StreamKind>)>,
    mut on_trade_handle: impl FnMut(Handle),
) -> Task<FetchUpdate> {
    let mut tasks = Vec::new();

    for (req_id, fetch, stream) in reqs {
        tasks.push(request_fetch(
            sources,
            pane_id,
            ready_streams,
            layout_id,
            req_id,
            fetch,
            stream,
            &mut on_trade_handle,
        ));
    }

    Task::batch(tasks)
}

pub fn oi_fetch_task(
    handles: AdapterHandles,
    layout_id: Uuid,
    pane_id: Uuid,
    stream: StreamKind,
    req_id: Option<Uuid>,
    range: Option<(UnixMs, UnixMs)>,
) -> Task<FetchUpdate> {
    let update_status = Task::done(FetchUpdate::Status {
        pane_id,
        status: FetchTaskStatus::Loading(InfoKind::FetchingOI),
    });

    let fetch_task = match stream {
        StreamKind::Kline {
            ticker_info,
            timeframe,
        } => {
            let fetch = async move {
                handles
                    .fetch_open_interest(ticker_info, timeframe, range)
                    .await
            };

            Task::perform(
                iced::futures::TryFutureExt::map_err(fetch, |err| {
                    log::error!("Open interest fetch failed: {err}");
                    err.ui_message()
                }),
                move |result| match result {
                    Ok(oi) => {
                        let data = FetchedData::OI { data: oi, req_id };
                        FetchUpdate::Data {
                            layout_id,
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

    update_status.chain(fetch_task)
}

pub fn kline_fetch_task(
    handles: AdapterHandles,
    layout_id: Uuid,
    pane_id: Uuid,
    stream: StreamKind,
    req_id: Option<Uuid>,
    range: Option<(UnixMs, UnixMs)>,
) -> Task<FetchUpdate> {
    let update_status = Task::done(FetchUpdate::Status {
        pane_id,
        status: FetchTaskStatus::Loading(InfoKind::FetchingKlines),
    });

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
                            data: klines,
                            req_id,
                        };
                        FetchUpdate::Data {
                            layout_id,
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

    update_status.chain(fetch_task)
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
            let batch = if let Some(ref client) = server {
                client
                    .fetch_trades_arrow(ticker_info, cursor, to_time, ARROW_LIMIT)
                    .await?
            } else {
                handles
                    .fetch_trades(ticker_info, cursor, Some(data_path.clone()))
                    .await?
            };

            if batch.is_empty() {
                break;
            }

            had_data = true;
            cursor = batch.last().map_or(cursor, |t| t.time.saturating_add(1));
            progress.send(batch).await;
        }

        Ok(had_data)
    })
}
