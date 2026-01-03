use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct RectInstance {
    // Overlay/world-space path (flags & 1 == 1):
    pub position: [f32; 2],
    pub size: [f32; 2],
    pub color: [f32; 4],

    // Heatmap path (flags & 1 == 0):
    pub qty: f32,
    pub side_sign: f32, // +1 bid, -1 ask

    pub x0_bin: i32,
    pub x1_bin_excl: i32,
    pub abs_y_bin: i32,
    pub flags: u32, // bit0: overlay/world-space
}

pub const RECT_VERTICES: &[[f32; 2]] = &[[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]];

pub const RECT_INDICES: &[u16] = &[0, 1, 2, 2, 3, 0];
