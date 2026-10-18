use crate::style;
use data::config::theme::{darken, lighten};
use data::config::timezone::TimeLabelKind;
pub use data::panel::timesales::Config;
use data::panel::timesales::{HistAgg, HistAggValues, StackedBar, TapeTrade, prune_trades};
use exchange::unit::Qty;
use exchange::{TickerInfo, Trade, UnixMs, unit::qty::volume_size_unit};

use iced::widget::canvas::{self, Canvas, Text};
use iced::{
    Alignment, Element, Event, Point, Rectangle, Renderer, Size, Theme, mouse, padding,
    widget::{center, container, text},
};
use std::collections::VecDeque;
use std::time::Instant;

const TEXT_SIZE: iced::Pixels = iced::Pixels(style::text_size::SMALL);
const METRICS_HEIGHT_COMPACT: f32 = 8.0;
const METRICS_HEIGHT_FULL: f32 = 18.0;
const TRADE_ROW_HEIGHT: f32 = 14.0;
#[derive(Debug, Clone, Copy)]
pub enum Message {
    Scrolled(f32),
    ResetScroll,
    Invalidate(Option<Instant>),
}

impl TimeAndSales {
    pub fn update(&mut self, message: Message) {
        match message {
            Message::Scrolled(delta) => self.scroll(delta),
            Message::ResetScroll => self.reset_scroll(),
            Message::Invalidate(now) => self.invalidate(now),
        }
    }

    fn scroll(&mut self, delta: f32) {
        self.scroll_offset -= delta;
        self.clamp_scroll_to_filter();
        self.sync_pause_with_scroll();
        self.invalidate(Some(Instant::now()));
    }

    fn sync_pause_with_scroll(&mut self) {
        if self.scroll_offset > self.stacked_bar_height() + TRADE_ROW_HEIGHT {
            self.is_paused = true;
        } else if self.is_paused {
            self.is_paused = false;

            self.resume_buffer();

            self.prune_by_time();
        }
    }

    fn reset_scroll(&mut self) {
        self.scroll_offset = 0.0;
        self.is_paused = false;

        self.resume_buffer();

        self.prune_by_time();

        self.invalidate(Some(Instant::now()));
    }

    fn is_empty(&self) -> bool {
        self.recent_trades.is_empty() && self.paused_trades_buffer.is_empty()
    }

    pub fn view<'a>(&'a self, timezone: data::UserTimezone) -> Element<'a, Message> {
        if self.is_empty() {
            return center(text("Waiting for data...").size(crate::style::text_size::TITLE)).into();
        }

        container(
            Canvas::new(TimeAndSalesView {
                timezone,
                panel: self,
            })
            .width(iced::Length::Fill)
            .height(iced::Length::Fill),
        )
        .padding(padding::left(1).right(1).bottom(1))
        .into()
    }
}

pub struct TimeAndSales {
    recent_trades: VecDeque<TapeTrade>,
    paused_trades_buffer: VecDeque<TapeTrade>,
    hist_agg: HistAgg,
    is_paused: bool,
    max_filtered_qty: Qty,
    multiple_tickers: bool,
    pub config: Config,
    cache: canvas::Cache,
    last_tick: Instant,
    scroll_offset: f32,
}

impl TimeAndSales {
    pub fn new(config: Option<Config>, ticker_count: usize) -> Self {
        Self {
            recent_trades: VecDeque::new(),
            paused_trades_buffer: VecDeque::new(),
            hist_agg: HistAgg::default(),
            is_paused: false,
            config: config.unwrap_or_default(),
            max_filtered_qty: Qty::ZERO,
            multiple_tickers: ticker_count > 1,
            cache: canvas::Cache::default(),
            last_tick: Instant::now(),
            scroll_offset: 0.0,
        }
    }

    pub fn insert_buffer(&mut self, ticker_info: TickerInfo, trades_buffer: &[Trade]) {
        let incoming: Vec<_> = trades_buffer
            .iter()
            .map(|trade| TapeTrade::new(ticker_info, *trade, volume_size_unit()))
            .collect();
        if self.is_paused {
            TapeTrade::merge_into(&mut self.paused_trades_buffer, incoming);
        } else {
            for entry in &incoming {
                let quantity = entry.quantity(self.multiple_tickers);
                self.hist_agg.add(entry.is_sell, quantity);
                if entry.passes_filter(self.config.trade_size_filter) {
                    self.max_filtered_qty = self.max_filtered_qty.max(quantity);
                }
            }
            TapeTrade::merge_into(&mut self.recent_trades, incoming);
            self.prune_by_time();
        }
        self.prune_paused_by_time();
    }

    fn resume_buffer(&mut self) {
        let incoming = self.paused_trades_buffer.drain(..).collect();
        TapeTrade::merge_into(&mut self.recent_trades, incoming);
        self.rebuild_metrics();
    }

