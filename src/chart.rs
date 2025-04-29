use data::chart::round_to_tick;
use iced::theme::palette::Extended;
use iced::widget::canvas::{self, Cache, Canvas, Event, Frame, LineDash, Stroke};
use iced::widget::{center, horizontal_rule, mouse_area, vertical_rule};
use iced::{
    Element, Length, Point, Rectangle, Size, Theme, Vector, alignment,
    mouse::{self},
    padding,
    widget::{
        Space, button, canvas::Path, column, container, row, text,
        tooltip::Position as TooltipPosition,
    },
};
use scale::{AxisLabelsX, AxisLabelsY, PriceInfoLabel};

use crate::widget::multi_split::MultiSplit;
use crate::{style, widget::tooltip};
use data::aggr::{ticks::TickAggr, time::TimeSeries};
use data::chart::{Basis, ChartLayout, indicators::Indicator};
use exchange::fetcher::{FetchRange, RequestHandler};
use exchange::{TickerInfo, Timeframe};

pub mod config;
pub mod heatmap;
pub mod indicator;
pub mod kline;
mod scale;
pub mod study;
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
    fn min_scaling(&self) -> f32;
    fn max_scaling(&self) -> f32;
    fn max_cell_width(&self) -> f32;
    fn min_cell_width(&self) -> f32;
    fn max_cell_height(&self) -> f32;
    fn min_cell_height(&self) -> f32;
    fn default_cell_width(&self) -> f32;
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
    SplitDragged(usize, f32),
    DoubleClick(AxisScaleClicked),
}

trait Chart: ChartConstants + canvas::Program<Message> {
    fn common_data(&self) -> &CommonChartData;

    fn common_data_mut(&mut self) -> &mut CommonChartData;

    fn update_chart(&mut self, message: &Message);

