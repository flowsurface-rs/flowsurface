use chrono::DateTime;
use iced::{
    alignment,
    mouse::{self},
    widget::{button, canvas::{LineDash, Path, Stroke}, column, container, row, text, Space},
    Alignment, Color, Element, Length, Point, Rectangle, Renderer, Size, Task, Theme, Vector,
};
use iced::widget::canvas::{self, Canvas, Event, Frame, Geometry, Cache};
use indicators::Indicator;
use uuid::Uuid;

use crate::{
    data_providers::{fetcher::{FetchRange, ReqError, RequestHandler}, TickerInfo},
    screen::UserTimezone,
    style,
    tooltip::{self, tooltip},
};
pub mod candlestick;
pub mod footprint;
pub mod heatmap;
pub mod indicators;
pub mod timeandsales;

// time steps in ms, to be used for x-axis labels on candlesticks and footprint charts
const M1_TIME_STEPS: [i64; 9] = [
    1000 * 60 * 720, // 12 hour
    1000 * 60 * 180, // 3 hour
    1000 * 60 * 60,  // 1 hour
    1000 * 60 * 30,  // 30 min
    1000 * 60 * 15,  // 15 min
    1000 * 60 * 10,  // 10 min
    1000 * 60 * 5,   // 5 min
    1000 * 60 * 2,   // 2 min
    60 * 1000,       // 1 min
];
const M3_TIME_STEPS: [i64; 9] = [
    1000 * 60 * 1440, // 24 hour
    1000 * 60 * 720,  // 12 hour
    1000 * 60 * 180,  // 6 hour
    1000 * 60 * 120,  // 2 hour
    1000 * 60 * 60,   // 1 hour
    1000 * 60 * 30,   // 30 min
    1000 * 60 * 15,   // 15 min
    1000 * 60 * 9,    // 9 min
    1000 * 60 * 3,    // 3 min
];
const M5_TIME_STEPS: [i64; 9] = [
    1000 * 60 * 1440, // 24 hour
    1000 * 60 * 720,  // 12 hour
    1000 * 60 * 480,  // 8 hour
    1000 * 60 * 240,  // 4 hour
    1000 * 60 * 120,  // 2 hour
    1000 * 60 * 60,   // 1 hour
    1000 * 60 * 30,   // 30 min
    1000 * 60 * 15,   // 15 min
    1000 * 60 * 5,    // 5 min
];
const HOURLY_TIME_STEPS: [i64; 8] = [
    1000 * 60 * 5760, // 96 hour
    1000 * 60 * 2880, // 48 hour
    1000 * 60 * 1440, // 24 hour
    1000 * 60 * 720,  // 12 hour
    1000 * 60 * 480,  // 8 hour
    1000 * 60 * 240,  // 4 hour
    1000 * 60 * 120,  // 2 hour
    1000 * 60 * 60,   // 1 hour
];
const MS_TIME_STEPS: [i64; 8] = [
    1000 * 30,
    1000 * 10,
    1000 * 5,
    1000 * 2,
    1000,
    500,
    200,
    100,
];

#[derive(Debug, Clone, Copy)]
pub enum Interaction {
    None,
    Zoomin { last_position: Point },
    Panning { translation: Vector, start: Point },
    ResizingCanvas { height: u16, start: Point },
}

impl Default for Interaction {
    fn default() -> Self {
        Self::None
    }
}

pub trait ChartConstants {
    const MIN_SCALING: f32;
    const MAX_SCALING: f32;
    const MIN_CELL_WIDTH: f32;
    const MAX_CELL_WIDTH: f32;
    const MIN_CELL_HEIGHT: f32;
    const MAX_CELL_HEIGHT: f32;
    const DEFAULT_CELL_WIDTH: f32;
}

#[derive(Debug, Clone)]
pub enum Message {
    Translated(Vector),
    Scaled(f32, Vector),
    AutoscaleToggle,
    CrosshairToggle,
    CrosshairMoved,
    YScaling(f32, f32, bool),
    XScaling(f32, f32, bool),
    BoundsChanged(Rectangle),
    ResizingCanvas(u16),
    NewDataRange(Uuid, FetchRange),
}

