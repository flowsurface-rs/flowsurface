#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Camera {
    pub scale: [f32; 2],     // pixels per world unit
    pub offset: [f32; 2],    // world coord of "live" point (x=0 at latest bucket end)
    pub right_pad_frac: f32, // fraction of viewport width reserved for x>0
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            scale: [100.0, 100.0],
            offset: [0.0, 0.0],
            right_pad_frac: 0.10, // 20% of screen for the x>0 depth profile
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CameraUniform {
    pub a: [f32; 4], // (scale.x, scale.y, center.x, center.y)
    pub b: [f32; 4], // (viewport_w, viewport_h, 0, 0)
}

impl Camera {
    /// Reset offset.x to starting state.
    /// Note: right padding is applied in `center()` via `right_edge()`,
    /// so offset.x should represent the live boundary at world x = 0
    pub fn reset_offset_x(&mut self, _viewport_w: f32) {
        self.offset[0] = 0.0;
    }

    #[inline]
    fn right_pad_world(&self, viewport_w: f32) -> f32 {
        let sx = self.scale[0].max(1e-6);
        (viewport_w * self.right_pad_frac) / sx
    }

    #[inline]
    pub fn right_edge(&self, viewport_w: f32) -> f32 {
        self.offset[0] + self.right_pad_world(viewport_w)
    }

    #[inline]
    fn center(&self, viewport_w: f32) -> [f32; 2] {
        let sx = self.scale[0].max(1e-6);
        let right_edge = self.right_edge(viewport_w);
        let center_x = right_edge - (viewport_w * 0.5) / sx;
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
        let sx = self.scale[0].max(1e-6);
        let sy = self.scale[1].max(1e-6);
        let [cx, cy] = self.center(viewport_w);

        let screen_x = (world_x - cx) * sx + viewport_w * 0.5;
        let screen_y = (world_y - cy) * sy + viewport_h * 0.5;

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

        let new_sx = (self.scale[0] * factor).clamp(10.0, 5000.0);
        let new_sy = (self.scale[1] * factor).clamp(10.0, 5000.0);
        self.scale = [new_sx, new_sy];

        let view_x_px = cursor_x - viewport_w * 0.5;
        let view_y_px = cursor_y - viewport_h * 0.5;

        let pad_world = self.right_pad_world(viewport_w);
        let right_edge = wx + (viewport_w * 0.5) / new_sx - view_x_px / new_sx;

        self.offset[0] = right_edge - pad_world;
        self.offset[1] = wy - view_y_px / new_sy;
    }

    /// Convert a screen pixel (origin top-left of the shader bounds) to world coords.
    pub fn screen_to_world(
        &self,
        screen_x: f32,
        screen_y: f32,
        viewport_w: f32,
        viewport_h: f32,
    ) -> [f32; 2] {
        let sx = self.scale[0].max(1e-6);
        let sy = self.scale[1].max(1e-6);
        let [cx, cy] = self.center(viewport_w);

        let view_x_px = screen_x - viewport_w * 0.5;
        let view_y_px = screen_y - viewport_h * 0.5;

        let world_x = cx + view_x_px / sx;
        let world_y = cy + view_y_px / sy;

        [world_x, world_y]
    }

    pub fn to_uniform(self, viewport_w: f32, viewport_h: f32) -> CameraUniform {
        let vw = viewport_w.round().max(1.0);
        let vh = viewport_h.round().max(1.0);

        let sx = self.scale[0].max(1e-6);
        let sy = self.scale[1].max(1e-6);

        let [center_x, center_y] = self.center(vw);

        CameraUniform {
            a: [sx, sy, center_x, center_y],
            b: [vw, vh, 0.0, 0.0],
        }
    }

    /// World Y at a screen Y, where screen origin is top-left of viewport and Y grows downward.
    /// This uses the *centered* anchor (matches current row-height zoom math).
    #[inline]
    pub fn world_y_at_screen_y_centered(
        &self,
        screen_y: f32,
        viewport_h: f32,
        min_scale: f32,
    ) -> f32 {
        let sy = self.scale[1].max(min_scale);
        self.offset[1] + (screen_y - 0.5 * viewport_h) / sy
    }

    /// Set camera.offset[1] so that `world_y` stays under `screen_y` (centered anchor).
    #[inline]
    pub fn set_offset_y_for_world_y_at_screen_y_centered(
        &mut self,
        world_y: f32,
        screen_y: f32,
        viewport_h: f32,
        min_scale: f32,
    ) {
        let sy = self.scale[1].max(min_scale);
        self.offset[1] = world_y - (screen_y - 0.5 * viewport_h) / sy;
    }

    /// World X at a screen X using the *right-anchored* mapping where screen_x == viewport_w maps to x=0.
    /// This matches your existing column-width zoom math.
    #[inline]
    pub fn world_x_at_screen_x_right_anchored(
        &self,
        screen_x: f32,
        viewport_w: f32,
        min_scale: f32,
    ) -> f32 {
        let sx = self.scale[0].max(min_scale);
        self.offset[0] + (screen_x - viewport_w) / sx
    }

    /// Set camera.offset[0] so that `world_x` stays under `screen_x` (right-anchored mapping).
    #[inline]
    pub fn set_offset_x_for_world_x_at_screen_x_right_anchored(
        &mut self,
        world_x: f32,
        screen_x: f32,
        viewport_w: f32,
        min_scale: f32,
    ) {
        let sx = self.scale[0].max(min_scale);
        self.offset[0] = world_x - (screen_x - viewport_w) / sx;
    }
}
