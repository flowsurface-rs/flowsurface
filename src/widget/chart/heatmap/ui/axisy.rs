use super::{AxisInteraction, Message};
use exchange::unit::{MinTicksize, Price, PriceStep};
use iced::{Rectangle, Renderer, Theme, widget::canvas};
use iced_core::mouse;

/// Rough vertical spacing (in screen pixels) between Y-axis labels.
const LABEL_TARGET_PX: f64 = 48.0;
const DRAG_ZOOM_SENS: f32 = 0.005;

pub struct AxisYLabelCanvas<'a> {
    pub cache: &'a iced::widget::canvas::Cache,
    pub plot_bounds: Option<Rectangle>,
    pub base_price: Option<Price>,
    pub step: PriceStep,
    pub row_h: f32,
    pub cam_offset_y: f32,
    pub cam_sy: f32,
    /// Rounds/formats labels to a decade step (e.g. power=-2 => 0.01).
    /// Type alias: MinTicksize = Power10<-8, 2>
    pub label_precision: MinTicksize,
}

/// Represents a label to be drawn on the Y axis.
enum LabelKind {
    Tick { y_px: f32, label: String },
    Base { y_px: f32, label: String },
    Cursor { y_px: f32, label: String },
}

impl LabelKind {
    fn clip_range(&self, label_height: f32) -> (f32, f32) {
        match self {
            LabelKind::Tick { y_px, .. }
            | LabelKind::Base { y_px, .. }
            | LabelKind::Cursor { y_px, .. } => {
                (*y_px - 0.5 * label_height, *y_px + 0.5 * label_height)
            }
        }
    }
}

/// Converts world y to pixel y.
fn world_to_px(y_world: f64, cam_offset_y: f64, cam_sy: f64, vh: f64) -> f32 {
    ((y_world - cam_offset_y) * cam_sy + 0.5 * vh) as f32
}

/// Converts pixel y to world y.
fn px_to_world(y_px: f32, cam_offset_y: f64, cam_sy: f64, vh: f64) -> f64 {
    cam_offset_y + (y_px as f64 - 0.5 * vh) / cam_sy
}

/// Checks if two ranges overlap.
fn ranges_overlap(a: (f32, f32), b: (f32, f32)) -> bool {
    a.1 >= b.0 && a.0 <= b.1
}

