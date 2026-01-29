use crate::TooltipPosition;
use crate::audio::{AudioError, SoundCache, SoundType};
use crate::style::{self, icon_text};
use crate::widget::{labeled_slider, tooltip};
use data::audio::StreamCfg;
use exchange::adapter::{Exchange, StreamKind, StreamTicksize};

use exchange::{PushFrequency, Trade};
use iced::widget::{button, column, container, row, text};
use iced::widget::{checkbox, slider, space};
use iced::{Element, padding};
use rustc_hash::FxHashMap;
use std::collections::HashMap;

const HARD_THRESHOLD: usize = 4;

#[derive(Debug, Clone)]
pub enum UpdateEvent {
    RetryFailed(String),
    RetrySucceeded,
}

#[derive(Debug, Clone, Copy)]
pub enum Message {
    SoundLevelChanged(f32),
    ToggleStream(bool, (Exchange, exchange::Ticker)),
    ToggleCard(Exchange, exchange::Ticker),
    SetThreshold(Exchange, exchange::Ticker, data::audio::Threshold),
    RetryInit,
}

pub struct AudioStream {
    cache: Option<SoundCache>,
    volume: Option<f32>,
    init_error: Option<AudioError>,
    streams: HashMap<Exchange, HashMap<exchange::Ticker, StreamCfg>>,
    expanded_card: Option<(Exchange, exchange::Ticker)>,
}

impl AudioStream {
    /// Returns `(AudioStream, init_error)`
    /// where `init_error` is `Some` if audio output couldn't be initialized
    pub fn new(cfg: data::AudioStream) -> (Self, Option<String>) {
        let mut streams: HashMap<Exchange, HashMap<exchange::Ticker, StreamCfg>> = HashMap::new();

        for (exchange_ticker, stream_cfg) in cfg.streams {
            let exchange = exchange_ticker.exchange;
            let ticker = exchange_ticker.ticker;

            streams
                .entry(exchange)
                .or_default()
                .insert(ticker, stream_cfg);
        }

        let volume = cfg.volume;

        match SoundCache::with_default_sounds(volume) {
            Ok(cache) => (
                AudioStream {
                    cache: Some(cache),
                    volume,
                    init_error: None,
                    streams,
                    expanded_card: None,
                },
                None,
            ),
            Err(err) => {
                let err_msg = err.to_string();
                log::error!("Audio initialization error: {err_msg}");
                (
                    AudioStream {
                        cache: None,
                        volume,
                        init_error: Some(err),
                        streams,
                        expanded_card: None,
                    },
                    Some(err_msg),
                )
            }
        }
    }

    pub fn update(&mut self, message: Message) -> Option<UpdateEvent> {
        match message {
            Message::SoundLevelChanged(value) => {
                if value == 0.0 {
                    self.volume = None;
                } else {
                    self.volume = Some(value.clamp(0.0, 100.0));
                }

                if let Some(cache) = &mut self.cache {
                    cache.set_volume(value);
                }
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
                if let Some(streams) = self.streams.get_mut(&exchange)
                    && let Some(cfg) = streams.get_mut(&ticker)
                {
                    cfg.threshold = threshold;
                }
            }
            Message::RetryInit => match SoundCache::with_default_sounds(self.volume) {
                Ok(cache) => {
                    self.cache = Some(cache);
                    self.init_error = None;
                    return Some(UpdateEvent::RetrySucceeded);
                }
                Err(err) => {
                    let err_msg = err.to_string();

                    self.cache = None;
                    self.init_error = Some(err);
                    return Some(UpdateEvent::RetryFailed(err_msg));
                }
            },
        }
        None
    }

    pub fn view(
        &self,
        active_streams: Vec<(exchange::TickerInfo, StreamTicksize, PushFrequency)>,
    ) -> Element<'_, Message> {
        let audio_output_error = self.init_error.as_ref().map(|err| {
            container(
                column![
                    text("Audio output unavailable").size(14),
                    text(err.to_string()).size(12),
                    button(text("Retry")).on_press(Message::RetryInit),
                ]
                .spacing(8),
            )
            .style(style::modal_container)
            .padding(8)
        });

        let volume_container = {
            let volume_slider = {
                let volume_pct = self.volume.unwrap_or(0.0);

                labeled_slider(
                    "Volume",
                    0.0..=100.0,
                    volume_pct,
                    Message::SoundLevelChanged,
                    |value| format!("{value}%"),
                    Some(1.0),
                )
            };

            column![text("Sound").size(14), volume_slider,].spacing(8)
        };

