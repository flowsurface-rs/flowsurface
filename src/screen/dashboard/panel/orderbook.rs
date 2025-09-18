use std::time::Instant;

use super::Message;
use crate::style;
use data::chart::orderbook::Config;
use exchange::{TickerInfo, depth::Depth};

use iced::widget::canvas::{self, Text};
use iced::{Alignment, Event, Point, Rectangle, Renderer, Size, Theme, mouse};
use ordered_float::OrderedFloat;

const TEXT_SIZE: iced::Pixels = iced::Pixels(11.0);
const ROW_HEIGHT: f32 = 16.0;
const SPREAD_ROW_HEIGHT: f32 = 20.0;

impl super::Panel for Orderbook {
    fn scroll(&mut self, delta: f32) {
        self.scroll_px += delta;
        Orderbook::invalidate(self, Some(Instant::now()));
    }

    fn reset_scroll(&mut self) {
        self.scroll_px = 0.0;
        Orderbook::invalidate(self, Some(Instant::now()));
    }

    fn invalidate(&mut self, now: Option<Instant>) -> Option<super::Action> {
        Orderbook::invalidate(self, now)
    }
}

pub struct Orderbook {
    depth: Depth,
    ticker_info: Option<TickerInfo>,
    pub config: Config,
    cache: canvas::Cache,
    last_tick: Instant,
    pub tick_size: f32,
    decimals: usize,
    scroll_px: f32,
}

impl Orderbook {
    pub fn new(config: Option<Config>, ticker_info: Option<TickerInfo>, tick_size: f32) -> Self {
        Self {
            depth: Depth::default(),
            config: config.unwrap_or_default(),
            ticker_info,
            cache: canvas::Cache::default(),
            last_tick: Instant::now(),
            tick_size,
            decimals: data::util::count_decimals(tick_size),
            scroll_px: 0.0,
        }
    }

    pub fn update_depth(&mut self, depth: &Depth) {
        self.depth = depth.clone();
        self.invalidate(Some(Instant::now()));
    }

    fn group_price_levels(
        &self,
        levels: &std::collections::BTreeMap<OrderedFloat<f32>, f32>,
        is_bid: bool,
    ) -> Vec<(f32, f32)> {
        let mut grouped_levels: std::collections::BTreeMap<OrderedFloat<f32>, f32> =
            std::collections::BTreeMap::new();

        let tick_size = self.tick_size;

        for (price, qty) in levels.iter() {
            let price_val = price.into_inner();
            let grouped_price = if is_bid {
                ((price_val * (1.0 / tick_size)).floor()) * tick_size
            } else {
                ((price_val * (1.0 / tick_size)).ceil()) * tick_size
            };
            let grouped_key = OrderedFloat(grouped_price);

            *grouped_levels.entry(grouped_key).or_insert(0.0) += qty;
        }

        if is_bid {
            grouped_levels
                .iter()
                .rev()
                .map(|(price, qty)| (price.into_inner(), *qty))
                .collect()
        } else {
            grouped_levels
                .iter()
                .map(|(price, qty)| (price.into_inner(), *qty))
                .collect()
        }
    }

    pub fn last_update(&self) -> Instant {
        self.last_tick
    }

    pub fn current_price(&self) -> Option<f32> {
        self.depth.mid_price()
    }

    pub fn min_tick_size(&self) -> Option<f32> {
        self.ticker_info.map(|info| info.min_ticksize.into())
    }

    pub fn set_tick_size(&mut self, tick_size: f32) {
        self.decimals = data::util::count_decimals(tick_size);
        self.tick_size = tick_size;
        self.invalidate(Some(Instant::now()));
    }

    pub fn invalidate(&mut self, now: Option<Instant>) -> Option<super::Action> {
        self.cache.clear();
        if let Some(now) = now {
            self.last_tick = now;
        }
        None
    }

    fn format_price(&self, price: f32) -> String {
        format!("{:.*}", self.decimals, price)
    }

    fn format_quantity(&self, qty: f32) -> String {
        data::util::abbr_large_numbers(qty)
    }
}

impl canvas::Program<Message> for Orderbook {
    type State = ();

