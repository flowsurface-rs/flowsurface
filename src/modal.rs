pub mod audio;
pub mod layout_manager;
pub mod network_editor;
pub mod pane;
pub mod theme_editor;

use iced::widget::{center, container, mouse_area, opaque, stack};
use iced::{Alignment, Color, Length, padding};
pub use layout_manager::LayoutManager;
pub use pane::indicators;
pub use pane::stream::{self, ModifierKind};
pub use theme_editor::ThemeEditor;

pub fn main_dialog_modal<'a, Message>(
    base: impl iced::widget::Widget<Message> + 'a,
    content: impl iced::widget::Widget<Message> + 'a,
    on_blur: Message,
) -> impl iced::widget::Widget<Message> + 'a
where
    Message: Clone + 'a,
{
    stack![
        base,
        opaque(
            mouse_area(center(opaque(content)).style(|_theme| {
                container::Style {
                    background: Some(
                        Color {
                            a: 0.8,
                            ..Color::BLACK
                        }
                        .into(),
                    ),
                    ..container::Style::default()
                }
            }))
            .on_press(on_blur)
        )
    ]
}

pub fn dashboard_modal<'a, Message>(
    base: impl iced::widget::Widget<Message> + 'a,
    content: impl iced::widget::Widget<Message> + 'a,
    on_blur: Message,
    padding: padding::Padding,
    align_y: Alignment,
    align_x: Alignment,
) -> impl iced::widget::Widget<Message> + 'a
where
    Message: Clone + 'a,
{
    stack![
        base,
        mouse_area(
            container(opaque(content))
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(padding)
                .align_y(align_y)
                .align_x(align_x)
        )
        .on_press(on_blur)
    ]
}
