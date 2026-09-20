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

#[derive(Default)]
pub struct TickerSeriesEditor {
    pub show_config_for: Option<TickerInfo>,
    pub editing_color: Option<SeriesColor>,
    pub editing_line_width: Option<u8>,
    pub editing_stroke_style: Option<LineStrokeStyle>,
}

impl TickerSeriesEditor {
    pub fn open_for(
        &mut self,
        ticker: TickerInfo,
        color: SeriesColor,
        line_width: u8,
        stroke_style: LineStrokeStyle,
    ) {
        self.show_config_for = Some(ticker);
        self.editing_color = Some(color);
        self.editing_line_width = Some(line_width);
        self.editing_stroke_style = Some(stroke_style);
    }

    pub fn is_open_for(&self, ticker: TickerInfo) -> bool {
        self.show_config_for == Some(ticker)
    }

    pub fn update(&mut self, message: Message) -> Option<Change> {
        match message {
            Message::ColorAccentChanged(accent) => {
                let shade = self.editing_theme_color().shade;
                self.update_theme_color(accent, shade)
            }
            Message::ColorShadeChanged(shade) => {
                let accent = self.editing_theme_color().accent;
                self.update_theme_color(accent, shade)
            }
            Message::LineWidthChanged(width) => {
                self.editing_line_width = Some(width);
                self.show_config_for
                    .map(|ticker| Change::LineWidthChanged(ticker, width))
            }
            Message::StrokeStyleChanged(stroke_style) => {
                self.editing_stroke_style = Some(stroke_style);
                self.show_config_for
                    .map(|ticker| Change::StrokeStyleChanged(ticker, stroke_style))
            }
        }
    }

    fn editing_theme_color(&self) -> data::config::theme::ThemeColor {
        self.editing_color
            .and_then(SeriesColor::theme_color)
            .unwrap_or_default()
    }

    fn update_theme_color(
        &mut self,
        accent: Option<ThemeAccent>,
        shade: ThemeColorShade,
    ) -> Option<Change> {
        let color = SeriesColor::theme(accent, shade);
        self.editing_color = Some(color);
        self.show_config_for
            .map(|ticker| Change::ColorChanged(ticker, color))
    }

    pub(crate) fn view<'a>(
        &'a self,
        entry: &SeriesStyleEntry,
        theme_accents: ThemeAccents,
    ) -> Element<'a, Message> {
        let is_open_for_entry = self.is_open_for(entry.ticker);
        let color = is_open_for_entry
            .then_some(self.editing_color)
            .flatten()
            .unwrap_or(entry.color);
        let line_width = is_open_for_entry
            .then_some(self.editing_line_width)
            .flatten()
            .unwrap_or(entry.line_width);
        let stroke_style = is_open_for_entry
            .then_some(self.editing_stroke_style)
            .flatten()
            .unwrap_or(entry.stroke_style);
        let theme_color = color.theme_color().unwrap_or_default();
        let color_choices =
            theme_color_control(theme_color, theme_accents, Message::ColorAccentChanged);

        let color_controls = column![color_choices].width(Length::Fill).spacing(4);

        let line_width_slider = classic_slider_row(
            Some(text("Width")),
            MIN_LINE_WIDTH..=MAX_LINE_WIDTH,
            line_width,
            Message::LineWidthChanged,
            |value| value.to_string(),
            Some(1),
        );
        let stroke_picker = pick_list(
            LineStrokeStyle::ALL,
            Some(stroke_style),
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

    pub fn close(&mut self) {
        self.show_config_for = None;
        self.editing_color = None;
        self.editing_line_width = None;
        self.editing_stroke_style = None;
    }
}
