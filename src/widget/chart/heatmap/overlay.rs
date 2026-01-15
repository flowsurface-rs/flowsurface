use iced::{Rectangle, Renderer, Theme, mouse, widget::canvas};

use crate::widget::chart::heatmap::Message;

#[derive(Clone, Copy, Default)]
pub struct OverlayCanvas;

impl canvas::Program<Message> for OverlayCanvas {
    type State = ();

    fn update(
        &self,
        _state: &mut Self::State,
        _event: &iced::Event,
        _bounds: iced::Rectangle,
        _cursor: iced_core::mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        None
    }

    fn draw(
        &self,
        _state: &Self::State,
        _renderer: &Renderer,
        _theme: &Theme,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        // TODO: draw overlay (crosshair, selections, etc) in plot bounds
        vec![]
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        _bounds: iced::Rectangle,
        _cursor: iced_core::mouse::Cursor,
    ) -> iced_core::mouse::Interaction {
        iced_core::mouse::Interaction::default()
    }
}
