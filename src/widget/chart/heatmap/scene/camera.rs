use crate::widget::chart::heatmap::scene::cell::{Cell, MIN_COL_W_WORLD, MIN_ROW_H_WORLD};

const MIN_CAMERA_SCALE: f32 = 1e1;
const MAX_CAMERA_SCALE: f32 = 1e3;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CameraUniform {
    pub a: [f32; 4], // (scale, center.x, center.y, pad)
    pub b: [f32; 4], // (viewport_w, viewport_h, pad, pad)
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Camera {
    scale: f32,              // pixels per world unit
    pub offset: [f32; 2],    // world coord of "live" point (x=0 at latest bucket end)
    pub right_pad_frac: f32, // fraction of viewport width reserved for x>0
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            scale: 100.0,
            offset: [0.0, 0.0],
            right_pad_frac: 0.10, // right padding used for current orderbook
        }
    }
}

impl Camera {
    /// Reset offset.x to starting state.
    /// Note: right padding is applied in `center()` via `right_edge()`,
    /// so offset.x should represent the live boundary at world x = 0
    pub fn reset_offset_x(&mut self, _viewport_w: f32) {
        self.offset[0] = 0.0;
    }

    /// Price at the camera's Y-center (world y = camera.offset[1])
    pub fn price_at_center(
        &self,
        row_height: f32,
        base_price: exchange::util::Price,
        step: exchange::util::PriceStep,
    ) -> exchange::util::Price {
        let y = self.offset[1];

        if !row_height.is_finite() || row_height <= 0.0 || !y.is_finite() {
            return base_price;
        }

        let steps_f = (-(y) / row_height).round();
        let steps_i = steps_f as i64;

        base_price.add_steps(steps_i, step)
    }

    #[inline]
    fn right_pad_world(&self, viewport_w: f32) -> f32 {
        let s = self.scale_with_min(MIN_CAMERA_SCALE);
        (viewport_w * self.right_pad_frac) / s
    }

    /// Return the camera scale clamped to allowable bounds.
    #[inline]
    pub fn scale(&self) -> f32 {
        self.clamped_scale()
    }

