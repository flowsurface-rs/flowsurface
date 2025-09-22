use super::Message;
use crate::style;
use data::chart::kline::KlineTrades;
use data::chart::ladder::Config;
use exchange::Trade;
use exchange::util::{Price, PriceStep};
use exchange::{TickerInfo, depth::Depth};

use iced::widget::canvas::{self, Text};
use iced::{Alignment, Event, Point, Rectangle, Renderer, Size, Theme, mouse};

use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;

const TEXT_SIZE: f32 = 11.0;
const ROW_HEIGHT: f32 = 16.0;

// Total width ratios must sum to 1.0
/// Uses half of the width for each side of the order quantity columns
const ORDER_QTY_COLS_WIDTH: f32 = 0.60;
/// Uses half of the width for each side of the trade quantity columns
const TRADE_QTY_COLS_WIDTH: f32 = 0.20;
const PRICE_COL_WIDTH: f32 = 0.20;

/// Horizontal gap between columns (pixels)
const COL_PADDING: f32 = 4.0;

impl super::Panel for Ladder {
    fn scroll(&mut self, delta: f32) {
        self.scroll_px += delta;
        Ladder::invalidate(self, Some(Instant::now()));
    }

    fn reset_scroll(&mut self) {
        self.scroll_px = 0.0;
        Ladder::invalidate(self, Some(Instant::now()));
    }

    fn invalidate(&mut self, now: Option<Instant>) -> Option<super::Action> {
        Ladder::invalidate(self, now)
    }
}

pub struct Ladder {
    depth: Depth,
    raw_trades: VecDeque<Trade>,
    grouped_trades: KlineTrades,
    ticker_info: TickerInfo,
    pub config: Config,
    cache: canvas::Cache,
    last_tick: Instant,
    tick_size: PriceStep,
    scroll_px: f32,
    last_exchange_ts_ms: Option<u64>,
}

impl Ladder {
    pub fn new(config: Option<Config>, ticker_info: TickerInfo, tick_size: f32) -> Self {
        Self {
            depth: Depth::default(),
            raw_trades: VecDeque::new(),
            grouped_trades: KlineTrades::new(),
            config: config.unwrap_or_default(),
            ticker_info,
            cache: canvas::Cache::default(),
            last_tick: Instant::now(),
            tick_size: PriceStep::from_f32(tick_size),
            scroll_px: 0.0,
            last_exchange_ts_ms: None,
        }
    }

    pub fn insert_buffers(&mut self, update_t: u64, depth: &Depth, trades_buffer: &[Trade]) {
        self.depth = depth.clone();
        let tick_size = self.tick_size;

        for trade in trades_buffer {
            self.grouped_trades.add_trade_to_side_bin(trade, tick_size);
            self.raw_trades.push_back(*trade);
        }

        self.last_exchange_ts_ms = Some(update_t);
        self.maybe_cleanup_trades(update_t);
    }

    fn maybe_cleanup_trades(&mut self, now_ms: u64) {
        let Some(oldest_trade) = self.raw_trades.front() else {
            return;
        };

        let oldest_ms = oldest_trade.time;

        // Derive cleanup step from retention: ~1/10th (min 5s)
        let retention_ms = self.config.trade_retention.as_millis() as u64;
        if retention_ms == 0 {
            return;
        }
        let cleanup_step_ms = (retention_ms / 10).max(5_000);

        let threshold_ms = retention_ms + cleanup_step_ms;
        if now_ms.saturating_sub(oldest_ms) < threshold_ms {
            return;
        }

        let keep_from_ms = now_ms.saturating_sub(retention_ms);

        let mut removed = 0usize;
        while let Some(trade) = self.raw_trades.front() {
            if trade.time < keep_from_ms {
                self.raw_trades.pop_front();
                removed += 1;
            } else {
                break;
            }
        }

        if removed > 0 {
            self.grouped_trades.clear();
            for trade in &self.raw_trades {
                self.grouped_trades
                    .add_trade_to_side_bin(trade, self.tick_size);
            }
            self.invalidate(Some(Instant::now()));
        }
    }

    pub fn last_update(&self) -> Instant {
        self.last_tick
    }

    pub fn current_price(&self) -> Option<Price> {
        self.depth.mid_price()
    }

    pub fn min_tick_size(&self) -> f32 {
        self.ticker_info.min_ticksize.into()
    }

    pub fn set_tick_size(&mut self, tick_size: f32) {
        let step = PriceStep::from_f32(tick_size);
        self.tick_size = step;

        self.grouped_trades.clear();
        for trade in &self.raw_trades {
            self.grouped_trades.add_trade_to_side_bin(trade, step);
        }

        self.invalidate(Some(Instant::now()));
    }

