use super::{
    Chart, ChartConstants, CommonChartData, Interaction, Message, scale::linear::PriceInfoLabel,
};
use crate::{chart::TEXT_SIZE, style};
use data::chart::{
    Basis, ChartLayout,
    heatmap::{Config, GroupedTrade, HistoricalDepth, QtyScale},
    indicator::HeatmapIndicator,
};
use data::util::{abbr_large_numbers, count_decimals};
use exchange::{TickerInfo, Trade, adapter::MarketKind, depth::Depth};

use iced::widget::canvas::{self, Event, Geometry, Path};
use iced::{
    Alignment, Color, Element, Point, Rectangle, Renderer, Size, Theme, Vector, mouse,
    theme::palette::Extended,
};

use ordered_float::OrderedFloat;
use std::{
    collections::{BTreeMap, HashMap},
    time::Instant,
};

const CLEANUP_THRESHOLD: usize = 4800;

const TOOLTIP_WIDTH: f32 = 180.0;
const TOOLTIP_HEIGHT: f32 = 60.0;
const TOOLTIP_PADDING: f32 = 12.0;

impl Chart for HeatmapChart {
    type IndicatorType = HeatmapIndicator;

    fn common_data(&self) -> &CommonChartData {
        &self.chart
    }

    fn common_data_mut(&mut self) -> &mut CommonChartData {
        &mut self.chart
    }

    fn invalidate(&mut self) {
        self.invalidate(None);
    }

    fn view_indicators(&self, _indicators: &[Self::IndicatorType]) -> Vec<Element<Message>> {
        vec![]
    }

    fn visible_timerange(&self) -> (u64, u64) {
        let chart = self.common_data();
        let visible_region = chart.visible_region(chart.bounds.size());

        (
            chart.x_to_interval(visible_region.x),
            chart.x_to_interval(visible_region.x + visible_region.width),
        )
    }

    fn interval_keys(&self) -> Option<Vec<u64>> {
        None
    }

    fn autoscaled_coords(&self) -> Vector {
        let chart = self.common_data();
        Vector::new(
            0.5 * (chart.bounds.width / chart.scaling) - (90.0 / chart.scaling),
            0.0,
        )
    }

    fn is_empty(&self) -> bool {
        self.timeseries.is_empty()
    }
}

impl ChartConstants for HeatmapChart {
    fn min_scaling(&self) -> f32 {
        data::chart::heatmap::MIN_SCALING
    }

    fn max_scaling(&self) -> f32 {
        data::chart::heatmap::MAX_SCALING
    }

    fn max_cell_width(&self) -> f32 {
        data::chart::heatmap::MAX_CELL_WIDTH
    }

    fn min_cell_width(&self) -> f32 {
        data::chart::heatmap::MIN_CELL_WIDTH
    }

    fn max_cell_height(&self) -> f32 {
        data::chart::heatmap::MAX_CELL_HEIGHT
    }

    fn min_cell_height(&self) -> f32 {
        data::chart::heatmap::MIN_CELL_HEIGHT
    }

    fn default_cell_width(&self) -> f32 {
        data::chart::heatmap::DEFAULT_CELL_WIDTH
    }
}

enum IndicatorData {
    Volume,
    SessionVolumeProfile(HashMap<OrderedFloat<f32>, (f32, f32)>),
}

pub struct HeatmapChart {
    chart: CommonChartData,
    timeseries: BTreeMap<u64, (Box<[GroupedTrade]>, (f32, f32))>,
    indicators: HashMap<HeatmapIndicator, IndicatorData>,
    pause_buffer: Vec<(u64, Box<[Trade]>, Depth)>,
    heatmap: HistoricalDepth,
    visual_config: Config,
    last_tick: Instant,
}

