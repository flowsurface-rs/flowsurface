use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};

use iced::widget::canvas::{LineDash, Path, Stroke};
use iced::widget::container;
use iced::{mouse, Alignment, Element, Length, Point, Rectangle, Renderer, Size, Task, Theme, Vector};
use iced::widget::{column, canvas::{self, Event, Geometry}};
use ordered_float::OrderedFloat;

use crate::data_providers::{MarketType, TickerInfo};
use crate::layout::SerializableChartData;
use crate::data_providers::{
    fetcher::{FetchRange, RequestHandler},
    Kline, Timeframe, Trade, OpenInterest as OIData,
};
use crate::screen::UserTimezone;

use super::scales::PriceInfoLabel;
use super::indicators::{self, FootprintIndicator, Indicator};
use super::{Caches, Chart, ChartConstants, CommonChartData, Interaction, Message};
use super::{canvas_interaction, view_chart, update_chart, count_decimals, request_fetch, abbr_large_numbers, round_to_tick};

impl Chart for FootprintChart {
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
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        canvas_interaction(self, interaction, event, bounds, cursor)
    }

    fn view_indicator<I: Indicator>(
        &self, 
        indicators: &[I],
    ) -> Option<Element<Message>> {
        self.view_indicators(indicators)
    }

    fn get_visible_timerange(&self) -> (i64, i64) {
        let chart = self.get_common_data();
        let region = chart.visible_region(chart.bounds.size());

        let (earliest, latest) = (
            chart.x_to_time(region.x) - (chart.timeframe / 2) as i64,
            chart.x_to_time(region.x + region.width) + (chart.timeframe / 2) as i64,
        );

        (earliest, latest)
    }
}

#[allow(dead_code)]
enum IndicatorData {
    Volume(Caches, BTreeMap<i64, (f32, f32)>),
    OpenInterest(Caches, BTreeMap<i64, f32>),
}

impl IndicatorData {
    fn clear_cache(&mut self) {
        match self {
            IndicatorData::Volume(caches, _) 
            | IndicatorData::OpenInterest(caches, _) => {
                caches.clear_all();
            }
        }
    }
}

impl ChartConstants for FootprintChart {
    const MIN_SCALING: f32 = 0.4;
    const MAX_SCALING: f32 = 1.2;

    const MAX_CELL_WIDTH: f32 = 360.0;
    const MIN_CELL_WIDTH: f32 = 80.0;

    const MAX_CELL_HEIGHT: f32 = 90.0;
    const MIN_CELL_HEIGHT: f32 = 1.0;

    const DEFAULT_CELL_WIDTH: f32 = 80.0;
}

pub struct FootprintChart {
    chart: CommonChartData,
    data_points: BTreeMap<i64, (HashMap<OrderedFloat<f32>, (f32, f32)>, Kline)>,
    raw_trades: Vec<Trade>,
    indicators: HashMap<FootprintIndicator, IndicatorData>,
    fetching_trades: bool,
    request_handler: RequestHandler,
}

