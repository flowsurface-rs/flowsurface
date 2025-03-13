use iced::widget::canvas::{self, Cache, Canvas, Event, Frame};
use iced::widget::{center, mouse_area};
use iced::{
    Element, Length, Point, Rectangle, Size, Task, Theme, Vector, alignment,
    mouse::{self},
    widget::{
        Space, button,
        canvas::{LineDash, Path, Stroke},
        column, container, row, text,
    },
};
use indicators::Indicator;
use scales::{AxisLabelsX, AxisLabelsY, PriceInfoLabel};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    data_providers::{
        TickerInfo,
        aggr::{
            ticks::TickAggr,
            time::{TimeSeries, Timeframe},
        },
        fetcher::{FetchRange, ReqError, RequestHandler},
    },
    layout::SerializableChartData,
    screen::UserTimezone,
    style,
    tooltip::{self, tooltip},
    widget::hsplit::HSplit,
};

pub mod candlestick;
pub mod config;
pub mod footprint;
pub mod heatmap;
pub mod indicators;
mod scales;
pub mod timeandsales;

#[derive(Default, Debug, Clone, Copy)]
pub enum Interaction {
    #[default]
    None,
    Zoomin {
        last_position: Point,
    },
    Panning {
        translation: Vector,
        start: Point,
    },
}

#[derive(Debug, Clone)]
pub enum AxisScaleClicked {
    X,
    Y,
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
    DoubleClick(AxisScaleClicked),
}

trait Chart: ChartConstants + canvas::Program<Message> {
    fn get_common_data(&self) -> &CommonChartData;

    fn get_common_data_mut(&mut self) -> &mut CommonChartData;

    fn update_chart(&mut self, message: &Message) -> Task<Message>;

