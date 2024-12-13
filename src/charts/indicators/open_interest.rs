use std::collections::BTreeMap;

use iced::widget::{container, row, Canvas};
use iced::{mouse, Element, Length, Point, Rectangle, Renderer, Size, Theme, Vector};
use iced::widget::canvas::{self, Cache, Event, Geometry, LineDash, Path, Stroke};

use crate::charts::{
    calc_price_step, convert_to_qty_abbr, round_to_tick, AxisLabel, Caches, CommonChartData, Interaction, Label, Message
};
use crate::data_providers::format_with_commas;

pub fn create_indicator_elem<'a>(
    chart_state: &'a CommonChartData,
    cache: &'a Caches, 
    data: &'a BTreeMap<i64, f32>,
    earliest: i64,
    latest: i64,
) -> Element<'a, Message> {
    let indi_chart = Canvas::new(OpenInterest {
        indicator_cache: &cache.main,
        crosshair_cache: &cache.crosshair,
        crosshair: chart_state.crosshair,
        max: chart_state.latest_x,
        scaling: chart_state.scaling,
        translation_x: chart_state.translation.x,
        timeframe: chart_state.timeframe as u32,
        cell_width: chart_state.cell_width,
        data_points: data,
        chart_bounds: chart_state.bounds,
    })
    .height(Length::Fill)
    .width(Length::Fill);

    let mut max_value: f32 = f32::MIN;
    let mut min_value: f32 = f32::MAX;

    data.range(earliest..=latest)
        .for_each(|(_, value)| {
            max_value = max_value.max(*value);
            min_value = min_value.min(*value);
        });

    let value_range = max_value - min_value;
    let padding = value_range * 0.01;
    max_value += padding;
    min_value -= padding;

    let indi_labels = Canvas::new(OpenInterestLabels {
        label_cache: &cache.y_labels,
        max: max_value,
        min: min_value,
        crosshair: chart_state.crosshair,
        chart_bounds: chart_state.bounds,
    })
    .height(Length::Fill)
    .width(Length::Fixed(60.0 + (chart_state.decimals as f32 * 2.0)));

    row![
        indi_chart,
        container(indi_labels),
    ].into()
}

pub struct OpenInterest<'a> {
    pub indicator_cache: &'a Cache,
    pub crosshair_cache: &'a Cache,
    pub crosshair: bool,
    pub max: i64,
    pub scaling: f32,
    pub translation_x: f32,
    pub timeframe: u32,
    pub cell_width: f32,
    pub data_points: &'a BTreeMap<i64, f32>,
    pub chart_bounds: Rectangle,
}

impl OpenInterest<'_> {
    fn visible_region(&self, size: Size) -> Rectangle {
        let width = size.width / self.scaling;
        let height = size.height / self.scaling;

        Rectangle {
            x: -self.translation_x - width / 2.0,
            y: 0.0,
            width,
            height,
        }
    }

    fn x_to_time(&self, x: f32) -> i64 {
        let time_per_cell = self.timeframe;
        self.max + ((x / self.cell_width) * time_per_cell as f32) as i64
    }

    fn time_to_x(&self, time: i64) -> f32 {
        let time_per_cell = self.timeframe;
        let x = (time - self.max) as f32 / time_per_cell as f32;
        x * self.cell_width
    }
}

