use crate::{
    theme::ThemeAccents,
    widget::{classic_slider_row, theme_color_control},
};
use data::chart::kline::{LineStrokeStyle, MAX_LINE_WIDTH, MIN_LINE_WIDTH, SeriesColor};
use data::config::theme::{ThemeAccent, ThemeColorShade};
use exchange::TickerInfo;
use iced::widget::{column, pick_list, row, text};
use iced::{Element, Length};

#[derive(Debug, Clone)]
pub enum Message {
    ColorAccentChanged(Option<ThemeAccent>),
    ColorShadeChanged(ThemeColorShade),
    LineWidthChanged(u8),
    StrokeStyleChanged(LineStrokeStyle),
}

#[derive(Debug, Clone)]
pub enum Change {
    ColorChanged(TickerInfo, SeriesColor),
    LineWidthChanged(TickerInfo, u8),
    StrokeStyleChanged(TickerInfo, LineStrokeStyle),
}

#[derive(Debug, Clone)]
pub struct SeriesStyleEntry {
    pub ticker: TickerInfo,
    pub color: SeriesColor,
    pub line_width: u8,
    pub stroke_style: LineStrokeStyle,
}

impl SeriesStyleEntry {
    pub fn change(&self, message: Message) -> Change {
        let theme_color = self.color.theme_color().unwrap_or_default();
        match message {
            Message::ColorAccentChanged(accent) => {
                Change::ColorChanged(self.ticker, SeriesColor::theme(accent, theme_color.shade))
            }
            Message::ColorShadeChanged(shade) => {
                Change::ColorChanged(self.ticker, SeriesColor::theme(theme_color.accent, shade))
            }
            Message::LineWidthChanged(width) => Change::LineWidthChanged(self.ticker, width),
            Message::StrokeStyleChanged(style) => Change::StrokeStyleChanged(self.ticker, style),
        }
    }

    pub(crate) fn view<'a>(&self, theme_accents: ThemeAccents) -> Element<'a, Message> {
        let theme_color = self.color.theme_color().unwrap_or_default();
        let color_choices =
            theme_color_control(theme_color, theme_accents, Message::ColorAccentChanged);

        let color_controls = column![color_choices].width(Length::Fill).spacing(4);

        let line_width_slider = classic_slider_row(
            Some(text("Width")),
            MIN_LINE_WIDTH..=MAX_LINE_WIDTH,
            self.line_width,
            Message::LineWidthChanged,
            |value| value.to_string(),
            Some(1),
        );
        let stroke_picker = pick_list(
            LineStrokeStyle::ALL,
            Some(self.stroke_style),
            Message::StrokeStyleChanged,
        )
        .width(Length::Fill);
        let shade_picklist = pick_list(
            ThemeColorShade::ALL,
            Some(theme_color.shade),
            Message::ColorShadeChanged,
        )
        .width(Length::Fill);

        let stroke_and_shade = row![stroke_picker, shade_picklist].spacing(4);

        column![color_controls, stroke_and_shade, line_width_slider,]
            .spacing(4)
            .into()
    }
}
