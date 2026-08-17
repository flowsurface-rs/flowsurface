pub(super) const LEGACY_ZOOM_SENSITIVITY: f32 = 30.0;

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub(crate) struct UnitSpacing(f32);

impl UnitSpacing {
    pub(super) fn new(size: f32) -> Self {
        Self(size.max(f32::EPSILON))
    }

    pub(super) fn size(self) -> f32 {
        self.0
    }

    pub(super) fn zoom(self, delta: f32, min_size: f32, max_size: f32) -> Self {
        let size = (self.0 * (1.0 + delta / LEGACY_ZOOM_SENSITIVITY))
            .clamp(min_size, max_size)
            .max(f32::EPSILON);
        Self::new(size)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct HorizontalWorld {
    unit_spacing: UnitSpacing,
    scaling: f32,
    offset: f32,
}

impl HorizontalWorld {
    pub(crate) fn new(cell_width: f32, scaling: f32, offset: f32) -> Self {
        Self {
            unit_spacing: UnitSpacing::new(cell_width),
            scaling: scaling.max(f32::EPSILON),
            offset,
        }
    }

    pub(crate) fn cell_width(self) -> f32 {
        self.unit_spacing.size()
    }

    pub(crate) fn scaling(self) -> f32 {
        self.scaling
    }

    pub(crate) fn offset(self) -> f32 {
        self.offset
    }

    pub(crate) fn effective_spacing(self) -> f32 {
        (self.unit_spacing.size() * self.scaling).max(f32::EPSILON)
    }

    pub(crate) fn with_offset(self, offset: f32) -> Self {
        Self { offset, ..self }
    }

    pub(crate) fn axis_at_right_unit(self, right_unit: f32) -> WorldAxis {
        WorldAxis::new(
            self.unit_spacing,
            self.scaling,
            -right_unit * self.unit_spacing.size(),
        )
    }

    pub(crate) fn updated_from_axis(self, current_right_unit: f32, next: WorldAxis) -> Self {
        let next_right_unit = -next.translation() / next.unit_spacing().size();
        Self {
            unit_spacing: next.unit_spacing(),
            scaling: next.scaling(),
            offset: self.offset + next_right_unit - current_right_unit,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct WorldAxis {
    unit_spacing: UnitSpacing,
    scaling: f32,
    translation: f32,
}

impl WorldAxis {
    pub(crate) fn new(unit_spacing: UnitSpacing, scaling: f32, translation: f32) -> Self {
        Self {
            unit_spacing,
            scaling: scaling.max(f32::EPSILON),
            translation,
        }
    }

    pub(super) fn unit_spacing(self) -> UnitSpacing {
        self.unit_spacing
    }

    pub(super) fn scaling(self) -> f32 {
        self.scaling
    }

    pub(super) fn translation(self) -> f32 {
        self.translation
    }

    pub(super) fn world_to_screen(self, world: f32, origin: f32) -> f32 {
        origin + (world * self.unit_spacing.size() + self.translation) * self.scaling
    }

    pub(super) fn screen_to_world(self, screen: f32, origin: f32) -> f32 {
        ((screen - origin) / self.scaling - self.translation) / self.unit_spacing.size()
    }

    pub(super) fn with_translation(self, translation: f32) -> Self {
        Self {
            translation,
            ..self
        }
    }

    pub(super) fn with_scale(self, scaling: f32) -> Self {
        Self {
            scaling: scaling.max(f32::EPSILON),
            ..self
        }
    }

    pub(super) fn with_unit_spacing(self, unit_spacing: UnitSpacing) -> Self {
        Self {
            unit_spacing,
            ..self
        }
    }

    pub(super) fn zoom_around_screen(
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

    pub(super) fn change_unit_spacing(
        self,
        delta: f32,
        min_spacing: f32,
        max_spacing: f32,
    ) -> Self {
        self.with_unit_spacing(self.unit_spacing.zoom(delta, min_spacing, max_spacing))
    }

    pub(super) fn zoom_or_change_unit_spacing(
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