    pub fn invalidate(&mut self, now: Option<Instant>) -> Option<super::Action> {
        self.cache.clear();
        if let Some(now) = now {
            self.last_tick = now;
        }
        None
    }

    pub fn tick_size(&self) -> f32 {
        self.tick_size.to_f32_lossy()
    }

    fn format_price(&self, price: Price) -> String {
        let precision = self.ticker_info.min_ticksize;
        price.to_string(precision)
    }

    fn format_quantity(&self, qty: f32) -> String {
        data::util::abbr_large_numbers(qty)
    }

    fn calculate_spread(&self) -> Option<Price> {
        if let (Some((best_ask, _)), Some((best_bid, _))) = (
            self.depth.asks.first_key_value(),
            self.depth.bids.last_key_value(),
        ) {
            Some(*best_ask - *best_bid)
        } else {
            None
        }
    }

    fn group_price_levels(
        &self,
        levels: &BTreeMap<Price, f32>,
        is_bid: bool,
    ) -> BTreeMap<Price, f32> {
        let mut grouped = BTreeMap::new();

        for (price, qty) in levels.iter() {
            let grouped_price = price.round_to_side_step(is_bid, self.tick_size);
            *grouped.entry(grouped_price).or_insert(0.0) += qty;
        }

        grouped
    }
}

impl canvas::Program<Message> for Ladder {
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

        let divider_color = style::split_ruler(theme).color;

        let asks_grouped = self.group_price_levels(&self.depth.asks, false);
        let bids_grouped = self.group_price_levels(&self.depth.bids, true);

        let orderbook_visual = self.cache.draw(renderer, bounds.size(), |frame| {
            let cols = self.column_ranges(bounds.width);

            if let Some(grid) = self.build_price_grid(&asks_grouped, &bids_grouped) {
                let (visible_rows, maxima) =
                    self.visible_rows(bounds, &asks_grouped, &bids_grouped, &grid);

                let mut spread_row: Option<(f32, f32)> = None;

                for visible_row in visible_rows {
                    match visible_row.row {
                        DomRow::Ask { price, qty } => {
                            self.draw_row(
                                frame,
                                visible_row.y,
                                price,
                                qty,
                                false,
                                ask_color,
                                text_color,
                                maxima.vis_max_order_qty,
                                visible_row.buy_t,
                                visible_row.sell_t,
                                maxima.vis_max_trade_qty,
                                bid_color,
                                ask_color,
                                &cols,
                            );
                        }
                        DomRow::Bid { price, qty } => {
                            self.draw_row(
                                frame,
                                visible_row.y,
                                price,
                                qty,
                                true,
                                bid_color,
                                text_color,
                                maxima.vis_max_order_qty,
                                visible_row.buy_t,
                                visible_row.sell_t,
                                maxima.vis_max_trade_qty,
                                bid_color,
                                ask_color,
                                &cols,
                            );
                        }
                        DomRow::Spread => {
                            if let Some(spread) = self.calculate_spread() {
                                let min_ticksize = self.ticker_info.min_ticksize;
                                spread_row = Some((visible_row.y, visible_row.y + ROW_HEIGHT));

                                let spread = spread.round_to_min_tick(min_ticksize);
                                let content = format!("Spread: {}", spread.to_string(min_ticksize));
                                frame.fill_text(Text {
                                    content,
                                    position: Point::new(
                                        bounds.width / 2.0,
                                        visible_row.y + ROW_HEIGHT / 2.0,
                                    ),
                                    color: palette.secondary.strong.color,
                                    size: (TEXT_SIZE - 1.0).into(),
                                    font: style::AZERET_MONO,
                                    align_x: Alignment::Center.into(),
                                    align_y: Alignment::Center.into(),
                                    ..Default::default()
                                });
                            }
                        }
                        DomRow::CenterDivider => {
                            let y_mid = visible_row.y + ROW_HEIGHT / 2.0 - 0.5;

                            frame.fill_rectangle(
                                Point::new(0.0, y_mid),
                                Size::new(bounds.width, 1.0),
                                divider_color,
                            );
                        }
                    }
                }

                // Price column vertical dividers with a gap over the spread row (if visible)
                let mut draw_vsplit = |x: f32, gap: Option<(f32, f32)>| {
                    let x = x.floor() + 0.5;
                    match gap {
                        Some((top, bottom)) => {
                            if top > 0.0 {
                                frame.fill_rectangle(
                                    Point::new(x, 0.0),
                                    Size::new(1.0, top.max(0.0)),
                                    divider_color,
                                );
                            }
                            if bottom < bounds.height {
                                frame.fill_rectangle(
                                    Point::new(x, bottom),
                                    Size::new(1.0, (bounds.height - bottom).max(0.0)),
                                    divider_color,
                                );
                            }
                        }
                        None => {
                            frame.fill_rectangle(
                                Point::new(x, 0.0),
                                Size::new(1.0, bounds.height),
                                divider_color,
                            );
                        }
                    }
                };
                draw_vsplit(cols.sell.1, spread_row);
                draw_vsplit(cols.buy.0, spread_row);

                if let Some((top, bottom)) = spread_row {
                    let y_top: f32 = top.floor() + 0.5;
                    let y_bot = bottom.floor() + 0.5;

                    frame.fill_rectangle(
                        Point::new(0.0, y_top),
                        Size::new(cols.sell.1, 1.0),
                        divider_color,
                    );
                    frame.fill_rectangle(
                        Point::new(0.0, y_bot),
                        Size::new(cols.sell.1, 1.0),
                        divider_color,
                    );

                    frame.fill_rectangle(
                        Point::new(cols.buy.0, y_top),
                        Size::new(bounds.width - cols.buy.0, 1.0),
                        divider_color,
                    );
                    frame.fill_rectangle(
                        Point::new(cols.buy.0, y_bot),
                        Size::new(bounds.width - cols.buy.0, 1.0),
                        divider_color,
                    );
                }
            }
        });

