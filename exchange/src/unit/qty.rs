use crate::{TickerInfo, adapter::MarketKind};

use super::MinQtySize;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU8, Ordering};

/// Unit for displaying volume/quantity size values.
///
/// - `Base`: Display in base asset units (e.g., BTC for BTCUSDT)
/// - `Quote`: Display in quote currency value (e.g., USD/USDT equivalent)
///
/// Note: Only applies to linear perpetuals and spot markets.
/// Inverse perpetuals always display in USD regardless of this setting.
#[repr(u8)]
#[derive(Default, Copy, Clone, Debug, Eq, PartialEq, Hash, Deserialize, Serialize)]
pub enum SizeUnit {
    Base = 0,
    #[default]
    Quote = 1,
}

static SIZE_CALC_UNIT: AtomicU8 = AtomicU8::new(SizeUnit::Base as u8);

pub fn set_preferred_currency(v: SizeUnit) {
    SIZE_CALC_UNIT.store(v as u8, Ordering::Relaxed);
}

pub fn volume_size_unit() -> SizeUnit {
    match SIZE_CALC_UNIT.load(Ordering::Relaxed) {
        0 => SizeUnit::Base,
        1 => SizeUnit::Quote,
        _ => SizeUnit::Base,
    }
}

/// Fixed atomic unit scale: 10^-QTY_SCALE is the smallest stored fraction.
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Deserialize, Serialize,
)]
pub struct Qty {
    /// number of atomic units (atomic unit = 10^-QTY_SCALE)
    pub units: i64,
}

impl Qty {
    /// number of decimal places of the atomic unit
    pub const QTY_SCALE: i32 = 8;
    pub const ZERO: Self = Self { units: 0 };

    pub const fn zero() -> Self {
        Self::ZERO
    }

    /// Lossy: convert qty to f32, may lose precision beyond `QTY_SCALE`
    pub fn to_f32_lossy(self) -> f32 {
        let scale = 10f32.powi(Self::QTY_SCALE);
        (self.units as f32) / scale
    }

    /// Lossy: create Qty from f32 (rounds to nearest atomic unit)
    pub fn from_f32_lossy(v: f32) -> Self {
        let scale = 10f32.powi(Self::QTY_SCALE);
        let units = (v * scale).round() as i64;
        Self { units }
    }

    pub fn from_f32(v: f32) -> Self {
        Self::from_f32_lossy(v)
    }

    pub fn to_f32(self) -> f32 {
        self.to_f32_lossy()
    }

    pub const fn from_units(units: i64) -> Self {
        Self { units }
    }

    /// Absolute quantity, panics on `i64::MIN`.
    pub fn abs(self) -> Self {
        Self {
            units: self.units.checked_abs().expect("Qty abs overflowed"),
        }
    }

    /// Absolute difference between two quantities.
    pub fn abs_diff(self, other: Self) -> Self {
        if self.units >= other.units {
            self - other
        } else {
            other - self
        }
    }

    /// Guards scale/denominator values against zero-ish inputs.
    pub fn scale_or_one(v: f32) -> f32 {
        if v <= f32::EPSILON { 1.0 } else { v }
    }

    /// Converts to f32 and applies `scale_or_one`.
    pub fn to_scale_or_one(self) -> f32 {
        Self::scale_or_one(self.to_f32_lossy())
    }

    fn min_qty_units(min_qty: MinQtySize) -> i64 {
        let exp = Self::QTY_SCALE + (min_qty.power as i32);
        assert!(exp >= 0, "QTY_SCALE must be >= -min_qty.power");
        10i64
            .checked_pow(exp as u32)
            .expect("Qty min_qty units overflowed")
    }

    pub fn round_to_min_qty(self, min_qty: MinQtySize) -> Self {
        let unit = Self::min_qty_units(min_qty);
        if unit <= 1 {
            return self;
        }

        let half = unit / 2;
        let rounded = if self.units >= 0 {
            ((self.units + half).div_euclid(unit)) * unit
        } else {
            ((self.units - half).div_euclid(unit)) * unit
        };

        Self { units: rounded }
    }

    pub fn to_lots(self, min_qty: MinQtySize) -> i64 {
        let unit = Self::min_qty_units(min_qty);
        if unit <= 1 {
            return self.units;
        }

        let half = unit / 2;
        if self.units >= 0 {
            (self.units + half).div_euclid(unit)
        } else {
            (self.units - half).div_euclid(unit)
        }
    }

