use exchange::UnixMs;
use serde::{Deserialize, Serialize};

pub const DEFAULT_HORIZONTAL_SPACING_PX: f32 = 5.0;
pub const MIN_HORIZONTAL_SPACING_PX: f32 = 2.0;
pub const MAX_HORIZONTAL_SPACING_PX: f32 = 360.0;
pub const MAX_AUTOSCALE_SPAN_FACTOR: f64 = 1_000.0;
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

    pub fn clamp_span(self, min_span: f64, max_span: f64) -> Option<Self> {
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

    pub fn clamp_to_bounds(self, lower: f64, upper: f64) -> Option<Self> {
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
