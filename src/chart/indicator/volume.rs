use std::collections::BTreeMap;

use iced::widget::canvas::{self, Cache, Event, Geometry, Path};
use iced::widget::{Canvas, container, row, vertical_rule};
use iced::{Element, Length};
use iced::{Point, Rectangle, Renderer, Size, Theme, Vector, mouse};

use crate::chart::{Basis, Caches, Interaction, Message, ViewState};
use crate::style::{self, dashed_line};

use data::util::{format_with_commas, round_to_tick};

pub fn indicator_elem<'a>(
    chart_state: &'a ViewState,
    cache: &'a Caches,
    datapoints: &'a BTreeMap<u64, (f32, f32)>,
    earliest: u64,
    latest: u64,
) -> Element<'a, Message> {
    let max_volume = {
        match chart_state.basis {
            Basis::Time(_) => {
                if latest < earliest {
                    return row![].into();
                }
                datapoints
                    .range(earliest..=latest)
                    .map(
                        |(_, (buy, sell))| {
                            if *buy == -1.0 { *sell } else { buy + sell }
                        },
                    )
                    .max_by(|a, b| a.partial_cmp(b).unwrap())
                    .unwrap_or(0.0)
            }
            Basis::Tick(_) => {
                let mut max_volume: f32 = 0.0;
                let earliest = earliest as usize;
                let latest = latest as usize;

                datapoints
                    .iter()
                    .rev()
                    .enumerate()
                    .filter(|(index, _)| *index <= latest && *index >= earliest)
                    .for_each(|(_, (_, (buy_volume, sell_volume)))| {
                        let total = if *buy_volume == -1.0 {
                            *sell_volume
                        } else {
                            buy_volume + sell_volume
                        };
                        max_volume = max_volume.max(total);
                    });

                max_volume
            }
        }
    };

    let indi_chart = Canvas::new(VolumeIndicator {
        indicator_cache: &cache.main,
        crosshair_cache: &cache.crosshair,
        chart_state,
        datapoints,
        max_volume,
    })
    .height(Length::Fill)
    .width(Length::Fill);

    let indi_labels = Canvas::new(super::IndicatorLabel {
        label_cache: &cache.y_labels,
        max: max_volume,
        min: 0.0,
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

pub struct VolumeIndicator<'a> {
    pub indicator_cache: &'a Cache,
    pub crosshair_cache: &'a Cache,
    pub max_volume: f32,
    pub datapoints: &'a BTreeMap<u64, (f32, f32)>,
    pub chart_state: &'a ViewState,
}

impl VolumeIndicator<'_> {
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

impl canvas::Program<Message> for VolumeIndicator<'_> {
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

        let max_volume = self.max_volume;

        if max_volume == 0.0 {
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

            match chart_state.basis {
                Basis::Time(_) => {
                    if latest < earliest {
                        return;
                    }

                    self.datapoints.range(earliest..=latest).for_each(
                        |(timestamp, (buy_volume, sell_volume))| {
                            let x_position = chart_state.interval_to_x(*timestamp);

                            let (total, delta, delta_positive, bybit_workaround) =
                                if *buy_volume == -1.0 {
                                    // Bybit workaround: only total volume available
                                    (*sell_volume, 0.0, true, true)
                                } else {
                                    let delta = (buy_volume - sell_volume).abs();
                                    let delta_positive = buy_volume >= sell_volume;
                                    (buy_volume + sell_volume, delta, delta_positive, false)
                                };

                            let total_bar_height =
                                (total / max_volume) * (bounds.height / chart_state.scaling);
                            let delta_bar_height =
                                (delta / max_volume) * (bounds.height / chart_state.scaling);

                            let bar_width = chart_state.cell_width * 0.9;

                            let base_color = if delta_positive {
                                palette.success.base.color
                            } else {
                                palette.danger.base.color
                            };

                            frame.fill_rectangle(
                                Point::new(
                                    x_position - (bar_width / 2.0),
                                    (bounds.height / chart_state.scaling) - total_bar_height,
                                ),
                                Size::new(bar_width, total_bar_height),
                                if bybit_workaround {
                                    palette.secondary.strong.color
                                } else {
                                    base_color.scale_alpha(0.3)
                                },
                            );

                            if delta > 0.0 {
                                frame.fill_rectangle(
                                    Point::new(
                                        x_position - (bar_width / 2.0),
                                        (bounds.height / chart_state.scaling) - delta_bar_height,
                                    ),
                                    Size::new(bar_width, delta_bar_height),
                                    base_color,
                                );
                            }
                        },
                    );
                }
                Basis::Tick(_) => {
                    let earliest = earliest as usize;
                    let latest = latest as usize;

                    self.datapoints
                        .iter()
                        .rev()
                        .enumerate()
                        .filter(|(index, _)| *index <= latest && *index >= earliest)
                        .for_each(|(index, (_, (buy_volume, sell_volume)))| {
                            let x_position = chart_state.interval_to_x(index as u64);

                            let (total, delta, delta_positive, bybit_workaround) =
                                if *buy_volume == -1.0 {
                                    (*sell_volume, 0.0, true, true)
                                } else {
                                    let delta = (buy_volume - sell_volume).abs();
                                    let delta_positive = buy_volume >= sell_volume;
                                    (buy_volume + sell_volume, delta, delta_positive, false)
                                };

                            let total_bar_height =
                                (total / max_volume) * (bounds.height / chart_state.scaling);
                            let delta_bar_height =
                                (delta / max_volume) * (bounds.height / chart_state.scaling);

                            let bar_width = chart_state.cell_width * 0.9;

                            let base_color = if delta_positive {
                                palette.success.base.color
                            } else {
                                palette.danger.base.color
                            };

                            frame.fill_rectangle(
                                Point::new(
                                    x_position - (bar_width / 2.0),
                                    (region.y + region.height) - total_bar_height,
                                ),
                                Size::new(bar_width, total_bar_height),
                                if bybit_workaround {
                                    base_color
                                } else {
                                    base_color.scale_alpha(0.3)
                                },
                            );

                            if delta > 0.0 {
                                frame.fill_rectangle(
                                    Point::new(
                                        x_position - (bar_width / 2.0),
                                        (region.y + region.height) - delta_bar_height,
                                    ),
                                    Size::new(bar_width, delta_bar_height),
                                    base_color,
                                );
                            }
                        });
                }
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

                let (rounded_interval, snap_ratio) = match chart_state.basis {
                    Basis::Time(timeframe) => {
                        let interval = timeframe.to_milliseconds();

                        let crosshair_millis = earliest + crosshair_ratio * (latest - earliest);

                        let rounded_timestamp =
                            (crosshair_millis / (interval as f64)).round() as u64 * interval;
                        let snap_ratio =
                            ((rounded_timestamp as f64 - earliest) / (latest - earliest)) as f32;

                        (rounded_timestamp, snap_ratio)
                    }
                    Basis::Tick(_) => {
                        let chart_x_min = region.x;
                        let chart_x_max = region.x + region.width;

                        let crosshair_pos = chart_x_min + crosshair_ratio as f32 * region.width;

                        let cell_index = (crosshair_pos / chart_state.cell_width).round() as i32;
                        let snapped_position = cell_index as f32 * chart_state.cell_width;

                        let snap_ratio =
                            (snapped_position - chart_x_min) / (chart_x_max - chart_x_min);

                        let tick_value = chart_state.x_to_interval(snapped_position);

                        (tick_value, snap_ratio)
                    }
                };

                frame.stroke(
                    &Path::line(
                        Point::new(snap_ratio * bounds.width, 0.0),
                        Point::new(snap_ratio * bounds.width, bounds.height),
                    ),
                    dashed_line,
                );

                let volume_data = match chart_state.basis {
                    Basis::Time(_) => {
                        let exact_match = self
                            .datapoints
                            .iter()
                            .find(|(interval, _)| **interval == rounded_interval);

                        if exact_match.is_none()
                            && rounded_interval
                                > self.datapoints.keys().last().copied().unwrap_or(0)
                        {
                            self.datapoints.iter().last()
                        } else {
                            exact_match
                        }
                    }
                    Basis::Tick(_) => {
                        let index_from_end = rounded_interval as usize;

                        if index_from_end < self.datapoints.len() {
                            self.datapoints.iter().rev().nth(index_from_end)
                        } else if !self.datapoints.is_empty() {
                            let right_edge = chart_state.x_to_interval(region.x + region.width);

                            if rounded_interval <= right_edge {
                                self.datapoints.iter().next_back()
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                };

                if let Some((_, (buy_v, sell_v))) = volume_data {
                    let mut tooltip_bg_height = 28.0;

                    let (tooltip_text, tooltip_bg_width) = if *buy_v == -1.0 {
                        tooltip_bg_height = 14.0;

                        let text = format!("Volume: {}", format_with_commas(*sell_v),);
                        let bg_width = text.len() as f32 * 8.0;

                        (text, bg_width)
                    } else {
                        let buy_volume = format!("Buy Volume: {}\n", format_with_commas(*buy_v));
                        let sell_volume = format!("Sell Volume: {}", format_with_commas(*sell_v));

                        let bg_width = buy_volume.len().max(sell_volume.len()) as f32 * 8.0;

                        let text = format!("{}{}", buy_volume, sell_volume);
                        (text, bg_width)
                    };

                    frame.fill_rectangle(
                        Point::new(4.0, 0.0),
                        Size::new(tooltip_bg_width, tooltip_bg_height),
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
                let highest = max_volume;
                let lowest = 0.0;

                let tick_size = data::util::guesstimate_ticks(highest - lowest);

                let crosshair_ratio = cursor_position.y / bounds.height;
                let crosshair_price = highest + crosshair_ratio * (lowest - highest);

                let rounded_price = round_to_tick(crosshair_price, tick_size);
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