impl FootprintChart {
    pub fn new(
        layout: SerializableChartData,
        timeframe: Timeframe,
        tick_size: f32,
        klines_raw: Vec<Kline>,
        raw_trades: Vec<Trade>,
        enabled_indicators: &[FootprintIndicator],
        ticker_info: Option<TickerInfo>,
    ) -> Self {
        let mut loading_chart = true;
        let mut data_points = BTreeMap::new();
        let mut volume_data = BTreeMap::new();

        let base_price_y = klines_raw.last().unwrap_or(&Kline::default()).close;

        for kline in klines_raw {
            data_points
                .entry(kline.time as i64)
                .or_insert((HashMap::new(), kline));
            volume_data.insert(kline.time as i64, (kline.volume.0, kline.volume.1));
        }

        let mut latest_x = 0;
        let (mut scale_high, mut scale_low) = (0.0f32, f32::MAX);
        data_points
            .iter()
            .rev()
            .take(12)
            .for_each(|(time, (_, kline))| {
                scale_high = scale_high.max(kline.high);
                scale_low = scale_low.min(kline.low);

                latest_x = latest_x.max(*time);
            });

        let aggregate_time = timeframe.to_milliseconds() as i64;

        for trade in &raw_trades {
            let rounded_time = (trade.time / aggregate_time) * aggregate_time;
            let price_level = OrderedFloat(round_to_tick(trade.price, tick_size));

            let entry = data_points
                .entry(rounded_time)
                .or_insert((HashMap::new(), Kline::default()));

            if let Some((buy_qty, sell_qty)) = entry.0.get_mut(&price_level) {
                if trade.is_sell {
                    *sell_qty += trade.qty;
                } else {
                    *buy_qty += trade.qty;
                }
            } else if trade.is_sell {
                entry.0.insert(price_level, (0.0, trade.qty));
            } else {
                entry.0.insert(price_level, (trade.qty, 0.0));
            }
        }

        if !data_points.is_empty() {
            loading_chart = false;
        }

        let y_ticks = (scale_high - scale_low) / tick_size;

        FootprintChart {
            chart: CommonChartData {
                cell_width: Self::DEFAULT_CELL_WIDTH,
                cell_height: 800.0 / y_ticks,
                base_range: 400.0 / y_ticks,
                base_price_y,
                latest_x,
                timeframe: timeframe.to_milliseconds(),
                tick_size,
                decimals: count_decimals(tick_size),
                crosshair: layout.crosshair,
                indicators_split: layout.indicators_split,
                loading_chart,
                ticker_info,
                ..Default::default()
            },
            data_points,
            raw_trades,
            indicators: {
                let mut indicators = HashMap::new();

                for indicator in enabled_indicators {
                    indicators.insert(
                        *indicator,
                        match indicator {
                            FootprintIndicator::Volume => {
                                IndicatorData::Volume(Caches::default(), volume_data.clone())
                            },
                            FootprintIndicator::OpenInterest => {
                                IndicatorData::OpenInterest(Caches::default(), BTreeMap::new())
                            }
                        }
                    );
                }

                indicators
            },
            fetching_trades: false,
            request_handler: RequestHandler::new(),
        }
    }

    pub fn set_loading_state(&mut self, loading: bool) {
        self.chart.loading_chart = loading;
    }

    pub fn update_latest_kline(&mut self, kline: &Kline) -> Task<Message> {
        if let Some((_, kline_value)) = self.data_points.get_mut(&(kline.time as i64)) {
            kline_value.open = kline.open;
            kline_value.high = kline.high;
            kline_value.low = kline.low;
            kline_value.close = kline.close;
            kline_value.volume = kline.volume;
        } else {
            self.data_points
                .insert(kline.time as i64, (HashMap::new(), *kline));
        }

        if let Some(IndicatorData::Volume(_, data)) = 
            self.indicators.get_mut(&FootprintIndicator::Volume) {
                data.insert(kline.time as i64, (kline.volume.0, kline.volume.1));
            };

        let chart = self.get_common_data_mut();

        if (kline.time as i64) > chart.latest_x {
            chart.latest_x = kline.time as i64;
        }

        chart.last_price = {
            if kline.close > kline.open {
                Some(PriceInfoLabel::Up(kline.close))
            } else {
                Some(PriceInfoLabel::Down(kline.close))
            }
        };

        self.render_start();
        self.get_missing_data_task().unwrap_or(Task::none())
    }

