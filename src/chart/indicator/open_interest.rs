use std::collections::BTreeMap;

use iced::widget::canvas::{self, Cache, Event, Geometry, Path, Stroke};
use iced::widget::{Canvas, center, container, row, text, vertical_rule};
use iced::{Element, Length, Point, Rectangle, Renderer, Size, Theme, Vector, mouse};

use crate::chart::{Basis, Caches, Interaction, Message, ViewState};
use crate::style::{self, dashed_line};
use data::util::{format_with_commas, guesstimate_ticks, round_to_tick};
use exchange::Timeframe;

pub fn indicator_elem<'a>(
    chart_state: &'a ViewState,
    cache: &'a Caches,
    datapoints: &'a BTreeMap<u64, f32>,
    earliest: u64,
    latest: u64,
) -> Element<'a, Message> {
    let (mut max_value, mut min_value) = {
        match chart_state.basis {
            Basis::Time(timeframe) => {
                let Some(ticker_info) = chart_state.ticker_info else {
                    return row![].into();
                };

                let exchange = ticker_info.exchange();
                if exchange == exchange::adapter::Exchange::HyperliquidLinear {
                    return center(text(format!(
                        "WIP: Open Interest is not available for {exchange}",
                    )))
                    .into();
                }

                if timeframe < Timeframe::M5
                    || timeframe == Timeframe::H2
                    || timeframe > Timeframe::H4
                {
                    return center(text(format!(
                        "WIP: Open Interest is not available on {timeframe} timeframe",
                    )))
                    .into();
                } else {
                    if latest < earliest {
                        return row![].into();
                    }
                    datapoints
                        .range(earliest..=latest)
                        .fold((f32::MIN, f32::MAX), |(max, min), (_, value)| {
                            (max.max(*value), min.min(*value))
                        })
                }
            }
            Basis::Tick(_) => {
                return center(text("WIP: Open Interest is not available for tick charts.")).into();
            }
        }
    };

    let value_range = max_value - min_value;
    let padding = value_range * 0.01;
    max_value += padding;
    min_value -= padding;

    let indi_chart = Canvas::new(OpenInterest {
        indicator_cache: &cache.main,
        crosshair_cache: &cache.crosshair,
        chart_state,
        max_value,
        timeseries: datapoints,
    })
    .height(Length::Fill)
    .width(Length::Fill);

    let indi_labels = Canvas::new(super::IndicatorLabel {
        label_cache: &cache.y_labels,
        max: max_value,
        min: min_value,
        chart_bounds: chart_state.bounds,
    })
    .height(Length::Fill)
    .width(chart_state.y_labels_width());

    row![
        indi_chart,
        vertical_rule(1).style(style::split_ruler),
        container(indi_labels),
    ]
    .into()
}

pub struct OpenInterest<'a> {
    pub indicator_cache: &'a Cache,
    pub crosshair_cache: &'a Cache,
    pub chart_state: &'a ViewState,
    pub max_value: f32,
    pub timeseries: &'a BTreeMap<u64, f32>,
}

impl OpenInterest<'_> {
    fn visible_region(&self, size: Size) -> Rectangle {
        let width = size.width / self.chart_state.scaling;
        let height = size.height / self.chart_state.scaling;

        Rectangle {
            x: -self.chart_state.translation.x - width / 2.0,
            y: 0.0,
            width,
            height,
        }
    }
}

