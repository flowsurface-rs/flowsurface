pub mod heatmap;
pub mod kline;
pub mod kline_data;
pub mod ticks;

use exchange::UnixMs;
use exchange::{Timeframe, unit::PriceStep};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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

/// Lightweight basis-aware keyed projection of values.
///
/// Unlike `PlotData`, this intentionally stores only x-key and value pairs.
/// Use it for derived/ephemeral series (e.g. indicator caches) where full
/// aggregation metadata is not needed.
#[derive(Debug, Clone)]
pub enum BasisSeries<T> {
    Time(BTreeMap<UnixMs, T>),
    Tick(BTreeMap<u64, T>),
}

impl<T> Default for BasisSeries<T> {
    fn default() -> Self {
        Self::Time(BTreeMap::new())
    }
}

impl<T> BasisSeries<T> {
    pub fn time(data: BTreeMap<UnixMs, T>) -> Self {
        Self::Time(data)
    }

    pub fn tick(data: BTreeMap<u64, T>) -> Self {
        Self::Tick(data)
    }

    pub fn time_mut(&mut self) -> Option<&mut BTreeMap<UnixMs, T>> {
        match self {
            Self::Time(data) => Some(data),
            Self::Tick(_) => None,
        }
    }

    pub fn tick_mut(&mut self) -> Option<&mut BTreeMap<u64, T>> {
        match self {
            Self::Tick(data) => Some(data),
            Self::Time(_) => None,
        }
    }

    pub fn map<U, F>(&self, mut f: F) -> BasisSeries<U>
    where
        F: FnMut(&T) -> U,
    {
        match self {
            Self::Time(data) => BasisSeries::Time(data.iter().map(|(&k, v)| (k, f(v))).collect()),
            Self::Tick(data) => BasisSeries::Tick(data.iter().map(|(&k, v)| (k, f(v))).collect()),
        }
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

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ViewConfig {
    pub splits: Vec<f32>,
    pub autoscale: Option<Autoscale>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq)]
pub enum Autoscale {
    #[default]
    CenterLatest,
    FitToVisible,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum Study {
    Heatmap(Vec<heatmap::HeatmapStudy>),
}
