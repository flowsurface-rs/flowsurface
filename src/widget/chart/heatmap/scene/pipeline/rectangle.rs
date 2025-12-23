#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RectInstance {
    pub position: [f32; 2], // x, y
    pub size: [f32; 2],     // width, height
    pub color: [f32; 4],    // RGBA
}

pub const RECT_VERTICES: &[[f32; 2]] = &[[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]];

pub const RECT_INDICES: &[u16] = &[0, 1, 2, 2, 3, 0];
