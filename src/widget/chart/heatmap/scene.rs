pub mod camera;
pub mod pipeline;

use super::Message;
use iced::wgpu;
use pipeline::Pipeline;

use iced::Rectangle;
use iced::mouse;
use iced::widget::shader::{self, Viewport};

use crate::widget::chart::heatmap::scene::pipeline::ParamsUniform;
use crate::widget::chart::heatmap::scene::pipeline::circle::CircleInstance;
use crate::widget::chart::heatmap::scene::pipeline::rectangle::RectInstance;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug)]
pub enum HeatmapUploadPlan {
    Full(HeatmapTextureCpuFull),
    Cols(Vec<HeatmapColumnCpu>),
    None,
}

#[derive(Clone, Debug)]
pub struct HeatmapColumnCpu {
    pub width: u32,
    pub height: u32,
    pub x: u32, // ring x index to update
    pub bid_col: Arc<Vec<u32>>,
    pub ask_col: Arc<Vec<u32>>,
}

#[derive(Clone, Debug)]
pub struct HeatmapTextureCpuFull {
    pub width: u32,
    pub height: u32,
    pub bid: Arc<Vec<u32>>,
    pub ask: Arc<Vec<u32>>,
    pub generation: u64,
}

fn next_scene_id() -> u64 {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone)]
pub struct Scene {
    pub id: u64,

    pub rectangles: Arc<[RectInstance]>,
    pub rectangles_gen: u64,

    pub circles: Arc<[CircleInstance]>,
    pub circles_gen: u64,

    pub camera: camera::Camera,
    pub params: ParamsUniform,

    pub heatmap_tex_gen: u64,

    pub heatmap_update: Option<HeatmapColumnCpu>,
    pub heatmap_cols: Arc<[HeatmapColumnCpu]>,
    pub heatmap_cols_gen: u64,
    pub heatmap_full: Option<HeatmapTextureCpuFull>,
}

impl Scene {
    pub fn new() -> Self {
        Self {
            id: next_scene_id(),
            rectangles: Arc::from(Vec::<RectInstance>::new()),
            rectangles_gen: 1,
            circles: Arc::from(Vec::<CircleInstance>::new()),
            circles_gen: 1,
            camera: camera::Camera::default(),
            params: ParamsUniform::default(),
            heatmap_tex_gen: 1,
            heatmap_update: None,
            heatmap_cols: Arc::from(Vec::<HeatmapColumnCpu>::new()),
            heatmap_cols_gen: 1,
            heatmap_full: None,
        }
    }

    pub fn set_rectangles(&mut self, rectangles: Vec<RectInstance>) {
        self.rectangles = Arc::from(rectangles);
        self.rectangles_gen = self.rectangles_gen.wrapping_add(1);
    }

    pub fn set_circles(&mut self, circles: Vec<CircleInstance>) {
        self.circles = Arc::from(circles);
        self.circles_gen = self.circles_gen.wrapping_add(1);
    }

    pub fn set_heatmap_update(&mut self, hm: Option<HeatmapColumnCpu>) {
        self.heatmap_update = hm;
    }

    pub fn set_heatmap_full(&mut self, hm: Option<HeatmapTextureCpuFull>) {
        self.heatmap_full = hm;
    }

    pub fn set_heatmap_cols(&mut self, cols: Vec<HeatmapColumnCpu>, generation: u64) {
        self.heatmap_cols = Arc::from(cols);
        self.heatmap_cols_gen = generation;
    }

    #[inline]
    fn bump_heatmap_gen(&mut self) -> u64 {
        self.heatmap_tex_gen = self.heatmap_tex_gen.wrapping_add(1);
        self.heatmap_tex_gen
    }

    pub fn apply_heatmap_upload_plan(&mut self, plan: HeatmapUploadPlan) {
        match plan {
            HeatmapUploadPlan::Full(mut full) => {
                let generation = self.bump_heatmap_gen();
                full.generation = generation;

                self.set_heatmap_full(Some(full));
                self.set_heatmap_cols(Vec::new(), generation);
                self.set_heatmap_update(None);
            }
            HeatmapUploadPlan::Cols(cols) => {
                let generation = self.bump_heatmap_gen();

                self.set_heatmap_cols(cols, generation);
                self.set_heatmap_full(None);
                self.set_heatmap_update(None);
            }
            HeatmapUploadPlan::None => {
                // no-op
            }
        }
    }
}

impl shader::Program<Message> for Scene {
    type State = Interaction;
    type Primitive = Primitive;

    fn update(
        &self,
        interaction: &mut Interaction,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: iced_core::mouse::Cursor,
    ) -> Option<shader::Action<Message>> {
        let current = [bounds.width, bounds.height];
        if interaction.last_bounds != current {
            interaction.last_bounds = current;
            return Some(shader::Action::publish(Message::BoundsChanged(current)));
        }

        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let cursor_in_abs = cursor.position_over(bounds)?;

                *interaction = Interaction {
                    last_bounds: interaction.last_bounds,
                    kind: InteractionKind::Panning {
                        last_position: cursor_in_abs,
                    },
                };
                None
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                interaction.kind = InteractionKind::None;
                None
            }
            iced::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let cursor_in_relative = cursor.position_in(bounds)?;

