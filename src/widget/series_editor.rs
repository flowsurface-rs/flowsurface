use crate::style;
use crate::widget::{labeled_slider, theme_color_control};
use data::chart::kline_v2::{LineStrokeStyle, MAX_LINE_WIDTH, MIN_LINE_WIDTH, SeriesColor};
use data::config::theme::{ThemeAccent, ThemeColorShade};
use exchange::TickerInfo;
use iced::widget::{button, column, container, pick_list, text};
use iced::{Element, Length};

#[derive(Debug, Clone)]
pub enum Message {
    ToggleEditFor {
        ticker: TickerInfo,
        applied_color: SeriesColor,
        applied_line_width: u8,
        applied_stroke_style: LineStrokeStyle,
    },
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

    pub fn update(&mut self, message: Message) -> Option<Change> {
        match message {
            Message::ToggleEditFor {
                ticker,
                applied_color,
                applied_line_width,
                applied_stroke_style,
            } => {
                if self.show_config_for == Some(ticker) {
                    self.close();
                    return None;
                }

                self.open_for(
                    ticker,
                    applied_color,
                    applied_line_width,
                    applied_stroke_style,
                );
                None
            }
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

    pub fn view<'a>(&'a self, series: Vec<SeriesStyleEntry>) -> Element<'a, Message> {
        let mut content = column![].spacing(6);

        for entry in series {
            let is_open = self.show_config_for == Some(entry.ticker);
            let applied_color = entry.color;
            let applied_line_width = entry.line_width;
            let applied_stroke_style = entry.stroke_style;
            let ticker = entry.ticker;
            let header = button(
                text(ticker.ticker.symbol_and_exchange_string())
                    .center()
                    .size(crate::style::text_size::EMPHASIS),
            )
            .padding(1)
            .on_press(Message::ToggleEditFor {
                ticker,
                applied_color,
                applied_line_width,
                applied_stroke_style,
            })
            .style(move |theme, status| style::button::transparent(theme, status, is_open))
            .width(Length::Fill);

            let mut section = column![header].padding(4);
            if is_open {
                let color = self.editing_color.unwrap_or(applied_color);
                let line_width = self.editing_line_width.unwrap_or(applied_line_width);
                let stroke_style = self.editing_stroke_style.unwrap_or(applied_stroke_style);
                let theme_color = color.theme_color().unwrap_or_default();
                let color_choices = theme_color_control(theme_color, Message::ColorAccentChanged);
                let shade_picklist = pick_list(
                    ThemeColorShade::ALL,
                    Some(theme_color.shade),
                    Message::ColorShadeChanged,
                )
                .width(Length::Fill);
                let color_controls = column![color_choices, shade_picklist]
                    .width(Length::Fill)
                    .spacing(4);
                let line_width_slider = labeled_slider(
                    "Line width",
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
                let line_style_section = column![line_width_slider, stroke_picker,].spacing(4);
                let editor = column![
                    text("Color").size(crate::style::text_size::SECTION),
                    color_controls,
                    line_style_section,
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
        self.editing_line_width = None;
        self.editing_stroke_style = None;
    }
}
