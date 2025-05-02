use iced::{
    Alignment, Element,
    widget::{Slider, button, column, container, horizontal_space, pick_list, row, text},
};

use crate::{
    style::{self, Icon, icon_text},
    widget::create_slider_row,
};

#[derive(Debug, Clone, PartialEq)]
pub enum ThemeField {
    Background,
    Text,
    Primary,
    Success,
    Danger,
    Warning,
}

impl std::fmt::Display for ThemeField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl ThemeField {
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
    HSVChanged((f32, f32, f32)),
    FocusedFieldChanged(ThemeField),
    CloseRequested,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    None,
    UpdateTheme(iced_core::Theme),
    Exit,
}

pub struct ThemeEditor {
    pub custom_theme: Option<iced_core::Theme>,
    pub hsv: Option<(f32, f32, f32)>,
    pub focused_field: ThemeField,
}

impl ThemeEditor {
    pub fn new(custom_theme: Option<data::Theme>) -> Self {
        if let Some(theme) = custom_theme {
            let bg_color = theme.0.palette().background;
            let initial_hsv = rgb_to_hsv(bg_color.r, bg_color.g, bg_color.b);

            Self {
                custom_theme: Some(theme.0),
                hsv: Some(initial_hsv),
                focused_field: ThemeField::Background,
            }
        } else {
            Self {
                custom_theme: None,
                hsv: None,
                focused_field: ThemeField::Background,
            }
        }
    }

    fn focused_color(&self, theme: &iced_core::Theme) -> iced_core::Color {
        let palette = theme.palette();
        match self.focused_field {
            ThemeField::Background => palette.background,
            ThemeField::Text => palette.text,
            ThemeField::Primary => palette.primary,
            ThemeField::Success => palette.success,
            ThemeField::Danger => palette.danger,
            ThemeField::Warning => palette.warning,
        }
    }

    pub fn update(&mut self, message: Message, theme: iced_core::Theme) -> Action {
        match message {
            Message::HSVChanged((h, s, v)) => {
                self.hsv = Some((h, s, v));
                let (r, g, b) = hsv_to_rgb(h, s, v);
                let color = iced_core::Color::from_rgb(r, g, b);

                let mut new_palette = theme.palette();

                match self.focused_field {
                    ThemeField::Background => new_palette.background = color,
                    ThemeField::Text => new_palette.text = color,
                    ThemeField::Primary => new_palette.primary = color,
                    ThemeField::Success => new_palette.success = color,
                    ThemeField::Danger => new_palette.danger = color,
                    ThemeField::Warning => new_palette.warning = color,
                }

                let new_theme = iced_core::Theme::custom("Custom".to_string(), new_palette);
                self.custom_theme = Some(new_theme.clone());

                Action::UpdateTheme(new_theme)
            }
            Message::FocusedFieldChanged(focused_field) => {
                self.focused_field = focused_field;
                let color = self.focused_color(&theme);
                self.hsv = Some(rgb_to_hsv(color.r, color.g, color.b));

                Action::None
            }
            Message::CloseRequested => Action::Exit,
        }
    }

    pub fn view(&self, theme: &iced_core::Theme) -> Element<'_, Message> {
        let close_editor = button(icon_text(Icon::Return, 11))
            .on_press(Message::CloseRequested)
            .style(move |theme, status| style::button::transparent(theme, status, false));

        let color_editor = {
            let (h, s, v) = {
                let rgb = self.focused_color(theme);
                rgb_to_hsv(rgb.r, rgb.g, rgb.b)
            };

            let hue_slider = create_slider_row(
                text("H"),
                Slider::new(0.0..=1.0, h, move |value| {
                    Message::HSVChanged((value, s, v))
                })
                .step(0.01)
                .into(),
                None,
            );

            let saturation_slider = create_slider_row(
                text("S"),
                Slider::new(0.0..=1.0, s, move |value| {
                    Message::HSVChanged((h, value, v))
                })
                .step(0.01)
                .into(),
                None,
            );

            let value_slider = create_slider_row(
                text("V"),
                Slider::new(0.0..=1.0, v, move |value| {
                    Message::HSVChanged((h, s, value))
                })
                .step(0.01)
                .into(),
                None,
            );

            column![hue_slider, saturation_slider, value_slider,].spacing(4)
        };

        let focused_field = pick_list(
            ThemeField::ALL.to_vec(),
            Some(&self.focused_field),
            Message::FocusedFieldChanged,
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
            color_editor,
        ]
        .spacing(10);

        container(content)
            .max_width(320)
            .padding(24)
            .style(style::dashboard_modal)
            .into()
    }
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    if s <= 0.0 {
        return (v, v, v);
    }

    let hh = (h % 1.0) * 6.0;
    let i = hh.floor();
    let ff = hh - i;

    let p = v * (1.0 - s);
    let q = v * (1.0 - (s * ff));
    let t = v * (1.0 - (s * (1.0 - ff)));

    match i as u8 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}

pub fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);

    let v = max;

    if max == 0.0 {
        return (0.0, 0.0, 0.0);
    }

    let s = (max - min) / max;

    if max == min {
        return (0.0, s, v);
    }

    let h = if max == r {
        (g - b) / (max - min) + (if g < b { 6.0 } else { 0.0 })
    } else if max == g {
        (b - r) / (max - min) + 2.0
    } else {
        (r - g) / (max - min) + 4.0
    };

    (h / 6.0, s, v)
}
