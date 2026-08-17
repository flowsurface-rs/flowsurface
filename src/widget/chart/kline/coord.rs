use exchange::UnixMs;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ChartStepMs(i64);

impl ChartStepMs {
    pub fn from_u64(step: u64) -> Self {
        let clamped = i64::try_from(step).unwrap_or(i64::MAX);
        Self(clamped.max(1))
    }

    pub fn get(self) -> i64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ChartCoord(i64);

impl ChartCoord {
    pub fn from_u64_clamped(value: u64) -> Self {
        Self(i64::try_from(value).unwrap_or(i64::MAX))
    }

    pub fn from_unix_ms(value: UnixMs) -> Self {
        Self::from_u64_clamped(value.as_u64())
    }

    pub fn get(self) -> i64 {
        self.0
    }

    pub fn saturating_add_i64(self, rhs: i64) -> Self {
        Self(self.0.saturating_add(rhs))
    }

    pub fn saturating_sub_i64(self, rhs: i64) -> Self {
        Self(self.0.saturating_sub(rhs))
    }

    pub fn to_unix_ms_non_negative(self) -> UnixMs {
        UnixMs::new(u64::try_from(self.0.max(0)).unwrap_or(u64::MAX))
    }
}