    fn get_missing_data_task(&mut self) -> Option<Task<Message>> {
        let (visible_earliest, visible_latest) = self.get_visible_timerange();
        let (kline_earliest, kline_latest) = self.get_kline_timerange();
        let earliest = visible_earliest - (visible_latest - visible_earliest);
        
        // priority 1, basic kline data fetch
        if visible_earliest < kline_earliest {
            if let Some(task) = request_fetch(
                &mut self.request_handler, 
                FetchRange::Kline(earliest, kline_earliest)
            ) {
                return Some(task);
            }
        }

        if !self.fetching_trades {
            let (kline_earliest, _) = self.get_trades_timerange(kline_latest);

            if visible_earliest < kline_earliest {
                let trade_earliest = self.raw_trades.iter()
                    .filter(|trade| trade.time >= kline_earliest)
                    .map(|trade| trade.time)
                    .min();
            
                if let Some(earliest) = trade_earliest {
                    if let Some(fetch_task) = request_fetch(
                        &mut self.request_handler, FetchRange::Trades(visible_earliest, earliest)
                    ) {
                        self.fetching_trades = true;
                        return Some(fetch_task);
                    }
                }
            }
        }

        // priority 2, Open Interest data
        for data in self.indicators.values() {
            if let IndicatorData::OpenInterest(_, _) = data {
                if self.chart.timeframe >= Timeframe::M5.to_milliseconds() 
                    && self.chart.ticker_info.is_some_and(|info| info.get_market_type() == MarketType::LinearPerps)
                {
                    let (oi_earliest, oi_latest) = self.get_oi_timerange(kline_latest);
    
                    if visible_earliest < oi_earliest {
                        if let Some(task) = request_fetch(
                            &mut self.request_handler, 
                            FetchRange::OpenInterest(earliest, oi_earliest)
                        ) {
                            return Some(task);
                        }
                    } 
                    
                    if oi_latest < kline_latest {
                        if let Some(task) = request_fetch(
                            &mut self.request_handler,
                            FetchRange::OpenInterest(oi_latest, kline_latest)
                        ) {
                            return Some(task);
                        }
                    }
                }
            }
        }
    
        // priority 3, missing klines & integrity check
        if let Some(missing_keys) = self.get_common_data()
            .check_kline_integrity(kline_earliest, kline_latest, &self.data_points) 
        {
            let latest = missing_keys.iter()
                .max().unwrap_or(&visible_latest) + self.chart.timeframe as i64;
            let earliest = missing_keys.iter()
                .min().unwrap_or(&visible_earliest) - self.chart.timeframe as i64;
    
            if let Some(task) = request_fetch(
                &mut self.request_handler, 
                FetchRange::Kline(earliest, latest)
            ) {
                return Some(task);
            }
        }

        None
    }

    pub fn reset_request_handler(&mut self) {
        self.request_handler = RequestHandler::new();
        self.fetching_trades = false;
    }

    pub fn get_raw_trades(&self) -> Vec<Trade> {
        self.raw_trades.clone()
    }

    pub fn clear_trades(&mut self, clear_raw: bool) {
        self.data_points.iter_mut().for_each(|(_, (trades, _))| {
            trades.clear();
        });

        if clear_raw {
            self.raw_trades.clear();
        } else {
            let aggregate_time = self.chart.timeframe as i64;
            let tick_size = self.chart.tick_size;

            for trade in &self.raw_trades {
                let rounded_time = (trade.time / aggregate_time) * aggregate_time;
                let price_level = OrderedFloat(round_to_tick(trade.price, tick_size));
    
                let entry = self.data_points
                    .entry(rounded_time)
                    .or_insert((HashMap::new(), Kline::default()));
    
                if let Some((buy_qty, sell_qty)) = entry.0.get_mut(&price_level) {
                    if trade.is_sell {
                        *sell_qty += trade.qty;
                    } else {
                        *buy_qty += trade.qty;
                    }
                } else if trade.is_sell {
                    entry.0.insert(price_level, (0.0, trade.qty));
                } else {
                    entry.0.insert(price_level, (trade.qty, 0.0));
                }
            }
        }
    }

    pub fn get_tick_size(&self) -> f32 {
        self.chart.tick_size
    }

    pub fn get_chart_layout(&self) -> SerializableChartData {
        self.chart.get_chart_layout()
    }

    pub fn change_tick_size(&mut self, new_tick_size: f32) {
        let chart = self.get_common_data_mut();
        let old_tick_size = chart.tick_size;

        chart.base_range *= new_tick_size / old_tick_size;
        chart.cell_height *= new_tick_size / old_tick_size;

        chart.tick_size = new_tick_size;

        self.clear_trades(false);
    }