impl HeatmapChart {
    pub fn new(
        layout: ChartLayout,
        basis: Basis,
        tick_size: f32,
        enabled_indicators: &[HeatmapIndicator],
        ticker_info: Option<TickerInfo>,
        config: Option<Config>,
    ) -> Self {
        HeatmapChart {
            chart: CommonChartData {
                cell_width: data::chart::heatmap::DEFAULT_CELL_WIDTH,
                cell_height: 4.0,
                tick_size,
                decimals: count_decimals(tick_size),
                crosshair: layout.crosshair,
                splits: layout.splits,
                ticker_info,
                basis,
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
            pause_buffer: vec![],
            heatmap: HistoricalDepth::new(
                ticker_info.expect("basis set without ticker info").min_qty,
                tick_size,
                basis,
            ),
            timeseries: BTreeMap::new(),
            visual_config: config.unwrap_or_default(),
            last_tick: Instant::now(),
        }
    }

    pub fn insert_datapoint(
        &mut self,
        trades_buffer: &[Trade],
        depth_update_t: u64,
        depth: &Depth,
    ) {
        let chart = &mut self.chart;

        let mid_price = depth.mid_price().unwrap_or(chart.base_price_y);
        chart.last_price = Some(PriceInfoLabel::Neutral(mid_price));

        // if current orderbook not visible, pause the data insertion and buffer them instead
        let is_paused = { chart.translation.x * chart.scaling > chart.bounds.width / 2.0 };

        if is_paused {
            self.pause_buffer.push((
                depth_update_t,
                trades_buffer.to_vec().into_boxed_slice(),
                depth.clone(),
            ));

            return;
        } else if !self.pause_buffer.is_empty() {
            self.pause_buffer.sort_by_key(|(time, _, _)| *time);

            for (time, trades, depth) in std::mem::take(&mut self.pause_buffer) {
                self.process_datapoint(&trades, time, &depth);
            }
        } else {
            self.cleanup_old_data();
        }

        self.process_datapoint(trades_buffer, depth_update_t, depth);
    }

    fn cleanup_old_data(&mut self) {
        if self.timeseries.len() > CLEANUP_THRESHOLD {
            let keys_to_remove = self
                .timeseries
                .keys()
                .take(CLEANUP_THRESHOLD / 10)
                .copied()
                .collect::<Vec<u64>>();

            for key in keys_to_remove {
                self.timeseries.remove(&key);
            }

            if let Some(oldest_time) = self.timeseries.keys().next().copied() {
                self.heatmap.cleanup_old_price_levels(oldest_time);
            }
        }
    }

    fn process_datapoint(&mut self, trades_buffer: &[Trade], depth_update: u64, depth: &Depth) {
        let chart = &mut self.chart;

        let aggregate_time: u64 = match chart.basis {
            Basis::Time(interval) => interval.into(),
            Basis::Tick(_) => todo!(),
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

                match grouped_trades
                    .binary_search_by(|probe| probe.compare_with(trade.price, trade.is_sell))
                {
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

            match self.timeseries.entry(rounded_depth_update) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert((grouped_trades.into_boxed_slice(), (buy_volume, sell_volume)));
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    let (existing_trades, (existing_buy, existing_sell)) = entry.get_mut();

                    *existing_buy += buy_volume;
                    *existing_sell += sell_volume;

                    let mut merged_trades = existing_trades.to_vec();

                    for trade in grouped_trades {
                        match merged_trades.binary_search_by(|probe| {
                            probe.compare_with(trade.price, trade.is_sell)
                        }) {
                            Ok(index) => merged_trades[index].qty += trade.qty,
                            Err(index) => merged_trades.insert(index, trade),
                        }
                    }

                    *existing_trades = merged_trades.into_boxed_slice();
                }
            }
        };

        self.heatmap
            .insert_latest_depth(depth, rounded_depth_update);

        {
            let mid_price = depth.mid_price().unwrap_or(chart.base_price_y);
            chart.base_price_y = (mid_price / (chart.tick_size)).round() * (chart.tick_size);
        }

        chart.latest_x = rounded_depth_update;
    }

    pub fn visual_config(&self) -> Config {
        self.visual_config
    }

    pub fn set_visual_config(&mut self, visual_config: Config) {
        self.visual_config = visual_config;
        self.invalidate(Some(Instant::now()));
    }

    pub fn set_basis(&mut self, basis: Basis) {
        self.chart.basis = basis;

        self.timeseries.clear();
        self.heatmap = HistoricalDepth::new(
            self.chart
                .ticker_info
                .expect("basis set without ticker info")
                .min_qty,
            self.chart.tick_size,
            basis,
        );

        let autoscaled_coords = self.autoscaled_coords();
        self.chart.translation = autoscaled_coords;

        self.invalidate(None);
    }

    pub fn basis_interval(&self) -> Option<u64> {
        match self.chart.basis {
            Basis::Time(interval) => Some(interval.into()),
            Basis::Tick(_) => None,
        }
    }

    pub fn chart_layout(&self) -> ChartLayout {
        self.chart.get_chart_layout()
    }

