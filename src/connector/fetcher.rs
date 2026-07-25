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

use crate::connector::client::ServerClient;

pub use data::TradeFetchMode;

static TRADE_FETCH_MODE: RwLock<TradeFetchMode> = RwLock::new(TradeFetchMode::Off);

/// Cached server client — created once and reused so the underlying
/// `reqwest::Client` (connection pool, TLS state) is preserved across
/// fetch calls.  Invalidated whenever [`set_trade_fetch_mode`] is called.
static SERVER_CLIENT: RwLock<Option<ServerClient>> = RwLock::new(None);

pub fn set_trade_fetch_mode(mode: TradeFetchMode) {
    if let Ok(mut guard) = TRADE_FETCH_MODE.write() {
        *guard = mode;
    } else {
        log::error!("Trade fetch mode lock poisoned — resetting to Off");
    }

    if let Ok(mut guard) = SERVER_CLIENT.write() {
        *guard = None;
    }
}

pub fn trade_fetch_mode() -> TradeFetchMode {
    TRADE_FETCH_MODE
        .read()
        .map(|g| g.clone())
        .unwrap_or(TradeFetchMode::Off)
}

pub fn is_trade_fetch_enabled() -> bool {
    trade_fetch_mode() != TradeFetchMode::Off
}

/// Returns a clone of the global server client, if one was configured
/// via the current [`TradeFetchMode::Server`] variant.
///
/// The underlying `reqwest::Client` is **cached** after the first call
/// so connection-pooling is preserved across fetch batches.  The cache
/// is invalidated whenever [`set_trade_fetch_mode`] is
/// called (e.g. after a restart with a new server URL).
fn server_client() -> Option<ServerClient> {
    if let Ok(guard) = SERVER_CLIENT.read()
        && let Some(ref client) = *guard
    {
        return Some(client.clone());
    }

    let mode = trade_fetch_mode();
    let client = if let TradeFetchMode::Server {
        url: Some(ref url),
        auth_token,
    } = mode
    {
        if url.is_empty() {
            None
        } else {
            ServerClient::new(url, auth_token.clone())
        }
    } else {
        None
    };

    if let Ok(mut guard) = SERVER_CLIENT.write() {
        *guard = client.clone();
    }

    client
}

#[derive(Debug, Clone)]
pub enum FetchedData {
    Trades {
        batch: Vec<Trade>,
        /// Upper bound of the fetch gap — trades beyond this timestamp
        /// already exist on the chart and must be filtered out.
        until_time: UnixMs,
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
    #[error("Request is already failed: {0}")]
    Failed(String),
    #[error("Request overlaps with an existing request")]
    Overlaps,
}

#[derive(PartialEq, Debug)]
enum RequestStatus {
    Pending,
    Completed(u64),
    Failed(String),
}

#[derive(Default)]
pub struct RequestHandler {
    requests: FxHashMap<Uuid, FetchRequest>,
}

impl RequestHandler {
    pub fn add_request(&mut self, fetch: FetchRange) -> Result<Option<Uuid>, ReqError> {
        let request = FetchRequest::new(fetch);
        let id = Uuid::new_v4();

        if let Some((existing_id, existing_req)) = self.requests.iter().find_map(|(k, v)| {
            if v.same_with(&request) {
                Some((*k, v))
            } else {
                None
            }
        }) {
            return match &existing_req.status {
                RequestStatus::Failed(error_msg) => Err(ReqError::Failed(error_msg.clone())),
                RequestStatus::Completed(ts) => {
                    // retry completed requests after a cooldown
                    // to handle data source failures or outdated results gracefully
                    if chrono::Utc::now().timestamp_millis() as u64 - ts > 30_000 {
                        Ok(Some(existing_id))
                    } else {
                        Ok(None)
                    }
                }
                RequestStatus::Pending => Err(ReqError::Overlaps),
            };
        }

        self.requests.insert(id, request);
        Ok(Some(id))
    }

