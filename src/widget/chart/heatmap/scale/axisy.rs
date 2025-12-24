use super::{AxisInteraction, AxisInteractionKind, Message};
use exchange::util::{Price, PriceStep};
use iced::{Rectangle, Renderer, Theme, mouse, widget::canvas};

const DEPTH_MIN_ROW_PX: f32 = 1.25;
const MAX_STEPS_PER_Y_BIN: i64 = 2048;

pub struct AxisYLabelCanvas {
    pub base_price: Price,
    pub step: PriceStep,
    pub row_h: f32,
    pub cam_offset_y: f32,
    pub cam_sy: f32,
}

impl canvas::Program<Message> for AxisYLabelCanvas {
    type State = AxisInteraction;

    fn update(
        &self,
        state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: iced_core::mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = cursor.position_over(bounds)?;
                state.kind = AxisInteractionKind::Panning { last_position: p };
                None
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.kind = AxisInteractionKind::None;
                None
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if let AxisInteractionKind::Panning { last_position } = &mut state.kind
                    && cursor.position_over(bounds).is_some()
                {
                    let delta_px = *position - *last_position;
                    *last_position = *position;

                    // Only vertical pan from Y axis.
                    Some(canvas::Action::publish(Message::PanDeltaPx(iced::Vector {
                        x: 0.0,
                        y: delta_px.y,
                    })))
                } else {
                    None
                }
            }
            iced::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                // Zoom row height around cursor Y.
                let p = cursor.position_in(bounds)?;
                let scroll_amount = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y * 0.1,
                    mouse::ScrollDelta::Pixels { y, .. } => *y * 0.01,
                };

                let factor = (1.0 + scroll_amount).clamp(0.01, 100.0);

                Some(canvas::Action::publish(Message::ZoomRowHeightAt {
                    factor,
                    cursor_y: p.y,
                    viewport_h: bounds.height,
                }))
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());

        if !self.row_h.is_finite()
            || self.row_h <= 0.0
            || !self.cam_sy.is_finite()
            || self.cam_sy <= 0.0
        {
            return vec![frame.into_geometry()];
        }

        let vh = bounds.height;

        let y_world_top = self.cam_offset_y + (0.0 - 0.5 * vh) / self.cam_sy;
        let y_world_bottom = self.cam_offset_y + (vh - 0.5 * vh) / self.cam_sy;

        let min_steps = (-(y_world_bottom) / self.row_h).floor() as i64;
        let max_steps = (-(y_world_top) / self.row_h).ceil() as i64;

        let px_per_step = (self.row_h * self.cam_sy).max(1e-6);
        let mut steps_per_y_bin: i64 = (DEPTH_MIN_ROW_PX / px_per_step).ceil() as i64;
        steps_per_y_bin = steps_per_y_bin.clamp(1, MAX_STEPS_PER_Y_BIN);

        let px_per_bin = (px_per_step * steps_per_y_bin as f32).max(1e-6);

        let target_px = 26.0f32;
        let rough_every_bins = (target_px / px_per_bin).ceil() as i64;
        let every_bins = super::nice_step_i64(rough_every_bins.max(1));

        let text_color = theme.palette().text;
        let tick_len = 7.0f32;
        let font_size = 12.0f32;

        let min_bin = min_steps.div_euclid(steps_per_y_bin);
        let max_bin = max_steps.div_euclid(steps_per_y_bin);

        let mut b = (min_bin.div_euclid(every_bins)) * every_bins;
        if b < min_bin {
            b += every_bins;
        }

        while b <= max_bin {
            let center_steps = b * steps_per_y_bin + (steps_per_y_bin / 2);
            let y_world = -((center_steps as f32 + 0.5) * self.row_h);
            let y_px = (y_world - self.cam_offset_y) * self.cam_sy + 0.5 * vh;

            if (0.0..=vh).contains(&y_px) {
                let price = self
                    .base_price
                    .add_steps(center_steps, self.step)
                    .to_f32_lossy();

                frame.fill_text(canvas::Text {
                    content: format!("{price}"),
                    position: iced::Point::new(tick_len + 4.0, y_px),
                    color: text_color,
                    size: font_size.into(),
                    font: crate::style::AZERET_MONO,
                    align_y: iced::Alignment::Center.into(),
                    ..Default::default()
                });
            }

            b = b.saturating_add(every_bins);
            if every_bins <= 0 {
                break;
            }
        }

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: iced_core::mouse::Cursor,
    ) -> iced_core::mouse::Interaction {
        if cursor.position_over(bounds).is_some() {
            match state.kind {
                AxisInteractionKind::Panning { .. } => iced_core::mouse::Interaction::Grabbing,
                _ => iced_core::mouse::Interaction::default(),
            }
        } else {
            iced_core::mouse::Interaction::default()
        }
    }
}