    fn canvas_interaction(
        &self,
        interaction: &mut Interaction,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>>;

    fn view_indicator<I: Indicator>(&self, enabled: &[I]) -> Option<Element<Message>>;

    fn get_visible_timerange(&self) -> (u64, u64);

    fn get_interval_keys(&self) -> Vec<u64>;

    fn is_empty(&self) -> bool;
}

fn canvas_interaction<T: Chart>(
    chart: &T,
    interaction: &mut Interaction,
    event: &Event,
    bounds: Rectangle,
    cursor: mouse::Cursor,
) -> Option<canvas::Action<Message>> {
    if let Event::Mouse(mouse::Event::ButtonReleased(_)) = event {
        *interaction = Interaction::None;
    }

    if chart.get_common_data().bounds != bounds {
        return Some(canvas::Action::publish(Message::BoundsChanged(bounds)));
    }

    let cursor_position = cursor.position_in(
        // padding for split draggers
        bounds.shrink(4.0),
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
                    if matches!(interaction, Interaction::Panning { .. }) {
                        return Some(canvas::Action::capture());
                    }

                    let cursor_to_center = cursor.position_from(bounds.center())?;

                    let y = match delta {
                        mouse::ScrollDelta::Lines { y, .. }
                        | mouse::ScrollDelta::Pixels { y, .. } => y,
                    };

                    // at max scaling, but the cell width can still be increased
                    if (*y > 0.0 && chart_state.scaling == T::MAX_SCALING)
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

                    // at min scaling, but the cell width can still be decreased
                    if (*y < 0.0 && chart_state.scaling == T::MIN_SCALING)
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
                    if (*y < 0.0 && chart_state.scaling > T::MIN_SCALING)
                        || (*y > 0.0 && chart_state.scaling < T::MAX_SCALING)
                    {
                        let old_scaling = chart_state.scaling;
                        let scaling = (chart_state.scaling * (1.0 + y / 30.0))
                            .clamp(T::MIN_SCALING, T::MAX_SCALING);

                        let translation = {
                            let factor = scaling - old_scaling;
                            let denominator = old_scaling * old_scaling;

                            // safeguard against division by very small numbers
                            let vector_diff = if denominator > 0.0001 {
                                Vector::new(
                                    cursor_to_center.x * factor / denominator,
                                    cursor_to_center.y * factor / denominator,
                                )
                            } else {
                                Vector::new(0.0, 0.0)
                            };

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
        Message::DoubleClick(scale) => match scale {
            AxisScaleClicked::X => {
                chart_state.cell_width = T::DEFAULT_CELL_WIDTH;
            }
            AxisScaleClicked::Y => {
                chart_state.autoscale = true;
            }
        },
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

                let latest_x = chart_state.interval_to_x(chart_state.latest_x);
                let is_interval_x_visible = chart_state.is_interval_x_visible(latest_x);

                let cursor_chart_x = {
                    if *is_wheel_scroll || !is_interval_x_visible {
                        cursor_to_center_x / old_scaling - old_translation_x
                    } else {
                        latest_x / old_scaling - old_translation_x
                    }
                };

                let new_cursor_x = match chart_state.basis {
                    ChartBasis::Time(_) => {
                        let cursor_time = chart_state.x_to_interval(cursor_chart_x);
                        chart_state.cell_width = new_width;

                        chart_state.interval_to_x(cursor_time)
                    }
                    ChartBasis::Tick(_) => {
                        let tick_index = cursor_chart_x / chart_state.cell_width;
                        chart_state.cell_width = new_width;

                        tick_index * chart_state.cell_width
                    }
                };

                if *is_wheel_scroll || !is_interval_x_visible {
                    if !new_cursor_x.is_nan() && !cursor_chart_x.is_nan() {
                        chart_state.translation.x -= new_cursor_x - cursor_chart_x;
                    }

                    chart_state.autoscale = false;
                }
            }
        }
        Message::YScaling(delta, cursor_to_center_y, is_wheel_scroll) => {
            if *delta < 0.0 && chart_state.cell_height > T::MIN_CELL_HEIGHT
                || *delta > 0.0 && chart_state.cell_height < T::MAX_CELL_HEIGHT
            {
                let (old_scaling, old_translation_y) =
                    { (chart_state.scaling, chart_state.translation.y) };

                let zoom_factor = if *is_wheel_scroll { 30.0 } else { 90.0 };

                let new_height = (chart_state.cell_height * (1.0 + delta / zoom_factor))
                    .clamp(T::MIN_CELL_HEIGHT, T::MAX_CELL_HEIGHT);

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
            // calculate how center shifted
            let old_center_x = chart_state.bounds.width / 2.0;
            let new_center_x = bounds.width / 2.0;
            let center_delta_x = (new_center_x - old_center_x) / chart_state.scaling;

            chart_state.bounds = *bounds;

            if !chart_state.autoscale {
                chart_state.translation.x += center_delta_x;
            }
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
    timezone: &'a UserTimezone,
) -> Element<'a, Message> {
    let chart_state = chart.get_common_data();

    if chart.is_empty() {
        return center(text("Waiting for data...").size(16)).into();
    }

    let axis_labels_x = Canvas::new(AxisLabelsX {
        labels_cache: &chart_state.cache.x_labels,
        scaling: chart_state.scaling,
        translation_x: chart_state.translation.x,
        max: chart_state.latest_x,
        crosshair: chart_state.crosshair,
        basis: chart_state.basis,
        cell_width: chart_state.cell_width,
        timezone,
        chart_bounds: chart_state.bounds,
        interval_keys: chart.get_interval_keys(),
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
        basis: chart_state.basis,
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

    let chart_canvas = Canvas::new(chart).width(Length::Fill).height(Length::Fill);

    let main_chart = row![
        container(chart_canvas)
            .width(Length::FillPortion(10))
            .height(Length::FillPortion(120)),
        container(
            mouse_area(axis_labels_y).on_double_click(Message::DoubleClick(AxisScaleClicked::Y))
        )
        .width(Length::Fixed(60.0 + (chart_state.decimals as f32 * 2.0)))
        .height(Length::FillPortion(120))
    ];

    let chart_content = match (chart_state.indicators_split, indicators.is_empty()) {
        (Some(split_at), false) => {
            if let Some(indicator) = chart.view_indicator(indicators) {
                row![HSplit::new(
                    main_chart,
                    indicator,
                    split_at,
                    Message::SplitDragged,
                )]
            } else {
                main_chart
            }
        }
        _ => main_chart,
    };

    column![
        chart_content,
        row![
            container(
                mouse_area(axis_labels_x)
                    .on_double_click(Message::DoubleClick(AxisScaleClicked::X))
            )
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

/// Defines how chart data is aggregated and displayed along the x-axis.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ChartBasis {
    /// Time-based aggregation where each datapoint represents a fixed time interval.
    ///
    /// The u64 value represents milliseconds. Common values include:
    /// - 60_000 (1 minute)
    /// - 300_000 (5 minutes)
    /// - 3_600_000 (1 hour)
    Time(u64),

    /// Trade-based aggregation where each datapoint represents a fixed number of trades.
    ///
    /// The u64 value represents the number of trades per aggregation unit.
    /// Common values include 100, 500, or 1000 trades per bar/candle.
    Tick(u64),
}

impl ChartBasis {
    pub fn is_time(&self) -> bool {
        matches!(self, ChartBasis::Time(_))
    }
}

impl std::fmt::Display for ChartBasis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChartBasis::Time(millis) => match *millis {
                60_000 => write!(f, "1m"),
                180_000 => write!(f, "3m"),
                300_000 => write!(f, "5m"),
                900_000 => write!(f, "15m"),
                1_800_000 => write!(f, "30m"),
                3_600_000 => write!(f, "1h"),
                7_200_000 => write!(f, "2h"),
                14_400_000 => write!(f, "4h"),
                _ => write!(f, "{}ms", millis),
            },
            ChartBasis::Tick(count) => write!(f, "{}T", count),
        }
    }
}

enum ChartData {
    TimeBased(TimeSeries),
    TickBased(TickAggr),
}

impl ChartData {
    pub fn get_latest_price_range_y_midpoint(&self, chart_state: &CommonChartData) -> f32 {
        match self {
            ChartData::TimeBased(timeseries) => timeseries
                .get_latest_kline()
                .map(|kline| {
                    let y_low = chart_state.price_to_y(kline.low);
                    let y_high = chart_state.price_to_y(kline.high);
                    -(y_low + y_high) / 2.0
                })
                .unwrap_or(0.0),
            ChartData::TickBased(tick_aggr) => tick_aggr
                .get_latest_dp()
                .map(|(dp, _)| {
                    let y_low = chart_state.price_to_y(dp.low_price);
                    let y_high = chart_state.price_to_y(dp.high_price);
                    -(y_low + y_high) / 2.0
                })
                .unwrap_or(0.0),
        }
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
    basis: ChartBasis,

    last_price: Option<PriceInfoLabel>,

    base_price_y: f32,
    latest_x: u64,
    tick_size: f32,
    decimals: usize,
    ticker_info: Option<TickerInfo>,

    indicators_split: Option<f32>,
}

impl Default for CommonChartData {
    fn default() -> Self {
        CommonChartData {
            cache: Caches::default(),
            crosshair: true,
            translation: Vector::default(),
            bounds: Rectangle::default(),
            basis: ChartBasis::Time(Timeframe::M5.to_milliseconds()),
            last_price: None,
            scaling: 1.0,
            autoscale: true,
            cell_width: 40.0,
            cell_height: 30.0,
            base_price_y: 0.0,
            latest_x: 0,
            tick_size: 0.0,
            decimals: 0,
            indicators_split: None,
            ticker_info: None,
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

    fn is_interval_x_visible(&self, interval_x: f32) -> bool {
        let region = self.visible_region(self.bounds.size());

        interval_x >= region.x && interval_x <= region.x + region.width
    }

    fn get_interval_range(&self, region: Rectangle) -> (u64, u64) {
        match self.basis {
            ChartBasis::Tick(_) => (
                self.x_to_interval(region.x + region.width),
                self.x_to_interval(region.x),
            ),
            ChartBasis::Time(interval) => (
                self.x_to_interval(region.x).saturating_sub(interval / 2),
                self.x_to_interval(region.x + region.width)
                    .saturating_add(interval / 2),
            ),
        }
    }

    fn get_price_range(&self, region: Rectangle) -> (f32, f32) {
        let highest = self.y_to_price(region.y);
        let lowest = self.y_to_price(region.y + region.height);

        (highest, lowest)
    }

    fn interval_to_x(&self, value: u64) -> f32 {
        match self.basis {
            ChartBasis::Time(timeframe) => {
                if value <= self.latest_x {
                    let diff = self.latest_x - value;
                    -(diff as f32 / timeframe as f32) * self.cell_width
                } else {
                    let diff = value - self.latest_x;
                    (diff as f32 / timeframe as f32) * self.cell_width
                }
            }
            ChartBasis::Tick(_) => -((value as f32) * self.cell_width),
        }
    }

    fn x_to_interval(&self, x: f32) -> u64 {
        match self.basis {
            ChartBasis::Time(interval) => {
                if x <= 0.0 {
                    let diff = (-x / self.cell_width * interval as f32) as u64;
                    self.latest_x.saturating_sub(diff)
                } else {
                    let diff = (x / self.cell_width * interval as f32) as u64;
                    self.latest_x.saturating_add(diff)
                }
            }
            ChartBasis::Tick(_) => {
                let tick = -(x / self.cell_width);
                tick.round() as u64
            }
        }
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
    ) -> (f32, u64) {
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
            palette
                .secondary
                .strong
                .color
                .scale_alpha(if palette.is_dark { 0.6 } else { 1.0 }),
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

        // Vertical time/tick line
        match self.basis {
            ChartBasis::Time(timeframe) => {
                let earliest = self.x_to_interval(region.x) as f64;
                let latest = self.x_to_interval(region.x + region.width) as f64;

                let crosshair_ratio = f64::from(cursor_position.x / bounds.width);
                let crosshair_millis = earliest + crosshair_ratio * (latest - earliest);

                let rounded_timestamp =
                    (crosshair_millis / (timeframe as f64)).round() as u64 * timeframe;
                let snap_ratio =
                    ((rounded_timestamp as f64 - earliest) / (latest - earliest)) as f32;

                frame.stroke(
                    &Path::line(
                        Point::new(snap_ratio * bounds.width, 0.0),
                        Point::new(snap_ratio * bounds.width, bounds.height),
                    ),
                    dashed_line,
                );

                (rounded_price, rounded_timestamp)
            }
            ChartBasis::Tick(aggregation) => {
                let crosshair_ratio = cursor_position.x / bounds.width;

                let (chart_x_min, chart_x_max) = (region.x, region.x + region.width);
                let crosshair_pos = chart_x_min + crosshair_ratio * region.width;

                let cell_index = (crosshair_pos / self.cell_width).round();

                let snapped_crosshair = cell_index * self.cell_width;

                let snap_ratio = (snapped_crosshair - chart_x_min) / (chart_x_max - chart_x_min);

                let rounded_tick = (-cell_index as u64) * aggregation;

                frame.stroke(
                    &Path::line(
                        Point::new(snap_ratio * bounds.width, 0.0),
                        Point::new(snap_ratio * bounds.width, bounds.height),
                    ),
                    dashed_line,
                );

                (rounded_price, rounded_tick)
            }
        }
    }

    fn get_chart_layout(&self) -> SerializableChartData {
        SerializableChartData {
            crosshair: self.crosshair,
            indicators_split: self.indicators_split,
        }
    }
}

fn request_fetch(handler: &mut RequestHandler, range: FetchRange) -> Option<Task<Message>> {
    match handler.add_request(range) {
        Ok(req_id) => Some(Task::done(Message::NewDataRange(req_id, range))),
        Err(e) => {
            match e {
                ReqError::Overlaps => log::debug!("Request overlaps with existing request"),
                ReqError::Failed(msg) => log::debug!("Request already failed: {}", msg),
                ReqError::Completed => log::debug!("Request already completed"),
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