impl canvas::Program<Message> for OpenInterest<'_> {
    type State = Interaction;

    fn update(
        &self,
        interaction: &mut Interaction,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let message = match *interaction {
                    Interaction::None => {
                        if cursor.is_over(bounds) {
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
        let chart_state = self.chart_state;

        if chart_state.bounds.width == 0.0 {
            return vec![];
        }

        let timeframe: u64 = match chart_state.basis {
            Basis::Time(interval) => interval.into(),
            Basis::Tick(_) => {
                // TODO: implement
                return vec![];
            }
        };

        if self.max_value == 0.0 {
            return vec![];
        }

        let center = Vector::new(bounds.width / 2.0, bounds.height / 2.0);
        let palette = theme.extended_palette();

        let indicator = self.indicator_cache.draw(renderer, bounds.size(), |frame| {
            frame.translate(center);
            frame.scale(chart_state.scaling);
            frame.translate(Vector::new(
                chart_state.translation.x,
                (-bounds.height / chart_state.scaling) / 2.0,
            ));

            let region = self.visible_region(frame.size());

            let (earliest, latest) = chart_state.interval_range(&region);

            let mut max_value: f32 = f32::MIN;
            let mut min_value: f32 = f32::MAX;

            self.timeseries
                .range(earliest..=latest)
                .for_each(|(_, value)| {
                    max_value = max_value.max(*value);
                    min_value = min_value.min(*value);
                });

            let padding = (max_value - min_value) * 0.08;
            max_value += padding;
            min_value -= padding;

            let points: Vec<Point> = self
                .timeseries
                .range(earliest..=latest)
                .map(|(timestamp, value)| {
                    let x_position = chart_state.interval_to_x(*timestamp);
                    let normalized_height = if max_value > min_value {
                        (value - min_value) / (max_value - min_value)
                    } else {
                        0.0
                    };
                    let y_position = (bounds.height / chart_state.scaling)
                        - (normalized_height * (bounds.height / chart_state.scaling));

                    Point::new(x_position - (chart_state.cell_width / 2.0), y_position)
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
                        Stroke::with_color(stroke, palette.secondary.strong.color),
                    );
                }
            }

            let radius = (chart_state.cell_width * 0.2).min(5.0);
            for point in points {
                frame.fill(
                    &Path::circle(Point::new(point.x, point.y), radius),
                    palette.secondary.strong.color,
                );
            }
        });

        let crosshair = self.crosshair_cache.draw(renderer, bounds.size(), |frame| {
            let dashed_line = dashed_line(theme);

            if let Some(cursor_position) = cursor.position_in(chart_state.bounds) {
                let region = self.visible_region(frame.size());

                // Vertical time line
                let earliest = chart_state.x_to_interval(region.x) as f64;
                let latest = chart_state.x_to_interval(region.x + region.width) as f64;

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

                let oi_data = {
                    let exact_match = self
                        .timeseries
                        .iter()
                        .find(|(time, _)| **time == rounded_timestamp);

                    if exact_match.is_none()
                        && rounded_timestamp > self.timeseries.keys().last().copied().unwrap_or(0)
                    {
                        self.timeseries.iter().last()
                    } else {
                        exact_match
                    }
                };

                if let Some((_, oi_value)) = oi_data {
                    let next_value = self
                        .timeseries
                        .range((rounded_timestamp + timeframe)..=u64::MAX)
                        .next()
                        .map(|(_, val)| *val);

                    let change_text = if let Some(next_oi) = next_value {
                        let difference = next_oi - *oi_value;
                        let sign = if difference >= 0.0 { "+" } else { "" };
                        format!("Change: {}{}", sign, format_with_commas(difference))
                    } else {
                        "Change: N/A".to_string()
                    };
                    let value_text = format!("Value: {}", format_with_commas(*oi_value));

                    let tooltip_text = format!("{}\n{}", value_text, change_text);
                    let tooltip_bg_width = value_text.len().max(change_text.len()) as f32 * 8.0;

                    frame.fill_rectangle(
                        Point::new(4.0, 0.0),
                        Size::new(tooltip_bg_width, 28.0),
                        palette.background.weakest.color.scale_alpha(0.9),
                    );

                    let text = canvas::Text {
                        content: tooltip_text,
                        position: Point::new(8.0, 2.0),
                        size: iced::Pixels(10.0),
                        color: palette.background.base.text,
                        font: style::AZERET_MONO,
                        ..canvas::Text::default()
                    };
                    frame.fill_text(text);
                }
            } else if let Some(cursor_position) = cursor.position_in(bounds) {
                // Horizontal price line
                let highest = self.max_value;
                let lowest = 0.0;

                let crosshair_ratio = cursor_position.y / bounds.height;
                let crosshair_price = highest + crosshair_ratio * (lowest - highest);

                let rounded_price =
                    round_to_tick(crosshair_price, guesstimate_ticks(highest - lowest));
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
            Interaction::None if cursor.is_over(bounds) => mouse::Interaction::Crosshair,
            _ => mouse::Interaction::default(),
        }
    }
}
