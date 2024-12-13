use std::collections::BTreeMap;

use iced::widget::canvas::{LineDash, Path, Stroke};
use iced::{mouse, Element, Point, Rectangle, Renderer, Size, Task, Theme, Vector};
use iced::widget::{canvas::{self, Event, Geometry}, column};

use crate::screen::UserTimezone;
use crate::data_providers::{
    fetcher::{FetchRange, RequestHandler},
    Kline, OpenInterest as OIData, Timeframe
};

use super::indicators::{self, CandlestickIndicator, Indicator};

use super::{request_fetch, Caches, Chart, ChartConstants, CommonChartData, Interaction, Message, PriceInfoLabel};
use super::{canvas_interaction, view_chart, update_chart, count_decimals};

impl Chart for CandlestickChart {
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

    fn view_indicator<I: Indicator>(&self, enabled: &[I]) -> Element<Message> {
        self.view_indicators(enabled)
    }

    fn get_visible_timerange(&self) -> (i64, i64) {
        let chart = self.get_common_data();

        let visible_region = chart.visible_region(chart.bounds.size());

        let earliest = chart.x_to_time(visible_region.x);
        let latest = chart.x_to_time(visible_region.x + visible_region.width);

        (earliest, latest)
    }
}

impl ChartConstants for CandlestickChart {
    const MIN_SCALING: f32 = 0.6;
    const MAX_SCALING: f32 = 2.5;

    const MAX_CELL_WIDTH: f32 = 16.0;
    const MIN_CELL_WIDTH: f32 = 1.0;

    const MAX_CELL_HEIGHT: f32 = 8.0;
    const MIN_CELL_HEIGHT: f32 = 1.0;

    const DEFAULT_CELL_WIDTH: f32 = 4.0;
}

#[allow(dead_code)]
enum Indicators {
    Volume(Caches, BTreeMap<i64, (f32, f32)>),
    OpenInterest(Caches, BTreeMap<i64, f32>),
}

impl Indicators {
    fn clear_cache(&mut self) {
        match self {
            Indicators::Volume(caches, _) 
            | Indicators::OpenInterest(caches, _) => {
                caches.clear_all();
            }
        }
    }
}

pub struct CandlestickChart {
    chart: CommonChartData,
    data_points: BTreeMap<i64, Kline>,
    indicators: Vec<Indicators>,
    request_handler: RequestHandler,
    fetching_oi: bool,
}

impl CandlestickChart {
    pub fn new(
        klines_raw: Vec<Kline>,
        timeframe: Timeframe,
        tick_size: f32,
        timezone: UserTimezone,
    ) -> CandlestickChart {
        let mut data_points = BTreeMap::new();
        let mut volume_data = BTreeMap::new();

        let base_price_y = klines_raw.last().unwrap_or(&Kline::default()).close;

        for kline in klines_raw {
            volume_data.insert(kline.time as i64, (kline.volume.0, kline.volume.1));
            data_points.entry(kline.time as i64).or_insert(kline);
        }

        let mut latest_x = 0;
        let (mut scale_high, mut scale_low) = (0.0f32, f32::MAX);
        data_points.iter().rev().for_each(|(time, kline)| {
            scale_high = scale_high.max(kline.high);
            scale_low = scale_low.min(kline.low);

            latest_x = latest_x.max(*time);
        });

        let y_ticks = (scale_high - scale_low) / tick_size;

        CandlestickChart {
            chart: CommonChartData {
                cell_width: Self::DEFAULT_CELL_WIDTH,
                cell_height: 200.0 / y_ticks,
                base_range: 100.0 / y_ticks,
                base_price_y,
                latest_x,
                timeframe: timeframe.to_milliseconds(),
                tick_size,
                timezone,
                indicators_height: 30,
                decimals: count_decimals(tick_size),
                ..Default::default()
            },
            data_points,
            indicators: vec![
                Indicators::Volume(Caches::default(), volume_data.clone()),
                Indicators::OpenInterest(Caches::default(), BTreeMap::new()),
            ],
            request_handler: RequestHandler::new(),
            fetching_oi: false,
        }
    }

    pub fn change_timezone(&mut self, timezone: UserTimezone) {
        let chart = self.get_common_data_mut();
        chart.timezone = timezone;
    }

    pub fn get_tick_size(&self) -> f32 {
        self.chart.tick_size
    }

