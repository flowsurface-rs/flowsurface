use crate::{
    chart::{
        Caches, Interaction, Message, TEXT_SIZE, ViewState,
        indicator::{
            kline::{BasisSeriesExt, KlineIndicatorImpl},
            plot::{AnySeries, Series},
        },
    },
    style,
};

use data::chart::{
    BasisSeries, PlotData,
    kline::{FootprintSummary, KlineDataPoint},
};
use data::util::abbr_large_numbers;
use exchange::{Kline, Trade};
use iced::{
    Alignment, Color, Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector,
    mouse,
    widget::{
        Canvas,
        canvas::{self, Cache, Geometry},
        container, row, rule, space,
    },
};
use std::{collections::BTreeMap, ops::RangeInclusive};

pub struct BarAnalysisIndicator {
    cache: Caches,
    data: BasisSeries<FootprintSummary>,
    settings: BarAnalysisSettings,
}

impl BarAnalysisIndicator {
    pub fn new() -> Self {
        Self {
            cache: Caches::default(),
            data: BasisSeries::default(),
            settings: BarAnalysisSettings::default(),
        }
    }

    fn indicator_elem<'a>(
        &'a self,
        main_chart: &'a ViewState,
        visible_range: RangeInclusive<u64>,
    ) -> Element<'a, Message> {
        let canvas = Canvas::new(BarAnalysisCanvas {
            cache: &self.cache.main,
            ctx: main_chart,
            series: self.data.as_plot_series(),
            settings: self.settings,
            visible_range,
        })
        .height(Length::Fill)
        .width(Length::Fill);

        row![
            canvas,
            rule::vertical(1).style(style::split_ruler),
            container(space::vertical()).width(main_chart.y_labels_width())
        ]
        .into()
    }

    fn rebuild(&mut self, source: &PlotData<KlineDataPoint>) {
        self.data = source.map_basis_series(
            |timeseries| {
                timeseries
                    .datapoints
                    .iter()
                    .filter_map(|(timestamp, dp)| {
                        FootprintSummary::from_trades(&dp.footprint).map(|row| (*timestamp, row))
                    })
                    .collect::<BTreeMap<_, _>>()
            },
            |tickseries| {
                tickseries
                    .datapoints
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, dp)| {
                        FootprintSummary::from_trades(&dp.footprint).map(|row| (idx as u64, row))
                    })
                    .collect::<BTreeMap<_, _>>()
            },
        );
        self.clear_all_caches();
    }
}

impl KlineIndicatorImpl for BarAnalysisIndicator {
    fn clear_all_caches(&mut self) {
        self.cache.clear_all();
    }

    fn clear_crosshair_caches(&mut self) {
        self.cache.clear_crosshair();
    }