    fn get_kline_timerange(&self) -> (i64, i64) {
        let mut from_time = i64::MAX;
        let mut to_time = i64::MIN;

        self.data_points.iter().for_each(|(time, _)| {
            from_time = from_time.min(*time);
            to_time = to_time.max(*time);
        });

        (from_time, to_time)
    }

    fn get_oi_timerange(&self, latest_kline: i64) -> (i64, i64) {
        let mut from_time = latest_kline;
        let mut to_time = i64::MIN;

        if let Some(IndicatorData::OpenInterest(_, data)) = 
            self.indicators.get(&FootprintIndicator::OpenInterest) {
                data.iter().for_each(|(time, _)| {
                    from_time = from_time.min(*time);
                    to_time = to_time.max(*time);
                });
            };

        (from_time, to_time)
    }

    fn get_trades_timerange(&self, latest_kline: i64) -> (i64, i64) {
        let mut from_time = latest_kline;
        let mut to_time = 0;

        self.data_points
            .iter()
            .filter(|(_, (trades, _))| !trades.is_empty())
            .for_each(|(time, _)| {
                from_time = from_time.min(*time);
                to_time = to_time.max(*time);
            });

        (from_time, to_time)
    }

    pub fn insert_datapoint(&mut self, trades_buffer: &[Trade], depth_update: i64) {
        let (tick_size, aggregate_time) = {
            let chart = self.get_common_data();
            (chart.tick_size, chart.timeframe as i64)
        };

        let rounded_depth_update = (depth_update / aggregate_time) * aggregate_time;

        self.data_points
            .entry(rounded_depth_update)
            .or_insert((HashMap::new(), Kline::default()));

        for trade in trades_buffer {
            let price_level = OrderedFloat(round_to_tick(trade.price, tick_size));
            if let Some((trades, _)) = self.data_points.get_mut(&rounded_depth_update) {
                if let Some((buy_qty, sell_qty)) = trades.get_mut(&price_level) {
                    if trade.is_sell {
                        *sell_qty += trade.qty;
                    } else {
                        *buy_qty += trade.qty;
                    }
                } else if trade.is_sell {
                    trades.insert(price_level, (0.0, trade.qty));
                } else {
                    trades.insert(price_level, (trade.qty, 0.0));
                }
            }
        }

        self.raw_trades.extend_from_slice(trades_buffer);
    }

    pub fn insert_trades(&mut self, raw_trades: Vec<Trade>, is_batches_done: bool) {
        let aggregate_time = self.chart.timeframe as i64;
        let tick_size = self.chart.tick_size;

        for trade in &raw_trades {
            let rounded_time = (trade.time / aggregate_time) * aggregate_time;
            let price_level = OrderedFloat(round_to_tick(trade.price, tick_size));

            let entry = self.data_points
                .entry(rounded_time)
                .or_insert((HashMap::new(), Kline::default()));

            if let Some((buy_qty, sell_qty)) = entry.0.get_mut(&price_level) {
                if trade.is_sell {
                    *sell_qty += trade.qty;
                } else {
                    *buy_qty += trade.qty;
                }
            } else if trade.is_sell {
                entry.0.insert(price_level, (0.0, trade.qty));
            } else {
                entry.0.insert(price_level, (trade.qty, 0.0));
            }
        }

        self.raw_trades.extend(raw_trades);

        if is_batches_done {
            self.fetching_trades = false;
        }
    }

    pub fn insert_new_klines(&mut self, req_id: uuid::Uuid, klines_raw: &Vec<Kline>) {
        let mut volume_data = BTreeMap::new();

        for kline in klines_raw {
            volume_data.insert(kline.time as i64, (kline.volume.0, kline.volume.1));
            self.data_points
                .entry(kline.time as i64)
                .or_insert((HashMap::new(), *kline));
        }

        if let Some(IndicatorData::Volume(_, data)) = 
            self.indicators.get_mut(&FootprintIndicator::Volume) {
                data.extend(volume_data.clone());
            };

        if !klines_raw.is_empty() {
            self.request_handler.mark_completed(req_id);
        } else {
            self.request_handler
                .mark_failed(req_id, "No data received".to_string());
        }

        self.chart.loading_chart = false;
        self.render_start();
    }

