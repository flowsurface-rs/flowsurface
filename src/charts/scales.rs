pub mod linear;
pub mod timeseries;

use chrono::DateTime;
use iced::{
    mouse, theme::palette::Extended, widget::canvas::{self, Cache, Frame, Geometry}, 
    Alignment, Color, Event, Point, Rectangle, Renderer, Size, Theme
};

use crate::screen::UserTimezone;

use super::{round_to_tick, ChartBasis, Interaction, Message};

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
pub struct Label {
    pub content: String,
    pub background_color: Option<Color>,
    pub text_color: Color,
    pub text_size: f32,
}

#[derive(Debug, Clone)]
pub enum AxisLabel {
    X(Rectangle, Label),
    Y(Rectangle, Label, Option<Label>),
}

impl AxisLabel {
    fn intersects(&self, other: &AxisLabel) -> bool {
        match (self, other) {
            (AxisLabel::X(self_rect, ..), AxisLabel::X(other_rect, ..)) => {
                self_rect.intersects(other_rect)
            }
            (AxisLabel::Y(self_rect, ..), AxisLabel::Y(other_rect, ..)) => {
                self_rect.intersects(other_rect)
            }
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
            AxisLabel::X(rect, label) => {
                if let Some(background_color) = label.background_color {
                    frame.fill_rectangle(
                        Point::new(rect.x, rect.y),
                        Size::new(rect.width, rect.height),
                        background_color,
                    );
                }

                let label = canvas::Text {
                    content: label.content.clone(),
                    position: rect.center(),
                    color: label.text_color,
                    vertical_alignment: Alignment::Center.into(),
                    horizontal_alignment: Alignment::Center.into(),
                    size: label.text_size.into(),
                    ..canvas::Text::default()
                };

                frame.fill_text(label);
            }
            AxisLabel::Y(rect, price_label, timer_label) => {
                if let Some(background_color) = price_label.background_color {
                    frame.fill_rectangle(
                        Point::new(rect.x, rect.y),
                        Size::new(rect.width, rect.height),
                        background_color,
                    );
                }

                if let Some(timer_label) = timer_label {
                    let price_label = canvas::Text {
                        content: price_label.content.clone(),
                        position: Point::new(rect.x + 4.0, rect.center_y() - 6.0),
                        color: price_label.text_color,
                        size: price_label.text_size.into(),
                        vertical_alignment: Alignment::Center.into(),
                        ..canvas::Text::default()
                    };

                    frame.fill_text(price_label);

                    let timer_label = canvas::Text {
                        content: timer_label.content.clone(),
                        position: Point::new(rect.x + 4.0, rect.center_y() + 6.0),
                        color: timer_label.text_color,
                        size: timer_label.text_size.into(),
                        vertical_alignment: Alignment::Center.into(),
                        ..canvas::Text::default()
                    };

                    frame.fill_text(timer_label);
                } else {
                    let price_label = canvas::Text {
                        content: price_label.content.clone(),
                        position: Point::new(rect.x + 4.0, rect.center_y()),
                        color: price_label.text_color,
                        size: price_label.text_size.into(),
                        vertical_alignment: Alignment::Center.into(),
                        ..canvas::Text::default()
                    };

                    frame.fill_text(price_label);
                }
            }
        }
    }
}

// X-AXIS LABELS
pub struct AxisLabelsX<'a> {
    pub labels_cache: &'a Cache,
    pub crosshair: bool,
    pub max: u64,
    pub scaling: f32,
    pub translation_x: f32,
    pub basis: ChartBasis,
    pub cell_width: f32,
    pub timezone: &'a UserTimezone,
    pub chart_bounds: Rectangle,
    pub interval_keys: Vec<u64>,
}

