use super::{
    camera::CameraUniform,
    pipeline::rectangle::{self, RECT_INDICES, RECT_VERTICES, RectInstance, RectXLayout},
    uniform::ParamsUniform,
};
use crate::{widget::chart::heatmap::view::TimeView, window::PixelRatio};
use data::chart::{
    heatmap::indicators::{HeatmapIndicator, HeatmapIndicators},
    kline::{
        composition::PanelScaleMode,
        viewport::{PanelValueScale, PanelYValueRange, PanelYViewportLimits, YUnit, YUnitRange},
    },
};
use exchange::{TickerInfo, UnixMs, unit::MinTicksize};
use iced::wgpu::util::DeviceExt;
use iced::{Rectangle, Theme, wgpu, widget::shader};
use rustc_hash::FxHashMap;
use std::{
    cell::RefCell,
    sync::{Arc, Weak},
};

#[derive(PartialEq)]
struct Key {
    revision: u64,
    first: u64,
    last: u64,
    latest: u64,
    buckets: u64,
    tick: MinTicksize,
    pixel_ratio: PixelRatio,
    colors: [[f32; 4]; 2],
}

struct Group {
    bucket: u64,
    means: [f64; 2],
    count: u64,
}

#[derive(Default)]
struct Prepared {
    key: Option<Key>,
    groups: Vec<Group>,
    instances: Arc<[RectInstance]>,
    generation: u64,
    max: Option<f64>,
    manual_range: Option<PanelYValueRange>,
}

impl Prepared {
    fn autoscale_range(&self, scale: PanelValueScale) -> Option<PanelYValueRange> {
        let (_, max) =
            PanelScaleMode::FitVisible.indicator_domain_units(0, scale.value_to_unit(self.max?).0);
        PanelYValueRange::from_units(YUnitRange::new(YUnit(0), max), scale)
    }

    fn viewport_limits(
        &self,
        scale: PanelValueScale,
        physical_height: f64,
    ) -> Option<PanelYViewportLimits> {
        PanelYViewportLimits::for_scale(
            scale,
            self.autoscale_range(scale).or(self.manual_range)?.span(),
            physical_height,
        )
    }

    fn range(&self, scale: PanelValueScale, physical_height: f64) -> Option<PanelYValueRange> {
        if let Some(range) = self.manual_range {
            self.viewport_limits(scale, physical_height)?.apply(range)
        } else {
            self.autoscale_range(scale)
        }
    }
}

pub struct Panel {
    id: u64,
    lifetime: Arc<()>,
    prepared: RefCell<Prepared>,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            id: super::next_scene_id(),
            lifetime: Arc::new(()),
            prepared: RefCell::new(Prepared::default()),
        }
    }
}

impl Panel {
    pub fn range(
        &self,
        scale: PanelValueScale,
        height: f32,
        pixel_ratio: PixelRatio,
    ) -> Option<PanelYValueRange> {
        self.prepared
            .borrow()
            .range(scale, f64::from(height) * f64::from(pixel_ratio.value()))
    }

    pub fn set_range(&mut self, range: Option<PanelYValueRange>) {
        self.prepared.get_mut().manual_range = range;
    }

    pub fn zoom_range(
        &self,
        initial: PanelYValueRange,
        anchor_ratio: f64,
        delta: f32,
        height: f32,
        pixel_ratio: PixelRatio,
        scale: PanelValueScale,
    ) -> Option<PanelYValueRange> {
        self.prepared
            .borrow()
            .viewport_limits(scale, f64::from(height) * f64::from(pixel_ratio.value()))?
            .zoom(initial, anchor_ratio, delta)
    }

