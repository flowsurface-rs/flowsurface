use std::{cmp::Ordering, collections::{BTreeMap, HashMap}};

use iced::{
    mouse, theme::palette::Extended, Alignment, Color, Element, 
    Point, Rectangle, Renderer, Size, Task, Theme, Vector
};
use iced::widget::canvas::{self, Event, Geometry, Path};

use crate::data_providers::TickerInfo;
use crate::{
    data_providers::{Depth, Trade},
    screen::UserTimezone,
};

use super::indicators::{HeatmapIndicator, Indicator};
use super::{Chart, ChartConstants, CommonChartData, Interaction, Message};
use super::{canvas_interaction, view_chart, update_chart, count_decimals, convert_to_qty_abbr};

use ordered_float::OrderedFloat;

impl Chart for HeatmapChart {
    fn get_common_data(&self) -> &CommonChartData {
        &self.chart
    }

    fn get_common_data_mut(&mut self) -> &mut CommonChartData {
        &mut self.chart
    }

    fn update_chart(&mut self, message: &Message) -> Task<Message> {
        let task = update_chart(self, message);
        self.render_start();

        task
    }

    fn canvas_interaction(
        &self,
        interaction: &mut Interaction,
        event: Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        canvas_interaction(self, interaction, event, bounds, cursor)
    }

    fn view_indicator<I: Indicator>(
        &self, 
        indicators: &[I], 
        ticker_info: Option<TickerInfo>
    ) -> Option<Element<Message>> {
        self.view_indicators(indicators, ticker_info)
    }

    fn get_visible_timerange(&self) -> (i64, i64) {
        let chart = self.get_common_data();

        let visible_region = chart.visible_region(chart.bounds.size());

        (
            chart.x_to_time(visible_region.x),
            chart.x_to_time(visible_region.x + visible_region.width),
        )
    }
}

impl ChartConstants for HeatmapChart {
    const MIN_SCALING: f32 = 0.6;
    const MAX_SCALING: f32 = 1.2;

    const MAX_CELL_WIDTH: f32 = 12.0;
    const MIN_CELL_WIDTH: f32 = 1.0;

    const MAX_CELL_HEIGHT: f32 = 10.0;
    const MIN_CELL_HEIGHT: f32 = 1.0;

    const DEFAULT_CELL_WIDTH: f32 = 3.0;
}

#[derive(Default, Debug, Clone, PartialEq)]
struct OrderRun {
    start_time: i64,
    until_time: i64,
    qty: OrderedFloat<f32>,
    is_bid: bool,
}

impl OrderRun {
    fn get_visible_runs(&self, earliest: i64, latest: i64) -> Option<&OrderRun> {
        if self.start_time <= latest && self.until_time >= earliest {
            Some(self)
        } else {
            None
        }
    }
}

#[derive(Default, Debug, Clone, PartialEq)]
struct Orderbook {
    price_levels: BTreeMap<OrderedFloat<f32>, Vec<OrderRun>>,
    aggr_time: i64,
    tick_size: f32,
}

impl Orderbook {
    fn new(tick_size: f32, aggr_time: i64) -> Self {
        Self {
            price_levels: BTreeMap::new(),
            aggr_time,
            tick_size,
        }
    }

    fn insert_latest_depth(&mut self, depth: &Depth, time: i64) {
        let tick_size = self.tick_size;

        self.process_side(&depth.bids, time, true, |price| {
            ((price * (1.0 / tick_size)).floor()) * tick_size
        });
        self.process_side(&depth.asks, time, false, |price| {
            ((price * (1.0 / tick_size)).ceil()) * tick_size
        });
    }

    fn process_side<F>(
        &mut self,
        side: &BTreeMap<OrderedFloat<f32>, f32>,
        time: i64,
        is_bid: bool,
        round_price: F,
    ) where
        F: Fn(f32) -> f32,
    {
        let mut current_price = None;
        let mut current_qty = 0.0;

        for (price, qty) in side {
            let rounded_price = round_price(price.into_inner());

            if Some(rounded_price) == current_price {
                current_qty += qty;
            } else {
                if let Some(price) = current_price {
                    self.update_price_level(time, price, current_qty, is_bid);
                }
                current_price = Some(rounded_price);
                current_qty = *qty;
            }
        }

        if let Some(price) = current_price {
            self.update_price_level(time, price, current_qty, is_bid);
        }
    }

