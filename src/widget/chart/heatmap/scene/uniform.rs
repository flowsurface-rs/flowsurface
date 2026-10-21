use super::{camera::CameraUniform, pipeline::rectangle::BAR_GAP_FRACTION};
use crate::widget::chart::heatmap::view::ViewWindow;
use bytemuck::{Pod, Zeroable};
use iced::wgpu::{self, util::DeviceExt};

// Shift volume-strip rects left by half a bucket to align with circle centers
const VOLUME_X_SHIFT_BUCKET: f32 = -0.5;

const PROFILE_FADE_START_MULT: f32 = 1.5;
const PROFILE_FADE_ALPHA_MIN: f32 = 0.15;
const PROFILE_FADE_ALPHA_MAX: f32 = 1.0;

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct ParamsUniform {
    /// (max_depth_denom, alpha_min, alpha_max, *pad*)
    depth: [f32; 4],
    /// (r, g, b, alpha for indicator layouts)
    bid_rgb: [f32; 4],
    /// (r, g, b, alpha for indicator layouts)
    ask_rgb: [f32; 4],
    /// (cell_w_world, cell_h_world, steps_per_y_bin, *pad*)
    grid: [f32; 4],
    /// (now_bucket_rel_with_phase, volume_min_w_world, volume_gap_frac, volume_x_shift_bucket)
    origin: [f32; 4],
    /// (latest_rel_bucket, y_start_bin, *pad*, latest_x_ring)
    heatmap_map: [f32; 4],
    /// (tex_w, tex_h, tex_w_mask=(tex_w-1, requires power-of-two width), inv_qty_scale)
    heatmap_tex: [f32; 4],
    /// (x_left_world, width_world, alpha_min, alpha_max)
    fade: [f32; 4],
}

impl Default for ParamsUniform {
    fn default() -> Self {
        Self {
            depth: [1.0, 0.01, 0.99, 0.0],
            bid_rgb: [0.0, 1.0, 0.0, 0.0],
            ask_rgb: [1.0, 0.0, 0.0, 0.0],
            grid: [0.1, 0.1, 1.0, 0.0],
            origin: [0.0, 0.0, BAR_GAP_FRACTION, VOLUME_X_SHIFT_BUCKET],
            heatmap_map: [0.0, 0.0, 1.0, 0.0],
            heatmap_tex: [0.0, 0.0, 0.0, 0.0],
            fade: [0.0, 0.0, 1.0, 1.0],
        }
    }
}

impl ParamsUniform {
    pub fn set_cell_world(&mut self, width: f32, height: f32) {
        self.grid[0] = width;
        self.grid[1] = height;
    }

    pub fn set_steps_per_y_bin(&mut self, steps_per_y_bin: i64) {
        self.grid[2] = (steps_per_y_bin.max(1)) as f32;
    }

    pub fn steps_per_y_bin(&self) -> i64 {
        self.grid[2].round().max(1.0) as i64
    }

    pub fn set_origin_x(&mut self, x: f32) {
        self.origin[0] = x;
    }

    pub fn set_heatmap_latest_rel_bucket(&mut self, latest_rel_bucket: i64) {
        self.heatmap_map[0] = latest_rel_bucket as f32;
    }

    pub fn set_heatmap_y_start_bin(&mut self, y_start_bin: f32) {
        self.heatmap_map[1] = y_start_bin;
    }

    pub fn set_heatmap_latest_x_ring(&mut self, x_ring: u32) {
        self.heatmap_map[3] = x_ring as f32;
    }

    pub fn set_heatmap_tex_info(&mut self, tex_w: u32, tex_h: u32, qty_scale: f32) {
        debug_assert!(
            tex_w.is_power_of_two(),
            "heatmap tex width must be power-of-two because shader wraps with bitmask (& (tex_w-1))"
        );

        let qty_scale_inv = 1.0 / qty_scale;
        self.heatmap_tex = [
            tex_w as f32,
            tex_h as f32,
            (tex_w.saturating_sub(1)) as f32,
            qty_scale_inv,
        ];
    }

    pub fn set_palette_rgb(&mut self, bid_rgb: [f32; 3], ask_rgb: [f32; 3]) {
        self.bid_rgb = [bid_rgb[0], bid_rgb[1], bid_rgb[2], 0.0];
        self.ask_rgb = [ask_rgb[0], ask_rgb[1], ask_rgb[2], 0.0];
    }

    pub fn set_palette_rgba(&mut self, bid: [f32; 4], ask: [f32; 4]) {
        self.bid_rgb = bid;
        self.ask_rgb = ask;
    }

    pub fn set_depth_denom(&mut self, denom: f32) {
        self.depth[0] = denom;
    }

    pub fn set_trade_fade(&mut self, view_window: &ViewWindow) {
        let fade_start = view_window.volume_profile_max_width * PROFILE_FADE_START_MULT;
        let fade_end = view_window.left_edge_world;

        let alpha_min = PROFILE_FADE_ALPHA_MIN;
        let alpha_max = PROFILE_FADE_ALPHA_MAX;

        self.fade = [fade_end, fade_start, alpha_min, alpha_max];
    }

    pub fn heatmap_start_bin(&self) -> i64 {
        self.heatmap_map[1].round() as i64
    }

    pub fn origin_x(&self) -> f32 {
        self.origin[0]
    }
}

pub(super) struct GpuUniforms {
    camera: wgpu::Buffer,
    params: wgpu::Buffer,
    pub(super) bind_group: wgpu::BindGroup,
    uploaded_camera: CameraUniform,
    uploaded_params: ParamsUniform,
}

impl GpuUniforms {
    pub(super) fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera+params bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: Some(
                            std::num::NonZeroU64::new(std::mem::size_of::<CameraUniform>() as u64)
                                .unwrap(),
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: Some(
                            std::num::NonZeroU64::new(std::mem::size_of::<ParamsUniform>() as u64)
                                .unwrap(),
                        ),
                    },
                    count: None,
                },
            ],
        })
    }

    pub(super) fn new(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        camera: CameraUniform,
        params: ParamsUniform,
    ) -> Self {
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("camera uniform"),
            contents: bytemuck::bytes_of(&camera),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("params uniform"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera+params bind group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params_buffer.as_entire_binding(),
                },
            ],
        });

        Self {
            camera: camera_buffer,
            params: params_buffer,
            bind_group,
            uploaded_camera: camera,
            uploaded_params: params,
        }
    }

    pub(super) fn update_camera(&mut self, queue: &wgpu::Queue, camera: &CameraUniform) {
        if bytemuck::bytes_of(&self.uploaded_camera) == bytemuck::bytes_of(camera) {
            return;
        }

        queue.write_buffer(&self.camera, 0, bytemuck::bytes_of(camera));
        self.uploaded_camera = *camera;
    }

    pub(super) fn update_params(&mut self, queue: &wgpu::Queue, params: &ParamsUniform) {
        if bytemuck::bytes_of(&self.uploaded_params) == bytemuck::bytes_of(params) {
            return;
        }

        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(params));
        self.uploaded_params = *params;
    }
}
