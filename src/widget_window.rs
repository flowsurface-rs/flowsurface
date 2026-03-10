//! Floating always-on-top mini ODB chart widget.
//! WebSocket-only — no ClickHouse dependency. Builds bars in-process
//! from live Binance `@aggTrade` trades via `OpenDeviationBarProcessor`.
// GitHub Issue: https://github.com/terrylica/flowsurface/issues/1

use crate::chart::kline::draw_candle_dp;
use crate::chart::scale::linear::PriceInfoLabel;

use exchange::adapter::clickhouse::{OpenDeviationBarProcessor, odb_to_kline, trade_to_agg_trade};
use exchange::adapter::{Exchange, StreamKind};
use exchange::unit::Price;
use exchange::{Kline, Ticker, TickerInfo, Trade};

use std::collections::HashMap;
use std::hash::BuildHasher;

use chrono::{Local, TimeZone, Utc};

use iced::widget::canvas::{self, Cache, Canvas, Frame, Path, Stroke, Text};
use iced::widget::mouse_area;
use iced::{self, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme, mouse, window};

/// The symbol string used to identify the BTCUSDT Binance Linear Perps ticker.
const BTCUSDT_SYMBOL: &str = "BTCUSDT";
/// Fixed threshold: BPR25 = 250 decimal basis points = 0.25%.
const THRESHOLD_DBPS: u32 = 250;
/// Maximum completed bars retained.
const MAX_BARS: usize = 100;
/// Cell width (pixels per bar) — same as existing BPR25 ODB rendering.
const CELL_WIDTH: f32 = 4.0;
/// Widget window dimensions.
const WINDOW_WIDTH: f32 = 600.0;
const WINDOW_HEIGHT: f32 = 350.0;

// ---------------------------------------------------------------------------
// Public message type
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum WidgetMessage {
    Drag,
}

// ---------------------------------------------------------------------------
// WidgetController — owns both chart state and window identity
// ---------------------------------------------------------------------------

pub struct WidgetController {
    state: WidgetState,
    wid: window::Id,
}

impl WidgetController {
    /// Open a new widget window and return the controller + iced task.
    pub fn open<M: Send + 'static>(ticker_info: TickerInfo) -> (Self, iced::Task<M>) {
        let (wid, task) = window::open(window::Settings {
            size: Size::new(WINDOW_WIDTH, WINDOW_HEIGHT),
            decorations: false,
            level: iced::window::Level::AlwaysOnTop,
            resizable: false,
            exit_on_close_request: false,
            ..Default::default()
        });
        let controller = Self {
            state: WidgetState::new(ticker_info),
            wid,
        };
        (controller, task.discard())
    }

    /// Close the widget window and return the iced task.
    pub fn close<M: Send + 'static>(self) -> iced::Task<M> {
        window::close(self.wid)
    }

    pub fn window_id(&self) -> window::Id {
        self.wid
    }

    /// Forward trades from a WebSocket event to the widget if the stream is BTCUSDT.
    pub fn forward_trades(&mut self, stream: &StreamKind, trades: &[Trade]) {
        let ti = stream.ticker_info();
        if Self::is_btcusdt(&ti.ticker) {
            self.state.insert_trades(trades);
        } else {
            // Diagnostic: log non-matching streams to understand routing
            log::trace!(
                "[widget] forward_trades: non-BTCUSDT stream, exchange={:?} symbol={}",
                ti.ticker.exchange,
                ti.ticker.to_full_symbol_and_type().0,
            );
        }
    }

    /// Render the widget canvas wrapped in a drag-triggering mouse area.
    pub fn view(&self) -> Element<'_, WidgetMessage> {
        mouse_area(
            Canvas::new(&self.state)
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .on_press(WidgetMessage::Drag)
        .into()
    }

    /// Check whether a ticker is BTCUSDT on any Binance exchange (Spot or Linear).
    pub fn is_btcusdt(ticker: &Ticker) -> bool {
        matches!(
            ticker.exchange,
            Exchange::BinanceLinear | Exchange::BinanceSpot
        ) && ticker.to_full_symbol_and_type().0.contains(BTCUSDT_SYMBOL)
    }

    /// Find a BTCUSDT BinanceLinear TickerInfo from the available ticker map.
    pub fn find_btcusdt_ticker_info<S: BuildHasher>(
        tickers: &HashMap<Ticker, Option<TickerInfo>, S>,
    ) -> Option<TickerInfo> {
        tickers.iter().find_map(|(ticker, info)| {
            if Self::is_btcusdt(ticker) {
                info.as_ref().copied()
            } else {
                None
            }
        })
    }
}