    pub fn insert_open_interest(&mut self, req_id: Option<uuid::Uuid>, oi_data: Vec<OIData>) {
        if let Some(req_id) = req_id {
            if !oi_data.is_empty() {
                self.request_handler.mark_completed(req_id);
            } else {
                self.request_handler
                    .mark_failed(req_id, "No data received".to_string());
            }
        }

        if let Some(IndicatorData::OpenInterest(_, data)) = 
            self.indicators.get_mut(&FootprintIndicator::OpenInterest) {
                data.extend(oi_data
                    .iter().map(|oi| (oi.time, oi.value))
                );
            };
    }

    fn calc_qty_scales(
        &self,
        earliest: i64,
        latest: i64,
        highest: f32,
        lowest: f32,
        tick_size: f32,
    ) -> (f32, f32) {
        let mut max_trade_qty: f32 = 0.0;
        let mut max_volume: f32 = 0.0;

        let rounded_highest = OrderedFloat(round_to_tick(highest + tick_size, tick_size));
        let rounded_lowest = OrderedFloat(round_to_tick(lowest - tick_size, tick_size));

        self.data_points
            .range(earliest..=latest)
            .for_each(|(_, (trades, kline))| {
                trades
                    .iter()
                    .filter(|(price, _)| **price > rounded_lowest && **price < rounded_highest)
                    .for_each(|(_, (buy_qty, sell_qty))| {
                        max_trade_qty = max_trade_qty.max(buy_qty.max(*sell_qty));
                    });

                max_volume = max_volume.max(kline.volume.0.max(kline.volume.1));
            });

        (max_trade_qty, max_volume)
    }

    fn render_start(&mut self) {
        let chart_state = &mut self.chart;

        if chart_state.loading_chart {
            return;
        }

        if chart_state.autoscale {
            chart_state.translation = Vector::new(
                0.5 * (chart_state.bounds.width / chart_state.scaling) - (chart_state.cell_width / chart_state.scaling),
                if let Some((_, (_, kline))) = self.data_points.last_key_value() {
                    let y_low = chart_state.price_to_y(kline.low);
                    let y_high = chart_state.price_to_y(kline.high);

                    -(y_low + y_high) / 2.0
                } else {
                    0.0
                },
            );
        }

        chart_state.cache.clear_all();

        self.indicators.iter_mut().for_each(|(_, data)| {
            data.clear_cache();
        });
    }

    pub fn toggle_indicator(&mut self, indicator: FootprintIndicator) {    
        match self.indicators.entry(indicator) {
            Entry::Occupied(entry) => {
                entry.remove();
            }
            Entry::Vacant(entry) => {
                let data = match indicator {
                    FootprintIndicator::Volume => {
                        let volume_data = self.data_points.iter()
                            .map(|(time, (_, kline))| (*time, (kline.volume.0, kline.volume.1)))
                            .collect();
                        IndicatorData::Volume(Caches::default(), volume_data)
                    },
                    FootprintIndicator::OpenInterest => {
                        IndicatorData::OpenInterest(Caches::default(), BTreeMap::new())
                    }
                };
                entry.insert(data);
    
                if self.chart.indicators_split.is_none() {
                    self.chart.indicators_split = Some(0.8);
                }
            }
        }
    
        if self.indicators.is_empty() {
            self.chart.indicators_split = None;
        }
    }

