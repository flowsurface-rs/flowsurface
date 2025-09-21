use super::{
    Chart, Interaction, Message, PlotConstants, TEXT_SIZE, ViewState, scale::linear::PriceInfoLabel,
};
use crate::{
    modal::pane::settings::study::{self, Study},
    style,
};
use data::chart::{
    Basis, ViewConfig,
    heatmap::{
        CLEANUP_THRESHOLD, Config, HeatmapDataPoint, HeatmapStudy, HistoricalDepth, ProfileKind,
        QtyScale,
    },
    indicator::HeatmapIndicator,
};
use data::util::{abbr_large_numbers, count_decimals};
use data::{
    aggr::time::{DataPoint, TimeSeries},
    chart::Autoscale,
};
use exchange::util::{Price, PriceStep};
use exchange::{SIZE_IN_QUOTE_CURRENCY, TickerInfo, Trade, depth::Depth};

use iced::widget::canvas::{self, Event, Geometry, Path};
use iced::{
    Alignment, Color, Element, Point, Rectangle, Renderer, Size, Theme, Vector, mouse,
    theme::palette::Extended,
};

use enum_map::EnumMap;
use rustc_hash::FxHashMap;
use std::time::Instant;

const MIN_SCALING: f32 = 0.6;
const MAX_SCALING: f32 = 1.2;

const MAX_CELL_WIDTH: f32 = 12.0;
const MIN_CELL_WIDTH: f32 = 1.0;

const MAX_CELL_HEIGHT: f32 = 10.0;
const MIN_CELL_HEIGHT: f32 = 1.0;

const DEFAULT_CELL_WIDTH: f32 = 3.0;

const TOOLTIP_WIDTH: f32 = 198.0;
const TOOLTIP_HEIGHT: f32 = 66.0;
const TOOLTIP_PADDING: f32 = 12.0;

const MAX_CIRCLE_RADIUS: f32 = 16.0;

impl Chart for HeatmapChart {
    type IndicatorKind = HeatmapIndicator;

    fn state(&self) -> &ViewState {
        &self.chart
    }

    fn mut_state(&mut self) -> &mut ViewState {
        &mut self.chart
    }

    fn invalidate_crosshair(&mut self) {
        self.chart.cache.clear_crosshair();
    }

    fn invalidate_all(&mut self) {
        self.invalidate(None);
    }

    fn view_indicators(&'_ self, _indicators: &[Self::IndicatorKind]) -> Vec<Element<'_, Message>> {
        vec![]
    }

    fn visible_timerange(&self) -> (u64, u64) {
        let chart = self.state();
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
        let chart = self.state();
        Vector::new(
            0.5 * (chart.bounds.width / chart.scaling) - (90.0 / chart.scaling),
            chart.translation.y,
        )
    }

    fn supports_fit_autoscaling(&self) -> bool {
        false
    }

    fn is_empty(&self) -> bool {
        self.trades.datapoints.is_empty()
    }
}

impl PlotConstants for HeatmapChart {
    fn min_scaling(&self) -> f32 {
        MIN_SCALING
    }

    fn max_scaling(&self) -> f32 {
        MAX_SCALING
    }

    fn max_cell_width(&self) -> f32 {
        MAX_CELL_WIDTH
    }

    fn min_cell_width(&self) -> f32 {
        MIN_CELL_WIDTH
    }

    fn max_cell_height(&self) -> f32 {
        MAX_CELL_HEIGHT
    }

    fn min_cell_height(&self) -> f32 {
        MIN_CELL_HEIGHT
    }

