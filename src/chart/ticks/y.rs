use super::{AxisLabel, LabelContent, calc_label_rect};
pub use data::chart::ticks::y::PriceAxis;
use data::chart::ticks::{Y_LABEL_DENSITY, y::YTickLabel, y_labels_that_fit};
use exchange::unit::Price;

use super::{Basis, Interaction, Message};
use iced::{
    Color, Event, Rectangle, Renderer, Size, Theme, mouse,
    widget::canvas::{self, Cache, Geometry},
};

/// Shared layout context for building Y-axis labels on a single axis canvas.
///
/// Holds the layout parameters every label shares (canvas bounds, text size
/// and color) and offers one method per label strategy: a single fallback
/// label, a row-aligned grid, or a plain float grid.
pub struct LabelLayout {
    bounds: iced::Rectangle,
    text_size: f32,
    text_color: iced::Color,
    labels_can_fit: i32,
}

impl LabelLayout {
    pub fn new(
        bounds: iced::Rectangle,
        text_size: f32,
        text_color: iced::Color,
        density: f32,
    ) -> Self {
        let labels_can_fit = y_labels_that_fit(bounds.height, text_size, density) as i32;
        Self {
            bounds,
            text_size,
            text_color,
            labels_can_fit,
        }
    }

    /// Generate the full label set for a price range.
    ///
    /// `axis` bundles the chart's effective tick size with the label precision
    /// (the market's min tick); when set, labels are aligned to its rows and
    /// formatted at its precision. When unset, a plain float grid is used and
    /// large numbers are abbreviated.
    pub fn generate(&self, lowest: f64, highest: f64, axis: Option<PriceAxis>) -> Vec<AxisLabel> {
        let labels = YTickLabel::for_range(
            lowest,
            highest,
            self.bounds.height,
            self.labels_can_fit,
            axis,
        );

        labels
            .into_iter()
            .map(|label| AxisLabel::Y {
                bounds: calc_label_rect(label.y_pos, 1, self.text_size, self.bounds),
                value_label: LabelContent {
                    content: label.content,
                    background_color: None,
                    text_color: self.text_color,
                    text_size: self.text_size,
                },
                timer_label: None,
            })
            .collect()
    }
}

pub struct AxisLabelsY<'a> {
    pub labels_cache: &'a Cache,
    pub translation_y: f32,
    pub scaling: f32,
    pub min: Price,
    pub last_price: Option<PriceInfoLabel>,
    pub axis: PriceAxis,
    pub cell_height: f32,
    pub basis: Basis,
    pub chart_bounds: Rectangle,
}

impl AxisLabelsY<'_> {
    fn drag_bounds(bounds: Rectangle) -> Rectangle {
        bounds.shrink(super::AXIS_DRAG_EDGE_GUARD)
    }

    fn visible_region(&self, size: Size) -> Rectangle {
        let width = size.width / self.scaling;
        let height = size.height / self.scaling;

        Rectangle {
            x: 0.0,
            y: -self.translation_y - height / 2.0,
            width,
            height,
        }
    }

    /// Convert a canvas y position (pixels) to a price, exact to within one
    /// atomic unit (1e-11). Only the pixel geometry (y, cell_height) is f32
    /// here; the base price and aggregation step are exact atomic-unit values,
    /// so the offset arithmetic runs in f64 and is quantized to the atomic
    /// grid once at the end.
    fn y_to_price(&self, y: f32) -> Price {
        let ticks = f64::from(y) / f64::from(self.cell_height);
        let price = self.min.to_f64() - ticks * self.axis.row_step.to_f64_lossy();
        Price::from_f64(price)
    }
}

