pub mod timeandsales;

use iced::{Element, widget::canvas};

#[derive(Debug, Clone, Copy)]
pub enum Message {
    Scrolled(f32),
    ResetScroll,
}

pub trait Panel: canvas::Program<Message> {
    fn scroll(&mut self, scroll: f32);

    fn reset_scroll_position(&mut self);
}

pub fn view<T: Panel>(panel: &T, _timezone: data::UserTimezone) -> Element<Message> {
    canvas(panel)
        .height(iced::Length::Fill)
        .width(iced::Length::Fill)
        .into()
}

pub fn update<T: Panel>(panel: &mut T, message: Message) {
    match message {
        Message::Scrolled(delta) => {
            panel.scroll(delta);
        }
        Message::ResetScroll => {
            panel.reset_scroll_position();
        }
    }
}
