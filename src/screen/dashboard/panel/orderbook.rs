use super::Message;
use crate::style;
use data::chart::kline::KlineTrades;
use data::chart::orderbook::Config;
use exchange::Trade;
use exchange::util::{Price, PriceStep};
use exchange::{TickerInfo, depth::Depth};

use iced::widget::canvas::{self, Text};
use iced::{Alignment, Event, Point, Rectangle, Renderer, Size, Theme, mouse};

use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;

const TEXT_SIZE: iced::Pixels = iced::Pixels(11.0);
const ROW_HEIGHT: f32 = 16.0;
const SPREAD_ROW_HEIGHT: f32 = 20.0;

// Total width ratios must sum to 1.0
/// Uses half of the width for each side of the order quantity columns
const ORDER_QTY_COLS_WIDTH: f32 = 0.60;
/// Uses half of the width for each side of the trade quantity columns
const TRADE_QTY_COLS_WIDTH: f32 = 0.20;
const PRICE_COL_WIDTH: f32 = 0.20;

/// Horizontal gap between columns (pixels)
const COL_PADDING: f32 = 4.0;

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
    raw_trades: VecDeque<Trade>,
    grouped_trades: KlineTrades,
    ticker_info: Option<TickerInfo>,
    pub config: Config,
    cache: canvas::Cache,
    last_tick: Instant,
    tick_size: PriceStep,
    decimals: usize,
    scroll_px: f32,
    last_exchange_ts_ms: Option<u64>,
}

