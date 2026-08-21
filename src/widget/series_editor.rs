use crate::style;
use data::chart::kline_v2::SeriesColor;
use data::config::theme::generated_theme_accent;
use exchange::TickerInfo;
use iced::widget::{button, column, container, radio, row, text};
use iced::{Element, Length, Theme};

fn series_color(theme: &Theme, color: SeriesColor) -> iced::Color {
    let palette = theme.extended_palette();
    match color {
        SeriesColor::Base => palette.background.base.text,
        SeriesColor::Accent(accent) => generated_theme_accent(palette, accent),
        SeriesColor::Custom(color) => color,
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    ToggleEditFor {
        ticker: TickerInfo,
        applied_color: SeriesColor,
    },
    ColorChanged(iced::Color),
}

#[derive(Debug, Clone)]
pub enum Change {
    ColorChanged(TickerInfo, SeriesColor),
}

#[derive(Debug, Clone)]
pub struct SeriesStyleEntry {
    pub ticker: TickerInfo,
    pub color: SeriesColor,
}

#[derive(Default)]
pub struct TickerSeriesEditor {
    pub show_config_for: Option<TickerInfo>,
    pub editing_color: Option<SeriesColor>,
}

impl TickerSeriesEditor {
    pub fn open_for(&mut self, ticker: TickerInfo, color: SeriesColor) {
        self.show_config_for = Some(ticker);
        self.editing_color = Some(color);
    }

    pub fn update(&mut self, message: Message) -> Option<Change> {
        match message {
            Message::ToggleEditFor {
                ticker,
                applied_color,
            } => {
                if self.show_config_for == Some(ticker) {
                    self.close();
                    return None;
                }

                self.open_for(ticker, applied_color);
                None
            }
            Message::ColorChanged(color) => {
                let color = SeriesColor::Custom(color);
                self.editing_color = Some(color);
                self.show_config_for
                    .map(|ticker| Change::ColorChanged(ticker, color))
            }
        }
    }

    pub fn view<'a>(&'a self, series: Vec<SeriesStyleEntry>) -> Element<'a, Message> {
        let mut content = column![].spacing(6);

        for entry in series {
            let is_open = self.show_config_for == Some(entry.ticker);
            let applied_color = entry.color;
            let ticker = entry.ticker;
            let header = button(
                row![
                    container("").width(14).height(14).style(move |theme| {
                        style::colored_circle_container(theme, series_color(theme, applied_color))
                    }),
                    text(ticker.ticker.symbol_and_exchange_string())
                        .size(crate::style::text_size::EMPHASIS),
                ]
                .width(Length::Fill)
                .spacing(8)
                .align_y(iced::Alignment::Center),
            )
            .on_press(Message::ToggleEditFor {
                ticker,
                applied_color,
            })
            .style(move |theme, status| style::button::transparent(theme, status, is_open))
            .width(Length::Fill);

            let mut section = column![header].padding(4);
            if is_open {
                let color = self.editing_color.unwrap_or(applied_color);
                let custom_color = match color {
                    SeriesColor::Custom(color) => Some(color),
                    SeriesColor::Base | SeriesColor::Accent(_) => None,
                };
                let editor = column![
                    text("Color").size(crate::style::text_size::SECTION),
                    color_control(custom_color, Message::ColorChanged),
                ]
                .spacing(12)
                .padding(4);
                section = section.push(editor).spacing(4);
            }

            content = content.push(container(section).style(style::modal_container));
        }

        content.into()
    }

    pub fn close(&mut self) {
        self.show_config_for = None;
        self.editing_color = None;
    }
}

pub fn color_control<'a, Message>(
    selected: Option<iced::Color>,
    on_change: impl Fn(iced::Color) -> Message + Clone + 'a,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let color_options = [
        iced::Color::from_rgb(0.95, 0.35, 0.35),
        iced::Color::from_rgb(0.95, 0.65, 0.25),
        iced::Color::from_rgb(0.35, 0.8, 0.45),
        iced::Color::from_rgb(0.25, 0.65, 0.95),
        iced::Color::from_rgb(0.55, 0.4, 0.95),
        iced::Color::from_rgb(0.9, 0.35, 0.7),
    ];
    let selected_option =
        selected.and_then(|selected| color_options.iter().position(|color| *color == selected));
    let color_choices = color_options.into_iter().enumerate().map(|(index, color)| {
        let on_change = on_change.clone();
        radio(String::new(), index, selected_option, move |_| {
            on_change(color)
        })
        .style(move |theme, status| {
            let mut style = iced::widget::radio::default(theme, status);
            style.dot_color = color;
            style.border_color = color;
            style
        })
        .into()
    });
    let color_choices: Element<'a, Message> = row(color_choices).spacing(8).into();
    color_choices
}
