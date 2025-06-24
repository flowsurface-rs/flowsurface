use std::time::Instant;

use super::Message;
use crate::style;
pub use data::chart::timeandsales::Config;
use data::config::theme::{darken, lighten};
use exchange::adapter::MarketKind;
use exchange::{TickerInfo, Trade};

use iced::widget::canvas;
use iced::{Alignment, Event, Point, Rectangle, Renderer, Size, Theme, mouse};

const TEXT_SIZE: iced::Pixels = iced::Pixels(11.0);
const HISTOGRAM_HEIGHT: f32 = 12.0;

struct TradeDisplay {
    time_str: String,
    price: f32,
    qty: f32,
    is_sell: bool,
}

const TRADE_ROW_HEIGHT: f32 = 14.0;

impl super::Panel for TimeAndSales {
    fn scroll(&mut self, delta: f32) {
        self.scroll_offset -= delta;

        let total_content_height =
            (self.recent_trades.len() as f32 * TRADE_ROW_HEIGHT) + HISTOGRAM_HEIGHT;
        let max_scroll_offset = (total_content_height - TRADE_ROW_HEIGHT).max(0.0);

        self.scroll_offset = self.scroll_offset.clamp(0.0, max_scroll_offset);

        if self.scroll_offset > HISTOGRAM_HEIGHT + TRADE_ROW_HEIGHT {
            self.is_paused = true;
        } else if self.is_paused {
            self.is_paused = false;
            self.recent_trades.append(&mut self.paused_trades_buffer);
        }

        self.invalidate();
    }

    fn reset_scroll_position(&mut self) {
        self.scroll_offset = 0.0;
        self.is_paused = false;

        self.recent_trades.append(&mut self.paused_trades_buffer);

        self.invalidate();
    }
}

pub struct TimeAndSales {
    recent_trades: Vec<TradeDisplay>,
    paused_trades_buffer: Vec<TradeDisplay>,
    is_paused: bool,
    max_filtered_qty: f32,
    ticker_info: Option<TickerInfo>,
    pub config: Config,
    cache: canvas::Cache,
    last_tick: Instant,
    scroll_offset: f32,
}

impl TimeAndSales {
    pub fn new(config: Option<Config>, ticker_info: Option<TickerInfo>) -> Self {
        Self {
            recent_trades: Vec::new(),
            paused_trades_buffer: Vec::new(),
            is_paused: false,
            config: config.unwrap_or_default(),
            max_filtered_qty: 0.0,
            ticker_info,
            cache: canvas::Cache::default(),
            last_tick: Instant::now(),
            scroll_offset: 0.0,
        }
    }

    pub fn insert_buffer(&mut self, trades_buffer: &[Trade]) {
        let size_filter = self.config.trade_size_filter;

        let market_type = match self.ticker_info {
            Some(ref ticker_info) => ticker_info.market_type(),
            None => return,
        };

        let target_buffer = if self.is_paused {
            &mut self.paused_trades_buffer
        } else {
            &mut self.recent_trades
        };

        for trade in trades_buffer {
            if let Some(trade_time) = chrono::DateTime::from_timestamp(
                trade.time as i64 / 1000,
                (trade.time % 1000) as u32 * 1_000_000,
            ) {
                let converted_trade = TradeDisplay {
                    time_str: trade_time.format("%M:%S.%3f").to_string(),
                    price: trade.price,
                    qty: trade.qty,
                    is_sell: trade.is_sell,
                };

                let trade_size = match market_type {
                    MarketKind::InversePerps => converted_trade.qty,
                    _ => converted_trade.qty * converted_trade.price,
                };

                if trade_size >= size_filter {
                    self.max_filtered_qty = self.max_filtered_qty.max(converted_trade.qty);
                }

                target_buffer.push(converted_trade);
            }
        }

        if !self.is_paused {
            let buffer_filter = self.config.buffer_filter;

            if self.recent_trades.len() > buffer_filter {
                let drain_amount = self.recent_trades.len() - (buffer_filter as f32 * 0.8) as usize;

                self.max_filtered_qty = self.recent_trades[drain_amount..]
                    .iter()
                    .filter(|t| {
                        let trade_size = match market_type {
                            MarketKind::InversePerps => t.qty,
                            _ => t.qty * t.price,
                        };
                        trade_size >= size_filter
                    })
                    .map(|t| t.qty)
                    .fold(0.0, f32::max);

                self.recent_trades.drain(0..drain_amount);
            }
        }
    }

    pub fn last_update(&self) -> Instant {
        self.last_tick
    }

    pub fn invalidate(&mut self) {
        self.cache.clear();
        self.last_tick = Instant::now();
    }
}

impl canvas::Program<Message> for TimeAndSales {
    type State = ();

