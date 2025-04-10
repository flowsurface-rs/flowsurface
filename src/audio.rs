use crate::style::{self, get_icon_text};
use crate::widget::create_slider_row;
use data::audio::{SoundCache, StreamCfg};
use exchange::adapter::{Exchange, StreamType};

use iced::widget::{button, column, container, row, text};
use iced::widget::{checkbox, horizontal_space, slider};
use iced::{Element, padding};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum Message {
    SoundLevelChanged(f32),
    ToggleStream(bool, (Exchange, exchange::Ticker)),
    ToggleCard(Exchange, exchange::Ticker),
}

pub enum Action {
    None,
    Select,
}

pub struct AudioStream {
    pub cache: SoundCache,
    pub streams: HashMap<Exchange, HashMap<exchange::Ticker, StreamCfg>>,
    pub expanded_card: Option<(Exchange, exchange::Ticker)>,
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
                self.cache.set_sound_level(value);
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

                let stream_row = row![
                    stream_checkbox,
                    horizontal_space(),
                    button(get_icon_text(style::Icon::Cog, 12))
                        .on_press(Message::ToggleCard(exchange, ticker))
                        .style(move |theme, status| {
                            style::button::transparent(theme, status, false)
                        })
                ]
                .align_y(iced::Alignment::Center)
                .padding(4)
                .spacing(4);

                column = column.push(stream_row);

                if self.expanded_card == Some((exchange, ticker)) {
                    if let Some(cfg) = self.streams.get(&exchange).and_then(|s| s.get(&ticker)) {
                        column = column.push(
                            row![text(format!("Threshold: {}", cfg.threshold))]
                                .padding(8)
                                .spacing(4),
                        );
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
        if let StreamType::DepthAndTrades { exchange, ticker } = stream {
            if let Some(streams) = self.streams.get(exchange) {
                if let Some(cfg) = streams.get(ticker) {
                    return cfg.enabled;
                }
            }
        }

        false
    }
}

impl From<&AudioStream> for data::AudioStream {
    fn from(audio_stream: &AudioStream) -> Self {
        let mut streams = HashMap::new();

        for (&exchange, ticker_map) in &audio_stream.streams {
            for (&ticker, cfg) in ticker_map {
                let exchange_ticker = exchange::ExchangeTicker::from_parts(exchange, ticker);
                streams.insert(exchange_ticker, *cfg);
            }
        }

        data::AudioStream {
            volume: audio_stream.cache.get_volume(),
            streams,
        }
    }
}
