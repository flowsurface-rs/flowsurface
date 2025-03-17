use super::Element;
use crate::style;
use iced::{
    Alignment,
    widget::{button, column, container, row, scrollable, text, tooltip::Position},
};

pub mod column_drag;
pub mod hsplit;
pub mod notification;

pub fn tooltip<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    tooltip: Option<&'a str>,
    position: Position,
) -> Element<'a, Message> {
    match tooltip {
        Some(tooltip) => iced::widget::tooltip(
            content,
            container(text(tooltip)).style(style::tooltip).padding(8),
            position,
        )
        .into(),
        None => content.into(),
    }
}

pub fn scrollable_content<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    scrollable::Scrollable::with_direction(
        content,
        scrollable::Direction::Vertical(scrollable::Scrollbar::new().width(4).scroller_width(4)),
    )
    .into()
}

pub fn confirm_dialog<'a, Message: 'a + Clone>(
    dialog: &'a str,
    on_confirm: Message,
    on_cancel: Message,
) -> Element<'a, Message> {
    let dialog_content = container(
        column![
            text(dialog).size(14),
            row![
                button(text("Cancel"))
                    .style(|theme, status| style::button_transparent(theme, status, false))
                    .on_press(on_cancel),
                button(text("Confirm")).on_press(on_confirm),
            ]
            .spacing(8),
        ]
        .align_x(Alignment::Center)
        .spacing(16),
    )
    .padding(24)
    .style(style::dashboard_modal);

    dialog_content.into()
}