    fn rebuild_metrics(&mut self) {
        self.hist_agg = HistAgg::default();
        self.max_filtered_qty = Qty::ZERO;
        let mut visible_trades = 0;
        for entry in &self.recent_trades {
            let quantity = entry.quantity(self.multiple_tickers);
            self.hist_agg.add(entry.is_sell, quantity);
            if entry.passes_filter(self.config.trade_size_filter) {
                visible_trades += 1;
                self.max_filtered_qty = self.max_filtered_qty.max(quantity);
            }
        }
        self.clamp_scroll(visible_trades);
        self.cache.clear();
    }

    fn clamp_scroll_to_filter(&mut self) {
        if self.scroll_offset <= 0.0 {
            self.scroll_offset = 0.0;
            return;
        }
        let rows_needed = ((self.scroll_offset - self.stacked_bar_height()).max(0.0)
            / TRADE_ROW_HEIGHT)
            .ceil() as usize;
        let visible_trades = self
            .recent_trades
            .iter()
            .rev()
            .filter(|entry| entry.passes_filter(self.config.trade_size_filter))
            .take(rows_needed.saturating_add(1))
            .count();
        self.clamp_scroll(visible_trades);
    }

    fn clamp_scroll(&mut self, visible_trades: usize) {
        let height = visible_trades as f32 * TRADE_ROW_HEIGHT + self.stacked_bar_height();
        self.scroll_offset = self
            .scroll_offset
            .clamp(0.0, (height - TRADE_ROW_HEIGHT).max(0.0));
    }

    pub fn set_sources(&mut self, tickers: &[TickerInfo]) {
        self.multiple_tickers = tickers.len() > 1;
        self.recent_trades.retain(|entry| {
            tickers
                .iter()
                .any(|info| info.ticker == entry.ticker_info().ticker)
        });
        self.paused_trades_buffer.retain(|entry| {
            tickers
                .iter()
                .any(|info| info.ticker == entry.ticker_info().ticker)
        });
        self.rebuild_metrics();
        self.sync_pause_with_scroll();
    }

    pub fn set_config(&mut self, config: Config) {
        let filter_changed = self.config.trade_size_filter != config.trade_size_filter;
        let previous_height = self.stacked_bar_height();
        self.config = config;
        let height_changed = self.stacked_bar_height() != previous_height;
        if filter_changed {
            self.rebuild_metrics();
        } else if height_changed {
            self.clamp_scroll_to_filter();
        }
        if filter_changed || height_changed {
            self.sync_pause_with_scroll();
        }
        self.invalidate(None);
    }

    pub fn last_update(&self) -> Instant {
        self.last_tick
    }

    pub fn invalidate(&mut self, now: Option<Instant>) {
        if !self.is_paused {
            self.prune_by_time();
        }
        self.prune_paused_by_time();

        self.cache.clear();
        if let Some(now) = now {
            self.last_tick = now;
        }
    }

    fn stacked_bar_height(&self) -> f32 {
        match &self.config.stacked_bar {
            Some(StackedBar::Compact(_)) => METRICS_HEIGHT_COMPACT,
            Some(StackedBar::Full(_)) => METRICS_HEIGHT_FULL,
            None => 0.0,
        }
    }

    fn pause_overlay_height(&self) -> f32 {
        self.stacked_bar_height().max(METRICS_HEIGHT_COMPACT) + TRADE_ROW_HEIGHT
    }

    fn prune_by_time(&mut self) {
        if prune_trades(
            &mut self.recent_trades,
            self.config.trade_retention,
            UnixMs::now(),
        ) {
            self.rebuild_metrics();
        }
    }

    fn prune_paused_by_time(&mut self) {
        prune_trades(
            &mut self.paused_trades_buffer,
            self.config.trade_retention,
            UnixMs::now(),
        );
    }
}

struct TimeAndSalesView<'a> {
    timezone: data::UserTimezone,
    panel: &'a TimeAndSales,
}