    pub fn view_indicators<I: Indicator>(
        &self, 
        enabled: &[I], 
    ) -> Option<Element<Message>> {
        let chart_state: &CommonChartData = self.get_common_data();

        if chart_state.loading_chart {
            return None;
        }

        let mut indicators: iced::widget::Column<'_, Message> = column![];

        let visible_region = chart_state.visible_region(chart_state.bounds.size());

        let earliest = chart_state.x_to_time(visible_region.x);
        let latest = chart_state.x_to_time(visible_region.x + visible_region.width);

        for indicator in I::get_enabled(
            enabled, 
            chart_state.ticker_info.map(|info| info.get_market_type())
        ) {
            if let Some(candlestick_indicator) = indicator
                .as_any()
                .downcast_ref::<FootprintIndicator>() 
            {
                match candlestick_indicator {
                    FootprintIndicator::Volume => {
                        if let Some(IndicatorData::Volume(cache, data)) = 
                            self.indicators.get(&FootprintIndicator::Volume) {
                                indicators = indicators.push(
                                    indicators::volume::create_indicator_elem(chart_state, cache, data, earliest, latest)
                                );
                            }
                    },
                    FootprintIndicator::OpenInterest => {
                        if let Some(IndicatorData::OpenInterest(cache, data)) = 
                            self.indicators.get(&FootprintIndicator::OpenInterest) {
                                indicators = indicators.push(
                                    indicators::open_interest::create_indicator_elem(chart_state, cache, data, earliest, latest)
                                );
                            }
                    }
                }
            }
        }

        Some(
            container(indicators)
                .width(Length::FillPortion(10))
                .height(Length::Fill)
                .into()
        )
    }

    pub fn update(&mut self, message: &Message) -> Task<Message> {
        self.update_chart(message)
    }

    pub fn view<'a, I: Indicator>(
        &'a self, 
        indicators: &'a [I], 
        timezone: &'a UserTimezone,
    ) -> Element<'a, Message> {
        view_chart(self, indicators, timezone)
    }
}

impl canvas::Program<Message> for FootprintChart {
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
        if self.data_points.is_empty() {
            return vec![];
        }

        let chart = self.get_common_data();

        let center = Vector::new(bounds.width / 2.0, bounds.height / 2.0);
        let bounds_size = bounds.size();

        let palette = theme.extended_palette();

