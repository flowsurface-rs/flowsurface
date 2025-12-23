use iced::wgpu::PipelineCompilationOptions;
use iced::wgpu::util::DeviceExt;
use iced::{Rectangle, wgpu};

pub mod rectangle;

use crate::widget::chart::heatmap::scene::camera::CameraUniform;
use crate::widget::chart::heatmap::scene::pipeline::rectangle::{
    RECT_INDICES, RECT_VERTICES, RectInstance,
};

use rustc_hash::FxHashMap;

struct PerSceneGpu {
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
}

pub struct Pipeline {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,

    camera_bind_group_layout: wgpu::BindGroupLayout,

    per_scene: FxHashMap<u64, PerSceneGpu>,

    num_indices: u32,
}

impl Pipeline {
    pub fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("rect vertex buffer"),
            contents: bytemuck::cast_slice(RECT_VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("rect index buffer"),
            contents: bytemuck::cast_slice(RECT_INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rect shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/rect.wgsl").into()),
        });

        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("camera bind group layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: Some(
                            std::num::NonZeroU64::new(std::mem::size_of::<CameraUniform>() as u64)
                                .unwrap(),
                        ),
                    },
                    count: None,
                }],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rect pipeline layout"),
            bind_group_layouts: &[&camera_bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rect pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<[f32; 2]>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x2,
                        }],
                    },
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<RectInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[
                            wgpu::VertexAttribute {
                                offset: 0,
                                shader_location: 1,
                                format: wgpu::VertexFormat::Float32x2,
                            },
                            wgpu::VertexAttribute {
                                offset: 8,
                                shader_location: 2,
                                format: wgpu::VertexFormat::Float32x2,
                            },
                            wgpu::VertexAttribute {
                                offset: 16,
                                shader_location: 3,
                                format: wgpu::VertexFormat::Float32x4,
                            },
                        ],
                    },
                ],
                compilation_options: PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Self {
            pipeline,
            vertex_buffer,
            index_buffer,
            camera_bind_group_layout,
            per_scene: FxHashMap::default(),
            num_indices: RECT_INDICES.len() as u32,
        }
    }

    fn ensure_scene(&mut self, id: u64, device: &wgpu::Device) -> &mut PerSceneGpu {
        self.per_scene.entry(id).or_insert_with(|| {
            let instance_capacity: usize = 4096;
            let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rect instance buffer"),
                size: (instance_capacity * std::mem::size_of::<RectInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Camera Buffer"),
                contents: bytemuck::cast_slice(&[CameraUniform {
                    a: [1.0, 1.0, 0.0, 0.0],
                    b: [1.0, 1.0, 0.0, 0.0],
                }]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

            let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                layout: &self.camera_bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                }],
                label: Some("camera bind group"),
            });

            PerSceneGpu {
                instance_buffer,
                instance_capacity,
                camera_buffer,
                camera_bind_group,
            }
        })
    }

    pub fn update_instances(
        &mut self,
        id: u64,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[RectInstance],
    ) {
        let gpu = self.ensure_scene(id, device);

        if instances.len() > gpu.instance_capacity {
            gpu.instance_capacity = instances.len().next_power_of_two();
            gpu.instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rect instance buffer (resized)"),
                size: (gpu.instance_capacity * std::mem::size_of::<RectInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }

        queue.write_buffer(&gpu.instance_buffer, 0, bytemuck::cast_slice(instances));
    }

    pub fn update_camera(
        &mut self,
        id: u64,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        camera: &CameraUniform,
    ) {
        let gpu = self.ensure_scene(id, device);

        queue.write_buffer(
            &gpu.camera_buffer,
            0,
            bytemuck::cast_slice(std::slice::from_ref(camera)),
        );
    }

    pub fn render(
        &self,
        id: u64,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        viewport: Rectangle<u32>,
        num_instances: u32,
    ) {
        let Some(gpu) = self.per_scene.get(&id) else {
            return;
        };

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("rect render pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        pass.set_viewport(
            viewport.x as f32,
            viewport.y as f32,
            viewport.width as f32,
            viewport.height as f32,
            0.0,
            1.0,
        );
        pass.set_scissor_rect(viewport.x, viewport.y, viewport.width, viewport.height);

        pass.set_bind_group(0, &gpu.camera_bind_group, &[]);
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, gpu.instance_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..self.num_indices, 0, 0..num_instances);
    }
}
