pub mod axisx;
pub mod axisy;
pub mod overlay;

pub use super::Message;

#[derive(Debug, Clone, Copy)]
pub enum AxisZoomAnchor {
    /// Keep a specific world coordinate (along the active axis) fixed at a given screen position.
    World { world: f32, screen: f32 },
    /// Zoom anchored to the captured cursor position (along the active axis).
    Cursor { screen: f32 },
}

#[derive(Debug, Default, Clone, Copy)]
pub enum AxisInteraction {
    #[default]
    None,
    Panning {
        last_position: iced::Point,
        zoom_anchor: Option<AxisZoomAnchor>,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct AxisState {
    pub interaction: AxisInteraction,
    pub previous_click: Option<iced_core::mouse::Click>,
}

impl Default for AxisState {
    fn default() -> Self {
        Self {
            interaction: AxisInteraction::None,
            previous_click: None,
        }
    }
}

pub struct CanvasCaches {
    pub y_axis: iced::widget::canvas::Cache,
    pub x_axis: iced::widget::canvas::Cache,
    pub overlay: iced::widget::canvas::Cache,
    pub scale_labels: iced::widget::canvas::Cache,
}

impl CanvasCaches {
    pub fn new() -> Self {
        Self {
            y_axis: iced::widget::canvas::Cache::new(),
            x_axis: iced::widget::canvas::Cache::new(),
            overlay: iced::widget::canvas::Cache::new(),
            scale_labels: iced::widget::canvas::Cache::new(),
        }
    }

    pub fn clear_axes(&self) {
        self.y_axis.clear();
        self.x_axis.clear();
    }

    pub fn clear_overlays(&self) {
        self.overlay.clear();
        self.scale_labels.clear();
    }
}

fn nice_step_i64(rough: i64) -> i64 {
    // Choose from 1,2,5 * 10^k
    let rough = rough.max(1);
    let mut pow10 = 1i64;
    while pow10.saturating_mul(10) <= rough {
        pow10 *= 10;
    }
    let m = (rough + pow10 - 1) / pow10; // ceil
    let mult = if m <= 1 {
        1
    } else if m <= 2 {
        2
    } else if m <= 5 {
        5
    } else {
        10
    };
    mult * pow10
}