impl canvas::Program<Message> for AxisYLabelCanvas<'_> {
    type State = super::AxisState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = cursor.position_over(bounds)?;

                // Double-click detection needs global cursor position + previous click state.
                if let Some(global_pos) = cursor.position() {
                    let new_click =
                        mouse::Click::new(global_pos, mouse::Button::Left, state.previous_click);

                    let is_double = new_click.kind() == mouse::click::Kind::Double;

                    state.previous_click = Some(new_click);

                    if is_double {
                        state.interaction = AxisInteraction::None;
                        return Some(canvas::Action::publish(Message::AxisYDoubleClicked));
                    }
                } else {
                    state.previous_click = None;
                }

                state.interaction = AxisInteraction::Panning {
                    last_position: p,
                    zoom_anchor: None,
                };
                None
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.interaction = AxisInteraction::None;
                None
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if let AxisInteraction::Panning { last_position, .. } = &mut state.interaction {
                    let delta_px = *position - *last_position;
                    *last_position = *position;

                    let factor = (-delta_px.y * DRAG_ZOOM_SENS).exp().clamp(0.01, 100.0);

                    Some(canvas::Action::publish(Message::ScrolledAxisY {
                        factor,
                        cursor_y: bounds.height * 0.5, // anchor at viewport center
                        viewport_h: bounds.height,
                    }))
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

                Some(canvas::Action::publish(Message::ScrolledAxisY {
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
        cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        if !self.row_h.is_finite()
            || self.row_h <= 0.0
            || !self.cam_sy.is_finite()
            || self.cam_sy <= 0.0
        {
            return vec![];
        }

        let Some(base_price) = self.base_price else {
            return vec![];
        };

        let palette = theme.extended_palette();

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
            let cursor_label_padding = 6.0f32;
            let cursor_label_height = font_size + 2.0 * cursor_label_padding;
            let size_cursor_label = iced::Size {
                width: bounds.width,
                height: cursor_label_height,
            };

            // --- Compute label positions and values ---
            let mut labels: Vec<LabelKind> = Vec::new();

            // Cursor label (highest priority)
            let cursor_label_snapped =
                self.plot_bounds
                    .and_then(|pb| cursor.position_in(pb))
                    .map(|p| {
                        let y_world_cursor = px_to_world(p.y, cam_offset_y, cam_sy, vh);
                        let step_at_cursor = (-(y_world_cursor) / row_h - 0.5).round() as i64;
                        let y_world_for_step = -((step_at_cursor as f64 + 0.5) * row_h);
                        let y_px = world_to_px(y_world_for_step, cam_offset_y, cam_sy, vh);

                        let price_at_cursor = base_price.add_steps(step_at_cursor, self.step);
                        let label_at_cursor = price_at_cursor.to_string(self.label_precision);

                        LabelKind::Cursor {
                            y_px,
                            label: label_at_cursor,
                        }
                    });

            // Base price label (secondary priority)
            let base_step: i64 = 0;
            let y_world_base = -((base_step as f64 + 0.5) * row_h);
            let y_px_base = world_to_px(y_world_base, cam_offset_y, cam_sy, vh);

            let price_base = base_price.add_steps(base_step, self.step);
            let label_base = price_base.to_string(self.label_precision);

            let base_label = LabelKind::Base {
                y_px: y_px_base,
                label: label_base,
            };

            // Tick labels
            let mut s = min_steps.div_euclid(every_steps) * every_steps;
            if s < min_steps {
                s += every_steps;
            }
            while s <= max_steps {
                let y_world = -((s as f64 + 0.5) * row_h);
                let y_px = world_to_px(y_world, cam_offset_y, cam_sy, vh);

                if (0.0..=bounds.height).contains(&y_px) {
                    let price = base_price.add_steps(s, self.step);
                    let label = price.to_string(self.label_precision);
                    labels.push(LabelKind::Tick { y_px, label });
                }

                s = s.saturating_add(every_steps);
                if every_steps <= 0 {
                    break;
                }
            }

            // --- Render labels with overlap filtering ---
            // Compute clip ranges for cursor and base labels
            let cursor_clip_range = cursor_label_snapped
                .as_ref()
                .map(|l| l.clip_range(size_cursor_label.height));
            let base_clip_range = base_label.clip_range(size_cursor_label.height);

            // Draw tick labels, skipping overlaps
            for label in &labels {
                let tick_clip = label.clip_range(font_size * 0.7 * 2.0);
                // Skip if overlaps cursor or base label
                if cursor_clip_range.is_some_and(|c| ranges_overlap(tick_clip, c)) {
                    continue;
                }
                if ranges_overlap(tick_clip, base_clip_range) {
                    continue;
                }

                if let LabelKind::Tick { y_px, label } = label {
                    frame.fill_text(canvas::Text {
                        content: label.clone(),
                        position: iced::Point::new(x, *y_px),
                        color: text_color,
                        size: font_size.into(),
                        font: crate::style::AZERET_MONO,
                        align_x: iced::Alignment::Center.into(),
                        align_y: iced::Alignment::Center.into(),
                        ..Default::default()
                    });
                }
            }

            // Draw base label if not overlapped by cursor
            let base_y_in_view = (0.0..=bounds.height).contains(&y_px_base);
            let overlaps_cursor = cursor_clip_range
                .map(|c| ranges_overlap(base_clip_range, c))
                .unwrap_or(false);
            if base_y_in_view && !overlaps_cursor {
                let mut bg = palette.secondary.strong.color;
                bg = iced::Color { a: 1.0, ..bg };

                frame.fill_rectangle(
                    iced::Point::new(0.0, y_px_base - 0.5 * size_cursor_label.height),
                    size_cursor_label,
                    bg,
                );

                if let LabelKind::Base { label, .. } = &base_label {
                    frame.fill_text(canvas::Text {
                        content: label.clone(),
                        position: iced::Point::new(x, y_px_base),
                        color: palette.primary.strong.text,
                        size: font_size.into(),
                        font: crate::style::AZERET_MONO,
                        align_x: iced::Alignment::Center.into(),
                        align_y: iced::Alignment::Center.into(),
                        ..Default::default()
                    });
                }
            }

            // Draw cursor label (highest priority)
            if let Some(LabelKind::Cursor { y_px, label }) = cursor_label_snapped {
                let mut bg = palette.secondary.base.color;
                bg = iced::Color { a: 1.0, ..bg };

                frame.fill_rectangle(
                    iced::Point::new(0.0, y_px - 0.5 * size_cursor_label.height),
                    size_cursor_label,
                    bg,
                );

                frame.fill_text(canvas::Text {
                    content: label,
                    position: iced::Point::new(x, y_px),
                    color: palette.secondary.base.text,
                    size: font_size.into(),
                    font: crate::style::AZERET_MONO,
                    align_x: iced::Alignment::Center.into(),
                    align_y: iced::Alignment::Center.into(),
                    ..Default::default()
                });
            }
        });

        vec![tick_labels]
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if cursor.position_over(bounds).is_some() {
            match state.interaction {
                AxisInteraction::Panning { .. } => mouse::Interaction::Grabbing,
                _ => mouse::Interaction::ResizingVertically,
            }
        } else {
            mouse::Interaction::default()
        }
    }
}