    fn default_cell_width(&self) -> f32 {
        DEFAULT_CELL_WIDTH
    }
}

#[derive(Default)]
enum IndicatorData {
    #[default]
    Volume,
}

pub struct HeatmapChart {
    chart: ViewState,
    trades: TimeSeries<HeatmapDataPoint>,
    indicators: EnumMap<HeatmapIndicator, Option<IndicatorData>>,
    pause_buffer: Vec<(u64, Box<[Trade]>, Depth)>,
    heatmap: HistoricalDepth,
    visual_config: Config,
    study_configurator: study::Configurator<HeatmapStudy>,
    last_tick: Instant,
    pub studies: Vec<HeatmapStudy>,
}

impl HeatmapChart {
    pub fn new(
        layout: ViewConfig,
        basis: Basis,
        tick_size: f32,
        enabled_indicators: &[HeatmapIndicator],
        ticker_info: Option<TickerInfo>,
        config: Option<Config>,
        studies: Vec<HeatmapStudy>,
    ) -> Self {
        let step = PriceStep::from_f32(tick_size);

        let mut indicators = EnumMap::default();
        for &indicator in enabled_indicators {
            let data = match indicator {
                HeatmapIndicator::Volume => IndicatorData::Volume,
            };
            indicators[indicator] = Some(data);
        }

        let heatmap = HistoricalDepth::new(
            ticker_info
                .expect("basis set without ticker info")
                .min_qty
                .into(),
            step,
            basis,
        );

        HeatmapChart {
            chart: ViewState {
                cell_width: DEFAULT_CELL_WIDTH,
                cell_height: 4.0,
                tick_size: step,
                decimals: count_decimals(tick_size),
                layout: ViewConfig {
                    splits: layout.splits,
                    autoscale: Some(Autoscale::CenterLatest),
                },
                ticker_info,
                basis,
                ..Default::default()
            },
            indicators,
            pause_buffer: vec![],
            heatmap,
            trades: TimeSeries::<HeatmapDataPoint>::new(basis, step),
            visual_config: config.unwrap_or_default(),
            study_configurator: study::Configurator::new(),
            studies,
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
        chart.last_price = Some(PriceInfoLabel::Neutral(mid_price.to_f32()));

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
        if self.trades.datapoints.len() > CLEANUP_THRESHOLD {
            let keys_to_remove = self
                .trades
                .datapoints
                .keys()
                .take(CLEANUP_THRESHOLD / 10)
                .copied()
                .collect::<Vec<u64>>();

            for key in keys_to_remove {
                self.trades.datapoints.remove(&key);
            }

            if let Some(oldest_time) = self.trades.datapoints.keys().next().copied() {
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
            let entry = self
                .trades
                .datapoints
                .entry(rounded_depth_update)
                .or_insert_with(|| HeatmapDataPoint {
                    grouped_trades: Box::new([]),
                    buy_sell: (0.0, 0.0),
                });

            for trade in trades_buffer {
                entry.add_trade(trade, chart.tick_size);
            }
        }

        self.heatmap
            .insert_latest_depth(depth, rounded_depth_update);

        {
            let mid_price = depth.mid_price().unwrap_or(chart.base_price_y);
            chart.base_price_y = mid_price.round_to_step(chart.tick_size);
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

        self.trades.datapoints.clear();
        self.heatmap = HistoricalDepth::new(
            self.chart
                .ticker_info
                .expect("basis set without ticker info")
                .min_qty
                .into(),
            self.chart.tick_size,
            basis,
        );

        let chart = &mut self.chart;
        chart.translation = Vector::new(
            0.5 * (chart.bounds.width / chart.scaling) - (90.0 / chart.scaling),
            0.0,
        );

        self.invalidate(None);
    }

    pub fn study_configurator(&self) -> &study::Configurator<HeatmapStudy> {
        &self.study_configurator
    }

    pub fn update_study_configurator(&mut self, message: study::Message<HeatmapStudy>) {
        let studies = &mut self.studies;

        match self.study_configurator.update(message) {
            Some(study::Action::ToggleStudy(study, is_selected)) => {
                if is_selected {
                    let already_exists = studies.iter().any(|s| s.is_same_type(&study));
                    if !already_exists {
                        studies.push(study);
                    }
                } else {
                    studies.retain(|s| !s.is_same_type(&study));
                }
            }
            Some(study::Action::ConfigureStudy(study)) => {
                if let Some(existing_study) = studies.iter_mut().find(|s| s.is_same_type(&study)) {
                    *existing_study = study;
                }
            }
            None => {}
        }

        self.invalidate(None);
    }

    pub fn basis_interval(&self) -> Option<u64> {
        match self.chart.basis {
            Basis::Time(interval) => Some(interval.into()),
            Basis::Tick(_) => None,
        }
    }

    pub fn chart_layout(&self) -> ViewConfig {
        self.chart.layout()
    }

    pub fn change_tick_size(&mut self, new_tick_size: f32) {
        let chart_state = self.mut_state();

        let basis = chart_state.basis;
        let step = PriceStep::from_f32(new_tick_size);

        chart_state.cell_height = 4.0;
        chart_state.tick_size = step;
        chart_state.decimals = count_decimals(new_tick_size);

        self.trades.datapoints.clear();
        self.heatmap = HistoricalDepth::new(
            self.chart
                .ticker_info
                .expect("basis set without ticker info")
                .min_qty
                .into(),
            step,
            basis,
        );
    }

    pub fn tick_size(&self) -> f32 {
        self.chart.tick_size.to_f32_lossy()
    }

    pub fn toggle_indicator(&mut self, indicator: HeatmapIndicator) {
        if self.indicators[indicator].is_some() {
            self.indicators[indicator] = None;
        } else {
            let data = match indicator {
                HeatmapIndicator::Volume => IndicatorData::Volume,
            };
            self.indicators[indicator] = Some(data);
        }
    }

    pub fn invalidate(&mut self, now: Option<Instant>) -> Option<super::Action> {
        let chart = &mut self.chart;

        if chart.layout.autoscale.is_some() {
            chart.translation = Vector::new(
                0.5 * (chart.bounds.width / chart.scaling) - (90.0 / chart.scaling),
                0.0,
            );
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

    fn calc_qty_scales(
        &self,
        earliest: u64,
        latest: u64,
        highest: Price,
        lowest: Price,
    ) -> QtyScale {
        let market_type = match self.chart.ticker_info {
            Some(ref ticker_info) => ticker_info.market_type(),
            None => return QtyScale::default(),
        };

        let (max_trade_qty, max_aggr_volume) =
            self.trades.max_trade_qty_and_aggr_volume(earliest, latest);

        let max_depth_qty = self.heatmap.max_depth_qty_in_range(
            earliest,
            latest,
            highest,
            lowest,
            market_type,
            self.visual_config.order_size_filter,
        );

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
        interaction: &Interaction,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let chart = self.state();

        if chart.bounds.width == 0.0 {
            return vec![];
        }

        let market_type = match self.chart.ticker_info {
            Some(ref ticker_info) => ticker_info.market_type(),
            None => return vec![],
        };

        let bounds_size = bounds.size();
        let palette = theme.extended_palette();

        let heatmap = chart.cache.main.draw(renderer, bounds_size, |frame| {
            let center = Vector::new(bounds.width / 2.0, bounds.height / 2.0);

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
            let qty_scales = self.calc_qty_scales(earliest, latest, highest, lowest);

            let max_depth_qty = qty_scales.max_depth_qty;
            let (max_aggr_volume, max_trade_qty) =
                (qty_scales.max_aggr_volume, qty_scales.max_trade_qty);

            let size_in_quote_currency = SIZE_IN_QUOTE_CURRENCY.get() == Some(&true);

            let volume_indicator = self.indicators[HeatmapIndicator::Volume].is_some();

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
                    let y_position = chart.price_to_y(price_of_run);

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
                        let y_position = chart.price_to_y(*price);

                        runs.iter()
                            .filter(|run| {
                                let order_size = market_type.qty_in_quote_value(
                                    run.qty(),
                                    *price,
                                    size_in_quote_currency,
                                );
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

            if let Some(latest_timestamp) = self.trades.latest_timestamp() {
                let max_qty = self
                    .heatmap
                    .latest_order_runs(highest, lowest, latest_timestamp)
                    .map(|(_, run)| run.qty())
                    .fold(f32::MIN, f32::max)
                    .ceil()
                    * 5.0
                    / 5.0;

                if !max_qty.is_infinite() {
                    self.heatmap
                        .latest_order_runs(highest, lowest, latest_timestamp)
                        .for_each(|(price, run)| {
                            let y_position = chart.price_to_y(*price);
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

            self.trades
                .datapoints
                .range(earliest..=latest)
                .for_each(|(time, dp)| {
                    let x_position = chart.interval_to_x(*time);

                    dp.grouped_trades.iter().for_each(|trade| {
                        let y_position = chart.price_to_y(trade.price);

                        let trade_size = market_type.qty_in_quote_value(
                            trade.qty,
                            trade.price,
                            size_in_quote_currency,
                        );

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
                                    1.0 + (trade.qty / max_trade_qty)
                                        * (MAX_CIRCLE_RADIUS - 1.0)
                                        * scale_factor
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
                        let area_height = (bounds.height / chart.scaling) * 0.1;

                        let (buy_volume, sell_volume) = dp.buy_sell;

                        super::draw_volume_bar(
                            frame,
                            x_position,
                            (region.y + region.height) - area_height,
                            buy_volume,
                            sell_volume,
                            max_aggr_volume,
                            area_height,
                            bar_width,
                            palette.success.base.color,
                            palette.danger.base.color,
                            1.0,
                            false,
                        );
                    }
                });

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
                    size: text_size.into(),
                    color: palette.background.base.text,
                    font: style::AZERET_MONO,
                    ..canvas::Text::default()
                });
            }

            let volume_profile: Option<&ProfileKind> = self
                .studies
                .iter()
                .map(|study| match study {
                    HeatmapStudy::VolumeProfile(profile) => profile,
                })
                .next();

            if let Some(profile_kind) = volume_profile {
                let area_width = (bounds.width / chart.scaling) * 0.1;

                let min_segment_width = 2.0;
                let segments = ((area_width / min_segment_width).floor() as usize).clamp(10, 40);

                for i in 0..segments {
                    let segment_width = area_width / segments as f32;
                    let segment_x = region.x + (i as f32 * segment_width);

                    let alpha = 0.95 - (0.85 * (i as f32 / (segments - 1) as f32).powf(2.0));

                    frame.fill_rectangle(
                        Point::new(segment_x, region.y),
                        Size::new(segment_width, region.height),
                        palette.background.weakest.color.scale_alpha(alpha),
                    );
                }

                draw_volume_profile(
                    frame,
                    &region,
                    profile_kind,
                    palette,
                    chart,
                    &self.trades,
                    area_width,
                );
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

        if !self.is_empty() {
            let crosshair = chart.cache.crosshair.draw(renderer, bounds_size, |frame| {
                if let Some(cursor_position) = cursor.position_in(bounds) {
                    let (cursor_at_price, cursor_at_time) = chart.draw_crosshair(
                        frame,
                        theme,
                        bounds_size,
                        cursor_position,
                        interaction,
                    );

                    if matches!(interaction, Interaction::Panning { .. })
                        || matches!(interaction, Interaction::Ruler { start } if start.is_some())
                    {
                        return;
                    }

                    let aggr_time: u64 = match chart.basis {
                        Basis::Time(interval) => interval.into(),
                        Basis::Tick(_) => return,
                    };
                    let tick_size = chart.tick_size.to_f32_lossy();
                    let step = chart.tick_size;

                    let base_data_price = Price::from_f32(cursor_at_price)
                        .round_to_step(step)
                        .to_f32();
                    let base_data_time = (cursor_at_time / aggr_time) * aggr_time;

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

                    let display_grid_qtys: FxHashMap<(u64, Price), (f32, bool)> =
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
                        let data_price_key = Price::from_f32(data_price_val).round_to_step(step);

                        for (display_col_idx, &data_time_val) in
                            times_for_display_lookup.iter().enumerate()
                        {
                            if let Some((qty, is_bid)) =
                                display_grid_qtys.get(&(data_time_val, data_price_key))
                            {
                                let text_content = abbr_large_numbers(*qty);
                                let color = if *is_bid {
                                    palette.success.strong.color
                                } else {
                                    palette.danger.strong.color
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
            Interaction::None | Interaction::Ruler { .. } => {
                if cursor.is_over(bounds) {
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

fn draw_volume_profile(
    frame: &mut canvas::Frame,
    region: &Rectangle,
    kind: &ProfileKind,
    palette: &Extended,
    chart: &ViewState,
    timeseries: &TimeSeries<HeatmapDataPoint>,
    area_width: f32,
) {
    let (highest, lowest) = chart.price_range(region);

    let time_range = match kind {
        ProfileKind::VisibleRange => {
            let earliest = chart.x_to_interval(region.x);
            let latest = chart.x_to_interval(region.x + region.width);
            earliest..=latest
        }
        ProfileKind::FixedWindow(datapoints) => {
            let basis_interval: u64 = match chart.basis {
                Basis::Time(interval) => interval.into(),
                Basis::Tick(_) => return,
            };

            let latest = chart
                .latest_x
                .min(chart.x_to_interval(region.x + region.width));
            let earliest = latest.saturating_sub((*datapoints as u64) * basis_interval);

            earliest..=latest
        }
    };

    let step = chart.tick_size;

    let first_tick = lowest.round_to_side_step(false, step);
    let last_tick = highest.round_to_side_step(true, step);

    let num_ticks = match Price::steps_between_inclusive(first_tick, last_tick, step) {
        Some(n) => n,
        None => return,
    };

    if num_ticks > 4096 {
        return;
    }

    let mut profile = vec![(0.0f32, 0.0f32); num_ticks];
    let mut max_aggr_volume = 0.0f32;

    timeseries.datapoints.range(time_range).for_each(|(_, dp)| {
        dp.grouped_trades
            .iter()
            .filter(|trade| trade.price >= lowest && trade.price <= highest)
            .for_each(|trade| {
                let grouped_price = trade.price.round_to_side_step(trade.is_sell, step);

                if grouped_price.units < first_tick.units || grouped_price.units > last_tick.units {
                    return;
                }

                let index = ((grouped_price.units - first_tick.units) / step.units) as usize;

                if let Some(entry) = profile.get_mut(index) {
                    if trade.is_sell {
                        entry.1 += trade.qty;
                    } else {
                        entry.0 += trade.qty;
                    }
                    max_aggr_volume = max_aggr_volume.max(entry.0 + entry.1);
                }
            });
    });

    profile
        .iter()
        .enumerate()
        .for_each(|(index, (buy_v, sell_v))| {
            if *buy_v > 0.0 || *sell_v > 0.0 {
                let price = first_tick.add_steps(index as i64, step);
                let y_position = chart.price_to_y(price);

                let next_price = price.add_steps(1, step);
                let next_y_position = chart.price_to_y(next_price);
                let bar_height = (next_y_position - y_position).abs();

                super::draw_volume_bar(
                    frame,
                    region.x,
                    y_position,
                    *buy_v,
                    *sell_v,
                    max_aggr_volume,
                    area_width,
                    bar_height,
                    palette.success.weak.color,
                    palette.danger.weak.color,
                    1.0,
                    true,
                );
            }
        });

    if max_aggr_volume > 0.0 {
        let text_size = 9.0 / chart.scaling;
        let text_content = abbr_large_numbers(max_aggr_volume);

        let text_position = Point::new(region.x + area_width, region.y);

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
