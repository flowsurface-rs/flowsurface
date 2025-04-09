use rodio::{Decoder, OutputStream, OutputStreamHandle};
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;

pub const BUY_SOUND: &str = "assets/sounds/hard-typewriter-click.wav";
pub const HARD_BUY_SOUND: &str = "assets/sounds/dry-pop-up.wav";

pub const SELL_SOUND: &str = "assets/sounds/hard-typewriter-hit.wav";
pub const HARD_SELL_SOUND: &str = "assets/sounds/fall-on-foam-splash.wav";

pub const TICK_SOUND: &str = "assets/sounds/typewriter-soft-click.wav";

pub const DEFAULT_SOUNDS: &[&str] = &[
    BUY_SOUND,
    SELL_SOUND,
    HARD_BUY_SOUND,
    HARD_SELL_SOUND,
    TICK_SOUND,
];

pub struct SoundCache {
    _stream: OutputStream,
    stream_handle: OutputStreamHandle,
    sound_data: HashMap<String, Vec<u8>>,
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
            sound_data: HashMap::new(),
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
        if self.sound_data.contains_key(path) {
            return Ok(());
        }

        let file = match File::open(path) {
            Ok(file) => file,
            Err(err) => return Err(format!("Failed to open sound file '{}': {}", path, err)),
        };

        let mut buf_reader = BufReader::new(file);
        let mut buffer = Vec::new();
        if let Err(err) = std::io::Read::read_to_end(&mut buf_reader, &mut buffer) {
            return Err(format!("Failed to read sound file '{}': {}", path, err));
        }

        self.sound_data.insert(path.to_string(), buffer);

        Ok(())
    }

    pub fn play(&self, path: &str) -> Result<(), String> {
        if self.volume.is_none() {
            return Ok(());
        }

        let sound_data = self
            .sound_data
            .get(path)
            .ok_or(format!("Sound '{}' not loaded in cache", path))?;

        let cursor = std::io::Cursor::new(sound_data.clone());

        let source = match Decoder::new(cursor) {
            Ok(source) => source,
            Err(err) => return Err(format!("Failed to decode sound data: {}", err)),
        };

        let sink = match rodio::Sink::try_new(&self.stream_handle) {
            Ok(sink) => sink,
            Err(err) => return Err(format!("Failed to create audio sink: {}", err)),
        };

        if let Some(level) = self.volume {
            let volume = level / 100.0;
            sink.set_volume(volume);
        }

        sink.append(source);
        sink.detach();

        Ok(())
    }

    pub fn set_sound_level(&mut self, level: f32) {
        if level == 0.0 {
            self.volume = None;
            return;
        };
        self.volume = Some(level.clamp(0.0, 100.0));
    }

    pub fn mute(&mut self) {
        self.volume = None;
    }

    pub fn is_muted(&self) -> bool {
        self.volume.is_none()
    }

    pub fn get_volume(&self) -> Option<f32> {
        self.volume
    }
}
