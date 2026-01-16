use super::{AxisInteraction, AxisInteractionKind, Message};
use exchange::util::{MinTicksize, Price, PriceStep};
use iced::{Rectangle, Renderer, Theme, mouse, widget::canvas};

/// Rough vertical spacing (in screen pixels) between Y-axis labels.
const LABEL_TARGET_PX: f64 = 48.0;

pub struct AxisYLabelCanvas<'a> {
    pub cache: &'a iced::widget::canvas::Cache,
    pub base_price: Price,
    pub step: PriceStep,
    pub row_h: f32,
    pub cam_offset_y: f32,
    pub cam_sy: f32,
    /// Rounds/formats labels to a decade step (e.g. power=-2 => 0.01).
    /// Type alias: MinTicksize = Power10<-8, 2>
    pub label_precision: MinTicksize,
}

impl canvas::Program<Message> for AxisYLabelCanvas<'_> {
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

                    Some(canvas::Action::publish(Message::PanDeltaPx(iced::Vector {
                        x: 0.0,
                        y: delta_px.y,
                    })))
                } else {
                    None
                }
            }
            iced::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
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
        if !self.row_h.is_finite()
            || self.row_h <= 0.0
            || !self.cam_sy.is_finite()
            || self.cam_sy <= 0.0
        {
            return vec![];
        }

        let tick_labels = self.cache.draw(renderer, bounds.size(), |frame| {
            let vh = bounds.height as f64;

            let cam_offset_y = self.cam_offset_y as f64;
            let cam_sy = self.cam_sy as f64;
            let row_h = self.row_h as f64;

            let y_world_top = cam_offset_y + (0.0 - 0.5 * vh) / cam_sy;
            let y_world_bottom = cam_offset_y + (vh - 0.5 * vh) / cam_sy;

            let min_steps = (-(y_world_bottom) / row_h).floor() as i64;
            let max_steps = (-(y_world_top) / row_h).ceil() as i64;

            let px_per_step = (row_h * cam_sy).max(1e-9);

            let rough_every_steps = (LABEL_TARGET_PX / px_per_step).ceil() as i64;
            let every_steps = super::nice_step_i64(rough_every_steps.max(1));

            let text_color = theme.palette().text;
            let font_size = 12.0f32;

            let x = bounds.width / 2.0;

            let mut s = min_steps.div_euclid(every_steps) * every_steps;
            if s < min_steps {
                s += every_steps;
            }

            while s <= max_steps {
                let y_world = -((s as f64 + 0.5) * row_h);
                let y_px = (y_world - cam_offset_y) * cam_sy + 0.5 * vh;
                let y_px = y_px as f32;

                if (0.0..=bounds.height).contains(&y_px) {
                    let price = self.base_price.add_steps(s, self.step);
                    let label = price.to_string(self.label_precision);

                    frame.fill_text(canvas::Text {
                        content: label,
                        position: iced::Point::new(x, y_px),
                        color: text_color,
                        size: font_size.into(),
                        font: crate::style::AZERET_MONO,
                        align_x: iced::Alignment::Center.into(),
                        align_y: iced::Alignment::Center.into(),
                        ..Default::default()
                    });
                }

                s = s.saturating_add(every_steps);
                if every_steps <= 0 {
                    break;
                }
            }
        });

        vec![tick_labels]
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
