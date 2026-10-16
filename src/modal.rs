pub mod audio;
pub mod layout_manager;
pub mod network_editor;
pub mod pane;
pub mod theme_editor;

use iced::widget::{container, mouse_area, opaque, stack};
use iced::{Alignment, Element, Length, padding};
pub use layout_manager::LayoutManager;
pub use pane::stream::{self, ModifierKind};
pub use theme_editor::ThemeEditor;

pub fn dashboard_modal<'a, Message>(
    base: impl Into<Element<'a, Message>>,
    content: impl Into<Element<'a, Message>>,
    on_blur: Message,
    padding: padding::Padding,
    align_y: Alignment,
    align_x: Alignment,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    stack![
        base.into(),
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
    .into()
}
