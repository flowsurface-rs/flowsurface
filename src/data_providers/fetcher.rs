use uuid::Uuid;
use std::collections::HashMap;

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

    pub fn add_request(&mut self, fetch: FetchRange) -> Result<Uuid, ReqError> {
        let request = FetchRequest::new(fetch);
        let id = Uuid::new_v4();

        if let Some(r) = self.requests.values().find(|r| r.ends_same_with(&request)) {
            return match &r.status {
                RequestStatus::Failed(error_msg) => Err(ReqError::Failed(error_msg.clone())),
                RequestStatus::Completed(_) => Err(ReqError::Completed),
                RequestStatus::Pending => Err(ReqError::Overlaps),
            };
        }

        self.requests.insert(id, request);

        Ok(id)
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
    Kline(i64, i64),
    OpenInterest(i64, i64),
    Trades(i64, i64),
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

    fn ends_same_with(&self, other: &FetchRequest) -> bool {
        match (&self.fetch_type, &other.fetch_type) {
            (FetchRange::Kline(_, e1), FetchRange::Kline(_, e2)) => e1 == e2,
            _ => false,
        }
    }
}