    pub const fn is_zero(self) -> bool {
        self.units == 0
    }
}

impl From<Qty> for f32 {
    fn from(value: Qty) -> Self {
        value.to_f32_lossy()
    }
}

impl From<f32> for Qty {
    fn from(value: f32) -> Self {
        Qty::from_f32(value)
    }
}

impl std::ops::Add for Qty {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            units: self
                .units
                .checked_add(rhs.units)
                .expect("Qty add overflowed"),
        }
    }
}

impl std::ops::AddAssign for Qty {
    fn add_assign(&mut self, rhs: Self) {
        self.units = self
            .units
            .checked_add(rhs.units)
            .expect("Qty add_assign overflowed");
    }
}

impl std::ops::Sub for Qty {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            units: self
                .units
                .checked_sub(rhs.units)
                .expect("Qty sub overflowed"),
        }
    }
}

impl std::ops::SubAssign for Qty {
    fn sub_assign(&mut self, rhs: Self) {
        self.units = self
            .units
            .checked_sub(rhs.units)
            .expect("Qty sub_assign overflowed");
    }
}

#[derive(Debug, Clone, Copy)]
pub struct QtyNormalization {
    size_in_quote_ccy: bool,
    contract_size: Option<f32>,
    market_kind: MarketKind,
    raw_qty_unit: Option<RawQtyUnit>,
}

/// Unit of raw quantity values returned by an exchange API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawQtyUnit {
    /// Raw quantity is in base asset units (e.g., BTC).
    Base,
    /// Raw quantity is in quote currency value (e.g., USD/USDT).
    Quote,
    /// Raw quantity is in contract count and requires `contract_size` for conversion.
    Contracts,
}

impl QtyNormalization {
    pub fn new(size_in_quote_ccy: bool, ticker: TickerInfo) -> Self {
        let market_kind = ticker.exchange().market_type();

        let contract_size = ticker.contract_size.map(|cs| cs.as_f32()).or({
            if matches!(market_kind, MarketKind::InversePerps) {
                Some(1.0)
            } else {
                None
            }
        });

        Self {
            size_in_quote_ccy,
            contract_size,
            market_kind,
            raw_qty_unit: None,
        }
    }

    pub fn with_raw_qty_unit(
        size_in_quote_ccy: bool,
        ticker: TickerInfo,
        raw_qty_unit: RawQtyUnit,
    ) -> Self {
        let mut normalizer = Self::new(size_in_quote_ccy, ticker);
        normalizer.raw_qty_unit = Some(raw_qty_unit);
        normalizer
    }

    fn normalize_with_raw_unit(self, qty: f32, price: f32, raw_qty_unit: RawQtyUnit) -> f32 {
        let safe_price = Qty::scale_or_one(price);

        let (base_qty, quote_qty) = match raw_qty_unit {
            RawQtyUnit::Base => (qty, qty * price),
            RawQtyUnit::Quote => (qty / safe_price, qty),
            RawQtyUnit::Contracts => {
                let contract_size = self.contract_size.unwrap_or(1.0);

                if matches!(self.market_kind, MarketKind::InversePerps) {
                    let quote_qty = qty * contract_size;
                    (quote_qty / safe_price, quote_qty)
                } else {
                    let base_qty = qty * contract_size;
                    (base_qty, base_qty * price)
                }
            }
        };

        if matches!(self.market_kind, MarketKind::InversePerps) || self.size_in_quote_ccy {
            quote_qty
        } else {
            base_qty
        }
    }

    pub fn normalize(self, qty: f32, price: f32) -> f32 {
        if let Some(raw_qty_unit) = self.raw_qty_unit {
            return self.normalize_with_raw_unit(qty, price, raw_qty_unit);
        }

        let is_inverse = matches!(self.market_kind, MarketKind::InversePerps);

        match self.contract_size {
            Some(contract_size) => {
                if is_inverse {
                    if self.size_in_quote_ccy {
                        qty * contract_size
                    } else {
                        qty
                    }
                } else if self.size_in_quote_ccy {
                    qty * contract_size * price
                } else {
                    qty * contract_size
                }
            }
            None => {
                if self.size_in_quote_ccy {
                    qty * price
                } else {
                    qty
                }
            }
        }
    }

    pub fn normalize_qty(self, qty: f32, price: f32) -> Qty {
        Qty::from_f32(self.normalize(qty, price))
    }
}
