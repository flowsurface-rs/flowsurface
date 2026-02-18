use super::{MinTicksize, Power10};
use serde::{Deserialize, Serialize};

/// Fixed atomic unit scale: 10^-PRICE_SCALE is the smallest stored fraction.
/// MinTicksize has range [-8, 2], e.g. PRICE_SCALE = 8 to represent 10^-8 atomic units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Deserialize, Serialize)]
pub struct Price {
    /// number of atomic units (atomic unit = 10^-PRICE_SCALE)
    pub units: i64,
}

impl Price {
    /// number of decimal places of the atomic unit (10^-8)
    pub const PRICE_SCALE: i32 = 8;

    #[inline]
    pub fn to_string<const MIN: i8, const MAX: i8>(self, precision: Power10<MIN, MAX>) -> String {
        let mut out = String::with_capacity(24);
        self.fmt_into(precision, &mut out).unwrap();
        out
    }

    #[inline]
    pub fn fmt_into<const MIN: i8, const MAX: i8, W: core::fmt::Write>(
        self,
        precision: Power10<MIN, MAX>,
        out: &mut W,
    ) -> core::fmt::Result {
        let scale_u = Self::PRICE_SCALE as u32;

        // number of atomic units for the given decade step: 10^(PRICE_SCALE + power)
        let exp = (Self::PRICE_SCALE + precision.power as i32) as u32;
        debug_assert!(Self::PRICE_SCALE + precision.power as i32 >= 0);
        let unit = 10i64
            .checked_pow(exp)
            .expect("Price::to_string unit overflow");

        let u = self.units;
        let half = unit / 2;
        let rounded_units = if u >= 0 {
            ((u + half).div_euclid(unit)) * unit
        } else {
            ((u - half).div_euclid(unit)) * unit
        };

        let decimals: u32 = if precision.power < 0 {
            ((-precision.power) as u32).min(scale_u)
        } else {
            0
        };

        if rounded_units < 0 {
            core::fmt::Write::write_char(out, '-')?;
        }
        let abs_u = (rounded_units as i128).unsigned_abs();

        let scale_pow = 10u128.pow(scale_u);
        let int_part = abs_u / scale_pow;
        write!(out, "{}", int_part)?;

        if decimals == 0 {
            return Ok(());
        }

        let frac_div = 10u128.pow(scale_u - decimals);
        let frac_part = (abs_u % scale_pow) / frac_div;
        write!(out, ".{:0width$}", frac_part, width = decimals as usize)
    }

    /// Lossy: convert price to f32, may lose precision if going beyond `PRICE_SCALE`
    pub fn to_f32_lossy(self) -> f32 {
        let scale = 10f32.powi(Self::PRICE_SCALE);
        (self.units as f32) / scale
    }

    /// Lossy: create Price from f32 (rounds to nearest atomic unit)
    pub fn from_f32_lossy(v: f32) -> Self {
        let scale = 10f32.powi(Self::PRICE_SCALE);
        let u = (v * scale).round() as i64;
        Self { units: u }
    }

    pub fn from_f32(v: f32) -> Self {
        Self::from_f32_lossy(v)
    }

    pub fn to_f32(self) -> f32 {
        self.to_f32_lossy()
    }

    pub fn round_to_step(self, step: PriceStep) -> Self {
        let unit = step.units;
        if unit <= 1 {
            return self;
        }
        let half = unit / 2;
        let rounded = ((self.units + half).div_euclid(unit)) * unit;
        Self { units: rounded }
    }

    /// Floor to multiple of an arbitrary step
    fn floor_to_step(self, step: PriceStep) -> Self {
        let unit = step.units;
        if unit <= 1 {
            return self;
        }
        let floored = (self.units.div_euclid(unit)) * unit;
        Self { units: floored }
    }

    /// Ceil to multiple of an arbitrary step
    fn ceil_to_step(self, step: PriceStep) -> Self {
        let unit = step.units;
        if unit <= 1 {
            return self;
        }
        let added = self.units.checked_add(unit - 1).unwrap_or_else(|| {
            if self.units.is_negative() {
                i64::MIN
            } else {
                i64::MAX
            }
        });

        let ceiled = (added.div_euclid(unit)) * unit;
        Self { units: ceiled }
    }

    /// Group with arbitrary step (e.g. sells floor, buys ceil)
    pub fn round_to_side_step(self, is_sell_or_bid: bool, step: PriceStep) -> Self {
        if is_sell_or_bid {
            self.floor_to_step(step)
        } else {
            self.ceil_to_step(step)
        }
    }

    /// Create Price from raw atomic units (no rounding) — internal only
    pub fn from_units(units: i64) -> Self {
        Self { units }
    }

    /// Returns the atomic-unit count that corresponds to one min tick (min_tick / atomic_unit)
    fn min_tick_units(min_tick: MinTicksize) -> i64 {
        let exp = Self::PRICE_SCALE + (min_tick.power as i32);
        assert!(exp >= 0, "PRICE_SCALE must be >= -min_tick.power");
        10i64
            .checked_pow(exp as u32)
            .expect("min_tick_units overflowed")
    }

    /// Round this Price to the nearest multiple of the provided min_ticksize
    pub fn round_to_min_tick(self, min_tick: MinTicksize) -> Self {
        let unit = Self::min_tick_units(min_tick);
        if unit <= 1 {
            return self;
        }
        let half = unit / 2;
        let rounded = ((self.units + half).div_euclid(unit)) * unit;
        Self { units: rounded }
    }

    pub fn add_steps(self, steps: i64, step: PriceStep) -> Self {
        Self::from_units(
            self.units
                .checked_add(steps.saturating_mul(step.units))
                .expect("add_steps overflowed"),
        )
    }

    /// Number of step increments between low..=high (inclusive), or None if invalid.
    pub fn steps_between_inclusive(low: Price, high: Price, step: PriceStep) -> Option<usize> {
        if high.units < low.units || step.units <= 0 {
            return None;
        }
        let span = high.units.checked_sub(low.units)?;
        Some((span / step.units) as usize + 1)
    }
}

impl std::ops::Add for Price {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            units: self
                .units
                .checked_add(rhs.units)
                .expect("Price add overflowed"),
        }
    }
}

impl std::ops::Div<i64> for Price {
    type Output = Self;

    fn div(self, rhs: i64) -> Self::Output {
        Self {
            units: self.units.div_euclid(rhs),
        }
    }
}

impl std::ops::Sub for Price {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            units: self
                .units
                .checked_sub(rhs.units)
                .expect("Price sub overflowed"),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct PriceStep {
    /// step size in atomic units (10^-PRICE_SCALE)
    pub units: i64,
}

impl PriceStep {
    /// Lossy: f32 step for UI
    pub fn to_f32_lossy(self) -> f32 {
        let scale = 10f32.powi(Price::PRICE_SCALE);
        (self.units as f32) / scale
    }

    /// Lossy: from f32 step (rounds to nearest atomic unit)
    pub fn from_f32_lossy(step: f32) -> Self {
        assert!(step > 0.0, "step must be > 0");
        let scale = 10f32.powi(Price::PRICE_SCALE);
        let units = (step * scale).round() as i64;
        assert!(units > 0, "step too small at given PRICE_SCALE");
        Self { units }
    }

    pub fn from_f32(step: f32) -> Self {
        Self::from_f32_lossy(step)
    }
}
