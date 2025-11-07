use crate::adapter::StreamKind;
use crate::{Kline, OpenInterest, Trade};

use smallvec::SmallVec;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use uuid::Uuid;

static TRADE_FETCH_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn toggle_trade_fetch(value: bool) {
    TRADE_FETCH_ENABLED.store(value, Ordering::Relaxed);
}

pub fn is_trade_fetch_enabled() -> bool {
    TRADE_FETCH_ENABLED.load(Ordering::Relaxed)
}

#[derive(Debug, Clone)]
pub enum FetchedData {
    Trades {
        batch: Vec<Trade>,
        until_time: u64,
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
    #[error("Request is already completed")]
    Completed,
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

pub struct RequestHandler {
    requests: HashMap<Uuid, FetchRequest>,
}

impl RequestHandler {
    pub fn new() -> Self {
        RequestHandler {
            requests: HashMap::new(),
        }
    }

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

impl Default for RequestHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(PartialEq, Debug, Clone, Copy)]
pub enum FetchRange {
    Kline(u64, u64),
    OpenInterest(u64, u64),
    Trades(u64, u64),
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

pub type FetchRequests = SmallVec<[FetchSpec; 1]>;

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
