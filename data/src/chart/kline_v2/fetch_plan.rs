use super::indicators::{
    AvailabilityContext, IndicatorAvailability, IndicatorInputSource, availability,
    input_requirements,
};
use super::series::KlineSeries;
use super::{IndicatorInstanceId, IndicatorInstanceStore};
use crate::chart::Basis;
use exchange::{TickerInfo, Timeframe, UnixMs};

const DEFAULT_FETCH_BARS: u64 = 500;
const MAX_AUTO_INDICATOR_WARMUP_BARS: u64 = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchRange {
    Kline(UnixMs, UnixMs),
    OpenInterest(UnixMs, UnixMs),
    Trades(UnixMs, UnixMs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FetchStage {
    PrimaryPlot,
    IndicatorHistory,
    ExternalInputs,
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
    pub open_interest_enabled: bool,
    pub trade_data_required: bool,
    pub trade_fetch_enabled: bool,
    pub visible_points: u64,
    pub required_history_start: Option<UnixMs>,
    pub required_history_bars: u64,
    pub now: UnixMs,
}

pub fn desired_fetch_batches(input: FetchPlanInput<'_>) -> Vec<FetchBatch> {
    let Basis::Time(timeframe) = input.basis else {
        return Vec::new();
    };

    let dt = timeframe.to_milliseconds().max(1);
    let visible_points = input.visible_points.max(2);
    let indicator_warmup_bars =
        active_indicator_warmup_bars(input.active_indicators, input.indicator_instances);
    let required_history_bars = indicator_warmup_bars.max(input.required_history_bars);
    let base_fetch_bars = DEFAULT_FETCH_BARS.max(visible_points + required_history_bars);
    let comparison_fetch_bars = DEFAULT_FETCH_BARS.max(visible_points);
    let base_span = base_fetch_bars.saturating_mul(dt);
    let comparison_span = comparison_fetch_bars.saturating_mul(dt);
    let last_closed = align_floor(input.now, timeframe);

    let mut batches = Vec::new();
    let mut empty_base = false;
    let mut empty_comparisons = Vec::new();

    for series in input.series {
        if series.bars.is_empty() {
            if series.ticker_info == input.base_ticker {
                empty_base = true;
            } else {
                empty_comparisons.push(series.ticker_info);
            }
        }
    }

    if empty_base {
        let end = last_closed;
        let start = end.saturating_sub(comparison_span);
        batches.push(FetchBatch {
            stage: FetchStage::PrimaryPlot,
            range: FetchRange::Kline(start, end),
            tickers: vec![input.base_ticker],
        });
    }

    if !empty_comparisons.is_empty() {
        let end = last_closed;
        batches.push(FetchBatch {
            stage: FetchStage::PrimaryPlot,
            range: FetchRange::Kline(end.saturating_sub(comparison_span), end),
            tickers: empty_comparisons,
        });
    }

    if let Some((window_min, window_max)) = visible_window(
        input.series,
        input.horizontal_offset,
        timeframe,
        input.visible_points,
    ) {
        let indicator_warmup_start = if indicator_warmup_bars > 0 {
            align_floor(
                window_min.saturating_sub(indicator_warmup_bars.saturating_mul(dt)),
                timeframe,
            )
        } else {
            window_min
        };
        let warmup_start = input
            .required_history_start
            .map(|start| start.min(indicator_warmup_start))
            .unwrap_or(indicator_warmup_start);

        let mut base_boundary_fetch_pending = false;
        for series in input.series {
            let (target_min, fetch_span, stage) = if series.ticker_info == input.base_ticker {
                if window_min
                    < series
                        .bars
                        .first()
                        .map(|bar| bar.time)
                        .unwrap_or(window_min)
                {
                    (window_min, comparison_span, FetchStage::PrimaryPlot)
                } else {
                    (warmup_start, base_span, FetchStage::IndicatorHistory)
                }
            } else {
                (window_min, comparison_span, FetchStage::PrimaryPlot)
            };

            if let Some(series_min) = series.bars.first().map(|bar| bar.time)
                && target_min < series_min
            {
                let end = align_floor(series_min, timeframe);
                batches.push(FetchBatch {
                    stage,
                    range: FetchRange::Kline(end.saturating_sub(fetch_span), end),
                    tickers: vec![series.ticker_info],
                });
                if stage == FetchStage::IndicatorHistory && series.ticker_info == input.base_ticker
                {
                    base_boundary_fetch_pending = true;
                }
            }
        }

        if !base_boundary_fetch_pending
            && let Some(base_series) = input
                .series
                .iter()
                .find(|series| series.ticker_info == input.base_ticker)
            && let Some(series_earliest) = base_series.bars.first().map(|bar| bar.time)
            && let Some(series_latest) = base_series.bars.last().map(|bar| bar.time)
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
                    tickers: vec![input.base_ticker],
                });
            }
        }

        let open_interest_indicator = input.active_indicators.iter().copied().find(|id| {
            input.indicator_instances.get(*id).is_some_and(|instance| {
                input_requirements(instance.kind).contains(&IndicatorInputSource::OpenInterest)
            })
        });

        if input.open_interest_enabled
            && let Some(open_interest_indicator) = open_interest_indicator
            && matches!(
                indicator_availability(&input, open_interest_indicator),
                IndicatorAvailability::Available
            )
            && let Some(base_series) = input
                .series
                .iter()
                .find(|series| series.ticker_info == input.base_ticker)
            && let Some(kline_latest) = base_series.bars.last().map(|bar| bar.time)
        {
            let visible_window = visible_window(
                input.series,
                input.horizontal_offset,
                timeframe,
                input.visible_points,
            );
            let visible_earliest = visible_window.map(|(start, _)| start).unwrap_or(window_min);
            let visible_span = visible_window
                .map(|(start, end)| end.as_u64().saturating_sub(start.as_u64()))
                .unwrap_or(base_span);
            let prefetch_earliest = visible_earliest.saturating_sub(visible_span);

            match base_series
                .indicator_snapshot(open_interest_indicator)
                .value_range
            {
                Some((oi_earliest, oi_latest)) => {
                    if visible_earliest < oi_earliest {
                        batches.push(FetchBatch {
                            stage: FetchStage::ExternalInputs,
                            range: FetchRange::OpenInterest(prefetch_earliest, oi_earliest),
                            tickers: vec![input.base_ticker],
                        });
                    }

                    if oi_latest < kline_latest {
                        let start = oi_latest.max(prefetch_earliest);
                        if start < kline_latest {
                            batches.push(FetchBatch {
                                stage: FetchStage::ExternalInputs,
                                range: FetchRange::OpenInterest(start, kline_latest),
                                tickers: vec![input.base_ticker],
                            });
                        }
                    }
                }
                None => {
                    if prefetch_earliest < kline_latest {
                        batches.push(FetchBatch {
                            stage: FetchStage::ExternalInputs,
                            range: FetchRange::OpenInterest(prefetch_earliest, kline_latest),
                            tickers: vec![input.base_ticker],
                        });
                    }
                }
            }
        }
    }

    batches
}