    pub fn update_latest_kline(&mut self, kline: &Kline) -> Task<Message> {
        self.data_points.insert(kline.time as i64, *kline);

        self.indicators.iter_mut().for_each(|indicator| {
            if let Indicators::Volume(_, data) = indicator {
                data.insert(kline.time as i64, (kline.volume.0, kline.volume.1));
            }
        });

        let chart = self.get_common_data_mut();

        if (kline.time as i64) > chart.latest_x {
            chart.latest_x = kline.time as i64;
        }

        chart.last_price = if kline.close > kline.open {
            Some(PriceInfoLabel::Up(kline.close))
        } else {
            Some(PriceInfoLabel::Down(kline.close))
        };
        
        if !chart.already_fetching {
            return self.get_missing_data_task();
        }

        self.render_start();
        Task::none()
    }

    fn get_missing_data_task(&mut self) -> Task<Message> {
        let mut task = Task::none();

        let (visible_earliest, visible_latest) = self.get_visible_timerange();
        let (kline_earliest, kline_latest) = self.get_kline_timerange();

        let earliest = visible_earliest - (visible_latest - visible_earliest);

        if visible_earliest < kline_earliest {
            let latest = kline_earliest;

            if let Some(task) = request_fetch(
                &mut self.request_handler, FetchRange::Kline(earliest, latest)
            ) {
                self.get_common_data_mut().already_fetching = true;
                return task;
            }
        }

        for indicator in &self.indicators {
            if let Indicators::OpenInterest(_, _) = indicator {
                if !self.fetching_oi {
                    let (oi_earliest, oi_latest) = self.get_oi_timerange(kline_latest);

                    if visible_earliest < oi_earliest {
                        let latest = oi_earliest;

                        if let Some(fetch_task) = request_fetch(
                            &mut self.request_handler, FetchRange::OpenInterest(earliest, latest)
                        ) {
                            self.fetching_oi = true;
                            task = fetch_task;
                        }
                    } else if oi_latest < kline_latest {
                        let latest = visible_latest;

                        if let Some(fetch_task) = request_fetch(
                            &mut self.request_handler, FetchRange::OpenInterest(oi_latest, latest)
                        ) {
                            self.fetching_oi = true;
                            task = fetch_task;
                        }
                    }
                }
            }
        };

        self.render_start();
        task
    }

    pub fn insert_new_klines(&mut self, req_id: uuid::Uuid, klines_raw: &Vec<Kline>) {
        let mut volume_data = BTreeMap::new();

        for kline in klines_raw {
            volume_data.insert(kline.time as i64, (kline.volume.0, kline.volume.1));
            self.data_points.entry(kline.time as i64).or_insert(*kline);
        }

        self.indicators.iter_mut().for_each(|indicator| {
            if let Indicators::Volume(_, data) = indicator {
                data.extend(volume_data.clone());
            }
        });

        if klines_raw.len() > 1 {
            self.request_handler.mark_completed(req_id);
        } else {
            self.request_handler
                .mark_failed(req_id, "No data received".to_string());
        }

        self.get_common_data_mut().already_fetching = false;

        self.render_start();
    }

    pub fn insert_open_interest(&mut self, _req_id: Option<uuid::Uuid>, oi_data: Vec<OIData>) {
        self.indicators.iter_mut().for_each(|indicator| {
            if let Indicators::OpenInterest(_, data) = indicator {
                data.extend(oi_data
                    .iter().map(|oi| (oi.time, oi.value))
                );
            }
        });
    
        self.fetching_oi = false;
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

        self.indicators.iter().for_each(|indicator| {
            if let Indicators::OpenInterest(_, data) = indicator {
                data.iter().for_each(|(time, _)| {
                    from_time = from_time.min(*time);
                    to_time = to_time.max(*time);
                });
            }
        });

        (from_time, to_time)
    }

    fn render_start(&mut self) {
        let chart_state = &mut self.chart;

        if chart_state.autoscale {
            chart_state.translation =
                Vector::new(0.4 * chart_state.bounds.width / chart_state.scaling, {
                    if let Some((_, kline)) = self.data_points.last_key_value() {
                        let y_low = chart_state.price_to_y(kline.low);
                        let y_high = chart_state.price_to_y(kline.high);

                        -(y_low + y_high) / 2.0
                    } else {
                        0.0
                    }
                });
        }

        chart_state.cache.clear_all();

        self.indicators.iter_mut().for_each(|indicator| {
            indicator.clear_cache();
        });
    }

