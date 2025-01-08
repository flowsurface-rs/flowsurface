use iced::{
    alignment,
    mouse::{self},
    widget::{button, canvas::{LineDash, Path, Stroke}, center, column, container, row, text, Space},
    Element, Length, Point, Rectangle, Size, Task, Theme, Vector,
};
use iced::widget::canvas::{self, Canvas, Event, Frame, Cache};
use indicators::Indicator;
use scales::{AxisLabelsX, AxisLabelsY, PriceInfoLabel};
use uuid::Uuid;

use crate::{
    data_providers::{fetcher::{FetchRange, ReqError, RequestHandler}, TickerInfo}, 
    layout::SerializableChartData, screen::UserTimezone, style, 
    tooltip::{self, tooltip}, widget::hsplit::HSplit
};

mod scales;
pub mod candlestick;
pub mod footprint;
pub mod heatmap;
pub mod indicators;
pub mod timeandsales;

#[derive(Default, Debug, Clone, Copy)]
pub enum Interaction {
    #[default]
    None,
    Zoomin { last_position: Point },
    Panning { translation: Vector, start: Point },
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
    SplitDragged(f32),
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

    let cursor_position = cursor.position_in(
        // padding for split draggers
        bounds.shrink(4.0)
    )?;

    match event {
        Event::Mouse(mouse_event) => {
            let chart_state = chart.get_common_data();

            match mouse_event {
                mouse::Event::ButtonPressed(button) => {
                    let message = match button {
                        mouse::Button::Left => {
                            *interaction = Interaction::Panning {
                                translation: chart_state.translation,
                                start: cursor_position,
                            };
                            None
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
                            if chart_state.crosshair {
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
        Message::SplitDragged(split) => {
            chart_state.indicators_split = Some(*split);
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

    if chart_state.loading_chart {
        return center(text("Loading...").size(16)).into();
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

    let main_chart = row![
        container(chart_canvas)
            .width(Length::FillPortion(10))
            .height(Length::FillPortion(120)),
        container(axis_labels_y)
            .width(Length::Fixed(60.0 + (chart_state.decimals as f32 * 2.0)))
            .height(Length::FillPortion(120))
    ];

    let chart_content = match (chart_state.indicators_split, indicators.is_empty()) {
        (Some(split_at), false) => {
            if let Some(indicator) = chart.view_indicator(indicators, ticker_info) {
                row![
                    HSplit::new(
                        main_chart,
                        indicator,
                        Message::SplitDragged,
                    )
                    .split(split_at),
                ]
            } else {
                main_chart
            }
        },
        _ => main_chart
    };

    column![
        chart_content,
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

    indicators_split: Option<f32>,

    already_fetching: bool,
    loading_chart: bool,
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
            indicators_split: None,
            already_fetching: false,
            loading_chart: true,
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

    pub fn check_kline_integrity<T: ContainsKey<i64>>(
        &self,
        earliest: i64,
        latest: i64,
        data_points: &T
    ) -> Option<Vec<i64>> {
        if self.already_fetching {
            return None;
        }
    
        let interval = self.timeframe as i64;
        
        let mut time = earliest;
        let mut missing_count = 0;
        while time < latest {
            if !data_points.contains_key(&time) {
                missing_count += 1;
                break; 
            }
            time += interval;
        }
    
        if missing_count > 0 {
            let mut missing_keys = Vec::with_capacity(((latest - earliest) / interval) as usize);
            let mut time = earliest;
            while time < latest {
                if !data_points.contains_key(&time) {
                    missing_keys.push(time);
                }
                time += interval;
            }
            
            log::warn!("Integrity check failed: missing {} klines", missing_keys.len());
            return Some(missing_keys);
        }

        None
    }

    fn get_chart_layout(&self) -> SerializableChartData {
        SerializableChartData {
            crosshair: self.crosshair,
            indicators_split: self.indicators_split,
        }
    }
}

pub trait ContainsKey<K> {
    fn contains_key(&self, key: &K) -> bool;
}

impl<K: Ord, V> ContainsKey<K> for std::collections::BTreeMap<K, V> {
    fn contains_key(&self, key: &K) -> bool {
        self.contains_key(key)
    }
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

fn abbr_large_numbers(value: f32) -> String {
    if value >= 1_000_000_000.0 {
        format!("{:.2}b", value / 1_000_000_000.0)
    } else if value >= 1_000_000.0 {
        format!("{:.2}m", value / 1_000_000.0)
    } else if value >= 1000.0 {
        format!("{:.1}k", value / 1000.0)
    } else if value >= 100.0 {
        format!("{value:.0}")
    } else if value >= 10.0 {
        format!("{value:.1}")
    } else if value >= 1.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.3}")
    }
}