    fn element<'a>(
        &'a self,
        chart: &'a ViewState,
        _data_labels_always_visible: bool,
        visible_range: RangeInclusive<u64>,
    ) -> Element<'a, Message> {
        self.indicator_elem(chart, visible_range)
    }

    fn rebuild_from_source(&mut self, source: &PlotData<KlineDataPoint>) {
        self.rebuild(source);
    }

    fn on_insert_klines(&mut self, klines: &[Kline], source: &PlotData<KlineDataPoint>) {
        match source {
            PlotData::TimeBased(timeseries) => {
                if let Some(data) = self.data.time_mut() {
                    for kline in klines {
                        match timeseries.datapoints.get(&kline.time) {
                            Some(dp) => match FootprintSummary::from_trades(&dp.footprint) {
                                Some(summary) => {
                                    data.insert(kline.time, summary);
                                }
                                None => {
                                    data.remove(&kline.time);
                                }
                            },
                            None => {
                                data.remove(&kline.time);
                            }
                        }
                    }
                }
            }
            PlotData::TickBased(_) => {}
        }
        self.clear_all_caches();
    }

    fn on_insert_trades(
        &mut self,
        trades: &[Trade],
        old_dp_len: usize,
        source: &PlotData<KlineDataPoint>,
    ) {
        match source {
            PlotData::TimeBased(timeseries) => {
                let mut affected = Vec::new();
                for trade in trades {
                    let rounded = trade.time.floor_to(timeseries.interval);
                    if !affected.contains(&rounded) {
                        affected.push(rounded);
                    }
                }
                if let Some(data) = self.data.time_mut() {
                    for ts in affected {
                        match timeseries.datapoints.get(&ts) {
                            Some(dp) => match FootprintSummary::from_trades(&dp.footprint) {
                                Some(summary) => {
                                    data.insert(ts, summary);
                                }
                                None => {
                                    data.remove(&ts);
                                }
                            },
                            None => {
                                data.remove(&ts);
                            }
                        }
                    }
                }
            }
            PlotData::TickBased(tick_aggr) => {
                let new_len = tick_aggr.datapoints.len();
                let start = old_dp_len.saturating_sub(1);
                if let Some(data) = self.data.tick_mut() {
                    // Remove entries for any indices that no longer exist
                    // (safety measure - bars are only appended, never removed)
                    data.retain(|&idx, _| idx < new_len as u64);
                    // Recompute the last old bar (may have been modified) and any new bars
                    for idx in start..new_len {
                        if let Some(dp) = tick_aggr.datapoints.get(idx) {
                            match FootprintSummary::from_trades(&dp.footprint) {
                                Some(summary) => {
                                    data.insert(idx as u64, summary);
                                }
                                None => {
                                    data.remove(&(idx as u64));
                                }
                            }
                        }
                    }
                }
            }
        }
        self.clear_all_caches();
    }

    fn on_ticksize_change(&mut self, source: &PlotData<KlineDataPoint>) {
        self.rebuild(source);
    }

    fn on_basis_change(&mut self, source: &PlotData<KlineDataPoint>) {
        self.rebuild(source);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BarAnalysisSettings {
    show_buy_sell: bool,
    show_volume: bool,
    show_delta: bool,
    show_delta_pct: bool,
}

impl Default for BarAnalysisSettings {
    fn default() -> Self {
        Self {
            show_buy_sell: true,
            show_volume: true,
            show_delta: true,
            show_delta_pct: true,
        }
    }
}

struct BarAnalysisCanvas<'a> {
    cache: &'a Cache,
    ctx: &'a ViewState,
    series: AnySeries<'a, FootprintSummary>,
    settings: BarAnalysisSettings,
    visible_range: RangeInclusive<u64>,
}

