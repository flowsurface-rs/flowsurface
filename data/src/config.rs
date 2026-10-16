use serde::{Deserialize, Deserializer, Serialize};

pub mod auth;
pub mod network;
pub mod sidebar;
pub mod state;
pub mod theme;
pub mod timezone;

pub const MIN_SCALE: f32 = 0.8;
pub const MAX_SCALE: f32 = 1.5;

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct ScaleFactor(f32);

impl Default for ScaleFactor {
    fn default() -> Self {
        Self(1.0)
    }
}

impl From<f32> for ScaleFactor {
    fn from(value: f32) -> Self {
        let value = if value.is_finite() {
            value.clamp(MIN_SCALE, MAX_SCALE)
        } else {
            Self::default().0
        };

        Self(value)
    }
}

impl<'de> Deserialize<'de> for ScaleFactor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = f32::deserialize(deserializer)?;
        Ok(value.into())
    }
}

impl From<ScaleFactor> for f32 {
    fn from(value: ScaleFactor) -> Self {
        value.0
    }
}
