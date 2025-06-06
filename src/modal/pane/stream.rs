use crate::style::{self, icon_text};

use data::chart::Basis;
use exchange::{TickMultiplier, Ticker, Timeframe, adapter::Exchange};
use iced::{
    Element, Length,
    alignment::Horizontal,
    padding,
    widget::{button, column, container, horizontal_rule, row, scrollable, text, text_input},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub enum ModifierKind {
    Candlestick(Basis),
    Footprint(Basis, TickMultiplier),
    Heatmap(Basis, TickMultiplier),
}

const RAW_TICK_INPUT_BUF_SIZE: usize = 5; // Max 5 digits for u16 (65535)

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct RawTickInput {
    buffer: [u8; RAW_TICK_INPUT_BUF_SIZE],
    len: u8,
}

impl RawTickInput {
    pub fn new() -> Self {
        Self {
            buffer: [0; RAW_TICK_INPUT_BUF_SIZE],
            len: 0,
        }
    }

    pub fn from_str(s: &str) -> Self {
        let mut buffer = [0; RAW_TICK_INPUT_BUF_SIZE];
        let bytes = s.as_bytes();
        let len = bytes.len().min(RAW_TICK_INPUT_BUF_SIZE);
        buffer[..len].copy_from_slice(&bytes[..len]);
        Self {
            buffer,
            len: len as u8,
        }
    }

    pub fn from_tick_multiplier(tm: TickMultiplier) -> Self {
        Self::from_str(&tm.0.to_string())
    }

    pub fn to_display_string(&self) -> String {
        if self.len == 0 {
            return String::new();
        }
        String::from_utf8_lossy(&self.buffer[..self.len as usize]).into_owned()
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn parse_tick_multiplier(&self) -> Option<TickMultiplier> {
        if self.len == 0 {
            return None;
        }
        std::str::from_utf8(&self.buffer[..self.len as usize])
            .ok()
            .and_then(|s| s.parse::<u16>().ok())
            .map(TickMultiplier)
    }
}

impl Default for RawTickInput {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum ViewMode {
    BasisSelection,
    TicksizeSelection {
        raw_input_buf: RawTickInput,
        parsed_input: Option<TickMultiplier>,
        is_input_valid: bool,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum SelectedTab {
    Timeframe,
    TickCount,
}

pub enum Action {
    BasisSelected(Basis),
    TicksizeSelected(TickMultiplier),
    TabSelected(SelectedTab),
}

#[derive(Debug, Clone)]
pub enum Message {
    BasisSelected(Basis),
    TicksizeSelected(TickMultiplier),
    TabSelected(SelectedTab),
    TicksizeInputChanged(String),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Modifier {
    pub tab: SelectedTab,
    pub view_mode: ViewMode,
    kind: ModifierKind,
}

impl Modifier {
    pub fn new(kind: ModifierKind) -> Self {
        let tab = SelectedTab::from(&kind);

        Self {
            tab,
            kind,
            view_mode: ViewMode::BasisSelection,
        }
    }

    pub fn with_view_mode(mut self, view_mode: ViewMode) -> Self {
        self.view_mode = view_mode;
        self
    }

    pub fn update_kind_with_basis(&mut self, basis: Basis) {
        match self.kind {
            ModifierKind::Candlestick(_) => self.kind = ModifierKind::Candlestick(basis),
            ModifierKind::Footprint(_, ticksize) => {
                self.kind = ModifierKind::Footprint(basis, ticksize)
            }
            ModifierKind::Heatmap(_, ticksize) => {
                self.kind = ModifierKind::Heatmap(basis, ticksize)
            }
        }
    }

    pub fn update_kind_with_multiplier(&mut self, ticksize: TickMultiplier) {
        match self.kind {
            ModifierKind::Footprint(basis, _) => {
                self.kind = ModifierKind::Footprint(basis, ticksize)
            }
            ModifierKind::Heatmap(basis, _) => self.kind = ModifierKind::Heatmap(basis, ticksize),
            _ => {}
        }
    }

    pub fn update(&mut self, message: Message) -> Option<Action> {
        match message {
            Message::TabSelected(tab) => Some(Action::TabSelected(tab)),
            Message::BasisSelected(basis) => Some(Action::BasisSelected(basis)),
            Message::TicksizeSelected(new_ticksize) => {
                if let ViewMode::TicksizeSelection {
                    ref mut raw_input_buf,
                    ref mut parsed_input,
                    ref mut is_input_valid,
                } = self.view_mode
                {
                    if *parsed_input == Some(new_ticksize) {
                        *is_input_valid = true;
                    } else {
                        *raw_input_buf = RawTickInput::default();
                        *parsed_input = None;
                        *is_input_valid = true;
                    }
                }
                Some(Action::TicksizeSelected(new_ticksize))
            }
            Message::TicksizeInputChanged(value_str) => {
                if let ViewMode::TicksizeSelection {
                    ref mut raw_input_buf,
                    ref mut parsed_input,
                    ref mut is_input_valid,
                } = self.view_mode
                {
                    let numeric_value_str: String =
                        value_str.chars().filter(|c| c.is_ascii_digit()).collect();

                    *raw_input_buf = RawTickInput::from_str(&numeric_value_str);
                    *parsed_input = raw_input_buf.parse_tick_multiplier();

                    if raw_input_buf.is_empty() {
                        *is_input_valid = true;
                    } else {
                        match parsed_input {
                            Some(tm) => {
                                *is_input_valid = tm.0 >= 1 && tm.0 <= 2000;
                            }
                            None => {
                                *is_input_valid = false;
                            }
                        }
                    }
                }
                None
            }
        }
    }

    pub fn view<'a>(&self, ticker_info: Option<(Exchange, Ticker)>) -> Element<'a, Message> {
        let kind = self.kind;

        let (selected_basis, selected_ticksize) = match kind {
            ModifierKind::Candlestick(basis) => (Some(basis), None),
            ModifierKind::Footprint(basis, ticksize) | ModifierKind::Heatmap(basis, ticksize) => {
                (Some(basis), Some(ticksize))
            }
        };

        let create_button = |content: iced::widget::text::Text<'a>,
                             msg: Option<Message>,
                             is_selected: bool| {
            let btn = button(content.align_x(iced::Alignment::Center))
                .width(Length::Fill)
                .style(move |theme, status| style::button::menu_body(theme, status, is_selected));

            if let Some(msg) = msg {
                btn.on_press(msg)
            } else {
                btn
            }
        };

        match self.view_mode {
            ViewMode::BasisSelection => {
                let mut basis_selection_column =
                    column![].padding(4).spacing(8).align_x(Horizontal::Center);

                let is_kline_chart = match kind {
                    ModifierKind::Candlestick(_) | ModifierKind::Footprint(_, _) => true,
                    ModifierKind::Heatmap(_, _) => false,
                };

                if selected_basis.is_some() {
                    let (timeframe_tab_is_selected, tick_count_tab_is_selected) = match self.tab {
                        SelectedTab::Timeframe => (true, false),
                        SelectedTab::TickCount => (false, true),
                    };

                    let tabs_row = {
                        if is_kline_chart {
                            let is_timeframe_selected = match selected_basis {
                                Some(Basis::Time(_)) => true,
                                _ => false,
                            };

                            let tab_button =
                                |content: iced::widget::text::Text<'a>,
                                 msg: Option<Message>,
                                 active: bool,
                                 checkmark: bool| {
                                    let content = if checkmark {
                                        row![
                                            content,
                                            iced::widget::horizontal_space(),
                                            icon_text(style::Icon::Checkmark, 12)
                                        ]
                                    } else {
                                        row![content]
                                    }
                                    .width(Length::Fill);

                                    let btn = button(content).style(move |theme, status| {
                                        style::button::transparent(theme, status, active)
                                    });

                                    if let Some(msg) = msg {
                                        btn.on_press(msg)
                                    } else {
                                        btn
                                    }
                                };

                            row![
                                tab_button(
                                    text("Timeframe"),
                                    if timeframe_tab_is_selected {
                                        None
                                    } else {
                                        Some(Message::TabSelected(SelectedTab::Timeframe))
                                    },
                                    !timeframe_tab_is_selected,
                                    is_timeframe_selected,
                                ),
                                tab_button(
                                    text("Ticks"),
                                    if tick_count_tab_is_selected {
                                        None
                                    } else {
                                        Some(Message::TabSelected(SelectedTab::TickCount))
                                    },
                                    !tick_count_tab_is_selected,
                                    !is_timeframe_selected,
                                ),
                            ]
                            .spacing(4)
                        } else {
                            row![text("Aggregation")]
                                .padding(padding::bottom(8))
                                .spacing(4)
                        }
                    };

                    basis_selection_column = basis_selection_column
                        .push(tabs_row)
                        .push(horizontal_rule(1).style(style::split_ruler));
                }

                let mut modifiers = column![].spacing(4);

                match self.tab {
                    SelectedTab::Timeframe => {
                        let selected_tf = match selected_basis {
                            Some(Basis::Time(tf)) => Some(tf),
                            _ => None,
                        };

                        if is_kline_chart {
                            for chunk in Timeframe::KLINE.chunks(3) {
                                let mut button_row = row![].spacing(4);

                                for timeframe in chunk {
                                    let is_selected = selected_tf == Some(*timeframe);
                                    let msg = if is_selected {
                                        None
                                    } else {
                                        Some(Message::BasisSelected((*timeframe).into()))
                                    };
                                    button_row = button_row.push(create_button(
                                        text(timeframe.to_string()),
                                        msg,
                                        is_selected,
                                    ));
                                }

                                modifiers = modifiers.push(button_row);
                            }
                        } else if let Some((exchange, _)) = ticker_info {
                            let heatmap_timeframes: Vec<_> = Timeframe::HEATMAP
                                .iter()
                                .filter(|tf| {
                                    !(exchange == Exchange::BybitSpot && *tf == &Timeframe::MS100)
                                })
                                .collect();

                            for chunk in heatmap_timeframes.chunks(3) {
                                let mut button_row = row![].spacing(4);

                                for timeframe_ref in chunk {
                                    let timeframe = **timeframe_ref;
                                    let is_selected = selected_tf == Some(timeframe);
                                    let msg = if is_selected {
                                        None
                                    } else {
                                        Some(Message::BasisSelected(timeframe.into()))
                                    };
                                    button_row = button_row.push(create_button(
                                        text(timeframe.to_string()),
                                        msg,
                                        is_selected,
                                    ));
                                }

                                modifiers = modifiers.push(button_row);
                            }
                        }
                    }
                    SelectedTab::TickCount => {
                        let selected_tick_count = match selected_basis {
                            Some(Basis::Tick(tc)) => Some(tc),
                            _ => None,
                        };

                        for chunk in data::aggr::TickCount::ALL.chunks(3) {
                            let mut button_row = row![].spacing(4);

                            for &tick_count in chunk {
                                let is_selected = selected_tick_count == Some(tick_count.0);
                                let msg = if is_selected {
                                    None
                                } else {
                                    Some(Message::BasisSelected(Basis::Tick(tick_count.0)))
                                };

                                button_row = button_row.push(create_button(
                                    text(tick_count.to_string()),
                                    msg,
                                    is_selected,
                                ));
                            }

                            modifiers = modifiers.push(button_row);
                        }
                    }
                }

                basis_selection_column = basis_selection_column.push(modifiers);

                container(scrollable::Scrollable::with_direction(
                    basis_selection_column,
                    scrollable::Direction::Vertical(
                        scrollable::Scrollbar::new().width(4).scroller_width(4),
                    ),
                ))
                .max_width(240)
                .padding(16)
                .style(style::chart_modal)
                .into()
            }
            ViewMode::TicksizeSelection {
                raw_input_buf,
                parsed_input,
                is_input_valid,
            } => {
                if let Some(ticksize) = selected_ticksize {
                    let mut ticksizes_column =
                        column![].padding(4).spacing(8).align_x(Horizontal::Center);

                    ticksizes_column = ticksizes_column
                        .push(text("Ticksize Mltp."))
                        .push(horizontal_rule(1).style(style::split_ruler));

                    let mut modifiers = column![].spacing(4);

                    for chunk in exchange::TickMultiplier::ALL.chunks(3) {
                        let mut button_row = row![].spacing(4);

                        for ticksize_value in chunk {
                            let is_selected = ticksize == *ticksize_value;
                            let msg = if is_selected {
                                None
                            } else {
                                Some(Message::TicksizeSelected(*ticksize_value))
                            };
                            button_row = button_row.push(create_button(
                                text(ticksize_value.to_string()),
                                msg,
                                is_selected,
                            ));
                        }

                        modifiers = modifiers.push(button_row);
                    }

                    let tick_multiplier_to_submit =
                        parsed_input.filter(|tm| tm.0 >= 1 && tm.0 <= 2000);

                    let custom_tsize = text_input("1-2000", &raw_input_buf.to_display_string())
                        .on_input(Message::TicksizeInputChanged)
                        .on_submit_maybe(tick_multiplier_to_submit.map(Message::TicksizeSelected))
                        .align_x(iced::Alignment::Center)
                        .style(move |theme, status| {
                            style::validated_text_input(theme, status, is_input_valid)
                        });

                    ticksizes_column = ticksizes_column.push(modifiers).push(
                        row![text("Custom: "), custom_tsize]
                            .padding(padding::right(20).left(20))
                            .spacing(4)
                            .align_y(iced::Alignment::Center),
                    );

                    container(scrollable::Scrollable::with_direction(
                        ticksizes_column,
                        scrollable::Direction::Vertical(
                            scrollable::Scrollbar::new().width(4).scroller_width(4),
                        ),
                    ))
                    .max_width(240)
                    .padding(16)
                    .style(style::chart_modal)
                    .into()
                } else {
                    container(text("No ticksize available for this chart type"))
                        .padding(16)
                        .style(style::chart_modal)
                        .into()
                }
            }
        }
    }
}

impl From<&ModifierKind> for SelectedTab {
    fn from(kind: &ModifierKind) -> Self {
        match kind {
            ModifierKind::Candlestick(basis)
            | ModifierKind::Footprint(basis, _)
            | ModifierKind::Heatmap(basis, _) => match basis {
                Basis::Time(_) => SelectedTab::Timeframe,
                Basis::Tick(_) => SelectedTab::TickCount,
            },
        }
    }
}
