use super::composition::PanelValuePrecision;
use exchange::{TickerInfo, Timeframe, UnixMs, unit::Price};

use serde::{Deserialize, Serialize};
use std::ops::Range;

pub const Y_UNIT_STEP_FALLBACK: f32 = 1e-4;
pub const DEFAULT_HORIZONTAL_SPACING_PX: f32 = 5.0;
pub const MIN_HORIZONTAL_SPACING_PX: f32 = 2.0;
pub const MAX_HORIZONTAL_SPACING_PX: f32 = 360.0;
const MAX_AUTOSCALE_SPAN_FACTOR: f64 = 1_000.0;
pub const ZOOM_SENSITIVITY: f32 = 30.0;

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

    pub fn min(self) -> YUnit {
        self.min
    }

    pub fn max(self) -> YUnit {
        self.max
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelYValueRange {
    min: f64,
    max: f64,
}

impl PanelYValueRange {
    pub fn normalized(min_value: f64, max_value: f64) -> Option<Self> {
        if !min_value.is_finite() || !max_value.is_finite() {
            return None;
        }
        let (mut min_value, mut max_value) = if min_value <= max_value {
            (min_value, max_value)
        } else {
            (max_value, min_value)
        };
        if (max_value - min_value).abs() <= 1e-8 {
            let pad = min_value.abs().max(1.0) * 0.01;
            min_value -= pad;
            max_value += pad;
        }
        if !min_value.is_finite() || !max_value.is_finite() {
            return None;
        }
        Some(Self {
            min: min_value,
            max: max_value,
        })
    }

    pub fn min_value(self) -> f64 {
        self.min
    }

    pub fn max_value(self) -> f64 {
        self.max
    }

    pub fn span(self) -> f64 {
        (self.max - self.min).abs()
    }

    pub fn zoom_around_ratio(
        self,
        anchor_ratio: f64,
        zoom_scale: f64,
        zoom_in: bool,
    ) -> Option<Self> {
        if !zoom_scale.is_finite() {
            return None;
        }
        let anchor_ratio = anchor_ratio.clamp(0.0, 1.0);
        let span = self.span().max(1e-8);
        let zoom_scale = zoom_scale.abs().max(1.0);
        let new_span = if zoom_in {
            span / zoom_scale
        } else if span <= f64::MAX / zoom_scale {
            span * zoom_scale
        } else {
            f64::MAX
        };
        if !new_span.is_finite() {
            return None;
        }
        let anchor_value = self.min + anchor_ratio * span;
        let new_min = anchor_value - anchor_ratio * new_span;
        Self::normalized(new_min, new_min + new_span)
    }

    fn clamp_span(self, min_span: f64, max_span: f64) -> Option<Self> {
        if !self.min.is_finite()
            || !self.max.is_finite()
            || !min_span.is_finite()
            || !max_span.is_finite()
        {
            return None;
        }

        let min_span = min_span.abs().max(1e-8);
        let max_span = max_span.abs().max(min_span);
        let span = self.span();
        let target_span = span.clamp(min_span, max_span);
        let center = self.min * 0.5 + self.max * 0.5;
        let min_value = center - target_span * 0.5;
        Self::normalized(min_value, min_value + target_span)
    }

    fn clamp_to_bounds(self, lower: f64, upper: f64) -> Option<Self> {
        if !lower.is_finite() || !upper.is_finite() || lower >= upper {
            return None;
        }

        let span = self.span().min(upper - lower);
        let mut min_value = self.min.max(lower);
        let mut max_value = min_value + span;
        if max_value > upper {
            max_value = upper;
            min_value = max_value - span;
        }

        Self::normalized(min_value, max_value)
    }

    pub fn pan_by_ratio(self, delta_ratio: f64) -> Option<Self> {
        if !delta_ratio.is_finite() {
            return None;
        }
        let span = self.span().max(1e-8);
        let shift = delta_ratio * span;
        Self::normalized(self.min + shift, self.max + shift)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HorizontalWorld {
    spacing: f32,
    offset: f32,
}

impl HorizontalWorld {
    pub fn visible_window(self, data_bounds: (i64, i64), plot_width: f32) -> (i64, i64, f32) {
        let (data_min_x, data_max_x) = data_bounds;

        let span = self.spacing();
        let visible_units = (plot_width.max(1.0) / span).max(1.0);
        let right_cap = data_max_x as f32 + visible_units;
        let right_floor = data_min_x as f32;
        let right = (data_max_x as f32 + self.offset()).clamp(right_floor, right_cap);
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

#[derive(Debug, Clone, Copy)]
pub struct PanelYViewportLimits {
    pub min_span: f64,
    pub max_span: f64,
    pub bounds: (f64, f64),
}

impl PanelYViewportLimits {
    pub fn apply(self, viewport: PanelYValueRange) -> Option<PanelYValueRange> {
        let viewport = viewport.clamp_span(self.min_span, self.max_span)?;
        viewport.clamp_to_bounds(self.bounds.0, self.bounds.1)
    }
}

pub fn autoscale_span_limits(
    autoscale_span: f64,
    physical_height: f64,
    min_span: f64,
) -> Option<(f64, f64)> {
    if autoscale_span.is_nan()
        || autoscale_span < 0.0
        || !physical_height.is_finite()
        || physical_height <= 0.0
        || !min_span.is_finite()
    {
        return None;
    }

    let min_span = min_span.abs().max(1e-8);
    let factor = physical_height.clamp(1.0, MAX_AUTOSCALE_SPAN_FACTOR);
    let autoscale_span = autoscale_span.min(f64::MAX).max(min_span);
    let max_span = if autoscale_span > f64::MAX / factor {
        f64::MAX
    } else {
        autoscale_span * factor
    };
    Some((min_span, max_span.max(min_span)))
}

#[derive(Debug, Clone, Copy)]
pub struct PanelValueScale {
    precision: Option<PanelValuePrecision>,
    base_ticker: Option<TickerInfo>,
}

impl PanelValueScale {
    fn round_to_i64_saturating(value: f32) -> i64 {
        if value.is_nan() {
            return 0;
        }
        if value.is_infinite() {
            return if value.is_sign_negative() {
                i64::MIN
            } else {
                i64::MAX
            };
        }

        let rounded = value.round();
        if rounded > i64::MAX as f32 {
            i64::MAX
        } else if rounded < i64::MIN as f32 {
            i64::MIN
        } else {
            rounded as i64
        }
    }

    fn fallback_value_to_unit_f32(value: f32) -> YUnit {
        if !value.is_finite() {
            return YUnit(0);
        }

        YUnit(Self::round_to_i64_saturating(value / Y_UNIT_STEP_FALLBACK))
    }

    fn fallback_value_to_unit(value: f64) -> YUnit {
        if !value.is_finite() {
            return YUnit(0);
        }

        let rounded = (value / f64::from(Y_UNIT_STEP_FALLBACK)).round();
        let unit = if rounded > i64::MAX as f64 {
            i64::MAX
        } else if rounded < i64::MIN as f64 {
            i64::MIN
        } else {
            rounded as i64
        };
        YUnit(unit)
    }

    pub fn new(precision: Option<PanelValuePrecision>, base_ticker: Option<TickerInfo>) -> Self {
        Self {
            precision,
            base_ticker,
        }
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
