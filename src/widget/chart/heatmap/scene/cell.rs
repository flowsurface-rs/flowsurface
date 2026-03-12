pub const MIN_COL_PX: f32 = 1.0;
pub const MIN_ROW_PX: f32 = 1.0;

pub const MAX_COL_PX: f32 = 20.0;
pub const MAX_ROW_PX: f32 = 20.0;

pub const MIN_COL_W_WORLD: f32 = 0.01;
pub const MAX_COL_W_WORLD: f32 = 1.;

pub const MIN_ROW_H_WORLD: f32 = 0.01;
pub const MAX_ROW_H_WORLD: f32 = 4.;

const DEFAULT_ROW_H_WORLD: f32 = 0.04;
const DEFAULT_COL_W_WORLD: f32 = 0.02;

#[derive(Debug, Clone, Copy)]
pub struct Cell {
    width_world: f32,
    height_world: f32,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            width_world: DEFAULT_COL_W_WORLD,
            height_world: DEFAULT_ROW_H_WORLD,
        }
    }
}

impl Cell {
    pub fn new(width_world: f32, height_world: f32) -> Self {
        let mut c = Self {
            width_world: if width_world.is_finite() {
                width_world
            } else {
                MIN_COL_W_WORLD
            },
            height_world: if height_world.is_finite() {
                height_world
            } else {
                MIN_ROW_H_WORLD
            },
        };
        c.clamp_world_only();
        c
    }

    pub fn width_world(&self) -> f32 {
        self.width_world
    }

    pub fn height_world(&self) -> f32 {
        self.height_world
    }

    pub fn set_default_width(&mut self) {
        self.set_width_world(DEFAULT_COL_W_WORLD);
    }

    pub fn set_width_world(&mut self, width_world: f32) {
        if width_world.is_finite() {
            self.width_world = width_world;
        }
        self.clamp_world_only();
    }

    pub fn set_height_world(&mut self, height_world: f32) {
        if height_world.is_finite() {
            self.height_world = height_world;
        }
        self.clamp_world_only();
    }

    /// Axis-zoom helper: apply factor, then clamp so resulting pixel size stays within
    /// [MIN_COL_PX, MAX_COL_PX] at the given camera scale, and also within world bounds.
    pub fn zoom_width_world_clamped(&mut self, factor: f32, cam_scale: f32) {
        let next = self.width_world * factor;
        self.width_world = Self::clamp_dim_world_for_scale(
            next,
            cam_scale,
            MIN_COL_PX,
            MAX_COL_PX,
            MIN_COL_W_WORLD,
            MAX_COL_W_WORLD,
        );
    }

    /// Axis-zoom helper: apply factor, then clamp so resulting pixel size stays within
    /// [MIN_ROW_PX, MAX_ROW_PX] at the given camera scale, and also within world bounds.
    pub fn zoom_height_world_clamped(&mut self, factor: f32, cam_scale: f32) {
        let next = self.height_world * factor;
        self.height_world = Self::clamp_dim_world_for_scale(
            next,
            cam_scale,
            MIN_ROW_PX,
            MAX_ROW_PX,
            MIN_ROW_H_WORLD,
            MAX_ROW_H_WORLD,
        );
    }

    fn clamp_world_only(&mut self) {
        if !self.width_world.is_finite() {
            self.width_world = MIN_COL_W_WORLD;
        }
        if !self.height_world.is_finite() {
            self.height_world = MIN_ROW_H_WORLD;
        }

        self.width_world = self.width_world.clamp(MIN_COL_W_WORLD, MAX_COL_W_WORLD);
        self.height_world = self.height_world.clamp(MIN_ROW_H_WORLD, MAX_ROW_H_WORLD);
    }

    fn clamp_dim_world_for_scale(
        value_world: f32,
        cam_scale: f32,
        min_px: f32,
        max_px: f32,
        min_world: f32,
        max_world: f32,
    ) -> f32 {
        if !value_world.is_finite() {
            return min_world;
        }

        if !cam_scale.is_finite() || cam_scale <= 0.0 {
            return value_world.clamp(min_world, max_world);
        }

        let min_from_px = (min_px / cam_scale).max(min_world);
        let max_from_px = (max_px / cam_scale).min(max_world);

        let max_eff = if max_from_px.is_finite() {
            max_from_px.max(min_from_px)
        } else {
            min_from_px
        };

        value_world.clamp(min_from_px, max_eff)
    }

    pub fn camera_scale_bounds_for_pixels(&self) -> Option<(f32, f32)> {
        if !(self.width_world.is_finite() && self.height_world.is_finite()) {
            return None;
        }
        if self.width_world <= 0.0 || self.height_world <= 0.0 {
            return None;
        }

        let min_s_for_width = MIN_COL_PX / self.width_world;
        let max_s_for_width = MAX_COL_PX / self.width_world;

        let min_s_for_height = MIN_ROW_PX / self.height_world;
        let max_s_for_height = MAX_ROW_PX / self.height_world;

        let min_s = min_s_for_width.max(min_s_for_height);
        let max_s = max_s_for_width.min(max_s_for_height);

        if !min_s.is_finite() || !max_s.is_finite() || min_s <= 0.0 || min_s > max_s {
            return None;
        }

        Some((min_s, max_s))
    }
}