// ---------------------------------------------------------------------------
// WidgetState — chart data + canvas rendering (private)
// ---------------------------------------------------------------------------

struct WidgetState {
    bars: Vec<Kline>,
    processor: OpenDeviationBarProcessor,
    next_agg_id: i64,
    last_price: Option<PriceInfoLabel>,
    last_trade_time: Option<u64>,
    cache: Cache,
    ticker_info: TickerInfo,
}

impl WidgetState {
    fn new(ticker_info: TickerInfo) -> Self {
        let processor = OpenDeviationBarProcessor::new(THRESHOLD_DBPS)
            .expect("BPR25 threshold is always valid");
        Self {
            bars: Vec::with_capacity(MAX_BARS),
            processor,
            next_agg_id: 0,
            last_price: None,
            last_trade_time: None,
            cache: Cache::new(),
            ticker_info,
        }
    }

    fn insert_trades(&mut self, trades: &[Trade]) {
        let min_tick = self.ticker_info.min_ticksize;

        for trade in trades {
            let agg = trade_to_agg_trade(trade, self.next_agg_id);
            self.next_agg_id += 1;

            match self.processor.process_single_trade(&agg) {
                Ok(Some(completed)) => {
                    let kline = odb_to_kline(&completed, min_tick);
                    self.bars.push(kline);
                    if self.bars.len() > MAX_BARS {
                        self.bars.remove(0);
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    log::warn!("[widget] OpenDeviationBarProcessor error: {e}");
                    exchange::tg_alert!(
                        exchange::telegram::Severity::Info,
                        "widget",
                        "Widget ODB processor error"
                    );
                }
            }
        }

        // Use the exchange-reported trade price directly — keeps the widget
        // in sync with the main chart regardless of processor state.
        if let Some(last_trade) = trades.last() {
            self.last_trade_time = Some(last_trade.time);
            let prev_close = self.bars.last().map(|k| k.close);
            let reference = prev_close.unwrap_or(last_trade.price);
            self.last_price = Some(PriceInfoLabel::new(last_trade.price, reference));
            log::trace!(
                "[PRICE/widget] trade_time={} price={:.2} trades_in_batch={}",
                last_trade.time,
                last_trade.price.to_f32(),
                trades.len(),
            );
        }

        self.cache.clear();
    }

    fn price_range(&self) -> (f32, f32) {
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;

        for k in &self.bars {
            let k_low = (k.low.units as f64 / 1e8) as f32;
            let k_high = (k.high.units as f64 / 1e8) as f32;
            lo = lo.min(k_low);
            hi = hi.max(k_high);
        }

        if let Some(forming) = self.processor.get_incomplete_bar() {
            lo = lo.min(forming.low.to_f64() as f32);
            hi = hi.max(forming.high.to_f64() as f32);
        }

        if lo == f32::MAX { (0.0, 1.0) } else { (lo, hi) }
    }
}

impl canvas::Program<WidgetMessage> for WidgetState {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            let palette = theme.extended_palette();

            frame.fill_rectangle(Point::ORIGIN, bounds.size(), palette.background.base.color);

            let header_h = 34.0;
            let margin = 6.0;

            if let Some(label) = self.last_price {
                let (price, color) = label.get_with_color(palette);
                let price_f64 = price.units as f64 / 1e8;
                let price_text = format!("BTC ${:.2}", price_f64);
                frame.fill_text(Text {
                    content: price_text,
                    position: Point::new(margin, margin),
                    color,
                    size: 16.0.into(),
                    ..Default::default()
                });

                if let Some(trade_ms) = self.last_trade_time {
                    let secs = (trade_ms / 1000) as i64;
                    let millis = (trade_ms % 1000) as u32;
                    let utc_dt = Utc.timestamp_opt(secs, millis * 1_000_000).single();
                    let local_dt = Local.timestamp_opt(secs, millis * 1_000_000).single();
                    if let (Some(u), Some(l)) = (utc_dt, local_dt) {
                        let ts_text = format!(
                            "{}.{:03} UTC  /  {}.{:03} local",
                            u.format("%H:%M:%S"),
                            millis,
                            l.format("%H:%M:%S"),
                            millis,
                        );
                        frame.fill_text(Text {
                            content: ts_text,
                            position: Point::new(margin, margin + 16.0),
                            color: palette.secondary.strong.color,
                            size: 10.0.into(),
                            ..Default::default()
                        });
                    }
                }
            } else {
                frame.fill_text(Text {
                    content: "Waiting for trades...".to_string(),
                    position: Point::new(margin, margin),
                    color: palette.secondary.strong.color,
                    size: 14.0.into(),
                    ..Default::default()
                });
            }

