use crate::connector::fetcher::{FetchSpec, RequestHandler};
use data::chart::Basis;
use data::chart::kline::FetchRange;
use exchange::{TickerInfo, adapter::StreamKind};
use iced::task::Handle;
use rustc_hash::FxHashMap;

pub enum FetchUpdate {
    Failed(uuid::Uuid),
    TradeFetchNoData {
        ticker_info: TickerInfo,
        req_id: uuid::Uuid,
    },
    FetchHandle {
        req_id: uuid::Uuid,
        handle: Handle,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FetchActivity {
    Kline,
    Trades { fetched: usize },
    OpenInterest,
}

pub struct FetchCoordinator {
    request_handlers: FxHashMap<TickerInfo, RequestHandler>,
    fetch_handles: FxHashMap<uuid::Uuid, Handle>,
}

impl FetchCoordinator {
    pub fn new(tickers: &[TickerInfo]) -> Self {
        let mut coordinator = Self {
            request_handlers: FxHashMap::default(),
            fetch_handles: FxHashMap::default(),
        };
        coordinator.rebuild_handlers(tickers);
        coordinator
    }

    pub fn reset(&mut self) {
        self.clear_fetch_state();
        self.request_handlers.clear();
    }

    pub fn accepts_data(&self, req_id: uuid::Uuid) -> bool {
        self.request_handlers
            .values()
            .any(|handler| handler.is_pending(req_id))
    }

    pub fn is_profile_request(&self, req_id: uuid::Uuid) -> bool {
        self.request_handlers
            .values()
            .any(|handler| handler.is_profile_request(req_id))
    }

    pub fn set_handle(&mut self, req_id: uuid::Uuid, handle: Handle) -> bool {
        if self.accepts_data(req_id) {
            self.fetch_handles.insert(req_id, handle);
            true
        } else {
            handle.abort();
            false
        }
    }

    pub fn mark_failed(&mut self, req_id: uuid::Uuid) -> bool {
        if !self.accepts_data(req_id) {
            log::debug!("Ignoring stale fetch error: {:?}", req_id);
            return false;
        }

        let is_profile = self.is_profile_request(req_id);
        if self
            .request_handlers
            .values_mut()
            .any(|handler| handler.mark_failed(req_id))
        {
            self.fetch_handles.remove(&req_id);
            if is_profile {
                self.clear_pending_profile_requests();
            }
            return true;
        }

        log::debug!("Ignoring stale kline request: {:?}", req_id);
        true
    }

    pub fn mark_completed(&mut self, ticker_info: TickerInfo, req_id: uuid::Uuid) {
        self.finish_request(ticker_info, req_id, |handler| {
            handler.mark_completed(req_id)
        });
    }

    pub fn mark_profile_completed(
        &mut self,
        ticker_info: TickerInfo,
        req_id: uuid::Uuid,
    ) -> Option<FetchRange> {
        let handler = self.request_handlers.get_mut(&ticker_info)?;
        if !handler.is_pending(req_id) {
            return None;
        }
        let fetch = handler.mark_profile_completed(req_id)?;
        self.fetch_handles.remove(&req_id);
        Some(fetch)
    }

    pub fn mark_failed_for(&mut self, ticker_info: TickerInfo, req_id: uuid::Uuid) {
        self.finish_request(ticker_info, req_id, |handler| handler.mark_failed(req_id));
    }

    pub fn mark_no_data(&mut self, ticker_info: TickerInfo, req_id: uuid::Uuid) -> bool {
        self.finish_request(ticker_info, req_id, |handler| handler.mark_no_data(req_id))
    }

    pub fn complete_trade_fetch(
        &mut self,
        ticker_info: TickerInfo,
        req_id: Option<uuid::Uuid>,
        is_batches_done: bool,
    ) {
        if !is_batches_done {
            return;
        }

        if let Some(req_id) = req_id {
            self.mark_completed(ticker_info, req_id);
        }
    }

    fn finish_request(
        &mut self,
        ticker_info: TickerInfo,
        req_id: uuid::Uuid,
        finish: impl FnOnce(&mut RequestHandler) -> bool,
    ) -> bool {
        let Some(handler) = self.request_handlers.get_mut(&ticker_info) else {
            return false;
        };
        if !handler.is_pending(req_id) || !finish(handler) {
            return false;
        }

        self.fetch_handles.remove(&req_id);
        true
    }

    pub fn add_ticker(&mut self, ticker_info: TickerInfo) {
        self.request_handlers.entry(ticker_info).or_default();
    }

    pub fn remove_ticker(&mut self, ticker_info: TickerInfo) {
        if let Some(handler) = self.request_handlers.remove(&ticker_info) {
            for req_id in handler.request_ids() {
                self.fetch_handles.remove(&req_id);
            }
        }
    }

    pub fn rebuild_handlers(&mut self, tickers: &[TickerInfo]) {
        self.reset();
        for &ticker in tickers {
            self.add_ticker(ticker);
        }
    }

    pub fn reset_for_basis_change(&mut self) {
        self.clear_fetch_state();
        for handler in self.request_handlers.values_mut() {
            handler.reset_for_basis_change();
        }
    }

    /// Open Interest is a disposable input cache. Once its last consumer is
    /// removed, completed request markers must be discarded as well so a
    /// later consumer can reacquire the same viewport range.
    pub fn clear_open_interest_requests(&mut self) {
        let request_ids = self
            .request_handlers
            .values_mut()
            .flat_map(RequestHandler::clear_open_interest_requests)
            .collect::<Vec<_>>();
        for req_id in request_ids {
            if let Some(handle) = self.fetch_handles.remove(&req_id) {
                handle.abort();
            }
        }
    }

    pub fn clear_pending_profile_requests(&mut self) {
        let request_ids = self
            .request_handlers
            .values_mut()
            .flat_map(RequestHandler::clear_pending_profile_requests)
            .collect::<Vec<_>>();
        for req_id in request_ids {
            if let Some(handle) = self.fetch_handles.remove(&req_id) {
                handle.abort();
            }
        }
    }

    pub fn clear_pending_trade_requests(&mut self) {
        let request_ids = self
            .request_handlers
            .values_mut()
            .flat_map(RequestHandler::clear_pending_trade_requests)
            .collect::<Vec<_>>();
        for req_id in request_ids {
            if let Some(handle) = self.fetch_handles.remove(&req_id) {
                handle.abort();
            }
        }
    }

    pub fn pending_requests(&self) -> impl Iterator<Item = (TickerInfo, FetchRange)> + '_ {
        self.request_handlers
            .iter()
            .flat_map(|(&ticker_info, handler)| {
                handler
                    .pending_ranges()
                    .map(move |range| (ticker_info, range))
            })
    }

    pub(super) fn pending_activity(&self) -> Vec<FetchActivity> {
        let mut has_klines = false;
        let mut trade_count: Option<usize> = None;
        let mut has_open_interest = false;

        for handler in self.request_handlers.values() {
            for range in handler.pending_ranges() {
                match range {
                    FetchRange::Kline(..) | FetchRange::ProfileKline { .. } => has_klines = true,
                    FetchRange::Trades(..) => {}
                    FetchRange::OpenInterest { .. } => has_open_interest = true,
                }
            }

            if let Some(count) = handler.pending_trade_count() {
                trade_count = Some(trade_count.unwrap_or_default().saturating_add(count));
            }
        }

        let mut activity = Vec::with_capacity(3);
        if has_klines {
            activity.push(FetchActivity::Kline);
        }
        if let Some(fetched) = trade_count {
            activity.push(FetchActivity::Trades { fetched });
        }
        if has_open_interest {
            activity.push(FetchActivity::OpenInterest);
        }
        activity
    }

    pub(super) fn record_trade_batch(&mut self, req_id: uuid::Uuid, count: usize) {
        for handler in self.request_handlers.values_mut() {
            if handler.record_trade_batch(req_id, count) {
                return;
            }
        }
    }

    pub fn collect_requests(
        &mut self,
        batches: impl IntoIterator<Item = (FetchRange, Vec<TickerInfo>)>,
        basis: Basis,
    ) -> Vec<FetchSpec> {
        let mut requests = Vec::new();

        for (range, tickers) in batches {
            match range {
                FetchRange::Trades(..) => {
                    for ticker_info in tickers {
                        self.queue_trade_fetch(ticker_info, range, &mut requests);
                    }
                }
                FetchRange::Kline(..)
                | FetchRange::ProfileKline { .. }
                | FetchRange::OpenInterest { .. } => {
                    for ticker_info in tickers {
                        self.queue_kline_fetch(ticker_info, range, basis, &mut requests);
                    }
                }
            }
        }

        requests
    }

    fn queue_kline_fetch(
        &mut self,
        ticker_info: TickerInfo,
        range: FetchRange,
        basis: Basis,
        out: &mut Vec<FetchSpec>,
    ) {
        let timeframe = match range {
            FetchRange::ProfileKline { timeframe, .. } => timeframe,
            FetchRange::Kline(..) | FetchRange::OpenInterest { .. } => {
                let Basis::Time(timeframe) = basis else {
                    return;
                };
                timeframe
            }
            FetchRange::Trades(..) => return,
        };

        let handler = self.request_handlers.entry(ticker_info).or_default();
        if matches!(range, FetchRange::ProfileKline { .. }) && handler.has_pending_profile_fetch() {
            return;
        }

        if let Ok(Some(req_id)) = handler.add_request(range) {
            out.push(FetchSpec::from((
                req_id,
                range,
                Some(StreamKind::Kline {
                    ticker_info,
                    timeframe,
                }),
            )));
        }
    }

    fn queue_trade_fetch(
        &mut self,
        ticker_info: TickerInfo,
        range: FetchRange,
        out: &mut Vec<FetchSpec>,
    ) {
        let handler = self.request_handlers.entry(ticker_info).or_default();

        if handler.has_pending_trade_fetch() {
            return;
        }

        if let Ok(Some(req_id)) = handler.add_request(range) {
            out.push(FetchSpec::from((
                req_id,
                range,
                Some(StreamKind::Trades { ticker_info }),
            )));
        }
    }

    fn clear_fetch_state(&mut self) {
        for handle in self.fetch_handles.values() {
            handle.abort();
        }
        self.fetch_handles.clear();
    }
}
