use chrono::DateTime;
use iced::{alignment, padding, Element, Length};
use iced::widget::{column, container, responsive, row, text, Space};
use serde::{Deserialize, Serialize};
use crate::screen::dashboard::pane::Message;
use crate::screen::UserTimezone;
use crate::style::ts_table_container;
use crate::data_providers::Trade;

struct ConvertedTrade {
    time_str: String,
    price: f32,
    qty: f32,
    is_sell: bool,
}
pub struct TimeAndSales {
    recent_trades: Vec<ConvertedTrade>,
    config: Config,
    max_filtered_qty: f32,
    max_size: usize,
    target_size: usize,
}

impl TimeAndSales {
    pub fn new(config: Option<Config>) -> Self {
        Self {
            recent_trades: Vec::new(),
            config: config.unwrap_or_default(),
            max_filtered_qty: 0.0,
            max_size: 900,
            target_size: 700,
        }
    }

    pub fn get_config(&self) -> Config {
        self.config.clone()
    }

    pub fn set_config(&mut self, cfg: Config) {
        self.config = cfg;
    }

    pub fn update(&mut self, trades_buffer: &[Trade]) {
        let size_filter = self.config.trade_size_filter;

        for trade in trades_buffer {
            if let Some(trade_time) =
                DateTime::from_timestamp(trade.time / 1000, (trade.time % 1000) as u32 * 1_000_000)
            {
                let converted_trade = ConvertedTrade {
                    time_str: trade_time.format("%M:%S.%3f").to_string(),
                    price: trade.price,
                    qty: trade.qty,
                    is_sell: trade.is_sell,
                };

                if (converted_trade.qty * converted_trade.price) >= size_filter {
                    self.max_filtered_qty = self.max_filtered_qty.max(converted_trade.qty);
                }

                self.recent_trades.push(converted_trade);
            }
        }

        if self.recent_trades.len() > self.max_size {
            let drain_amount = self.recent_trades.len() - self.target_size;

            self.max_filtered_qty = self.recent_trades[drain_amount..]
                .iter()
                .filter(|t| (t.qty * t.price) >= size_filter)
                .map(|t| t.qty)
                .fold(0.0, f32::max);

            self.recent_trades.drain(0..drain_amount);
        }
    }

    pub fn view(&self, _timezone: &UserTimezone) -> Element<'_, Message> {
        responsive(move |size| {
            let mut column = column![]
                .padding(padding::top(4).left(4).right(4))
                .height(Length::Fill);

            let row_height = 16.0;
            let rows_can_fit = size.height / row_height;

            let filtered_trades_iter = self
                .recent_trades
                .iter()
                .filter(|t| (t.qty * t.price) >= self.config.trade_size_filter);

            for trade in filtered_trades_iter.rev().take(rows_can_fit as usize) {
                column = column.push(container(Space::new(
                    Length::Fixed(0.0),
                    Length::Fixed(2.0),
                )));

                let trade_row = row![
                    container(text(&trade.time_str))
                        .width(Length::FillPortion(8))
                        .align_x(alignment::Horizontal::Center),
                    container(text(trade.price)).width(Length::FillPortion(6)),
                    container(text(trade.qty)).width(Length::FillPortion(4))
                ]
                .height(Length::Fixed(row_height));

                column = column.push(container(trade_row).style(move |theme| {
                    ts_table_container(theme, trade.is_sell, trade.qty / self.max_filtered_qty)
                }));
            }

            column.into()
        })
        .into()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub trade_size_filter: f32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            trade_size_filter: 0.0,
        }
    }
}