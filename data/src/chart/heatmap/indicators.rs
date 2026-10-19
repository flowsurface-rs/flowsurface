use super::{HistoricalDepth, ProfileKind};
use crate::chart::kline::{
    composition::{PanelValueLabelMode, PanelValueLabelPolicy, PanelValuePrecision},
    viewport::PanelValueScale,
};
use enum_map::{Enum, EnumMap};
use exchange::{
    TickerInfo, Timeframe, UnixMs,
    depth::Depth,
    unit::{MinTicksize, Power10, Price, PriceStep, Qty},
};
use serde::{Deserialize, Deserializer, Serialize};
use std::{
    collections::BTreeMap,
    ops::{Bound, ControlFlow},
};

const MIN_PANEL_RATIO: f32 = 0.08;
const DEFAULT_PANEL_WEIGHTS: [f32; 3] = [70.0, 15.0, 15.0];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeatmapIndicator {
    Volume,
    Spread,
    Depth,
    VolumeProfile,
}

impl HeatmapIndicator {
    pub const ALL: [Self; 4] = [Self::Volume, Self::VolumeProfile, Self::Spread, Self::Depth];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Volume => "Volume",
            Self::Spread => "Spread",
            Self::Depth => "Depth",
            Self::VolumeProfile => "Volume Profile",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum HeatmapPanel {
    Spread,
    Depth,
}

impl HeatmapPanel {
    pub const ALL: [Self; 2] = [Self::Spread, Self::Depth];

    fn weight_index(self) -> usize {
        match self {
            Self::Spread => 1,
            Self::Depth => 2,
        }
    }

    pub fn value_scale(
        self,
        config: &HeatmapIndicatorConfig,
        ticker: TickerInfo,
    ) -> PanelValueScale {
        let precision = match self {
            Self::Spread => match config.spread_unit {
                SpreadUnit::Bps => PanelValuePrecision::FixedStep(0.001),
                SpreadUnit::Ticks => PanelValuePrecision::FixedStep(1.0),
            },
            Self::Depth => PanelValuePrecision::FixedPower10(Power10::new(-Qty::QTY_SCALE as i8)),
        };
        PanelValueScale::new(Some(precision), Some(ticker))
    }

    pub fn value_label_policy(self) -> PanelValueLabelPolicy {
        let mode = match self {
            Self::Spread => PanelValueLabelMode::Compact,
            Self::Depth => PanelValueLabelMode::Abbreviated,
        };
        PanelValueLabelPolicy {
            axis_mode: mode,
            header_mode: mode,
            max_decimals: None,
        }
    }
}

impl From<HeatmapPanel> for HeatmapIndicator {
    fn from(panel: HeatmapPanel) -> Self {
        match panel {
            HeatmapPanel::Spread => Self::Spread,
            HeatmapPanel::Depth => Self::Depth,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpreadUnit {
    #[default]
    Bps,
    Ticks,
}

impl std::fmt::Display for SpreadUnit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Bps => "Bps",
            Self::Ticks => "Ticks",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct DepthDistance(u16);

impl Default for DepthDistance {
    fn default() -> Self {
        Self(10)
    }
}

// Persist basis points so saved layouts retain their percentages.
impl TryFrom<f64> for DepthDistance {
    type Error = &'static str;

    fn try_from(basis_points: f64) -> Result<Self, Self::Error> {
        if !basis_points.is_finite() || !(1.0..=1_000.0).contains(&basis_points) {
            return Err("Depth distance must be between 0.01% and 10%");
        }
        Ok(Self(basis_points.round() as u16))
    }
}

impl From<DepthDistance> for f64 {
    fn from(value: DepthDistance) -> Self {
        f64::from(value.0)
    }
}

impl DepthDistance {
    pub fn from_percent(input: &str) -> Option<Self> {
        let percent = input.trim().parse::<f64>().ok()?;
        Self::try_from(percent * 100.0).ok()
    }

    pub fn percent(self) -> f64 {
        f64::from(self.0) / 100.0
    }

    fn bounds(self, bid: Price, ask: Price) -> (Price, Price) {
        let distance = i128::from(self.0);
        let lower = (i128::from(bid.units) * (10_000 - distance) + 9_999) / 10_000;
        let upper = i128::from(ask.units) * (10_000 + distance) / 10_000;
        (
            Price::from_units(lower as i64),
            Price::from_units(upper.min(i128::from(i64::MAX)) as i64),
        )
    }
}

impl std::fmt::Display for DepthDistance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.2}", self.percent())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HeatmapIndicatorConfig {
    pub volume: bool,
    pub volume_profile: Option<ProfileKind>,
    pub spread: bool,
    pub spread_unit: SpreadUnit,
    pub depth: bool,
    pub distance: DepthDistance,
    #[serde(deserialize_with = "deserialize_panel_weights")]
    panel_weights: [f32; 3],
}

impl Default for HeatmapIndicatorConfig {
    fn default() -> Self {
        Self {
            volume: true,
            volume_profile: Some(ProfileKind::default()),
            spread: false,
            spread_unit: SpreadUnit::default(),
            depth: false,
            distance: DepthDistance::default(),
            panel_weights: DEFAULT_PANEL_WEIGHTS,
        }
    }
}

impl HeatmapIndicatorConfig {
    fn accepts_depth_replay(&self, distance: DepthDistance) -> bool {
        self.depth && self.distance == distance
    }

