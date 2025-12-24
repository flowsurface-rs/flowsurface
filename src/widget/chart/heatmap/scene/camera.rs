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
            right_pad_frac: 0.20, // 20% of screen for the x>0 depth profile
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
    #[inline]
    fn right_pad_world(&self, viewport_w: f32) -> f32 {
        let sx = self.scale[0].max(1e-6);
        // world units = px / (px per world)
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
        let [center_x, center_y] = self.center(viewport_w);

        CameraUniform {
            a: [self.scale[0], self.scale[1], center_x, center_y],
            b: [viewport_w, viewport_h, 0.0, 0.0],
        }
    }
}
