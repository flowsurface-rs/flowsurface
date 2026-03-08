use crate::chart::{Message, ViewState};
use crate::connector::fetcher::FetchRange;

use data::chart::PlotData;
use data::chart::indicator::KlineIndicator;
use data::chart::kline::KlineDataPoint;
use exchange::{Kline, Timeframe, Trade};
use iced::Color;

pub mod delta;
pub mod ofi;
pub mod ofi_cumulative_ema;
pub mod open_interest;
pub mod trade_count;
pub mod trade_intensity;
// GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97
pub mod trade_intensity_heatmap;
pub mod volume;
// GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97
pub mod zigzag;

pub trait KlineIndicatorImpl {
    /// Clear all caches for a full redraw
    fn clear_all_caches(&mut self);

    /// Clear caches related to crosshair only
    /// e.g. tooltips and scale labels for a partial redraw
    fn clear_crosshair_caches(&mut self);

    fn element<'a>(
        &'a self,
        chart: &'a ViewState,
        visible_range: std::ops::RangeInclusive<u64>,
    ) -> iced::Element<'a, Message>;

    /// If the indicator needs data fetching, return the required range
    fn fetch_range(&mut self, _ctx: &FetchCtx) -> Option<FetchRange> {
        None
    }

    /// Rebuild data using kline(OHLCV) source
    fn rebuild_from_source(&mut self, _source: &PlotData<KlineDataPoint>) {}

    fn on_insert_klines(&mut self, _klines: &[Kline]) {}

    fn on_insert_trades(
        &mut self,
        _trades: &[Trade],
        _old_dp_len: usize,
        _source: &PlotData<KlineDataPoint>,
    ) {
    }

    fn on_ticksize_change(&mut self, _source: &PlotData<KlineDataPoint>) {}

    /// Timeframe/tick interval has changed
    fn on_basis_change(&mut self, _source: &PlotData<KlineDataPoint>) {}

    fn on_open_interest(&mut self, _pairs: &[exchange::OpenInterest]) {}

    /// Return a thermal body colour for the candle at `storage_idx` (oldest-first index).
    /// Default: `None` (normal green/red palette colours used).
    /// Only overridden by `TradeIntensityHeatmap` which colours candle bodies, not a subplot.
    fn thermal_body_color(&self, _storage_idx: u64) -> Option<Color> {
        None
    }

    /// Return the number of processed datapoints in this indicator's internal storage.
    /// Used for divergence detection between indicator state and data source.
    fn data_len(&self) -> usize {
        0
    }

    /// Draw overlay graphics on the main candle pane.
    /// Default: no-op. Overridden by overlay indicators (e.g. ZigZag).
    fn draw_overlay(
        &self,
        _frame: &mut iced::widget::canvas::Frame,
        _total_len: usize,
        _earliest_visual: usize,
        _latest_visual: usize,
        _price_to_y: &dyn Fn(exchange::unit::Price) -> f32,
        _interval_to_x: &dyn Fn(u64) -> f32,
        _palette: &iced::theme::palette::Extended,
    ) {
    }
}

pub struct FetchCtx<'a> {
    pub main_chart: &'a ViewState,
    pub timeframe: Timeframe,
    pub visible_earliest: u64,
    pub kline_latest: u64,
    pub prefetch_earliest: u64,
}

pub fn make_empty(which: KlineIndicator) -> Box<dyn KlineIndicatorImpl> {
    match which {
        KlineIndicator::Volume => Box::new(super::kline::volume::VolumeIndicator::new()),
        KlineIndicator::OpenInterest => {
            Box::new(super::kline::open_interest::OpenInterestIndicator::new())
        }
        KlineIndicator::Delta => Box::new(super::kline::delta::DeltaIndicator::new()),
        KlineIndicator::TradeCount => {
            Box::new(super::kline::trade_count::TradeCountIndicator::new())
        }
        KlineIndicator::OFI => Box::new(super::kline::ofi::OFIIndicator::new()),
        // GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97
        KlineIndicator::OFICumulativeEma => {
            Box::new(super::kline::ofi_cumulative_ema::OFICumulativeEmaIndicator::new())
        }
        KlineIndicator::TradeIntensity => {
            Box::new(super::kline::trade_intensity::TradeIntensityIndicator::new())
        }
        // GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97
        KlineIndicator::TradeIntensityHeatmap => {
            Box::new(super::kline::trade_intensity_heatmap::TradeIntensityHeatmapIndicator::new())
        }
        // GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97
        KlineIndicator::ZigZag => Box::new(super::kline::zigzag::ZigZagOverlayIndicator::new()),
    }
}
