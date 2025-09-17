use std::time::Instant;

use super::Message;
use crate::style;
use data::chart::orderbook::Config;
use exchange::{TickerInfo, depth::Depth, TickMultiplier};

use iced::widget::canvas::{self, Text};
use iced::{Alignment, Event, Point, Rectangle, Renderer, Size, Theme, mouse};
use ordered_float::OrderedFloat;

const TEXT_SIZE: iced::Pixels = iced::Pixels(11.0);
const ROW_HEIGHT: f32 = 16.0;
const SPREAD_ROW_HEIGHT: f32 = 20.0;


impl super::Panel for Orderbook {
    fn scroll(&mut self, delta: f32) {
        // Orderbooks typically don't need scrolling as they show top levels
        // But we could implement it to show more levels if needed
        let _ = delta;
    }

    fn reset_scroll(&mut self) {
        // Nothing to reset for orderbook
    }

    fn invalidate(&mut self, now: Option<Instant>) -> Option<super::Action> {
        self.invalidate(now)
    }
}

pub struct Orderbook {
    depth: Depth,
    ticker_info: Option<TickerInfo>,
    pub config: Config,
    cache: canvas::Cache,
    last_tick: Instant,
    max_bid_qty: f32,
    max_ask_qty: f32,
    tick_multiplier: TickMultiplier,
}

impl Orderbook {
    pub fn new(config: Option<Config>, ticker_info: Option<TickerInfo>, tick_multiplier: TickMultiplier) -> Self {
        Self {
            depth: Depth::default(),
            config: config.unwrap_or_default(),
            ticker_info,
            cache: canvas::Cache::default(),
            last_tick: Instant::now(),
            max_bid_qty: 0.0,
            max_ask_qty: 0.0,
            tick_multiplier,
        }
    }

    pub fn update_depth(&mut self, depth: &Depth) {
        self.depth = depth.clone();
        self.calculate_max_quantities();
        self.invalidate(Some(Instant::now()));
    }

    fn calculate_max_quantities(&mut self) {
        let grouped_bids = self.group_price_levels(&self.depth.bids, true);
        let grouped_asks = self.group_price_levels(&self.depth.asks, false);
        
        self.max_bid_qty = grouped_bids
            .iter()
            .map(|(_, qty)| *qty)
            .fold(0.0, f32::max);
        
        self.max_ask_qty = grouped_asks
            .iter()
            .map(|(_, qty)| *qty)
            .fold(0.0, f32::max);
    }

