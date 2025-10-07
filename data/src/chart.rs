pub mod heatmap;
pub mod indicator;
pub mod kline;

use exchange::Timeframe;
use serde::{Deserialize, Serialize};

use super::aggr::{
    self,
    ticks::TickAggr,
    time::{DataPoint, TimeSeries},
};
pub use kline::KlineChartKind;

pub enum PlotData<D: DataPoint> {
    TimeBased(TimeSeries<D>),
    TickBased(TickAggr),
}

impl<D: DataPoint> PlotData<D> {
    pub fn latest_y_midpoint(&self, calculate_target_y: impl Fn(exchange::Kline) -> f32) -> f32 {
        match self {
            PlotData::TimeBased(timeseries) => timeseries
                .latest_kline()
                .map_or(0.0, |kline| calculate_target_y(*kline)),
            PlotData::TickBased(tick_aggr) => tick_aggr
                .latest_dp()
                .map_or(0.0, |(dp, _)| calculate_target_y(dp.kline)),
        }
    }

    pub fn visible_price_range(
        &self,
        start_interval: u64,
        end_interval: u64,
    ) -> Option<(f32, f32)> {
        match self {
            PlotData::TimeBased(timeseries) => {
                timeseries.min_max_price_in_range(start_interval, end_interval)
            }
            PlotData::TickBased(tick_aggr) => {
                tick_aggr.min_max_price_in_range(start_interval as usize, end_interval as usize)
            }
        }
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

/// Defines how chart data is aggregated and displayed along the x-axis.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum Basis {
    /// Time-based aggregation where each datapoint represents a fixed time interval.
    Time(exchange::Timeframe),

    /// Trade-based aggregation where each datapoint represents a fixed number of trades.
    ///
    /// The u16 value represents the number of trades per aggregation unit.
    Tick(aggr::TickCount),
}

impl Basis {
    pub fn is_time(&self) -> bool {
        matches!(self, Basis::Time(_))
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum Study {
    Heatmap(Vec<heatmap::HeatmapStudy>),
    Footprint(Vec<kline::FootprintStudy>),
}
