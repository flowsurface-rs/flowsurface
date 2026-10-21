use super::series::KlineSeries;
use crate::chart::Basis;
pub use crate::chart::axis::{
    PanelYValueRange, PanelYViewportLimits, ZOOM_SENSITIVITY, autoscale_span_limits,
};
pub use crate::chart::value::{PanelValueScale, Y_UNIT_STEP_FALLBACK, YUnit, YUnitRange};
use exchange::{Timeframe, UnixMs};

use std::ops::Range;

pub const DEFAULT_HORIZONTAL_SPACING_PX: f32 = 5.0;
pub const MIN_HORIZONTAL_SPACING_PX: f32 = 2.0;
pub const MAX_HORIZONTAL_SPACING_PX: f32 = 360.0;

const BAR_CENTER_OFFSET_UNITS: f32 = 0.5;
const RIGHT_EDGE_INSET_PX: f32 = 1.0;

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
    fn from_u64_clamped(value: u64) -> Self {
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HorizontalWorld {
    spacing: f32,
    offset: f32,
}

impl HorizontalWorld {
    pub fn visible_window(self, data_bounds: (i64, i64), plot_width: f32) -> (i64, i64, f32) {
        let (_, data_max_x) = data_bounds;

        let span = self.spacing();
        let visible_units = (plot_width.max(1.0) / span).max(1.0);
        let right = data_max_x as f32 + self.offset();
        let left = right - visible_units + 1.0;
        let left_unit = left.floor() as i64;
        let right_unit = right.ceil() as i64;

        if right_unit <= left_unit {
            return (left_unit, left_unit.saturating_add(1), right);
        }

        (left_unit, right_unit, right)
    }

    pub fn new(spacing: f32, offset: f32) -> Self {
        let spacing = Self::normalized_spacing(spacing);

        Self { spacing, offset }
    }

    fn normalized_spacing(spacing: f32) -> f32 {
        if spacing.is_finite() {
            spacing.clamp(MIN_HORIZONTAL_SPACING_PX, MAX_HORIZONTAL_SPACING_PX)
        } else {
            DEFAULT_HORIZONTAL_SPACING_PX
        }
    }

    pub fn spacing(self) -> f32 {
        self.spacing
    }

    pub fn offset(self) -> f32 {
        self.offset
    }

    pub fn with_offset(self, offset: f32) -> Self {
        Self { offset, ..self }
    }

    pub fn axis_at_right_unit(self, right_unit: f32) -> WorldAxis {
        WorldAxis::new(
            self.spacing,
            -(right_unit + BAR_CENTER_OFFSET_UNITS) * self.spacing - RIGHT_EDGE_INSET_PX,
        )
    }

    pub fn zoom_around_screen(
        self,
        current_right_unit: f32,
        cursor_screen: f32,
        origin: f32,
        delta: f32,
    ) -> Self {
        let axis = self.axis_at_right_unit(current_right_unit);
        let spacing = (self.spacing * (delta / ZOOM_SENSITIVITY).exp())
            .clamp(MIN_HORIZONTAL_SPACING_PX, MAX_HORIZONTAL_SPACING_PX);
        let cursor_world = axis.screen_to_world(cursor_screen, origin);
        let translation = cursor_screen - origin - cursor_world * spacing;
        let next_right_unit =
            (-translation - RIGHT_EDGE_INSET_PX) / spacing - BAR_CENTER_OFFSET_UNITS;

        Self {
            spacing,
            offset: self.offset + next_right_unit - current_right_unit,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldAxis {
    spacing: f32,
    translation: f32,
}

impl WorldAxis {
    fn new(spacing: f32, translation: f32) -> Self {
        Self {
            spacing: spacing.max(f32::EPSILON),
            translation,
        }
    }

    pub fn spacing(self) -> f32 {
        self.spacing
    }

    pub fn world_to_screen(self, world: f32, origin: f32) -> f32 {
        origin + world * self.spacing + self.translation
    }

    pub fn screen_to_world(self, screen: f32, origin: f32) -> f32 {
        (screen - origin - self.translation) / self.spacing
    }

    pub fn screen_to_logical_index(self, screen: f32, origin: f32) -> f32 {
        self.screen_to_world(screen, origin) - BAR_CENTER_OFFSET_UNITS
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TickIndex(u64);

impl TickIndex {
    pub const ZERO: Self = Self(0);

    #[inline]
    pub fn as_u64(self) -> u64 {
        self.0
    }

    pub fn from_u64(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum XAxis {
    Time {
        timeframe: Timeframe,
        anchor: UnixMs,
    },
    Tick {
        anchor: TickIndex,
    },
}

impl XAxis {
    pub fn for_series(basis: Basis, series: &[KlineSeries]) -> Option<Self> {
        match basis {
            Basis::Time(timeframe) => {
                let anchor = series
                    .iter()
                    .filter_map(KlineSeries::latest_point)
                    .map(|point| point.kline.time)
                    .max()?;
                Some(Self::Time { timeframe, anchor })
            }
            Basis::Tick(_) => series
                .iter()
                .any(|series| series.point_count() > 0)
                .then_some(Self::Tick {
                    anchor: TickIndex::ZERO,
                }),
        }
    }

    pub fn data_bounds(self, series: &[KlineSeries]) -> Option<(i64, i64)> {
        match self {
            Self::Time { .. } => series
                .iter()
                .filter_map(|series| {
                    let first = series.first_point()?.kline.time;
                    let last = series.latest_point()?.kline.time;
                    Some((self.unit_from_time(first), self.unit_from_time(last)))
                })
                .reduce(|(min, max), (first, last)| (min.min(first), max.max(last))),
            Self::Tick { .. } => {
                let point_count = series.iter().map(KlineSeries::point_count).max()?;
                let last = self.unit_from_tick(TickIndex::ZERO);
                let first =
                    self.unit_from_tick(TickIndex::from_u64(point_count.saturating_sub(1) as u64));
                Some((first, last))
            }
        }
    }

    pub fn visible_window(
        self,
        data_bounds: (i64, i64),
        output_offset: i64,
        world: HorizontalWorld,
        plot_width: f32,
    ) -> (i64, i64, f32) {
        let (min_extension, max_extension) = self.output_domain_expansion(output_offset);
        world.visible_window(
            (
                data_bounds.0.saturating_add(min_extension),
                data_bounds.1.saturating_add(max_extension),
            ),
            plot_width,
        )
    }

    pub fn output_domain_expansion(self, offset: i64) -> (i64, i64) {
        match self {
            Self::Time { .. } => (0, offset),
            Self::Tick { .. } => (offset.saturating_neg(), 0),
        }
    }

    #[inline]
    pub fn unit_from_time(self, value: UnixMs) -> i64 {
        match self {
            Self::Time { timeframe, anchor } => {
                let aligned = ChartCoord::from_unix_ms(value.floor_to(timeframe)).get();
                let anchor = ChartCoord::from_unix_ms(anchor).get();
                let step = ChartStepMs::from_u64(timeframe.to_milliseconds().max(1)).get();

                aligned.saturating_sub(anchor) / step
            }
            Self::Tick { .. } => 0,
        }
    }

    pub fn unit_from_time_f32(self, value: UnixMs) -> Option<f32> {
        match self {
            Self::Time { timeframe, anchor } => {
                let delta_ms = value.as_u64() as f64 - anchor.as_u64() as f64;
                Some((delta_ms / timeframe.to_milliseconds().max(1) as f64) as f32)
            }
            Self::Tick { .. } => None,
        }
    }

    #[inline]
    pub fn unit_from_tick(self, value: TickIndex) -> i64 {
        match self {
            Self::Tick { anchor } => {
                let anchor = ChartCoord::from_u64_clamped(anchor.as_u64()).get();
                let index = ChartCoord::from_u64_clamped(value.as_u64()).get();

                anchor.saturating_sub(index)
            }
            Self::Time { .. } => 0,
        }
    }

    #[inline]
    pub fn time_from_unit(self, unit: i64) -> Option<UnixMs> {
        match self {
            Self::Time { timeframe, anchor } => {
                let step = i64::try_from(timeframe.to_milliseconds()).unwrap_or(i64::MAX);
                Some(anchor.saturating_add_signed(unit.saturating_mul(step)))
            }
            Self::Tick { .. } => None,
        }
    }

    pub fn time_from_unit_f32(self, unit: f32) -> Option<UnixMs> {
        if !unit.is_finite() {
            return None;
        }

        match self {
            Self::Time { timeframe, anchor } => {
                let offset = (f64::from(unit) * timeframe.to_milliseconds() as f64).round();
                let offset = offset.clamp(i64::MIN as f64, i64::MAX as f64) as i64;
                Some(anchor.saturating_add_signed(offset))
            }
            Self::Tick { .. } => None,
        }
    }

    #[inline]
    pub fn tick_from_unit(self, unit: i64) -> Option<TickIndex> {
        match self {
            Self::Tick { anchor } => {
                let anchor = anchor.as_u64();
                if unit >= 0 {
                    anchor.checked_sub(unit as u64).map(TickIndex)
                } else {
                    anchor.checked_add(unit.unsigned_abs()).map(TickIndex)
                }
            }
            Self::Time { .. } => None,
        }
    }

    #[inline]
    pub fn step_ms(self, step_units: i64) -> u64 {
        match self {
            Self::Time { timeframe, .. } => {
                let step = step_units.max(1) as u64;
                timeframe.to_milliseconds().max(1).saturating_mul(step)
            }
            Self::Tick { .. } => 1,
        }
    }

    pub fn bucket_start(self, unit: i64) -> Option<UnixMs> {
        let Self::Time { timeframe, .. } = self else {
            return None;
        };
        Some(self.time_from_unit(unit)?.floor_to(timeframe))
    }

    pub fn bucket_end(self, unit: i64) -> Option<UnixMs> {
        let Self::Time { timeframe, .. } = self else {
            return None;
        };
        Some(
            self.bucket_start(unit)?
                .saturating_add(timeframe.to_milliseconds().max(1).saturating_sub(1)),
        )
    }

    pub fn output_unit(self, source_unit: i64, offset_bars: i64) -> i64 {
        match self {
            Self::Time { .. } => source_unit.saturating_add(offset_bars),
            Self::Tick { .. } => source_unit.saturating_sub(offset_bars),
        }
    }

    pub fn source_unit(self, output_unit: i64, offset_bars: i64) -> i64 {
        match self {
            Self::Time { .. } => output_unit.saturating_sub(offset_bars),
            Self::Tick { .. } => output_unit.saturating_add(offset_bars),
        }
    }

    pub fn source_window(self, min: i64, max: i64, offsets: &[i64]) -> (i64, i64) {
        let mut source_min = min;
        let mut source_max = max;
        for &offset in offsets {
            source_min = source_min.min(self.source_unit(min, offset));
            source_max = source_max.max(self.source_unit(max, offset));
        }
        (source_min.saturating_sub(1), source_max.saturating_add(1))
    }
}

#[derive(Debug, Clone)]
pub struct BarUnitWindow {
    values: Vec<i64>,
    start_index: usize,
}

impl BarUnitWindow {
    pub(crate) fn from_units(values: Vec<i64>, start_index: usize) -> Self {
        Self {
            values,
            start_index,
        }
    }

    pub fn values(&self) -> &[i64] {
        &self.values
    }

    pub fn start_index(&self) -> usize {
        self.start_index
    }

    pub fn global_index(&self, local_index: usize) -> Option<usize> {
        self.start_index.checked_add(local_index)
    }

    pub fn global_range(&self, local_range: Range<usize>) -> Range<usize> {
        self.start_index.saturating_add(local_range.start)
            ..self.start_index.saturating_add(local_range.end)
    }

    pub fn unit_at_global_index(&self, index: usize) -> Option<i64> {
        index
            .checked_sub(self.start_index)
            .and_then(|local_index| self.values.get(local_index).copied())
    }

    pub fn index_range(&self, min: i64, max: i64) -> Range<usize> {
        self.values.partition_point(|&unit| unit < min)
            ..self.values.partition_point(|&unit| unit <= max)
    }

    pub fn output_range(
        &self,
        axis: XAxis,
        min: i64,
        max: i64,
        offset: i64,
        end: usize,
    ) -> Range<usize> {
        let values = &self.values[..end];
        values.partition_point(|&unit| axis.output_unit(unit, offset) < min)
            ..values.partition_point(|&unit| axis.output_unit(unit, offset) <= max)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimaryViewportState {
    Auto,
    Manual { range: Option<YUnitRange> },
}

impl PrimaryViewportState {
    pub fn is_auto(self) -> bool {
        matches!(self, Self::Auto)
    }

    pub fn range(self) -> Option<YUnitRange> {
        match self {
            Self::Auto => None,
            Self::Manual { range } => range,
        }
    }

    pub fn disable(&mut self) -> bool {
        if !self.is_auto() {
            return false;
        }

        *self = Self::Manual { range: None };
        true
    }

    pub fn set_range(&mut self, range: YUnitRange) -> bool {
        let next = Self::Manual { range: Some(range) };
        if *self == next {
            return false;
        }

        *self = next;
        true
    }

    pub fn reset(&mut self) -> bool {
        if matches!(self, Self::Auto) {
            return false;
        }

        *self = Self::Auto;
        true
    }

    pub fn toggle(&mut self) -> bool {
        if self.is_auto() {
            *self = Self::Manual { range: None };
        } else {
            *self = Self::Auto;
        }
        true
    }
}