            frame.fill_text(Text {
                content: "BPR25".to_string(),
                position: Point::new(bounds.width - 50.0, margin),
                color: palette.secondary.strong.color,
                size: 12.0.into(),
                ..Default::default()
            });

            if self.bars.is_empty() && self.processor.get_incomplete_bar().is_none() {
                return;
            }

            let (min_price, max_price) = self.price_range();
            let padding_pct = 0.05;
            let price_span = max_price - min_price;
            let p_high = max_price + price_span * padding_pct;
            let p_low = min_price - price_span * padding_pct;
            let p_range = (p_high - p_low).max(0.01);

            let chart_top = header_h;
            let chart_height = bounds.height - chart_top - 4.0;

            let price_to_y = |price: Price| -> f32 {
                let p = price.units as f64 / 1e8;
                chart_top + ((p_high as f64 - p) / p_range as f64 * chart_height as f64) as f32
            };

            let price_f32_to_y =
                |p: f32| -> f32 { chart_top + (p_high - p) / p_range * chart_height };

            let candle_width = CELL_WIDTH * 0.8;
            let right_edge = bounds.width - margin - CELL_WIDTH;

            for (i, kline) in self.bars.iter().rev().enumerate() {
                let x = right_edge - (i as f32 * CELL_WIDTH);
                if x < margin {
                    break;
                }
                draw_candle_dp(
                    frame,
                    price_to_y,
                    candle_width,
                    palette,
                    x,
                    kline,
                    None,
                    None,
                );
            }

            if let Some(forming) = self.processor.get_incomplete_bar() {
                let x_forming = right_edge + CELL_WIDTH;

                let open_f32 = forming.open.to_f64() as f32;
                let high_f32 = forming.high.to_f64() as f32;
                let low_f32 = forming.low.to_f64() as f32;
                let close_f32 = forming.close.to_f64() as f32;

                let direction_color = if close_f32 >= open_f32 {
                    palette.success.base.color
                } else {
                    palette.danger.base.color
                };
                let forming_color = Color {
                    a: 0.4,
                    ..direction_color
                };

                let y_open = price_f32_to_y(open_f32);
                let y_close = price_f32_to_y(close_f32);
                let y_high = price_f32_to_y(high_f32);
                let y_low = price_f32_to_y(low_f32);

                frame.fill_rectangle(
                    Point::new(x_forming - candle_width / 2.0, y_open.min(y_close)),
                    Size::new(candle_width, (y_open - y_close).abs().max(1.0)),
                    forming_color,
                );
                frame.fill_rectangle(
                    Point::new(x_forming - candle_width / 8.0, y_high),
                    Size::new(candle_width / 4.0, (y_high - y_low).abs().max(1.0)),
                    forming_color,
                );
            }

            if let Some(label) = self.last_price {
                let (price, color) = label.get_with_color(palette);
                let y = price_to_y(price);
                let dash_color = Color { a: 0.5, ..color };
                draw_dashed_hline(frame, y, margin, bounds.width - margin, dash_color);
            }
        });

        vec![geometry]
    }
}

fn draw_dashed_hline(frame: &mut Frame, y: f32, x_start: f32, x_end: f32, color: Color) {
    frame.stroke(
        &Path::line(Point::new(x_start, y), Point::new(x_end, y)),
        Stroke {
            width: 1.0,
            style: iced::widget::canvas::Style::Solid(color),
            line_dash: iced::widget::canvas::LineDash {
                segments: &[3.0, 3.0],
                offset: 0,
            },
            ..Default::default()
        },
    );
}