    fn group_price_levels(&self, levels: &std::collections::BTreeMap<OrderedFloat<f32>, f32>, is_bid: bool) -> Vec<(f32, f32)> {
        let base_tick_size = self.tick_size().unwrap_or(0.01);
        let tick_size = (self.tick_multiplier.0 as f32) * base_tick_size;
        
        if tick_size <= 0.01 {
            // No grouping, return original levels
            if is_bid {
                return levels.iter()
                    .rev()
                    .map(|(price, qty)| (price.into_inner(), *qty))
                    .collect();
            } else {
                return levels.iter()
                    .map(|(price, qty)| (price.into_inner(), *qty))
                    .collect();
            }
        }

        let mut grouped_levels: std::collections::BTreeMap<OrderedFloat<f32>, f32> = std::collections::BTreeMap::new();
        
        for (price, qty) in levels.iter() {
            let price_val = price.into_inner();
            let grouped_price = if is_bid {
                // For bids, round down to the nearest tick size
                ((price_val * (1.0 / tick_size)).floor()) * tick_size
            } else {
                // For asks, round up to the nearest tick size  
                ((price_val * (1.0 / tick_size)).ceil()) * tick_size
            };
            let grouped_key = OrderedFloat(grouped_price);
            
            *grouped_levels.entry(grouped_key).or_insert(0.0) += qty;
        }

        if is_bid {
            grouped_levels.iter()
                .rev()
                .map(|(price, qty)| (price.into_inner(), *qty))
                .collect()
        } else {
            grouped_levels.iter()
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

    pub fn tick_size(&self) -> Option<f32> {
        self.ticker_info.map(|info| info.min_ticksize.into())
    }

    pub fn set_tick_multiplier(&mut self, tick_multiplier: TickMultiplier) {
        self.tick_multiplier = tick_multiplier;
        self.calculate_max_quantities();
        self.invalidate(Some(Instant::now()));
    }

    pub fn tick_multiplier(&self) -> TickMultiplier {
        self.tick_multiplier
    }

    pub fn invalidate(&mut self, now: Option<Instant>) -> Option<super::Action> {
        self.cache.clear();
        if let Some(now) = now {
            self.last_tick = now;
        }
        None
    }

    fn format_price(&self, price: f32) -> String {
        format!("{:.4}", price) // Default to 4 decimal places since config.precision was removed
    }

    fn format_quantity(&self, qty: f32) -> String {
        if qty < 1.0 {
            format!("{:.3}", qty)
        } else if qty < 1000.0 {
            format!("{:.1}", qty)
        } else {
            format!("{:.0}", qty)
        }
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
            Event::Mouse(mouse_event) => match mouse_event {
                mouse::Event::ButtonPressed(button) => match button {
                    mouse::Button::Middle => {
                        Some(canvas::Action::publish(Message::ResetScroll).and_capture())
                    }
                    _ => None,
                },
                mouse::Event::WheelScrolled { delta } => {
                    let scroll_amount = match delta {
                        mouse::ScrollDelta::Lines { y, .. } => *y * ROW_HEIGHT,
                        mouse::ScrollDelta::Pixels { y, .. } => *y,
                    };

                    Some(canvas::Action::publish(Message::Scrolled(scroll_amount)).and_capture())
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
        _cursor: iced_core::mouse::Cursor,
    ) -> Vec<iced::widget::canvas::Geometry<Renderer>> {
        let orderbook_visual = self.cache.draw(renderer, bounds.size(), |frame| {
            let palette = theme.extended_palette();
            let text_color = palette.background.base.text;
            let bid_color = palette.success.base.color;
            let ask_color = palette.danger.base.color;


            let mid_point = bounds.height / 2.0;
            let spread_height = if self.config.show_spread { SPREAD_ROW_HEIGHT } else { 0.0 };

            // Calculate how many levels can fit in each section
            let ask_section_height = (mid_point - spread_height / 2.0).max(0.0);
            let bid_section_height = bounds.height - mid_point - spread_height / 2.0;
            
            let max_ask_levels = ((ask_section_height / ROW_HEIGHT).floor() as usize).max(1);
            let max_bid_levels = ((bid_section_height / ROW_HEIGHT).floor() as usize).max(1);

            // Draw asks (top half, lowest ask closest to spread line)
            let asks = self.group_price_levels(&self.depth.asks, false);

            for (i, (price, qty)) in asks.iter().take(max_ask_levels).enumerate() {
                let y = ask_section_height - ((i + 1) as f32 * ROW_HEIGHT);
                if y < 0.0 { break; }

                self.draw_order_row(
                    frame,
                    y,
                    bounds.width,
                    *price,
                    *qty,
                    false, // is_bid
                    ask_color,
                    text_color,
                    self.max_ask_qty,
                );
            }

            // Draw spread in the middle
            if self.config.show_spread {
                if let Some(spread) = self.calculate_spread() {
                    let spread_y = mid_point - spread_height / 2.0;
                    let spread_text = format!("Spread: {:.4}", spread);
                    
                    frame.fill_text(Text {
                        content: spread_text,
                        position: Point::new(bounds.width / 2.0, spread_y + spread_height / 2.0),
                        color: text_color,
                        size: TEXT_SIZE,
                        font: style::AZERET_MONO,
                        align_x: Alignment::Center.into(),
                        align_y: Alignment::Center.into(),
                        ..Default::default()
                    });
                }
            }

            // Draw bids (bottom half)
            let bid_section_start = mid_point + spread_height / 2.0;
            let bids = self.group_price_levels(&self.depth.bids, true);

            for (i, (price, qty)) in bids.iter().take(max_bid_levels).enumerate() {
                let y = bid_section_start + (i as f32 * ROW_HEIGHT);
                if y + ROW_HEIGHT > bounds.height { break; }

                self.draw_order_row(
                    frame,
                    y,
                    bounds.width,
                    *price,
                    *qty,
                    true, // is_bid
                    bid_color,
                    text_color,
                    self.max_bid_qty,
                );
            }
        });

        vec![orderbook_visual]
    }
}

impl Orderbook {
    fn calculate_spread(&self) -> Option<f32> {
        if let (Some((best_ask, _)), Some((best_bid, _))) = 
            (self.depth.asks.first_key_value(), self.depth.bids.last_key_value()) {
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
            color: color,
            size: TEXT_SIZE,
            font: style::AZERET_MONO,
            align_x: if is_bid { Alignment::Start.into() } else { Alignment::End.into() },
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
            align_x: if is_bid { Alignment::Start.into() } else { Alignment::End.into() },
            align_y: Alignment::Center.into(),
            ..Default::default()
        });
    }
}