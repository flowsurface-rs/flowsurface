pub mod ladder;
pub mod timeandsales;

use iced::{
    Element, padding,
    widget::{canvas, center, container, text},
};
use std::time::Instant;

#[derive(Debug, Clone, Copy)]
pub enum Message {
    Scrolled(f32),
    ResetScroll,
    Invalidate(Option<Instant>),
}

pub enum Action {}

pub trait Panel: canvas::Program<Message> {
    fn scroll(&mut self, scroll: f32);

    fn reset_scroll(&mut self);

    fn invalidate(&mut self, now: Option<Instant>) -> Option<Action>;

    fn is_empty(&self) -> bool;
}

pub fn view<T: Panel>(panel: &'_ T, _timezone: data::UserTimezone) -> Element<'_, Message> {
    if panel.is_empty() {
        return center(text("Waiting for data...").size(16)).into();
    }

    container(
        canvas(panel)
            .height(iced::Length::Fill)
            .width(iced::Length::Fill),
    )
    .padding(padding::left(1).right(1).bottom(1))
    .into()
}

pub fn update<T: Panel>(panel: &mut T, message: Message) {
    match message {
        Message::Scrolled(delta) => {
            panel.scroll(delta);
        }
        Message::ResetScroll => {
            panel.reset_scroll();
        }
        Message::Invalidate(now) => {
            panel.invalidate(now);
        }
    }
}