        vec![orderbook_visual]
    }
}

#[derive(Default)]
struct Maxima {
    vis_max_order_qty: f32,
    vis_max_trade_qty: f32,
}

struct VisibleRow {
    row: DomRow,
    y: f32,
    buy_t: f32,
    sell_t: f32,
}

struct ColumnRanges {
    bid_order: (f32, f32),
    sell: (f32, f32),
    price: (f32, f32),
    buy: (f32, f32),
    ask_order: (f32, f32),
}

impl Ladder {
    // [BidOrderQty][SellQty][ Price ][BuyQty][AskOrderQty]
    const NUMBER_OF_COLUMN_GAPS: f32 = 4.0;

    fn column_ranges(&self, width: f32) -> ColumnRanges {
        let order_qty_ratio = ORDER_QTY_COLS_WIDTH / 2.0;
        let trade_qty_ratio = TRADE_QTY_COLS_WIDTH / 2.0;

        let total_gutter_width = COL_PADDING * Self::NUMBER_OF_COLUMN_GAPS;

        let usable_width = (width - total_gutter_width).max(0.0);

        let bid_order_width = order_qty_ratio * usable_width;
        let sell_trades_width = trade_qty_ratio * usable_width;
        let price_width = PRICE_COL_WIDTH * usable_width;
        let buy_trades_width = trade_qty_ratio * usable_width;
        let ask_order_width = order_qty_ratio * usable_width;

        let mut cursor_x = 0.0;

        let bid_order_end = cursor_x + bid_order_width;
        let bid_order_range = (cursor_x, bid_order_end);
        cursor_x = bid_order_end + COL_PADDING;

        let sell_trades_end = cursor_x + sell_trades_width;
        let sell_trades_range = (cursor_x, sell_trades_end);
        cursor_x = sell_trades_end + COL_PADDING;

        let price_end = cursor_x + price_width;
        let price_range = (cursor_x, price_end);
        cursor_x = price_end + COL_PADDING;

        let buy_trades_end = cursor_x + buy_trades_width;
        let buy_trades_range = (cursor_x, buy_trades_end);
        cursor_x = buy_trades_end + COL_PADDING;

        let ask_order_end = cursor_x + ask_order_width;
        let ask_order_range = (cursor_x, ask_order_end);

        ColumnRanges {
            bid_order: bid_order_range,
            sell: sell_trades_range,
            price: price_range,
            buy: buy_trades_range,
            ask_order: ask_order_range,
        }
    }

    fn trade_qty_at(&self, price: Price) -> (f32, f32) {
        if let Some(g) = self.grouped_trades.trades.get(&price) {
            (g.buy_qty, g.sell_qty)
        } else {
            (0.0, 0.0)
        }
    }

