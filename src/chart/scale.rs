pub mod linear;
pub mod timeseries;

use crate::{chart::TEXT_SIZE, style::AZERET_MONO};

use super::{Basis, Interaction, Message};
use data::{chart::Autoscale, util::round_to_tick};
use iced::{
    Alignment, Color, Event, Point, Rectangle, Renderer, Size, Theme, mouse,
    theme::palette::Extended,
    widget::canvas::{self, Cache, Frame, Geometry},
};

const REGULAR_LABEL_WIDTH: f32 = TEXT_SIZE * 6.0;

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
        is_crosshair: bool,
        palette: &Extended,
    ) -> Self {
        let content_width = text_content.len() as f32 * (TEXT_SIZE / 2.6);

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
            text_color: if is_crosshair {
                palette.secondary.base.text
            } else {
                palette.background.base.text
            },
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
        for i in (0..labels.len()).rev() {
            let should_draw = labels[i + 1..]
                .iter()
                .all(|existing| !existing.intersects(&labels[i]));

            if should_draw {
                labels[i].draw(frame);
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

// X-AXIS LABELS
pub struct AxisLabelsX<'a> {
    pub labels_cache: &'a Cache,
    pub max: u64,
    pub scaling: f32,
    pub translation_x: f32,
    pub basis: Basis,
    pub cell_width: f32,
    pub timezone: data::UserTimezone,
    pub chart_bounds: Rectangle,
    pub interval_keys: Option<Vec<u64>>,
    pub autoscaling: Option<Autoscale>,
}

impl AxisLabelsX<'_> {
    fn calc_crosshair_pos(&self, cursor_pos: Point, region: Rectangle) -> (f32, f32, i32) {
        let crosshair_ratio = f64::from(cursor_pos.x) / f64::from(self.chart_bounds.width);
        let chart_x_min = region.x;
        let crosshair_pos = chart_x_min + crosshair_ratio as f32 * region.width;
        let cell_index = (crosshair_pos / self.cell_width).round();

        (crosshair_pos, crosshair_ratio as f32, cell_index as i32)
    }

    fn generate_crosshair(
        &self,
        cursor_pos: Point,
        region: Rectangle,
        bounds: Rectangle,
        palette: &Extended,
    ) -> Option<AxisLabel> {
        match self.basis {
            Basis::Tick(interval) => {
                let Some(interval_keys) = &self.interval_keys else {
                    return None;
                };

                let (crosshair_pos, _, cell_index) = self.calc_crosshair_pos(cursor_pos, region);

                let chart_x_min = region.x;
                let chart_x_max = region.x + region.width;

                let snapped_position = (crosshair_pos / self.cell_width).round() * self.cell_width;
                let snap_ratio = (snapped_position - chart_x_min) / (chart_x_max - chart_x_min);
                let snap_x = snap_ratio * bounds.width;

                if snap_x.is_nan() || snap_x < 0.0 || snap_x > bounds.width {
                    return None;
                }

                let last_index = interval_keys.len() - 1;
                let offset = i64::from(-cell_index) as usize;
                if offset > last_index {
                    return None;
                }

                let array_index = last_index - offset;

                if let Some(timestamp) = interval_keys.get(array_index) {
                    let text_content = self
                        .timezone
                        .format_crosshair_timestamp(*timestamp as i64, interval.0.into());

                    return Some(AxisLabel::new_x(
                        snap_x,
                        text_content,
                        bounds,
                        true,
                        palette,
                    ));
                }
            }
            Basis::Time(timeframe) => {
                let (_, crosshair_ratio, _) = self.calc_crosshair_pos(cursor_pos, region);

                let x_min = self.x_to_interval(region.x);
                let x_max = self.x_to_interval(region.x + region.width);

                let crosshair_millis =
                    x_min as f64 + f64::from(crosshair_ratio) * (x_max as f64 - x_min as f64);

                let interval = timeframe.to_milliseconds();

                let crosshair_time =
                    chrono::DateTime::from_timestamp_millis(crosshair_millis as i64)?;
                let rounded_timestamp =
                    (crosshair_time.timestamp_millis() as f64 / (interval as f64)).round() as u64
                        * interval;

                let snap_ratio =
                    (rounded_timestamp as f64 - x_min as f64) / (x_max as f64 - x_min as f64);

                let snap_x = snap_ratio * f64::from(bounds.width);
                if snap_x.is_nan() || snap_x < 0.0 || snap_x > f64::from(bounds.width) {
                    return None;
                }

                let text_content = self
                    .timezone
                    .format_crosshair_timestamp(rounded_timestamp as i64, interval);

                return Some(AxisLabel::new_x(
                    snap_x as f32,
                    text_content,
                    bounds,
                    true,
                    palette,
                ));
            }
        }

        None
    }

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

    fn x_to_interval(&self, x: f32) -> u64 {
        match self.basis {
            Basis::Time(timeframe) => {
                let interval = timeframe.to_milliseconds() as f64;

                if x <= 0.0 {
                    let diff = (f64::from(-x / self.cell_width) * interval) as u64;
                    self.max.saturating_sub(diff)
                } else {
                    let diff = (f64::from(x / self.cell_width) * interval) as u64;
                    self.max.saturating_add(diff)
                }
            }
            Basis::Tick(_) => {
                let tick = -(x / self.cell_width);
                tick.round() as u64
            }
        }
    }
}

impl canvas::Program<Message> for AxisLabelsX<'_> {
    type State = Interaction;

    fn update(
        &self,
        interaction: &mut Interaction,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if let Event::Mouse(mouse::Event::ButtonReleased(_)) = event {
            *interaction = Interaction::None;
        }

        let cursor_position = cursor.position_in(bounds)?;

        if let Event::Mouse(mouse_event) = event {
            match mouse_event {
                mouse::Event::ButtonPressed(mouse::Button::Left) => {
                    *interaction = Interaction::Zoomin {
                        last_position: cursor_position,
                    };
                }
                mouse::Event::CursorMoved { .. } => {
                    if let Interaction::Zoomin {
                        ref mut last_position,
                    } = *interaction
                    {
                        let difference_x = last_position.x - cursor_position.x;

                        if difference_x.abs() > 1.0 {
                            *last_position = cursor_position;

                            let delta = if self.autoscaling == Some(Autoscale::FitToVisible) {
                                difference_x * 0.05
                            } else {
                                difference_x * 0.2
                            };

                            let message = Message::XScaling(delta, 0.0, false);

                            return Some(canvas::Action::publish(message).and_capture());
                        }
                    }
                }
                mouse::Event::WheelScrolled { delta } => match delta {
                    mouse::ScrollDelta::Lines { y, .. } | mouse::ScrollDelta::Pixels { y, .. } => {
                        let message = Message::XScaling(
                            *y,
                            {
                                if let Some(cursor_to_center) =
                                    cursor.position_from(bounds.center())
                                {
                                    cursor_to_center.x
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
        let palette = theme.extended_palette();

        let labels = self.labels_cache.draw(renderer, bounds.size(), |frame| {
            let region = self.visible_region(frame.size());

            let target_spacing = REGULAR_LABEL_WIDTH * 2.0;
            let target_count = (bounds.width / target_spacing).floor() as usize;

            let label_count = target_count.max(2);

            let mut labels: Vec<AxisLabel> = Vec::with_capacity(label_count + 1); // +1 for crosshair

            match self.basis {
                Basis::Tick(_) => {
                    if let Some(interval_keys) = &self.interval_keys {
                        let last_idx = interval_keys.len() - 1;
                        let mut last_x: Option<f32> = None;
                        for (i, timestamp) in interval_keys.iter().enumerate() {
                            let cell_index = -(last_idx as i32) + i as i32;
                            let x_position = cell_index as f32 * self.cell_width;

                            let x_min_region = region.x;
                            let x_max_region = region.x + region.width;
                            let snap_ratio = if (x_max_region - x_min_region).abs() < f32::EPSILON {
                                0.5
                            } else {
                                (x_position - x_min_region) / (x_max_region - x_min_region)
                            };
                            let snap_x = snap_ratio * bounds.width;

                            if last_x.is_none_or(|lx| (snap_x - lx).abs() >= target_spacing) {
                                let label_text = self.timezone.format_timestamp(
                                    (*timestamp / 1000) as i64,
                                    exchange::Timeframe::MS100,
                                );
                                labels.push(AxisLabel::new_x(
                                    snap_x, label_text, bounds, false, palette,
                                ));
                                last_x = Some(snap_x);
                            }
                        }
                    }
                }
                Basis::Time(timeframe) => {
                    let x_min_region = self.x_to_interval(region.x);
                    let x_max_region = self.x_to_interval(region.x + region.width);

                    let generated_labels = timeseries::generate_time_labels(
                        timeframe,
                        self.timezone,
                        bounds,
                        x_min_region,
                        x_max_region,
                        label_count as i32,
                        palette,
                    );

                    labels.extend(generated_labels);
                }
            }

            if let Some(cursor_pos) = cursor.position_in(self.chart_bounds)
                && let Some(label) = self.generate_crosshair(cursor_pos, region, bounds, palette)
            {
                labels.push(label);
            }

            AxisLabel::filter_and_draw(&labels, frame);
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
            Interaction::Panning { .. } => mouse::Interaction::None,
            Interaction::Zoomin { .. } => mouse::Interaction::ResizingHorizontally,
            Interaction::None if cursor.is_over(bounds) => mouse::Interaction::ResizingHorizontally,
            _ => mouse::Interaction::default(),
        }
    }
}

// Y-AXIS LABELS
pub struct AxisLabelsY<'a> {
    pub labels_cache: &'a Cache,
    pub translation_y: f32,
    pub scaling: f32,
    pub min: f32,
    pub last_price: Option<linear::PriceInfoLabel>,
    pub tick_size: f32,
    pub decimals: usize,
    pub cell_height: f32,
    pub basis: Basis,
    pub chart_bounds: Rectangle,
}

impl AxisLabelsY<'_> {
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

    fn y_to_price(&self, y: f32) -> f32 {
        self.min - (y / self.cell_height) * self.tick_size
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
        if let Event::Mouse(mouse::Event::ButtonReleased(_)) = event {
            *interaction = Interaction::None;
        }

        let cursor_position = cursor.position_in(bounds)?;

        if let Event::Mouse(mouse_event) = event {
            match mouse_event {
                mouse::Event::ButtonPressed(mouse::Button::Left) => {
                    *interaction = Interaction::Zoomin {
                        last_position: cursor_position,
                    };
                }
                mouse::Event::CursorMoved { .. } => {
                    if let Interaction::Zoomin {
                        ref mut last_position,
                    } = *interaction
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
        let text_size = 12.0;
        let palette = theme.extended_palette();

        let labels = self.labels_cache.draw(renderer, bounds.size(), |frame| {
            let region = self.visible_region(frame.size());

            let highest = self.y_to_price(region.y);
            let lowest = self.y_to_price(region.y + region.height);

            let range = highest - lowest;

            let mut all_labels = linear::generate_labels(
                bounds,
                lowest,
                highest,
                text_size,
                palette.background.base.text,
                Some(self.decimals),
            );

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
                                text_size: 11.0,
                            })
                        } else {
                            None
                        }
                    }
                    Basis::Tick(_) => None,
                };

                let (price, color) = label.get_with_color(palette);
                let price = price.to_f32();

                let price_label = LabelContent {
                    content: format!("{:.*}", self.decimals, price),
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
                    text_size: 12.0,
                };

                let y_pos = bounds.height - ((price - lowest) / range * bounds.height);
                let content_amt = if candle_close_label.is_some() { 2 } else { 1 };

                all_labels.push(AxisLabel::Y {
                    bounds: calc_label_rect(y_pos, content_amt, text_size, bounds),
                    value_label: price_label,
                    timer_label: candle_close_label,
                });
            }

            // Crosshair price (priority 3)
            if let Some(crosshair_pos) = cursor.position_in(self.chart_bounds) {
                let rounded_price = round_to_tick(
                    lowest + (range * (bounds.height - crosshair_pos.y) / bounds.height),
                    self.tick_size,
                );
                let y_position = bounds.height - ((rounded_price - lowest) / range * bounds.height);

                let label = LabelContent {
                    content: format!("{:.*}", self.decimals, rounded_price),
                    background_color: Some(palette.secondary.base.color),
                    text_color: palette.secondary.base.text,
                    text_size: 12.0,
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
            Interaction::None if cursor.is_over(bounds) => mouse::Interaction::ResizingVertically,
            _ => mouse::Interaction::default(),
        }
    }
}