    fn update_price_level(&mut self, time: i64, price: f32, qty: f32, is_bid: bool) {
        let price_level = self.price_levels.entry(OrderedFloat(price)).or_default();

        if let Some(last_run) = price_level.last_mut() {
            if last_run.qty != OrderedFloat(qty) || last_run.is_bid != is_bid {
                price_level.push(OrderRun {
                    start_time: time,
                    until_time: time + self.aggr_time,
                    qty: OrderedFloat(qty),
                    is_bid,
                });
            } else {
                last_run.until_time = time + self.aggr_time;
            }
        } else {
            price_level.push(OrderRun {
                start_time: time,
                until_time: time + self.aggr_time,
                qty: OrderedFloat(qty),
                is_bid,
            });
        }
    }

    fn iter_time_filtered(
        &self,
        earliest: i64,
        latest: i64,
        highest: f32,
        lowest: f32,
    ) -> impl Iterator<Item = (&OrderedFloat<f32>, &Vec<OrderRun>)> {
        self.price_levels.iter().filter(move |(price, runs)| {
            **price <= OrderedFloat(highest)
                && **price >= OrderedFloat(lowest)
                && runs
                    .iter()
                    .any(|run| run.until_time >= earliest && run.start_time <= latest)
        })
    }

    fn latest_order_runs(
        &self,
        highest: f32,
        lowest: f32,
        latest_timestamp: i64,
    ) -> impl Iterator<Item = (&OrderedFloat<f32>, &OrderRun)> {
        self.price_levels.iter().filter_map(move |(price, runs)| {
            if **price <= *OrderedFloat(highest) && **price >= *OrderedFloat(lowest) {
                runs.last()
                    .filter(|run| run.until_time >= latest_timestamp)
                    .map(|run| (price, run))
            } else {
                None
            }
        })
    }
}

#[derive(Default)]
struct QtyScale {
    max_trade_qty: f32,
    max_aggr_volume: f32,
    max_depth_qty: f32,
}

#[derive(Debug, Clone)]
pub struct GroupedTrade {
    pub is_sell: bool,
    pub price: f32,
    pub qty: f32,
}

#[allow(dead_code)]
enum IndicatorData {
    Volume,
}

pub struct HeatmapChart {
    chart: CommonChartData,
    data_points: Vec<(i64, Box<[GroupedTrade]>, (f32, f32))>,
    indicators: HashMap<HeatmapIndicator, IndicatorData>,
    orderbook: Orderbook,
    trade_size_filter: f32,
    order_size_filter: f32,
}

impl HeatmapChart {
    pub fn new(tick_size: f32, aggr_time: i64, timezone: UserTimezone, enabled_indicators: &[HeatmapIndicator]) -> Self {
        HeatmapChart {
            chart: CommonChartData {
                cell_width: Self::DEFAULT_CELL_WIDTH,
                cell_height: 4.0,
                timeframe: aggr_time as u64,
                tick_size,
                decimals: count_decimals(tick_size),
                timezone,
                ..Default::default()
            },
            indicators: {
                let mut indicators = HashMap::new();

                for indicator in enabled_indicators {
                    indicators.insert(
                        *indicator,
                        match indicator {
                            HeatmapIndicator::Volume => {
                                IndicatorData::Volume
                            },
                        }
                    );
                }

                indicators
            },
            orderbook: Orderbook::new(tick_size, aggr_time),
            data_points: Vec::new(),
            trade_size_filter: 0.0,
            order_size_filter: 0.0,
        }
    }