    pub fn is_enabled(&self, kind: HeatmapIndicator) -> bool {
        match kind {
            HeatmapIndicator::Volume => self.volume,
            HeatmapIndicator::Spread => self.spread,
            HeatmapIndicator::Depth => self.depth,
            HeatmapIndicator::VolumeProfile => self.volume_profile.is_some(),
        }
    }

    pub fn set_enabled(&mut self, kind: HeatmapIndicator, enabled: bool) {
        match kind {
            HeatmapIndicator::Volume => self.volume = enabled,
            HeatmapIndicator::Spread => self.spread = enabled,
            HeatmapIndicator::Depth => self.depth = enabled,
            HeatmapIndicator::VolumeProfile => {
                self.volume_profile = enabled.then(|| self.volume_profile.unwrap_or_default());
            }
        }
    }

    pub fn enabled_panels(&self) -> impl Iterator<Item = HeatmapPanel> + '_ {
        HeatmapPanel::ALL
            .into_iter()
            .filter(|kind| self.is_enabled((*kind).into()))
    }

    fn panel_indices(&self) -> Vec<usize> {
        std::iter::once(0)
            .chain(self.enabled_panels().map(HeatmapPanel::weight_index))
            .collect()
    }

    pub fn normalized_weights(&self) -> Vec<f32> {
        let indices = self.panel_indices();
        let weights: Vec<_> = indices
            .iter()
            .map(|&index| self.panel_weights[index])
            .collect();

        let total: f64 = weights.iter().map(|weight| f64::from(*weight)).sum();
        let mut cumulative = 0.0;
        let mut previous = 0.0;
        let mut normalized = Vec::with_capacity(weights.len());
        for (index, weight) in weights.iter().enumerate() {
            cumulative += (f64::from(*weight) / total) as f32;
            let boundary = if index + 1 == weights.len() {
                1.0
            } else {
                cumulative.clamp(
                    previous + MIN_PANEL_RATIO,
                    1.0 - MIN_PANEL_RATIO * (weights.len() - index - 1) as f32,
                )
            };
            normalized.push(boundary - previous);
            previous = boundary;
        }
        normalized
    }

