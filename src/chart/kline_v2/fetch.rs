use crate::connector::fetcher::{FetchSpec, RequestHandler};
use data::chart::Basis;
use data::chart::kline_v2::FetchRange;
use exchange::{TickerInfo, adapter::StreamKind};
use iced::task::Handle;
use rustc_hash::FxHashMap;

pub enum FetchUpdate {
    Failed(uuid::Uuid),
    TradeFetchNoData(uuid::Uuid),
    TradeFetchHandle(Handle),
}

pub struct FetchCoordinator {
    request_handlers: FxHashMap<TickerInfo, RequestHandler>,
    trade_fetch_handle: Option<Handle>,
    trade_fetch_in_flight: bool,
}

impl FetchCoordinator {
    pub fn new(tickers: &[TickerInfo]) -> Self {
        let mut coordinator = Self {
            request_handlers: FxHashMap::default(),
            trade_fetch_handle: None,
            trade_fetch_in_flight: false,
        };
        coordinator.rebuild_handlers(tickers);
        coordinator
    }

    pub fn reset(&mut self) {
        self.trade_fetch_handle = None;
        self.trade_fetch_in_flight = false;
    }

    pub fn set_handle(&mut self, handle: Handle) {
        self.trade_fetch_handle = Some(handle);
    }

    pub fn mark_trade_fetch_no_data(&mut self, base_ticker: TickerInfo, req_id: uuid::Uuid) {
        if let Some(handler) = self.request_handlers.get_mut(&base_ticker) {
            handler.mark_no_data(req_id);
        }
        self.reset();
    }

    pub fn mark_failed(&mut self, req_id: uuid::Uuid) {
        let is_trade_fetch = self
            .request_handlers
            .values()
            .find_map(|handler| handler.request_range(req_id))
            .is_some_and(|range| matches!(range, FetchRange::Trades(..)));

        if self
            .request_handlers
            .values_mut()
            .any(|handler| handler.mark_failed(req_id))
        {
            if is_trade_fetch {
                self.reset();
            }
            return;
        }

        log::warn!("KlineV2 request not found: {:?}", req_id);
    }

    pub fn mark_completed(&mut self, ticker_info: TickerInfo, req_id: uuid::Uuid) {
        if let Some(handler) = self.request_handlers.get_mut(&ticker_info) {
            handler.mark_completed(req_id);
        }
    }

    pub fn mark_failed_for(&mut self, ticker_info: TickerInfo, req_id: uuid::Uuid) {
        if let Some(handler) = self.request_handlers.get_mut(&ticker_info) {
            handler.mark_failed(req_id);
        }
    }

    pub fn complete_trade_fetch(
        &mut self,
        base_ticker: TickerInfo,
        req_id: Option<uuid::Uuid>,
        is_batches_done: bool,
    ) {
        if !is_batches_done {
            return;
        }

        if let Some(req_id) = req_id
            && let Some(handler) = self.request_handlers.get_mut(&base_ticker)
        {
            handler.mark_completed(req_id);
            self.reset();
        }
    }

    pub fn add_ticker(&mut self, ticker_info: TickerInfo) {
        self.request_handlers.entry(ticker_info).or_default();
    }

    pub fn remove_ticker(&mut self, ticker_info: TickerInfo) {
        self.request_handlers.remove(&ticker_info);
    }

    pub fn rebuild_handlers(&mut self, tickers: &[TickerInfo]) {
        self.request_handlers.clear();
        for &ticker in tickers {
            self.add_ticker(ticker);
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
                FetchRange::Kline(..) | FetchRange::OpenInterest(..) => {
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
        let Basis::Time(timeframe) = basis else {
            return;
        };

        let handler = self.request_handlers.entry(ticker_info).or_default();

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
        base_ticker: TickerInfo,
        range: FetchRange,
        out: &mut Vec<FetchSpec>,
    ) {
        if self.trade_fetch_in_flight {
            return;
        }

        let handler = self.request_handlers.entry(base_ticker).or_default();

        if let Ok(Some(req_id)) = handler.add_request(range) {
            self.trade_fetch_in_flight = true;
            out.push(FetchSpec::from((
                req_id,
                range,
                Some(StreamKind::Trades {
                    ticker_info: base_ticker,
                }),
            )));
        }
    }
}
