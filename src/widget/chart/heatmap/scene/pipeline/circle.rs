#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CircleInstance {
    // y stays world (you already compute it from price/bins)
    pub y_world: f32,

    // time in bucket space relative to scroll_ref_bucket
    pub x_bin_rel: i32,
    pub x_frac: f32, // in [0,1)

    pub radius_px: f32,
    pub _pad: f32,

    pub color: [f32; 4],
}

pub const CIRCLE_VERTICES: &[[f32; 2]] = &[[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];

pub const CIRCLE_INDICES: &[u16] = &[0, 1, 2, 2, 3, 0];