    pub fn insert_datapoint(&mut self, trades_buffer: &[Trade], depth_update: i64, depth: &Depth) {
        let chart = &mut self.chart;

        if self.data_points.len() > 2400 {
            self.data_points.drain(0..400);

            if let Some(oldest_time) = self.data_points.first().map(|(time, _, _)| *time) {
                self.orderbook
                    .price_levels
                    .iter_mut()
                    .for_each(|(_, runs)| {
                        runs.retain(|run| run.start_time >= oldest_time);
                    });
            }
        }

        let aggregate_time = chart.timeframe as i64;
        let rounded_depth_update = (depth_update / aggregate_time) * aggregate_time;

        {
            let (mut buy_volume, mut sell_volume) = (0.0, 0.0);

            let mut grouped_trades: Vec<GroupedTrade> = Vec::with_capacity(trades_buffer.len());

            for trade in trades_buffer {
                if trade.is_sell {
                    sell_volume += trade.qty;
                } else {
                    buy_volume += trade.qty;
                }

                let grouped_price = if trade.is_sell {
                    (trade.price * (1.0 / chart.tick_size)).floor() * chart.tick_size
                } else {
                    (trade.price * (1.0 / chart.tick_size)).ceil() * chart.tick_size
                };

                match grouped_trades.binary_search_by(|probe| {
                    if probe.is_sell != trade.is_sell {
                        probe.is_sell.cmp(&trade.is_sell)
                    } else {
                        probe
                            .price
                            .partial_cmp(&grouped_price)
                            .unwrap_or(Ordering::Equal)
                    }
                }) {
                    Ok(index) => grouped_trades[index].qty += trade.qty,
                    Err(index) => grouped_trades.insert(
                        index,
                        GroupedTrade {
                            is_sell: trade.is_sell,
                            price: grouped_price,
                            qty: trade.qty,
                        },
                    ),
                }
            }

            self.data_points.push((
                rounded_depth_update,
                grouped_trades.into_boxed_slice(),
                (buy_volume, sell_volume),
            ));
        };

        self.orderbook
            .insert_latest_depth(depth, rounded_depth_update);

        chart.latest_x = rounded_depth_update;

        if !(chart.translation.x * chart.scaling > chart.bounds.width / 2.0) {
            chart.base_price_y = {
                let best_ask_price = depth
                    .asks
                    .first_key_value()
                    .map_or(0.0, |(price, _)| price.into_inner());

                let best_bid_price = depth
                    .bids
                    .last_key_value()
                    .map_or(0.0, |(price, _)| price.into_inner());

                let mid_price = (best_ask_price + best_bid_price) / 2.0;

                (mid_price / (chart.tick_size)).round() * (chart.tick_size)
            };
        } else {
            chart.translation.x += chart.cell_width;
        }

        self.render_start();
    }

    pub fn set_size_filter(&mut self, size: f32, is_trade_filter: bool) {
        if is_trade_filter {
            self.trade_size_filter = size;
        } else {
            self.order_size_filter = size;
        }
    }

    pub fn get_size_filters(&self) -> (f32, f32) {
        (self.trade_size_filter, self.order_size_filter)
    }

    pub fn change_timezone(&mut self, timezone: UserTimezone) {
        let chart = self.get_common_data_mut();
        chart.timezone = timezone;
    }

    pub fn change_tick_size(&mut self, new_tick_size: f32) {
        let chart_state = self.get_common_data_mut();

        chart_state.cell_height = 4.0;
        chart_state.tick_size = new_tick_size;
        chart_state.decimals = count_decimals(new_tick_size);

        let aggr_time = self.chart.timeframe as i64;

        self.data_points.clear();
        self.orderbook = Orderbook::new(new_tick_size, aggr_time);
    }

    pub fn toggle_indicator(&mut self, indicator: HeatmapIndicator) {
        if self.indicators.contains_key(&indicator) {
            self.indicators.remove(&indicator);
        } else {
            match indicator {
                HeatmapIndicator::Volume => {
                    self.indicators.insert(
                        indicator,
                        IndicatorData::Volume,
                    );
                },
            }
        }
    }

    fn render_start(&mut self) {
        let chart_state = self.get_common_data_mut();

        if chart_state.autoscale {
            chart_state.translation = Vector::new(
                0.4 * chart_state.bounds.width / chart_state.scaling,
                0.0,
            );
        }

        chart_state.cache.clear_all();
    }

    fn visible_data_iter(
        &self,
        earliest: i64,
        latest: i64,
    ) -> impl Iterator<Item = &(i64, Box<[GroupedTrade]>, (f32, f32))> {
        self.data_points
            .iter()
            .filter(move |(time, _, _)| *time >= earliest && *time <= latest)
    }

    fn calc_qty_scales(&self, earliest: i64, latest: i64, highest: f32, lowest: f32) -> QtyScale {
        let (mut max_aggr_volume, mut max_trade_qty) = (0.0f32, 0.0f32);
        let mut max_depth_qty = 0.0f32;

        self.visible_data_iter(earliest, latest)
            .for_each(|(_, trades, _)| {
                let (mut buy_volume, mut sell_volume) = (0.0, 0.0);

                trades.iter().for_each(|trade| {
                    max_trade_qty = max_trade_qty.max(trade.qty);

                    if trade.is_sell {
                        sell_volume += trade.qty;
                    } else {
                        buy_volume += trade.qty;
                    }
                });

                max_aggr_volume = max_aggr_volume.max(buy_volume).max(sell_volume);
            });

        self.orderbook
            .iter_time_filtered(earliest, latest, highest, lowest)
            .for_each(|(price, runs)| {
                runs.iter()
                    .filter_map(|run| {
                        let visible_run = run.get_visible_runs(earliest, latest)?;
                        if **price * visible_run.qty.0 > self.order_size_filter {
                            Some(visible_run)
                        } else {
                            None
                        }
                    })
                    .for_each(|run| {
                        max_depth_qty = max_depth_qty.max(run.qty.0);
                    });
            });

        QtyScale {
            max_trade_qty,
            max_aggr_volume,
            max_depth_qty,
        }
    }