    pub fn mark_completed(&mut self, id: Uuid) {
        if let Some(request) = self.requests.get_mut(&id) {
            let timestamp = chrono::Utc::now().timestamp_millis() as u64;
            request.status = RequestStatus::Completed(timestamp);
        } else {
            log::warn!("Request not found: {:?}", id);
        }
    }

    pub fn mark_failed(&mut self, id: Uuid, error: String) {
        if let Some(request) = self.requests.get_mut(&id) {
            request.status = RequestStatus::Failed(error);
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
        match (&self.fetch_type, &other.fetch_type) {
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
pub enum InfoKind {
    FetchingKlines,
    FetchingTrades(usize),
    FetchingOI,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FetchTaskStatus {
    Loading(InfoKind),
    Completed,
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
    },
}

pub fn request_fetch(
    handles: AdapterHandles,
    pane_id: Uuid,
    ready_streams: &[StreamKind],
    layout_id: Uuid,
    req_id: Uuid,
    fetch: FetchRange,
    stream: Option<StreamKind>,
    on_trade_handle: &mut impl FnMut(Handle),
) -> Task<FetchUpdate> {
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
                    handles.clone(),
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
                let server = server_client();
                let mode = trade_fetch_mode();
                let data_path = data::data_path(Some("market_data/binance/"));

                if let Some(ref client) = server {
                    log::info!(
                        "Trade fetch: using server at {} ({})",
                        client.base_url(),
                        ticker_info.exchange()
                    );
                } else if matches!(mode, TradeFetchMode::Server { .. }) {
                    log::error!(
                        "Server mode selected but server URL is invalid, check Network Manager settings"
                    );
                    return Task::done(FetchUpdate::Error {
                        pane_id,
                        error: "Server mode selected but the server URL is invalid.".to_string(),
                    });
                } else if is_binance {
                    log::info!(
                        "Trade fetch: using direct exchange API for {}",
                        ticker_info.exchange()
                    );
                } else {
                    return Task::none();
                }

                let (task, handle) = Task::sip(
                    fetch_trades_paged(
                        server,
                        handles.clone(),
                        ticker_info,
                        from_time,
                        to_time,
                        data_path,
                    ),
                    move |batch| {
                        let data = FetchedData::Trades {
                            batch,
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
                        Ok(()) => FetchUpdate::Status {
                            pane_id,
                            status: FetchTaskStatus::Completed,
                        },
                        Err(err) => {
                            log::error!("Trade fetch failed: {err}");
                            FetchUpdate::Error {
                                pane_id,
                                error: err.ui_message(),
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
    handles: AdapterHandles,
    pane_id: Uuid,
    ready_streams: &[StreamKind],
    layout_id: Uuid,
    reqs: impl IntoIterator<Item = (Uuid, FetchRange, Option<StreamKind>)>,
    mut on_trade_handle: impl FnMut(Handle),
) -> Task<FetchUpdate> {
    let mut tasks = Vec::new();

    for (req_id, fetch, stream) in reqs {
        tasks.push(request_fetch(
            handles.clone(),
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
pub fn fetch_trades_paged(
    server: Option<ServerClient>,
    handles: AdapterHandles,
    ticker_info: TickerInfo,
    from_time: UnixMs,
    to_time: UnixMs,
    data_path: PathBuf,
) -> impl Straw<(), Vec<Trade>, AdapterError> {
    sipper(async move |mut progress| {
        let mut cursor = from_time;

        while cursor < to_time {
            let batch = if let Some(ref client) = server {
                client
                    .fetch_trades_arrow(ticker_info, cursor, to_time, super::client::ARROW_LIMIT)
                    .await?
            } else {
                handles
                    .fetch_trades(ticker_info, cursor, Some(data_path.clone()))
                    .await?
            };

            if batch.is_empty() {
                break;
            }

            cursor = batch.last().map_or(cursor, |t| t.time);

            // Server path only: a batch smaller than the requested limit
            // means the range [prev_cursor, to_time] is fully exhausted
            // and there is nothing left to page through.
            let is_exhausted = server.is_some() && batch.len() < super::client::ARROW_LIMIT;

            let () = progress.send(batch).await;

            if is_exhausted {
                break;
            }
        }

        Ok(())
    })
}
