use crate::screen::dashboard::pane::Message;
use crate::style::{self, ts_table_container};
use data::UserTimezone;
use data::chart::timeandsales::Config;
use exchange::adapter::MarketType;
use exchange::{TickerInfo, Trade};
use iced::widget::{center, column, container, responsive, row, text};
use iced::{Alignment, Element, Length};

use super::abbr_large_numbers;

const TARGET_SIZE: usize = 700;
const MAX_SIZE: usize = 900;

struct TradeDisplay {
    time_str: String,
    price: f32,
    qty: f32,
    is_sell: bool,
}

const TRADE_ROW_HEIGHT: f32 = 14.0;

pub struct TimeAndSales {
    recent_trades: Vec<TradeDisplay>,
    config: Config,
    max_filtered_qty: f32,
    ticker_info: Option<TickerInfo>,
}

impl TimeAndSales {
    pub fn new(config: Option<Config>, ticker_info: Option<TickerInfo>) -> Self {
        Self {
            recent_trades: Vec::new(),
            config: config.unwrap_or_default(),
            max_filtered_qty: 0.0,
            ticker_info,
        }
    }

    pub fn get_config(&self) -> Config {
        self.config
    }

    pub fn set_config(&mut self, cfg: Config) {
        self.config = cfg;
    }

    pub fn update(&mut self, trades_buffer: &[Trade]) {
        let size_filter = self.config.trade_size_filter;

        let market_type = match self.ticker_info {
            Some(ref ticker_info) => ticker_info.get_market_type(),
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
                    MarketType::InversePerps => converted_trade.qty,
                    _ => converted_trade.qty * converted_trade.price,
                };

                if trade_size >= size_filter {
                    self.max_filtered_qty = self.max_filtered_qty.max(converted_trade.qty);
                }

                self.recent_trades.push(converted_trade);
            }
        }

        if self.recent_trades.len() > MAX_SIZE {
            let drain_amount = self.recent_trades.len() - TARGET_SIZE;

            self.max_filtered_qty = self.recent_trades[drain_amount..]
                .iter()
                .filter(|t| (t.qty * t.price) >= size_filter)
                .map(|t| t.qty)
                .fold(0.0, f32::max);

            self.recent_trades.drain(0..drain_amount);
        }
    }

    pub fn view(&self, _timezone: UserTimezone) -> Element<'_, Message> {
        responsive(move |size| {
            let market_type = match self.ticker_info {
                Some(ref ticker_info) => ticker_info.get_market_type(),
                None => {
                    return center(container(
                        text("No ticker info. Resetting this pane should fix").size(14),
                    ))
                    .into();
                }
            };

            let mut content = column![].padding(4);

            let rows_can_fit = ((size.height / TRADE_ROW_HEIGHT).floor()) as usize;

            let filtered_trades_iter = self.recent_trades.iter().filter(|t| {
                let trade_size = match market_type {
                    MarketType::InversePerps => t.qty,
                    _ => t.qty * t.price,
                };
                trade_size >= self.config.trade_size_filter
            });

            for trade in filtered_trades_iter.rev().take(rows_can_fit) {
                let trade_row = row![
                    container(
                        text(&trade.time_str)
                            .font(style::AZERET_MONO)
                            .size(iced::Pixels(11.0))
                    )
                    .width(Length::FillPortion(8))
                    .align_x(Alignment::Center),
                    container(
                        text(trade.price)
                            .font(style::AZERET_MONO)
                            .size(iced::Pixels(11.0))
                    )
                    .width(Length::FillPortion(6)),
                    container(
                        text(abbr_large_numbers(trade.qty))
                            .font(style::AZERET_MONO)
                            .size(iced::Pixels(11.0))
                    )
                    .width(Length::FillPortion(4))
                ]
                .align_y(Alignment::Center)
                .height(Length::Fixed(TRADE_ROW_HEIGHT));

                content = content.push(container(trade_row).padding(1).style(move |theme| {
                    ts_table_container(theme, trade.is_sell, trade.qty / self.max_filtered_qty)
                }));
            }

            content.into()
        })
        .into()
    }
}
