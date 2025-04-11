use crate::layout::pane::ok_or_default;
use exchange::SerTicker;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Source};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;

pub const BUY_SOUND: &str = "assets/sounds/hard-typewriter-click.wav";
pub const HARD_BUY_SOUND: &str = "assets/sounds/dry-pop-up.wav";

pub const SELL_SOUND: &str = "assets/sounds/hard-typewriter-hit.wav";
pub const HARD_SELL_SOUND: &str = "assets/sounds/fall-on-foam-splash.wav";

pub const DEFAULT_SOUNDS: &[&str] = &[BUY_SOUND, SELL_SOUND, HARD_BUY_SOUND, HARD_SELL_SOUND];

pub struct SoundCache {
    _stream: OutputStream,
    stream_handle: OutputStreamHandle,
    sample_buffers: HashMap<String, rodio::buffer::SamplesBuffer<i16>>,
    volume: Option<f32>,
}

impl SoundCache {
    pub fn new(volume: Option<f32>) -> Result<Self, String> {
        let (stream, stream_handle) = match OutputStream::try_default() {
            Ok(result) => result,
            Err(err) => return Err(format!("Failed to open audio output: {}", err)),
        };

        Ok(SoundCache {
            _stream: stream,
            stream_handle,
            sample_buffers: HashMap::new(),
            volume,
        })
    }

    pub fn with_default_sounds(volume: Option<f32>) -> Result<Self, String> {
        let mut cache = Self::new(volume)?;

        for path in DEFAULT_SOUNDS {
            if let Err(e) = cache.load_sound(path) {
                log::error!("Failed to load sound {}: {}", path, e);
            }
        }

        Ok(cache)
    }

    pub fn load_sound(&mut self, path: &str) -> Result<(), String> {
        if self.sample_buffers.contains_key(path) {
            return Ok(());
        }

        let file = match File::open(path) {
            Ok(file) => file,
            Err(err) => return Err(format!("Failed to open sound file '{}': {}", path, err)),
        };

        let mut buf_reader = BufReader::new(file);
        let mut buffer = vec![];
        if let Err(err) = std::io::Read::read_to_end(&mut buf_reader, &mut buffer) {
            return Err(format!("Failed to read sound file '{}': {}", path, err));
        }

        let cursor = std::io::Cursor::new(buffer);
        let decoder = match Decoder::new(cursor) {
            Ok(decoder) => decoder,
            Err(err) => return Err(format!("Failed to decode sound file '{}': {}", path, err)),
        };

        let sample_buffer = rodio::buffer::SamplesBuffer::new(
            decoder.channels(),
            decoder.sample_rate(),
            decoder.collect::<Vec<i16>>(),
        );

        self.sample_buffers.insert(path.to_string(), sample_buffer);

        Ok(())
    }

    pub fn play(&self, path: &str) -> Result<(), String> {
        let Some(volume) = self.volume else {
            return Ok(());
        };

        let buffer = self
            .sample_buffers
            .get(path)
            .ok_or(format!("Sound '{}' not loaded in cache", path))?;

        let sink = match rodio::Sink::try_new(&self.stream_handle) {
            Ok(sink) => sink,
            Err(err) => return Err(format!("Failed to create audio sink: {}", err)),
        };

        sink.set_volume(volume / 100.0);

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
    pub streams: HashMap<SerTicker, StreamCfg>,
    #[serde(deserialize_with = "ok_or_default")]
    pub volume: Option<f32>,
}
