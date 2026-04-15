use super::Message;
use crate::style;
use data::panel::tpo::{Config, TpoData};
use exchange::unit::{Price, PriceStep};
use exchange::{Kline, TickerInfo};

use iced::widget::canvas::{self, Text};
use iced::{Alignment, Event, Point, Rectangle, Renderer, Size, Theme, mouse};
use std::time::Instant;

const TEXT_SIZE: iced::Pixels = iced::Pixels(10.0);
const PRICE_LABEL_WIDTH: f32 = 64.0;
const LETTER_WIDTH: f32 = 10.0;
const ROW_HEIGHT_MAX: f32 = 14.0;
const ROW_HEIGHT_MIN: f32 = 2.0;
const SESSION_GAP: f32 = 8.0;

pub struct TpoPanel {
    data: TpoData,
    ticker_info: TickerInfo,
    pub config: Config,
    cache: canvas::Cache,
    scroll_x: f32,
    last_tick: Instant,
}

impl TpoPanel {
    pub fn new(config: Option<Config>, ticker_info: TickerInfo) -> Self {
        let cfg = config.unwrap_or_default();
        Self {
            data: TpoData::new(ticker_info, Some(cfg)),
            ticker_info,
            config: cfg,
            cache: canvas::Cache::default(),
            scroll_x: 0.0,
            last_tick: Instant::now(),
        }
    }

    pub fn insert_kline(&mut self, kline: &Kline) {
        self.data.add_kline(kline);
        self.cache.clear();
        self.last_tick = Instant::now();
    }

    pub fn load_klines(&mut self, klines: &[Kline]) {
        self.data.load_klines(klines);
        self.cache.clear();
        self.last_tick = Instant::now();
    }

    pub fn last_update(&self) -> Instant {
        self.last_tick
    }

    fn total_content_width(&self) -> f32 {
        self.data
            .sorted_profiles()
            .iter()
            .map(|p| p.periods.len() as f32 * LETTER_WIDTH + SESSION_GAP)
            .sum::<f32>()
    }
}

impl super::Panel for TpoPanel {
    fn scroll(&mut self, delta: f32) {
        let max_x = (self.total_content_width() - 100.0).max(0.0);
        self.scroll_x = (self.scroll_x + delta).clamp(-max_x, 0.0);
        self.cache.clear();
    }

    fn reset_scroll(&mut self) {
        self.scroll_x = 0.0;
        self.cache.clear();
    }

    fn invalidate(&mut self, now: Option<Instant>) -> Option<super::Action> {
        self.cache.clear();
        if let Some(t) = now {
            self.last_tick = t;
        }
        None
    }

    fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

impl canvas::Program<Message> for TpoPanel {
    type State = ();

