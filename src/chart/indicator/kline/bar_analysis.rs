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

    fn on_insert_klines(&mut self, _klines: &[Kline], source: &PlotData<KlineDataPoint>) {
        self.rebuild(source);
    }

    fn on_insert_trades(
        &mut self,
        _trades: &[Trade],
        _old_dp_len: usize,
        source: &PlotData<KlineDataPoint>,
    ) {
        self.rebuild(source);
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

            let center = Vector::new(bounds.width / 2.0, bounds.height / 2.0);
            frame.translate(center);
            frame.scale(ctx.scaling);
            frame.translate(Vector::new(
                ctx.translation.x,
                (-bounds.height / ctx.scaling) / 2.0,
            ));

            let pane_height = frame.height() / ctx.scaling;
            let table_top = 3.0;
            let table_height = (pane_height - 6.0).max(24.0);
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
            let text_size = (row_height * 0.42).clamp(5.0, TEXT_SIZE * 0.75);
            let border_color = palette.background.weakest.text.scale_alpha(0.25);
            let view_left = -ctx.translation.x - bounds.width / ctx.scaling;
            let view_right = -ctx.translation.x + bounds.width / ctx.scaling;

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

                    let mut rows = Vec::with_capacity(5);
                    if self.settings.show_buy_sell {
                        rows.push((
                            format!("Ask {}", abbr_large_numbers(row.sell.to_f64())),
                            palette.danger.base.color,
                        ));
                        rows.push((
                            format!("Bid {}", abbr_large_numbers(row.buy.to_f64())),
                            palette.success.base.color,
                        ));
                    }
                    if self.settings.show_volume {
                        rows.push((
                            format!("Vol {}", abbr_large_numbers(row.total.to_f64())),
                            palette.background.weakest.text,
                        ));
                    }
                    if self.settings.show_delta {
                        rows.push((
                            format!("Δ {}", abbr_large_numbers(row.delta.to_f64())),
                            delta_color,
                        ));
                    }
                    if self.settings.show_delta_pct {
                        rows.push((format!("Δ% {:+.1}%", row.delta_pct), delta_color));
                    }

                    for (idx, (label, color)) in rows.iter().enumerate() {
                        let row_y = table_top + row_height * idx as f32;
                        frame.fill_rectangle(
                            Point::new(column_left, row_y),
                            Size::new(column_width, 1.0),
                            border_color,
                        );
                        draw_text(
                            frame,
                            label,
                            Point::new(column_left + column_width / 2.0, row_y + row_height / 2.0),
                            text_size,
                            *color,
                        );
                    }

                    frame.fill_rectangle(
                        Point::new(column_left, table_top + table_height - 1.0),
                        Size::new(column_width, 1.0),
                        border_color,
                    );
                    frame.fill_rectangle(
                        Point::new(column_left, table_top),
                        Size::new(1.0, table_height),
                        border_color,
                    );
                    frame.fill_rectangle(
                        Point::new(column_left + column_width - 1.0, table_top),
                        Size::new(1.0, table_height),
                        border_color,
                    );
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