    fn update(
        &self,
        _state: &mut Self::State,
        event: &iced::Event,
        bounds: iced::Rectangle,
        cursor: iced_core::mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let cursor_position = cursor.position_in(bounds)?;

        let paused_box = Rectangle {
            x: 0.0,
            y: 0.0,
            width: bounds.width,
            height: HISTOGRAM_HEIGHT + TRADE_ROW_HEIGHT,
        };

        match event {
            Event::Mouse(mouse_event) => match mouse_event {
                mouse::Event::ButtonPressed(button) => match button {
                    mouse::Button::Middle => {
                        Some(canvas::Action::publish(Message::ResetScroll).and_capture())
                    }
                    mouse::Button::Left => {
                        if self.is_paused && paused_box.contains(cursor_position) {
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
                    if self.is_paused {
                        Some(canvas::Action::publish(Message::Scrolled(0.0)).and_capture())
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
        let market_type = match self.ticker_info {
            Some(ref ticker_info) => ticker_info.market_type(),
            None => return vec![],
        };

        let palette = theme.extended_palette();

        let is_scroll_paused = self.is_paused;

        let content = self.cache.draw(renderer, bounds.size(), |frame| {
            let content_top_y = -self.scroll_offset;

            // Histogram
            let (buy_count, sell_count) =
                self.recent_trades.iter().fold((0, 0), |(buy, sell), t| {
                    if t.is_sell {
                        (buy, sell + 1)
                    } else {
                        (buy + 1, sell)
                    }
                });
            let total_count = buy_count + sell_count;

            if total_count > 0 {
                let buy_bar_width =
                    (bounds.width * (buy_count as f32 / total_count as f32)).max(1.0);
                let sell_bar_width =
                    (bounds.width * (sell_count as f32 / total_count as f32)).max(1.0);

                frame.fill_rectangle(
                    Point {
                        x: 0.0,
                        y: content_top_y,
                    },
                    Size {
                        width: buy_bar_width,
                        height: HISTOGRAM_HEIGHT,
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
                        height: HISTOGRAM_HEIGHT,
                    },
                    palette.danger.weak.color,
                );
            }

            // Feed
            let row_height = TRADE_ROW_HEIGHT;

            let row_scroll_offset = (self.scroll_offset - HISTOGRAM_HEIGHT).max(0.0);
            let start_index = (row_scroll_offset / row_height).floor() as usize;
            let visible_rows = (bounds.height / row_height).ceil() as usize;

            let trades_to_draw = self
                .recent_trades
                .iter()
                .filter(|t| {
                    let trade_size = match market_type {
                        MarketKind::InversePerps => t.qty,
                        _ => t.qty * t.price,
                    };
                    trade_size >= self.config.trade_size_filter
                })
                .rev()
                .skip(start_index)
                .take(visible_rows + 2);

            for (i, trade) in trades_to_draw.enumerate() {
                let y_position =
                    content_top_y + HISTOGRAM_HEIGHT + ((start_index + i) as f32 * row_height);

                if y_position + row_height < 0.0 || y_position > bounds.height {
                    continue;
                }

                let (bg_color, base_text_color) = if trade.is_sell {
                    (palette.danger.base.color, palette.danger.strong.color)
                } else {
                    (palette.success.base.color, palette.success.strong.color)
                };

                let row_bg_color_alpha = (trade.qty / self.max_filtered_qty).clamp(0.05, 1.0);

                let mut text_color = if palette.is_dark {
                    lighten(base_text_color, row_bg_color_alpha * 0.5)
                } else {
                    darken(base_text_color, row_bg_color_alpha * 0.5)
                };

                if is_scroll_paused && y_position < HISTOGRAM_HEIGHT + (TRADE_ROW_HEIGHT * 0.8) {
                    text_color = text_color.scale_alpha(0.2);
                }

                frame.fill_rectangle(
                    Point {
                        x: 0.0,
                        y: y_position,
                    },
                    Size {
                        width: frame.width(),
                        height: row_height,
                    },
                    bg_color.scale_alpha(row_bg_color_alpha),
                );

                frame.fill_text(iced::widget::canvas::Text {
                    content: trade.time_str.clone(),
                    position: Point {
                        x: frame.width() * 0.1,
                        y: y_position,
                    },
                    size: TEXT_SIZE,
                    font: style::AZERET_MONO,
                    color: text_color,
                    align_x: Alignment::Start.into(),
                    ..Default::default()
                });

                frame.fill_text(iced::widget::canvas::Text {
                    content: trade.price.to_string(),
                    position: Point {
                        x: frame.width() * 0.65,
                        y: y_position,
                    },
                    size: TEXT_SIZE,
                    font: style::AZERET_MONO,
                    color: text_color,
                    align_x: Alignment::End.into(),
                    ..Default::default()
                });

                frame.fill_text(iced::widget::canvas::Text {
                    content: data::util::abbr_large_numbers(trade.qty),
                    position: Point {
                        x: frame.width() * 0.9,
                        y: y_position,
                    },
                    size: TEXT_SIZE,
                    font: style::AZERET_MONO,
                    color: text_color,
                    align_x: Alignment::End.into(),
                    ..Default::default()
                });
            }

            if is_scroll_paused {
                let pause_box_height = HISTOGRAM_HEIGHT + TRADE_ROW_HEIGHT;
                let pause_box_y = 0.0;

                let cursor_position = cursor.position_in(bounds);

                let paused_box = Rectangle {
                    x: 0.0,
                    y: pause_box_y,
                    width: frame.width(),
                    height: pause_box_height,
                };

                let bg_color = if let Some(cursor) = cursor_position {
                    if paused_box.contains(cursor) {
                        palette.background.strong.color
                    } else {
                        palette.background.weak.color
                    }
                } else {
                    palette.background.strong.color
                };

                frame.fill_rectangle(
                    Point {
                        x: 0.0,
                        y: pause_box_y,
                    },
                    Size {
                        width: frame.width(),
                        height: pause_box_height,
                    },
                    bg_color,
                );

                frame.fill_text(iced::widget::canvas::Text {
                    content: "Paused".to_string(),
                    position: Point {
                        x: frame.width() * 0.5,
                        y: pause_box_y + (pause_box_height / 2.0),
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
        if self.is_paused {
            let paused_box = Rectangle {
                x: bounds.x,
                y: bounds.y,
                width: bounds.width,
                height: HISTOGRAM_HEIGHT + TRADE_ROW_HEIGHT,
            };

            if cursor.is_over(paused_box) {
                return mouse::Interaction::Pointer;
            }
        }

        mouse::Interaction::default()
    }
}
