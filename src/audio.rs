use crate::style;
use crate::widget::create_slider_row;
use data::audio::{SoundCache, StreamCfg};
use exchange::adapter::{Exchange, StreamType};

use iced::Element;
use iced::widget::{button, column, container, row, text};
use iced::widget::{checkbox, horizontal_space, slider};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum Message {
    SoundLevelChanged(f32),
    ToggleStream(bool, (Exchange, exchange::Ticker)),
}

pub enum Action {
    None,
    Select,
}

pub struct AudioStream {
    pub cache: SoundCache,
    pub streams: HashMap<Exchange, HashMap<exchange::Ticker, StreamCfg>>,
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
        }
    }

    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::SoundLevelChanged(value) => {
                self.cache.set_sound_level(value);
            }
            Message::ToggleStream(is_checked, (exchange, ticker)) => {
                if is_checked {
                    self.streams
                        .entry(exchange)
                        .or_default()
                        .insert(ticker, StreamCfg::default());
                } else if let Some(streams) = self.streams.get_mut(&exchange) {
                    if let Some(cfg) = streams.get_mut(&ticker) {
                        cfg.enabled = false;
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

        let mut streams_col = column![].spacing(4);

        if !active_streams.is_empty() {
            for (exchange, ticker) in active_streams {
                let is_stream_audio_enabled = self
                    .streams
                    .get(&exchange)
                    .and_then(|streams| streams.get(&ticker))
                    .is_some_and(|cfg| cfg.enabled);

                let stream_checkbox =
                    checkbox(format!("{exchange} - {ticker}"), is_stream_audio_enabled).on_toggle(
                        move |is_checked| Message::ToggleStream(is_checked, (exchange, ticker)),
                    );

                streams_col = streams_col.push(
                    row![
                        stream_checkbox,
                        horizontal_space(),
                        button("+").style(move |theme, status| {
                            style::button::transparent(theme, status, false)
                        })
                    ]
                    .padding(2)
                    .spacing(4),
                );
            }
        } else {
            streams_col = streams_col.push(text("No trade streams found"));
        }

        container(
            column![
                column![text("Sound").size(14), volume_slider,].spacing(8),
                column![text(format!("Audio streams")).size(14), streams_col,].spacing(8),
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
            true
        } else {
            false
        }
    }
}

impl From<&AudioStream> for data::AudioStream {
    fn from(audio_stream: &AudioStream) -> Self {
        let mut streams = HashMap::new();

        for (&exchange, ticker_map) in &audio_stream.streams {
            for (&ticker, stream_cfg) in ticker_map {
                let exchange_ticker = exchange::ExchangeTicker::from_parts(exchange, ticker);
                streams.insert(exchange_ticker, *stream_cfg);
            }
        }

        data::AudioStream {
            volume: audio_stream.cache.get_volume(),
            streams,
        }
    }
}
