use exchange::{
    TickerInfo,
    unit::{Power10, Price, PriceStep, Qty},
};
use serde::{Deserialize, Serialize};

pub const Y_UNIT_STEP_FALLBACK: f32 = 1e-4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct YUnit(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct YUnitRange {
    min: YUnit,
    max: YUnit,
}

impl YUnitRange {
    pub fn new(min: YUnit, max: YUnit) -> Self {
        if min <= max {
            Self { min, max }
        } else {
            Self { min: max, max: min }
        }
    }

    pub fn padded(self) -> Self {
        let span = self.max.0.abs_diff(self.min.0);
        let pad = if span == 0 {
            self.max.0.unsigned_abs().max(1).saturating_add(49) / 50
        } else {
            span.saturating_mul(5).saturating_add(99) / 100
        }
        .max(1);
        let pad = i64::try_from(pad).unwrap_or(i64::MAX);
        Self::new(
            YUnit(self.min.0.saturating_sub(pad)),
            YUnit(self.max.0.saturating_add(pad)),
        )
    }

    pub fn unit_at_ratio(self, ratio: f32) -> YUnit {
        let ratio = ratio.clamp(0.0, 1.0);
        let span = self.max.0.saturating_sub(self.min.0);
        let offset = (span as f32 * ratio).round() as i64;
        YUnit(self.min.0.saturating_add(offset).min(self.max.0))
    }

    pub fn min(self) -> YUnit {
        self.min
    }

    pub fn max(self) -> YUnit {
        self.max
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PanelValueLabelMode {
    Compact,
    Commas,
    Abbreviated,
}

impl PanelValueLabelMode {
    pub fn format_value(
        self,
        scale: PanelValueScale,
        value: f64,
        fallback_step: f64,
        max_decimals: Option<u8>,
    ) -> String {
        match self {
            Self::Compact => scale.format_compact(value, fallback_step, max_decimals),
            Self::Commas => crate::util::format_with_commas(scale.quantize_value(value)),
            Self::Abbreviated => crate::util::abbr_large_numbers(scale.quantize_value(value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelValueLabelPolicy {
    pub axis_mode: PanelValueLabelMode,
    pub header_mode: PanelValueLabelMode,
    pub max_decimals: Option<u8>,
}

impl Default for PanelValueLabelPolicy {
    fn default() -> Self {
        Self {
            axis_mode: PanelValueLabelMode::Compact,
            header_mode: PanelValueLabelMode::Commas,
            max_decimals: None,
        }
    }
}

impl PanelValueLabelPolicy {
    pub fn format_axis_value(
        self,
        scale: PanelValueScale,
        value: f64,
        fallback_step: f64,
    ) -> String {
        self.axis_mode
            .format_value(scale, value, fallback_step, self.max_decimals)
    }

    pub fn format_header_value(
        self,
        scale: PanelValueScale,
        value: f64,
        fallback_step: f64,
    ) -> String {
        self.header_mode
            .format_value(scale, value, fallback_step, self.max_decimals)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PanelValuePrecision {
    BaseTickerMinTick,
    BaseTickerMinQty,
    FixedPower10(Power10<-8, 8>),
    FixedStep(f64),
}

impl PanelValuePrecision {
    const MIN_STEP_F64: f64 = 1e-8;

    pub fn quantization_step_f32(self, base_ticker: Option<TickerInfo>) -> Option<f32> {
        self.quantization_step(base_ticker).map(|step| step as f32)
    }

    pub fn quantization_step(self, base_ticker: Option<TickerInfo>) -> Option<f64> {
        let step = match self {
            Self::BaseTickerMinTick => base_ticker.map(|ticker| ticker.min_ticksize.as_f64()),
            Self::BaseTickerMinQty => base_ticker.map(|ticker| ticker.min_qty.as_f64()),
            Self::FixedPower10(step) => Some(step.as_f64()),
            Self::FixedStep(step) => Some(step),
        }?;

        Some(step.abs().max(Self::MIN_STEP_F64))
    }

    pub fn value_bounds(self, base_ticker: Option<TickerInfo>) -> Option<(f64, f64)> {
        let bounds = match self {
            Self::BaseTickerMinTick => {
                let unit_size =
                    base_ticker.map(|ticker| Price::min_tick_units(ticker.min_ticksize))?;
                let min_units = i64::MIN.div_euclid(unit_size).saturating_mul(unit_size);
                let max_units = i64::MAX.div_euclid(unit_size).saturating_mul(unit_size);
                (
                    Price::from_units(min_units).to_f64(),
                    Price::from_units(max_units).to_f64(),
                )
            }
            Self::BaseTickerMinQty => {
                let unit_size = base_ticker.map(|ticker| Qty::min_qty_units(ticker.min_qty))?;
                let min_units = i64::MIN.div_euclid(unit_size).saturating_mul(unit_size);
                let max_units = i64::MAX.div_euclid(unit_size).saturating_mul(unit_size);
                (
                    Qty::from_units(min_units).to_f64(),
                    Qty::from_units(max_units).to_f64(),
                )
            }
            Self::FixedPower10(step) => {
                let step = step.as_f64().abs().max(Self::MIN_STEP_F64);
                (i64::MIN as f64 * step, i64::MAX as f64 * step)
            }
            Self::FixedStep(step) => {
                let step = step.abs().max(Self::MIN_STEP_F64);
                (i64::MIN as f64 * step, i64::MAX as f64 * step)
            }
        };

        (bounds.0.is_finite() && bounds.1.is_finite() && bounds.0 < bounds.1).then_some(bounds)
    }

    fn decimals_from_power10(power: i8) -> usize {
        if power < 0 { (-power) as usize } else { 0 }
    }

    pub fn decimals_for_step(step: f64) -> usize {
        let step = step.abs();
        if !step.is_finite() || step <= 0.0 {
            return 4;
        }

        for decimals in 0..=8 {
            let scaled = step * 10_f64.powi(decimals as i32);
            let nearest = scaled.round();
            let tolerance = (scaled.abs() * 1e-9).max(1e-12);

            if (scaled - nearest).abs() <= tolerance {
                return decimals;
            }
        }

        8
    }

    pub fn decimals(self, base_ticker: Option<TickerInfo>) -> Option<usize> {
        match self {
            Self::BaseTickerMinTick => {
                base_ticker.map(|ticker| Self::decimals_from_power10(ticker.min_ticksize.power))
            }
            Self::BaseTickerMinQty => {
                base_ticker.map(|ticker| Self::decimals_from_power10(ticker.min_qty.power))
            }
            Self::FixedPower10(step) => Some(Self::decimals_from_power10(step.power)),
            Self::FixedStep(step) => Some(Self::decimals_for_step(step)),
        }
    }

    pub fn value_to_unit_f32(
        self,
        value: f32,
        base_ticker: Option<TickerInfo>,
        fallback_step: f32,
    ) -> YUnit {
        self.value_to_unit(f64::from(value), base_ticker, f64::from(fallback_step))
    }

    /// Convert an exact price to display units when the precision is price-native.
    /// `None` means the precision uses another numeric domain or an arbitrary step.
    pub fn price_to_unit(self, value: Price, base_ticker: Option<TickerInfo>) -> Option<YUnit> {
        match self {
            Self::BaseTickerMinTick => {
                let ticker = base_ticker?;
                let min_tick = ticker.min_ticksize;
                let unit_size = Price::min_tick_units(min_tick);
                Some(YUnit(
                    value
                        .round_to_min_tick(min_tick)
                        .units
                        .div_euclid(unit_size),
                ))
            }
            Self::FixedPower10(step) => {
                let step_value = step.as_f64();
                let step_price = Price::from_f64(step_value);
                if step_price.units <= 0 || step_price.to_f64() != step_value {
                    return None;
                }
                Some(YUnit(
                    value
                        .round_to_step(PriceStep {
                            units: step_price.units,
                        })
                        .units
                        .div_euclid(step_price.units),
                ))
            }
            Self::BaseTickerMinQty | Self::FixedStep(_) => None,
        }
    }

    pub fn value_to_unit(
        self,
        value: f64,
        base_ticker: Option<TickerInfo>,
        fallback_step: f64,
    ) -> YUnit {
        if !value.is_finite() {
            return YUnit(0);
        }

        match self {
            Self::BaseTickerMinTick => {
                if let Some(ticker) = base_ticker {
                    let min_tick = ticker.min_ticksize;
                    let unit_size = Price::min_tick_units(min_tick);
                    let price_units = Price::from_f64(value).round_to_min_tick(min_tick).units;
                    return YUnit(price_units.div_euclid(unit_size));
                }
            }
            Self::BaseTickerMinQty => {
                if let Some(ticker) = base_ticker {
                    let min_qty = ticker.min_qty;
                    let unit_size = Qty::min_qty_units(min_qty);
                    let qty_units = Qty::from_f64(value).round_to_min_qty(min_qty).units;
                    return YUnit(qty_units.div_euclid(unit_size));
                }
            }
            Self::FixedPower10(_) | Self::FixedStep(_) => {}
        }

        let step = self
            .quantization_step(base_ticker)
            .unwrap_or(fallback_step.abs().max(Self::MIN_STEP_F64));
        YUnit((value / step).round() as i64)
    }

    pub fn unit_to_value_f32(
        self,
        y_unit: YUnit,
        base_ticker: Option<TickerInfo>,
        fallback_step: f32,
    ) -> f32 {
        self.unit_to_value(y_unit, base_ticker, f64::from(fallback_step)) as f32
    }

    pub fn unit_to_value(
        self,
        y_unit: YUnit,
        base_ticker: Option<TickerInfo>,
        fallback_step: f64,
    ) -> f64 {
        match self {
            Self::BaseTickerMinTick => {
                if let Some(ticker) = base_ticker {
                    let min_tick = ticker.min_ticksize;
                    let unit_size = Price::min_tick_units(min_tick);
                    let units = y_unit.0.saturating_mul(unit_size);
                    return Price::from_units(units).to_f64();
                }
            }
            Self::BaseTickerMinQty => {
                if let Some(ticker) = base_ticker {
                    let min_qty = ticker.min_qty;
                    let unit_size = Qty::min_qty_units(min_qty);
                    let units = y_unit.0.saturating_mul(unit_size);
                    return Qty::from_units(units).to_f64();
                }
            }
            Self::FixedPower10(_) | Self::FixedStep(_) => {}
        }

        let step = self
            .quantization_step(base_ticker)
            .unwrap_or(fallback_step.abs().max(Self::MIN_STEP_F64));
        (y_unit.0 as f64) * step
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PanelValueScale {
    precision: Option<PanelValuePrecision>,
    base_ticker: Option<TickerInfo>,
}

impl PanelValueScale {
    fn fallback_value_to_unit_f32(value: f32) -> YUnit {
        if !value.is_finite() {
            return YUnit(0);
        }

        YUnit((value / Y_UNIT_STEP_FALLBACK).round() as i64)
    }

    fn fallback_value_to_unit(value: f64) -> YUnit {
        if !value.is_finite() {
            return YUnit(0);
        }

        YUnit((value / f64::from(Y_UNIT_STEP_FALLBACK)).round() as i64)
    }

    pub fn new(precision: Option<PanelValuePrecision>, base_ticker: Option<TickerInfo>) -> Self {
        Self {
            precision,
            base_ticker,
        }
    }

    pub fn minimum_span(self) -> f64 {
        self.quantization_step()
            .unwrap_or(f64::from(Y_UNIT_STEP_FALLBACK))
            .abs()
            .max(1e-8)
            * 8.0
    }

    pub fn value_bounds(self) -> (f64, f64) {
        self.precision
            .and_then(|precision| precision.value_bounds(self.base_ticker))
            .unwrap_or_else(|| {
                let step = f64::from(Y_UNIT_STEP_FALLBACK);
                (i64::MIN as f64 * step, i64::MAX as f64 * step)
            })
    }

    pub fn quantize_value(self, value: f64) -> f64 {
        self.unit_to_value(self.value_to_unit(value))
    }

    fn format_compact(self, value: f64, fallback_step: f64, max_decimals: Option<u8>) -> String {
        let quantized = self.quantize_value(value);
        let fallback = fallback_step.abs().max(1e-6);
        let decimals = self
            .precision
            .and_then(|precision| precision.decimals(self.base_ticker))
            .unwrap_or_else(|| PanelValuePrecision::decimals_for_step(fallback));
        let decimals = max_decimals
            .map(|max| decimals.min(max as usize))
            .unwrap_or(decimals);
        format!("{quantized:.decimals$}")
    }

    pub fn quantization_step_f32(self) -> Option<f32> {
        self.precision
            .and_then(|precision| precision.quantization_step_f32(self.base_ticker))
    }

    pub fn quantization_step(self) -> Option<f64> {
        self.precision
            .and_then(|precision| precision.quantization_step(self.base_ticker))
    }

    pub fn value_to_unit_f32(self, value: f32) -> YUnit {
        self.precision
            .map(|precision| {
                precision.value_to_unit_f32(value, self.base_ticker, Y_UNIT_STEP_FALLBACK)
            })
            .unwrap_or_else(|| Self::fallback_value_to_unit_f32(value))
    }

    pub fn value_to_unit(self, value: f64) -> YUnit {
        self.precision
            .map(|precision| {
                precision.value_to_unit(value, self.base_ticker, f64::from(Y_UNIT_STEP_FALLBACK))
            })
            .unwrap_or_else(|| Self::fallback_value_to_unit(value))
    }

    pub fn unit_to_value_f32(self, unit: YUnit) -> f32 {
        self.precision
            .map(|precision| {
                precision.unit_to_value_f32(unit, self.base_ticker, Y_UNIT_STEP_FALLBACK)
            })
            .unwrap_or_else(|| (unit.0 as f32) * Y_UNIT_STEP_FALLBACK)
    }

    pub fn unit_to_value(self, unit: YUnit) -> f64 {
        self.precision
            .map(|precision| {
                precision.unit_to_value(unit, self.base_ticker, f64::from(Y_UNIT_STEP_FALLBACK))
            })
            .unwrap_or_else(|| (unit.0 as f64) * f64::from(Y_UNIT_STEP_FALLBACK))
    }

    pub fn price_to_unit(self, value: Price) -> YUnit {
        self.precision
            .and_then(|precision| precision.price_to_unit(value, self.base_ticker))
            .unwrap_or_else(|| self.value_to_unit_f32(value.to_f32_lossy()))
    }

    pub fn values(self, range: YUnitRange) -> (f64, f64) {
        (
            self.unit_to_value(range.min()),
            self.unit_to_value(range.max()),
        )
    }

    pub fn values_f32(self, range: YUnitRange) -> (f32, f32) {
        (
            self.unit_to_value_f32(range.min()),
            self.unit_to_value_f32(range.max()),
        )
    }
}