        let audio_contents = {
            let mut available_streams = column![].spacing(4);

            if active_streams.is_empty() {
                available_streams = available_streams.push(text("No trade streams found"));
            } else {
                // de-dup by (exchange, ticker), ignoring depth_aggr
                let mut streams = active_streams;
                let mut seen = Vec::with_capacity(streams.len());
                streams.retain(|pair| {
                    let t = pair.0;
                    let key = (t.exchange(), t);
                    if seen.contains(&key) {
                        false
                    } else {
                        seen.push(key);
                        true
                    }
                });

                for (ticker_info, depth_aggr, _) in streams {
                    let exchange = ticker_info.exchange();
                    let ticker = ticker_info.ticker;

                    let mut column = column![].padding(padding::left(4));

                    let is_audio_enabled =
                        self.is_stream_audio_enabled(&StreamKind::DepthAndTrades {
                            ticker_info,
                            depth_aggr,
                            push_freq: PushFrequency::ServerDefault,
                        });

                    let stream_checkbox = checkbox(is_audio_enabled)
                        .label(format!("{exchange} - {ticker}"))
                        .on_toggle(move |is_checked| {
                            Message::ToggleStream(is_checked, (exchange, ticker))
                        });

                    let mut stream_row = row![stream_checkbox, space::horizontal(),]
                        .height(36)
                        .align_y(iced::Alignment::Center)
                        .padding(4)
                        .spacing(4);

                    let is_expanded = self
                        .expanded_card
                        .is_some_and(|(ex, tk)| ex == exchange && tk == ticker);

                    if is_audio_enabled {
                        stream_row = stream_row.push(tooltip(
                            button(icon_text(style::Icon::Cog, 12))
                                .on_press(Message::ToggleCard(exchange, ticker))
                                .style(move |theme, status| {
                                    style::button::transparent(theme, status, is_expanded)
                                }),
                            Some("Toggle filters for triggering a sound"),
                            TooltipPosition::Top,
                        ));
                    }

                    column = column.push(stream_row);

                    if is_expanded
                        && is_audio_enabled
                        && let Some(cfg) = self.streams.get(&exchange).and_then(|s| s.get(&ticker))
                    {
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

                    available_streams =
                        available_streams.push(container(column).style(style::modal_container));
                }
            }

            column![text("Audio streams").size(14), available_streams,].spacing(8)
        };

        container(if let Some(output_error) = audio_output_error {
            column![output_error]
        } else {
            column![volume_container, audio_contents,].spacing(20)
        })
        .max_width(320)
        .padding(24)
        .style(style::dashboard_modal)
        .into()
    }

    pub fn volume(&self) -> Option<f32> {
        self.volume
    }

    pub fn play(&mut self, sound: SoundType) -> Result<(), AudioError> {
        let Some(cache) = &mut self.cache else {
            return Ok(());
        };
        cache.play(sound)
    }

    pub fn is_stream_audio_enabled(&self, stream: &StreamKind) -> bool {
        match stream {
            StreamKind::DepthAndTrades { ticker_info, .. } => self
                .streams
                .get(&ticker_info.exchange())
                .and_then(|streams| streams.get(&ticker_info.ticker))
                .is_some_and(|cfg| cfg.enabled),
            _ => false,
        }
    }

    pub fn should_play_sound(&self, stream: &StreamKind) -> Option<StreamCfg> {
        if self.cache.is_none() || self.volume.is_none() {
            return None;
        }

        let StreamKind::DepthAndTrades { ticker_info, .. } = stream else {
            return None;
        };

        match self
            .streams
            .get(&ticker_info.exchange())
            .and_then(|streams| streams.get(&ticker_info.ticker))
        {
            Some(cfg) if cfg.enabled => Some(*cfg),
            _ => None,
        }
    }

    /// Attempts to play sounds;
    /// on any audio error, disables audio and returns error message to show to user
    pub fn try_play_sound(
        &mut self,
        stream: &StreamKind,
        trades_buffer: &[Trade],
    ) -> Option<String> {
        let cfg = self.should_play_sound(stream)?;

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
                    return None;
                }

                let sound = |count: usize, is_sell: bool| {
                    if count > (v * HARD_THRESHOLD) {
                        if is_sell {
                            SoundType::HardSell
                        } else {
                            SoundType::HardBuy
                        }
                    } else if is_sell {
                        SoundType::Sell
                    } else {
                        SoundType::Buy
                    }
                };

                let play_one = |this: &mut Self, s: SoundType| -> Option<String> {
                    match this.play(s) {
                        Ok(()) => None,
                        Err(err) => {
                            let msg = err.to_string();
                            log::error!("Audio play error: {msg}");

                            if this.disable_audio(err) {
                                Some(format!("Audio disabled: {msg}"))
                            } else {
                                None
                            }
                        }
                    }
                };

                match buy_count.cmp(&sell_count) {
                    std::cmp::Ordering::Greater => play_one(self, sound(buy_count, false)),
                    std::cmp::Ordering::Less => play_one(self, sound(sell_count, true)),
                    std::cmp::Ordering::Equal => play_one(self, sound(buy_count, false))
                        .or_else(|| play_one(self, sound(sell_count, true))),
                }
            }
            data::audio::Threshold::Qty(_) => todo!(),
        }
    }

    /// Disables audio and stores the real error. Returns `true` if we just transitioned
    fn disable_audio(&mut self, err: AudioError) -> bool {
        let was_enabled = self.cache.is_some();
        self.cache = None;
        self.init_error = Some(err);
        was_enabled
    }
}

impl From<&AudioStream> for data::AudioStream {
    fn from(audio_stream: &AudioStream) -> Self {
        let mut streams = FxHashMap::default();

        for ticker_map in audio_stream.streams.values() {
            for (&ticker, cfg) in ticker_map {
                let exchange_ticker = exchange::SerTicker::from_parts(ticker);
                streams.insert(exchange_ticker, *cfg);
            }
        }

        data::AudioStream {
            volume: audio_stream.volume,
            streams,
        }
    }
}
