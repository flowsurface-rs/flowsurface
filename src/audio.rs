use rodio::{Decoder, OutputStream, OutputStreamHandle, Source};
use std::time::{Duration, Instant};

pub const BUY_SOUND_DATA: &[u8] = include_bytes!("../assets/sounds/hard-typewriter-click.wav");
pub const HARD_BUY_SOUND_DATA: &[u8] = include_bytes!("../assets/sounds/dry-pop-up.wav");
pub const SELL_SOUND_DATA: &[u8] = include_bytes!("../assets/sounds/hard-typewriter-hit.wav");
pub const HARD_SELL_SOUND_DATA: &[u8] = include_bytes!("../assets/sounds/fall-on-foam-splash.wav");

pub const BUY_SOUND: &str = "hard-typewriter-click.wav";
pub const HARD_BUY_SOUND: &str = "dry-pop-up.wav";
pub const SELL_SOUND: &str = "hard-typewriter-hit.wav";
pub const HARD_SELL_SOUND: &str = "fall-on-foam-splash.wav";

const OVERLAP_THRESHOLD: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, Copy)]
pub enum SoundType {
    Buy = 0,
    HardBuy = 1,
    Sell = 2,
    HardSell = 3,
}

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("Failed to open audio output: {0}")]
    OpenOutput(#[from] rodio::StreamError),
    #[error("Failed to decode sound data: {0}")]
    Decode(#[from] rodio::decoder::DecoderError),
    #[error("Failed to create audio sink: {0}")]
    CreateSink(#[from] rodio::PlayError),
    #[error("Sound '{0}' not loaded")]
    NotLoaded(SoundType),
    #[error("Failed to load default sound '{path}': {source}")]
    LoadDefaultSound {
        path: &'static str,
        #[source]
        source: Box<AudioError>,
    },
}

impl AudioError {
    /// True when the audio output device is missing/unavailable/lost
    pub fn is_no_device(&self) -> bool {
        match self {
            AudioError::OpenOutput(rodio::StreamError::NoDevice) => true,
            AudioError::CreateSink(rodio::PlayError::NoDevice) => true,
            AudioError::LoadDefaultSound { source, .. } => source.is_no_device(),
            _ => false,
        }
    }
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
    pub fn new(volume: Option<f32>) -> Result<Self, AudioError> {
        let (stream, stream_handle) = OutputStream::try_default()?;

        Ok(SoundCache {
            _stream: stream,
            stream_handle,
            volume,
            sample_buffers: [None, None, None, None],
            last_played: [(None, 0), (None, 0), (None, 0), (None, 0)],
        })
    }

    pub fn with_default_sounds(volume: Option<f32>) -> Result<Self, AudioError> {
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

            cache
                .load_sound_from_memory(*sound_type, data)
                .map_err(|e| AudioError::LoadDefaultSound {
                    path,
                    source: Box::new(e),
                })?;
        }

        Ok(cache)
    }

    pub fn load_sound_from_memory(
        &mut self,
        sound_type: SoundType,
        data: &[u8],
    ) -> Result<(), AudioError> {
        let index = sound_type as usize;

        if self.sample_buffers[index].is_some() {
            return Ok(());
        }

        let cursor = std::io::Cursor::new(data.to_vec());
        let decoder = Decoder::new(cursor)?;

        let sample_buffer = rodio::buffer::SamplesBuffer::new(
            decoder.channels(),
            decoder.sample_rate(),
            decoder.collect::<Vec<i16>>(),
        );

        self.sample_buffers[index] = Some(sample_buffer);
        Ok(())
    }

    pub fn play(&mut self, sound_type: SoundType) -> Result<(), AudioError> {
        let Some(base_volume) = self.volume else {
            return Ok(());
        };

        let index = usize::from(sound_type);

        let Some(buffer) = self.sample_buffers[index].as_ref() else {
            return Err(AudioError::NotLoaded(sound_type));
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

        let sink = rodio::Sink::try_new(&self.stream_handle)?;
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
}
