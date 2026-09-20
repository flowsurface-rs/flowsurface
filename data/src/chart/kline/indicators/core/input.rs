use exchange::UnixMs;
use std::collections::BTreeMap;

/// Provider-timestamped, finite Open Interest values owned by `KlineSeries`.
///
/// The concrete model deliberately has no provider or field identity. Open
/// Interest is the only fetch-backed input in the Kline indicator runtime, so
/// a sorted map is enough to support viewport merging and as-of bar lookup.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OpenInterestSeries {
    pub values: BTreeMap<UnixMs, f64>,
}

impl OpenInterestSeries {
    pub fn from_points(points: impl IntoIterator<Item = (UnixMs, f64)>) -> Self {
        let mut series = Self::default();
        series.extend(points);
        series
    }

    pub fn extend(&mut self, points: impl IntoIterator<Item = (UnixMs, f64)>) {
        for (time, value) in points {
            if value.is_finite() {
                self.values.insert(time, value);
            }
        }
    }

    pub fn merge(&mut self, other: &Self) {
        self.extend(other.values.iter().map(|(&time, &value)| (time, value)));
    }

    pub fn value_at(&self, timestamp: UnixMs) -> Option<f64> {
        self.values.get(&timestamp).copied()
    }

    pub fn latest_at_or_before(&self, timestamp: UnixMs) -> Option<f64> {
        self.values
            .range(..=timestamp)
            .next_back()
            .map(|(_, value)| *value)
    }

    pub fn covered_range(&self) -> Option<(UnixMs, UnixMs)> {
        Some((
            *self.values.first_key_value()?.0,
            *self.values.last_key_value()?.0,
        ))
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IndicatorInputSource {
    Klines,
    DirectionalVolume,
    Trades,
    OpenInterest,
}

impl IndicatorInputSource {
    pub const fn id(self) -> &'static str {
        match self {
            Self::Klines => "klines",
            Self::DirectionalVolume => "directional_volume",
            Self::Trades => "trades",
            Self::OpenInterest => "open_interest",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndicatorInputRequirement {
    pub source: IndicatorInputSource,
    required: bool,
}

impl IndicatorInputRequirement {
    pub const fn required(source: IndicatorInputSource) -> Self {
        Self {
            source,
            required: true,
        }
    }

    pub const fn optional(source: IndicatorInputSource) -> Self {
        Self {
            source,
            required: false,
        }
    }

    pub const fn is_required(self) -> bool {
        self.required
    }

    pub const fn id(self) -> &'static str {
        self.source.id()
    }
}