    fn draw_row(
        &self,
        frame: &mut iced::widget::canvas::Frame,
        y: f32,
        price: Price,
        order_qty: f32,
        is_bid: bool,
        side_color: iced::Color,
        text_color: iced::Color,
        max_order_qty: f32,
        trade_buy_qty: f32,
        trade_sell_qty: f32,
        max_trade_qty: f32,
        trade_buy_color: iced::Color,
        trade_sell_color: iced::Color,
        cols: &ColumnRanges,
    ) {
        if is_bid {
            Self::fill_bar(
                frame,
                cols.bid_order,
                y,
                ROW_HEIGHT,
                order_qty,
                max_order_qty,
                side_color,
                true,
                0.20,
            );
            let qty_txt = self.format_quantity(order_qty);
            let x_text = cols.bid_order.0 + 6.0;
            Self::draw_cell_text(frame, &qty_txt, x_text, y, text_color, Alignment::Start);
        } else {
            Self::fill_bar(
                frame,
                cols.ask_order,
                y,
                ROW_HEIGHT,
                order_qty,
                max_order_qty,
                side_color,
                false,
                0.20,
            );
            let qty_txt = self.format_quantity(order_qty);
            let x_text = cols.ask_order.1 - 6.0;
            Self::draw_cell_text(frame, &qty_txt, x_text, y, text_color, Alignment::End);
        }

        // Sell trades (right-to-left)
        Self::fill_bar(
            frame,
            cols.sell,
            y,
            ROW_HEIGHT,
            trade_sell_qty,
            max_trade_qty,
            trade_sell_color,
            false,
            0.30,
        );
        let sell_txt = if trade_sell_qty > 0.0 {
            self.format_quantity(trade_sell_qty)
        } else {
            "".into()
        };
        Self::draw_cell_text(
            frame,
            &sell_txt,
            cols.sell.1 - 6.0,
            y,
            text_color,
            Alignment::End,
        );

        // Buy trades (left-to-right)
        Self::fill_bar(
            frame,
            cols.buy,
            y,
            ROW_HEIGHT,
            trade_buy_qty,
            max_trade_qty,
            trade_buy_color,
            true,
            0.30,
        );
        let buy_txt = if trade_buy_qty > 0.0 {
            self.format_quantity(trade_buy_qty)
        } else {
            "".into()
        };
        Self::draw_cell_text(
            frame,
            &buy_txt,
            cols.buy.0 + 6.0,
            y,
            text_color,
            Alignment::Start,
        );

        // Price
        let price_text = self.format_price(price);
        let price_x_center = (cols.price.0 + cols.price.1) * 0.5;
        Self::draw_cell_text(
            frame,
            &price_text,
            price_x_center,
            y,
            side_color,
            Alignment::Center,
        );
    }

    fn fill_bar(
        frame: &mut iced::widget::canvas::Frame,
        (x_start, x_end): (f32, f32),
        y: f32,
        height: f32,
        value: f32,
        scale_value_max: f32,
        color: iced::Color,
        from_left: bool,
        alpha: f32,
    ) {
        if scale_value_max <= 0.0 || value <= 0.0 {
            return;
        }
        let col_width = x_end - x_start;

        let mut bar_width = (value / scale_value_max) * col_width.max(1.0);
        bar_width = bar_width.min(col_width);
        let bar_x = if from_left {
            x_start
        } else {
            x_end - bar_width
        };

        frame.fill_rectangle(
            Point::new(bar_x, y),
            Size::new(bar_width, height),
            iced::Color { a: alpha, ..color },
        );
    }

    fn draw_cell_text(
        frame: &mut iced::widget::canvas::Frame,
        text: &str,
        x_anchor: f32,
        y: f32,
        color: iced::Color,
        align: Alignment,
    ) {
        frame.fill_text(Text {
            content: text.to_string(),
            position: Point::new(x_anchor, y + ROW_HEIGHT / 2.0),
            color,
            size: TEXT_SIZE.into(),
            font: style::AZERET_MONO,
            align_x: align.into(),
            align_y: Alignment::Center.into(),
            ..Default::default()
        });
    }