                let scroll_amount = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y * 0.1,
                    mouse::ScrollDelta::Pixels { y, .. } => *y * 0.01,
                };

                let factor = (1.0 + scroll_amount).clamp(0.01, 100.0);

                Some(shader::Action::publish(Message::ZoomAt {
                    factor,
                    cursor: cursor_in_relative,
                }))
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if let InteractionKind::Panning { last_position } = &mut interaction.kind
                    && cursor.position_over(bounds).is_some()
                {
                    let delta_px = *position - *last_position;
                    *last_position = *position;

                    Some(shader::Action::publish(Message::PanDeltaPx(delta_px)))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: mouse::Cursor,
        _bounds: Rectangle,
    ) -> Self::Primitive {
        Primitive::new(
            self.id,
            self.rectangles.clone(),
            self.rectangles_gen,
            self.circles.clone(),
            self.circles_gen,
            self.camera,
            self.params,
            self.heatmap_cols.clone(),
            self.heatmap_cols_gen,
            self.heatmap_full.clone(),
        )
    }

    fn mouse_interaction(
        &self,
        interaction: &Interaction,
        bounds: Rectangle,
        cursor: iced_core::mouse::Cursor,
    ) -> iced_core::mouse::Interaction {
        if cursor.position_over(bounds).is_some() {
            match interaction.kind {
                InteractionKind::Panning { .. } => iced_core::mouse::Interaction::Grabbing,
                _ => iced_core::mouse::Interaction::default(),
            }
        } else {
            iced_core::mouse::Interaction::default()
        }
    }
}

#[derive(Debug)]
pub struct Primitive {
    id: u64,

    rectangles: Arc<[RectInstance]>,
    rectangles_gen: u64,

    circles: Arc<[CircleInstance]>,
    circles_gen: u64,

    camera: camera::Camera,
    params: ParamsUniform,
    heatmap_cols: Arc<[HeatmapColumnCpu]>,
    heatmap_cols_gen: u64,
    heatmap_full: Option<HeatmapTextureCpuFull>,
}

impl Primitive {
    pub fn new(
        id: u64,
        rectangles: Arc<[RectInstance]>,
        rectangles_gen: u64,
        circles: Arc<[CircleInstance]>,
        circles_gen: u64,
        camera: camera::Camera,
        params: ParamsUniform,
        heatmap_cols: Arc<[HeatmapColumnCpu]>,
        heatmap_cols_gen: u64,
        heatmap_full: Option<HeatmapTextureCpuFull>,
    ) -> Self {
        Self {
            id,
            rectangles,
            rectangles_gen,
            circles,
            circles_gen,
            camera,
            params,
            heatmap_cols,
            heatmap_cols_gen,
            heatmap_full,
        }
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
        _viewport: &Viewport,
    ) {
        let cam_u = self.camera.to_uniform(bounds.width, bounds.height);

        pipeline.update_camera(self.id, device, queue, &cam_u);
        pipeline.update_params(self.id, device, queue, &self.params);

        pipeline.update_rect_instances(
            self.id,
            device,
            queue,
            self.rectangles.as_ref(),
            self.rectangles_gen,
        );
        pipeline.update_circle_instances(
            self.id,
            device,
            queue,
            self.circles.as_ref(),
            self.circles_gen,
        );

        if let Some(hm) = &self.heatmap_full {
            pipeline.update_heatmap_textures_u32(
                self.id,
                device,
                queue,
                hm.width,
                hm.height,
                hm.bid.as_slice(),
                hm.ask.as_slice(),
                hm.generation,
            );
        } else if !self.heatmap_cols.is_empty() {
            // batch column upload
            pipeline.update_heatmap_columns_u32(
                self.id,
                device,
                queue,
                self.heatmap_cols[0].width,
                self.heatmap_cols[0].height,
                self.heatmap_cols.as_ref(),
                self.heatmap_cols_gen,
            );
        }
    }

    fn render(
        &self,
        pipeline: &Pipeline,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        pipeline.single_pass_render_all(
            self.id,
            encoder,
            target,
            *clip_bounds,
            self.rectangles.len() as u32,
            self.circles.len() as u32,
            true,
        );
    }
}

impl shader::Pipeline for Pipeline {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Pipeline {
        Self::new(device, queue, format)
    }
}

#[derive(Debug, Default)]
pub struct Interaction {
    pub last_bounds: [f32; 2],
    pub kind: InteractionKind,
}

#[derive(Debug, Default)]
pub enum InteractionKind {
    #[default]
    None,
    Panning {
        last_position: iced::Point,
    },
}
