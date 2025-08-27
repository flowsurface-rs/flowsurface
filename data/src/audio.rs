use exchange::SerTicker;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Source};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

use crate::util::ok_or_default;

pub const BUY_SOUND_DATA: &[u8] = include_bytes!("../../assets/sounds/hard-typewriter-click.wav");
pub const HARD_BUY_SOUND_DATA: &[u8] = include_bytes!("../../assets/sounds/dry-pop-up.wav");
pub const SELL_SOUND_DATA: &[u8] = include_bytes!("../../assets/sounds/hard-typewriter-hit.wav");
pub const HARD_SELL_SOUND_DATA: &[u8] =
    include_bytes!("../../assets/sounds/fall-on-foam-splash.wav");

pub const BUY_SOUND: &str = "hard-typewriter-click.wav";
pub const HARD_BUY_SOUND: &str = "dry-pop-up.wav";
pub const SELL_SOUND: &str = "hard-typewriter-hit.wav";
pub const HARD_SELL_SOUND: &str = "fall-on-foam-splash.wav";

pub const DEFAULT_SOUNDS: &[&str] = &[BUY_SOUND, SELL_SOUND, HARD_BUY_SOUND, HARD_SELL_SOUND];

const OVERLAP_THRESHOLD: Duration = Duration::from_millis(10);

#[derive(Clone, Copy)]
pub enum SoundType {
    Buy = 0,
    HardBuy = 1,
    Sell = 2,
    HardSell = 3,
}

impl std::fmt::Display for SoundType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Buy => BUY_SOUND,
                Self::HardBuy => HARD_BUY_SOUND,
                Self::Sell => SELL_SOUND,
                Self::HardSell => HARD_SELL_SOUND,
            }
        )
    }
}

impl From<SoundType> for usize {
    fn from(sound_type: SoundType) -> Self {
        sound_type as usize
    }
}

pub struct SoundCache {
    _stream: OutputStream,
    stream_handle: OutputStreamHandle,
    volume: Option<f32>,
    sample_buffers: [Option<rodio::buffer::SamplesBuffer<i16>>; 4],
    last_played: [(Option<Instant>, usize); 4],
}

impl SoundCache {
    pub fn new(volume: Option<f32>) -> Result<Self, String> {
        let (stream, stream_handle) = match OutputStream::try_default() {
            Ok(result) => result,
            Err(err) => {
                return Err(format!("Failed to open audio output: {}", err));
            }
        };

        Ok(SoundCache {
            _stream: stream,
            stream_handle,
            volume,
            sample_buffers: [None, None, None, None],
            last_played: [(None, 0), (None, 0), (None, 0), (None, 0)],
        })
    }

    pub fn with_default_sounds(volume: Option<f32>) -> Result<Self, String> {
        let mut cache = Self::new(volume)?;

        let sound_types = [
            SoundType::Buy,
            SoundType::HardBuy,
            SoundType::Sell,
            SoundType::HardSell,
        ];

        for sound_type in &sound_types {
            let (path, data) = match sound_type {
                SoundType::Buy => (BUY_SOUND, BUY_SOUND_DATA),
                SoundType::HardBuy => (HARD_BUY_SOUND, HARD_BUY_SOUND_DATA),
                SoundType::Sell => (SELL_SOUND, SELL_SOUND_DATA),
                SoundType::HardSell => (HARD_SELL_SOUND, HARD_SELL_SOUND_DATA),
            };

            if let Err(e) = cache.load_sound_from_memory(*sound_type, data) {
                return Err(format!("Failed to load default sound '{}': {}", path, e));
            }
        }

        Ok(cache)
    }

    pub fn load_sound_from_memory(
        &mut self,
        sound_type: SoundType,
        data: &[u8],
    ) -> Result<(), String> {
        let index = sound_type as usize;

        if self.sample_buffers[index].is_some() {
            return Ok(());
        }

        let cursor = std::io::Cursor::new(data.to_vec());
        let decoder = match Decoder::new(cursor) {
            Ok(decoder) => decoder,
            Err(err) => {
                return Err(format!("Failed to decode sound data: {}", err));
            }
        };

        let sample_buffer = rodio::buffer::SamplesBuffer::new(
            decoder.channels(),
            decoder.sample_rate(),
            decoder.collect::<Vec<i16>>(),
        );

        self.sample_buffers[index] = Some(sample_buffer);
        Ok(())
    }

    pub fn play(&mut self, sound_type: SoundType) -> Result<(), String> {
        let Some(base_volume) = self.volume else {
            return Ok(());
        };

        let index = usize::from(sound_type);

        let Some(buffer) = self.sample_buffers[index].as_ref() else {
            return Err(format!("Sound '{sound_type}' not loaded",));
        };

        let now = Instant::now();
        let (last_time, count) = &mut self.last_played[index];

        let overlap_count = if let Some(last) = last_time {
            if now.duration_since(*last) < OVERLAP_THRESHOLD {
                *count += 1;
                *last = now;
                *count
            } else {
                *last = now;
                *count = 1;
                1
            }
        } else {
            *last_time = Some(now);
            *count = 1;
            1
        };

        let adjusted_volume = base_volume / (overlap_count as f32);

        let sink = match rodio::Sink::try_new(&self.stream_handle) {
            Ok(sink) => sink,
            Err(err) => {
                return Err(format!("Failed to create audio sink: {}", err));
            }
        };

        sink.set_volume(adjusted_volume / 100.0);
        sink.append(buffer.clone());
        sink.detach();

        Ok(())
    }

    pub fn set_volume(&mut self, level: f32) {
        if level == 0.0 {
            self.volume = None;
            return;
        };
        self.volume = Some(level.clamp(0.0, 100.0));
    }

    pub fn get_volume(&self) -> Option<f32> {
        self.volume
    }

    pub fn mute(&mut self) {
        self.volume = None;
    }

    pub fn is_muted(&self) -> bool {
        self.volume.is_none()
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub enum Threshold {
    Count(usize),
    Qty(f32),
}

impl std::fmt::Display for Threshold {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Threshold::Count(count) => write!(f, "Count based: {}", count),
            Threshold::Qty(qty) => write!(f, "Qty based: {:.2}", qty),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct StreamCfg {
    pub enabled: bool,
    pub threshold: Threshold,
}

impl Default for StreamCfg {
    fn default() -> Self {
        StreamCfg {
            enabled: true,
            threshold: Threshold::Count(10),
        }
    }
}

#[derive(Default, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct AudioStream {
    #[serde(deserialize_with = "ok_or_default")]
    pub streams: FxHashMap<SerTicker, StreamCfg>,
    #[serde(deserialize_with = "ok_or_default")]
    pub volume: Option<f32>,
}