    pub fn set_split(&mut self, index: usize, split: f32) {
        if !split.is_finite() {
            return;
        }
        let indices = self.panel_indices();
        if index + 1 >= indices.len() {
            return;
        }

        let mut weights = self.normalized_weights();
        let before: f32 = weights[..index].iter().sum();
        let combined = weights[index] + weights[index + 1];
        let first = (split - before).clamp(
            MIN_PANEL_RATIO,
            (combined - MIN_PANEL_RATIO).max(MIN_PANEL_RATIO),
        );
        weights[index] = first;
        weights[index + 1] = combined - first;
        let all_weights = self.panel_weights.map(f64::from);

        let total: f64 = all_weights.iter().sum();
        let active_total: f64 = indices
            .iter()
            .map(|&index| all_weights[index] / total)
            .sum();
        let mut panel_weights = all_weights.map(|weight| (weight / total) as f32);
        for (&slot, weight) in indices.iter().zip(weights) {
            panel_weights[slot] = (f64::from(weight) * active_total) as f32;
        }
        self.panel_weights = sanitize_panel_weights(panel_weights);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepthCoverage {
    Unknown,
    ReachesBounds,
    FallsShort,
}

#[derive(Clone, Copy)]
pub enum DepthSource {
    Live,
    Reconstructed,
}

#[derive(Clone, Copy)]
enum IndicatorValue {
    Spread {
        bid: Price,
        ask: Price,
    },
    Depth {
        bid: Qty,
        ask: Qty,
        coverage: DepthCoverage,
    },
}

impl IndicatorValue {
    fn percentage_depth(
        depth: &Depth,
        (bid, ask): (Price, Price),
        step: PriceStep,
        distance: DepthDistance,
        source: DepthSource,
    ) -> Option<Self> {
        let bid = bid.round_to_side_step(true, step);
        let ask = ask.round_to_side_step(false, step);
        if bid.units <= 0 {
            return None;
        }

        let (lower, upper) = distance.bounds(bid, ask);
        let sum = |side: &BTreeMap<Price, Qty>, is_bid, lower: Price, upper: Price| {
            let raw_lower = Price::from_units(lower.units.saturating_sub(step.units));
            let raw_upper = Price::from_units(upper.units.saturating_add(step.units));

            side.range(raw_lower..=raw_upper)
                .filter(|(price, qty)| {
                    let price = price.round_to_side_step(is_bid, step);
                    **qty > Qty::ZERO && price >= lower && price <= upper
                })
                .try_fold(0i64, |sum, (_, qty)| sum.checked_add(qty.units))
                .map(Qty::from_units)
        };

        let coverage = match source {
            DepthSource::Reconstructed => DepthCoverage::Unknown,
            DepthSource::Live => {
                let bid_reaches_bound = depth
                    .bids
                    .iter()
                    .find(|(_, qty)| **qty > Qty::ZERO)
                    .is_some_and(|(price, _)| price.round_to_side_step(true, step) <= lower);
                let ask_reaches_bound = depth
                    .asks
                    .iter()
                    .rev()
                    .find(|(_, qty)| **qty > Qty::ZERO)
                    .is_some_and(|(price, _)| price.round_to_side_step(false, step) >= upper);

                if bid_reaches_bound && ask_reaches_bound {
                    DepthCoverage::ReachesBounds
                } else {
                    DepthCoverage::FallsShort
                }
            }
        };

        Some(Self::Depth {
            bid: sum(&depth.bids, true, lower, bid)?,
            ask: sum(&depth.asks, false, ask, upper)?,
            coverage,
        })
    }
}

struct Sample {
    timestamp: UnixMs,
    value: Option<IndicatorValue>,
}

impl Sample {
    fn from_depth(
        kind: HeatmapPanel,
        depth: &Depth,
        timestamp: UnixMs,
        step: PriceStep,
        distance: DepthDistance,
        source: DepthSource,
    ) -> Self {
        let best_prices = depth
            .bids
            .iter()
            .rev()
            .find(|(_, qty)| **qty > Qty::ZERO)
            .zip(depth.asks.iter().find(|(_, qty)| **qty > Qty::ZERO))
            .map(|((&bid, _), (&ask, _))| (bid, ask))
            .filter(|(bid, ask)| bid.units > 0 && *ask >= *bid);

        let value = best_prices.and_then(|(bid, ask)| match kind {
            HeatmapPanel::Spread => Some(IndicatorValue::Spread { bid, ask }),
            HeatmapPanel::Depth => {
                IndicatorValue::percentage_depth(depth, (bid, ask), step, distance, source)
            }
        });

        Self { timestamp, value }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum IndicatorValues {
    Spread(f64),
    Depth { bid: f64, ask: f64 },
}

#[derive(Clone)]
pub struct HeatmapReplay {
    distance: DepthDistance,
    depth_timestamp: Option<UnixMs>,
}

pub struct DepthReplayResult {
    distance: DepthDistance,
    samples: BTreeMap<UnixMs, Sample>,
}

impl HeatmapReplay {
    pub fn matches(&self, config: &HeatmapIndicatorConfig) -> bool {
        config.accepts_depth_replay(self.distance)
    }

    pub fn include(&mut self, previous: &Self) {
        self.depth_timestamp = self.depth_timestamp.max(previous.depth_timestamp);
    }

    pub fn run(
        self,
        history: &HistoricalDepth,
        start: UnixMs,
        control: impl FnMut() -> ControlFlow<()>,
    ) -> ControlFlow<(), DepthReplayResult> {
        let mut samples = BTreeMap::new();
        history.replay_depth_interruptible(
            start,
            |time, depth| {
                let sample = Sample::from_depth(
                    HeatmapPanel::Depth,
                    depth,
                    time,
                    history.tick_size,
                    self.distance,
                    DepthSource::Reconstructed,
                );
                samples.insert(time.floor_to(history.aggr_time), sample);
            },
            control,
        )?;

        if let Some(time) = self.depth_timestamp
            && let Some(mut latest) = samples.last_entry()
        {
            latest.get_mut().timestamp = latest.get().timestamp.max(time);
        }

        ControlFlow::Continue(DepthReplayResult {
            distance: self.distance,
            samples,
        })
    }
}

pub struct HeatmapIndicators {
    config: HeatmapIndicatorConfig,
    samples: EnumMap<HeatmapPanel, BTreeMap<UnixMs, Sample>>,
    revision: u64,
}

impl HeatmapIndicators {
    pub fn new(config: HeatmapIndicatorConfig) -> Self {
        Self {
            config,
            revision: 0,
            samples: EnumMap::default(),
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn config(&self) -> &HeatmapIndicatorConfig {
        &self.config
    }

    pub fn set_panel_split(&mut self, index: usize, split: f32) {
        self.config.set_split(index, split);
    }

    pub fn reconfigure(&mut self, config: HeatmapIndicatorConfig) -> Option<HeatmapReplay> {
        let depth_distance_changed = self.config.distance != config.distance;
        let replay = (config.depth && (!self.config.depth || depth_distance_changed)).then(|| {
            HeatmapReplay {
                distance: config.distance,
                depth_timestamp: self.samples[HeatmapPanel::Depth]
                    .last_key_value()
                    .map(|(_, sample)| sample.timestamp),
            }
        });

        let depth_changed = self.config.depth != config.depth || depth_distance_changed;
        let values_changed = depth_changed
            || self.config.spread != config.spread
            || self.config.spread_unit != config.spread_unit;
        if depth_changed {
            self.samples[HeatmapPanel::Depth].clear();
        }

        self.config = config;
        if values_changed {
            self.revision = self.revision.wrapping_add(1);
        }

        replay
    }

    pub fn merge_replay(&mut self, replay: DepthReplayResult, cutoff: UnixMs) -> bool {
        if !self.config.accepts_depth_replay(replay.distance) {
            return false;
        }

        for (time, sample) in replay.samples {
            self.samples[HeatmapPanel::Depth]
                .entry(time)
                .or_insert(sample);
        }

        self.prune_before(cutoff);
        true
    }

    pub fn ingest(
        &mut self,
        kind: HeatmapPanel,
        depth: &Depth,
        timestamp: UnixMs,
        interval: Timeframe,
        source: DepthSource,
        step: PriceStep,
    ) {
        if kind == HeatmapPanel::Depth && !self.config.depth {
            return;
        }

        let samples = &mut self.samples[kind];
        if samples
            .last_key_value()
            .is_some_and(|(_, sample)| sample.timestamp > timestamp)
        {
            return;
        }

        let sample = Sample::from_depth(kind, depth, timestamp, step, self.config.distance, source);
        samples.insert(timestamp.floor_to(interval), sample);
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn prune_before(&mut self, cutoff: UnixMs) {
        self.revision = self.revision.wrapping_add(1);
        for samples in self.samples.values_mut() {
            let mut retained = samples.split_off(&cutoff);
            if !retained.contains_key(&cutoff)
                && let Some((_, sample)) = samples.pop_last()
            {
                retained.insert(cutoff, sample);
            }
            *samples = retained;
        }
    }

    pub fn samples(
        &self,
        kind: HeatmapPanel,
        min_ticksize: MinTicksize,
        start: UnixMs,
        end: UnixMs,
        interval_ms: u64,
    ) -> impl Iterator<Item = (UnixMs, Option<IndicatorValues>)> + '_ {
        let interval_ms = interval_ms.max(1);
        let start = start.as_u64() / interval_ms * interval_ms;
        let end = end.as_u64();
        let samples = &self.samples[kind];

        let mut current = samples.range(..=UnixMs::new(start)).next_back();
        let mut updates = samples
            .range((Bound::Excluded(UnixMs::new(start)), Bound::Unbounded))
            .peekable();

        // Project the last observed book onto chart buckets without inventing source timestamps.
        std::iter::successors((start <= end).then_some(start), move |time| {
            time.checked_add(interval_ms).filter(|next| *next <= end)
        })
        .map(move |time| {
            let time = UnixMs::new(time);
            while updates.peek().is_some_and(|(update, _)| **update <= time) {
                current = updates.next();
            }
            (
                time,
                current.and_then(|(_, sample)| self.project(sample, min_ticksize)),
            )
        })
    }

    pub fn value_at(
        &self,
        kind: HeatmapPanel,
        min_ticksize: MinTicksize,
        time: UnixMs,
    ) -> Option<IndicatorValues> {
        self.samples[kind]
            .range(..=time)
            .next_back()
            .and_then(|(_, sample)| self.project(sample, min_ticksize))
    }

    pub fn depth_coverage_at(&self, time: UnixMs) -> Option<DepthCoverage> {
        let (_, sample) = self.samples[HeatmapPanel::Depth]
            .range(..=time)
            .next_back()?;
        match sample.value? {
            IndicatorValue::Depth { coverage, .. } => Some(coverage),
            IndicatorValue::Spread { .. } => None,
        }
    }

    fn project(&self, sample: &Sample, min_ticksize: MinTicksize) -> Option<IndicatorValues> {
        match sample.value? {
            IndicatorValue::Spread { bid, ask } => {
                let gap = (ask.units - bid.units) as f64;
                Some(IndicatorValues::Spread(match self.config.spread_unit {
                    SpreadUnit::Bps => 20_000.0 * gap / (bid.units as f64 + ask.units as f64),
                    SpreadUnit::Ticks => gap / Price::min_tick_units(min_ticksize) as f64,
                }))
            }
            IndicatorValue::Depth { bid, ask, .. } => Some(IndicatorValues::Depth {
                bid: bid.to_f64(),
                ask: ask.to_f64(),
            }),
        }
    }
}

fn sanitize_panel_weights(weights: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|index| {
        let weight = weights[index];
        if weight.is_finite() && weight > 0.0 {
            weight
        } else {
            DEFAULT_PANEL_WEIGHTS[index]
        }
    })
}

fn deserialize_panel_weights<'de, D>(deserializer: D) -> Result<[f32; 3], D::Error>
where
    D: Deserializer<'de>,
{
    <[f32; 3]>::deserialize(deserializer).map(sanitize_panel_weights)
}