pub fn desired_trade_fetch_range(input: FetchPlanInput<'_>) -> Option<FetchRange> {
    let Basis::Time(timeframe) = input.basis else {
        return None;
    };

    if !input.trade_data_required || !input.trade_fetch_enabled {
        return None;
    }

    let dt = timeframe.to_milliseconds().max(1);
    let visible_points = input.visible_points.max(2);
    let (visible_earliest, visible_latest) = visible_window(
        input.series,
        input.horizontal_offset,
        timeframe,
        input.visible_points,
    )
    .unwrap_or_else(|| {
        let latest = align_floor(input.now, timeframe);
        let span = DEFAULT_FETCH_BARS.max(visible_points).saturating_mul(dt);
        (latest.saturating_sub(span), latest)
    });
    let visible_latest = visible_latest.min(align_floor(input.now, timeframe));

    if visible_earliest >= visible_latest {
        return None;
    }

    let base_series = input
        .series
        .iter()
        .find(|series| series.ticker_info == input.base_ticker)?;
    if base_series.bars.is_empty() {
        return None;
    }
    let Some((data_earliest, data_latest)) = base_series.trade_time_range() else {
        return Some(FetchRange::Trades(visible_earliest, visible_latest));
    };

    if visible_earliest < data_earliest {
        let fetch_to = data_earliest.saturating_sub(1);
        return (visible_earliest < fetch_to)
            .then_some(FetchRange::Trades(visible_earliest, fetch_to));
    }
    if visible_latest > data_latest {
        return Some(FetchRange::Trades(
            data_latest.saturating_add(1),
            visible_latest,
        ));
    }

    base_series
        .suggest_trade_fetch_range(visible_earliest, visible_latest, timeframe)
        .map(|(from, to)| FetchRange::Trades(from, to))
}

fn indicator_availability(
    input: &FetchPlanInput<'_>,
    id: IndicatorInstanceId,
) -> IndicatorAvailability {
    let Some(instance) = input.indicator_instances.get(id) else {
        return IndicatorAvailability::Unsupported(
            super::indicators::IndicatorUnsupportedReason::MissingRequiredInput,
        );
    };
    let base_series_data = input
        .series
        .iter()
        .find(|series| series.ticker_info == input.base_ticker)
        .and_then(|series| series.indicators.as_ref());

    availability(
        id,
        instance.kind,
        AvailabilityContext {
            basis: input.basis,
            base_ticker: input.base_ticker,
        },
        base_series_data,
    )
}

fn active_indicator_warmup_bars(
    active_indicators: &[IndicatorInstanceId],
    indicator_instances: &IndicatorInstanceStore,
) -> u64 {
    active_indicators
        .iter()
        .copied()
        .filter_map(|id| indicator_instances.get(id)?.config.warmup_bars())
        .filter(|bars| *bars <= MAX_AUTO_INDICATOR_WARMUP_BARS)
        .max()
        .unwrap_or(0)
}

pub fn visible_window(
    series: &[KlineSeries],
    horizontal_offset: f32,
    timeframe: Timeframe,
    visible_points: u64,
) -> Option<(UnixMs, UnixMs)> {
    let max_seen = series
        .iter()
        .flat_map(|series| series.bars.iter())
        .map(|bar| bar.time)
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

fn align_floor(timestamp: UnixMs, timeframe: Timeframe) -> UnixMs {
    timestamp.floor_to(timeframe)
}
