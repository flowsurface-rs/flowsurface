use serde::{Deserialize, Serialize};
use std::fmt;

/// Unix timestamp in milliseconds.
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize,
)]
#[serde(transparent)]
pub struct UnixMs(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnixMsRangeError {
    InvalidBounds { min: UnixMs, max: UnixMs },
    BelowMinimum { value: UnixMs, min: UnixMs },
    AboveMaximum { value: UnixMs, max: UnixMs },
}

impl fmt::Display for UnixMsRangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBounds { min, max } => {
                write!(
                    f,
                    "invalid UnixMs bounds: min ({min}) is greater than max ({max})"
                )
            }
            Self::BelowMinimum { value, min } => {
                write!(f, "UnixMs value {value} is below minimum {min}")
            }
            Self::AboveMaximum { value, max } => {
                write!(f, "UnixMs value {value} is above maximum {max}")
            }
        }
    }
}

impl std::error::Error for UnixMsRangeError {}

impl UnixMs {
    pub const MILLIS_PER_SECOND: u64 = 1_000;
    pub const ZERO: Self = Self(0);

    #[inline]
    pub const fn new(ms: u64) -> Self {
        Self(ms)
    }

    #[inline]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    #[inline]
    pub const fn from_millis(ms: u64) -> Self {
        Self(ms)
    }

    #[inline]
    pub const fn try_from_seconds(seconds: u64) -> Option<Self> {
        match seconds.checked_mul(Self::MILLIS_PER_SECOND) {
            Some(ms) => Some(Self(ms)),
            None => None,
        }
    }

    #[inline]
    pub const fn from_seconds_saturating(seconds: u64) -> Self {
        Self(seconds.saturating_mul(Self::MILLIS_PER_SECOND))
    }

    #[inline]
    pub const fn as_seconds_floor(self) -> u64 {
        self.0 / Self::MILLIS_PER_SECOND
    }

    #[inline]
    pub const fn is_within(self, min: Self, max: Self) -> bool {
        if min.0 > max.0 {
            return false;
        }
        self.0 >= min.0 && self.0 <= max.0
    }

    #[inline]
    pub fn ensure_within(self, min: Self, max: Self) -> Result<Self, UnixMsRangeError> {
        if min.0 > max.0 {
            return Err(UnixMsRangeError::InvalidBounds { min, max });
        }
        if self.0 < min.0 {
            return Err(UnixMsRangeError::BelowMinimum { value: self, min });
        }
        if self.0 > max.0 {
            return Err(UnixMsRangeError::AboveMaximum { value: self, max });
        }
        Ok(self)
    }

    #[inline]
    pub fn try_new_with_bounds(ms: u64, min: Self, max: Self) -> Result<Self, UnixMsRangeError> {
        Self(ms).ensure_within(min, max)
    }

    #[inline]
    pub const fn checked_add(self, delta_ms: u64) -> Option<Self> {
        match self.0.checked_add(delta_ms) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }

    #[inline]
    pub const fn checked_sub(self, delta_ms: u64) -> Option<Self> {
        match self.0.checked_sub(delta_ms) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }

    #[inline]
    pub const fn saturating_add(self, delta_ms: u64) -> Self {
        Self(self.0.saturating_add(delta_ms))
    }

    #[inline]
    pub const fn saturating_sub(self, delta_ms: u64) -> Self {
        Self(self.0.saturating_sub(delta_ms))
    }

    #[inline]
    pub fn saturating_add_signed(self, delta_ms: i64) -> Self {
        if delta_ms >= 0 {
            self.saturating_add(delta_ms as u64)
        } else {
            self.saturating_sub(delta_ms.unsigned_abs())
        }
    }

    #[inline]
    pub const fn saturating_diff(self, earlier: Self) -> u64 {
        self.0.saturating_sub(earlier.0)
    }
}

impl From<u64> for UnixMs {
    #[inline]
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<UnixMs> for u64 {
    #[inline]
    fn from(value: UnixMs) -> Self {
        value.0
    }
}

impl fmt::Display for UnixMs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
