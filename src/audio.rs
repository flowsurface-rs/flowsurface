use crate::TooltipPosition;
use crate::style::{self, get_icon_text};
use crate::widget::{create_slider_row, tooltip};
use data::audio::{SoundCache, StreamCfg};
use exchange::adapter::{Exchange, StreamType};

use exchange::Trade;
use iced::widget::{button, column, container, row, text};
use iced::widget::{checkbox, horizontal_space, slider};
use iced::{Element, padding};
use std::collections::HashMap;

const HARD_THRESHOLD: usize = 4;

#[derive(Debug, Clone)]
pub enum Message {
    SoundLevelChanged(f32),
    ToggleStream(bool, (Exchange, exchange::Ticker)),
    ToggleCard(Exchange, exchange::Ticker),
    SetThreshold(Exchange, exchange::Ticker, data::audio::Threshold),
}

pub enum Action {
    None,
    Select,
}

pub struct AudioStream {
    cache: SoundCache,
    streams: HashMap<Exchange, HashMap<exchange::Ticker, StreamCfg>>,
    expanded_card: Option<(Exchange, exchange::Ticker)>,
}

impl AudioStream {
    pub fn new(cfg: data::AudioStream) -> Self {
        let mut streams: HashMap<Exchange, HashMap<exchange::Ticker, StreamCfg>> = HashMap::new();

        for (exchange_ticker, stream_cfg) in cfg.streams {
            let exchange = exchange_ticker.exchange;
            let ticker = exchange_ticker.ticker;

            streams
                .entry(exchange)
                .or_default()
                .insert(ticker, stream_cfg);
        }

        AudioStream {
            cache: SoundCache::with_default_sounds(cfg.volume)
                .expect("Failed to create sound cache"),
            streams,
            expanded_card: None,
        }
    }

    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::SoundLevelChanged(value) => {
                self.cache.set_volume(value);
            }
            Message::ToggleStream(is_checked, (exchange, ticker)) => {
                if is_checked {
                    if let Some(streams) = self.streams.get_mut(&exchange) {
                        if let Some(cfg) = streams.get_mut(&ticker) {
                            cfg.enabled = true;
                        } else {
                            streams.insert(ticker, StreamCfg::default());
                        }
                    } else {
                        self.streams
                            .entry(exchange)
                            .or_default()
                            .insert(ticker, StreamCfg::default());
                    }
                } else if let Some(streams) = self.streams.get_mut(&exchange) {
                    if let Some(cfg) = streams.get_mut(&ticker) {
                        cfg.enabled = false;
                    }
                } else {
                    self.streams
                        .entry(exchange)
                        .or_default()
                        .insert(ticker, StreamCfg::default());
                }
            }
            Message::ToggleCard(exchange, ticker) => {
                self.expanded_card = match self.expanded_card {
                    Some((ex, tk)) if ex == exchange && tk == ticker => None,
                    _ => Some((exchange, ticker)),
                };
            }
            Message::SetThreshold(exchange, ticker, threshold) => {
                if let Some(streams) = self.streams.get_mut(&exchange) {
                    if let Some(cfg) = streams.get_mut(&ticker) {
                        cfg.threshold = threshold;
                    }
                }
            }
        }

        Action::None
    }

    pub fn view(&self, active_streams: Vec<(Exchange, exchange::Ticker)>) -> Element<'_, Message> {
        let volume_slider = {
            let volume_pct = self.cache.get_volume().unwrap_or(0.0);

            create_slider_row(
                text("Volume"),
                slider(0.0..=100.0, volume_pct, move |value| {
                    Message::SoundLevelChanged(value)
                })
                .step(1.0)
                .into(),
                text(format!("{volume_pct}%")).size(13),
            )
        };

        let mut content = column![].spacing(4);

        if !active_streams.is_empty() {
            for (exchange, ticker) in active_streams {
                let mut column = column![].padding(padding::left(4));

                let is_audio_enabled =
                    self.is_stream_audio_enabled(&StreamType::DepthAndTrades { exchange, ticker });

                let stream_checkbox = checkbox(format!("{exchange} - {ticker}"), is_audio_enabled)
                    .on_toggle(move |is_checked| {
                        Message::ToggleStream(is_checked, (exchange, ticker))
                    });

                let is_expanded = self
                    .expanded_card
                    .is_some_and(|(ex, tk)| ex == exchange && tk == ticker);

                let stream_row = row![
                    stream_checkbox,
                    horizontal_space(),
                    tooltip(
                        button(get_icon_text(style::Icon::Cog, 12))
                            .on_press(Message::ToggleCard(exchange, ticker))
                            .style(move |theme, status| {
                                style::button::transparent(theme, status, is_expanded)
                            }),
                        Some("Toggle filters for triggering a sound"),
                        TooltipPosition::Top,
                    )
                ]
                .align_y(iced::Alignment::Center)
                .padding(4)
                .spacing(4);

                column = column.push(stream_row);

                if is_expanded {
                    if let Some(cfg) = self.streams.get(&exchange).and_then(|s| s.get(&ticker)) {
                        match cfg.threshold {
                            data::audio::Threshold::Count(v) => {
                                let threshold_slider =
                                    slider(1.0..=100.0, v as f32, move |value| {
                                        Message::SetThreshold(
                                            exchange,
                                            ticker,
                                            data::audio::Threshold::Count(value as usize),
                                        )
                                    });

                                column = column.push(
                                    column![
                                        text(format!("Buy/sell trade count in buffer ≥ {}", v)),
                                        threshold_slider
                                    ]
                                    .padding(8)
                                    .spacing(4),
                                );
                            }
                            data::audio::Threshold::Qty(v) => {
                                column = column.push(
                                    row![text(format!("Any trade's size in buffer ≥ {}", v))]
                                        .padding(8)
                                        .spacing(4),
                                );
                            }
                        }
                    }
                }

                content = content.push(container(column).style(style::modal_container));
            }
        } else {
            content = content.push(text("No trade streams found"));
        }

        container(
            column![
                column![text("Sound").size(14), volume_slider,].spacing(8),
                column![text(format!("Audio streams")).size(14), content,].spacing(8),
            ]
            .spacing(20),
        )
        .max_width(320)
        .padding(24)
        .style(style::dashboard_modal)
        .into()
    }

    pub fn get_volume(&self) -> Option<f32> {
        self.cache.get_volume()
    }

    pub fn play(&self, sound: &str) -> Result<(), String> {
        self.cache.play(sound)
    }

    pub fn is_stream_audio_enabled(&self, stream: &StreamType) -> bool {
        match stream {
            StreamType::DepthAndTrades { exchange, ticker } => self
                .streams
                .get(exchange)
                .and_then(|streams| streams.get(ticker))
                .is_some_and(|cfg| cfg.enabled),
            _ => false,
        }
    }

    pub fn should_play_sound(&self, stream: &StreamType) -> Option<StreamCfg> {
        let StreamType::DepthAndTrades { exchange, ticker } = stream else {
            return None;
        };

        match self
            .streams
            .get(exchange)
            .and_then(|streams| streams.get(ticker))
        {
            Some(cfg) if cfg.enabled => Some(*cfg),
            _ => None,
        }
    }

    pub fn try_play_sound(
        &self,
        stream: &StreamType,
        trades_buffer: &[Trade],
    ) -> Result<(), String> {
        let Some(cfg) = self.should_play_sound(stream) else {
            return Ok(());
        };

        match cfg.threshold {
            data::audio::Threshold::Count(v) => {
                let (buy_count, sell_count) =
                    trades_buffer.iter().fold((0, 0), |(buy_c, sell_c), trade| {
                        if trade.is_sell {
                            (buy_c, sell_c + 1)
                        } else {
                            (buy_c + 1, sell_c)
                        }
                    });

                if buy_count < v && sell_count < v {
                    return Ok(());
                }

                let sound = |count: usize, is_sell: bool| {
                    if count > (v * HARD_THRESHOLD) {
                        if is_sell {
                            data::audio::HARD_SELL_SOUND
                        } else {
                            data::audio::HARD_BUY_SOUND
                        }
                    } else if is_sell {
                        data::audio::SELL_SOUND
                    } else {
                        data::audio::BUY_SOUND
                    }
                };

                match buy_count.cmp(&sell_count) {
                    std::cmp::Ordering::Greater => {
                        self.play(sound(buy_count, false))?;
                    }
                    std::cmp::Ordering::Less => {
                        self.play(sound(sell_count, true))?;
                    }
                    std::cmp::Ordering::Equal => {
                        self.play(sound(buy_count, false))?;
                        self.play(sound(sell_count, true))?;
                    }
                }
            }
            data::audio::Threshold::Qty(_) => {
                unimplemented!()
            }
        }

        Ok(())
    }
}

impl From<&AudioStream> for data::AudioStream {
    fn from(audio_stream: &AudioStream) -> Self {
        let mut streams = HashMap::new();

        for (&exchange, ticker_map) in &audio_stream.streams {
            for (&ticker, cfg) in ticker_map {
                let exchange_ticker = exchange::SerTicker::from_parts(exchange, ticker);
                streams.insert(exchange_ticker, *cfg);
            }
        }

        data::AudioStream {
            volume: audio_stream.cache.get_volume(),
            streams,
        }
    }
}