    fn update(
        &self,
        _state: &mut Self::State,
        event: &iced::Event,
        bounds: iced::Rectangle,
        cursor: iced_core::mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let _cursor_position = cursor.position_in(bounds)?;

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(
                mouse::Button::Middle | mouse::Button::Left | mouse::Button::Right,
            )) => Some(canvas::Action::publish(Message::ResetScroll).and_capture()),
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let scroll_amount = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => -(*y) * ROW_HEIGHT,
                    mouse::ScrollDelta::Pixels { y, .. } => -*y,
                };

                Some(canvas::Action::publish(Message::Scrolled(scroll_amount)).and_capture())
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: iced_core::mouse::Cursor,
    ) -> Vec<iced::widget::canvas::Geometry<Renderer>> {
        let palette = theme.extended_palette();

        let text_color = palette.background.base.text;
        let bid_color = palette.success.base.color;
        let ask_color = palette.danger.base.color;

        let asks = self.group_price_levels(&self.depth.asks, false);
        let bids = self.group_price_levels(&self.depth.bids, true);

        let mut rows: Vec<Row> = Vec::with_capacity(
            asks.len() + bids.len() + if self.config.show_spread { 1 } else { 0 },
        );

        for (price, qty) in asks.iter().rev() {
            rows.push(Row::Ask {
                price: *price,
                qty: *qty,
            });
        }
        if self.config.show_spread {
            rows.push(Row::Spread);
        }
        for (price, qty) in bids.iter() {
            rows.push(Row::Bid {
                price: *price,
                qty: *qty,
            });
        }

        let orderbook_visual = self.cache.draw(renderer, bounds.size(), |frame| {
            let pre_spread_height = (asks.len() as f32) * ROW_HEIGHT;
            let center_target = if self.config.show_spread {
                pre_spread_height + SPREAD_ROW_HEIGHT / 2.0
            } else {
                pre_spread_height
            };

            let base_scroll = center_target - bounds.height / 2.0;

            let mut y_cursor = -(base_scroll + self.scroll_px);
            let mut vis_max_ask_qty: f32 = 0.0;
            let mut vis_max_bid_qty: f32 = 0.0;
            let mut visible_idx: Vec<(usize, f32)> = vec![];

            for (idx, row) in rows.iter().enumerate() {
                let h = match row {
                    Row::Spread => SPREAD_ROW_HEIGHT,
                    _ => ROW_HEIGHT,
                };

                if y_cursor + h <= 0.0 {
                    y_cursor += h;
                    continue;
                }
                if y_cursor >= bounds.height {
                    break;
                }

                match row {
                    Row::Ask { qty, .. } => {
                        vis_max_ask_qty = vis_max_ask_qty.max(*qty);
                    }
                    Row::Bid { qty, .. } => {
                        vis_max_bid_qty = vis_max_bid_qty.max(*qty);
                    }
                    Row::Spread => {}
                }

                visible_idx.push((idx, y_cursor));
                y_cursor += h;
            }

            for (idx, y) in visible_idx {
                match &rows[idx] {
                    Row::Ask { price, qty } => {
                        self.draw_order_row(
                            frame,
                            y,
                            bounds.width,
                            *price,
                            *qty,
                            false,
                            ask_color,
                            text_color,
                            vis_max_ask_qty,
                        );
                    }
                    Row::Bid { price, qty } => {
                        self.draw_order_row(
                            frame,
                            y,
                            bounds.width,
                            *price,
                            *qty,
                            true,
                            bid_color,
                            text_color,
                            vis_max_bid_qty,
                        );
                    }
                    Row::Spread => {
                        if let (Some(info), Some(spread)) =
                            (self.ticker_info, self.calculate_spread())
                        {
                            let min_ticksize: f32 = info.min_ticksize.into();
                            let spread = (spread / min_ticksize).round() * min_ticksize;
                            let content = format!("Spread: {spread}");
                            frame.fill_text(Text {
                                content,
                                position: Point::new(
                                    bounds.width / 2.0,
                                    y + SPREAD_ROW_HEIGHT / 2.0,
                                ),
                                color: text_color,
                                size: TEXT_SIZE,
                                font: style::AZERET_MONO,
                                align_x: Alignment::Center.into(),
                                align_y: Alignment::Center.into(),
                                ..Default::default()
                            });
                        }
                    }
                }
            }
        });

        vec![orderbook_visual]
    }
}

impl Orderbook {
    fn calculate_spread(&self) -> Option<f32> {
        if let (Some((best_ask, _)), Some((best_bid, _))) = (
            self.depth.asks.first_key_value(),
            self.depth.bids.last_key_value(),
        ) {
            Some(best_ask.into_inner() - best_bid.into_inner())
        } else {
            None
        }
    }

    fn draw_order_row(
        &self,
        frame: &mut iced::widget::canvas::Frame,
        y: f32,
        width: f32,
        price: f32,
        qty: f32,
        is_bid: bool,
        color: iced::Color,
        text_color: iced::Color,
        max_qty: f32,
    ) {
        let price_text = self.format_price(price);
        let qty_text = self.format_quantity(qty);

        // Draw quantity bar background
        if max_qty > 0.0 {
            let bar_width = (qty / max_qty) * width * 0.3;
            let bar_x = if is_bid { 0.0 } else { width - bar_width };

            let bar_color = iced::Color {
                r: color.r,
                g: color.g,
                b: color.b,
                a: 0.2,
            };

            frame.fill_rectangle(
                Point::new(bar_x, y),
                Size::new(bar_width, ROW_HEIGHT),
                bar_color,
            );
        }

        // Draw price text
        let price_x = if is_bid { width * 0.35 } else { width * 0.65 };
        frame.fill_text(Text {
            content: price_text,
            position: Point::new(price_x, y + ROW_HEIGHT / 2.0),
            color,
            size: TEXT_SIZE,
            font: style::AZERET_MONO,
            align_x: if is_bid {
                Alignment::Start.into()
            } else {
                Alignment::End.into()
            },
            align_y: Alignment::Center.into(),
            ..Default::default()
        });

        // Draw quantity text
        let qty_x = if is_bid { width * 0.05 } else { width * 0.95 };
        frame.fill_text(Text {
            content: qty_text,
            position: Point::new(qty_x, y + ROW_HEIGHT / 2.0),
            color: text_color,
            size: TEXT_SIZE,
            font: style::AZERET_MONO,
            align_x: if is_bid {
                Alignment::Start.into()
            } else {
                Alignment::End.into()
            },
            align_y: Alignment::Center.into(),
            ..Default::default()
        });
    }
}

enum Row {
    Ask { price: f32, qty: f32 },
    Spread,
    Bid { price: f32, qty: f32 },
}