impl canvas::Program<Message> for OpenInterest<'_> {
    type State = Interaction;

    fn update(
        &self,
        interaction: &mut Interaction,
        event: Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let message = match *interaction {
                    Interaction::None => {
                        if self.crosshair && cursor.is_over(bounds) {
                            Some(Message::CrosshairMoved)
                        } else {
                            None
                        }
                    }
                    _ => None,
                };

                let action =
                    message.map_or(canvas::Action::request_redraw(), canvas::Action::publish);

                Some(match interaction {
                    Interaction::None => action,
                    _ => action.and_capture(),
                })
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
    ) -> Vec<Geometry> {
        if self.data_points.is_empty() {
            return vec![];
        }

        let center = Vector::new(bounds.width / 2.0, bounds.height / 2.0);

        let palette = theme.extended_palette();

        let indicator = self.indicator_cache.draw(renderer, bounds.size(), |frame| {
            frame.translate(center);
            frame.scale(self.scaling);
            frame.translate(Vector::new(
                self.translation_x,
                (-bounds.height / self.scaling) / 2.0,
            ));

            let region = self.visible_region(frame.size());

            frame.fill_rectangle(
                Point::new(region.x, 0.0),
                Size::new(region.width, 1.0 / self.scaling),
                if palette.is_dark {
                    palette.background.weak.color.scale_alpha(0.2)
                } else {
                    palette.background.strong.color.scale_alpha(0.2)
                },
            );

            let (earliest, latest) = (
                self.x_to_time(region.x) - i64::from(self.timeframe / 2),
                self.x_to_time(region.x + region.width) + i64::from(self.timeframe / 2),
            );

            let mut max_value: f32 = f32::MIN;
            let mut min_value: f32 = f32::MAX;

            self.data_points
                .range(earliest..=latest)
                .for_each(|(_, value)| {
                    max_value = max_value.max(*value);
                    min_value = min_value.min(*value);
                });

            let padding = (max_value - min_value) * 0.08;
            max_value += padding;
            min_value -= padding;

            let points: Vec<Point> = self.data_points
                .range(earliest..=latest)
                .map(|(timestamp, value)| {
                    let x_position = self.time_to_x(*timestamp);
                    let normalized_height = if max_value > min_value {
                        (value - min_value) / (max_value - min_value)
                    } else {
                        0.0
                    };
                    let y_position = (bounds.height / self.scaling) - 
                        (normalized_height * (bounds.height / self.scaling));
                    
                    Point::new(x_position - (self.cell_width / 2.0), y_position)
                })
                .collect();

            if points.len() >= 2 {
                for points in points.windows(2) {
                    let stroke = Stroke {
                        width: 1.0,
                        ..Stroke::default()
                    };
                    frame.stroke(
                        &Path::line(points[0], points[1]), 
                        Stroke::with_color(stroke, palette.secondary.strong.color)
                    )
                }
            }

            let radius = (self.cell_width * 0.2).min(5.0);
            for point in points {
                frame.fill(
                    &Path::circle(Point::new(point.x, point.y), radius),
                    palette.secondary.strong.color,
                );
            }
        });

        if self.crosshair {
            let crosshair = self.crosshair_cache.draw(renderer, bounds.size(), |frame| {
                let dashed_line = Stroke::with_color(
                    Stroke {
                        width: 1.0,
                        line_dash: LineDash {
                            segments: &[4.0, 4.0],
                            offset: 8,
                        },
                        ..Default::default()
                    },
                    palette.secondary.strong.color
                        .scale_alpha(
                            if palette.is_dark {
                                0.6
                            } else {
                                1.0
                            },
                        ),
                );

                if let Some(cursor_position) = cursor.position_in(self.chart_bounds) {
                    let region = self.visible_region(frame.size());
                    
                    // Vertical time line
                    let earliest = self.x_to_time(region.x) as f64;
                    let latest = self.x_to_time(region.x + region.width) as f64;

                    let crosshair_ratio = f64::from(cursor_position.x / bounds.width);
                    let crosshair_millis = earliest + crosshair_ratio * (latest - earliest);

                    let rounded_timestamp =
                        (crosshair_millis / (self.timeframe as f64)).round() as i64 * self.timeframe as i64;
                    let snap_ratio = ((rounded_timestamp as f64 - earliest) / (latest - earliest)) as f32;

                    frame.stroke(
                        &Path::line(
                            Point::new(snap_ratio * bounds.width, 0.0),
                            Point::new(snap_ratio * bounds.width, bounds.height),
                        ),
                        dashed_line,
                    );

                    if let Some((_, oi_value)) = self
                        .data_points
                        .iter()
                        .find(|(time, _)| **time == rounded_timestamp)
                    {
                        let next_value = self
                            .data_points
                            .range((rounded_timestamp + (self.timeframe as i64))..=i64::MAX)
                            .next()
                            .map(|(_, val)| *val);

                        let change_text = if let Some(next_oi) = next_value {
                            let difference = next_oi - *oi_value;
                            let sign = if difference >= 0.0 { "+" } else { "" };
                            format!("Change: {}{}", sign, format_with_commas(difference))
                        } else {
                            "Change: N/A".to_string()
                        };

                        let tooltip_text = format!(
                            "Open Interest: {}\n{}",
                            format_with_commas(*oi_value),
                            change_text,
                        );

                        let text = canvas::Text {
                            content: tooltip_text,
                            position: Point::new(8.0, 2.0),
                            size: iced::Pixels(10.0),
                            color: palette.background.base.text,
                            ..canvas::Text::default()
                        };
                        frame.fill_text(text);

                        frame.fill_rectangle(
                            Point::new(4.0, 0.0),
                            Size::new(140.0, 28.0),
                            palette.background.base.color,
                        );
                    }
                } else if let Some(cursor_position) = cursor.position_in(bounds) {
                    // Horizontal price line
                    let highest = self.max as f32;
                    let lowest = 0.0;

                    let crosshair_ratio = cursor_position.y / bounds.height;
                    let crosshair_price = highest + crosshair_ratio * (lowest - highest);

                    let rounded_price = round_to_tick(crosshair_price, 1.0);
                    let snap_ratio = (rounded_price - highest) / (lowest - highest);

                    frame.stroke(
                        &Path::line(
                            Point::new(0.0, snap_ratio * bounds.height),
                            Point::new(bounds.width, snap_ratio * bounds.height),
                        ),
                        dashed_line,
                    );
                }
            });

            vec![indicator, crosshair]
        } else {
            vec![indicator]
        }
    }

    fn mouse_interaction(
        &self,
        interaction: &Interaction,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        match interaction {
            Interaction::Panning { .. } => mouse::Interaction::Grabbing,
            Interaction::Zoomin { .. } => mouse::Interaction::ZoomIn,
            Interaction::None if cursor.is_over(bounds) => {
                if self.crosshair {
                    mouse::Interaction::Crosshair
                } else {
                    mouse::Interaction::default()
                }
            }
            _ => mouse::Interaction::default(),
        }
    }
}

