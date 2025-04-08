use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap, hash_map::Entry},
};

use data::UserTimezone;
use data::chart::{
    Basis, ChartLayout,
    heatmap::Config,
    indicators::{HeatmapIndicator, Indicator},
};
use iced::widget::canvas::{self, Event, Geometry, Path};
use iced::{
    Alignment, Color, Element, Point, Rectangle, Renderer, Size, Theme, Vector, mouse,
    theme::palette::Extended,
};

use exchange::{TickerInfo, Trade, adapter::MarketType, depth::Depth};

use super::scales::PriceInfoLabel;
use super::{Chart, ChartConstants, CommonChartData, Interaction, Message};
use super::{abbr_large_numbers, canvas_interaction, count_decimals, update_chart, view_chart};

use ordered_float::OrderedFloat;

impl Chart for HeatmapChart {
    fn get_common_data(&self) -> &CommonChartData {
        &self.chart
    }

    fn get_common_data_mut(&mut self) -> &mut CommonChartData {
        &mut self.chart
    }

    fn update_chart(&mut self, message: &Message) {
        update_chart(self, message);
        self.render_start();
    }

    fn canvas_interaction(
        &self,
        interaction: &mut Interaction,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        canvas_interaction(self, interaction, event, bounds, cursor)
    }

    fn view_indicator<I: Indicator>(&self, indicators: &[I]) -> Option<Element<Message>> {
        self.view_indicators(indicators)
    }

    fn get_visible_timerange(&self) -> (u64, u64) {
        let chart = self.get_common_data();
        let visible_region = chart.visible_region(chart.bounds.size());

        (
            chart.x_to_interval(visible_region.x),
            chart.x_to_interval(visible_region.x + visible_region.width),
        )
    }

    fn get_interval_keys(&self) -> Vec<u64> {
        vec![]
        //self.timeseries.iter().map(|(time, _, _)| *time).collect()
    }

    fn is_empty(&self) -> bool {
        self.timeseries.is_empty()
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
    start_time: u64,
    until_time: u64,
    qty: OrderedFloat<f32>,
    is_bid: bool,
}

impl OrderRun {
    fn get_visible_runs(&self, earliest: u64, latest: u64) -> Option<&OrderRun> {
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
    aggr_time: u64,
    tick_size: f32,
}

impl Orderbook {
    fn new(tick_size: f32, aggr_time: u64) -> Self {
        Self {
            price_levels: BTreeMap::new(),
            aggr_time,
            tick_size,
        }
    }

