use iced::{
    Alignment, Element,
    widget::{button, column, container, pick_list, row, space, text_input::default},
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
        write!(f, "{self:?}")
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
    HexInput(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    UpdateTheme(iced_core::Theme),
    Exit,
}

pub struct ThemeEditor {
    pub custom_theme: Option<iced_core::Theme>,
    component: Component,
    hex_input: Option<String>,
}

impl ThemeEditor {
    pub fn new(custom_theme: Option<data::Theme>) -> Self {
        Self {
            custom_theme: custom_theme.map(|theme| theme.0),
            component: Component::Background,
            hex_input: None,
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

    pub fn update(&mut self, message: Message, theme: &iced_core::Theme) -> Option<Action> {
        match message {
            Message::Color(color) => {
                self.hex_input = None;

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

                Some(Action::UpdateTheme(new_theme))
            }
            Message::ComponentChanged(component) => {
                self.component = component;
                None
            }
            Message::HexInput(input) => {
                let mut action = None;

                if let Some(color) = data::config::theme::hex_to_color(&input) {
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

                    action = Some(Action::UpdateTheme(new_theme));
                }

                self.hex_input = Some(input);
                action
            }
            Message::CloseRequested => Some(Action::Exit),
        }
    }

    pub fn view(&self, theme: &iced_core::Theme) -> Element<'_, Message> {
        let color = self.focused_color(theme);

        let close_editor = button(icon_text(Icon::Return, 11)).on_press(Message::CloseRequested);

        let is_input_valid = self.hex_input.is_none()
            || self
                .hex_input
                .as_deref()
                .and_then(data::config::theme::hex_to_color)
                .is_some();

        let hex_input = iced::widget::text_input(
            "",
            self.hex_input
                .as_deref()
                .unwrap_or(data::config::theme::color_to_hex(color).as_str()),
        )
        .on_input(Message::HexInput)
        .width(80)
        .style(move |theme: &iced::Theme, status| {
            let palette = theme.extended_palette();

            iced::widget::text_input::Style {
                border: iced::Border {
                    color: if is_input_valid {
                        palette.background.strong.color
                    } else {
                        palette.danger.base.color
                    },
                    width: 1.0,
                    radius: 2.0.into(),
                },
                ..default(theme, status)
            }
        });

        let focused_field = pick_list(
            Component::ALL.to_vec(),
            Some(&self.component),
            Message::ComponentChanged,
        );

        let content = column![
            row![
                close_editor,
                space::horizontal(),
                row![hex_input, focused_field,].spacing(4),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            color_picker(color, Message::Color),
        ]
        .spacing(10);

        container(content)
            .max_width(380)
            .padding(24)
            .style(style::dashboard_modal)
            .into()
    }
}