pub struct OpenInterestLabels<'a> {
    pub label_cache: &'a Cache,
    pub crosshair: bool,
    pub max: f32,
    pub min: f32,
    pub chart_bounds: Rectangle,
}

impl canvas::Program<Message> for OpenInterestLabels<'_> {
    type State = Interaction;

    fn update(
        &self,
        _state: &mut Self::State,
        _event: Event,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
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
        let palette = theme.extended_palette();

        let highest = self.max;
        let lowest = self.min;

        let text_size = 12.0;

        let labels = self.label_cache.draw(renderer, bounds.size(), |frame| {
            frame.fill_rectangle(
                Point::new(0.0, 0.0),
                Size::new(bounds.width, 1.0),
                if palette.is_dark {
                    palette.background.weak.color.scale_alpha(0.2)
                } else {
                    palette.background.strong.color.scale_alpha(0.2)
                },
            );

            frame.fill_rectangle(
                Point::new(0.0, 0.0),
                Size::new(1.0, bounds.height),
                if palette.is_dark {
                    palette.background.weak.color.scale_alpha(0.4)
                } else {
                    palette.background.strong.color.scale_alpha(0.4)
                },
            );

            let y_range = highest - lowest;

            let y_labels_can_fit: i32 = (bounds.height / (text_size * 2.0)) as i32;

            let mut all_labels: Vec<AxisLabel> =
                Vec::with_capacity((y_labels_can_fit + 2) as usize); // +2 for last_price and crosshair

            let rect = |y_pos: f32, label_amt: i16| {
                let label_offset = text_size + (f32::from(label_amt) * (text_size / 2.0) + 2.0);

                Rectangle {
                    x: 6.0,
                    y: y_pos - label_offset / 2.0,
                    width: bounds.width - 8.0,
                    height: label_offset,
                }
            };

            // Regular price labels (priority 1)
            let (step, rounded_lowest) = calc_price_step(highest, lowest, y_labels_can_fit, 1.0);

            let mut y = rounded_lowest;

            while y <= highest {
                let y_position = bounds.height - ((y - lowest) / y_range * bounds.height);

                if y > 0.0 {
                    let text_content = convert_to_qty_abbr(y);

                    let label = Label {
                        content: text_content,
                        background_color: None,
                        marker_color: if palette.is_dark {
                            palette.background.weak.color.scale_alpha(0.6)
                        } else {
                            palette.background.strong.color.scale_alpha(0.6)
                        },
                        text_color: palette.background.base.text,
                        text_size: 12.0,
                    };

                    all_labels.push(AxisLabel::Y(rect(y_position, 1), label, None));
                }

                y += step;
            }

            // Crosshair price (priority 3)
            if self.crosshair {
                let common_bounds = Rectangle {
                    x: self.chart_bounds.x,
                    y: bounds.y,
                    width: self.chart_bounds.width,
                    height: bounds.height,
                };

                if let Some(crosshair_pos) = cursor.position_in(common_bounds) {
                    let raw_price =
                        lowest + (y_range * (bounds.height - crosshair_pos.y) / bounds.height);
                    let rounded_price = round_to_tick(raw_price, 1.0);
                    let y_position =
                        bounds.height - ((rounded_price - lowest) / y_range * bounds.height);

                    let text_content = convert_to_qty_abbr(rounded_price);

                    let label = Label {
                        content: text_content,
                        background_color: Some(palette.secondary.base.color),
                        marker_color: palette.background.strong.color,
                        text_color: palette.secondary.base.text,
                        text_size: 12.0,
                    };

                    all_labels.push(AxisLabel::Y(rect(y_position, 1), label, None));
                }
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
            Interaction::None if cursor.is_over(bounds) => mouse::Interaction::ResizingVertically,
            _ => mouse::Interaction::default(),
        }
    }
}