        let footprint = chart.cache.main.draw(renderer, bounds_size, |frame| {
            frame.with_save(|frame| {
                frame.translate(center);
                frame.scale(chart.scaling);
                frame.translate(chart.translation);

                let region = chart.visible_region(frame.size());

                let (cell_width, cell_height) = (chart.cell_width, chart.cell_height);

                let (earliest, latest) = (
                    chart.x_to_time(region.x) - (chart.timeframe / 2) as i64,
                    chart.x_to_time(region.x + region.width) + (chart.timeframe / 2) as i64,
                );
                let (highest, lowest) = (
                    chart.y_to_price(region.y),
                    chart.y_to_price(region.y + region.height),
                );

                let (max_trade_qty, _) =
                    self.calc_qty_scales(earliest, latest, highest, lowest, chart.tick_size);

                let cell_height_unscaled = cell_height * chart.scaling;
                let cell_width_unscaled = cell_width * chart.scaling;

                let text_size = cell_height_unscaled.round().min(16.0) - 4.0;

                let candle_width = 0.1 * cell_width;

                self.data_points.range(earliest..=latest)
                    .for_each(|(timestamp, (trades, kline))| {
                        let x_position = chart.time_to_x(*timestamp);

                        let y_open = chart.price_to_y(kline.open);
                        let y_high = chart.price_to_y(kline.high);
                        let y_low = chart.price_to_y(kline.low);
                        let y_close = chart.price_to_y(kline.close);

                        // Kline body
                        let body_color = if kline.close >= kline.open {
                            palette.success.weak.color
                        } else {
                            palette.danger.weak.color
                        };
                        frame.fill_rectangle(
                            Point::new(x_position - (candle_width / 8.0), y_open.min(y_close)),
                            Size::new(candle_width / 4.0, (y_open - y_close).abs()),
                            body_color,
                        );

                        // Kline wick
                        let wick_color = if kline.close >= kline.open {
                            palette.success.weak.color
                        } else {
                            palette.danger.weak.color
                        };

                        let marker_line = Stroke::with_color(
                            Stroke {
                                width: 1.0,
                                ..Default::default()
                            },
                            wick_color.scale_alpha(0.6),
                        );
        
                        frame.stroke(
                            &Path::line(
                                Point::new(x_position, y_high),
                                Point::new(x_position, y_low),
                            ),
                            marker_line,
                        );

                        // Trades
                        for trade in trades {
                            let y_position = chart.price_to_y(**trade.0);

                            let mut bar_color_alpha = 1.0;

                            if trade.1 .0 > 0.0 {
                                if cell_height_unscaled > 12.0 && cell_width_unscaled > 108.0 {
                                    let text_content = abbr_large_numbers(trade.1 .0);

                                    let text_position =
                                        Point::new(x_position + (candle_width / 4.0), y_position);

                                    frame.fill_text(canvas::Text {
                                        content: text_content,
                                        position: text_position,
                                        size: iced::Pixels(text_size),
                                        color: palette.background.weak.text,
                                        horizontal_alignment: Alignment::Start.into(),
                                        vertical_alignment: Alignment::Center.into(),
                                        ..canvas::Text::default()
                                    });

                                    bar_color_alpha = 0.6;
                                }

                                let bar_width = (trade.1 .0 / max_trade_qty) * (cell_width * 0.4);

                                frame.fill_rectangle(
                                    Point::new(
                                        x_position + (candle_width / 4.0),
                                        y_position - (cell_height / 2.0),
                                    ),
                                    Size::new(bar_width, cell_height),
                                    palette.success.base.color.scale_alpha(bar_color_alpha),
                                );
                            }
                            if trade.1 .1 > 0.0 {
                                if cell_height_unscaled > 12.0 && cell_width_unscaled > 108.0 {
                                    let text_content = abbr_large_numbers(trade.1 .1);

                                    let text_position =
                                        Point::new(x_position - (candle_width / 4.0), y_position);

                                    frame.fill_text(canvas::Text {
                                        content: text_content,
                                        position: text_position,
                                        size: iced::Pixels(text_size),
                                        color: palette.background.weak.text,
                                        horizontal_alignment: Alignment::End.into(),
                                        vertical_alignment: Alignment::Center.into(),
                                        ..canvas::Text::default()
                                    });

                                    bar_color_alpha = 0.6;
                                }

                                let bar_width = -(trade.1 .1 / max_trade_qty) * (cell_width * 0.4);

                                frame.fill_rectangle(
                                    Point::new(
                                        x_position - (candle_width / 4.0),
                                        y_position - (cell_height / 2.0),
                                    ),
                                    Size::new(bar_width, cell_height),
                                    palette.danger.base.color.scale_alpha(bar_color_alpha),
                                );
                            }
                        }
                    },
                    );

                // last price line
                if let Some(price) = &chart.last_price {
                    let (mut y_pos, line_color) = price.get_with_color(palette);
                    y_pos = chart.price_to_y(y_pos);

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
                };
            });
        });

        if chart.crosshair {
            let crosshair = chart.cache.crosshair.draw(renderer, bounds_size, |frame| {
                if let Some(cursor_position) = cursor.position_in(bounds) {
                    let (_, rounded_timestamp) =
                        chart.draw_crosshair(frame, theme, bounds_size, cursor_position);

                    if let Some((_, (_, kline))) = self
                        .data_points
                        .iter()
                        .find(|(time, _)| **time == rounded_timestamp)
                    {
                        let tooltip_text = format!(
                            "O: {}   H: {}   L: {}   C: {}",
                            kline.open,
                            kline.high,
                            kline.low,
                            kline.close,
                        );

                        let text = canvas::Text {
                            content: tooltip_text,
                            position: Point::new(8.0, 8.0),
                            size: iced::Pixels(12.0),
                            color: palette.background.base.text,
                            ..canvas::Text::default()
                        };
                        frame.fill_text(text);
                    }
                }
            });

            vec![footprint, crosshair]
        } else {
            vec![footprint]
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
