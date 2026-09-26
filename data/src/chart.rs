pub mod heatmap;
pub mod kline;
pub mod ticks;

use exchange::{Timeframe, unit::PriceStep};
use serde::{Deserialize, Serialize};

use super::aggr::{
    self,
    ticks::TickAggr,
    time::{DataPoint, TimeSeries},
};

/// Source-of-truth chart data with aggregation metadata.
///
/// `PlotData` owns full aggregation state (`TimeSeries`/`TickAggr`) so callers
/// can retain the source context needed for basis-specific calculations.
pub enum PlotData<D: DataPoint> {
    TimeBased(TimeSeries<D>),
    TickBased(TickAggr),
}

impl<D: DataPoint> PlotData<D> {
    /// Returns the aggregation basis represented by this source.
    pub fn basis(&self) -> Basis {
        match self {
            Self::TimeBased(series) => Basis::Time(series.interval),
            Self::TickBased(series) => Basis::Tick(series.interval),
        }
    }

    /// Returns the footprint price step used by this source.
    pub fn tick_size(&self) -> PriceStep {
        match self {
            Self::TimeBased(series) => series.tick_size,
            Self::TickBased(series) => series.tick_size,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::TimeBased(series) => series.datapoints.len(),
            Self::TickBased(series) => series.datapoints.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Defines how chart data is aggregated and displayed along the x-axis.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum Basis {
    /// Time-based aggregation where each datapoint represents a fixed time interval.
    Time(exchange::Timeframe),

    /// Trade-based aggregation where each datapoint represents a fixed number of trades.
    Tick(aggr::TickCount),
}

impl Basis {
    pub fn is_time(&self) -> bool {
        matches!(self, Basis::Time(_))
    }

    pub fn default_kline_time(
        ticker_info: Option<exchange::TickerInfo>,
        fallback: Timeframe,
    ) -> Self {
        let interval = ticker_info.map_or(fallback, |info| {
            let exchange = info.exchange();

            if exchange.supports_kline_timeframe(fallback) {
                fallback
            } else {
                Timeframe::KLINE
                    .iter()
                    .copied()
                    .find(|tf| exchange.supports_kline_timeframe(*tf))
                    .unwrap_or(fallback)
            }
        });

        interval.into()
    }

    pub fn default_heatmap_time(ticker_info: Option<exchange::TickerInfo>) -> Self {
        let fallback = Timeframe::MS500;

        let interval = ticker_info.map_or(fallback, |info| {
            let ex = info.exchange();
            Timeframe::HEATMAP
                .iter()
                .copied()
                .find(|tf| ex.supports_heatmap_timeframe(*tf))
                .unwrap_or(fallback)
        });

        interval.into()
    }
}

impl std::fmt::Display for Basis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Basis::Time(timeframe) => write!(f, "{timeframe}"),
            Basis::Tick(count) => write!(f, "{count}"),
        }
    }
}

impl From<exchange::Timeframe> for Basis {
    fn from(timeframe: exchange::Timeframe) -> Self {
        Self::Time(timeframe)
    }
}
