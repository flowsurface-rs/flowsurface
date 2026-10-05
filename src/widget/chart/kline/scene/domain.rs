use super::{ChartCoordinates, PanelValueScale, PrimaryPercentScaleState};
use crate::widget::chart::kline::composition::{MarkKind, PanelScaleMode, PanelValuePrecision};
use crate::widget::chart::kline::presentation::{IndicatorView, ResolvedIndicatorLayer};
use data::chart::kline::series::KlineWindows;
use data::chart::kline::viewport::YUnit;
use exchange::unit::Price;

pub(crate) struct WindowDomains<'a> {
    pub(crate) windows: KlineWindows<'a>,
    pub(crate) indicators: IndicatorView<'a>,
    pub(crate) coordinates: ChartCoordinates<'a>,
    pub(crate) min_x_unit: i64,
    pub(crate) max_x_unit: i64,
    pub(crate) include_footprint_range: bool,
}

impl WindowDomains<'_> {
    fn primary_anchor(&self) -> Option<f32> {
        let mut first_unit = i64::MAX;
        let mut base_close = None;

        self.windows.for_each_point(
            self.min_x_unit,
            self.max_x_unit,
            |series_index, _, _, unit, point| {
                if series_index != 0 {
                    return;
                }

                if unit < first_unit {
                    first_unit = unit;
                    base_close = Some(point.kline.close.to_f32_lossy());
                }
            },
        );

        base_close
    }

    fn series_anchors(&self) -> Vec<Option<f32>> {
        let mut anchors = vec![None; self.windows.series.len()];
        let mut first_units = vec![i64::MAX; self.windows.series.len()];

        self.windows.for_each_point(
            self.min_x_unit,
            self.max_x_unit,
            |series_index, _, _, unit, point| {
                if unit < first_units[series_index] {
                    first_units[series_index] = unit;
                    anchors[series_index] = Some(point.kline.close.to_f32_lossy());
                }
            },
        );

        anchors
    }

    fn primary_percent_domain(
        &self,
        primary_mark: MarkKind,
        anchors: &[Option<f32>],
    ) -> Option<(f32, f32)> {
        let mut min_pct: Option<f64> = None;
        let mut max_pct: Option<f64> = None;
        let primary_overlay_layers = self.indicators.visible_primary_overlay_layers();
        let include_primary_ohlc_range = self.uses_primary_ohlc_range(primary_mark);
        let include_footprint_range = self.include_footprint_range;
        let footprint_half_tick = self
            .windows
            .series
            .first()
            .map(|series| series.tick_size().to_f32_lossy() * 0.5)
            .unwrap_or(0.0);
        let visible_series: Vec<_> = self
            .windows
            .series
            .iter()
            .map(|series| {
                self.coordinates
                    .series_participates_in_primary_plot(series.ticker_info)
            })
            .collect();
        self.windows.for_each_point(
            self.min_x_unit,
            self.max_x_unit,
            |series_index, _bar_index, _series, _unit, point| {
                if !visible_series[series_index] {
                    return;
                }
                let Some(anchor) = anchors
                    .get(series_index)
                    .copied()
                    .flatten()
                    .filter(|anchor| anchor.abs() > f32::EPSILON)
                else {
                    return;
                };
                let anchor = f64::from(anchor);

                let mut emit_pct = |value: f32| {
                    let pct = ((f64::from(value) / anchor) - 1.0) * 100.0;
                    min_pct = Some(min_pct.map_or(pct, |current| current.min(pct)));
                    max_pct = Some(max_pct.map_or(pct, |current| current.max(pct)));
                };

                if series_index == 0 && include_primary_ohlc_range {
                    emit_pct(point.kline.low.to_f32_lossy());
                    emit_pct(point.kline.high.to_f32_lossy());
                } else {
                    emit_pct(point.kline.close.to_f32_lossy());
                }

                if series_index == 0 && include_footprint_range {
                    for price in point.footprint.trades.keys() {
                        let price = price.to_f32_lossy();
                        emit_pct(price - footprint_half_tick);
                        emit_pct(price + footprint_half_tick);
                    }
                }
            },
        );

        for layer in &primary_overlay_layers {
            let output = &layer.presentation_output;
            self.indicators.for_each_indicator_output_index_with_units(
                self.windows.windows,
                self.windows.axis,
                self.min_x_unit,
                self.max_x_unit,
                output,
                |series, source_index, _output_unit| {
                    if !self.indicators.indicator_input_available_at(
                        series,
                        source_index,
                        layer.required_input,
                    ) {
                        return;
                    }
                    let Some(anchor) = anchors
                        .first()
                        .copied()
                        .flatten()
                        .filter(|anchor| anchor.abs() > f32::EPSILON)
                    else {
                        return;
                    };
                    let anchor = f64::from(anchor);
                    let emit_output_value = |value: f64| {
                        if !value.is_finite() {
                            return;
                        }
                        let pct = ((value / anchor) - 1.0) * 100.0;
                        min_pct = Some(min_pct.map_or(pct, |current| current.min(pct)));
                        max_pct = Some(max_pct.map_or(pct, |current| current.max(pct)));
                    };

                    self.indicators.for_each_layer_domain_value(
                        series,
                        layer,
                        source_index,
                        emit_output_value,
                    );
                },
            );
        }

        let min_pct = min_pct?;
        let max_pct = max_pct?;

        let pad = if (max_pct - min_pct).abs() <= f64::EPSILON {
            max_pct.abs().max(1.0)
        } else {
            (max_pct - min_pct).abs() * 0.05
        };

        Some(((min_pct - pad) as f32, (max_pct + pad) as f32))
    }

    fn uses_primary_ohlc_range(&self, primary_mark: MarkKind) -> bool {
        !primary_mark.is_line_like() || self.include_footprint_range
    }

    pub(crate) fn percent_scale_state(
        &self,
        primary_mark: MarkKind,
        primary_scale_mode: PanelScaleMode,
    ) -> Option<PrimaryPercentScaleState> {
        if !primary_scale_mode.uses_percent_base() {
            return Some(PrimaryPercentScaleState {
                primary_scale_anchor: None,
                series_percent_anchors: vec![None; self.windows.series.len()],
                primary_domain_display_override: None,
            });
        }

        if primary_scale_mode.uses_series_percent_anchors(self.windows.series.len()) {
            let anchors = self.series_anchors();
            let domain_override = self.primary_percent_domain(primary_mark, &anchors)?;

            let base_anchor = anchors.first().copied().flatten();
            return Some(PrimaryPercentScaleState {
                primary_scale_anchor: base_anchor,
                series_percent_anchors: anchors,
                primary_domain_display_override: Some(domain_override),
            });
        }

        let base_anchor = self.primary_anchor();
        Some(PrimaryPercentScaleState {
            primary_scale_anchor: base_anchor,
            series_percent_anchors: vec![base_anchor; self.windows.series.len()],
            primary_domain_display_override: None,
        })
    }

    pub(crate) fn primary_domain(
        &self,
        primary_value_precision: Option<PanelValuePrecision>,
        primary_mark: MarkKind,
        primary_plot_height: f32,
    ) -> Option<(YUnit, YUnit)> {
        let value_scale =
            PanelValueScale::new(primary_value_precision, self.coordinates.base_ticker_info());
        let mut primary_extrema = PrimaryDomainExtrema::default();
        let primary_overlay_layers = self.indicators.visible_primary_overlay_layers();
        let include_primary_ohlc_range = self.uses_primary_ohlc_range(primary_mark);
        let include_footprint_range = self.include_footprint_range;
        let mut has_footprint_rows = false;

        let visible_series: Vec<_> = self
            .windows
            .series
            .iter()
            .map(|series| {
                self.coordinates
                    .series_participates_in_primary_plot(series.ticker_info)
            })
            .collect();
        self.windows.for_each_point(
            self.min_x_unit,
            self.max_x_unit,
            |series_index, _bar_index, _series, _unit, point| {
                if !visible_series[series_index] {
                    return;
                }
                let (low, high) = if series_index == 0 && include_primary_ohlc_range {
                    (point.kline.low, point.kline.high)
                } else {
                    let close = point.kline.close;
                    (close, close)
                };

                primary_extrema.record_price(low);
                primary_extrema.record_price(high);

                if series_index == 0 && include_footprint_range {
                    for price in point.footprint.trades.keys() {
                        has_footprint_rows = true;
                        primary_extrema.record_price(*price);
                    }
                }
            },
        );

        for layer in &primary_overlay_layers {
            let output = &layer.presentation_output;
            self.indicators.for_each_indicator_output_index_with_units(
                self.windows.windows,
                self.windows.axis,
                self.min_x_unit,
                self.max_x_unit,
                output,
                |series, source_index, _output_unit| {
                    if !self.indicators.indicator_input_available_at(
                        series,
                        source_index,
                        layer.required_input,
                    ) {
                        return;
                    }
                    self.indicators.for_each_layer_domain_value(
                        series,
                        layer,
                        source_index,
                        |value| primary_extrema.record_scalar(value),
                    );
                },
            );
        }

        let min_primary_unit = match primary_extrema.min? {
            PrimaryDomainValue::Price(value) => value_scale.price_to_unit(value).0,
            PrimaryDomainValue::Scalar(value) => value_scale.value_to_unit(value).0,
        };
        let max_primary_unit = match primary_extrema.max? {
            PrimaryDomainValue::Price(value) => value_scale.price_to_unit(value).0,
            PrimaryDomainValue::Scalar(value) => value_scale.value_to_unit(value).0,
        };

        let footprint_tick_units = has_footprint_rows.then(|| {
            let zero_unit = value_scale.price_to_unit(Price::from_units(0));
            let tick_value = self
                .windows
                .series
                .first()
                .map(|series| Price::from_units(series.tick_size().units))
                .unwrap_or_else(|| Price::from_units(0));
            let tick_unit = value_scale.price_to_unit(tick_value);
            i64::try_from(tick_unit.0.abs_diff(zero_unit.0))
                .unwrap_or(i64::MAX)
                .max(1)
        });
        let (padded_min, padded_max) = padded_primary_domain_units(
            min_primary_unit,
            max_primary_unit,
            primary_plot_height,
            footprint_tick_units,
        );

        Some((YUnit(padded_min), YUnit(padded_max)))
    }

    pub(crate) fn indicator_domain(
        &self,
        output_layers: &[ResolvedIndicatorLayer],
        panel_value_precision: Option<PanelValuePrecision>,
        independent_scale: bool,
        scale_mode: PanelScaleMode,
    ) -> Option<(YUnit, YUnit)> {
        let mut min_value = f64::INFINITY;
        let mut max_value = f64::NEG_INFINITY;

        for layer in output_layers {
            let Some(spec) = self
                .indicators
                .indicator_output_spec(&layer.presentation_output)
            else {
                continue;
            };
            if spec.independent_scale != independent_scale {
                continue;
            }
            self.indicators.for_each_indicator_output_index_with_units(
                self.windows.windows,
                self.windows.axis,
                self.min_x_unit,
                self.max_x_unit,
                &layer.presentation_output,
                |series, source_index, _output_unit| {
                    let record_value = |value: f64| {
                        if !value.is_finite() {
                            return;
                        }
                        min_value = min_value.min(value);
                        max_value = max_value.max(value);
                    };

                    self.indicators.for_each_layer_domain_value(
                        series,
                        layer,
                        source_index,
                        record_value,
                    );
                },
            );
        }

        if !min_value.is_finite() {
            return None;
        }

        let value_scale =
            PanelValueScale::new(panel_value_precision, self.coordinates.base_ticker_info());
        let min_unit = value_scale.value_to_unit(min_value).0;
        let max_unit = value_scale.value_to_unit(max_value).0;
        let scale_mode = if independent_scale {
            PanelScaleMode::FitVisibleIncludeZero
        } else {
            scale_mode
        };
        Some(scale_mode.indicator_domain_units(min_unit, max_unit))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum PrimaryDomainValue {
    Price(Price),
    Scalar(f64),
}

#[derive(Debug, Default)]
struct PrimaryDomainExtrema {
    min: Option<PrimaryDomainValue>,
    max: Option<PrimaryDomainValue>,
}

impl PrimaryDomainExtrema {
    fn record_price(&mut self, value: Price) {
        self.record(PrimaryDomainValue::Price(value));
    }

    fn record_scalar(&mut self, value: f64) {
        if value.is_finite() {
            self.record(PrimaryDomainValue::Scalar(value));
        }
    }

    fn record(&mut self, value: PrimaryDomainValue) {
        if self
            .min
            .is_none_or(|current| primary_domain_value_less(value, current))
        {
            self.min = Some(value);
        }
        if self
            .max
            .is_none_or(|current| primary_domain_value_less(current, value))
        {
            self.max = Some(value);
        }
    }
}

fn primary_domain_value_less(lhs: PrimaryDomainValue, rhs: PrimaryDomainValue) -> bool {
    match (lhs, rhs) {
        (PrimaryDomainValue::Price(lhs), PrimaryDomainValue::Price(rhs)) => lhs < rhs,
        (PrimaryDomainValue::Scalar(lhs), PrimaryDomainValue::Scalar(rhs)) => lhs < rhs,
        (lhs, rhs) => lhs.as_f64() < rhs.as_f64(),
    }
}

impl PrimaryDomainValue {
    fn as_f64(self) -> f64 {
        match self {
            Self::Price(value) => value.to_f64(),
            Self::Scalar(value) => value,
        }
    }
}

fn padded_primary_domain_units(
    min_unit: i64,
    max_unit: i64,
    plot_height: f32,
    footprint_tick_units: Option<i64>,
) -> (i64, i64) {
    let mut padded_min = min_unit.min(max_unit);
    let mut padded_max = min_unit.max(max_unit);

    if let Some(tick_units) = footprint_tick_units {
        let half_tick_units = tick_units.saturating_add(1) / 2;
        padded_min = padded_min.saturating_sub(half_tick_units);
        padded_max = padded_max.saturating_add(half_tick_units);
    }

    let span = padded_min.abs_diff(padded_max);
    let base_padding = if span == 0 {
        padded_max.unsigned_abs().max(1).saturating_add(199) / 200
    } else {
        span.saturating_mul(5).saturating_add(99) / 100
    };
    let base_padding = i64::try_from(base_padding).unwrap_or(i64::MAX).max(1);

    let outer_padding = if footprint_tick_units.is_some() && plot_height > f32::EPSILON {
        let provisional_span = span.saturating_add((base_padding as u64).saturating_mul(2));
        (4.0 * provisional_span as f32 / plot_height).ceil() as i64
    } else {
        0
    };
    let padding = base_padding.saturating_add(outer_padding);

    padded_min = padded_min.saturating_sub(padding);
    padded_max = padded_max.saturating_add(padding);
    (padded_min, padded_max)
}
