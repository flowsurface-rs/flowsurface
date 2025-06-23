use crate::modal::{self, pane::settings::timesales_cfg_view};
use crate::screen::dashboard::pane::{self, Message};
use crate::style;
use data::UserTimezone;
pub use data::chart::timeandsales::Config;
use data::config::theme::{darken, lighten};
use exchange::adapter::MarketKind;
use exchange::{TickerInfo, Trade};

use iced::widget::canvas;
use iced::{Alignment, Element, Point, Rectangle, Renderer, Size, Theme, mouse, padding};

const TEXT_SIZE: iced::Pixels = iced::Pixels(11.0);

struct TradeDisplay {
    time_str: String,
    price: f32,
    qty: f32,
    is_sell: bool,
}

const TRADE_ROW_HEIGHT: f32 = 14.0;

impl super::PanelView for TimeAndSales {
    fn view(
        &self,
        pane: iced::widget::pane_grid::Pane,
        state: &pane::State,
        timezone: data::UserTimezone,
    ) -> Element<Message> {
        let underlay = self.view(timezone);

        let settings_view = timesales_cfg_view(self.config, pane);

        match state.modal {
            Some(pane::Modal::Settings) => modal::pane::stack_modal(
                underlay,
                settings_view,
                Message::ShowModal(pane, pane::Modal::Settings),
                padding::right(12).left(12),
                Alignment::End,
            ),
            _ => underlay,
        }
    }
}

pub struct TimeAndSales {
    recent_trades: Vec<TradeDisplay>,
    max_filtered_qty: f32,
    ticker_info: Option<TickerInfo>,
    pub config: Config,
    rows_cache: canvas::Cache,
    histogram_cache: canvas::Cache,
}

impl TimeAndSales {
    pub fn new(config: Option<Config>, ticker_info: Option<TickerInfo>) -> Self {
        Self {
            recent_trades: Vec::new(),
            config: config.unwrap_or_default(),
            max_filtered_qty: 0.0,
            ticker_info,
            rows_cache: canvas::Cache::default(),
            histogram_cache: canvas::Cache::default(),
        }
    }

    pub fn insert_buffer(&mut self, trades_buffer: &[Trade]) {
        let size_filter = self.config.trade_size_filter;

        let market_type = match self.ticker_info {
            Some(ref ticker_info) => ticker_info.market_type(),
            None => return,
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

                self.recent_trades.push(converted_trade);
            }
        }

        let buffer_filter = self.config.buffer_filter;

        if self.recent_trades.len() > buffer_filter {
            let drain_amount = self.recent_trades.len() - (buffer_filter as f32 * 0.8) as usize;

            self.max_filtered_qty = self.recent_trades[drain_amount..]
                .iter()
                .filter(|t| (t.qty * t.price) >= size_filter)
                .map(|t| t.qty)
                .fold(0.0, f32::max);

            self.recent_trades.drain(0..drain_amount);
        }

        self.rows_cache.clear();
        self.histogram_cache.clear();
    }

    pub fn view(&self, _timezone: UserTimezone) -> Element<'_, Message> {
        canvas(self)
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .into()
    }
}

impl canvas::Program<Message> for TimeAndSales {
    type State = ();

    fn update(
        &self,
        _state: &mut Self::State,
        _event: &iced::Event,
        _bounds: iced::Rectangle,
        _cursor: iced_core::mouse::Cursor,
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
    ) -> Vec<canvas::Geometry> {
        let market_type = match self.ticker_info {
            Some(ref ticker_info) => ticker_info.market_type(),
            None => return vec![],
        };

        let palette = theme.extended_palette();

        let histogram_bounds = Rectangle {
            x: 0.0,
            y: 0.0,
            width: bounds.width,
            height: 12.0,
        };

        let histogram = self.histogram_cache.draw(renderer, bounds.size(), |frame| {
            let buy_count = self.recent_trades.iter().filter(|t| !t.is_sell).count();
            let sell_count = self.recent_trades.iter().filter(|t| t.is_sell).count();

            let total_count = buy_count + sell_count;

            if total_count == 0 {
                return;
            }

            let buy_bar_width = (bounds.width * (buy_count as f32 / total_count as f32)).max(1.0);
            let sell_bar_width = (bounds.width * (sell_count as f32 / total_count as f32)).max(1.0);

            let bar_height = histogram_bounds.height;
            let bar_y = (histogram_bounds.height - bar_height) / 2.0;

            frame.fill_rectangle(
                Point { x: 0.0, y: bar_y },
                Size {
                    width: buy_bar_width,
                    height: bar_height,
                },
                palette.success.weak.color,
            );

            frame.fill_rectangle(
                Point {
                    x: buy_bar_width,
                    y: bar_y,
                },
                Size {
                    width: sell_bar_width,
                    height: bar_height,
                },
                palette.danger.weak.color,
            );
        });

        let rows_bounds = Rectangle {
            x: 0.0,
            y: histogram_bounds.height,
            width: bounds.width,
            height: (bounds.height - histogram_bounds.height),
        };

        let rows = self.rows_cache.draw(renderer, bounds.size(), |frame| {
            let row_height = TRADE_ROW_HEIGHT;
            let total_rows = (rows_bounds.height / row_height).floor() as usize;

            let filtered_trades_iter = self.recent_trades.iter().filter(|t| {
                let trade_size = match market_type {
                    MarketKind::InversePerps => t.qty,
                    _ => t.qty * t.price,
                };
                trade_size >= self.config.trade_size_filter
            });

            let trades_to_draw = filtered_trades_iter.rev().take(total_rows + 3);

            for (i, trade) in trades_to_draw.enumerate() {
                let y_position = rows_bounds.y + i as f32 * row_height;

                let (bg_color, base_text_color) = if trade.is_sell {
                    (palette.danger.base.color, palette.danger.strong.color)
                } else {
                    (palette.success.base.color, palette.success.strong.color)
                };

                let row_bg_color_alpha = (trade.qty / self.max_filtered_qty).clamp(0.05, 1.0);

                let text_color = if palette.is_dark {
                    lighten(base_text_color, row_bg_color_alpha * 0.5)
                } else {
                    darken(base_text_color, row_bg_color_alpha * 0.5)
                };

                frame.fill_rectangle(
                    Point {
                        x: rows_bounds.x,
                        y: y_position,
                    },
                    Size {
                        width: rows_bounds.width,
                        height: row_height,
                    },
                    bg_color.scale_alpha(row_bg_color_alpha),
                );

                frame.fill_text(iced::widget::canvas::Text {
                    content: trade.time_str.clone(),
                    position: Point {
                        x: rows_bounds.x + rows_bounds.width * 0.1,
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
                        x: rows_bounds.x + rows_bounds.width * 0.65,
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
                        x: rows_bounds.x + rows_bounds.width * 0.9,
                        y: y_position,
                    },
                    size: TEXT_SIZE,
                    font: style::AZERET_MONO,
                    color: text_color,
                    align_x: Alignment::End.into(),
                    ..Default::default()
                });
            }
        });

        vec![histogram, rows]
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        _bounds: iced::Rectangle,
        _cursor: iced_core::mouse::Cursor,
    ) -> iced_core::mouse::Interaction {
        iced_core::mouse::Interaction::default()
    }
}
