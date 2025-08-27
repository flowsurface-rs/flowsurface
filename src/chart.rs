pub mod heatmap;
pub mod indicator;
pub mod kline;
mod scale;

use crate::style;
use crate::widget::multi_split::{DRAG_SIZE, MultiSplit};
use crate::widget::tooltip;
use data::chart::{Autoscale, Basis, PlotData, ViewConfig, indicator::Indicator};
use exchange::fetcher::{FetchRange, RequestHandler};
use exchange::{TickerInfo, Timeframe};
use scale::linear::PriceInfoLabel;
use scale::{AxisLabelsX, AxisLabelsY};

use iced::theme::palette::Extended;
use iced::widget::canvas::{self, Cache, Canvas, Event, Frame, LineDash, Path, Stroke};
use iced::{
    Alignment, Element, Length, Point, Rectangle, Size, Theme, Vector, keyboard, mouse, padding,
    widget::{
        button, center, column, container, horizontal_rule, mouse_area, row, text, vertical_rule,
    },
};

const ZOOM_SENSITIVITY: f32 = 30.0;
const TEXT_SIZE: f32 = 12.0;

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
    Ruler {
        start: Option<Point>,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum AxisScaleClicked {
    X,
    Y,
}

#[derive(Debug, Clone, Copy)]
pub enum Message {
    Translated(Vector),
    Scaled(f32, Vector),
    AutoscaleToggled,
    CrosshairMoved,
    YScaling(f32, f32, bool),
    XScaling(f32, f32, bool),
    BoundsChanged(Rectangle),
    SplitDragged(usize, f32),
    DoubleClick(AxisScaleClicked),
}

pub trait Chart: PlotConstants + canvas::Program<Message> {
    type IndicatorKind: Indicator;

    fn state(&self) -> &ViewState;

    fn mut_state(&mut self) -> &mut ViewState;

    fn invalidate_all(&mut self);

    fn invalidate_crosshair(&mut self);

    fn view_indicators(&'_ self, enabled: &[Self::IndicatorKind]) -> Vec<Element<'_, Message>>;

    fn visible_timerange(&self) -> (u64, u64);

    fn interval_keys(&self) -> Option<Vec<u64>>;

    fn autoscaled_coords(&self) -> Vector;

    fn supports_fit_autoscaling(&self) -> bool;

    fn is_empty(&self) -> bool;
}

fn canvas_interaction<T: Chart>(
    chart: &T,
    interaction: &mut Interaction,
    event: &Event,
    bounds: Rectangle,
    cursor: mouse::Cursor,
) -> Option<canvas::Action<Message>> {
    if chart.state().bounds != bounds {
        return Some(canvas::Action::publish(Message::BoundsChanged(bounds)));
    }

    let shrunken_bounds = bounds.shrink(DRAG_SIZE * 4.0);
    let cursor_position = cursor.position_in(shrunken_bounds);

    if let Event::Mouse(mouse::Event::ButtonReleased(_)) = event {
        match interaction {
            Interaction::Panning { .. } | Interaction::Zoomin { .. } => {
                *interaction = Interaction::None;
            }
            _ => {}
        }
    }

    if let Interaction::Ruler { .. } = interaction
        && cursor_position.is_none()
    {
        *interaction = Interaction::None;
    }

    match event {
        Event::Mouse(mouse_event) => {
            let state = chart.state();

            match mouse_event {
                mouse::Event::ButtonPressed(button) => {
                    let cursor_in_bounds = cursor_position?;

                    if let mouse::Button::Left = button {
                        match interaction {
                            Interaction::None
                            | Interaction::Panning { .. }
                            | Interaction::Zoomin { .. } => {
                                *interaction = Interaction::Panning {
                                    translation: state.translation,
                                    start: cursor_in_bounds,
                                };
                            }
                            Interaction::Ruler { start } if start.is_none() => {
                                *interaction = Interaction::Ruler {
                                    start: Some(cursor_in_bounds),
                                };
                            }
                            Interaction::Ruler { .. } => {
                                *interaction = Interaction::None;
                            }
                        }
                    }
                    Some(canvas::Action::request_redraw().and_capture())
                }
                mouse::Event::CursorMoved { .. } => match *interaction {
                    Interaction::Panning { translation, start } => {
                        let cursor_in_bounds = cursor_position?;
                        let msg = Message::Translated(
                            translation + (cursor_in_bounds - start) * (1.0 / state.scaling),
                        );
                        Some(canvas::Action::publish(msg).and_capture())
                    }
                    Interaction::None | Interaction::Ruler { .. } => {
                        Some(canvas::Action::publish(Message::CrosshairMoved))
                    }
                    _ => None,
                },
                mouse::Event::WheelScrolled { delta } => {
                    cursor_position?;

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

                    if let Some(Autoscale::FitToVisible) = state.layout.autoscale {
                        return Some(
                            canvas::Action::publish(Message::XScaling(
                                y / 2.0,
                                cursor_to_center.x,
                                false,
                            ))
                            .and_capture(),
                        );
                    }

                    let should_adjust_cell_width = match (y.signum(), state.scaling) {
                        (-1.0, scaling)
                            if scaling == max_scaling && state.cell_width > default_cell_width =>
                        {
                            true
                        }
                        (1.0, scaling)
                            if scaling == min_scaling && state.cell_width < default_cell_width =>
                        {
                            true
                        }
                        (1.0, scaling)
                            if scaling == max_scaling && state.cell_width < max_cell_width =>
                        {
                            true
                        }
                        (-1.0, scaling)
                            if scaling == min_scaling && state.cell_width > min_cell_width =>
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
                    if (*y < 0.0 && state.scaling > min_scaling)
                        || (*y > 0.0 && state.scaling < max_scaling)
                    {
                        let old_scaling = state.scaling;
                        let scaling = (state.scaling * (1.0 + y / ZOOM_SENSITIVITY))
                            .clamp(min_scaling, max_scaling);

                        let denominator = old_scaling * scaling;
                        let vector_diff = if denominator.abs() > 0.0001 {
                            let factor = scaling - old_scaling;
                            Vector::new(
                                cursor_to_center.x * factor / denominator,
                                cursor_to_center.y * factor / denominator,
                            )
                        } else {
                            Vector::default()
                        };

                        let translation = state.translation - vector_diff;

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
        Event::Keyboard(keyboard_event) => {
            cursor_position?;
            match keyboard_event {
                iced::keyboard::Event::KeyPressed { key, .. } => match key.as_ref() {
                    keyboard::Key::Named(keyboard::key::Named::Shift) => {
                        *interaction = Interaction::Ruler { start: None };
                        Some(canvas::Action::request_redraw().and_capture())
                    }
                    keyboard::Key::Named(keyboard::key::Named::Escape) => {
                        *interaction = Interaction::None;
                        Some(canvas::Action::request_redraw().and_capture())
                    }
                    _ => None,
                },
                _ => None,
            }
        }
        _ => None,
    }
}

pub enum Action {
    ErrorOccurred(data::InternalError),
    FetchRequested(uuid::Uuid, FetchRange),
}

pub fn update<T: Chart>(chart: &mut T, message: &Message) {
    match message {
        Message::DoubleClick(scale) => {
            let default_chart_width = T::default_cell_width(chart);
            let autoscaled_coords = chart.autoscaled_coords();
            let supports_fit_autoscaling = chart.supports_fit_autoscaling();

            let state = chart.mut_state();

            match scale {
                AxisScaleClicked::X => {
                    state.cell_width = default_chart_width;
                    state.translation = autoscaled_coords;
                }
                AxisScaleClicked::Y => {
                    if supports_fit_autoscaling {
                        state.layout.autoscale = Some(Autoscale::FitToVisible);
                        state.scaling = 1.0;
                    } else {
                        state.layout.autoscale = Some(Autoscale::CenterLatest);
                    }
                }
            }
        }
        Message::Translated(translation) => {
            let state = chart.mut_state();

            if let Some(Autoscale::FitToVisible) = state.layout.autoscale {
                state.translation.x = translation.x;
            } else {
                state.translation = *translation;
                state.layout.autoscale = None;
            }
        }
        Message::Scaled(scaling, translation) => {
            let state = chart.mut_state();
            state.scaling = *scaling;
            state.translation = *translation;

            state.layout.autoscale = None;
        }
        Message::AutoscaleToggled => {
            let supports_fit_autoscaling = chart.supports_fit_autoscaling();
            let state = chart.mut_state();

            let current_autoscale = state.layout.autoscale;
            state.layout.autoscale = {
                match current_autoscale {
                    None => Some(Autoscale::CenterLatest),
                    Some(Autoscale::CenterLatest) => {
                        if supports_fit_autoscaling {
                            Some(Autoscale::FitToVisible)
                        } else {
                            None
                        }
                    }
                    Some(Autoscale::FitToVisible) => None,
                }
            };

            if state.layout.autoscale.is_some() {
                state.scaling = 1.0;
            }
        }
        Message::XScaling(delta, cursor_to_center_x, is_wheel_scroll) => {
            let min_cell_width = T::min_cell_width(chart);
            let max_cell_width = T::max_cell_width(chart);

            let state = chart.mut_state();

            if !(*delta < 0.0 && state.cell_width > min_cell_width
                || *delta > 0.0 && state.cell_width < max_cell_width)
            {
                return;
            }

            let is_fit_to_visible_zoom =
                !is_wheel_scroll && matches!(state.layout.autoscale, Some(Autoscale::FitToVisible));

            let zoom_factor = if is_fit_to_visible_zoom {
                ZOOM_SENSITIVITY / 1.5
            } else if *is_wheel_scroll {
                ZOOM_SENSITIVITY
            } else {
                ZOOM_SENSITIVITY * 3.0
            };

            let new_width = (state.cell_width * (1.0 + delta / zoom_factor))
                .clamp(min_cell_width, max_cell_width);

            if is_fit_to_visible_zoom {
                let anchor_interval = {
                    let latest_x_coord = state.interval_to_x(state.latest_x);
                    if state.is_interval_x_visible(latest_x_coord) {
                        state.latest_x
                    } else {
                        let visible_region = state.visible_region(state.bounds.size());
                        state.x_to_interval(visible_region.x + visible_region.width)
                    }
                };

                let old_anchor_chart_x = state.interval_to_x(anchor_interval);

                state.cell_width = new_width;

                let new_anchor_chart_x = state.interval_to_x(anchor_interval);

                let shift = new_anchor_chart_x - old_anchor_chart_x;
                state.translation.x -= shift;
            } else {
                let (old_scaling, old_translation_x) = { (state.scaling, state.translation.x) };

                let latest_x = state.interval_to_x(state.latest_x);
                let is_interval_x_visible = state.is_interval_x_visible(latest_x);

                let cursor_chart_x = {
                    if *is_wheel_scroll || !is_interval_x_visible {
                        cursor_to_center_x / old_scaling - old_translation_x
                    } else {
                        latest_x / old_scaling - old_translation_x
                    }
                };

                let new_cursor_x = match state.basis {
                    Basis::Time(_) => {
                        let cursor_time = state.x_to_interval(cursor_chart_x);
                        state.cell_width = new_width;

                        state.interval_to_x(cursor_time)
                    }
                    Basis::Tick(_) => {
                        let tick_index = cursor_chart_x / state.cell_width;
                        state.cell_width = new_width;

                        tick_index * state.cell_width
                    }
                };

                if *is_wheel_scroll || !is_interval_x_visible {
                    if !new_cursor_x.is_nan() && !cursor_chart_x.is_nan() {
                        state.translation.x -= new_cursor_x - cursor_chart_x;
                    }

                    state.layout.autoscale = None;
                }
            }
        }
        Message::YScaling(delta, cursor_to_center_y, is_wheel_scroll) => {
            let min_cell_height = T::min_cell_height(chart);
            let max_cell_height = T::max_cell_height(chart);

            let state = chart.mut_state();

            if state.layout.autoscale == Some(Autoscale::FitToVisible) {
                state.layout.autoscale = None;
            }

            if *delta < 0.0 && state.cell_height > min_cell_height
                || *delta > 0.0 && state.cell_height < max_cell_height
            {
                let (old_scaling, old_translation_y) = { (state.scaling, state.translation.y) };

                let zoom_factor = if *is_wheel_scroll {
                    ZOOM_SENSITIVITY
                } else {
                    ZOOM_SENSITIVITY * 3.0
                };

                let new_height = (state.cell_height * (1.0 + delta / zoom_factor))
                    .clamp(min_cell_height, max_cell_height);

                let cursor_chart_y = cursor_to_center_y / old_scaling - old_translation_y;

                let cursor_price = state.y_to_price(cursor_chart_y);

                state.cell_height = new_height;

                let new_cursor_y = state.price_to_y(cursor_price);

                state.translation.y -= new_cursor_y - cursor_chart_y;

                if *is_wheel_scroll {
                    state.layout.autoscale = None;
                }
            }
        }
        Message::BoundsChanged(bounds) => {
            let state = chart.mut_state();

            // calculate how center shifted
            let old_center_x = state.bounds.width / 2.0;
            let new_center_x = bounds.width / 2.0;
            let center_delta_x = (new_center_x - old_center_x) / state.scaling;

            state.bounds = *bounds;

            if state.layout.autoscale != Some(Autoscale::CenterLatest) {
                state.translation.x += center_delta_x;
            }
        }
        Message::SplitDragged(split, size) => {
            let state = chart.mut_state();

            if let Some(split) = state.layout.splits.get_mut(*split) {
                *split = (size * 100.0).round() / 100.0;
            }
        }
        Message::CrosshairMoved => return chart.invalidate_crosshair(),
    }
    chart.invalidate_all();
}

pub fn view<'a, T: Chart>(
    chart: &'a T,
    indicators: &'a [T::IndicatorKind],
    timezone: data::UserTimezone,
) -> Element<'a, Message> {
    if chart.is_empty() {
        return center(text("Waiting for data...").size(16)).into();
    }

    let state = chart.state();

    let axis_labels_x = Canvas::new(AxisLabelsX {
        labels_cache: &state.cache.x_labels,
        scaling: state.scaling,
        translation_x: state.translation.x,
        max: state.latest_x,
        basis: state.basis,
        cell_width: state.cell_width,
        timezone,
        chart_bounds: state.bounds,
        interval_keys: chart.interval_keys(),
        autoscaling: state.layout.autoscale,
    })
    .width(Length::Fill)
    .height(Length::Fill);

    let buttons = {
        let (autoscale_btn_placeholder, autoscale_btn_tooltip) = match state.layout.autoscale {
            Some(Autoscale::CenterLatest) => (text("C"), Some("Center last price")),
            Some(Autoscale::FitToVisible) => (text("A"), Some("Auto")),
            None => (text("C"), Some("Toggle autoscaling")),
        };
        let is_active = state.layout.autoscale.is_some();

        let autoscale_button = button(
            autoscale_btn_placeholder
                .size(10)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center),
        )
        .height(Length::Fill)
        .on_press(Message::AutoscaleToggled)
        .style(move |theme: &Theme, status| style::button::transparent(theme, status, is_active));

        row![
            iced::widget::horizontal_space(),
            tooltip(
                autoscale_button,
                autoscale_btn_tooltip,
                iced::widget::tooltip::Position::Top
            ),
        ]
        .padding(2)
    };

    let y_labels_width = state.y_labels_width();

    let content = {
        let axis_labels_y = Canvas::new(AxisLabelsY {
            labels_cache: &state.cache.y_labels,
            translation_y: state.translation.y,
            scaling: state.scaling,
            decimals: state.decimals,
            min: state.base_price_y,
            last_price: state.last_price,
            tick_size: state.tick_size,
            cell_height: state.cell_height,
            basis: state.basis,
            chart_bounds: state.bounds,
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
            .width(y_labels_width)
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

            MultiSplit::new(panels, &state.layout.splits, |index, position| {
                Message::SplitDragged(index, position)
            })
            .into()
        }
    };

    column![
        content,
        horizontal_rule(1).style(style::split_ruler),
        row![
            container(
                mouse_area(axis_labels_x)
                    .on_double_click(Message::DoubleClick(AxisScaleClicked::X))
            )
            .padding(padding::right(1))
            .width(Length::FillPortion(10))
            .height(Length::Fixed(26.0)),
            buttons.width(y_labels_width).height(Length::Fixed(26.0))
        ]
    ]
    .padding(padding::left(1).right(1).bottom(1))
    .into()
}

pub trait PlotConstants {
    fn min_scaling(&self) -> f32;
    fn max_scaling(&self) -> f32;
    fn max_cell_width(&self) -> f32;
    fn min_cell_width(&self) -> f32;
    fn max_cell_height(&self) -> f32;
    fn min_cell_height(&self) -> f32;
    fn default_cell_width(&self) -> f32;
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

    fn clear_crosshair(&self) {
        self.crosshair.clear();
        self.y_labels.clear();
        self.x_labels.clear();
    }
}

pub struct ViewState {
    cache: Caches,
    bounds: Rectangle,
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
    layout: ViewConfig,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            cache: Caches::default(),
            translation: Vector::default(),
            bounds: Rectangle::default(),
            basis: Timeframe::M5.into(),
            last_price: None,
            scaling: 1.0,
            cell_width: 4.0,
            cell_height: 3.0,
            base_price_y: 0.0,
            latest_x: 0,
            tick_size: 0.0,
            decimals: 0,
            ticker_info: None,
            layout: ViewConfig::default(),
        }
    }
}

impl ViewState {
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
            Basis::Time(timeframe) => {
                let interval = timeframe.to_milliseconds();
                (
                    self.x_to_interval(region.x).saturating_sub(interval / 2),
                    self.x_to_interval(region.x + region.width)
                        .saturating_add(interval / 2),
                )
            }
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
                let interval = timeframe.to_milliseconds() as f64;
                let cell_width = f64::from(self.cell_width);

                let diff = value as f64 - self.latest_x as f64;
                (diff / interval * cell_width) as f32
            }
            Basis::Tick(_) => -((value as f32) * self.cell_width),
        }
    }

    fn x_to_interval(&self, x: f32) -> u64 {
        match self.basis {
            Basis::Time(timeframe) => {
                let interval = timeframe.to_milliseconds();

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
        interaction: &Interaction,
    ) -> (f32, u64) {
        let region = self.visible_region(bounds);
        let dashed_line = style::dashed_line(theme);

        let highest = self.y_to_price(region.y);
        let lowest = self.y_to_price(region.y + region.height);

        if let Interaction::Ruler { start: Some(start) } = interaction {
            let p1 = *start;
            let p2 = cursor_position;

            let snap_y = |y: f32| {
                let ratio = y / bounds.height;
                let price = highest + ratio * (lowest - highest);
                let rounded_price = data::util::round_to_tick(price, self.tick_size);
                let snap_ratio = (rounded_price - highest) / (lowest - highest);
                snap_ratio * bounds.height
            };

            let snap_x = |x: f32| {
                let (_, snap_ratio) = self.snap_x_to_timestamp(x, bounds, region);
                snap_ratio * bounds.width
            };

            let snapped_p1_x = snap_x(p1.x);
            let snapped_p1_y = snap_y(p1.y);
            let snapped_p2_x = snap_x(p2.x);
            let snapped_p2_y = snap_y(p2.y);

            let price1 = self.y_to_price(snapped_p1_y);
            let price2 = self.y_to_price(snapped_p2_y);

            let pct = if price1 == 0.0 {
                0.0
            } else {
                ((price2 - price1) / price1) * 100.0
            };
            let pct_text = format!("{:.2}%", pct);

            let interval_diff: String = match self.basis {
                Basis::Time(_) => {
                    let (timestamp1, _) = self.snap_x_to_timestamp(p1.x, bounds, region);
                    let (timestamp2, _) = self.snap_x_to_timestamp(p2.x, bounds, region);

                    let diff_ms: u64 = timestamp1.abs_diff(timestamp2);
                    data::util::format_duration_ms(diff_ms)
                }
                Basis::Tick(_) => {
                    let (tick1, _) = self.snap_x_to_timestamp(p1.x, bounds, region);
                    let (tick2, _) = self.snap_x_to_timestamp(p2.x, bounds, region);

                    let tick_diff = tick1.abs_diff(tick2);
                    format!("{} ticks", tick_diff)
                }
            };

            let rect_x = snapped_p1_x.min(snapped_p2_x);
            let rect_y = snapped_p1_y.min(snapped_p2_y);
            let rect_w = (snapped_p1_x - snapped_p2_x).abs();
            let rect_h = (snapped_p1_y - snapped_p2_y).abs();

            let palette = theme.extended_palette();

            frame.fill_rectangle(
                Point::new(rect_x, rect_y),
                Size::new(rect_w, rect_h),
                palette.primary.base.color.scale_alpha(0.08),
            );
            let corners = [
                Point::new(rect_x, rect_y),
                Point::new(rect_x + rect_w, rect_y),
                Point::new(rect_x, rect_y + rect_h),
                Point::new(rect_x + rect_w, rect_y + rect_h),
            ];

            let (text_corner, idx) = corners
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| {
                    let da = (a.x - p2.x).hypot(a.y - p2.y);
                    let db = (b.x - p2.x).hypot(b.y - p2.y);
                    da.partial_cmp(&db).unwrap()
                })
                .map(|(i, &c)| (c, i))
                .unwrap();

            let text_padding = 8.0;
            let text_pos = match idx {
                0 => Point::new(text_corner.x + text_padding, text_corner.y + text_padding),
                1 => Point::new(text_corner.x - text_padding, text_corner.y + text_padding),
                2 => Point::new(text_corner.x + text_padding, text_corner.y - text_padding),
                3 => Point::new(text_corner.x - text_padding, text_corner.y - text_padding),
                _ => text_corner,
            };

            let datapoints_text = match self.basis {
                Basis::Time(timeframe) => {
                    let interval_ms = timeframe.to_milliseconds();
                    let (timestamp1, _) = self.snap_x_to_timestamp(p1.x, bounds, region);
                    let (timestamp2, _) = self.snap_x_to_timestamp(p2.x, bounds, region);

                    let diff_ms = timestamp1.abs_diff(timestamp2);
                    let datapoints = (diff_ms / interval_ms).max(1);
                    format!("{} bars", datapoints)
                }
                Basis::Tick(aggregation) => {
                    let (tick1, _) = self.snap_x_to_timestamp(p1.x, bounds, region);
                    let (tick2, _) = self.snap_x_to_timestamp(p2.x, bounds, region);

                    let tick_diff = tick1.abs_diff(tick2);
                    let datapoints = (tick_diff / u64::from(aggregation.0)).max(1);
                    format!("{} bars", datapoints)
                }
            };

            let label_text = format!("{}, {} | {}", datapoints_text, interval_diff, pct_text);

            let text_width = (label_text.len() as f32) * TEXT_SIZE * 0.6;
            let text_height = TEXT_SIZE * 1.2;
            let rect_padding = 4.0;

            let (bg_x, bg_y) = match idx {
                0 => (text_pos.x - rect_padding, text_pos.y - rect_padding),
                1 => (
                    text_pos.x - text_width - rect_padding,
                    text_pos.y - rect_padding,
                ),
                2 => (
                    text_pos.x - rect_padding,
                    text_pos.y - text_height - rect_padding,
                ),
                3 => (
                    text_pos.x - text_width - rect_padding,
                    text_pos.y - text_height - rect_padding,
                ),
                _ => (
                    text_pos.x - text_width / 2.0 - rect_padding,
                    text_pos.y - text_height / 2.0 - rect_padding,
                ),
            };

            frame.fill_rectangle(
                Point::new(bg_x, bg_y),
                Size::new(
                    text_width + rect_padding * 2.0,
                    text_height + rect_padding * 2.0,
                ),
                palette.background.weakest.color.scale_alpha(0.9),
            );

            frame.fill_text(iced::widget::canvas::Text {
                content: label_text,
                position: text_pos,
                color: palette.background.base.text,
                size: iced::Pixels(11.0),
                align_x: match idx {
                    0 | 2 => Alignment::Start.into(),
                    1 | 3 => Alignment::End.into(),
                    _ => Alignment::Center.into(),
                },
                align_y: match idx {
                    0 | 1 => Alignment::Start.into(),
                    2 | 3 => Alignment::End.into(),
                    _ => Alignment::Center.into(),
                },
                font: style::AZERET_MONO,
                ..Default::default()
            });
        }

        // Horizontal price line
        let crosshair_ratio = cursor_position.y / bounds.height;
        let crosshair_price = highest + crosshair_ratio * (lowest - highest);

        let rounded_price = data::util::round_to_tick(crosshair_price, self.tick_size);
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
            Basis::Time(_) => {
                let (rounded_timestamp, snap_ratio) =
                    self.snap_x_to_timestamp(cursor_position.x, bounds, region);

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
                let (chart_x_min, chart_x_max) = (region.x, region.x + region.width);
                let crosshair_pos = chart_x_min + (cursor_position.x / bounds.width) * region.width;

                let cell_index = (crosshair_pos / self.cell_width).round();

                let snapped_crosshair = cell_index * self.cell_width;
                let snap_ratio = (snapped_crosshair - chart_x_min) / (chart_x_max - chart_x_min);

                let rounded_tick = (-cell_index as u64) * (u64::from(aggregation.0));

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

    fn layout(&self) -> ViewConfig {
        let layout = &self.layout;
        ViewConfig {
            splits: layout.splits.clone(),
            autoscale: layout.autoscale,
        }
    }

    fn y_labels_width(&self) -> Length {
        let base_value = self.base_price_y;
        let decimals = self.decimals;

        let value = format!("{base_value:.decimals$}");
        let width = (value.len() as f32 * TEXT_SIZE * 0.8).max(72.0);

        Length::Fixed(width.ceil())
    }

    fn snap_x_to_timestamp(&self, x: f32, bounds: Size, region: Rectangle) -> (u64, f32) {
        let x_ratio = x / bounds.width;

        match self.basis {
            Basis::Time(timeframe) => {
                let interval = timeframe.to_milliseconds();
                let earliest = self.x_to_interval(region.x) as f64;
                let latest = self.x_to_interval(region.x + region.width) as f64;

                let millis_at_x = earliest + f64::from(x_ratio) * (latest - earliest);

                let rounded_timestamp = (millis_at_x / (interval as f64)).round() as u64 * interval;

                let snap_ratio = if latest - earliest > 0.0 {
                    ((rounded_timestamp as f64 - earliest) / (latest - earliest)) as f32
                } else {
                    0.5
                };

                (rounded_timestamp, snap_ratio)
            }
            Basis::Tick(aggregation) => {
                let (chart_x_min, chart_x_max) = (region.x, region.x + region.width);
                let chart_x = chart_x_min + x_ratio * (chart_x_max - chart_x_min);

                let cell_index = (chart_x / self.cell_width).round();
                let snapped_x = cell_index * self.cell_width;

                let snap_ratio = if chart_x_max - chart_x_min > 0.0 {
                    (snapped_x - chart_x_min) / (chart_x_max - chart_x_min)
                } else {
                    0.5
                };

                let rounded_tick = (-cell_index as u64) * u64::from(aggregation.0);

                (rounded_tick, snap_ratio)
            }
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

fn draw_volume_bar(
    frame: &mut canvas::Frame,
    start_x: f32,
    start_y: f32,
    buy_qty: f32,
    sell_qty: f32,
    max_qty: f32,
    bar_length: f32,
    thickness: f32,
    buy_color: iced::Color,
    sell_color: iced::Color,
    bar_color_alpha: f32,
    horizontal: bool,
) {
    let total_qty = buy_qty + sell_qty;
    if total_qty <= 0.0 || max_qty <= 0.0 {
        return;
    }

    let total_bar_length = (total_qty / max_qty) * bar_length;

    let buy_proportion = buy_qty / total_qty;
    let sell_proportion = sell_qty / total_qty;

    let buy_bar_length = buy_proportion * total_bar_length;
    let sell_bar_length = sell_proportion * total_bar_length;

    if horizontal {
        let start_y = start_y - (thickness / 2.0);

        if sell_qty > 0.0 {
            frame.fill_rectangle(
                Point::new(start_x, start_y),
                Size::new(sell_bar_length, thickness),
                sell_color.scale_alpha(bar_color_alpha),
            );
        }

        if buy_qty > 0.0 {
            frame.fill_rectangle(
                Point::new(start_x + sell_bar_length, start_y),
                Size::new(buy_bar_length, thickness),
                buy_color.scale_alpha(bar_color_alpha),
            );
        }
    } else {
        let start_x = start_x - (thickness / 2.0);

        if sell_qty > 0.0 {
            frame.fill_rectangle(
                Point::new(start_x, start_y + (bar_length - sell_bar_length)),
                Size::new(thickness, sell_bar_length),
                sell_color.scale_alpha(bar_color_alpha),
            );
        }

        if buy_qty > 0.0 {
            frame.fill_rectangle(
                Point::new(
                    start_x,
                    start_y + (bar_length - sell_bar_length - buy_bar_length),
                ),
                Size::new(thickness, buy_bar_length),
                buy_color.scale_alpha(bar_color_alpha),
            );
        }
    }
}
