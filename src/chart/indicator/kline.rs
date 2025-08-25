use crate::chart::{Message, ViewState};

use data::chart::PlotData;
use data::chart::indicator::KlineIndicator;
use data::chart::kline::KlineDataPoint;
use exchange::fetcher::FetchRange;
use exchange::{Kline, Trade};

pub mod open_interest;
pub mod volume;

pub struct FetchCtx<'a> {
    pub chart: &'a ViewState,
    pub timeframe_ms: u64,
    pub visible_earliest: u64,
    pub kline_latest: u64,
    pub prefetch_earliest: u64,
}

pub trait KlineIndicatorImpl {
    fn clear_all_caches(&mut self);

    fn clear_crosshair_caches(&mut self);

    fn element<'a>(
        &'a self,
        chart: &'a ViewState,
        visible_range: std::ops::RangeInclusive<u64>,
    ) -> iced::Element<'a, Message>;

    fn fetch_range(&mut self, _ctx: &FetchCtx) -> Option<FetchRange> {
        None
    }

    fn rebuild_from_source(&mut self, _source: &PlotData<KlineDataPoint>) {}

    fn on_new_klines(&mut self, _klines: &[Kline]) {}

    fn on_insert_trades(&mut self, _trades: &[Trade], _source: &PlotData<KlineDataPoint>) {}

    fn on_change_tick_size(&mut self, _source: &PlotData<KlineDataPoint>) {}

    fn on_basis_changed(&mut self, _source: &PlotData<KlineDataPoint>) {}

    fn on_open_interest(&mut self, _pairs: &[exchange::OpenInterest]) {}
}

pub fn make_empty(which: KlineIndicator) -> Box<dyn KlineIndicatorImpl> {
    match which {
        KlineIndicator::Volume => Box::new(super::kline::volume::VolumeIndicator::new()),
        KlineIndicator::OpenInterest => {
            Box::new(super::kline::open_interest::OpenInterestIndicator::new())
        }
    }
}