    pub fn prepare(
        &self,
        indicators: &HeatmapIndicators,
        kind: HeatmapIndicator,
        time: TimeView<'_>,
        bounds: Rectangle,
        ticker: TickerInfo,
        theme: &Theme,
        pixel_ratio: PixelRatio,
    ) -> (Primitive, Option<PanelYValueRange>) {
        let tick = ticker.min_ticksize;
        let scale = kind.value_scale(indicators.config(), ticker);

        let pixels_per_bucket = time.column_world * time.camera.scale();
        let min_group_pixels = 3.0;
        let buckets = (min_group_pixels
            / (pixels_per_bucket * pixel_ratio.value()).max(f32::EPSILON))
        .ceil()
        .clamp(1.0, rectangle::MAX_BAR_BUCKETS as f32) as u64;

        let first =
            time.time_at_x(0.0, bounds.width).as_u64() / time.interval_ms / buckets * buckets;
        let last = time.time_at_x(bounds.width, bounds.width).as_u64() / time.interval_ms / buckets
            * buckets;
        let latest = time.latest_bucket.max(0) as u64;

        let palette = theme.extended_palette();
        let colors = match kind {
            HeatmapIndicator::Spread => [palette.primary.base.color.into_linear(); 2],
            HeatmapIndicator::Depth => [
                palette.success.strong.color.into_linear(),
                palette.danger.strong.color.into_linear(),
            ],
        };

        let key = Key {
            revision: indicators.revision(),
            first,
            last,
            latest,
            buckets,
            tick,
            pixel_ratio,
            colors,
        };

        let mut prepared = self.prepared.borrow_mut();
        if prepared.key.as_ref() != Some(&key) {
            prepared.groups.clear();
            let start = UnixMs::new(first.saturating_mul(time.interval_ms));
            let end = UnixMs::new(
                last.saturating_add(buckets - 1)
                    .min(latest)
                    .saturating_mul(time.interval_ms),
            );

            let mut channels = indicators.channels(kind, tick);
            let first_channel = channels.next().expect("indicator has an output channel");
            let second_channel = channels.next();
            let mut second =
                second_channel.map(|channel| channel.samples(start, end, time.interval_ms));

            for (timestamp, value) in first_channel.samples(start, end, time.interval_ms) {
                let second_value = second
                    .as_mut()
                    .and_then(|samples| samples.next())
                    .and_then(|(_, value)| value);
                let Some(value) = value else { continue };

                let values = match kind {
                    HeatmapIndicator::Spread => [value, 0.0],
                    HeatmapIndicator::Depth => {
                        let Some(ask) = second_value else { continue };
                        [value, ask]
                    }
                };

                let bucket = timestamp.as_u64() / time.interval_ms / buckets * buckets;
                if let Some(group) = prepared
                    .groups
                    .last_mut()
                    .filter(|group| group.bucket == bucket)
                {
                    group.count += 1;
                    for (mean, value) in group.means.iter_mut().zip(values) {
                        *mean += (value - *mean) / group.count as f64;
                    }
                } else {
                    prepared.groups.push(Group {
                        bucket,
                        means: values,
                        count: 1,
                    });
                }
            }

            let max = prepared
                .groups
                .iter()
                .flat_map(|group| group.means)
                .reduce(f64::max);
            prepared.max = max.map(|max| max.max(f64::EPSILON));

            let mut instances = Vec::with_capacity(prepared.groups.len());
            if let Some(max) = prepared.max {
                for group in &prepared.groups {
                    let heights = group.means.map(|value| (value / max) as f32);
                    if heights.iter().any(|height| *height > 0.0) {
                        let layout = match kind {
                            HeatmapIndicator::Spread => RectXLayout::PixelBar,
                            HeatmapIndicator::Depth => RectXLayout::IndicatorPair,
                        };
                        instances.push(RectInstance {
                            position: [0.0, 0.0],
                            size: heights,
                            color: colors[0],
                            x0_bin: (group.bucket - first) as i32,
                            x1_bin_excl: (group
                                .bucket
                                .saturating_add(buckets)
                                .min(latest.saturating_add(1))
                                as i64
                                - first as i64) as i32,
                            x_layout: layout as u32,
                            fade_mode: 1,
                            subpx_alpha: 1.0,
                        });
                    }
                }
            }

            prepared.instances = instances.into();
            prepared.generation = prepared.generation.wrapping_add(1);
            prepared.key = Some(key);
        }

        let mut camera = time.camera.to_uniform(bounds.width, bounds.height);
        camera.a[2] = time
            .camera
            .screen_to_world_x(bounds.width * 0.5, bounds.width);
        camera.b[0] = bounds.width;
        camera.b[1] = bounds.height;
        camera.b[2] = pixel_ratio.value();
        camera.b[3] = bounds.x;

        let range = prepared.range(
            scale,
            f64::from(bounds.height) * f64::from(pixel_ratio.value()),
        );
        let normalization_max = prepared.max.unwrap_or(1.0);
        if let Some(range) = range {
            camera.a[1] = (f64::from(bounds.height) * normalization_max / range.span()) as f32;
            camera.a[3] = -(range.min_value() * 0.5 / normalization_max
                + range.max_value() * 0.5 / normalization_max) as f32;
        } else {
            camera.a[1] = bounds.height.max(f32::EPSILON);
            camera.a[3] = -0.5;
        }

        let mut params = ParamsUniform::default();
        params.set_cell_world(time.column_world, 1.0);
        params.set_palette_rgba(colors[0], colors[1]);
        params.set_origin_x((latest as i64 - first as i64) as f32 + time.phase);

        (
            Primitive {
                id: self.id,
                lifetime: self.lifetime.clone(),
                instances: prepared.instances.clone(),
                generation: prepared.generation,
                camera,
                params,
            },
            range,
        )
    }
}