trait Chart: ChartConstants + canvas::Program<Message> {
    fn get_common_data(&self) -> &CommonChartData;

    fn get_common_data_mut(&mut self) -> &mut CommonChartData;

    fn update_chart(&mut self, message: &Message) -> Task<Message>;

    fn canvas_interaction(
        &self,
        interaction: &mut Interaction,
        event: Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>>;

    fn view_indicator<I: Indicator>(
        &self, 
        enabled: &[I], 
        ticker_info: Option<TickerInfo>
    ) -> Option<Element<Message>>;

    fn get_visible_timerange(&self) -> (i64, i64);
}

fn canvas_interaction<T: Chart>(
    chart: &T,
    interaction: &mut Interaction,
    event: Event,
    bounds: Rectangle,
    cursor: mouse::Cursor,
) -> Option<canvas::Action<Message>> {
    if let Event::Mouse(mouse::Event::ButtonReleased(_)) = event {
        *interaction = Interaction::None;
    }

    if chart.get_common_data().bounds != bounds {
        return Some(canvas::Action::publish(Message::BoundsChanged(
            bounds,
        )));
    }

    let cursor_position = cursor.position_in(bounds)?;

    match event {
        Event::Mouse(mouse_event) => {
            let chart_state = chart.get_common_data();

            match mouse_event {
                mouse::Event::ButtonPressed(button) => {
                    let message = match button {
                        mouse::Button::Left => {
                            if cursor.is_over(Rectangle {
                                x: bounds.x,
                                y: bounds.y,
                                width: bounds.width,
                                height: bounds.height - 8.0,
                            }) {
                                *interaction = Interaction::Panning {
                                    translation: chart_state.translation,
                                    start: cursor_position,
                                };
                                None
                            } else if cursor.is_over(Rectangle {
                                x: bounds.x,
                                y: bounds.y + bounds.height - 8.0,
                                width: bounds.width,
                                height: 8.0,
                            }) {
                                *interaction = Interaction::ResizingCanvas {
                                    start: cursor_position,
                                    height: chart_state.indicators_height,
                                };
                                None
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };

                    Some(
                        message
                            .map_or(canvas::Action::request_redraw(), canvas::Action::publish)
                            .and_capture(),
                    )
                }
                mouse::Event::CursorMoved { .. } => {
                    let message = match *interaction {
                        Interaction::Panning { translation, start } => Some(Message::Translated(
                            translation + (cursor_position - start) * (1.0 / chart_state.scaling),
                        )),
                        Interaction::None => {
                            if chart_state.crosshair && cursor.is_over(bounds) {
                                Some(Message::CrosshairMoved)
                            } else {
                                None
                            }
                        }
                        Interaction::ResizingCanvas { start, height } => {
                            let diff =
                                ((cursor_position.y - start.y) / (bounds.height / 200.0)) as i16;
                            let height = (height as i16 - diff).clamp(8, 60);
                            Some(Message::ResizingCanvas(height as u16))
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
                mouse::Event::WheelScrolled { delta } => {
                    let cursor_to_center = cursor.position_from(bounds.center())?;

                    let y = match delta {
                        mouse::ScrollDelta::Lines { y, .. }
                        | mouse::ScrollDelta::Pixels { y, .. } => y,
                    };

                    // max scaling case
                    if (y > 0.0 && chart_state.scaling == T::MAX_SCALING)
                        && (chart_state.cell_width < T::MAX_CELL_WIDTH)
                    {
                        return Some(
                            canvas::Action::publish(Message::XScaling(
                                y / 2.0,
                                cursor_to_center.x,
                                true,
                            ))
                            .and_capture(),
                        );
                    }

                    // min scaling case
                    if (y < 0.0 && chart_state.scaling == T::MIN_SCALING)
                        && (chart_state.cell_width > T::MIN_CELL_WIDTH)
                    {
                        return Some(
                            canvas::Action::publish(Message::XScaling(
                                y / 2.0,
                                cursor_to_center.x,
                                true,
                            ))
                            .and_capture(),
                        );
                    }

                    // normal scaling case
                    if (y < 0.0 && chart_state.scaling > T::MIN_SCALING)
                        || (y > 0.0 && chart_state.scaling < T::MAX_SCALING)
                    {
                        let old_scaling = chart_state.scaling;
                        let scaling = (chart_state.scaling * (1.0 + y / 30.0))
                            .clamp(T::MIN_SCALING, T::MAX_SCALING);

                        let translation = {
                            let factor = scaling - old_scaling;
                            let vector_diff = Vector::new(
                                cursor_to_center.x * factor / (old_scaling * old_scaling),
                                cursor_to_center.y * factor / (old_scaling * old_scaling),
                            );

                            chart_state.translation - vector_diff
                        };

                        return Some(
                            canvas::Action::publish(Message::Scaled(scaling, translation))
                                .and_capture(),
                        );
                    }

                    Some(canvas::Action::capture())
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn update_chart<T: Chart>(chart: &mut T, message: &Message) -> Task<Message> {
    let chart_state = chart.get_common_data_mut();

    match message {
        Message::Translated(translation) => {
            chart_state.translation = *translation;
            chart_state.autoscale = false;
        }
        Message::Scaled(scaling, translation) => {
            chart_state.scaling = *scaling;
            chart_state.translation = *translation;

            chart_state.autoscale = false;
        }
        Message::AutoscaleToggle => {
            chart_state.autoscale = !chart_state.autoscale;
            if chart_state.autoscale {
                chart_state.scaling = 1.0;
                chart_state.cell_width = T::DEFAULT_CELL_WIDTH;
            }
        }
        Message::CrosshairToggle => {
            chart_state.crosshair = !chart_state.crosshair;
        }
        Message::XScaling(delta, cursor_to_center_x, is_wheel_scroll) => {
            if *delta < 0.0 && chart_state.cell_width > T::MIN_CELL_WIDTH
                || *delta > 0.0 && chart_state.cell_width < T::MAX_CELL_WIDTH
            {
                let (old_scaling, old_translation_x) =
                    { (chart_state.scaling, chart_state.translation.x) };

                let zoom_factor = if *is_wheel_scroll { 30.0 } else { 90.0 };

                let new_width = (chart_state.cell_width * (1.0 + delta / zoom_factor))
                    .clamp(T::MIN_CELL_WIDTH, T::MAX_CELL_WIDTH);

                let cursor_chart_x = cursor_to_center_x / old_scaling - old_translation_x;

                let cursor_time = chart_state.x_to_time(cursor_chart_x);

                chart_state.cell_width = new_width;

                let new_cursor_x = chart_state.time_to_x(cursor_time);

                chart_state.translation.x -= new_cursor_x - cursor_chart_x;

                chart_state.autoscale = false;
            }
        }
        Message::YScaling(delta, cursor_to_center_y, is_wheel_scroll) => {
            let max_scaled_height = chart_state.base_range * T::MAX_CELL_HEIGHT;
            let min_scaled_height = chart_state.base_range * T::MIN_CELL_HEIGHT;

            if *delta < 0.0 && chart_state.cell_height > min_scaled_height
                || *delta > 0.0 && chart_state.cell_height < max_scaled_height
            {
                let (old_scaling, old_translation_y) =
                    { (chart_state.scaling, chart_state.translation.y) };

                let zoom_factor = if *is_wheel_scroll { 30.0 } else { 90.0 };

                let new_height = (chart_state.cell_height * (1.0 + delta / zoom_factor))
                    .clamp(min_scaled_height, max_scaled_height);

                let cursor_chart_y = cursor_to_center_y / old_scaling - old_translation_y;

                let cursor_price = chart_state.y_to_price(cursor_chart_y);

                chart_state.cell_height = new_height;

                let new_cursor_y = chart_state.price_to_y(cursor_price);

                chart_state.translation.y -= new_cursor_y - cursor_chart_y;

                if *is_wheel_scroll {
                    chart_state.autoscale = false;
                }
            }
        }
        Message::BoundsChanged(bounds) => {
            chart_state.bounds = *bounds;
        }
        Message::ResizingCanvas(y) => {
            chart_state.indicators_height = *y;
        }
        _ => {}
    }

    Task::none()
}

fn view_chart<'a, T: Chart, I: Indicator>(
    chart: &'a T, 
    indicators: &'a [I], 
    ticker_info: Option<TickerInfo>,
) -> Element<'a, Message> {
    let chart_state = chart.get_common_data();

    if chart_state.latest_x == 0 || chart_state.base_price_y == 0.0 {
        return column![
            Space::new(Length::Fill, Length::Fill),
            text("Loading...").size(16).center(),
            Space::new(Length::Fill, Length::Fill)
        ]
        .align_x(alignment::Horizontal::Center)
        .padding(5)
        .into();
    }

    let chart_canvas = Canvas::new(chart)
        .width(Length::Fill)
        .height(Length::Fill);

    let axis_labels_x = Canvas::new(AxisLabelsX {
        labels_cache: &chart_state.cache.x_labels,
        scaling: chart_state.scaling,
        translation_x: chart_state.translation.x,
        max: chart_state.latest_x,
        crosshair: chart_state.crosshair,
        timeframe: chart_state.timeframe as u32,
        cell_width: chart_state.cell_width,
        timezone: &chart_state.timezone,
        chart_bounds: chart_state.bounds,
    })
    .width(Length::Fill)
    .height(Length::Fill);

    let axis_labels_y = Canvas::new(AxisLabelsY {
        labels_cache: &chart_state.cache.y_labels,
        translation_y: chart_state.translation.y,
        scaling: chart_state.scaling,
        decimals: chart_state.decimals,
        min: chart_state.base_price_y,
        last_price: chart_state.last_price,
        crosshair: chart_state.crosshair,
        tick_size: chart_state.tick_size,
        cell_height: chart_state.cell_height,
        timeframe: chart_state.timeframe as u32,
        chart_bounds: chart_state.bounds,
    })
    .width(Length::Fill)
    .height(Length::Fill);

    let chart_controls = {
        let center_button = button(text("C").size(10).align_x(alignment::Horizontal::Center))
            .width(Length::Shrink)
            .height(Length::Fill)
            .on_press(Message::AutoscaleToggle)
            .style(move |theme, status| {
                style::button_transparent(theme, status, chart_state.autoscale)
            });

        let crosshair_button = button(text("+").size(10).align_x(alignment::Horizontal::Center))
            .width(Length::Shrink)
            .height(Length::Fill)
            .on_press(Message::CrosshairToggle)
            .style(move |theme, status| {
                style::button_transparent(theme, status, chart_state.crosshair)
            });

        container(
            row![
                Space::new(Length::Fill, Length::Fill),
                tooltip(center_button, Some("Center Latest"), tooltip::Position::Top),
                tooltip(crosshair_button, Some("Crosshair"), tooltip::Position::Top),
            ]
            .spacing(2),
        )
        .padding(2)
    };

    let mut indicators_row = row![];
    if !indicators.is_empty() {
        indicators_row = indicators_row
            .push_maybe(
                chart.view_indicator(indicators, ticker_info)
            )
    }

    column![
        row![
            container(chart_canvas)
                .width(Length::FillPortion(10))
                .height(Length::FillPortion(120)),
            container(axis_labels_y)
                .width(Length::Fixed(60.0 + (chart_state.decimals as f32 * 2.0)))
                .height(Length::FillPortion(120))
        ],
        indicators_row,
        row![
            container(axis_labels_x)
                .width(Length::FillPortion(10))
                .height(Length::Fixed(26.0)),
            chart_controls
                .width(Length::Fixed(60.0 + (chart_state.decimals as f32 * 2.0)))
                .height(Length::Fixed(26.0))
        ]
    ]
    .into()
}

fn request_fetch(handler: &mut RequestHandler, range: FetchRange) -> Option<Task<Message>> {
    match handler.add_request(range) {
        Ok(req_id) => Some(Task::done(Message::NewDataRange(req_id, range))),
        Err(e) => {
            match e {
                ReqError::Overlaps => log::warn!("Request overlaps with existing request"),
                ReqError::Failed(msg) => log::warn!("Request already failed: {}", msg),
                ReqError::Completed => log::warn!("Request already completed"),
            }
            None
        }
    }
}

#[derive(Default)]
pub struct Caches {
    main: Cache,
    x_labels: Cache,
    y_labels: Cache,
    crosshair: Cache,
}

impl Caches {
    fn clear_all(&self) {
        self.main.clear();
        self.x_labels.clear();
        self.y_labels.clear();
        self.crosshair.clear();
    }
}

pub struct CommonChartData {
    cache: Caches,

    crosshair: bool,
    bounds: Rectangle,

    autoscale: bool,

    translation: Vector,
    scaling: f32,
    cell_width: f32,
    cell_height: f32,

    base_range: f32,

    timezone: UserTimezone,
    last_price: Option<PriceInfoLabel>,

    base_price_y: f32,
    latest_x: i64,
    timeframe: u64,
    tick_size: f32,
    decimals: usize,

    indicators_height: u16,

    already_fetching: bool,
}

impl Default for CommonChartData {
    fn default() -> Self {
        CommonChartData {
            cache: Caches::default(),
            crosshair: true,
            translation: Vector::default(),
            timezone: UserTimezone::default(),
            bounds: Rectangle::default(),
            last_price: None,
            base_range: 1.0,
            scaling: 1.0,
            autoscale: true,
            cell_width: 40.0,
            cell_height: 30.0,
            base_price_y: 0.0,
            latest_x: 0,
            timeframe: 0,
            tick_size: 0.0,
            decimals: 0,
            indicators_height: 0,
            already_fetching: false,
        }
    }
}

impl CommonChartData {
    fn visible_region(&self, size: Size) -> Rectangle {
        let width = size.width / self.scaling;
        let height = size.height / self.scaling;

        Rectangle {
            x: -self.translation.x - width / 2.0,
            y: -self.translation.y - height / 2.0,
            width,
            height,
        }
    }

    fn time_to_x(&self, time: i64) -> f32 {
        ((time - self.latest_x) as f32 / self.timeframe as f32) * self.cell_width
    }

    fn x_to_time(&self, x: f32) -> i64 {
        self.latest_x + ((x / self.cell_width) * self.timeframe as f32) as i64
    }

    fn price_to_y(&self, price: f32) -> f32 {
        ((self.base_price_y - price) / self.tick_size) * self.cell_height
    }

    fn y_to_price(&self, y: f32) -> f32 {
        self.base_price_y - (y / self.cell_height) * self.tick_size
    }

    fn draw_crosshair(
        &self,
        frame: &mut Frame,
        theme: &Theme,
        bounds: Size,
        cursor_position: Point,
    ) -> (f32, i64) {
        let region = self.visible_region(bounds);

        let palette = theme.extended_palette();

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

        // Horizontal price line
        let highest = self.y_to_price(region.y);
        let lowest = self.y_to_price(region.y + region.height);

        let crosshair_ratio = cursor_position.y / bounds.height;
        let crosshair_price = highest + crosshair_ratio * (lowest - highest);

        let rounded_price = round_to_tick(crosshair_price, self.tick_size);
        let snap_ratio = (rounded_price - highest) / (lowest - highest);

        frame.stroke(
            &Path::line(
                Point::new(0.0, snap_ratio * bounds.height),
                Point::new(bounds.width, snap_ratio * bounds.height),
            ),
            dashed_line,
        );

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

        // return incase consumer needs them
        (rounded_price, rounded_timestamp)
    }
}

// X-AXIS LABELS
struct AxisLabelsX<'a> {
    labels_cache: &'a Cache,
    crosshair: bool,
    max: i64,
    scaling: f32,
    translation_x: f32,
    timeframe: u32,
    cell_width: f32,
    timezone: &'a UserTimezone,
    chart_bounds: Rectangle,
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

    fn x_to_time(&self, x: f32) -> i64 {
        let time_per_cell = self.timeframe;
        self.max + ((x / self.cell_width) * time_per_cell as f32) as i64
    }
}

impl canvas::Program<Message> for AxisLabelsX<'_> {
    type State = Interaction;

    fn update(
        &self,
        interaction: &mut Interaction,
        event: Event,
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
                            y,
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

            let earliest_in_millis = self.x_to_time(region.x);
            let latest_in_millis = self.x_to_time(region.x + region.width);

            let x_labels_can_fit = (bounds.width / 192.0) as i32;

            let mut all_labels: Vec<AxisLabel> = Vec::with_capacity(x_labels_can_fit as usize + 1); // +1 for crosshair

            // Regular time labels (priority 1)
            let (time_step, rounded_earliest) = calc_time_step(
                earliest_in_millis,
                latest_in_millis,
                x_labels_can_fit,
                self.timeframe,
            );
            let mut time: i64 = rounded_earliest;

            while time <= latest_in_millis {
                let x_position = ((time - earliest_in_millis) as f64
                    / (latest_in_millis - earliest_in_millis) as f64)
                    * f64::from(bounds.width);

                if x_position >= 0.0 && x_position <= f64::from(bounds.width) {
                    if let Some(time_as_datetime) = DateTime::from_timestamp(time / 1000, 0) {
                        let text_content = match self.timezone {
                            UserTimezone::Local => {
                                let time_with_zone = time_as_datetime.with_timezone(&chrono::Local);

                                if self.timeframe < 10000 {
                                    time_with_zone.format("%M:%S").to_string()
                                } else if time_with_zone.format("%H:%M").to_string() == "00:00" {
                                    time_with_zone.format("%-d").to_string()
                                } else {
                                    time_with_zone.format("%H:%M").to_string()
                                }
                            }
                            UserTimezone::Utc => {
                                let time_with_zone = time_as_datetime.with_timezone(&chrono::Utc);

                                if self.timeframe < 10000 {
                                    time_with_zone.format("%M:%S").to_string()
                                } else if time_with_zone.format("%H:%M").to_string() == "00:00" {
                                    time_with_zone.format("%-d").to_string()
                                } else {
                                    time_with_zone.format("%H:%M").to_string()
                                }
                            }
                        };

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
                            marker_color: if palette.is_dark {
                                palette.background.weak.color.scale_alpha(0.6)
                            } else {
                                palette.background.strong.color.scale_alpha(0.6)
                            },
                            text_color: palette.background.base.text,
                            text_size: 12.0,
                        };

                        all_labels.push(AxisLabel::X(rect, label));
                    }
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
                                / f64::from(self.timeframe))
                            .round() as i64
                                * i64::from(self.timeframe);

                            if let Some(rounded_time) =
                                DateTime::from_timestamp_millis(rounded_timestamp)
                            {
                                let snap_ratio = (rounded_timestamp as f64
                                    - earliest_in_millis as f64)
                                    / (latest_in_millis as f64 - earliest_in_millis as f64);

                                (snap_ratio, {
                                    if self.timeframe < 10000 {
                                        rounded_time
                                            .format("%M:%S:%3f")
                                            .to_string()
                                            .replace('.', "")
                                    } else {
                                        match self.timezone {
                                            UserTimezone::Local => rounded_time
                                                .with_timezone(&chrono::Local)
                                                .format("%a %b %-d  %H:%M")
                                                .to_string(),
                                            UserTimezone::Utc => rounded_time
                                                .with_timezone(&chrono::Utc)
                                                .format("%a %b %-d  %H:%M")
                                                .to_string(),
                                        }
                                    }
                                })
                            } else {
                                (0.0, String::new())
                            }
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
                        marker_color: palette.background.strong.color,
                        text_color: palette.secondary.base.text,
                        text_size: 12.0,
                    };

                    all_labels.push(AxisLabel::X(rect, label));
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
            Interaction::Panning { .. } => mouse::Interaction::None,
            Interaction::Zoomin { .. } => mouse::Interaction::ResizingHorizontally,
            Interaction::None if cursor.is_over(bounds) => mouse::Interaction::ResizingHorizontally,
            _ => mouse::Interaction::default(),
        }
    }
}

fn calc_time_step(earliest: i64, latest: i64, labels_can_fit: i32, timeframe: u32) -> (i64, i64) {
    let timeframe_in_min = timeframe / 60000;

    let time_steps: &[i64] = match timeframe_in_min {
        0_u32..1_u32 => &MS_TIME_STEPS,
        1..=30 => match timeframe_in_min {
            1 => &M1_TIME_STEPS,
            3 => &M3_TIME_STEPS,
            5 => &M5_TIME_STEPS,
            15 => &M5_TIME_STEPS[..7],
            30 => &M5_TIME_STEPS[..6],
            _ => &HOURLY_TIME_STEPS,
        },
        31.. => &HOURLY_TIME_STEPS,
    };

    let duration = latest - earliest;
    let mut selected_step = time_steps[0];

    for &step in time_steps {
        if duration / step >= i64::from(labels_can_fit) {
            selected_step = step;
            break;
        }
        if step <= duration {
            selected_step = step;
        }
    }

    let rounded_earliest = (earliest / selected_step) * selected_step;

    (selected_step, rounded_earliest)
}

// Y-AXIS LABELS
struct AxisLabelsY<'a> {
    labels_cache: &'a Cache,
    crosshair: bool,
    translation_y: f32,
    scaling: f32,
    min: f32,
    last_price: Option<PriceInfoLabel>,
    tick_size: f32,
    decimals: usize,
    cell_height: f32,
    timeframe: u32,
    chart_bounds: Rectangle,
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
        event: Event,
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
                            y,
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
                if palette.is_dark {
                    palette.background.weak.color.scale_alpha(0.4)
                } else {
                    palette.background.strong.color.scale_alpha(0.4)
                },
            );

            let highest = self.y_to_price(region.y);
            let lowest = self.y_to_price(region.y + region.height);

            let y_range = highest - lowest;

            let y_labels_can_fit: i32 = (bounds.height / (text_size * 4.0)) as i32;

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
            let (step, rounded_lowest) =
                calc_price_step(highest, lowest, y_labels_can_fit, self.tick_size);

            let mut y = rounded_lowest;
            while y <= highest {
                let y_position = bounds.height - ((y - lowest) / y_range * bounds.height);

                let label = Label {
                    content: format!("{:.*}", self.decimals, y),
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

                y += step;
            }

            // Last price (priority 2)
            if let Some(last_price) = self.last_price {
                let (price, color) = match last_price {
                    PriceInfoLabel::Up(price) => (price, palette.success.base.color),
                    PriceInfoLabel::Down(price) => (price, palette.danger.base.color),
                };

                let candle_close_label = {
                    let current_time = chrono::Utc::now().timestamp_millis();
                    let next_kline_open =
                        (current_time / i64::from(self.timeframe) + 1) * i64::from(self.timeframe);

                    let remaining_seconds = (next_kline_open - current_time) / 1000;
                    let hours = remaining_seconds / 3600;
                    let minutes = (remaining_seconds % 3600) / 60;
                    let seconds = remaining_seconds % 60;

                    let time_format = if hours > 0 {
                        format!("{hours:02}:{minutes:02}:{seconds:02}")
                    } else {
                        format!("{minutes:02}:{seconds:02}")
                    };

                    Label {
                        content: time_format,
                        background_color: Some(palette.background.strong.color),
                        marker_color: palette.background.strong.color,
                        text_color: if palette.is_dark {
                            Color::BLACK.scale_alpha(0.8)
                        } else {
                            Color::WHITE.scale_alpha(0.8)
                        },
                        text_size: 11.0,
                    }
                };

                let price_label = Label {
                    content: format!("{:.*}", self.decimals, price),
                    background_color: Some(color),
                    marker_color: color,
                    text_color: if palette.is_dark {
                        Color::BLACK
                    } else {
                        Color::WHITE
                    },
                    text_size: 12.0,
                };

                let y_position = bounds.height - ((price - lowest) / y_range * bounds.height);

                all_labels.push(AxisLabel::Y(
                    rect(y_position, 2),
                    price_label,
                    Some(candle_close_label),
                ));
            }

            // Crosshair price (priority 3)
            if self.crosshair {
                if let Some(crosshair_pos) = cursor.position_in(self.chart_bounds) {
                    let raw_price =
                        lowest + (y_range * (bounds.height - crosshair_pos.y) / bounds.height);
                    let rounded_price = round_to_tick(raw_price, self.tick_size);
                    let y_position =
                        bounds.height - ((rounded_price - lowest) / y_range * bounds.height);

                    let label = Label {
                        content: format!("{:.*}", self.decimals, rounded_price),
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

fn calc_price_step(highest: f32, lowest: f32, labels_can_fit: i32, tick_size: f32) -> (f32, f32) {
    let range = highest - lowest;
    let labels = labels_can_fit as f32;

    // Find the order of magnitude of the range
    let base = 10.0f32.powf(range.log10().floor());

    // Try steps of 1, 2, 5 times the base magnitude
    let step = if range / (0.1 * base) <= labels {
        0.1 * base
    } else if range / (0.2 * base) <= labels {
        0.2 * base
    } else if range / (0.5 * base) <= labels {
        0.5 * base
    } else if range / base <= labels {
        base
    } else if range / (2.0 * base) <= labels {
        2.0 * base
    } else {
        5.0 * base
    };

    let rounded_lowest = (lowest / step).floor() * step;
    let rounded_lowest = (rounded_lowest / tick_size).round() * tick_size;

    (step, rounded_lowest)
}

// other helpers
#[derive(Debug, Clone, Copy)]
enum PriceInfoLabel {
    Up(f32),
    Down(f32),
}

fn convert_to_qty_abbr(price: f32) -> String {
    if price >= 1_000_000_000.0 {
        format!("{:.2}b", price / 1_000_000_000.0)
    } else if price >= 1_000_000.0 {
        format!("{:.2}m", price / 1_000_000.0)
    } else if price >= 1000.0 {
        format!("{:.1}k", price / 1000.0)
    } else if price >= 100.0 {
        format!("{price:.0}")
    } else if price >= 10.0 {
        format!("{price:.1}")
    } else if price >= 1.0 {
        format!("{price:.2}")
    } else {
        format!("{price:.3}")
    }
}

fn count_decimals(value: f32) -> usize {
    let value_str = value.to_string();
    if let Some(pos) = value_str.find('.') {
        value_str.len() - pos - 1
    } else {
        0
    }
}

fn round_to_tick(value: f32, tick_size: f32) -> f32 {
    (value / tick_size).round() * tick_size
}

#[derive(Debug, Clone)]
struct Label {
    content: String,
    background_color: Option<Color>,
    marker_color: Color,
    text_color: Color,
    text_size: f32,
}

#[derive(Debug, Clone)]
enum AxisLabel {
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

    fn filter_and_draw(labels: &[AxisLabel], frame: &mut Frame) {
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

                let marker_line = Stroke::with_color(
                    Stroke {
                        width: 1.0,
                        ..Default::default()
                    },
                    price_label.marker_color,
                );

                frame.stroke(
                    &Path::line(
                        Point::new(0.0, rect.center_y()),
                        Point::new(4.0, rect.center_y()),
                    ),
                    marker_line,
                );

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
