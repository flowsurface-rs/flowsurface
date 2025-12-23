#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CircleInstance {
    pub center: [f32; 2], // world
    pub radius_px: f32,   // screen pixels
    pub _pad: f32,
    pub color: [f32; 4],
}

pub const CIRCLE_VERTICES: &[[f32; 2]] = &[[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];

pub const CIRCLE_INDICES: &[u16] = &[0, 1, 2, 2, 3, 0];
