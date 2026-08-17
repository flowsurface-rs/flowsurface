pub const LEGACY_ZOOM_SENSITIVITY: f32 = 30.0;

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct UnitSpacing(f32);

impl UnitSpacing {
    pub fn new(size: f32) -> Self {
        Self(size.max(f32::EPSILON))
    }

    pub fn size(self) -> f32 {
        self.0
    }

    pub fn zoom(self, delta: f32, min_size: f32, max_size: f32) -> Self {
        let size = (self.0 * (1.0 + delta / LEGACY_ZOOM_SENSITIVITY))
            .clamp(min_size, max_size)
            .max(f32::EPSILON);
        Self::new(size)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HorizontalWorld {
    unit_spacing: UnitSpacing,
    scaling: f32,
    offset: f32,
}

impl HorizontalWorld {
    pub fn new(cell_width: f32, scaling: f32, offset: f32) -> Self {
        Self {
            unit_spacing: UnitSpacing::new(cell_width),
            scaling: scaling.max(f32::EPSILON),
            offset,
        }
    }

    pub fn cell_width(self) -> f32 {
        self.unit_spacing.size()
    }

    pub fn scaling(self) -> f32 {
        self.scaling
    }

    pub fn offset(self) -> f32 {
        self.offset
    }

    pub fn effective_spacing(self) -> f32 {
        (self.unit_spacing.size() * self.scaling).max(f32::EPSILON)
    }

    pub fn with_offset(self, offset: f32) -> Self {
        Self { offset, ..self }
    }

    pub fn axis_at_right_unit(self, right_unit: f32) -> WorldAxis {
        WorldAxis::new(
            self.unit_spacing,
            self.scaling,
            -right_unit * self.unit_spacing.size(),
        )
    }

    pub fn updated_from_axis(self, current_right_unit: f32, next: WorldAxis) -> Self {
        let next_right_unit = -next.translation() / next.unit_spacing().size();
        Self {
            unit_spacing: next.unit_spacing(),
            scaling: next.scaling(),
            offset: self.offset + next_right_unit - current_right_unit,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldAxis {
    unit_spacing: UnitSpacing,
    scaling: f32,
    translation: f32,
}

impl WorldAxis {
    pub fn new(unit_spacing: UnitSpacing, scaling: f32, translation: f32) -> Self {
        Self {
            unit_spacing,
            scaling: scaling.max(f32::EPSILON),
            translation,
        }
    }

    pub fn unit_spacing(self) -> UnitSpacing {
        self.unit_spacing
    }

    pub fn scaling(self) -> f32 {
        self.scaling
    }

    pub fn translation(self) -> f32 {
        self.translation
    }

    pub fn world_to_screen(self, world: f32, origin: f32) -> f32 {
        origin + (world * self.unit_spacing.size() + self.translation) * self.scaling
    }

    pub fn screen_to_world(self, screen: f32, origin: f32) -> f32 {
        ((screen - origin) / self.scaling - self.translation) / self.unit_spacing.size()
    }

    pub fn with_translation(self, translation: f32) -> Self {
        Self {
            translation,
            ..self
        }
    }

    pub fn with_scale(self, scaling: f32) -> Self {
        Self {
            scaling: scaling.max(f32::EPSILON),
            ..self
        }
    }

    pub fn with_unit_spacing(self, unit_spacing: UnitSpacing) -> Self {
        Self {
            unit_spacing,
            ..self
        }
    }

    pub fn zoom_around_screen(
        self,
        cursor_screen: f32,
        origin: f32,
        delta: f32,
        min_scaling: f32,
        max_scaling: f32,
    ) -> Self {
        let old_scaling = self.scaling;
        let new_scaling = (old_scaling * (1.0 + delta / LEGACY_ZOOM_SENSITIVITY))
            .clamp(min_scaling, max_scaling)
            .max(f32::EPSILON);
        let cursor_world = self.screen_to_world(cursor_screen, origin);
        let translation =
            (cursor_screen - origin) / new_scaling - cursor_world * self.unit_spacing.size();

        self.with_scale(new_scaling).with_translation(translation)
    }

    pub fn change_unit_spacing(self, delta: f32, min_spacing: f32, max_spacing: f32) -> Self {
        self.with_unit_spacing(self.unit_spacing.zoom(delta, min_spacing, max_spacing))
    }

    pub fn zoom_or_change_unit_spacing(
        self,
        cursor_screen: f32,
        origin: f32,
        delta: f32,
        min_scaling: f32,
        max_scaling: f32,
        default_spacing: f32,
        min_spacing: f32,
        max_spacing: f32,
    ) -> Self {
        let zoom_in = delta > 0.0;
        let should_change_unit_spacing = (zoom_in
            && self.scaling >= max_scaling
            && self.unit_spacing.size() < max_spacing)
            || (!zoom_in && self.scaling <= min_scaling && self.unit_spacing.size() > min_spacing)
            || (zoom_in
                && self.scaling <= min_scaling
                && self.unit_spacing.size() < default_spacing)
            || (!zoom_in
                && self.scaling >= max_scaling
                && self.unit_spacing.size() > default_spacing);

        if should_change_unit_spacing {
            let cursor_world = self.screen_to_world(cursor_screen, origin);
            let next = self.change_unit_spacing(delta, min_spacing, max_spacing);
            let translation =
                (cursor_screen - origin) / next.scaling - cursor_world * next.unit_spacing.size();
            return next.with_translation(translation);
        }

        self.zoom_around_screen(cursor_screen, origin, delta, min_scaling, max_scaling)
    }
}

use exchange::UnixMs;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelYViewport {
    pub min_value: f32,
    pub max_value: f32,
}

impl PanelYViewport {
    pub fn normalized(min_value: f32, max_value: f32) -> Option<Self> {
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
        Some(Self {
            min_value,
            max_value,
        })
    }

    pub fn span(self) -> f32 {
        (self.max_value - self.min_value).abs()
    }

    pub fn zoom_around_ratio(
        self,
        anchor_ratio: f32,
        zoom_scale: f32,
        zoom_in: bool,
        min_span: f32,
    ) -> Option<Self> {
        let anchor_ratio = anchor_ratio.clamp(0.0, 1.0);
        let span = self.span().max(1e-8);
        let zoom_scale = zoom_scale.abs().max(1.0);
        let min_span = min_span.abs().max(1e-8);
        let new_span = if zoom_in {
            span / zoom_scale
        } else {
            span * zoom_scale
        }
        .max(min_span);
        let anchor_value = self.min_value + anchor_ratio * span;
        let new_min = anchor_value - anchor_ratio * new_span;
        Self::normalized(new_min, new_min + new_span)
    }

    pub fn pan_by_ratio(self, delta_ratio: f32) -> Option<Self> {
        if !delta_ratio.is_finite() {
            return None;
        }
        let span = self.span().max(1e-8);
        let shift = delta_ratio * span;
        Self::normalized(self.min_value + shift, self.max_value + shift)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct YUnit(pub i64);