    /// Set the camera scale, clamped to allowable bounds.
    #[inline]
    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale.clamp(MIN_CAMERA_SCALE, MAX_CAMERA_SCALE);
    }

    /// Clamp the stored scale in-place.
    #[inline]
    pub fn clamp_scale(&mut self) {
        self.set_scale(self.scale);
    }

    /// Internal: return clamped scale but don't mutate.
    #[inline]
    fn clamped_scale(&self) -> f32 {
        self.scale.clamp(MIN_CAMERA_SCALE, MAX_CAMERA_SCALE)
    }

    #[inline]
    fn scale_with_min(&self, min_scale: f32) -> f32 {
        self.scale
            .clamp(min_scale.max(MIN_CAMERA_SCALE), MAX_CAMERA_SCALE)
    }

    #[inline]
    fn scale_x_with_min(&self, min_scale: f32) -> f32 {
        self.scale_with_min(min_scale)
    }

    #[inline]
    fn scale_y_with_min(&self, min_scale: f32) -> f32 {
        self.scale_with_min(min_scale)
    }

    #[inline]
    pub fn right_edge(&self, viewport_w: f32) -> f32 {
        self.offset[0] + self.right_pad_world(viewport_w)
    }

    #[inline]
    fn center(&self, viewport_w: f32) -> [f32; 2] {
        let s = self.scale_with_min(MIN_CAMERA_SCALE);
        let right_edge = self.right_edge(viewport_w);
        let center_x = right_edge - (viewport_w * 0.5) / s;
        let center_y = self.offset[1];
        [center_x, center_y]
    }

    /// Convert world coords to screen pixel coords (origin top-left of viewport).
    pub fn world_to_screen(
        &self,
        world_x: f32,
        world_y: f32,
        viewport_w: f32,
        viewport_h: f32,
    ) -> [f32; 2] {
        let s = self.clamped_scale();
        let [cx, cy] = self.center(viewport_w);

        let screen_x = (world_x - cx) * s + viewport_w * 0.5;
        let screen_y = (world_y - cy) * s + viewport_h * 0.5;

        [screen_x, screen_y]
    }

    pub fn zoom_at_cursor(
        &mut self,
        factor: f32,
        cursor_x: f32,
        cursor_y: f32,
        viewport_w: f32,
        viewport_h: f32,
    ) {
        let factor = factor.clamp(0.01, 100.0);

        let [wx, wy] = self.screen_to_world(cursor_x, cursor_y, viewport_w, viewport_h);

        let new_s = (self.scale * factor).clamp(MIN_CAMERA_SCALE, MAX_CAMERA_SCALE);

        self.set_scale(new_s);

        let view_x_px = cursor_x - viewport_w * 0.5;
        let view_y_px = cursor_y - viewport_h * 0.5;

        let pad_world = self.right_pad_world(viewport_w);
        let right_edge = wx + (viewport_w * 0.5) / new_s - view_x_px / new_s;

        self.offset[0] = right_edge - pad_world;
        self.offset[1] = wy - view_y_px / new_s;
    }

    #[inline]
    fn world_x_at_screen_x_padded_right(
        &self,
        screen_x: f32,
        viewport_w: f32,
        min_scale: f32,
    ) -> f32 {
        let s = self.scale_x_with_min(min_scale);
        self.offset[0] + self.right_pad_world(viewport_w) + (screen_x - viewport_w) / s
    }

    pub fn zoom_row_h_at(&mut self, factor: f32, cursor_y: f32, vh_px: f32, cell: &mut Cell) {
        if !factor.is_finite() || vh_px <= 1.0 {
            return;
        }

        let world_y_before = self.world_y_at_screen_y_centered(cursor_y, vh_px, MIN_CAMERA_SCALE);
        let row_units_at_cursor = world_y_before / cell.height_world().max(MIN_ROW_H_WORLD);

        let s = self.scale_y_with_min(MIN_CAMERA_SCALE);
        cell.zoom_height_world_clamped(factor, s);

        let world_y_after = row_units_at_cursor * cell.height_world();

        self.set_offset_y_for_world_y_at_screen_y_centered(
            world_y_after,
            cursor_y,
            vh_px,
            MIN_CAMERA_SCALE,
        );
    }

    pub fn zoom_column_world_at(
        &mut self,
        factor: f32,
        cursor_x: f32,
        vw_px: f32,
        cell: &mut Cell,
    ) {
        if !factor.is_finite() || vw_px <= 1.0 {
            return;
        }

        let world_x_before =
            self.world_x_at_screen_x_padded_right(cursor_x, vw_px, MIN_CAMERA_SCALE);

        let col_units_at_cursor = world_x_before / cell.width_world().max(MIN_COL_W_WORLD);

        let s = self.scale_x_with_min(MIN_CAMERA_SCALE);
        cell.zoom_width_world_clamped(factor, s);

        let world_x_after = col_units_at_cursor * cell.width_world();

        self.set_offset_x_for_world_x_at_screen_x_padded_right(
            world_x_after,
            cursor_x,
            vw_px,
            MIN_CAMERA_SCALE,
        );
    }

    pub fn zoom_column_world_keep_anchor(
        &mut self,
        factor: f32,
        anchor_world_x: f32,
        anchor_screen_x: f32,
        vw_px: f32,
        cell: &mut Cell,
    ) {
        if !factor.is_finite()
            || !anchor_world_x.is_finite()
            || !anchor_screen_x.is_finite()
            || !vw_px.is_finite()
            || vw_px <= 1.0
        {
            return;
        }

        let s = self.scale_x_with_min(MIN_CAMERA_SCALE);
        cell.zoom_width_world_clamped(factor, s);

        self.set_offset_x_for_world_x_at_screen_x_padded_right(
            anchor_world_x,
            anchor_screen_x,
            vw_px,
            MIN_CAMERA_SCALE,
        );
    }

    /// Set camera.offset[0] so that `world_x` stays under `screen_x` using the padded-right mapping.
    #[inline]
    fn set_offset_x_for_world_x_at_screen_x_padded_right(
        &mut self,
        world_x: f32,
        screen_x: f32,
        viewport_w: f32,
        min_scale: f32,
    ) {
        let s = self.scale_x_with_min(min_scale);
        let pad_world = self.right_pad_world(viewport_w);
        self.offset[0] = world_x - pad_world - (screen_x - viewport_w) / s;
    }

    /// Convert a screen pixel (origin top-left of the shader bounds) to world coords.
    pub fn screen_to_world(
        &self,
        screen_x: f32,
        screen_y: f32,
        viewport_w: f32,
        viewport_h: f32,
    ) -> [f32; 2] {
        let s = self.clamped_scale();
        let [cx, cy] = self.center(viewport_w);

        let view_x_px = screen_x - viewport_w * 0.5;
        let view_y_px = screen_y - viewport_h * 0.5;

        let world_x = cx + view_x_px / s;
        let world_y = cy + view_y_px / s;

        [world_x, world_y]
    }

    pub fn to_uniform(self, viewport_w: f32, viewport_h: f32) -> CameraUniform {
        let vw = viewport_w.round().max(1.0);
        let vh = viewport_h.round().max(1.0);

        let s = self.clamped_scale();
        let [center_x, center_y] = self.center(vw);

        CameraUniform {
            a: [s, s, center_x, center_y],
            b: [vw, vh, 0.0, 0.0],
        }
    }

    /// World Y at a screen Y, where screen origin is top-left of viewport and Y grows downward.
    /// This uses the *centered* anchor (matches current row-height zoom math).
    #[inline]
    fn world_y_at_screen_y_centered(&self, screen_y: f32, viewport_h: f32, min_scale: f32) -> f32 {
        let s = self.scale_y_with_min(min_scale);
        self.offset[1] + (screen_y - 0.5 * viewport_h) / s
    }

    /// Set camera.offset[1] so that `world_y` stays under `screen_y` (centered anchor).
    #[inline]
    fn set_offset_y_for_world_y_at_screen_y_centered(
        &mut self,
        world_y: f32,
        screen_y: f32,
        viewport_h: f32,
        min_scale: f32,
    ) {
        let s = self.scale_y_with_min(min_scale);
        self.offset[1] = world_y - (screen_y - 0.5 * viewport_h) / s;
    }

    pub fn zoom_at_cursor_to_scale(
        &mut self,
        target_scale: f32,
        cursor_x: f32,
        cursor_y: f32,
        viewport_w: f32,
        viewport_h: f32,
    ) {
        let target_scale = target_scale.clamp(MIN_CAMERA_SCALE, MAX_CAMERA_SCALE);

        let [wx, wy] = self.screen_to_world(cursor_x, cursor_y, viewport_w, viewport_h);

        self.set_scale(target_scale);

        let view_x_px = cursor_x - viewport_w * 0.5;
        let view_y_px = cursor_y - viewport_h * 0.5;

        let pad_world = self.right_pad_world(viewport_w);
        let right_edge = wx + (viewport_w * 0.5) / target_scale - view_x_px / target_scale;

        self.offset[0] = right_edge - pad_world;
        self.offset[1] = wy - view_y_px / target_scale;
    }
}