impl canvas::Program<Message> for TimeAndSalesView<'_> {
    type State = ();

    fn update(
        &self,
        _state: &mut Self::State,
        event: &iced::Event,
        bounds: iced::Rectangle,
        cursor: iced_core::mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let cursor_position = cursor.position_in(bounds)?;

        match event {
            Event::Mouse(mouse_event) => match mouse_event {
                mouse::Event::ButtonPressed(button) => match button {
                    mouse::Button::Middle => {
                        Some(canvas::Action::publish(Message::ResetScroll).and_capture())
                    }
                    mouse::Button::Left => {
                        let paused_box_height = self.panel.pause_overlay_height();
                        let paused_box = Rectangle {
                            x: 0.0,
                            y: 0.0,
                            width: bounds.width,
                            height: paused_box_height,
                        };

                        if self.panel.is_paused && paused_box.contains(cursor_position) {
                            Some(canvas::Action::publish(Message::ResetScroll).and_capture())
                        } else {
                            None
                        }
                    }
                    _ => None,
                },
                mouse::Event::WheelScrolled { delta } => {
                    let scroll_amount = match delta {
                        mouse::ScrollDelta::Lines { y, .. } => *y * TRADE_ROW_HEIGHT * 3.0,
                        mouse::ScrollDelta::Pixels { y, .. } => *y,
                    };

                    Some(canvas::Action::publish(Message::Scrolled(scroll_amount)).and_capture())
                }
                mouse::Event::CursorMoved { .. } => {
                    if self.panel.is_paused {
                        let now = Some(Instant::now());
                        Some(canvas::Action::publish(Message::Invalidate(now)).and_capture())
                    } else {
                        None
                    }
                }
                _ => None,
            },
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
    ) -> Vec<canvas::Geometry> {
        let palette = theme.extended_palette();
        let is_scroll_paused = self.panel.is_paused;
        let stacked_bar_h = self.panel.stacked_bar_height();

        let content = self.panel.cache.draw(renderer, bounds.size(), |frame| {
            let content_top_y = -self.panel.scroll_offset;

            if let Some(hist) = &self.panel.config.stacked_bar {
                let ratio_kind = match hist {
                    StackedBar::Compact(r) | StackedBar::Full(r) => *r,
                };

                if let Some(values) = self.panel.hist_agg.values_for(ratio_kind) {
                    let draw_stacked_bar =
                        |frame: &mut canvas::Frame, buy_bar_width: f32, sell_bar_width: f32| {
                            frame.fill_rectangle(
                                Point {
                                    x: 0.0,
                                    y: content_top_y,
                                },
                                Size {
                                    width: buy_bar_width,
                                    height: stacked_bar_h,
                                },
                                palette.success.weak.color,
                            );

                            frame.fill_rectangle(
                                Point {
                                    x: buy_bar_width,
                                    y: content_top_y,
                                },
                                Size {
                                    width: sell_bar_width,
                                    height: stacked_bar_h,
                                },
                                palette.danger.weak.color,
                            );
                        };

                    let (buy_ratio, buy_text_content, sell_text_content) = match values {
                        HistAggValues::Count { buy, sell } => {
                            let total = buy.saturating_add(sell) as f32;
                            let buy_ratio = if total > 0.0 { buy as f32 / total } else { 0.5 };
                            (buy_ratio, buy.to_string(), sell.to_string())
                        }
                        HistAggValues::Qty { buy, sell } => {
                            let buy_val = buy.to_f32_lossy();
                            let sell_val = sell.to_f32_lossy();
                            let total = buy_val + sell_val;
                            let buy_ratio = if total > 0.0 { buy_val / total } else { 0.5 };

                            (
                                buy_ratio,
                                data::util::abbr_large_numbers(buy_val as f64),
                                data::util::abbr_large_numbers(sell_val as f64),
                            )
                        }
                    };

                    let buy_bar_width = (bounds.width * buy_ratio).round();
                    let sell_bar_width = bounds.width - buy_bar_width;

                    draw_stacked_bar(frame, buy_bar_width, sell_bar_width);

                    if matches!(hist, StackedBar::Full(_)) {
                        let center_y = content_top_y + (stacked_bar_h / 2.0);

                        let buy_text = Text {
                            content: buy_text_content,
                            position: Point {
                                x: 8.0,
                                y: center_y,
                            },
                            size: TEXT_SIZE,
                            font: style::AZERET_MONO,
                            color: palette.success.weak.text,
                            align_x: Alignment::Start.into(),
                            align_y: Alignment::Center.into(),
                            ..Default::default()
                        };
                        frame.fill_text(buy_text);

                        let sell_text = Text {
                            content: sell_text_content,
                            position: Point {
                                x: bounds.width - 8.0,
                                y: center_y,
                            },
                            size: TEXT_SIZE,
                            font: style::AZERET_MONO,
                            color: palette.danger.weak.text,
                            align_x: Alignment::End.into(),
                            align_y: Alignment::Center.into(),
                            ..Default::default()
                        };
                        frame.fill_text(sell_text);
                    }
                }
            }

            // Feed
            let row_height = TRADE_ROW_HEIGHT;
            let row_width = bounds.width;

            let row_scroll_offset = (self.panel.scroll_offset - stacked_bar_h).max(0.0);
            let start_index = (row_scroll_offset / row_height).floor() as usize;
            let visible_rows = (bounds.height / row_height).ceil() as usize;

            let max_filtered_qty_f32 = self.panel.max_filtered_qty.to_f32_lossy();

            let trades_to_draw = self
                .panel
                .recent_trades
                .iter()
                .filter(|entry| entry.passes_filter(self.panel.config.trade_size_filter))
                .rev()
                .skip(start_index)
                .take(visible_rows + 2);

            let create_text =
                |content: String, position: Point, align_x: Alignment, color: iced::Color| Text {
                    content,
                    position,
                    size: TEXT_SIZE,
                    font: style::AZERET_MONO,
                    color,
                    align_x: align_x.into(),
                    ..Default::default()
                };

            for (i, entry) in trades_to_draw.enumerate() {
                let y_position =
                    content_top_y + stacked_bar_h + ((start_index + i) as f32 * row_height);

                if y_position + row_height < 0.0 || y_position > bounds.height {
                    continue;
                }

                let bg_color = if entry.is_sell {
                    palette.danger.weak.color
                } else {
                    palette.success.weak.color
                };

                let bg_color_alpha = if max_filtered_qty_f32 > 0.0 {
                    (entry.quantity(self.panel.multiple_tickers).to_f32_lossy()
                        / max_filtered_qty_f32)
                        .clamp(0.02, 1.0)
                } else {
                    0.02
                };

                let mut text_color = if palette.is_dark {
                    lighten(bg_color, bg_color_alpha.max(0.1))
                } else {
                    darken(bg_color, (bg_color_alpha * 0.8).max(0.1))
                };

                if is_scroll_paused
                    && y_position
                        < (stacked_bar_h.max(METRICS_HEIGHT_COMPACT)) + (TRADE_ROW_HEIGHT * 0.8)
                {
                    text_color = text_color.scale_alpha(0.1);
                }

                frame.fill_rectangle(
                    Point {
                        x: 0.0,
                        y: y_position,
                    },
                    Size {
                        width: row_width,
                        height: row_height,
                    },
                    bg_color.scale_alpha(bg_color_alpha.min(0.9)),
                );

                let trade_time = create_text(
                    self.timezone
                        .format_with_kind(
                            entry.time.as_u64() as i64,
                            TimeLabelKind::Custom("%M:%S.%3f"),
                        )
                        .unwrap_or_default(),
                    Point {
                        x: row_width * 0.1,
                        y: y_position,
                    },
                    Alignment::Start,
                    text_color,
                );
                frame.fill_text(trade_time);

                let trade_price = create_text(
                    entry.price.to_string(entry.ticker_info().min_ticksize),
                    Point {
                        x: row_width * 0.67,
                        y: y_position,
                    },
                    Alignment::End,
                    text_color,
                );
                frame.fill_text(trade_price);

                let trade_qty = create_text(
                    data::util::abbr_large_numbers(
                        entry.quantity(self.panel.multiple_tickers).to_f64(),
                    ),
                    Point {
                        x: row_width * 0.9,
                        y: y_position,
                    },
                    Alignment::End,
                    text_color,
                );
                frame.fill_text(trade_qty);
            }

            if is_scroll_paused {
                let pause_overlay_height = self.panel.pause_overlay_height();
                let pause_overlay_y = 0.0;

                let cursor_position = cursor.position_in(bounds);

                let paused_box = Rectangle {
                    x: 0.0,
                    y: pause_overlay_y,
                    width: frame.width(),
                    height: pause_overlay_height,
                };

                let bg_color = if let Some(cursor) = cursor_position {
                    if paused_box.contains(cursor) {
                        palette.background.strong.color
                    } else {
                        palette.background.weak.color
                    }
                } else {
                    palette.background.weak.color
                };

                frame.fill_rectangle(
                    Point {
                        x: 0.0,
                        y: pause_overlay_y,
                    },
                    Size {
                        width: frame.width(),
                        height: pause_overlay_height,
                    },
                    bg_color,
                );

                frame.fill_text(Text {
                    content: "Paused".to_string(),
                    position: Point {
                        x: frame.width() * 0.5,
                        y: pause_overlay_y + (pause_overlay_height / 2.0),
                    },
                    size: 12.0.into(),
                    font: style::AZERET_MONO,
                    color: palette.background.strong.text,
                    align_x: Alignment::Center.into(),
                    align_y: Alignment::Center.into(),
                    ..Default::default()
                });
            }
        });

        vec![content]
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        bounds: iced::Rectangle,
        cursor: iced_core::mouse::Cursor,
    ) -> iced_core::mouse::Interaction {
        if self.panel.is_paused {
            let stacked_bar_h = self.panel.stacked_bar_height();
            let paused_box = Rectangle {
                x: bounds.x,
                y: bounds.y,
                width: bounds.width,
                height: stacked_bar_h.max(METRICS_HEIGHT_COMPACT) + TRADE_ROW_HEIGHT,
            };

            if cursor.is_over(paused_box) {
                return mouse::Interaction::Pointer;
            }
        }

        mouse::Interaction::default()
    }
}
