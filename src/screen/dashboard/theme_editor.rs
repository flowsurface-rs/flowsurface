use iced::{
    Alignment, Element,
    widget::{button, column, container, horizontal_space, pick_list, row, text},
};

use crate::{
    style::{self, Icon, icon_text},
    widget::color_picker::color_picker,
};

#[derive(Debug, Clone, PartialEq)]
pub enum Component {
    Background,
    Text,
    Primary,
    Success,
    Danger,
    Warning,
}

impl std::fmt::Display for Component {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl Component {
    const ALL: [Self; 6] = [
        Self::Background,
        Self::Text,
        Self::Primary,
        Self::Success,
        Self::Danger,
        Self::Warning,
    ];
}

#[derive(Debug, Clone)]
pub enum Message {
    ComponentChanged(Component),
    CloseRequested,
    Color(iced::Color),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    None,
    UpdateTheme(iced_core::Theme),
    Exit,
}

pub struct ThemeEditor {
    pub custom_theme: Option<iced_core::Theme>,
    pub component: Component,
}

impl ThemeEditor {
    pub fn new(custom_theme: Option<data::Theme>) -> Self {
        Self {
            custom_theme: custom_theme.map(|theme| theme.0),
            component: Component::Background,
        }
    }

    fn focused_color(&self, theme: &iced_core::Theme) -> iced_core::Color {
        let palette = theme.palette();
        match self.component {
            Component::Background => palette.background,
            Component::Text => palette.text,
            Component::Primary => palette.primary,
            Component::Success => palette.success,
            Component::Danger => palette.danger,
            Component::Warning => palette.warning,
        }
    }

    pub fn update(&mut self, message: Message, theme: iced_core::Theme) -> Action {
        match message {
            Message::Color(color) => {
                let mut new_palette = theme.palette();

                match self.component {
                    Component::Background => new_palette.background = color,
                    Component::Text => new_palette.text = color,
                    Component::Primary => new_palette.primary = color,
                    Component::Success => new_palette.success = color,
                    Component::Danger => new_palette.danger = color,
                    Component::Warning => new_palette.warning = color,
                }

                let new_theme = iced_core::Theme::custom("Custom".to_string(), new_palette);
                self.custom_theme = Some(new_theme.clone());

                Action::UpdateTheme(new_theme)
            }
            Message::ComponentChanged(component) => {
                self.component = component;
                Action::None
            }
            Message::CloseRequested => Action::Exit,
        }
    }

    pub fn view(&self, theme: &iced_core::Theme) -> Element<'_, Message> {
        let close_editor = button(icon_text(Icon::Return, 11))
            .on_press(Message::CloseRequested)
            .style(move |theme, status| style::button::transparent(theme, status, false));

        let focused_field = pick_list(
            Component::ALL.to_vec(),
            Some(&self.component),
            Message::ComponentChanged,
        );

        let content = column![
            row![
                close_editor,
                text("Theme Editor"),
                horizontal_space(),
                focused_field,
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            color_picker(self.focused_color(theme), Message::Color),
        ]
        .spacing(10);

        container(content)
            .max_width(380)
            .padding(24)
            .style(style::dashboard_modal)
            .into()
    }
}
