pub mod axisx;
pub mod axisy;
pub mod overlay;

pub use super::Message;

const AXIS_FONT_SIZE: f32 = 12.0;

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

#[derive(Default)]
pub struct CanvasCaches {
    pub y_axis: iced::widget::canvas::Cache,
    pub x_axis: iced::widget::canvas::Cache,
    pub overlay: iced::widget::canvas::Cache,
    pub scale_labels: iced::widget::canvas::Cache,
}

#[derive(Debug, Clone, Copy)]
pub struct CanvasInvalidation {
    x_axis: bool,
    y_axis: bool,
    overlay_tooltip: bool,
    overlay_scale_labels: bool,
}

impl Default for CanvasInvalidation {
    fn default() -> Self {
        Self {
            x_axis: true,
            y_axis: true,
            overlay_tooltip: true,
            overlay_scale_labels: true,
        }
    }
}

impl CanvasInvalidation {
    pub fn mark_all(&mut self) {
        self.x_axis = true;
        self.y_axis = true;
        self.overlay_tooltip = true;
        self.overlay_scale_labels = true;
    }

    pub fn mark_axis_x(&mut self) {
        self.x_axis = true;
    }

    pub fn mark_axis_y(&mut self) {
        self.y_axis = true;
    }

    pub fn mark_axis_x_motion(&mut self) {
        self.mark_axis_x();
        self.mark_overlay_tooltip();
        self.mark_overlay_scale_labels();
    }

    pub fn mark_axis_y_motion(&mut self) {
        self.mark_axis_y();
        self.mark_overlay_tooltip();
        self.mark_overlay_scale_labels();
    }

    pub fn mark_axes_motion(&mut self) {
        self.mark_axis_x_motion();
        self.mark_axis_y_motion();
    }

    pub fn mark_cursor_moved(&mut self, x_axis_has_cursor_label: bool) {
        self.mark_axis_y();

        if x_axis_has_cursor_label {
            self.mark_axis_x();
        }

        self.mark_overlay_tooltip();
        self.mark_overlay_scale_labels();
    }

    pub fn mark_overlay_scale_labels(&mut self) {
        self.overlay_scale_labels = true;
    }

    pub fn mark_overlay_tooltip(&mut self) {
        self.overlay_tooltip = true;
    }

    pub fn apply(&mut self, caches: &CanvasCaches) {
        if self.x_axis {
            caches.x_axis.clear();
            self.x_axis = false;
        }

        if self.y_axis {
            caches.y_axis.clear();
            self.y_axis = false;
        }

        if self.overlay_tooltip {
            caches.overlay.clear();
            self.overlay_tooltip = false;
        }

        if self.overlay_scale_labels {
            caches.scale_labels.clear();
            self.overlay_scale_labels = false;
        }
    }
}

fn step_center_pos_from_world_y(world_y: f32, row_h: f32) -> f32 {
    (-(world_y) / row_h) - 0.5
}

fn world_y_for_step_center(step: i64, row_h: f32) -> f32 {
    -((step as f32 + 0.5) * row_h)
}

fn step_floor_from_world_y(world_y: f32, row_h: f32) -> i64 {
    ((-(world_y)) / row_h).floor() as i64
}

fn world_y_for_y_bin_center(y_bin: i64, steps_per_y_bin: i64, row_h: f32) -> f32 {
    -((y_bin as f32 + 0.5) * (steps_per_y_bin.max(1) as f32) * row_h)
}
