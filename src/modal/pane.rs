use iced::{
    Alignment, Element, Length, padding,
    widget::{container, mouse_area, opaque},
};

pub mod indicators;
pub mod mini_tickers_list;
pub mod settings;
pub mod stream;

#[derive(Debug, Clone, PartialEq)]
pub enum Modal {
    StreamModifier(super::stream::Modifier),
    MiniTickersList(mini_tickers_list::MiniPanel),
    Settings,
    Indicators,
    LinkGroup,
    Controls,
}

pub fn stack_modal<'a, Message>(
    base: impl Into<Element<'a, Message>>,
    content: impl Into<Element<'a, Message>>,
    on_blur: Message,
    padding: padding::Padding,
    alignment: Alignment,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    iced::widget::stack![
        base.into(),
        mouse_area(
            container(opaque(content))
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(padding)
                .align_x(alignment)
        )
        .on_press(on_blur)
    ]
    .into()
}
