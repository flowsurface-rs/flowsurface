use iced::{
    Element, Length,
    widget::{column, container, row, text},
};

use crate::{style, widget::color_picker::color_picker};

pub fn color_picker_section<'a, Message: Clone + 'a>(
    label: &'static str,
    color: iced::Color,
    on_change: impl Fn(iced::Color) -> Message + Copy + 'a,
) -> Element<'a, Message> {
    let hsva = data::config::theme::to_hsva(color);

    column![
        row![
            container("")
                .width(14)
                .height(14)
                .style(move |theme| { style::colored_circle_container(theme, color) }),
            text(label),
        ]
        .width(Length::Fill)
        .spacing(8)
        .align_y(iced::Alignment::Center),
        color_picker(hsva, move |hsva| on_change(data::config::theme::from_hsva(
            hsva
        ))),
    ]
    .spacing(4)
    .into()
}