impl Orderbook {
    pub fn new(config: Option<Config>, ticker_info: Option<TickerInfo>, tick_size: f32) -> Self {
        Self {
            depth: Depth::default(),
            raw_trades: VecDeque::new(),
            grouped_trades: KlineTrades::new(),
            config: config.unwrap_or_default(),
            ticker_info,
            cache: canvas::Cache::default(),
            last_tick: Instant::now(),
            tick_size: PriceStep::from_f32(tick_size),
            decimals: data::util::count_decimals(tick_size),
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

    pub fn min_tick_size(&self) -> Option<f32> {
        self.ticker_info.map(|info| info.min_ticksize.into())
    }

    pub fn set_tick_size(&mut self, tick_size: f32) {
        self.decimals = data::util::count_decimals(tick_size);

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

    fn format_price(&self, price: Price) -> Option<String> {
        self.ticker_info
            .map(|info| price.to_string(info.min_ticksize))
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

        let asks_grouped = self.group_price_levels(&self.depth.asks, false);
        let bids_grouped = self.group_price_levels(&self.depth.bids, true);

        let meta = self.row_meta(
            &asks_grouped,
            &bids_grouped,
            self.tick_size,
            self.config.show_spread,
        );

        let pre_spread_height = (meta.ask_rows as f32) * ROW_HEIGHT;
        let center_target = if self.config.show_spread {
            pre_spread_height + SPREAD_ROW_HEIGHT / 2.0
        } else {
            pre_spread_height
        };
        let base_scroll = center_target - bounds.height / 2.0;

        let orderbook_visual = self.cache.draw(renderer, bounds.size(), |frame| {
            let cols = self.column_ranges(bounds.width);

            let (visible_rows, maxima) = self.visible_rows(
                bounds,
                &asks_grouped,
                &bids_grouped,
                self.tick_size,
                &meta,
                base_scroll,
            );

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
                        if let (Some(info), Some(spread)) =
                            (self.ticker_info, self.calculate_spread())
                        {
                            let spread = spread.round_to_min_tick(info.min_ticksize);
                            let content =
                                format!("Spread: {}", spread.to_string(info.min_ticksize));
                            frame.fill_text(Text {
                                content,
                                position: Point::new(
                                    bounds.width / 2.0,
                                    visible_row.y + SPREAD_ROW_HEIGHT / 2.0,
                                ),
                                color: palette.secondary.strong.color,
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

struct RowMeta {
    ask_rows: usize,
    bid_rows: usize,
    spread_rows: usize,
    total_rows: usize,
    max_ask: Option<Price>,
    best_bid: Option<Price>,
}

struct ColumnRanges {
    bid_order: (f32, f32),
    sell: (f32, f32),
    price: (f32, f32),
    buy: (f32, f32),
    ask_order: (f32, f32),
}

impl Orderbook {
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

    fn row_meta(
        &self,
        asks_grouped: &BTreeMap<Price, f32>,
        bids_grouped: &BTreeMap<Price, f32>,
        tick_size: PriceStep,
        show_spread: bool,
    ) -> RowMeta {
        let (max_ask_opt, ask_rows) = if let (Some((best_ask, _)), Some((max_ask, _))) = (
            asks_grouped.first_key_value(),
            asks_grouped.last_key_value(),
        ) {
            let rows = Price::steps_between_inclusive(*best_ask, *max_ask, tick_size).unwrap_or(0);
            (Some(*max_ask), rows)
        } else {
            (None, 0)
        };

        let (best_bid_opt, bid_rows) = if let (Some((min_bid, _)), Some((best_bid, _))) = (
            bids_grouped.first_key_value(),
            bids_grouped.last_key_value(),
        ) {
            let rows = Price::steps_between_inclusive(*min_bid, *best_bid, tick_size).unwrap_or(0);
            (Some(*best_bid), rows)
        } else {
            (None, 0)
        };

        let spread_rows = if show_spread { 1 } else { 0 };
        let total_rows = ask_rows + spread_rows + bid_rows;

        RowMeta {
            ask_rows,
            bid_rows,
            spread_rows,
            total_rows,
            max_ask: max_ask_opt,
            best_bid: best_bid_opt,
        }
    }

    fn visible_rows(
        &self,
        bounds: Rectangle,
        asks_grouped: &BTreeMap<Price, f32>,
        bids_grouped: &BTreeMap<Price, f32>,
        tick_size: PriceStep,
        meta: &RowMeta,
        base_scroll: f32,
    ) -> (Vec<VisibleRow>, Maxima) {
        let mut y_cursor = -(base_scroll + self.scroll_px);
        let mut row_idx: usize = 0;
        let mut visible: Vec<VisibleRow> = Vec::new();

        let mut maxima = Maxima::default();

        if y_cursor < 0.0 {
            let ask_skip = ((-y_cursor) / ROW_HEIGHT).floor() as usize;
            let ask_skipped = ask_skip.min(meta.ask_rows);
            y_cursor += (ask_skipped as f32) * ROW_HEIGHT;
            row_idx += ask_skipped;

            if self.config.show_spread
                && row_idx == meta.ask_rows
                && y_cursor < 0.0
                && y_cursor + SPREAD_ROW_HEIGHT <= 0.0
            {
                y_cursor += SPREAD_ROW_HEIGHT;
                row_idx += 1;
            }

            let after_spread = meta.ask_rows + if self.config.show_spread { 1 } else { 0 };
            if row_idx >= after_spread && y_cursor < 0.0 {
                let remaining_neg = -y_cursor;
                let bid_skip = (remaining_neg / ROW_HEIGHT).floor() as usize;
                let bid_skipped = bid_skip.min(meta.bid_rows);
                y_cursor += (bid_skipped as f32) * ROW_HEIGHT;
                row_idx = (after_spread + bid_skipped).min(meta.total_rows);
            }
        }

        let mut drawn = 0usize;
        let rows_budget = ((bounds.height / ROW_HEIGHT).ceil() as usize) + 4;

        while row_idx < meta.total_rows && y_cursor < bounds.height && drawn < rows_budget {
            let pick_row = || -> (f32, DomRow) {
                if row_idx < meta.ask_rows {
                    let ma = match meta.max_ask {
                        Some(v) => v,
                        None => return (ROW_HEIGHT, DomRow::Spread),
                    };

                    let price = ma.add_steps(-(row_idx as i64), tick_size);
                    let qty = asks_grouped.get(&price).copied().unwrap_or(0.0);
                    (ROW_HEIGHT, DomRow::Ask { price, qty })
                } else if self.config.show_spread && row_idx == meta.ask_rows {
                    (SPREAD_ROW_HEIGHT, DomRow::Spread)
                } else {
                    let offset = row_idx - meta.ask_rows - meta.spread_rows;
                    let bb = match meta.best_bid {
                        Some(v) => v,
                        None => return (ROW_HEIGHT, DomRow::Spread),
                    };

                    let price = bb.add_steps(-(offset as i64), tick_size);
                    let qty = bids_grouped.get(&price).copied().unwrap_or(0.0);

                    (ROW_HEIGHT, DomRow::Bid { price, qty })
                }
            };

            let (h, row) = pick_row();
            if h <= 0.0 {
                break;
            }

            if y_cursor + h <= 0.0 {
                y_cursor += h;
                row_idx += 1;
                continue;
            }

            match row {
                DomRow::Ask { price, qty } => {
                    maxima.vis_max_order_qty = maxima.vis_max_order_qty.max(qty);

                    let (buy_t, sell_t) = self.trade_qty_at(price);
                    maxima.vis_max_trade_qty = maxima.vis_max_trade_qty.max(buy_t.max(sell_t));

                    visible.push(VisibleRow {
                        row,
                        y: y_cursor,
                        buy_t,
                        sell_t,
                    });
                }
                DomRow::Bid { price, qty } => {
                    maxima.vis_max_order_qty = maxima.vis_max_order_qty.max(qty);

                    let (buy_t, sell_t) = self.trade_qty_at(price);
                    maxima.vis_max_trade_qty = maxima.vis_max_trade_qty.max(buy_t.max(sell_t));

                    visible.push(VisibleRow {
                        row,
                        y: y_cursor,
                        buy_t,
                        sell_t,
                    });
                }
                DomRow::Spread => {
                    visible.push(VisibleRow {
                        row,
                        y: y_cursor,
                        buy_t: 0.0,
                        sell_t: 0.0,
                    });
                }
            }

            y_cursor += h;
            row_idx += 1;
            drawn += 1;
        }

        (visible, maxima)
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
        if let Some(price_text) = self.format_price(price) {
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
            size: TEXT_SIZE,
            font: style::AZERET_MONO,
            align_x: align.into(),
            align_y: Alignment::Center.into(),
            ..Default::default()
        });
    }
}

enum DomRow {
    Ask { price: Price, qty: f32 },
    Spread,
    Bid { price: Price, qty: f32 },
}