impl AxisLabelsX<'_> {
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
            ChartBasis::Time(interval) => {
                if x <= 0.0 {
                    let diff = (-x / self.cell_width * interval as f32) as u64;
                    self.max.saturating_sub(diff)
                } else {
                    let diff = (x / self.cell_width * interval as f32) as u64;
                    self.max.saturating_add(diff)
                }
            },
            ChartBasis::Tick(_) => {
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

                            let message = Message::XScaling(difference_x * 0.2, 0.0, false);

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
        let text_size = 12.0;

        let palette = theme.extended_palette();

        let labels = self.labels_cache.draw(renderer, bounds.size(), |frame| {
            let region = self.visible_region(frame.size());

            let x_labels_can_fit = (bounds.width / 192.0) as i32;
            let mut all_labels: Vec<AxisLabel> = Vec::with_capacity(x_labels_can_fit as usize + 1); // +1 for crosshair

            match self.basis {
                ChartBasis::Tick(interval) => {
                    let x_min = self.x_to_interval(region.x);
                    let x_max = self.x_to_interval(region.x + region.width);
            
                    if x_min == x_max {
                        return;
                    }

                    if !self.interval_keys.is_empty() {
                        let last_index = self.interval_keys.len() - 1;
                        
                        let step_size = (self.interval_keys.len() as f32 / x_labels_can_fit as f32).ceil() as usize;
                        
                        let min_cell = (region.x / self.cell_width).floor() as i32;
                        let max_cell = ((region.x + region.width) / self.cell_width).ceil() as i32;
                        
                        let max_cell = max_cell.min(0);
                        let min_cell = min_cell.max(-((last_index + 1) as i32));
                        
                        for cell_index in (min_cell..=max_cell).step_by(step_size.max(1)) {
                            let offset = (-cell_index as i64) as usize;
                            if offset > last_index {
                                continue;
                            }
                            
                            let array_index = last_index - offset;
                            
                            let snapped_position = cell_index as f32 * self.cell_width;
                            let chart_x_min = region.x;
                            let chart_x_max = region.x + region.width;
                            
                            if snapped_position < chart_x_min || snapped_position > chart_x_max {
                                continue;
                            }
                            
                            let snap_ratio = (snapped_position - chart_x_min) / (chart_x_max - chart_x_min);
                            let snap_x = snap_ratio as f64 * f64::from(bounds.width);
                            
                            if snap_x.is_nan() || snap_x < 0.0 || snap_x > f64::from(bounds.width) {
                                continue;
                            }
                            
                            if let Some(timestamp) = self.interval_keys.get(array_index) {
                                let label_text = self.timezone.format_timestamp((*timestamp / 1000) as i64, interval);
                                
                                let content_width = label_text.len() as f32 * (text_size / 3.0);
                                
                                let rect = Rectangle {
                                    x: (snap_x as f32) - content_width,
                                    y: 4.0,
                                    width: 2.0 * content_width,
                                    height: bounds.height - 8.0,
                                };
                                
                                let label = Label {
                                    content: label_text,
                                    background_color: None,
                                    text_color: palette.background.base.text,
                                    text_size: 12.0,
                                };
                                
                                all_labels.push(AxisLabel::X(rect, label));
                            }
                        }
                    }

                    if self.crosshair {
                        if let Some(crosshair_pos) = cursor.position_in(self.chart_bounds) {
                            let crosshair_ratio = f64::from(crosshair_pos.x) / f64::from(self.chart_bounds.width);
                            
                            let chart_x_min = region.x;
                            let chart_x_max = region.x + region.width;
                            let crosshair_pos = chart_x_min + crosshair_ratio as f32 * region.width;
                            
                            let cell_index = (crosshair_pos / self.cell_width).round();
                            
                            let aggregation: u64 = interval.into();
                            let rounded_tick = (-cell_index as i64) * (aggregation as i64);
                            
                            // Cell 0 (latest) maps to last element, Cell -1 to second-to-last, etc.
                            let array_index = if !self.interval_keys.is_empty() {
                                let last_index = self.interval_keys.len() - 1;
                                let offset = (-cell_index as i64) as usize;
                                if offset <= last_index {
                                    Some(last_index - offset)
                                } else {
                                    None
                                }
                            } else {
                                None
                            };
                            
                            let snapped_crosshair = cell_index * self.cell_width;
                            let snap_ratio = (snapped_crosshair - chart_x_min) / (chart_x_max - chart_x_min);
                            let snap_x = snap_ratio as f64 * f64::from(bounds.width);
                            
                            if !snap_x.is_nan() && snap_x >= 0.0 && snap_x <= f64::from(bounds.width) {
                                if let Some(index) = array_index {
                                    if let Some(timestamp) = self.interval_keys.get(index) {
                                        let label_text = self.timezone.format_crosshair_timestamp(*timestamp as i64, interval);
                                        let text_content = format!("{} (#{}.{})", label_text, rounded_tick, interval);
                                        
                                        let content_width = text_content.len() as f32 * (text_size / 3.0);
                                        
                                        let rect = Rectangle {
                                            x: (snap_x as f32) - content_width,
                                            y: 4.0,
                                            width: 2.0 * content_width,
                                            height: bounds.height - 8.0,
                                        };
                                        
                                        let label = Label {
                                            content: text_content,
                                            background_color: Some(palette.secondary.base.color),
                                            text_color: palette.secondary.base.text,
                                            text_size: 12.0,
                                        };
                                        
                                        all_labels.push(AxisLabel::X(rect, label));
                                    }
                                }
                            }
                        }
                    }
                    
                    AxisLabel::filter_and_draw(&all_labels, frame);
                },
                ChartBasis::Time(timeframe) => {
                    let earliest_in_millis = self.x_to_interval(region.x);
                    let latest_in_millis = self.x_to_interval(region.x + region.width);

                    if earliest_in_millis >= latest_in_millis {
                        return;
                    }

                    // Regular time labels (priority 1)
                    let (time_step, rounded_earliest) = timeseries::calc_time_step(
                        earliest_in_millis,
                        latest_in_millis,
                        x_labels_can_fit,
                        timeframe,
                    );
                    let mut time: u64 = rounded_earliest;

                    while time <= latest_in_millis {
                        let x_position = if time >= earliest_in_millis {
                            ((time - earliest_in_millis) as f64 / (latest_in_millis - earliest_in_millis) as f64)
                                * f64::from(bounds.width)
                        } else {
                            0.0
                        };

                        if x_position > 0.0 && x_position <= f64::from(bounds.width) {
                            let text_content = self.timezone
                                .format_timestamp(
                                    (time / 1000) as i64, 
                                    timeframe,
                                );

                            let content_width = text_content.len() as f32 * (text_size / 3.0);

                            let rect = Rectangle {
                                x: (x_position as f32) - content_width,
                                y: 4.0,
                                width: 2.0 * content_width,
                                height: bounds.height - 8.0,
                            };

                            let label = Label {
                                content: text_content,
                                background_color: None,
                                text_color: palette.background.base.text,
                                text_size: 12.0,
                            };

                            all_labels.push(AxisLabel::X(rect, label));
                        }
                        time += time_step;
                    }

                    // Crosshair label (priority 2)
                    if self.crosshair {
                        if let Some(crosshair_pos) = cursor.position_in(self.chart_bounds) {
                            let crosshair_ratio = f64::from(crosshair_pos.x) / f64::from(bounds.width);
                            let crosshair_millis = earliest_in_millis as f64
                                + crosshair_ratio * (latest_in_millis - earliest_in_millis) as f64;

                            let (snap_ratio, text_content) = {
                                if let Some(crosshair_time) =
                                    DateTime::from_timestamp_millis(crosshair_millis as i64)
                                {
                                    let rounded_timestamp = (crosshair_time.timestamp_millis() as f64
                                        / (timeframe as f64)).round() as u64 * timeframe;

                                    let snap_ratio = (rounded_timestamp as f64 - earliest_in_millis as f64)
                                        / (latest_in_millis as f64 - earliest_in_millis as f64);
                                    
                                    (
                                        snap_ratio, 
                                        self.timezone
                                            .format_crosshair_timestamp(
                                                rounded_timestamp as i64, 
                                                timeframe,
                                            ),
                                    )
                                } else {
                                    (0.0, String::new())
                                }
                            };

                            let snap_x = snap_ratio * f64::from(bounds.width);

                            if snap_x.is_nan() {
                                return;
                            }

                            let content_width = text_content.len() as f32 * (text_size / 3.0);

                            let rect = Rectangle {
                                x: (snap_x as f32) - content_width,
                                y: 4.0,
                                width: 2.0 * (content_width),
                                height: bounds.height - 8.0,
                            };

                            let label = Label {
                                content: text_content,
                                background_color: Some(palette.secondary.base.color),
                                text_color: palette.secondary.base.text,
                                text_size: 12.0,
                            };

                            all_labels.push(AxisLabel::X(rect, label));
                        }
                    }

                    AxisLabel::filter_and_draw(&all_labels, frame);
                }
            }
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
    pub crosshair: bool,
    pub translation_y: f32,
    pub scaling: f32,
    pub min: f32,
    pub last_price: Option<PriceInfoLabel>,
    pub tick_size: f32,
    pub decimals: usize,
    pub cell_height: f32,
    pub basis: ChartBasis,
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

            frame.fill_rectangle(
                Point::new(0.0, 0.0),
                Size::new(1.0, bounds.height),
                palette.background.strong.color.scale_alpha(0.4),
            );

            let highest = self.y_to_price(region.y);
            let lowest = self.y_to_price(region.y + region.height);

            let range = highest - lowest;

            let mut all_labels = linear::generate_labels(
                bounds,
                lowest,
                highest,
                text_size,
                palette.background.base.text,
                self.tick_size,
                Some(self.decimals),
            );

            // Last price (priority 2)
            if let Some(label) = self.last_price {
                let candle_close_label = match self.basis {
                    ChartBasis::Time(timeframe) => {
                        let current_time = chrono::Utc::now().timestamp_millis() as u64;
                        let next_kline_open =
                            (current_time / timeframe + 1) * timeframe;
                
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
                
                            Some(Label {
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
                    },
                    ChartBasis::Tick(_) => {
                        None
                    }
                };

                let (price, color) = label.get_with_color(palette);
                
                let price_label = Label {
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
                            palette.primary.weak.text
                        }
                    },
                    text_size: 12.0,
                };

                let y_pos = bounds.height - ((price - lowest) / range * bounds.height);
                let content_amt = if candle_close_label.is_some() { 2 } else { 1 };

                all_labels.push(AxisLabel::Y(
                    calc_label_rect(y_pos, content_amt, text_size, bounds),
                    price_label,
                    candle_close_label,
                ));
            }

            // Crosshair price (priority 3)
            if self.crosshair {
                if let Some(crosshair_pos) = cursor.position_in(self.chart_bounds) {
                    let rounded_price = round_to_tick(
                        lowest + (range * (bounds.height - crosshair_pos.y) / bounds.height), 
                        self.tick_size
                    );
                    let y_position =
                        bounds.height - ((rounded_price - lowest) / range * bounds.height);

                    let label = Label {
                        content: format!("{:.*}", self.decimals, rounded_price),
                        background_color: Some(palette.secondary.base.color),
                        text_color: palette.secondary.base.text,
                        text_size: 12.0,
                    };

                    all_labels.push(
                        AxisLabel::Y(
                            calc_label_rect(y_position, 1, text_size, bounds),
                            label, 
                            None
                        )
                    );
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

// other helpers
#[derive(Debug, Clone, Copy)]
pub enum PriceInfoLabel {
    Up(f32),
    Down(f32),
    Neutral(f32),
}

impl PriceInfoLabel {
    pub fn get_with_color(&self, palette: &Extended) -> (f32, iced::Color) {
        match self {
            PriceInfoLabel::Up(p) => (*p, palette.success.base.color),
            PriceInfoLabel::Down(p) => (*p, palette.danger.base.color),
            PriceInfoLabel::Neutral(p) => (*p, palette.primary.weak.color),
        }
    }
}