#[derive(Debug)]
pub struct Primitive {
    id: u64,
    lifetime: Arc<()>,
    instances: Arc<[RectInstance]>,
    generation: u64,
    camera: CameraUniform,
    params: ParamsUniform,
}

struct PanelGpu {
    lifetime: Weak<()>,
    instances: wgpu::Buffer,
    capacity: usize,
    generation: Option<u64>,
    camera: wgpu::Buffer,
    params: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

pub struct Pipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    panels: FxHashMap<u64, PanelGpu>,
}

impl shader::Pipeline for Pipeline {
    fn new(device: &wgpu::Device, _: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let layout = rectangle::bind_group_layout(device);
        let pipeline = rectangle::pipeline(device, format, &layout);
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("indicator vertices"),
            contents: bytemuck::cast_slice(RECT_VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("indicator indices"),
            contents: bytemuck::cast_slice(RECT_INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });
        Self {
            pipeline,
            layout,
            vertices,
            indices,
            panels: FxHashMap::default(),
        }
    }

    fn trim(&mut self) {
        self.panels
            .retain(|_, panel| panel.lifetime.strong_count() > 0);
    }
}

impl shader::Primitive for Primitive {
    type Pipeline = Pipeline;

    fn prepare(
        &self,
        pipeline: &mut Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        _: &shader::Viewport,
    ) {
        let mut camera = self.camera;
        camera.b[3] = bounds.x;
        pipeline
            .panels
            .retain(|_, panel| panel.lifetime.strong_count() > 0);
        let gpu = pipeline.panels.entry(self.id).or_insert_with(|| {
            let camera = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("indicator camera"),
                contents: bytemuck::bytes_of(&camera),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

            let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("indicator params"),
                contents: bytemuck::bytes_of(&self.params),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("indicator uniforms"),
                layout: &pipeline.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: camera.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: params.as_entire_binding(),
                    },
                ],
            });
            PanelGpu {
                lifetime: Arc::downgrade(&self.lifetime),
                instances: instance_buffer(device, 1),
                capacity: 1,
                generation: None,
                camera,
                params,
                bind_group,
            }
        });

        queue.write_buffer(&gpu.camera, 0, bytemuck::bytes_of(&camera));
        queue.write_buffer(&gpu.params, 0, bytemuck::bytes_of(&self.params));

        if gpu.generation != Some(self.generation) {
            if self.instances.len() > gpu.capacity {
                gpu.capacity = self.instances.len().next_power_of_two();
                gpu.instances = instance_buffer(device, gpu.capacity);
            }

            if !self.instances.is_empty() {
                queue.write_buffer(&gpu.instances, 0, bytemuck::cast_slice(&self.instances));
            }

            gpu.generation = Some(self.generation);
        }
    }

    fn draw(&self, pipeline: &Pipeline, pass: &mut wgpu::RenderPass<'_>) -> bool {
        let Some(gpu) = pipeline.panels.get(&self.id) else {
            return true;
        };
        if self.instances.is_empty() {
            return true;
        }
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, &gpu.bind_group, &[]);
        pass.set_vertex_buffer(0, pipeline.vertices.slice(..));
        pass.set_vertex_buffer(1, gpu.instances.slice(..));
        pass.set_index_buffer(pipeline.indices.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(
            0..RECT_INDICES.len() as u32,
            0,
            0..self.instances.len() as u32,
        );
        true
    }
}

fn instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("indicator bars"),
        size: (capacity * std::mem::size_of::<RectInstance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
