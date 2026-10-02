use super::indicators::{AvailabilityContext, IndicatorAvailability, availability};
use super::series::KlineSeries;
use super::{IndicatorInstanceId, IndicatorInstanceStore};
use crate::chart::Basis;
use exchange::adapter::Exchange;
use exchange::{TickerInfo, Timeframe, UnixMs};

const DEFAULT_FETCH_BARS: u64 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchRange {
    Kline(UnixMs, UnixMs),
    ProfileKline {
        timeframe: Timeframe,
        from: UnixMs,
        to: UnixMs,
    },
    OpenInterest {
        from: UnixMs,
        to: UnixMs,
    },
    Trades(UnixMs, UnixMs),
}

impl FetchRange {
    pub const fn open_interest(from: UnixMs, to: UnixMs) -> Self {
        Self::OpenInterest { from, to }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
/// Classifies a fetch for diagnostics and planning; it does not impose order.
pub enum FetchStage {
    PrimaryPlot,
    IndicatorHistory,
    OpenInterestInputs,
    Trades,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchBatch {
    pub stage: FetchStage,
    pub range: FetchRange,
    pub tickers: Vec<TickerInfo>,
}

#[derive(Clone, Copy)]
pub struct FetchPlanInput<'a> {
    pub basis: Basis,
    pub horizontal_offset: f32,
    pub base_ticker: TickerInfo,
    pub series: &'a [KlineSeries],
    pub active_indicators: &'a [IndicatorInstanceId],
    pub indicator_instances: &'a IndicatorInstanceStore,
    pub open_interest_required: bool,
    pub trade_data_required: bool,
    pub trade_fetch_enabled: bool,
    pub visible_points: u64,
    pub required_history_start: Option<UnixMs>,
    pub required_history_bars: u64,
    pub now: UnixMs,
}

impl FetchPlanInput<'_> {
    pub fn desired_fetch_batches(self) -> Vec<FetchBatch> {
        let Basis::Time(timeframe) = self.basis else {
            return Vec::new();
        };

        let dt = timeframe.to_milliseconds().max(1);
        let visible_points = self.visible_points.max(2);

        let indicator_warmup_bars = self
            .indicator_instances
            .dependency_warmup_bars(self.active_indicators);

        let required_history_bars = indicator_warmup_bars.max(self.required_history_bars);
        let base_fetch_bars = DEFAULT_FETCH_BARS.max(visible_points + required_history_bars);
        let comparison_fetch_bars = DEFAULT_FETCH_BARS.max(visible_points);
        let base_span = base_fetch_bars.saturating_mul(dt);
        let comparison_span = comparison_fetch_bars.saturating_mul(dt);
        let initial_end = self.now;

        let mut batches = Vec::new();
        let mut empty_base = false;
        let mut empty_comparisons = Vec::new();

        for series in self.series {
            if series.is_empty() {
                if series.ticker_info == self.base_ticker {
                    empty_base = true;
                } else {
                    empty_comparisons.push(series.ticker_info);
                }
            }
        }

        if empty_base {
            let end = initial_end;
            let start = end.saturating_sub(comparison_span);
            batches.push(FetchBatch {
                stage: FetchStage::PrimaryPlot,
                range: FetchRange::Kline(start, end),
                tickers: vec![self.base_ticker],
            });
        }

        if !empty_comparisons.is_empty() {
            let end = initial_end;
            batches.push(FetchBatch {
                stage: FetchStage::PrimaryPlot,
                range: FetchRange::Kline(end.saturating_sub(comparison_span), end),
                tickers: empty_comparisons,
            });
        }

        if let Some((window_min, window_max)) = visible_window(
            self.series,
            self.horizontal_offset,
            timeframe,
            self.visible_points,
        ) {
            let indicator_warmup_start = if indicator_warmup_bars > 0 {
                window_min
                    .saturating_sub(indicator_warmup_bars.saturating_mul(dt))
                    .floor_to(timeframe)
            } else {
                window_min
            };
            let warmup_start = self
                .required_history_start
                .map(|start| start.min(indicator_warmup_start))
                .unwrap_or(indicator_warmup_start);

            let mut base_boundary_fetch_pending = false;
            for series in self.series {
                let (target_min, fetch_span, stage) = if series.ticker_info == self.base_ticker {
                    if window_min
                        < series
                            .first_point()
                            .map(|point| point.kline.time)
                            .unwrap_or(window_min)
                    {
                        (window_min, comparison_span, FetchStage::PrimaryPlot)
                    } else {
                        (warmup_start, base_span, FetchStage::IndicatorHistory)
                    }
                } else {
                    (window_min, comparison_span, FetchStage::PrimaryPlot)
                };

                if let Some(series_min) = series.first_point().map(|point| point.kline.time)
                    && target_min < series_min
                {
                    let end = series_min.floor_to(timeframe);
                    batches.push(FetchBatch {
                        stage,
                        range: FetchRange::Kline(end.saturating_sub(fetch_span), end),
                        tickers: vec![series.ticker_info],
                    });
                    if stage == FetchStage::IndicatorHistory
                        && series.ticker_info == self.base_ticker
                    {
                        base_boundary_fetch_pending = true;
                    }
                }
            }

            if !base_boundary_fetch_pending
                && let Some(base_series) = self
                    .series
                    .iter()
                    .find(|series| series.ticker_info == self.base_ticker)
                && let Some(series_earliest) =
                    base_series.first_point().map(|point| point.kline.time)
                && let Some(series_latest) =
                    base_series.latest_point().map(|point| point.kline.time)
            {
                let visible_span = window_max.as_u64().saturating_sub(window_min.as_u64());
                let prefetch_earliest = window_min.saturating_sub(visible_span);
                let check_earliest = warmup_start.min(prefetch_earliest).max(series_earliest);
                let check_latest = window_max.saturating_add(dt).min(series_latest);

                if let Some((missing_earliest, missing_latest)) =
                    base_series.missing_kline_range(check_earliest, check_latest, timeframe)
                {
                    let stage = if missing_earliest < window_max.saturating_add(dt)
                        && missing_latest > window_min
                    {
                        FetchStage::PrimaryPlot
                    } else {
                        FetchStage::IndicatorHistory
                    };
                    batches.push(FetchBatch {
                        stage,
                        range: FetchRange::Kline(
                            missing_earliest.saturating_sub(dt),
                            missing_latest.saturating_add(dt),
                        ),
                        tickers: vec![self.base_ticker],
                    });
                }
            }

            if self.open_interest_required {
                let has_available_open_interest_consumer = self
                    .active_indicators
                    .iter()
                    .copied()
                    .filter_map(|id| self.indicator_instances.get(id))
                    .filter(|instance| {
                        super::indicators::indicator_definition(&instance.kind)
                            .uses(super::indicators::IndicatorInputSource::OpenInterest)
                    })
                    .any(|instance| {
                        matches!(
                            self.indicator_availability(instance.id),
                            IndicatorAvailability::Available
                        )
                    });
                if !has_available_open_interest_consumer {
                    return batches;
                }
                let Some(base_series) = self
                    .series
                    .iter()
                    .find(|series| series.ticker_info == self.base_ticker)
                else {
                    return batches;
                };
                let Some(kline_latest) = base_series.latest_point().map(|point| point.kline.time)
                else {
                    return batches;
                };
                let visible_window = visible_window(
                    self.series,
                    self.horizontal_offset,
                    timeframe,
                    self.visible_points,
                );
                let visible_earliest = visible_window.map(|(start, _)| start).unwrap_or(window_min);
                let visible_span = visible_window
                    .map(|(start, end)| end.as_u64().saturating_sub(start.as_u64()))
                    .unwrap_or(base_span);
                let prefetch_earliest = visible_earliest.saturating_sub(visible_span);

                match base_series.open_interest_range() {
                    Some((oi_earliest, oi_latest)) => {
                        if visible_earliest < oi_earliest {
                            batches.push(FetchBatch {
                                stage: FetchStage::OpenInterestInputs,
                                range: FetchRange::open_interest(prefetch_earliest, oi_earliest),
                                tickers: vec![self.base_ticker],
                            });
                        }

                        if oi_latest < kline_latest {
                            let start = oi_latest.max(prefetch_earliest);
                            if start < kline_latest {
                                batches.push(FetchBatch {
                                    stage: FetchStage::OpenInterestInputs,
                                    range: FetchRange::open_interest(start, kline_latest),
                                    tickers: vec![self.base_ticker],
                                });
                            }
                        }
                    }
                    None => {
                        if prefetch_earliest < kline_latest {
                            batches.push(FetchBatch {
                                stage: FetchStage::OpenInterestInputs,
                                range: FetchRange::open_interest(prefetch_earliest, kline_latest),
                                tickers: vec![self.base_ticker],
                            });
                        }
                    }
                }
            }
        }

        batches
    }

    pub fn desired_trade_fetch_range(self, target_ticker: TickerInfo) -> Option<FetchRange> {
        if target_ticker != self.base_ticker {
            return None;
        }
        let Basis::Time(timeframe) = self.basis else {
            return None;
        };

        if !self.trade_data_required || !self.trade_fetch_enabled {
            return None;
        }

        let target_series = self
            .series
            .iter()
            .find(|series| series.ticker_info == target_ticker)?;
        if target_series.is_empty() {
            return None;
        }

        let dt = timeframe.to_milliseconds().max(1);
        let visible_points = self.visible_points.max(2);
        let (visible_earliest, visible_latest) = visible_window(
            std::slice::from_ref(target_series),
            self.horizontal_offset,
            timeframe,
            visible_points,
        )
        .map(|(earliest, latest)| {
            let half_interval = dt / 2;
            let half_visible_span = latest.saturating_diff(earliest) / 2;
            (
                earliest.saturating_sub(half_visible_span.saturating_add(half_interval)),
                latest.saturating_add(half_interval),
            )
        })?;

        if visible_earliest >= visible_latest {
            return None;
        }

        if let Some((earliest, latest)) =
            target_series.suggest_trade_fetch_range(visible_earliest, visible_latest)
        {
            return Some(FetchRange::Trades(earliest, latest));
        }

        None
    }

    fn indicator_availability(&self, id: IndicatorInstanceId) -> IndicatorAvailability {
        let Some(instance) = self.indicator_instances.get(id) else {
            return IndicatorAvailability::Unsupported(
                super::indicators::IndicatorUnsupportedReason::NotInstalled,
            );
        };
        if !instance.kind.is_registered() {
            return IndicatorAvailability::Unsupported(
                super::indicators::IndicatorUnsupportedReason::NotInstalled,
            );
        }
        availability(
            &instance.kind,
            AvailabilityContext {
                basis: self.basis,
                base_ticker: self.base_ticker,
            },
        )
    }
}

pub fn visible_window(
    series: &[KlineSeries],
    horizontal_offset: f32,
    timeframe: Timeframe,
    visible_points: u64,
) -> Option<(UnixMs, UnixMs)> {
    let max_seen = series
        .iter()
        .filter_map(|series| series.latest_point().map(|point| point.kline.time))
        .max()?;

    let dt = timeframe.to_milliseconds().max(1);
    let span_units = visible_points.saturating_sub(1);
    let right_offset = (f64::from(horizontal_offset) * dt as f64)
        .round()
        .clamp(i64::MIN as f64, i64::MAX as f64) as i64;
    let right = super::ChartCoord::from_unix_ms(max_seen).saturating_add_i64(right_offset);
    let span_offset = span_units.saturating_mul(dt);
    let left = right.saturating_sub_i64(i64::try_from(span_offset).unwrap_or(i64::MAX));

    Some((
        left.to_unix_ms_non_negative(),
        right.to_unix_ms_non_negative(),
    ))
}

/// Provider capability predicates shared by fetch planning and indicators.
pub fn supports_open_interest(ticker_info: TickerInfo, timeframe: Timeframe) -> bool {
    supports_open_interest_exchange(ticker_info.exchange())
        && supports_open_interest_timeframe(timeframe)
}

pub fn supports_open_interest_exchange(exchange: Exchange) -> bool {
    matches!(
        exchange,
        Exchange::BinanceLinear
            | Exchange::BinanceInverse
            | Exchange::BybitLinear
            | Exchange::BybitInverse
            | Exchange::OkexLinear
            | Exchange::OkexInverse
    )
}

pub fn supports_open_interest_timeframe(timeframe: Timeframe) -> bool {
    timeframe >= Timeframe::M5 && timeframe <= Timeframe::H4 && timeframe != Timeframe::H2
}
