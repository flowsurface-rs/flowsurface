use iced::{
    Alignment, Element,
    theme::Palette,
    widget::{Slider, button, column, container, horizontal_space, pick_list, row, text},
};

use crate::{
    style::{self, Icon, icon_text},
    widget::create_slider_row,
};

#[derive(Debug, Clone)]
pub enum Message {
    HSVChanged((f32, f32, f32)),
    FocusedFieldChanged(FocusedField),
    CloseRequested,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    None,
    UpdateTheme(iced_core::Theme),
    Exit,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FocusedField {
    Background,
    Text,
    Primary,
    Success,
    Danger,
    Warning,
}

impl std::fmt::Display for FocusedField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl FocusedField {
    const ALL: [Self; 6] = [
        Self::Background,
        Self::Text,
        Self::Primary,
        Self::Success,
        Self::Danger,
        Self::Warning,
    ];
}

pub struct ThemeEditor {
    pub theme: iced_core::Theme,
    pub palette: Palette,
    pub hsv: Option<(f32, f32, f32)>,
    pub focused_field: Option<FocusedField>,
}

impl ThemeEditor {
    pub fn new(theme: data::Theme) -> Self {
        let palette: Palette = theme.0.palette();

        let bg_color = palette.background;
        let initial_hsv = rgb_to_hsv(bg_color.r, bg_color.g, bg_color.b);

        Self {
            theme: theme.0,
            palette,
            hsv: Some(initial_hsv),
            focused_field: Some(FocusedField::Background),
        }
    }

    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::HSVChanged(hsv) => {
                self.hsv = Some(hsv);
                let (h, s, v) = hsv;

                let (r, g, b) = hsv_to_rgb(h, s, v);
                let new_color = iced_core::Color::from_rgb(r, g, b);

                let mut new_palette = self.palette;

                if let Some(focused_field) = &mut self.focused_field {
                    match focused_field {
                        FocusedField::Background => {
                            new_palette.background = new_color;
                        }
                        FocusedField::Text => {
                            new_palette.text = new_color;
                        }
                        FocusedField::Primary => {
                            new_palette.primary = new_color;
                        }
                        FocusedField::Success => {
                            new_palette.success = new_color;
                        }
                        FocusedField::Danger => {
                            new_palette.danger = new_color;
                        }
                        FocusedField::Warning => {
                            new_palette.warning = new_color;
                        }
                    }
                }

                self.palette = new_palette;
                self.theme = iced_core::Theme::custom("Custom".to_string(), new_palette);

                Action::UpdateTheme(self.theme.clone())
            }
            Message::FocusedFieldChanged(focused_field) => {
                self.focused_field = Some(focused_field.clone());

                let color = match focused_field {
                    FocusedField::Background => self.palette.background,
                    FocusedField::Text => self.palette.text,
                    FocusedField::Primary => self.palette.primary,
                    FocusedField::Success => self.palette.success,
                    FocusedField::Danger => self.palette.danger,
                    FocusedField::Warning => self.palette.warning,
                };

                self.hsv = Some(rgb_to_hsv(color.r, color.g, color.b));

                Action::None
            }

            Message::CloseRequested => Action::Exit,
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let close_editor = button(icon_text(Icon::Return, 11))
            .on_press(Message::CloseRequested)
            .style(move |theme, status| style::button::transparent(theme, status, false));

        let color_editor = {
            let (h, s, v) = self.hsv.unwrap_or((0.0, 0.0, 0.0));

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
            FocusedField::ALL.to_vec(),
            self.focused_field.clone(),
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

fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
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