    fn canvas_interaction(
        &self,
        interaction: &mut Interaction,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>>;

    fn view_indicators<I: Indicator>(&self, enabled: &[I]) -> Vec<Element<Message>>;

    fn visible_timerange(&self) -> (u64, u64);

    fn interval_keys(&self) -> Vec<u64>;

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

    if chart.common_data().bounds != bounds {
        return Some(canvas::Action::publish(Message::BoundsChanged(bounds)));
    }

    let cursor_position = cursor.position_in(
        // padding for split draggers
        bounds.shrink(4.0),
    )?;

    match event {
        Event::Mouse(mouse_event) => {
            let chart_state = chart.common_data();

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
                    let default_cell_width = T::default_cell_width(chart);
                    let min_cell_width = T::min_cell_width(chart);
                    let max_cell_width = T::max_cell_width(chart);
                    let max_scaling = T::max_scaling(chart);
                    let min_scaling = T::min_scaling(chart);

                    if matches!(interaction, Interaction::Panning { .. }) {
                        return Some(canvas::Action::capture());
                    }

                    let cursor_to_center = cursor.position_from(bounds.center())?;
                    let y = match delta {
                        mouse::ScrollDelta::Lines { y, .. }
                        | mouse::ScrollDelta::Pixels { y, .. } => y,
                    };

                    let should_adjust_cell_width = match (y.signum(), chart_state.scaling) {
                        // zooming out at max scaling with increased cell width
                        (-1.0, scaling)
                            if scaling == max_scaling
                                && chart_state.cell_width > default_cell_width =>
                        {
                            true
                        }

                        // zooming in at min scaling with decreased cell width
                        (1.0, scaling)
                            if scaling == min_scaling
                                && chart_state.cell_width < default_cell_width =>
                        {
                            true
                        }

                        // zooming in at max scaling with room to increase cell width
                        (1.0, scaling)
                            if scaling == max_scaling
                                && chart_state.cell_width < max_cell_width =>
                        {
                            true
                        }

                        // zooming out at min scaling with room to decrease cell width
                        (-1.0, scaling)
                            if scaling == min_scaling
                                && chart_state.cell_width > min_cell_width =>
                        {
                            true
                        }

                        _ => false,
                    };

                    if should_adjust_cell_width {
                        return Some(
                            canvas::Action::publish(Message::XScaling(
                                y / 2.0,
                                cursor_to_center.x,
                                true,
                            ))
                            .and_capture(),
                        );
                    }

                    // normal scaling cases
                    if (*y < 0.0 && chart_state.scaling > min_scaling)
                        || (*y > 0.0 && chart_state.scaling < max_scaling)
                    {
                        let old_scaling = chart_state.scaling;
                        let scaling = (chart_state.scaling * (1.0 + y / 30.0))
                            .clamp(min_scaling, max_scaling);

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

pub enum Action {
    ErrorOccurred(data::InternalError),
    FetchRequested(uuid::Uuid, FetchRange),
    None,
}

fn update_chart<T: Chart>(chart: &mut T, message: &Message) {
    match message {
        Message::DoubleClick(scale) => {
            let default_chart_width = T::default_cell_width(chart);

            let chart_state = chart.common_data_mut();

            match scale {
                AxisScaleClicked::X => {
                    chart_state.cell_width = default_chart_width;
                }
                AxisScaleClicked::Y => {
                    chart_state.autoscale = true;
                }
            }
        }
        Message::Translated(translation) => {
            let chart_state = chart.common_data_mut();
            chart_state.translation = *translation;
            chart_state.autoscale = false;
        }
        Message::Scaled(scaling, translation) => {
            let chart_state = chart.common_data_mut();
            chart_state.scaling = *scaling;
            chart_state.translation = *translation;

            chart_state.autoscale = false;
        }
        Message::AutoscaleToggle => {
            let chart_state = chart.common_data_mut();
            chart_state.autoscale = !chart_state.autoscale;
            if chart_state.autoscale {
                chart_state.scaling = 1.0;
            }
        }
        Message::CrosshairToggle => {
            let chart_state = chart.common_data_mut();
            chart_state.crosshair = !chart_state.crosshair;
        }
        Message::XScaling(delta, cursor_to_center_x, is_wheel_scroll) => {
            let min_cell_width = T::min_cell_width(chart);
            let max_cell_width = T::max_cell_width(chart);

            let chart_state = chart.common_data_mut();

            if *delta < 0.0 && chart_state.cell_width > min_cell_width
                || *delta > 0.0 && chart_state.cell_width < max_cell_width
            {
                let (old_scaling, old_translation_x) =
                    { (chart_state.scaling, chart_state.translation.x) };

                let zoom_factor = if *is_wheel_scroll { 30.0 } else { 90.0 };

                let new_width = (chart_state.cell_width * (1.0 + delta / zoom_factor))
                    .clamp(min_cell_width, max_cell_width);

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
                    Basis::Time(_) => {
                        let cursor_time = chart_state.x_to_interval(cursor_chart_x);
                        chart_state.cell_width = new_width;

                        chart_state.interval_to_x(cursor_time)
                    }
                    Basis::Tick(_) => {
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
            let min_cell_height = T::min_cell_height(chart);
            let max_cell_height = T::max_cell_height(chart);

            let chart_state = chart.common_data_mut();

            if *delta < 0.0 && chart_state.cell_height > min_cell_height
                || *delta > 0.0 && chart_state.cell_height < max_cell_height
            {
                let (old_scaling, old_translation_y) =
                    { (chart_state.scaling, chart_state.translation.y) };

                let zoom_factor = if *is_wheel_scroll { 30.0 } else { 90.0 };

                let new_height = (chart_state.cell_height * (1.0 + delta / zoom_factor))
                    .clamp(min_cell_height, max_cell_height);

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
            let chart_state = chart.common_data_mut();

            // calculate how center shifted
            let old_center_x = chart_state.bounds.width / 2.0;
            let new_center_x = bounds.width / 2.0;
            let center_delta_x = (new_center_x - old_center_x) / chart_state.scaling;

            chart_state.bounds = *bounds;

            if !chart_state.autoscale {
                chart_state.translation.x += center_delta_x;
            }
        }
        Message::SplitDragged(split, size) => {
            let chart_state = chart.common_data_mut();

            if let Some(split) = chart_state.splits.get_mut(*split) {
                *split = (*size * 100.0).round() / 100.0;
            }
        }
        Message::CrosshairMoved => {}
    }
}

fn view_chart<'a, T: Chart, I: Indicator>(
    chart: &'a T,
    indicators: &'a [I],
    timezone: data::UserTimezone,
) -> Element<'a, Message> {
    let chart_state = chart.common_data();

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
        interval_keys: chart.interval_keys(),
    })
    .width(Length::Fill)
    .height(Length::Fill);

    let chart_controls = {
        let center_button = button(text("C").size(10).align_x(alignment::Horizontal::Center))
            .width(Length::Shrink)
            .height(Length::Fill)
            .on_press(Message::AutoscaleToggle)
            .style(move |theme, status| {
                style::button::transparent(theme, status, chart_state.autoscale)
            });

        let crosshair_button = button(text("+").size(10).align_x(alignment::Horizontal::Center))
            .width(Length::Shrink)
            .height(Length::Fill)
            .on_press(Message::CrosshairToggle)
            .style(move |theme, status| {
                style::button::transparent(theme, status, chart_state.crosshair)
            });

        container(
            row![
                Space::new(Length::Fill, Length::Fill),
                tooltip(center_button, Some("Center Latest"), TooltipPosition::Top),
                tooltip(crosshair_button, Some("Crosshair"), TooltipPosition::Top),
            ]
            .spacing(2),
        )
        .padding(2)
    };

    let chart_content = {
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

        let main_chart: Element<_> = row![
            container(Canvas::new(chart).width(Length::Fill).height(Length::Fill))
                .width(Length::FillPortion(10))
                .height(Length::FillPortion(120)),
            vertical_rule(1).style(style::split_ruler),
            container(
                mouse_area(axis_labels_y)
                    .on_double_click(Message::DoubleClick(AxisScaleClicked::Y))
            )
            .width(Length::Fixed(60.0 + (chart_state.decimals as f32 * 4.0)))
            .height(Length::FillPortion(120))
        ]
        .into();

        let indicators = chart.view_indicators(indicators);

        if indicators.is_empty() {
            main_chart
        } else {
            let panels = std::iter::once(main_chart)
                .chain(indicators)
                .collect::<Vec<_>>();

            MultiSplit::new(panels, &chart_state.splits, |index, position| {
                Message::SplitDragged(index, position)
            })
            .into()
        }
    };

    column![
        chart_content,
        horizontal_rule(1).style(style::split_ruler),
        row![
            container(
                mouse_area(axis_labels_x)
                    .on_double_click(Message::DoubleClick(AxisScaleClicked::X))
            )
            .padding(padding::right(1))
            .width(Length::FillPortion(10))
            .height(Length::Fixed(26.0)),
            chart_controls
                .width(Length::Fixed(60.0 + (chart_state.decimals as f32 * 4.0)))
                .height(Length::Fixed(26.0))
        ]
    ]
    .padding(1)
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

enum ChartData {
    TimeBased(TimeSeries),
    TickBased(TickAggr),
}

impl ChartData {
    pub fn get_latest_price_range_y_midpoint(&self, chart_state: &CommonChartData) -> f32 {
        match self {
            ChartData::TimeBased(timeseries) => timeseries.latest_kline().map_or(0.0, |kline| {
                let y_low = chart_state.price_to_y(kline.low);
                let y_high = chart_state.price_to_y(kline.high);
                -(y_low + y_high) / 2.0
            }),
            ChartData::TickBased(tick_aggr) => tick_aggr.latest_dp().map_or(0.0, |(dp, _)| {
                let y_low = chart_state.price_to_y(dp.kline.low);
                let y_high = chart_state.price_to_y(dp.kline.high);
                -(y_low + y_high) / 2.0
            }),
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
    basis: Basis,

    last_price: Option<PriceInfoLabel>,

    base_price_y: f32,
    latest_x: u64,
    tick_size: f32,
    decimals: usize,
    ticker_info: Option<TickerInfo>,

    splits: Vec<f32>,
}

impl Default for CommonChartData {
    fn default() -> Self {
        CommonChartData {
            cache: Caches::default(),
            crosshair: true,
            translation: Vector::default(),
            bounds: Rectangle::default(),
            basis: Basis::Time(Timeframe::M5.to_milliseconds()),
            last_price: None,
            scaling: 1.0,
            autoscale: true,
            cell_width: 40.0,
            cell_height: 30.0,
            base_price_y: 0.0,
            latest_x: 0,
            tick_size: 0.0,
            decimals: 0,
            ticker_info: None,
            splits: vec![],
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

    fn interval_range(&self, region: &Rectangle) -> (u64, u64) {
        match self.basis {
            Basis::Tick(_) => (
                self.x_to_interval(region.x + region.width),
                self.x_to_interval(region.x),
            ),
            Basis::Time(interval) => (
                self.x_to_interval(region.x).saturating_sub(interval / 2),
                self.x_to_interval(region.x + region.width)
                    .saturating_add(interval / 2),
            ),
        }
    }

    fn price_range(&self, region: &Rectangle) -> (f32, f32) {
        let highest = self.y_to_price(region.y);
        let lowest = self.y_to_price(region.y + region.height);

        (highest, lowest)
    }

    fn interval_to_x(&self, value: u64) -> f32 {
        match self.basis {
            Basis::Time(timeframe) => {
                if value <= self.latest_x {
                    let diff = self.latest_x - value;
                    -(diff as f32 / timeframe as f32) * self.cell_width
                } else {
                    let diff = value - self.latest_x;
                    (diff as f32 / timeframe as f32) * self.cell_width
                }
            }
            Basis::Tick(_) => -((value as f32) * self.cell_width),
        }
    }

    fn x_to_interval(&self, x: f32) -> u64 {
        match self.basis {
            Basis::Time(interval) => {
                if x <= 0.0 {
                    let diff = (-x / self.cell_width * interval as f32) as u64;
                    self.latest_x.saturating_sub(diff)
                } else {
                    let diff = (x / self.cell_width * interval as f32) as u64;
                    self.latest_x.saturating_add(diff)
                }
            }
            Basis::Tick(_) => {
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

        let dashed_line = style::get_dashed_line(theme);

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
            Basis::Time(timeframe) => {
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
            Basis::Tick(aggregation) => {
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

    fn draw_last_price_line(
        &self,
        frame: &mut canvas::Frame,
        palette: &Extended,
        region: Rectangle,
    ) {
        if let Some(price) = &self.last_price {
            let (mut y_pos, line_color) = price.get_with_color(palette);
            y_pos = self.price_to_y(y_pos);

            let marker_line = Stroke::with_color(
                Stroke {
                    width: 1.0,
                    line_dash: LineDash {
                        segments: &[2.0, 2.0],
                        offset: 4,
                    },
                    ..Default::default()
                },
                line_color.scale_alpha(0.5),
            );

            frame.stroke(
                &Path::line(
                    Point::new(0.0, y_pos),
                    Point::new(region.x + region.width, y_pos),
                ),
                marker_line,
            );
        }
    }

    fn get_chart_layout(&self) -> ChartLayout {
        ChartLayout {
            crosshair: self.crosshair,
            splits: self.splits.clone(),
        }
    }
}

fn request_fetch(handler: &mut RequestHandler, range: FetchRange) -> Option<Action> {
    match handler.add_request(range) {
        Ok(Some(req_id)) => Some(Action::FetchRequested(req_id, range)),
        Ok(None) => None,
        Err(reason) => {
            log::error!("Failed to request {:?}: {}", range, reason);
            // TODO: handle this more explicitly, maybe by returning Action::ErrorOccurred
            None
        }
    }
}

pub fn draw_horizontal_volume_bars(
    frame: &mut canvas::Frame,
    start_x: f32,
    y_position: f32,
    buy_qty: f32,
    sell_qty: f32,
    max_qty: f32,
    bar_height: f32,
    width_factor: f32,
    buy_color: iced::Color,
    sell_color: iced::Color,
    bar_color_alpha: f32,
) {
    let total_qty = buy_qty + sell_qty;
    if total_qty <= 0.0 {
        return;
    }

    let total_bar_width = (total_qty / max_qty) * width_factor;

    let buy_proportion = buy_qty / total_qty;
    let sell_proportion = sell_qty / total_qty;

    let buy_bar_width = buy_proportion * total_bar_width;
    let sell_bar_width = sell_proportion * total_bar_width;

    let start_y = y_position - (bar_height / 2.0);

    if sell_qty > 0.0 {
        frame.fill_rectangle(
            Point::new(start_x, start_y),
            Size::new(sell_bar_width, bar_height),
            sell_color.scale_alpha(bar_color_alpha),
        );
    }

    if buy_qty > 0.0 {
        frame.fill_rectangle(
            Point::new(start_x + sell_bar_width, start_y),
            Size::new(buy_bar_width, bar_height),
            buy_color.scale_alpha(bar_color_alpha),
        );
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

fn abbr_large_numbers(value: f32) -> String {
    let abs_value = value.abs();
    let sign = if value < 0.0 { "-" } else { "" };

    if abs_value >= 1_000_000_000.0 {
        format!("{}{:.2}b", sign, abs_value / 1_000_000_000.0)
    } else if abs_value >= 1_000_000.0 {
        format!("{}{:.2}m", sign, abs_value / 1_000_000.0)
    } else if abs_value >= 1000.0 {
        format!("{}{:.1}k", sign, abs_value / 1000.0)
    } else if abs_value >= 100.0 {
        format!("{}{:.0}", sign, abs_value)
    } else if abs_value >= 10.0 {
        format!("{}{:.1}", sign, abs_value)
    } else if abs_value >= 1.0 {
        format!("{}{:.2}", sign, abs_value)
    } else {
        format!("{}{:.3}", sign, abs_value)
    }
}

pub fn format_with_commas(num: f32) -> String {
    if num == 0.0 {
        return "0".to_string();
    }

    let abs_num = num.abs();
    let is_negative = num < 0.0;

    let decimals = if abs_num >= 100.0 {
        0
    } else if abs_num >= 10.0 {
        1
    } else if abs_num >= 1.0 {
        2
    } else {
        3
    };

    if abs_num < 1000.0 {
        return format!(
            "{}{:.*}",
            if is_negative { "-" } else { "" },
            decimals,
            abs_num
        );
    }

    let s = format!("{:.*}", decimals, abs_num);

    let (integer_part, decimal_part) = match s.find('.') {
        Some(pos) => (&s[..pos], Some(&s[pos..])),
        None => (s.as_str(), None),
    };

    let num_commas = (integer_part.len() - 1) / 3;
    let decimal_len = decimal_part.map_or(0, str::len);
    let capacity = usize::from(is_negative) + integer_part.len() + num_commas + decimal_len;

    let mut result = String::with_capacity(capacity);

    if is_negative {
        result.push('-');
    }

    let digits_len = integer_part.len();
    for (i, ch) in integer_part.chars().enumerate() {
        result.push(ch);

        let pos_from_right = digits_len - i - 1;
        if i < digits_len - 1 && pos_from_right % 3 == 0 {
            result.push(',');
        }
    }

    if let Some(decimal) = decimal_part {
        result.push_str(decimal);
    }

    result
}

pub fn calc_splits(main_split: f32, active_indicators: usize) -> Vec<f32> {
    let mut splits = vec![main_split];

    if active_indicators > 1 {
        let remaining_space = 0.9 - main_split;
        let section_size = remaining_space / (active_indicators) as f32;

        for i in 1..active_indicators {
            let split_position = main_split + section_size * (i as f32);
            splits.push(split_position);
        }
    }
    splits
}