    pub fn change_tick_size(&mut self, new_tick_size: f32) {
        let chart_state = self.common_data_mut();

        let basis = chart_state.basis;

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
        self.heatmap = HistoricalDepth::new(
            self.chart
                .ticker_info
                .expect("basis set without ticker info")
                .min_qty,
            new_tick_size,
            basis,
        );
    }

    pub fn tick_size(&self) -> f32 {
        self.chart.tick_size
    }

    pub fn toggle_indicator(&mut self, indicator: HeatmapIndicator) {
        match self.indicators.entry(indicator) {
            std::collections::hash_map::Entry::Occupied(entry) => {
                entry.remove();
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
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

    pub fn invalidate(&mut self, now: Option<Instant>) -> Option<super::Action> {
        let autoscaled_coords = self.autoscaled_coords();
        let chart = &mut self.chart;

        if chart.autoscale {
            chart.translation = autoscaled_coords;
        }

        chart.cache.clear_all();

        if let Some(t) = now {
            self.last_tick = t;
        }

        None
    }

    pub fn last_update(&self) -> Instant {
        self.last_tick
    }

    fn calc_qty_scales(&self, earliest: u64, latest: u64, highest: f32, lowest: f32) -> QtyScale {
        let market_type = match self.chart.ticker_info {
            Some(ref ticker_info) => ticker_info.market_type(),
            None => return QtyScale::default(),
        };

        let (mut max_aggr_volume, mut max_trade_qty) = (0.0f32, 0.0f32);
        let mut max_depth_qty = 0.0f32;

        self.timeseries
            .range(earliest..=latest)
            .for_each(|(_, (trades, _))| {
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

        self.heatmap
            .iter_time_filtered(earliest, latest, highest, lowest)
            .for_each(|(price, runs)| {
                runs.iter()
                    .filter_map(|run| {
                        let visible_run = run.with_range(earliest, latest)?;

                        let order_size = match market_type {
                            MarketKind::InversePerps => visible_run.qty(),
                            _ => **price * visible_run.qty(),
                        };

                        if order_size > self.visual_config.order_size_filter {
                            Some(visible_run)
                        } else {
                            None
                        }
                    })
                    .for_each(|run| {
                        max_depth_qty = max_depth_qty.max(run.qty());
                    });
            });

        QtyScale {
            max_trade_qty,
            max_aggr_volume,
            max_depth_qty,
        }
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
        super::canvas_interaction(self, interaction, event, bounds, cursor)
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let chart = self.common_data();

        if chart.bounds.width == 0.0 {
            return vec![];
        }

        let market_type = match self.chart.ticker_info {
            Some(ref ticker_info) => ticker_info.market_type(),
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

            let (earliest, latest) = chart.interval_range(&region);
            let (highest, lowest) = chart.price_range(&region);

            if latest < earliest {
                return;
            }

            let cell_height = chart.cell_height;
            let cell_height_scaled = cell_height * chart.scaling;

            let qty_scales = self.calc_qty_scales(earliest, latest, highest, lowest);

            let max_depth_qty = qty_scales.max_depth_qty;
            let (max_aggr_volume, max_trade_qty) =
                (qty_scales.max_aggr_volume, qty_scales.max_trade_qty);

            if let Some(merge_strat) = self.visual_config().coalescing {
                let coalesced_visual_runs = self.heatmap.coalesced_runs(
                    earliest,
                    latest,
                    highest,
                    lowest,
                    market_type,
                    self.visual_config.order_size_filter,
                    merge_strat,
                );

                for (price_of_run, visual_run) in coalesced_visual_runs {
                    let y_position = chart.price_to_y(price_of_run.into_inner());

                    let run_start_time_clipped = visual_run.start_time.max(earliest);
                    let run_until_time_clipped = visual_run.until_time.min(latest);

                    if run_start_time_clipped >= run_until_time_clipped {
                        continue;
                    }

                    let start_x = chart.interval_to_x(run_start_time_clipped);
                    let end_x = chart.interval_to_x(run_until_time_clipped).min(0.0);

                    let width = end_x - start_x;

                    if width > 0.001 {
                        let color_alpha = (visual_run.qty() / max_depth_qty).min(1.0);

                        frame.fill_rectangle(
                            Point::new(start_x, y_position - (cell_height / 2.0)),
                            Size::new(width, cell_height),
                            depth_color(palette, visual_run.is_bid, color_alpha),
                        );
                    }
                }
            } else {
                self.heatmap
                    .iter_time_filtered(earliest, latest, highest, lowest)
                    .for_each(|(price, runs)| {
                        let y_position = chart.price_to_y(price.0);

                        runs.iter()
                            .filter(|run| {
                                let order_size = match market_type {
                                    MarketKind::InversePerps => run.qty(),
                                    _ => **price * run.qty(),
                                };
                                order_size > self.visual_config.order_size_filter
                            })
                            .for_each(|run| {
                                let start_x = chart.interval_to_x(run.start_time.max(earliest));
                                let end_x =
                                    chart.interval_to_x(run.until_time.min(latest)).min(0.0);

                                let width = end_x - start_x;

                                let color_alpha = (run.qty() / max_depth_qty).min(1.0);

                                frame.fill_rectangle(
                                    Point::new(start_x, y_position - (cell_height / 2.0)),
                                    Size::new(width, cell_height),
                                    depth_color(palette, run.is_bid, color_alpha),
                                );
                            });
                    });
            }

            if let Some((latest_timestamp, _)) = self.timeseries.last_key_value() {
                let max_qty = self
                    .heatmap
                    .latest_order_runs(highest, lowest, *latest_timestamp)
                    .map(|(_, run)| run.qty())
                    .fold(f32::MIN, f32::max)
                    .ceil()
                    * 5.0
                    / 5.0;

                if !max_qty.is_infinite() {
                    self.heatmap
                        .latest_order_runs(highest, lowest, *latest_timestamp)
                        .for_each(|(price, run)| {
                            let y_position = chart.price_to_y(price.0);
                            let bar_width = (run.qty() / max_qty) * 50.0;

                            frame.fill_rectangle(
                                Point::new(0.0, y_position - (cell_height / 2.0)),
                                Size::new(bar_width, cell_height),
                                depth_color(palette, run.is_bid, 0.5),
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
                        font: style::AZERET_MONO,
                        ..canvas::Text::default()
                    });
                }
            };

            self.timeseries.range(earliest..=latest).for_each(
                |(time, (trades, (buy_volume, sell_volume)))| {
                    let x_position = chart.interval_to_x(*time);

                    trades.iter().for_each(|trade| {
                        let y_position = chart.price_to_y(trade.price);

                        let trade_size = match market_type {
                            MarketKind::InversePerps => trade.qty,
                            _ => trade.qty * trade.price,
                        };

                        if trade_size > self.visual_config.trade_size_filter {
                            let color = if trade.is_sell {
                                palette.danger.base.color
                            } else {
                                palette.success.base.color
                            };

                            let radius = {
                                if let Some(trade_size_scale) = self.visual_config.trade_size_scale
                                {
                                    let scale_factor = (trade_size_scale as f32) / 100.0;
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
                let segments = ((max_bar_width / min_segment_width).floor() as usize).clamp(10, 40);

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

                        super::draw_horizontal_volume_bars(
                            frame,
                            region.x,
                            y_position,
                            *buy_v,
                            *sell_v,
                            max_vpsr,
                            vpsr_height,
                            max_bar_width,
                            palette.success.weak.color,
                            palette.danger.weak.color,
                            1.0,
                        );
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
                        font: style::AZERET_MONO,
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
                    font: style::AZERET_MONO,
                    ..canvas::Text::default()
                });
            }

            let is_paused = chart.translation.x * chart.scaling > chart.bounds.width / 2.0;
            if is_paused {
                let bar_width = 8.0 / chart.scaling;
                let bar_height = 32.0 / chart.scaling;
                let padding = 24.0 / chart.scaling;

                let total_icon_width = bar_width * 3.0;

                let pause_bar = Rectangle {
                    x: (region.x + region.width) - total_icon_width - padding,
                    y: region.y + padding,
                    width: bar_width,
                    height: bar_height,
                };

                frame.fill_rectangle(
                    pause_bar.position(),
                    pause_bar.size(),
                    palette.background.base.text.scale_alpha(0.4),
                );

                frame.fill_rectangle(
                    pause_bar.position() + Vector::new(pause_bar.width * 2.0, 0.0),
                    pause_bar.size(),
                    palette.background.base.text.scale_alpha(0.4),
                );
            }
        });

        if chart.crosshair & !self.timeseries.is_empty() {
            let crosshair = chart.cache.crosshair.draw(renderer, bounds_size, |frame| {
                if let Some(cursor_position) = cursor.position_in(bounds) {
                    let (cursor_at_price, cursor_at_time) =
                        chart.draw_crosshair(frame, theme, bounds_size, cursor_position);

                    let aggr_time: u64 = match chart.basis {
                        Basis::Time(interval) => interval.into(),
                        Basis::Tick(_) => return,
                    };
                    let tick_size = chart.tick_size;

                    let base_data_price = (cursor_at_price / tick_size).round() * tick_size;
                    let base_data_time = cursor_at_time.saturating_sub(aggr_time);

                    let price_tick_offsets = [1i64, 0, -1];
                    let time_interval_offsets = [-1i64, 0, 1, 2];

                    let prices_for_display_lookup: [f32; 3] = std::array::from_fn(|i| {
                        let offset = price_tick_offsets[i];
                        base_data_price + (offset as f32 * tick_size)
                    });
                    let times_for_display_lookup: [u64; 4] = std::array::from_fn(|i| {
                        let offset = time_interval_offsets[i];
                        base_data_time.saturating_add_signed(offset * aggr_time as i64)
                    });

                    let display_grid_qtys: HashMap<(u64, OrderedFloat<f32>), (f32, bool)> =
                        self.heatmap.query_grid_qtys(
                            base_data_time,
                            base_data_price,
                            &time_interval_offsets,
                            &price_tick_offsets,
                            market_type,
                            self.visual_config.order_size_filter,
                            self.visual_config.coalescing,
                        );

                    if display_grid_qtys.is_empty() {
                        return;
                    }

                    let should_draw_below = cursor_position.y < TOOLTIP_HEIGHT + TOOLTIP_PADDING;
                    let should_draw_left =
                        cursor_position.x > bounds.width - (TOOLTIP_WIDTH + TOOLTIP_PADDING);

                    let overlay_top_left_x = if should_draw_left {
                        cursor_position.x - TOOLTIP_WIDTH - TOOLTIP_PADDING
                    } else {
                        cursor_position.x + TOOLTIP_PADDING
                    };

                    let overlay_top_left_y = if should_draw_below {
                        cursor_position.y + TOOLTIP_PADDING
                    } else {
                        cursor_position.y - TOOLTIP_HEIGHT - TOOLTIP_PADDING
                    };

                    let overlay_background = Path::rectangle(
                        Point::new(overlay_top_left_x, overlay_top_left_y),
                        Size::new(TOOLTIP_WIDTH, TOOLTIP_HEIGHT),
                    );
                    frame.fill(
                        &overlay_background,
                        palette.background.weakest.color.scale_alpha(0.9),
                    );

                    let cell_width_overlay = TOOLTIP_WIDTH / 4.0;
                    let cell_height_overlay = TOOLTIP_HEIGHT / 3.0;

                    let palette = theme.extended_palette();

                    for (display_row_idx, &data_price_val) in
                        prices_for_display_lookup.iter().enumerate()
                    {
                        let data_price_key = OrderedFloat(data_price_val);

                        for (display_col_idx, &data_time_val) in
                            times_for_display_lookup.iter().enumerate()
                        {
                            if let Some((qty, is_bid)) =
                                display_grid_qtys.get(&(data_time_val, data_price_key))
                            {
                                let text_content = abbr_large_numbers(*qty);
                                let color = if *is_bid {
                                    palette.success.base.color
                                } else {
                                    palette.danger.base.color
                                };

                                let text_pos_x = overlay_top_left_x
                                    + (display_col_idx as f32 * cell_width_overlay)
                                    + cell_width_overlay / 2.0;
                                let text_pos_y = overlay_top_left_y
                                    + (display_row_idx as f32 * cell_height_overlay)
                                    + cell_height_overlay / 2.0;

                                frame.fill_text(canvas::Text {
                                    content: text_content,
                                    position: Point::new(text_pos_x, text_pos_y),
                                    size: iced::Pixels(TEXT_SIZE - 2.0),
                                    color,
                                    font: style::AZERET_MONO,
                                    align_y: Alignment::Center.into(),
                                    align_x: Alignment::Center.into(),
                                    ..canvas::Text::default()
                                });
                            }
                        }
                    }
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

fn depth_color(palette: &Extended, is_bid: bool, alpha: f32) -> Color {
    if is_bid {
        palette.success.strong.color.scale_alpha(alpha)
    } else {
        palette.danger.strong.color.scale_alpha(alpha)
    }
}
