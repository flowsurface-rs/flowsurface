use serde::{Deserialize, Serialize};

use super::pane::Pane;

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct Window<T> {
    pub width: T,
    pub height: T,
    pub pos_x: T,
    pub pos_y: T,
}

impl Window<f32> {
    pub fn get_size(&self) -> iced_core::Size {
        iced_core::Size {
            width: self.width,
            height: self.height,
        }
    }

    pub fn get_position(&self) -> iced_core::Point {
        iced_core::Point {
            x: self.pos_x,
            y: self.pos_y,
        }
    }
}

impl Default for Window<f32> {
    fn default() -> Self {
        Self {
            width: 1024.0,
            height: 768.0,
            pos_x: 0.0,
            pos_y: 0.0,
        }
    }
}

pub type WindowSpec = Window<f32>;

impl From<(&iced_core::Point, &iced_core::Size)> for WindowSpec {
    fn from((point, size): (&iced_core::Point, &iced_core::Size)) -> Self {
        Self {
            width: size.width,
            height: size.height,
            pos_x: point.x,
            pos_y: point.y,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Dashboard {
    pub pane: Pane,
    pub popout: Vec<(Pane, WindowSpec)>,
    pub trade_fetch_enabled: bool,
}

impl Default for Dashboard {
    fn default() -> Self {
        Self {
            pane: Pane::Starter,
            popout: vec![],
            trade_fetch_enabled: false,
        }
    }
}