impl canvas::Program<Message> for BarAnalysisCanvas<'_> {
    type State = Interaction;

    fn update(
        &self,
        _state: &mut Self::State,
        _event: &Event,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        None
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let ctx = self.ctx;
        let palette = theme.extended_palette();

        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            if ctx.bounds.width == 0.0 || ctx.scaling <= f32::EPSILON {
                return;
            }

            let pane_height = bounds.height / ctx.scaling;
            let table_top = 0.0;
            let table_height = pane_height.max(24.0);
            let rows_count = {
                let mut count: f32 = 0.0;
                if self.settings.show_buy_sell {
                    count += 2.0;
                }
                if self.settings.show_volume {
                    count += 1.0;
                }
                if self.settings.show_delta {
                    count += 1.0;
                }
                if self.settings.show_delta_pct {
                    count += 1.0;
                }
                count.max(1.0)
            };
            let row_height = table_height / rows_count;
            let column_width = ctx.cell_width;
            let text_size = (row_height * 0.5).clamp(5.0, TEXT_SIZE);
            let border_color = palette.background.strong.color;

            let mut header_labels: Vec<&str> = Vec::with_capacity(5);
            if self.settings.show_buy_sell {
                header_labels.push("Buy");
                header_labels.push("Sell");
            }
            if self.settings.show_volume {
                header_labels.push("Total");
            }
            if self.settings.show_delta {
                header_labels.push("Δ");
            }
            if self.settings.show_delta_pct {
                header_labels.push("Δ%");
            }

            let max_label_chars = header_labels
                .iter()
                .map(|s| s.chars().count())
                .max()
                .unwrap_or(3);
            let header_width = text_size * max_label_chars as f32 * 0.72 + 12.0;
            let screen_text_size = text_size * ctx.scaling;
            let screen_row_height = row_height * ctx.scaling;
            let screen_table_top = table_top * ctx.scaling;
            let screen_header_width = header_width * ctx.scaling;
            let header_x = bounds.width - screen_header_width;

            // Header row labels and separators
            for (idx, label) in header_labels.iter().enumerate() {
                let row_y = screen_table_top + screen_row_height * idx as f32;
                if idx > 0 {
                    frame.fill_rectangle(
                        Point::new(header_x, row_y),
                        Size::new(screen_header_width, 1.0),
                        border_color,
                    );
                }
                draw_text(
                    frame,
                    label,
                    Point::new(
                        header_x + screen_header_width / 2.0,
                        row_y + screen_row_height / 2.0,
                    ),
                    screen_text_size,
                    palette.background.weakest.text,
                );
            }

            // Header vertical border
            frame.fill_rectangle(
                Point::new(header_x, screen_table_top),
                Size::new(1.0, screen_row_height * rows_count),
                border_color,
            );

            let header_left_chart =
                (header_x - bounds.width / 2.0) / ctx.scaling - ctx.translation.x;
            let view_left = -ctx.translation.x - bounds.width / ctx.scaling;
            let view_right =
                (-ctx.translation.x + bounds.width / ctx.scaling).min(header_left_chart);

            let clip_region = Rectangle::new(
                Point::new(0.0, screen_table_top),
                Size::new(
                    bounds.width - screen_header_width,
                    screen_row_height * rows_count,
                ),
            );
            frame.with_clip(clip_region, |frame| {
                let center = Vector::new(bounds.width / 2.0, bounds.height / 2.0);
                frame.translate(center);
                frame.scale(ctx.scaling);
                frame.translate(Vector::new(
                    ctx.translation.x,
                    (-bounds.height / ctx.scaling) / 2.0,
                ));

                self.series
                    .for_each_in(self.visible_range.clone(), |x, row| {
                        let column_left = ctx.interval_to_x(x) - column_width / 2.0;

                        if column_left > view_right || column_left + column_width < view_left {
                            return;
                        }

                        frame.fill_rectangle(
                            Point::new(column_left, table_top),
                            Size::new(column_width, table_height),
                            palette.background.weakest.color.scale_alpha(0.22),
                        );

                        let delta_color = if row.delta.to_f64() >= 0.0 {
                            palette.success.base.color
                        } else {
                            palette.danger.base.color
                        };

                        let mut rows: Vec<(String, Color)> = Vec::with_capacity(5);
                        if self.settings.show_buy_sell {
                            rows.push((
                                abbr_large_numbers(row.buy.to_f64()),
                                palette.success.base.color,
                            ));
                            rows.push((
                                abbr_large_numbers(row.sell.to_f64()),
                                palette.danger.base.color,
                            ));
                        }
                        if self.settings.show_volume {
                            rows.push((
                                abbr_large_numbers(row.total.to_f64()),
                                palette.background.weakest.text,
                            ));
                        }
                        if self.settings.show_delta {
                            rows.push((abbr_large_numbers(row.delta.to_f64()), delta_color));
                        }
                        if self.settings.show_delta_pct {
                            rows.push((format!("{:+.1}%", row.delta_pct), delta_color));
                        }

                        for (idx, (label, color)) in rows.iter().enumerate() {
                            let row_y = table_top + row_height * idx as f32;
                            if idx > 0 {
                                frame.fill_rectangle(
                                    Point::new(column_left, row_y),
                                    Size::new(column_width, 1.0),
                                    border_color,
                                );
                            }
                            draw_text(
                                frame,
                                label,
                                Point::new(
                                    column_left + column_width / 2.0,
                                    row_y + row_height / 2.0,
                                ),
                                text_size,
                                *color,
                            );
                        }

                        frame.fill_rectangle(
                            Point::new(column_left, table_top),
                            Size::new(1.0, table_height),
                            border_color.scale_alpha(0.4),
                        );
                    });
            });
        });

        vec![geometry]
    }

    fn mouse_interaction(
        &self,
        _state: &Interaction,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        mouse::Interaction::default()
    }
}

fn draw_text(frame: &mut canvas::Frame, text: &str, position: Point, size: f32, color: Color) {
    frame.fill_text(canvas::Text {
        content: text.to_string(),
        position,
        size: iced::Pixels(size),
        color,
        align_x: Alignment::Center.into(),
        align_y: Alignment::Center.into(),
        font: style::AZERET_MONO,
        ..canvas::Text::default()
    });
}