    pub fn view_indicators<I: Indicator>(&self, _indis: &[I], _ticker_info: Option<TickerInfo>) -> Option<Element<Message>> {
        None
    }

    pub fn update(&mut self, message: &Message) -> Task<Message> {
        self.update_chart(message)
    }

    pub fn view<'a, I: Indicator>(
        &'a self, 
        indicators: &'a [I], 
        ticker_info: Option<TickerInfo>
    ) -> Element<Message> {
        view_chart(self, indicators, ticker_info)
    }
}

impl canvas::Program<Message> for HeatmapChart {
    type State = Interaction;

    fn update(
        &self,
        interaction: &mut Interaction,
        event: Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        self.canvas_interaction(interaction, event, bounds, cursor)
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

        let chart = self.get_common_data();

        let center = Vector::new(bounds.width / 2.0, bounds.height / 2.0);
        let bounds_size = bounds.size();

        let palette = theme.extended_palette();
        
        let volume_indicator = self.indicators.contains_key(&HeatmapIndicator::Volume);

        let heatmap = chart.cache.main.draw(renderer, bounds_size, |frame| {
            frame.with_save(|frame| {
                frame.translate(center);
                frame.scale(chart.scaling);
                frame.translate(chart.translation);

                let region = chart.visible_region(frame.size());

                let cell_height = chart.cell_height;
                let cell_height_scaled = cell_height * chart.scaling;

                let (earliest, latest) = (
                    chart.x_to_time(region.x),
                    chart.x_to_time(region.x + region.width),
                );
                let (highest, lowest) = (
                    chart.y_to_price(region.y),
                    chart.y_to_price(region.y + region.height),
                );

                let qty_scales = self.calc_qty_scales(earliest, latest, highest, lowest);

                let max_depth_qty = qty_scales.max_depth_qty;
                let (max_aggr_volume, max_trade_qty) =
                    (qty_scales.max_aggr_volume, qty_scales.max_trade_qty);

                self.orderbook
                    .iter_time_filtered(earliest, latest, highest, lowest)
                    .for_each(|(price, runs)| {
                        let y_position = chart.price_to_y(price.0);

                        runs.iter()
                            .filter(|run| **price * run.qty.0 > self.order_size_filter)
                            .for_each(|run| {
                                let start_x = chart.time_to_x(run.start_time.max(earliest));
                                let end_x = chart.time_to_x(run.until_time.min(latest)).min(0.0);

                                let width = end_x - start_x;

                                if width > 0.0 {
                                    let color_alpha = (run.qty.0 / max_depth_qty).min(1.0);
                                    let width_unscaled = width / chart.scaling;

                                    if width_unscaled > 40.0
                                        && cell_height_scaled >= 10.0
                                        && color_alpha > 0.4
                                    {
                                        frame.fill_text(canvas::Text {
                                            content: convert_to_qty_abbr(run.qty.0),
                                            position: Point::new(
                                                start_x + (cell_height / 2.0),
                                                y_position,
                                            ),
                                            size: iced::Pixels(cell_height),
                                            color: Color::WHITE,
                                            vertical_alignment: Alignment::Center.into(),
                                            ..canvas::Text::default()
                                        });

                                        frame.fill_rectangle(
                                            Point::new(start_x, y_position - (cell_height / 2.0)),
                                            Size::new(width, cell_height),
                                            get_depth_color(palette, run.is_bid, color_alpha),
                                        );

                                        frame.fill_rectangle(
                                            Point::new(start_x, y_position - (cell_height / 2.0)),
                                            Size::new(1.0, cell_height),
                                            Color::WHITE,
                                        );
                                    } else {
                                        frame.fill_rectangle(
                                            Point::new(start_x, y_position - (cell_height / 2.0)),
                                            Size::new(width, cell_height),
                                            get_depth_color(palette, run.is_bid, color_alpha),
                                        );
                                    }
                                }
                            });
                    });

                if let Some((latest_timestamp, _, _)) = self.data_points.last() {
                    let max_qty = self
                        .orderbook
                        .latest_order_runs(highest, lowest, *latest_timestamp)
                        .map(|(_, order_run)| order_run.qty.0)
                        .fold(f32::MIN, f32::max)
                        .ceil()
                        * 5.0
                        / 5.0;
                    if max_qty.is_infinite() {
                        return;
                    }

                    self.orderbook
                        .latest_order_runs(highest, lowest, *latest_timestamp)
                        .for_each(|(price, run)| {
                            let y_position = chart.price_to_y(price.0);
                            let bar_width = (run.qty.0 / max_qty) * 50.0;

                            frame.fill_rectangle(
                                Point::new(0.0, y_position - (cell_height / 2.0)),
                                Size::new(bar_width, cell_height),
                                get_depth_color(palette, run.is_bid, 0.5),
                            );
                        });

                    // max bid/ask quantity text
                    let text_size = 9.0 / chart.scaling;
                    let text_content = convert_to_qty_abbr(max_qty);
                    let text_position = Point::new(50.0, region.y);

                    frame.fill_text(canvas::Text {
                        content: text_content,
                        position: text_position,
                        size: iced::Pixels(text_size),
                        color: palette.background.base.text,
                        ..canvas::Text::default()
                    });
                };

                self.visible_data_iter(earliest, latest).for_each(
                    |(time, trades, (buy_volume, sell_volume))| {
                        let x_position = chart.time_to_x(*time);

                        trades.iter().for_each(|trade| {
                            let y_position = chart.price_to_y(trade.price);

                            if trade.qty * trade.price > self.trade_size_filter {
                                let color = if trade.is_sell {
                                    palette.danger.base.color
                                } else {
                                    palette.success.base.color
                                };

                                let radius = 1.0 + (trade.qty / max_trade_qty) * (28.0 - 1.0);

                                frame.fill(
                                    &Path::circle(Point::new(x_position, y_position), radius),
                                    color,
                                );
                            }
                        });

                        if volume_indicator {
                            let bar_width = (chart.cell_width / 2.0) * 0.9;

                            let buy_bar_height =
                                (buy_volume / max_aggr_volume) * (bounds.height / chart.scaling) * 0.1;
                            let sell_bar_height =
                                (sell_volume / max_aggr_volume) * (bounds.height / chart.scaling) * 0.1;

                            frame.fill_rectangle(
                                Point::new(x_position, (region.y + region.height) - buy_bar_height),
                                Size::new(bar_width, buy_bar_height),
                                palette.success.base.color,
                            );

                            frame.fill_rectangle(
                                Point::new(
                                    x_position - bar_width,
                                    (region.y + region.height) - sell_bar_height,
                                ),
                                Size::new(bar_width, sell_bar_height),
                                palette.danger.base.color,
                            );
                        }
                    },
                );

                if volume_indicator && max_aggr_volume > 0.0 {
                    let text_size = 9.0 / chart.scaling;
                    let text_content = convert_to_qty_abbr(max_aggr_volume);
                    let text_width = (text_content.len() as f32 * text_size) / 1.5;

                    let text_position = Point::new(
                        (region.x + region.width) - text_width,
                        (region.y + region.height)
                            - (bounds.height / chart.scaling) * 0.1
                            - text_size,
                    );

                    frame.fill_text(canvas::Text {
                        content: text_content,
                        position: text_position,
                        size: iced::Pixels(text_size),
                        color: palette.background.base.text,
                        ..canvas::Text::default()
                    });
                }
            });
        });

        if chart.crosshair & !self.data_points.is_empty() {
            let crosshair = chart.cache.crosshair.draw(renderer, bounds_size, |frame| {
                if let Some(cursor_position) = cursor.position_in(bounds) {
                    chart.draw_crosshair(frame, theme, bounds_size, cursor_position);
                }
            });

            vec![heatmap, crosshair]
        } else {
            vec![heatmap]
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
            Interaction::None => {
                if cursor.is_over(Rectangle {
                    x: bounds.x,
                    y: bounds.y,
                    width: bounds.width,
                    height: bounds.height - 8.0,
                }) {
                    if self.chart.crosshair {
                        return mouse::Interaction::Crosshair;
                    }
                } else if cursor.is_over(Rectangle {
                    x: bounds.x,
                    y: bounds.y + bounds.height - 8.0,
                    width: bounds.width,
                    height: 8.0,
                }) {
                    return mouse::Interaction::ResizingVertically;
                }

                mouse::Interaction::default()
            }
            _ => mouse::Interaction::default(),
        }
    }
}

fn get_depth_color(palette: &Extended, is_bid: bool, alpha: f32) -> Color {
    if is_bid {
        palette.success.strong.color.scale_alpha(alpha)
    } else {
        palette.danger.strong.color.scale_alpha(alpha)
    }
}
