use serde::{Deserialize, Serialize};

pub mod state;
pub mod theme;
pub mod timezone;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
pub struct ScaleFactor(f64);

impl Default for ScaleFactor {
    fn default() -> Self {
        Self(1.0)
    }
}

impl From<f64> for ScaleFactor {
    fn from(value: f64) -> Self {
        ScaleFactor(value.clamp(0.8, 1.8))
    }
}

impl From<ScaleFactor> for f64 {
    fn from(value: ScaleFactor) -> Self {
        value.0
    }
}

#[derive(Default, Debug, Clone, PartialEq, Copy, Deserialize, Serialize)]
pub enum Sidebar {
    #[default]
    Left,
    Right,
}

impl std::fmt::Display for Sidebar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Sidebar::Left => write!(f, "Left"),
            Sidebar::Right => write!(f, "Right"),
        }
    }
}
