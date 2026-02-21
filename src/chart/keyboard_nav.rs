//! Keyboard-driven chart panning.
//!
//! Computes a [`Message::Translated`] pan step from a keyboard event,
//! using the chart's current [`ViewState`] (translation, cell_width, scaling,
//! bounds).  Returns `None` for any key that is not a navigation key.
//!
//! ## Key bindings
//!
//! | Key            | Action                                         |
//! |----------------|------------------------------------------------|
//! | `←`            | Scroll left 10 bars (towards history)          |
//! | `→`            | Scroll right 10 bars (towards present)         |
//! | `Shift + ←`    | Scroll left 50 bars (fast)                     |
//! | `Shift + →`    | Scroll right 50 bars (fast)                    |
//! | `PageUp`       | Scroll left one full viewport width            |
//! | `PageDown`     | Scroll right one full viewport width           |
//! | `Home`         | Jump to latest bar (reset translation.x = 0)  |
//!
//! ## Sign convention
//!
//! `interval_to_x` returns a negative value for older bars, so the x-axis
//! grows *leftward* in chart coordinates.  Increasing `translation.x` shifts
//! content to the right, revealing older history — so `ArrowLeft` (backwards
//! in time) *increases* `translation.x`, mirroring a rightward mouse drag.

use super::{Message, ViewState};
use iced::{Vector, keyboard};

const BARS_SMALL: f32 = 10.0;
const BARS_LARGE: f32 = 50.0;

/// Compute a [`Message::Translated`] pan step from a keyboard event.
///
/// Returns `None` when `event` is not a navigation key, so callers can
/// fall through to other handlers.
pub fn handle(event: &keyboard::Event, state: &ViewState) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
        return None;
    };

    let shift = modifiers.shift();
    let bars = if shift { BARS_LARGE } else { BARS_SMALL };
    // Convert bar count → chart-coordinate step (screen pixels ÷ scaling)
    let step = bars * state.cell_width / state.scaling;

    let new_x = match key.as_ref() {
        keyboard::Key::Named(keyboard::key::Named::ArrowLeft) => state.translation.x + step,
        keyboard::Key::Named(keyboard::key::Named::ArrowRight) => state.translation.x - step,
        keyboard::Key::Named(keyboard::key::Named::PageUp) => {
            state.translation.x + state.bounds.width / state.scaling
        }
        keyboard::Key::Named(keyboard::key::Named::PageDown) => {
            state.translation.x - state.bounds.width / state.scaling
        }
        // Jump to the latest bar (reset pan)
        keyboard::Key::Named(keyboard::key::Named::Home) => 0.0,
        _ => return None,
    };

    Some(Message::Translated(Vector::new(new_x, state.translation.y)))
}