    fn build_price_grid(
        &self,
        asks_grouped: &BTreeMap<Price, f32>,
        bids_grouped: &BTreeMap<Price, f32>,
    ) -> Option<PriceGrid> {
        let best_bid = bids_grouped.last_key_value().map(|(k, _)| *k);
        let best_ask = asks_grouped.first_key_value().map(|(k, _)| *k);

        let (best_bid, best_ask) = match (best_bid, best_ask) {
            (Some(bb), Some(ba)) => (bb, ba),
            (Some(bb), None) => (bb, bb.add_steps(1, self.tick_size)),
            (None, Some(ba)) => (ba.add_steps(-1, self.tick_size), ba),
            (None, None) => {
                let mut min_t: Option<Price> = None;
                let mut max_t: Option<Price> = None;

                for &p in self.grouped_trades.trades.keys() {
                    min_t = Some(min_t.map_or(p, |cur| cur.min(p)));
                    max_t = Some(max_t.map_or(p, |cur| cur.max(p)));
                }
                let (Some(min_t), Some(max_t)) = (min_t, max_t) else {
                    return None;
                };

                let steps =
                    Price::steps_between_inclusive(min_t, max_t, self.tick_size).unwrap_or(1);
                let mid = max_t.add_steps(-(steps as i64 / 2), self.tick_size);

                (mid, mid.add_steps(1, self.tick_size))
            }
        };

        Some(PriceGrid {
            best_bid,
            best_ask,
            tick: self.tick_size,
        })
    }

    fn visible_rows(
        &self,
        bounds: Rectangle,
        asks_grouped: &BTreeMap<Price, f32>,
        bids_grouped: &BTreeMap<Price, f32>,
        grid: &PriceGrid,
    ) -> (Vec<VisibleRow>, Maxima) {
        let mut visible: Vec<VisibleRow> = Vec::new();
        let mut maxima = Maxima::default();

        let mid_screen_y = bounds.height * 0.5;
        let scroll = self.scroll_px;

        let y0 = mid_screen_y + PriceGrid::top_y(0) - scroll;
        let idx_top = ((0.0 - y0) / ROW_HEIGHT).floor() as i32;

        let rows_needed = (bounds.height / ROW_HEIGHT).ceil() as i32 + 1;
        let idx_bottom = idx_top + rows_needed;

        for idx in idx_top..=idx_bottom {
            if idx == 0 {
                let top_y_screen = mid_screen_y + PriceGrid::top_y(0) - scroll;
                if top_y_screen < bounds.height && top_y_screen + ROW_HEIGHT > 0.0 {
                    let row = if self.config.show_spread
                        && self.ticker_info.exchange().is_depth_client_aggr()
                    {
                        DomRow::Spread
                    } else {
                        DomRow::CenterDivider
                    };

                    visible.push(VisibleRow {
                        row,
                        y: top_y_screen,
                        buy_t: 0.0,
                        sell_t: 0.0,
                    });
                }
                continue;
            }

            let Some(price) = grid.index_to_price(idx) else {
                continue;
            };

            let is_bid = idx > 0;
            let order_qty = if is_bid {
                bids_grouped.get(&price).copied().unwrap_or(0.0)
            } else {
                asks_grouped.get(&price).copied().unwrap_or(0.0)
            };

            let top_y_screen = mid_screen_y + PriceGrid::top_y(idx) - scroll;
            if top_y_screen >= bounds.height || top_y_screen + ROW_HEIGHT <= 0.0 {
                continue;
            }

            maxima.vis_max_order_qty = maxima.vis_max_order_qty.max(order_qty);
            let (buy_t, sell_t) = self.trade_qty_at(price);
            maxima.vis_max_trade_qty = maxima.vis_max_trade_qty.max(buy_t.max(sell_t));

            let row = if is_bid {
                DomRow::Bid {
                    price,
                    qty: order_qty,
                }
            } else {
                DomRow::Ask {
                    price,
                    qty: order_qty,
                }
            };

            visible.push(VisibleRow {
                row,
                y: top_y_screen,
                buy_t,
                sell_t,
            });
        }

        visible.sort_by(|a, b| a.y.total_cmp(&b.y));
        (visible, maxima)
    }
}

enum DomRow {
    Ask { price: Price, qty: f32 },
    Spread,
    CenterDivider,
    Bid { price: Price, qty: f32 },
}

struct PriceGrid {
    best_bid: Price,
    best_ask: Price,
    tick: PriceStep,
}

impl PriceGrid {
    /// Returns None for index 0 (spread row)
    fn index_to_price(&self, idx: i32) -> Option<Price> {
        if idx == 0 {
            return None;
        }
        if idx > 0 {
            let off = (idx - 1) as i64; // 1 => best_bid, 2 => best_bid - 1 tick
            Some(self.best_bid.add_steps(-off, self.tick))
        } else {
            let off = (-1 - idx) as i64; // -1 => best_ask, -2 => best_ask + 1 tick
            Some(self.best_ask.add_steps(off, self.tick))
        }
    }

    fn top_y(idx: i32) -> f32 {
        (idx as f32) * ROW_HEIGHT - ROW_HEIGHT * 0.5
    }
}
