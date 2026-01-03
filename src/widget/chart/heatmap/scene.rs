mod camera;
pub mod pipeline;

use super::Message;
use iced::wgpu;
use pipeline::Pipeline;

use iced::Rectangle;
use iced::mouse;
use iced::widget::shader::{self, Viewport};

use crate::widget::chart::heatmap::scene::camera::Camera;
use crate::widget::chart::heatmap::scene::pipeline::ParamsUniform;
use crate::widget::chart::heatmap::scene::pipeline::circle::CircleInstance;
use crate::widget::chart::heatmap::scene::pipeline::rectangle::RectInstance;

use std::sync::atomic::{AtomicU64, Ordering};

fn next_scene_id() -> u64 {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone)]
pub struct Scene {
    pub id: u64,
    pub rectangles: Vec<RectInstance>,
    pub circles: Vec<CircleInstance>,
    pub camera: camera::Camera,
    pub params: ParamsUniform,
}

impl Scene {
    pub fn new() -> Self {
        Self {
            id: next_scene_id(),
            rectangles: Vec::new(),
            circles: Vec::new(),
            camera: Camera::default(),
            params: ParamsUniform::default(),
        }
    }

    pub fn set_params(&mut self, params: ParamsUniform) {
        self.params = params;
    }

    pub fn set_rectangles(&mut self, rectangles: Vec<RectInstance>) {
        self.rectangles = rectangles;
    }

    pub fn set_circles(&mut self, circles: Vec<CircleInstance>) {
        self.circles = circles;
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
            &self.rectangles,
            &self.circles,
            self.camera,
            self.params,
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
    rectangles: Vec<RectInstance>,
    circles: Vec<CircleInstance>,
    camera: camera::Camera,
    params: ParamsUniform,
}

impl Primitive {
    pub fn new(
        id: u64,
        rectangles: &[RectInstance],
        circles: &[CircleInstance],
        camera: camera::Camera,
        params: ParamsUniform,
    ) -> Self {
        Self {
            id,
            rectangles: rectangles.to_vec(),
            circles: circles.to_vec(),
            camera,
            params,
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
        pipeline.update_rect_instances(self.id, device, queue, &self.rectangles);
        pipeline.update_circle_instances(self.id, device, queue, &self.circles);

        let cam_u = self.camera.to_uniform(bounds.width, bounds.height);
        pipeline.update_camera(self.id, device, queue, &cam_u);

        pipeline.update_params(self.id, device, queue, &self.params);
    }

    fn render(
        &self,
        pipeline: &Pipeline,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        pipeline.render_rectangles(
            self.id,
            encoder,
            target,
            *clip_bounds,
            self.rectangles.len() as u32,
        );
        pipeline.render_circles(
            self.id,
            encoder,
            target,
            *clip_bounds,
            self.circles.len() as u32,
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
