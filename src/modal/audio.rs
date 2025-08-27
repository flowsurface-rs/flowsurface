use crate::TooltipPosition;
use crate::style::{self, icon_text};
use crate::widget::{labeled_slider, tooltip};
use data::audio::{SoundCache, SoundType, StreamCfg};
use exchange::adapter::{Exchange, StreamKind, StreamTicksize};

use exchange::Trade;
use iced::widget::{button, column, container, row, text};
use iced::widget::{checkbox, horizontal_space, slider};
use iced::{Element, padding};
use rustc_hash::FxHashMap;
use std::collections::HashMap;

const HARD_THRESHOLD: usize = 4;

#[derive(Debug, Clone, Copy)]
pub enum Message {
    SoundLevelChanged(f32),
    ToggleStream(bool, (Exchange, exchange::Ticker)),
    ToggleCard(Exchange, exchange::Ticker),
    SetThreshold(Exchange, exchange::Ticker, data::audio::Threshold),
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

    pub fn update(&mut self, message: Message) {
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
                if let Some(streams) = self.streams.get_mut(&exchange)
                    && let Some(cfg) = streams.get_mut(&ticker)
                {
                    cfg.threshold = threshold;
                }
            }
        }
    }

    pub fn view(
        &self,
        active_streams: Vec<(exchange::Ticker, StreamTicksize)>,
    ) -> Element<'_, Message> {
        let volume_container = {
            let volume_slider = {
                let volume_pct = self.cache.get_volume().unwrap_or(0.0);

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
                    let key = (t.exchange, t);
                    if seen.contains(&key) {
                        false
                    } else {
                        seen.push(key);
                        true
                    }
                });

                for (ticker, depth_aggr) in streams {
                    let exchange = ticker.exchange;

                    let mut column = column![].padding(padding::left(4));

                    let is_audio_enabled =
                        self.is_stream_audio_enabled(&StreamKind::DepthAndTrades {
                            ticker,
                            depth_aggr,
                        });

                    let stream_checkbox =
                        checkbox(format!("{exchange} - {ticker}"), is_audio_enabled).on_toggle(
                            move |is_checked| Message::ToggleStream(is_checked, (exchange, ticker)),
                        );

                    let mut stream_row = row![stream_checkbox, horizontal_space(),]
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

        container(column![volume_container, audio_contents,].spacing(20))
            .max_width(320)
            .padding(24)
            .style(style::dashboard_modal)
            .into()
    }

    pub fn volume(&self) -> Option<f32> {
        self.cache.get_volume()
    }

    pub fn play(&mut self, sound: SoundType) -> Result<(), String> {
        self.cache.play(sound)
    }

    pub fn is_stream_audio_enabled(&self, stream: &StreamKind) -> bool {
        match stream {
            StreamKind::DepthAndTrades { ticker, .. } => self
                .streams
                .get(&ticker.exchange)
                .and_then(|streams| streams.get(ticker))
                .is_some_and(|cfg| cfg.enabled),
            _ => false,
        }
    }

    pub fn should_play_sound(&self, stream: &StreamKind) -> Option<StreamCfg> {
        if self.cache.is_muted() {
            return None;
        }

        let StreamKind::DepthAndTrades { ticker, .. } = stream else {
            return None;
        };

        match self
            .streams
            .get(&ticker.exchange)
            .and_then(|streams| streams.get(ticker))
        {
            Some(cfg) if cfg.enabled => Some(*cfg),
            _ => None,
        }
    }

    pub fn try_play_sound(
        &mut self,
        stream: &StreamKind,
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
                            data::audio::SoundType::HardSell
                        } else {
                            data::audio::SoundType::HardBuy
                        }
                    } else if is_sell {
                        data::audio::SoundType::Sell
                    } else {
                        data::audio::SoundType::Buy
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
        let mut streams = FxHashMap::default();

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