    fn update(
        &self,
        _state: &mut Self::State,
        event: &Event,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if let Event::Mouse(mouse::Event::WheelScrolled { delta }) = event {
            let scroll = match delta {
                mouse::ScrollDelta::Lines { x, y } => (*x + *y) * 20.0,
                mouse::ScrollDelta::Pixels { x, y } => *x + *y,
            };
            return Some(canvas::Action::publish(Message::Scrolled(scroll)).and_capture());
        }
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
        let content = self.cache.draw(renderer, bounds.size(), |frame| {
            let palette = theme.extended_palette();
            let profiles = self.data.sorted_profiles();
            if profiles.is_empty() {
                return;
            }

            let price_min = profiles.iter().filter_map(|p| p.price_min).min();
            let price_max = profiles.iter().filter_map(|p| p.price_max).max();
            let (price_min, price_max) = match (price_min, price_max) {
                (Some(lo), Some(hi)) => (lo, hi),
                _ => return,
            };

            let step: PriceStep = self.ticker_info.min_ticksize.into();
            let tick_units = step.units.max(1);
            let price_levels = ((price_max - price_min) / tick_units + 1).max(1) as usize;
            let row_h =
                (bounds.height / price_levels as f32).clamp(ROW_HEIGHT_MIN, ROW_HEIGHT_MAX);

            frame.fill_rectangle(
                Point::ORIGIN,
                Size::new(PRICE_LABEL_WIDTH, bounds.height),
                palette.background.weak.color,
            );

            let mut cursor_x = PRICE_LABEL_WIDTH + self.scroll_x;

            for profile in &profiles {
                let profile_width = profile.periods.len() as f32 * LETTER_WIDTH;
                if cursor_x + profile_width < PRICE_LABEL_WIDTH || cursor_x > bounds.width {
                    cursor_x += profile_width + SESSION_GAP;
                    continue;
                }

                if self.config.show_ib {
                    if let (Some(ib_lo), Some(ib_hi)) = (profile.ib_low, profile.ib_high) {
                        let y_top = ((price_max - ib_hi) / tick_units) as f32 * row_h;
                        let y_bot = ((price_max - ib_lo) / tick_units + 1) as f32 * row_h;
                        let draw_x = cursor_x.max(PRICE_LABEL_WIDTH);
                        frame.fill_rectangle(
                            Point::new(draw_x, y_top),
                            Size::new(profile_width, (y_bot - y_top).max(0.0)),
                            palette.success.weak.color.scale_alpha(0.07),
                        );
                        for &border_y in &[y_top, y_bot] {
                            frame.fill_rectangle(
                                Point::new(draw_x, border_y),
                                Size::new(profile_width, 1.0),
                                palette.success.strong.color.scale_alpha(0.45),
                            );
                        }
                    }
                }

                if self.config.show_value_area {
                    if let (Some(va_lo), Some(va_hi)) = (profile.va_low, profile.va_high) {
                        let y_top = ((price_max - va_hi) / tick_units) as f32 * row_h;
                        let y_bot = ((price_max - va_lo) / tick_units + 1) as f32 * row_h;
                        frame.fill_rectangle(
                            Point::new(cursor_x.max(PRICE_LABEL_WIDTH), y_top),
                            Size::new(profile_width, (y_bot - y_top).max(0.0)),
                            palette.primary.weak.color.scale_alpha(0.07),
                        );
                    }
                }

                for period in &profile.periods {
                    let col_x = cursor_x + (period.letter_idx as f32 * LETTER_WIDTH);
                    if col_x + LETTER_WIDTH < PRICE_LABEL_WIDTH || col_x > bounds.width {
                        continue;
                    }
                    let letter_str = period.letter().to_string();
                    for &price_units in &period.prices {
                        let row_from_top = ((price_max - price_units) / tick_units) as f32;
                        let y = row_from_top * row_h;
                        if y + row_h < 0.0 || y > bounds.height {
                            continue;
                        }
                        let is_poc =
                            self.config.show_poc && profile.poc == Some(price_units);
                        let in_va = self.config.show_value_area
                            && profile
                                .va_low
                                .zip(profile.va_high)
                                .is_some_and(|(lo, hi)| price_units >= lo && price_units <= hi);
                        let (bg, fg) = if is_poc {
                            (palette.danger.base.color.scale_alpha(0.85), palette.danger.base.text)
                        } else if in_va {
                            (palette.primary.weak.color.scale_alpha(0.65), palette.primary.weak.text)
                        } else {
                            (palette.background.strong.color.scale_alpha(0.70), palette.background.strong.text)
                        };
                        frame.fill_rectangle(Point::new(col_x, y), Size::new(LETTER_WIDTH, row_h), bg);
                        if row_h >= 8.0 {
                            frame.fill_text(Text {
                                content: letter_str.clone(),
                                position: Point::new(col_x + LETTER_WIDTH / 2.0, y + row_h / 2.0),
                                size: TEXT_SIZE,
                                font: style::AZERET_MONO,
                                color: fg,
                                align_x: Alignment::Center.into(),
                                align_y: Alignment::Center.into(),
                                ..Default::default()
                            });
                        }
                    }
                }

                if self.config.show_poc {
                    if let Some(poc) = profile.poc {
                        let y = ((price_max - poc) / tick_units) as f32 * row_h + row_h / 2.0;
                        frame.fill_rectangle(
                            Point::new(cursor_x.max(PRICE_LABEL_WIDTH), y - 0.5),
                            Size::new(profile_width, 1.0),
                            palette.danger.strong.color,
                        );
                    }
                }

                cursor_x += profile_width + SESSION_GAP;
            }

            let label_step = {
                let ideal = (bounds.height / 20.0) as i64;
                if ideal <= 0 { 1 } else { (price_levels as i64 / ideal).max(1) }
            };
            let mut label_tick = price_min;
            while label_tick <= price_max {
                let row_from_top = (price_max - label_tick) / tick_units;
                let y = row_from_top as f32 * row_h + row_h / 2.0;
                if y >= 0.0 && y <= bounds.height {
                    frame.fill_text(Text {
                        content: Price::from_units(label_tick)
                            .to_string(self.ticker_info.min_ticksize),
                        position: Point::new(PRICE_LABEL_WIDTH - 4.0, y),
                        size: TEXT_SIZE,
                        font: style::AZERET_MONO,
                        color: palette.background.base.text,
                        align_x: Alignment::End.into(),
                        align_y: Alignment::Center.into(),
                        ..Default::default()
                    });
                }
                label_tick += label_step * tick_units;
            }
        });
        vec![content]
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        mouse::Interaction::default()
    }
}