impl canvas::Program<Message> for AxisLabelsY<'_> {
    type State = Interaction;

    fn update(
        &self,
        interaction: &mut Interaction,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let drag_bounds = Self::drag_bounds(bounds);

        if let Event::Mouse(mouse::Event::ButtonReleased(_)) = event {
            *interaction = Interaction::None;
        }

        if let Event::Mouse(mouse_event) = event {
            match mouse_event {
                mouse::Event::ButtonPressed(mouse::Button::Left) => {
                    if cursor.position_in(drag_bounds).is_some()
                        && let Some(cursor_position) = cursor.position()
                    {
                        *interaction = Interaction::Zoomin {
                            last_position: cursor_position,
                        };
                    }
                }
                mouse::Event::CursorMoved { .. } => {
                    if let Interaction::Zoomin {
                        ref mut last_position,
                    } = *interaction
                        && let Some(cursor_position) = cursor.position()
                    {
                        let difference_y = last_position.y - cursor_position.y;

                        if difference_y.abs() > 1.0 {
                            *last_position = cursor_position;

                            let message = Message::YScaling(difference_y * 0.4, 0.0, false);

                            return Some(canvas::Action::publish(message).and_capture());
                        }
                    }
                }
                mouse::Event::WheelScrolled { delta } => match delta {
                    mouse::ScrollDelta::Lines { y, .. } | mouse::ScrollDelta::Pixels { y, .. } => {
                        cursor.position_in(drag_bounds)?;

                        let message = Message::YScaling(
                            *y,
                            {
                                if let Some(cursor_to_center) =
                                    cursor.position_from(bounds.center())
                                {
                                    cursor_to_center.y
                                } else {
                                    0.0
                                }
                            },
                            true,
                        );

                        return Some(canvas::Action::publish(message).and_capture());
                    }
                },
                _ => {}
            }
        }

        None
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let text_size = crate::style::text_size::BODY;
        let palette = theme.extended_palette();

        let labels = self.labels_cache.draw(renderer, bounds.size(), |frame| {
            let region = self.visible_region(frame.size());

            let highest = self.y_to_price(region.y);
            let lowest = self.y_to_price(region.y + region.height);

            let range = highest.saturating_sub(lowest);
            let ratio_of = |price: Price| price.ratio_in_range(lowest, highest);

            let mut all_labels = LabelLayout::new(
                bounds,
                text_size,
                palette.background.base.text,
                Y_LABEL_DENSITY,
            )
            .generate(lowest.to_f64(), highest.to_f64(), Some(self.axis));

            // Last price (priority 2)
            if let Some(label) = self.last_price {
                let candle_close_label = match self.basis {
                    Basis::Time(timeframe) => {
                        let interval = timeframe.to_milliseconds();

                        let current_time = chrono::Utc::now().timestamp_millis() as u64;
                        let next_kline_open = (current_time / interval + 1) * interval;

                        let remaining_seconds = (next_kline_open - current_time) / 1000;

                        if remaining_seconds > 0 {
                            let hours = remaining_seconds / 3600;
                            let minutes = (remaining_seconds % 3600) / 60;
                            let seconds = remaining_seconds % 60;

                            let time_format = if hours > 0 {
                                format!("{hours:02}:{minutes:02}:{seconds:02}")
                            } else {
                                format!("{minutes:02}:{seconds:02}")
                            };

                            Some(LabelContent {
                                content: time_format,
                                background_color: Some(palette.background.strong.color),
                                text_color: if palette.is_dark {
                                    Color::BLACK.scale_alpha(0.8)
                                } else {
                                    Color::WHITE.scale_alpha(0.8)
                                },
                                text_size: crate::style::text_size::SMALL,
                            })
                        } else {
                            None
                        }
                    }
                    Basis::Tick(_) => None,
                };

                let (price, color) = label.get_with_color(palette);

                let price_label = LabelContent {
                    content: price.to_string(self.axis.precision),
                    background_color: Some(color),
                    text_color: {
                        if candle_close_label.is_some() {
                            if palette.is_dark {
                                Color::BLACK
                            } else {
                                Color::WHITE
                            }
                        } else {
                            palette.primary.strong.text
                        }
                    },
                    text_size: crate::style::text_size::BODY,
                };

                let y_pos = bounds.height - ratio_of(price) as f32 * bounds.height;
                let content_amt = if candle_close_label.is_some() { 2 } else { 1 };

                all_labels.push(AxisLabel::Y {
                    bounds: calc_label_rect(y_pos, content_amt, text_size, bounds),
                    value_label: price_label,
                    timer_label: candle_close_label,
                });
            }

            // Crosshair price (priority 3)
            if let Some(crosshair_pos) = cursor.position_in(self.chart_bounds) {
                let ratio = f64::from(bounds.height - crosshair_pos.y) / f64::from(bounds.height);
                let rounded_price = Price::from_f64(lowest.to_f64() + ratio * range.to_f64())
                    .round_to_step(self.axis.row_step);
                let y_position = bounds.height - ratio_of(rounded_price) as f32 * bounds.height;

                let label = LabelContent {
                    content: rounded_price.to_string(self.axis.precision),
                    background_color: Some(palette.secondary.base.color),
                    text_color: palette.secondary.base.text,
                    text_size: crate::style::text_size::BODY,
                };

                all_labels.push(AxisLabel::Y {
                    bounds: calc_label_rect(y_position, 1, text_size, bounds),
                    value_label: label,
                    timer_label: None,
                });
            }

            AxisLabel::filter_and_draw(&all_labels, frame);
        });

        vec![labels]
    }

    fn mouse_interaction(
        &self,
        interaction: &Interaction,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        match interaction {
            Interaction::Zoomin { .. } => mouse::Interaction::ResizingVertically,
            Interaction::Panning { .. } => mouse::Interaction::None,
            Interaction::None if cursor.is_over(Self::drag_bounds(bounds)) => {
                mouse::Interaction::ResizingVertically
            }
            _ => mouse::Interaction::default(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum PriceInfoLabel {
    Up(Price),
    Down(Price),
    Neutral(Price),
}

impl PriceInfoLabel {
    pub fn new(close_price: Price, open_price: Price) -> Self {
        if close_price >= open_price {
            PriceInfoLabel::Up(close_price)
        } else {
            PriceInfoLabel::Down(close_price)
        }
    }

    pub fn get_with_color(self, palette: &iced::theme::palette::Extended) -> (Price, iced::Color) {
        match self {
            PriceInfoLabel::Up(p) => (p, palette.success.base.color),
            PriceInfoLabel::Down(p) => (p, palette.danger.base.color),
            PriceInfoLabel::Neutral(p) => (p, palette.secondary.strong.color),
        }
    }
}
