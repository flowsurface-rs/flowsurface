use data::chart::kline as persisted;

use super::{FLOATING_TOOLBAR_HEIGHT, FLOATING_TOOLBAR_MARGIN};

#[derive(Debug, Clone, Copy)]
pub enum FloatingToolbarEvent {
    HandleMoved(iced::Point),
    DragStarted,
    DragMoved {
        cursor: iced::Point,
        overlay_size: iced::Size,
    },
    DragFinished,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct FloatingToolbarState {
    pub(super) position: persisted::FloatingToolbarPosition,
    handle_cursor: iced::Point,
    pub(super) is_dragging: bool,
}

impl Default for FloatingToolbarState {
    fn default() -> Self {
        Self {
            position: persisted::FloatingToolbarPosition::default(),
            handle_cursor: iced::Point::ORIGIN,
            is_dragging: false,
        }
    }
}

impl FloatingToolbarState {
    pub(super) fn from_position(position: persisted::FloatingToolbarPosition) -> Self {
        Self {
            position: position.clamped(),
            ..Self::default()
        }
    }

    pub(super) fn update(&mut self, event: FloatingToolbarEvent) {
        match event {
            FloatingToolbarEvent::HandleMoved(cursor) => {
                self.handle_cursor = cursor;
            }
            FloatingToolbarEvent::DragStarted => {
                self.is_dragging = true;
            }
            FloatingToolbarEvent::DragMoved {
                cursor,
                overlay_size,
            } => {
                if !self.is_dragging {
                    return;
                }

                let position = iced::Point::new(
                    (cursor.x - self.handle_cursor.x)
                        .clamp(FLOATING_TOOLBAR_MARGIN, Self::max_x(overlay_size)),
                    (cursor.y - self.handle_cursor.y)
                        .clamp(FLOATING_TOOLBAR_MARGIN, Self::max_y(overlay_size)),
                );
                self.position = persisted::FloatingToolbarPosition {
                    x_ratio: Self::ratio(position.x, overlay_size.width),
                    y_ratio: Self::ratio(position.y, overlay_size.height),
                };
            }
            FloatingToolbarEvent::DragFinished => {
                self.is_dragging = false;
            }
        }
    }

    pub(super) fn position_within(&self, overlay_size: iced::Size) -> iced::Point {
        let position = iced::Point::new(
            self.position.x_ratio * overlay_size.width,
            self.position.y_ratio * overlay_size.height,
        );

        iced::Point::new(
            position
                .x
                .clamp(FLOATING_TOOLBAR_MARGIN, Self::max_x(overlay_size)),
            position
                .y
                .clamp(FLOATING_TOOLBAR_MARGIN, Self::max_y(overlay_size)),
        )
    }

    fn max_x(overlay_size: iced::Size) -> f32 {
        (overlay_size.width - FLOATING_TOOLBAR_MARGIN).max(FLOATING_TOOLBAR_MARGIN)
    }

    fn max_y(overlay_size: iced::Size) -> f32 {
        (overlay_size.height - FLOATING_TOOLBAR_HEIGHT - FLOATING_TOOLBAR_MARGIN)
            .max(FLOATING_TOOLBAR_MARGIN)
    }

    fn ratio(position: f32, extent: f32) -> f32 {
        if !extent.is_finite() || extent <= 0.0 {
            return 0.0;
        }

        (position / extent).clamp(0.0, 1.0)
    }
}
