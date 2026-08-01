pub mod x;
pub mod y;

use crate::{chart::TEXT_SIZE, style::AZERET_MONO};

use super::{Basis, Interaction, Message};
use data::chart::ticks::x::TimeTickTier;
use iced::{
    Alignment, Color, Point, Rectangle, Size,
    theme::palette::Extended,
    widget::canvas::{self, Frame},
};

const REGULAR_LABEL_WIDTH: f32 = TEXT_SIZE * 6.0;
const X_LABEL_CHAR_W: f32 = TEXT_SIZE * 0.65;

/// Guard area on the edges of the axis
/// to prevent conflicts with pane state interactions
///
/// (e.g. pane split dragging when trying to interact with labels)
const AXIS_DRAG_EDGE_GUARD: f32 = 4.0;

#[derive(Debug, Clone)]
pub enum AxisLabel {
    X {
        bounds: Rectangle,
        label: LabelContent,
    },
    Y {
        bounds: Rectangle,
        value_label: LabelContent,
        timer_label: Option<LabelContent>,
    },
}

impl AxisLabel {
    pub fn new_x(
        center_x_position: f32,
        text_content: String,
        axis_bounds: Rectangle,
        tier: TimeTickTier,
        is_crosshair: bool,
        palette: &Extended,
    ) -> Self {
        let content_width = text_content.len() as f32
            * if is_crosshair {
                TEXT_SIZE / 2.6
            } else {
                X_LABEL_CHAR_W
            };

        let rect = Rectangle {
            x: center_x_position - content_width,
            y: 4.0,
            width: 2.0 * content_width,
            height: axis_bounds.height - 8.0,
        };

        let label = LabelContent {
            content: text_content,
            background_color: if is_crosshair {
                Some(palette.secondary.base.color)
            } else {
                None
            },
            text_color: x_label_text_color(palette, tier, is_crosshair),
            text_size: TEXT_SIZE,
        };

        AxisLabel::X {
            bounds: rect,
            label,
        }
    }

    fn intersects(&self, other: &AxisLabel) -> bool {
        match (self, other) {
            (
                AxisLabel::Y {
                    bounds: self_rect, ..
                },
                AxisLabel::Y {
                    bounds: other_rect, ..
                },
            )
            | (
                AxisLabel::X {
                    bounds: self_rect, ..
                },
                AxisLabel::X {
                    bounds: other_rect, ..
                },
            ) => self_rect.intersects(other_rect),
            _ => false,
        }
    }

    pub fn filter_and_draw(labels: &[AxisLabel], frame: &mut Frame) {
        let mut drawn: Vec<&AxisLabel> = Vec::with_capacity(labels.len());
        for label in labels.iter().rev() {
            if drawn.iter().all(|existing| !existing.intersects(label)) {
                label.draw(frame);
                drawn.push(label);
            }
        }
    }

    fn draw(&self, frame: &mut Frame) {
        match self {
            AxisLabel::X { bounds, label } => {
                let frame_bounds = frame.size();
                if bounds.x + bounds.width < 0.0 || bounds.x > frame_bounds.width {
                    return;
                }

                if let Some(background_color) = label.background_color {
                    // `bounds` is already the snug crosshair box, so the pill
                    // is drawn straight from it.
                    frame.fill_rectangle(
                        Point::new(bounds.x, bounds.y),
                        Size::new(bounds.width, bounds.height),
                        background_color,
                    );
                }

                let label = canvas::Text {
                    content: label.content.clone(),
                    position: bounds.center(),
                    size: label.text_size.into(),
                    color: label.text_color,
                    align_y: Alignment::Center.into(),
                    align_x: Alignment::Center.into(),
                    font: AZERET_MONO,
                    ..canvas::Text::default()
                };

                frame.fill_text(label);
            }
            AxisLabel::Y {
                bounds,
                value_label,
                timer_label,
            } => {
                if let Some(background_color) = value_label.background_color {
                    frame.fill_rectangle(
                        Point::new(bounds.x, bounds.y),
                        Size::new(bounds.width, bounds.height),
                        background_color,
                    );
                }

                if let Some(timer_label) = timer_label {
                    let value_label = canvas::Text {
                        content: value_label.content.clone(),
                        position: Point::new(bounds.x + 4.0, bounds.y + 2.0),
                        color: value_label.text_color,
                        size: value_label.text_size.into(),
                        font: AZERET_MONO,
                        ..canvas::Text::default()
                    };

                    frame.fill_text(value_label);

                    let timer_label = canvas::Text {
                        content: timer_label.content.clone(),
                        position: Point::new(bounds.x + 4.0, bounds.y + 15.0),
                        color: timer_label.text_color,
                        size: timer_label.text_size.into(),
                        font: AZERET_MONO,
                        ..canvas::Text::default()
                    };

                    frame.fill_text(timer_label);
                } else {
                    let value_label = canvas::Text {
                        content: value_label.content.clone(),
                        position: Point::new(bounds.x + 4.0, bounds.y + 4.0),
                        color: value_label.text_color,
                        size: value_label.text_size.into(),
                        font: AZERET_MONO,
                        ..canvas::Text::default()
                    };

                    frame.fill_text(value_label);
                }
            }
        }
    }
}

/// calculates `Rectangle` from given content, clamps it within bounds if needed
pub fn calc_label_rect(
    y_pos: f32,
    content_amt: i16,
    text_size: f32,
    bounds: Rectangle,
) -> Rectangle {
    let content_amt = content_amt.max(1);
    let label_height = text_size + (f32::from(content_amt) * (text_size / 2.0) + 4.0);

    let rect = Rectangle {
        x: 1.0,
        y: y_pos - label_height / 2.0,
        width: bounds.width - 1.0,
        height: label_height,
    };

    // clamp when label is partially visible within bounds
    if rect.y < bounds.height && rect.y + label_height > 0.0 {
        Rectangle {
            y: rect.y.clamp(0.0, (bounds.height - label_height).max(0.0)),
            ..rect
        }
    } else {
        rect
    }
}

#[derive(Debug, Clone)]
pub struct LabelContent {
    pub content: String,
    pub background_color: Option<Color>,
    pub text_color: Color,
    pub text_size: f32,
}

/// Text color for an X-axis tick label.
///
/// `Main` labels use the standard readable text color; `Secondary` (coarse
/// calendar boundaries) use the strongest text color so they stand out and
/// stay readable on the chart background.
fn x_label_text_color(palette: &Extended, tier: TimeTickTier, is_crosshair: bool) -> Color {
    if is_crosshair {
        palette.secondary.base.text
    } else {
        match tier {
            TimeTickTier::Secondary => palette.background.strongest.text,
            TimeTickTier::Main => palette.background.base.text,
        }
    }
}