    fn insert_latest_depth(&mut self, depth: &Depth, time: u64) {
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
        time: u64,
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

    fn update_price_level(&mut self, time: u64, price: f32, qty: f32, is_bid: bool) {
        let price_level = self.price_levels.entry(OrderedFloat(price)).or_default();

        match price_level.last_mut() {
            Some(last_run) if last_run.qty == OrderedFloat(qty) && last_run.is_bid == is_bid => {
                last_run.until_time = time + self.aggr_time;
            }
            _ => {
                price_level.push(OrderRun {
                    start_time: time,
                    until_time: time + self.aggr_time,
                    qty: OrderedFloat(qty),
                    is_bid,
                });
            }
        }
    }

    fn iter_time_filtered(
        &self,
        earliest: u64,
        latest: u64,
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
        latest_timestamp: u64,
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
    SessionVolumeProfile(HashMap<OrderedFloat<f32>, (f32, f32)>),
}

pub struct HeatmapChart {
    chart: CommonChartData,
    timeseries: Vec<(u64, Box<[GroupedTrade]>, (f32, f32))>,
    indicators: HashMap<HeatmapIndicator, IndicatorData>,
    orderbook: Orderbook,
    visual_config: Config,
}

impl HeatmapChart {
    pub fn new(
        layout: ChartLayout,
        tick_size: f32,
        aggr_time: u64,
        enabled_indicators: &[HeatmapIndicator],
        ticker_info: Option<TickerInfo>,
        config: Option<Config>,
    ) -> Self {
        HeatmapChart {
            chart: CommonChartData {
                cell_width: Self::DEFAULT_CELL_WIDTH,
                cell_height: 4.0,
                tick_size,
                decimals: count_decimals(tick_size),
                crosshair: layout.crosshair,
                indicators_split: layout.indicators_split,
                ticker_info,
                basis: Basis::Time(aggr_time),
                ..Default::default()
            },
            indicators: {
                enabled_indicators
                    .iter()
                    .map(|&indicator| {
                        let data = match indicator {
                            HeatmapIndicator::Volume => IndicatorData::Volume,
                            HeatmapIndicator::SessionVolumeProfile => {
                                IndicatorData::SessionVolumeProfile(HashMap::new())
                            }
                        };
                        (indicator, data)
                    })
                    .collect()
            },
            orderbook: Orderbook::new(tick_size, aggr_time),
            timeseries: vec![],
            visual_config: config.unwrap_or_default(),
        }
    }

    pub fn insert_datapoint(&mut self, trades_buffer: &[Trade], depth_update: u64, depth: &Depth) {
        let chart = &mut self.chart;

        if self.timeseries.len() > 2400 {
            self.timeseries.drain(0..400);

            if let Some(oldest_time) = self.timeseries.first().map(|(time, _, _)| *time) {
                self.orderbook
                    .price_levels
                    .iter_mut()
                    .for_each(|(_, runs)| {
                        runs.retain(|run| run.start_time >= oldest_time);
                    });
            }
        }

        let aggregate_time: u64 = match chart.basis {
            Basis::Time(interval) => interval,
            Basis::Tick(_) => {
                // TODO: implement
                unimplemented!()
            }
        };

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
                    if probe.is_sell == trade.is_sell {
                        probe
                            .price
                            .partial_cmp(&grouped_price)
                            .unwrap_or(Ordering::Equal)
                    } else {
                        probe.is_sell.cmp(&trade.is_sell)
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

            if let Some(IndicatorData::SessionVolumeProfile(data)) = self
                .indicators
                .get_mut(&HeatmapIndicator::SessionVolumeProfile)
            {
                for trade in &grouped_trades {
                    if trade.is_sell {
                        data.entry(OrderedFloat(trade.price))
                            .or_insert_with(|| (0.0, 0.0))
                            .1 += trade.qty;
                    } else {
                        data.entry(OrderedFloat(trade.price))
                            .or_insert_with(|| (0.0, 0.0))
                            .0 += trade.qty;
                    }
                }
            }

            self.timeseries.push((
                rounded_depth_update,
                grouped_trades.into_boxed_slice(),
                (buy_volume, sell_volume),
            ));
        };

        self.orderbook
            .insert_latest_depth(depth, rounded_depth_update);

        chart.latest_x = rounded_depth_update;

        let mid_price = match (depth.asks.first_key_value(), depth.bids.last_key_value()) {
            (Some((ask_price, _)), Some((bid_price, _))) => {
                (ask_price.into_inner() + bid_price.into_inner()) / 2.0
            }
            _ => chart.base_price_y,
        };

        chart.last_price = Some(PriceInfoLabel::Neutral(mid_price));

        if chart.translation.x * chart.scaling > chart.bounds.width / 2.0 {
            chart.translation.x += chart.cell_width;
        } else {
            chart.base_price_y = (mid_price / (chart.tick_size)).round() * (chart.tick_size);
        }

        self.render_start();
    }

    pub fn get_visual_config(&self) -> Config {
        self.visual_config
    }

    pub fn set_visual_config(&mut self, visual_config: Config) {
        self.visual_config = visual_config;
    }

    pub fn get_chart_layout(&self) -> ChartLayout {
        self.chart.get_chart_layout()
    }

    pub fn change_tick_size(&mut self, new_tick_size: f32) {
        let chart_state = self.get_common_data_mut();

        let aggregate_time: u64 = match chart_state.basis {
            Basis::Time(interval) => interval,
            Basis::Tick(_) => {
                // TODO: implement
                unimplemented!()
            }
        };

        chart_state.cell_height = 4.0;
        chart_state.tick_size = new_tick_size;
        chart_state.decimals = count_decimals(new_tick_size);

        if let Some(IndicatorData::SessionVolumeProfile(data)) = self
            .indicators
            .get_mut(&HeatmapIndicator::SessionVolumeProfile)
        {
            data.clear();
        }

        self.timeseries.clear();

        self.orderbook = Orderbook::new(new_tick_size, aggregate_time);
    }

    pub fn toggle_indicator(&mut self, indicator: HeatmapIndicator) {
        match self.indicators.entry(indicator) {
            Entry::Occupied(entry) => {
                entry.remove();
            }
            Entry::Vacant(entry) => {
                let data = match indicator {
                    HeatmapIndicator::Volume => IndicatorData::Volume,
                    HeatmapIndicator::SessionVolumeProfile => {
                        IndicatorData::SessionVolumeProfile(HashMap::new())
                    }
                };
                entry.insert(data);
            }
        }
    }

    fn render_start(&mut self) {
        let chart_state = self.get_common_data_mut();

        if chart_state.autoscale {
            chart_state.translation = Vector::new(
                0.5 * (chart_state.bounds.width / chart_state.scaling)
                    - (90.0 / chart_state.scaling),
                0.0,
            );
        }

        chart_state.cache.clear_all();
    }

    fn visible_data_iter(
        &self,
        earliest: u64,
        latest: u64,
    ) -> impl Iterator<Item = &(u64, Box<[GroupedTrade]>, (f32, f32))> {
        self.timeseries
            .iter()
            .filter(move |(time, _, _)| *time >= earliest && *time <= latest)
    }

    fn calc_qty_scales(&self, earliest: u64, latest: u64, highest: f32, lowest: f32) -> QtyScale {
        let market_type = match self.chart.ticker_info {
            Some(ref ticker_info) => ticker_info.get_market_type(),
            None => return QtyScale::default(),
        };

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

                        let order_size = match market_type {
                            MarketType::InversePerps => visible_run.qty.0,
                            _ => **price * visible_run.qty.0,
                        };

                        if order_size > self.visual_config.order_size_filter {
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

    pub fn view_indicators<I: Indicator>(&self, _indis: &[I]) -> Option<Element<Message>> {
        None
    }

    pub fn update(&mut self, message: &Message) {
        self.update_chart(message)
    }

    pub fn view<'a, I: Indicator>(
        &'a self,
        indicators: &'a [I],
        timezone: UserTimezone,
    ) -> Element<'a, Message> {
        view_chart(self, indicators, timezone)
    }
}

impl canvas::Program<Message> for HeatmapChart {
    type State = Interaction;

    fn update(
        &self,
        interaction: &mut Interaction,
        event: &Event,
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
        let chart = self.get_common_data();

        if chart.bounds.width == 0.0 {
            return vec![];
        }

        let market_type = match self.chart.ticker_info {
            Some(ref ticker_info) => ticker_info.get_market_type(),
            None => return vec![],
        };

        let center = Vector::new(bounds.width / 2.0, bounds.height / 2.0);
        let bounds_size = bounds.size();

        let palette = theme.extended_palette();

        let volume_indicator = self.indicators.contains_key(&HeatmapIndicator::Volume);

        let heatmap = chart.cache.main.draw(renderer, bounds_size, |frame| {
            frame.translate(center);
            frame.scale(chart.scaling);
            frame.translate(chart.translation);

            let region = chart.visible_region(frame.size());

            let (earliest, latest) = chart.get_interval_range(region);
            let (highest, lowest) = chart.get_price_range(region);

            if latest < earliest {
                return;
            }

            let cell_height = chart.cell_height;
            let cell_height_scaled = cell_height * chart.scaling;

            let qty_scales = self.calc_qty_scales(earliest, latest, highest, lowest);

            let max_depth_qty = qty_scales.max_depth_qty;
            let (max_aggr_volume, max_trade_qty) =
                (qty_scales.max_aggr_volume, qty_scales.max_trade_qty);

            self.orderbook
                .iter_time_filtered(earliest, latest, highest, lowest)
                .for_each(|(price, runs)| {
                    let y_position = chart.price_to_y(price.0);

                    runs.iter()
                        .filter(|run| {
                            let order_size = match market_type {
                                MarketType::InversePerps => run.qty.0,
                                _ => **price * run.qty.0,
                            };
                            order_size > self.visual_config.order_size_filter
                        })
                        .for_each(|run| {
                            let start_x = chart.interval_to_x(run.start_time.max(earliest));
                            let end_x = chart.interval_to_x(run.until_time.min(latest)).min(0.0);

                            let width = end_x - start_x;

                            if width > 0.0 {
                                let color_alpha = (run.qty.0 / max_depth_qty).min(1.0);
                                let width_unscaled = width / chart.scaling;

                                if width_unscaled > 40.0
                                    && cell_height_scaled >= 10.0
                                    && color_alpha > 0.4
                                {
                                    frame.fill_text(canvas::Text {
                                        content: abbr_large_numbers(run.qty.0),
                                        position: Point::new(
                                            start_x + (cell_height / 2.0),
                                            y_position,
                                        ),
                                        size: iced::Pixels(cell_height),
                                        color: Color::WHITE,
                                        align_y: Alignment::Center.into(),
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

            if let Some((latest_timestamp, _, _)) = self.timeseries.last() {
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
                let text_content = abbr_large_numbers(max_qty);
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
                    let x_position = chart.interval_to_x(*time);

                    trades.iter().for_each(|trade| {
                        let y_position = chart.price_to_y(trade.price);

                        let trade_size = match market_type {
                            MarketType::InversePerps => trade.qty,
                            _ => trade.qty * trade.price,
                        };

                        if trade_size > self.visual_config.trade_size_filter {
                            let color = if trade.is_sell {
                                palette.danger.base.color
                            } else {
                                palette.success.base.color
                            };

                            let radius = {
                                if self.visual_config.dynamic_sized_trades {
                                    // normalize range
                                    let scale_factor =
                                        (self.visual_config.trade_size_scale as f32) / 100.0;
                                    1.0 + (trade.qty / max_trade_qty) * (28.0 - 1.0) * scale_factor
                                } else {
                                    cell_height / 2.0
                                }
                            };

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

                        if buy_bar_height > sell_bar_height {
                            frame.fill_rectangle(
                                Point::new(x_position, (region.y + region.height) - buy_bar_height),
                                Size::new(bar_width, buy_bar_height),
                                palette.success.base.color,
                            );

                            frame.fill_rectangle(
                                Point::new(
                                    x_position,
                                    (region.y + region.height) - sell_bar_height,
                                ),
                                Size::new(bar_width, sell_bar_height),
                                palette.danger.base.color,
                            );
                        } else {
                            frame.fill_rectangle(
                                Point::new(
                                    x_position,
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
                        }
                    }
                },
            );

            if let Some(IndicatorData::SessionVolumeProfile(data)) =
                self.indicators.get(&HeatmapIndicator::SessionVolumeProfile)
            {
                let max_vpsr = data
                    .iter()
                    .filter(|(price, _)| {
                        **price <= OrderedFloat(highest) && **price >= OrderedFloat(lowest)
                    })
                    .map(|(_, (buy_v, sell_v))| buy_v + sell_v)
                    .fold(0.0, f32::max);

                let max_bar_width = (bounds.width / chart.scaling) * 0.1;

                let min_segment_width = 2.0;
                let segments = ((max_bar_width / min_segment_width).floor() as usize)
                    .max(10)
                    .min(40);

                for i in 0..segments {
                    let segment_width = max_bar_width / segments as f32;
                    let segment_x = region.x + (i as f32 * segment_width);

                    let alpha = 0.95 - (0.85 * (i as f32 / (segments - 1) as f32).powf(2.0));

                    frame.fill_rectangle(
                        Point::new(segment_x, region.y),
                        Size::new(segment_width, region.height),
                        palette.background.weakest.color.scale_alpha(alpha),
                    );
                }

                let vpsr_height = cell_height_scaled * 0.8;

                data.iter()
                    .filter(|(price, _)| {
                        **price <= OrderedFloat(highest) && **price >= OrderedFloat(lowest)
                    })
                    .for_each(|(price, (buy_v, sell_v))| {
                        let y_position = chart.price_to_y(**price);

                        let buy_vpsr_width = (buy_v / max_vpsr) * max_bar_width;
                        let sell_vpsr_width = (sell_v / max_vpsr) * max_bar_width;

                        if buy_vpsr_width > sell_vpsr_width {
                            frame.fill_rectangle(
                                Point::new(region.x, y_position - (vpsr_height / 2.0)),
                                Size::new(buy_vpsr_width, vpsr_height),
                                palette.success.weak.color,
                            );

                            frame.fill_rectangle(
                                Point::new(region.x, y_position - (vpsr_height / 2.0)),
                                Size::new(sell_vpsr_width, vpsr_height),
                                palette.danger.weak.color,
                            );
                        } else {
                            frame.fill_rectangle(
                                Point::new(region.x, y_position - (vpsr_height / 2.0)),
                                Size::new(sell_vpsr_width, vpsr_height),
                                palette.danger.weak.color,
                            );

                            frame.fill_rectangle(
                                Point::new(region.x, y_position - (vpsr_height / 2.0)),
                                Size::new(buy_vpsr_width, vpsr_height),
                                palette.success.weak.color,
                            );
                        }
                    });

                if max_vpsr > 0.0 {
                    let text_size = 9.0 / chart.scaling;
                    let text_content = abbr_large_numbers(max_vpsr);

                    let text_position = Point::new(region.x + max_bar_width, region.y);

                    frame.fill_text(canvas::Text {
                        content: text_content,
                        position: text_position,
                        size: iced::Pixels(text_size),
                        color: palette.background.base.text,
                        ..canvas::Text::default()
                    });
                }
            }

            if volume_indicator && max_aggr_volume > 0.0 {
                let text_size = 9.0 / chart.scaling;
                let text_content = abbr_large_numbers(max_aggr_volume);
                let text_width = (text_content.len() as f32 * text_size) / 1.5;

                let text_position = Point::new(
                    (region.x + region.width) - text_width,
                    (region.y + region.height) - (bounds.height / chart.scaling) * 0.1 - text_size,
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

        if chart.crosshair & !self.timeseries.is_empty() {
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
                if cursor.is_over(bounds) && self.chart.crosshair {
                    return mouse::Interaction::Crosshair;
                }
                mouse::Interaction::default()
            }
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