    fn get_volume_indicator(&self) -> Option<(&Caches, &BTreeMap<i64, (f32, f32)>)> {
        for indicator in &self.indicators {
            if let Indicators::Volume(cache, data) = indicator {
                return Some((cache, data));
            }
        }

        None
    }

    fn get_oi_indicator(&self) -> Option<(&Caches, &BTreeMap<i64, f32>)> {
        for indicator in &self.indicators {
            if let Indicators::OpenInterest(cache, data) = indicator {
                return Some((cache, data));
            }
        }

        None
    }

    pub fn view_indicators<I: Indicator>(&self, enabled: &[I]) -> Element<Message> {
        let chart_state: &CommonChartData = self.get_common_data();

        let visible_region = chart_state.visible_region(chart_state.bounds.size());

        let earliest = chart_state.x_to_time(visible_region.x);
        let latest = chart_state.x_to_time(visible_region.x + visible_region.width);

        let mut indicators: iced::widget::Column<'_, Message> = column![];

        for indicator in I::get_enabled(enabled) {
            if let Some(candlestick_indicator) = indicator
                .as_any()
                .downcast_ref::<CandlestickIndicator>() 
            {
                match candlestick_indicator {
                    CandlestickIndicator::Volume => {
                        if let Some((cache, data)) = self.get_volume_indicator() {
                            indicators = indicators.push(
                                indicators::volume::create_indicator_elem(chart_state, cache, data, earliest, latest)
                            );
                        }
                    },
                    CandlestickIndicator::OpenInterest => {
                        if let Some((cache, data)) = self.get_oi_indicator() {
                            indicators = indicators.push(
                                indicators::open_interest::create_indicator_elem(chart_state, cache, data, earliest, latest)
                            );
                        }
                    }
                }
            }
        }

        indicators.into()
    }

    pub fn update(&mut self, message: &Message) -> Task<Message> {
        self.update_chart(message)
    }

    pub fn view<'a, I: Indicator>(&'a self, indicators: &'a [I]) -> Element<Message> {
        view_chart(self, indicators)
    }
}

impl canvas::Program<Message> for CandlestickChart {
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

        let candlesticks = chart.cache.main.draw(renderer, bounds_size, |frame| {
            frame.with_save(|frame| {
                frame.translate(center);
                frame.scale(chart.scaling);
                frame.translate(chart.translation);

                let region = chart.visible_region(frame.size());

                let earliest = chart.x_to_time(region.x);
                let latest = chart.x_to_time(region.x + region.width);

                let candle_width = chart.cell_width * 0.8;

                self.data_points.range(earliest..=latest)
                    .for_each(|(timestamp, kline)| {
                        let x_position = chart.time_to_x(*timestamp);

                        let y_open = chart.price_to_y(kline.open);
                        let y_high = chart.price_to_y(kline.high);
                        let y_low = chart.price_to_y(kline.low);
                        let y_close = chart.price_to_y(kline.close);

                        let body_color = if kline.close >= kline.open {
                            palette.success.base.color
                        } else {
                            palette.danger.base.color
                        };
                        frame.fill_rectangle(
                            Point::new(x_position - (candle_width / 2.0), y_open.min(y_close)),
                            Size::new(candle_width, (y_open - y_close).abs()),
                            body_color,
                        );

                        let wick_color = if kline.close >= kline.open {
                            palette.success.base.color
                        } else {
                            palette.danger.base.color
                        };
                        frame.fill_rectangle(
                            Point::new(x_position - (candle_width / 8.0), y_high),
                            Size::new(candle_width / 4.0, (y_high - y_low).abs()),
                            wick_color,
                        );
                    });

                // last price line
                chart.last_price.map(|price| {
                    let (line_color, y_pos) = match price {
                        PriceInfoLabel::Up(p) => (palette.success.weak.color, chart.price_to_y(p)),
                        PriceInfoLabel::Down(p) => (palette.danger.weak.color, chart.price_to_y(p)),
                    };

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
                });
            });
        });

        if chart.crosshair {
            let crosshair = chart.cache.crosshair.draw(renderer, bounds_size, |frame| {
                if let Some(cursor_position) = cursor.position_in(bounds) {
                    let (_, rounded_timestamp) =
                        chart.draw_crosshair(frame, theme, bounds_size, cursor_position);

                    if let Some((_, kline)) = self
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

            vec![candlesticks, crosshair]
        } else {
            vec![candlesticks]
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
