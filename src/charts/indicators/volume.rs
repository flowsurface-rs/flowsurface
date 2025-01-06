use std::collections::BTreeMap;

use iced::widget::{container, row, Canvas};
use iced::{Element, Length};
use iced::{mouse, Point, Rectangle, Renderer, Size, Theme, Vector};
use iced::widget::canvas::{self, Cache, Event, Geometry, LineDash, Path, Stroke};

use crate::charts::{
    round_to_tick, Caches, CommonChartData, Interaction, Message
};
use crate::data_providers::format_with_commas;

pub fn create_indicator_elem<'a>(
    chart_state: &'a CommonChartData,
    cache: &'a Caches, 
    data: &'a BTreeMap<i64, (f32, f32)>,
    earliest: i64,
    latest: i64,
) -> Element<'a, Message> {
    let indi_chart = Canvas::new(VolumeIndicator {
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

    let max_volume = data
        .range(earliest..=latest)
        .map(|(_, (buy, sell))| buy.max(*sell))
        .max_by(|a, b| a.partial_cmp(b).unwrap())
        .unwrap_or(0.0);

    let indi_labels = Canvas::new(super::IndicatorLabel {
        label_cache: &cache.y_labels,
        max: max_volume,
        min: 0.0,
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

pub struct VolumeIndicator<'a> {
    pub indicator_cache: &'a Cache,
    pub crosshair_cache: &'a Cache,
    pub crosshair: bool,
    pub max: i64,
    pub scaling: f32,
    pub translation_x: f32,
    pub timeframe: u32,
    pub cell_width: f32,
    pub data_points: &'a BTreeMap<i64, (f32, f32)>,
    pub chart_bounds: Rectangle,
}

impl VolumeIndicator<'_> {
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

impl canvas::Program<Message> for VolumeIndicator<'_> {
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

            let (earliest, latest) = (
                self.x_to_time(region.x) - i64::from(self.timeframe / 2),
                self.x_to_time(region.x + region.width) + i64::from(self.timeframe / 2),
            );

            let mut max_volume: f32 = 0.0;

            self.data_points
                .range(earliest..=latest)
                .for_each(|(_, (buy_volume, sell_volume))| {
                    max_volume = max_volume.max(buy_volume.max(*sell_volume));
                });

            self.data_points.range(earliest..=latest).for_each(
                |(timestamp, (buy_volume, sell_volume))| {
                    let x_position = self.time_to_x(*timestamp);

                    if max_volume > 0.0 {
                        if *buy_volume != -1.0 {
                            let buy_bar_height =
                                (buy_volume / max_volume) * (bounds.height / self.scaling);
                            let sell_bar_height =
                                (sell_volume / max_volume) * (bounds.height / self.scaling);

                            let bar_width = (self.cell_width / 2.0) * 0.9;

                            frame.fill_rectangle(
                                Point::new(
                                    x_position - bar_width,
                                    (region.y + region.height) - sell_bar_height,
                                ),
                                Size::new(bar_width, sell_bar_height),
                                palette.danger.base.color,
                            );

                            frame.fill_rectangle(
                                Point::new(x_position, (region.y + region.height) - buy_bar_height),
                                Size::new(bar_width, buy_bar_height),
                                palette.success.base.color,
                            );
                        } else {
                            let bar_height =
                                (sell_volume / max_volume) * (bounds.height / self.scaling);

                            let bar_width = self.cell_width * 0.9;

                            frame.fill_rectangle(
                                Point::new(
                                    x_position - (bar_width / 2.0),
                                    (bounds.height / self.scaling) - bar_height,
                                ),
                                Size::new(bar_width, bar_height),
                                palette.secondary.strong.color,
                            );
                        }
                    }
                },
            );
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

                    if let Some((_, (buy_v, sell_v))) = self
                        .data_points
                        .iter()
                        .find(|(time, _)| **time == rounded_timestamp)
                    {
                        let mut tooltip_bg_height = 28.0;

                        let tooltip_text: String = if *buy_v != -1.0 {
                            format!(
                                "Buy Volume: {}\nSell Volume: {}",
                                format_with_commas(*buy_v),
                                format_with_commas(*sell_v),
                            )
                        } else {
                            tooltip_bg_height = 14.0;

                            format!(
                                "Volume: {}",
                                format_with_commas(*sell_v),
                            )
                        };

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
                            Size::new(140.0, tooltip_bg_height),
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