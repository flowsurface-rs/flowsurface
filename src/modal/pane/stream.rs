use crate::{
    style::{self, icon_text},
    widget::numeric_input_box,
};

use data::chart::Basis;
use exchange::{
    TickMultiplier, TickerInfo, Timeframe,
    adapter::{Exchange, hyperliquid::allowed_multipliers_for_base_tick},
};
use iced::{
    Element, Length,
    alignment::Horizontal,
    padding,
    widget::{button, column, container, horizontal_rule, row, scrollable, text},
};
use serde::{Deserialize, Serialize};

const NUMERIC_INPUT_BUF_SIZE: usize = 5; // Max 5 digits for u16 (65535)

const TICK_COUNT_MIN: u16 = 4;
const TICK_COUNT_MAX: u16 = 1000;

const TICK_MULTIPLIER_MIN: u16 = 1;
const TICK_MULTIPLIER_MAX: u16 = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub enum ModifierKind {
    Candlestick(Basis),
    Footprint(Basis, TickMultiplier),
    Heatmap(Basis, TickMultiplier),
    Orderbook(Basis, TickMultiplier),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct NumericInput {
    buffer: [u8; NUMERIC_INPUT_BUF_SIZE],
    len: u8,
}

impl NumericInput {
    pub fn new() -> Self {
        Self {
            buffer: [0; NUMERIC_INPUT_BUF_SIZE],
            len: 0,
        }
    }

    pub fn from_str(s: &str) -> Self {
        let mut buffer = [0; NUMERIC_INPUT_BUF_SIZE];
        let bytes = s.as_bytes();
        let len = bytes.len().min(NUMERIC_INPUT_BUF_SIZE);
        buffer[..len].copy_from_slice(&bytes[..len]);
        Self {
            buffer,
            len: len as u8,
        }
    }

    pub fn from_tick_multiplier(tm: TickMultiplier) -> Self {
        Self::from_str(&tm.0.to_string())
    }

    pub fn from_tick_count(tc: data::aggr::TickCount) -> Self {
        Self::from_str(&tc.0.to_string())
    }

    pub fn to_display_string(self) -> String {
        if self.len == 0 {
            return String::new();
        }
        String::from_utf8_lossy(&self.buffer[..self.len as usize]).into_owned()
    }

    pub fn is_empty(self) -> bool {
        self.len == 0
    }

    pub fn parse_tick_multiplier(self) -> Option<TickMultiplier> {
        if self.len == 0 {
            return None;
        }
        std::str::from_utf8(&self.buffer[..self.len as usize])
            .ok()
            .and_then(|s| s.parse::<u16>().ok())
            .map(TickMultiplier)
    }

    pub fn parse_tick_count(self) -> Option<data::aggr::TickCount> {
        if self.len == 0 {
            return None;
        }
        std::str::from_utf8(&self.buffer[..self.len as usize])
            .ok()
            .and_then(|s| s.parse::<u16>().ok())
            .map(data::aggr::TickCount)
    }
}

impl Default for NumericInput {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum ViewMode {
    BasisSelection,
    TicksizeSelection {
        raw_input_buf: NumericInput,
        parsed_input: Option<TickMultiplier>,
        is_input_valid: bool,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum SelectedTab {
    Timeframe,
    TickCount {
        raw_input_buf: NumericInput,
        parsed_input: Option<data::aggr::TickCount>,
        is_input_valid: bool,
    },
}

pub enum Action {
    BasisSelected(Basis),
    TicksizeSelected(TickMultiplier),
    TabSelected(SelectedTab),
}

#[derive(Debug, Clone)]
pub enum Message {
    BasisSelected(Basis),
    TabSelected(SelectedTab),
    TicksizeInputChanged(String),
    TicksizeSelected(TickMultiplier),
    TickCountInputChanged(String),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Modifier {
    pub tab: SelectedTab,
    pub view_mode: ViewMode,
    kind: ModifierKind,
    base_ticksize: Option<f32>,
    exchange: Option<Exchange>,
}

impl Modifier {
    pub fn new(kind: ModifierKind) -> Self {
        let tab = SelectedTab::from(&kind);

        Self {
            tab,
            kind,
            view_mode: ViewMode::BasisSelection,
            base_ticksize: None,
            exchange: None,
        }
    }

    pub fn with_view_mode(mut self, view_mode: ViewMode) -> Self {
        self.view_mode = view_mode;
        self
    }

    pub fn with_ticksize_view(
        mut self,
        base_ticksize: f32,
        multiplier: TickMultiplier,
        exchange: Option<Exchange>,
    ) -> Self {
        self.view_mode = ViewMode::TicksizeSelection {
            raw_input_buf: if multiplier.is_custom() {
                NumericInput::from_tick_multiplier(multiplier)
            } else {
                NumericInput::default()
            },
            parsed_input: if multiplier.is_custom() {
                Some(multiplier)
            } else {
                None
            },
            is_input_valid: true,
        };
        self.base_ticksize = Some(base_ticksize);
        self.exchange = exchange;
        self
    }

    pub fn update_kind_with_basis(&mut self, basis: Basis) {
        match self.kind {
            ModifierKind::Candlestick(_) => self.kind = ModifierKind::Candlestick(basis),
            ModifierKind::Footprint(_, ticksize) => {
                self.kind = ModifierKind::Footprint(basis, ticksize);
            }
            ModifierKind::Heatmap(_, ticksize) => {
                self.kind = ModifierKind::Heatmap(basis, ticksize);
            }
            ModifierKind::Orderbook(_, ticksize) => {
                self.kind = ModifierKind::Orderbook(basis, ticksize);
            }
        }
    }

    pub fn update_kind_with_multiplier(&mut self, ticksize: TickMultiplier) {
        match self.kind {
            ModifierKind::Footprint(basis, _) => {
                self.kind = ModifierKind::Footprint(basis, ticksize);
            }
            ModifierKind::Heatmap(basis, _) => self.kind = ModifierKind::Heatmap(basis, ticksize),
            ModifierKind::Orderbook(basis, _) => {
                self.kind = ModifierKind::Orderbook(basis, ticksize)
            }
            _ => {}
        }
    }

    pub fn update(&mut self, message: Message) -> Option<Action> {
        match message {
            Message::TabSelected(tab) => Some(Action::TabSelected(tab)),
            Message::BasisSelected(basis) => match basis {
                Basis::Time(_) => Some(Action::BasisSelected(basis)),
                Basis::Tick(new_tc) => {
                    if let SelectedTab::TickCount {
                        raw_input_buf,
                        parsed_input,
                        is_input_valid,
                    } = &mut self.tab
                    {
                        if *parsed_input == Some(new_tc) {
                            *is_input_valid = true;
                        } else {
                            *raw_input_buf = NumericInput::default();
                            *parsed_input = None;
                            *is_input_valid = true;
                        };

                        Some(Action::BasisSelected(basis))
                    } else {
                        None
                    }
                }
            },
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
                        *raw_input_buf = NumericInput::default();
                        *parsed_input = None;
                        *is_input_valid = true;
                    };
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
                        value_str.chars().filter(char::is_ascii_digit).collect();

                    *raw_input_buf = NumericInput::from_str(&numeric_value_str);
                    *parsed_input = raw_input_buf.parse_tick_multiplier();

                    if raw_input_buf.is_empty() {
                        *is_input_valid = true;
                    } else {
                        match parsed_input {
                            Some(tm) => {
                                *is_input_valid =
                                    tm.0 >= TICK_MULTIPLIER_MIN && tm.0 <= TICK_MULTIPLIER_MAX;
                            }
                            None => {
                                *is_input_valid = false;
                            }
                        }
                    }
                }
                None
            }
            Message::TickCountInputChanged(value_str) => {
                if let SelectedTab::TickCount {
                    ref mut raw_input_buf,
                    ref mut parsed_input,
                    ref mut is_input_valid,
                } = self.tab
                {
                    let numeric_value_str: String =
                        value_str.chars().filter(char::is_ascii_digit).collect();

                    *raw_input_buf = NumericInput::from_str(&numeric_value_str);
                    *parsed_input = raw_input_buf.parse_tick_count();

                    if raw_input_buf.is_empty() {
                        *is_input_valid = true;
                    } else {
                        match parsed_input {
                            Some(tc) => {
                                *is_input_valid = tc.0 >= TICK_COUNT_MIN && tc.0 <= TICK_COUNT_MAX;
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

    pub fn view<'a>(&self, ticker_info: Option<TickerInfo>) -> Element<'a, Message> {
        let kind = self.kind;

        let (selected_basis, selected_ticksize) = match kind {
            ModifierKind::Candlestick(basis) => (Some(basis), None),
            ModifierKind::Footprint(basis, ticksize)
            | ModifierKind::Heatmap(basis, ticksize)
            | ModifierKind::Orderbook(basis, ticksize) => (Some(basis), Some(ticksize)),
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
                    ModifierKind::Heatmap(_, _) | ModifierKind::Orderbook(_, _) => false,
                };

                if selected_basis.is_some() {
                    let (timeframe_tab_is_selected, tick_count_tab_is_selected) = match self.tab {
                        SelectedTab::Timeframe => (true, false),
                        SelectedTab::TickCount { .. } => (false, true),
                    };

                    let tabs_row = {
                        if is_kline_chart {
                            let is_timeframe_selected =
                                matches!(selected_basis, Some(Basis::Time(_)));

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
                                        let tick_count_tab = match self.tab {
                                            SelectedTab::TickCount {
                                                raw_input_buf,
                                                parsed_input,
                                                is_input_valid,
                                            } => SelectedTab::TickCount {
                                                raw_input_buf,
                                                parsed_input,
                                                is_input_valid,
                                            },
                                            _ => SelectedTab::TickCount {
                                                raw_input_buf: NumericInput::default(),
                                                parsed_input: None,
                                                is_input_valid: true,
                                            },
                                        };
                                        Some(Message::TabSelected(tick_count_tab))
                                    },
                                    !tick_count_tab_is_selected,
                                    !is_timeframe_selected,
                                ),
                            ]
                            .spacing(4)
                        } else {
                            row![text("Aggregation").size(13)]
                        }
                    };

                    basis_selection_column = basis_selection_column
                        .push(tabs_row)
                        .push(horizontal_rule(1).style(style::split_ruler));
                }

                match self.tab {
                    SelectedTab::Timeframe => {
                        let selected_tf = match selected_basis {
                            Some(Basis::Time(tf)) => Some(tf),
                            _ => None,
                        };

                        if is_kline_chart {
                            let kline_timeframe_grid = modifiers_grid(
                                &Timeframe::KLINE,
                                selected_tf,
                                |tf| Message::BasisSelected(tf.into()),
                                &create_button,
                                3,
                            );
                            basis_selection_column =
                                basis_selection_column.push(kline_timeframe_grid);
                        } else if let Some(info) = ticker_info {
                            let heatmap_timeframes: Vec<Timeframe> = Timeframe::HEATMAP
                                .iter()
                                .copied()
                                .filter(|tf| info.exchange().supports_heatmap_timeframe(*tf))
                                .collect();
                            let heatmap_timeframe_grid = modifiers_grid(
                                &heatmap_timeframes,
                                selected_tf,
                                |tf| Message::BasisSelected(tf.into()),
                                &create_button,
                                2,
                            );
                            basis_selection_column =
                                basis_selection_column.push(heatmap_timeframe_grid);
                        }
                    }
                    SelectedTab::TickCount {
                        raw_input_buf,
                        parsed_input,
                        is_input_valid,
                    } => {
                        let selected_tick_count = match selected_basis {
                            Some(Basis::Tick(tc)) => Some(tc),
                            _ => None,
                        };

                        let tick_count_grid = modifiers_grid(
                            &data::aggr::TickCount::ALL,
                            selected_tick_count,
                            |tc| Message::BasisSelected(Basis::Tick(tc)),
                            &create_button,
                            3,
                        );

                        let custom_input = {
                            let tick_count_to_submit = parsed_input
                                .filter(|tc| tc.0 >= TICK_COUNT_MIN && tc.0 <= TICK_COUNT_MAX);

                            numeric_input_box::<_, Message>(
                                "Custom: ",
                                &format!("{}-{}", TICK_COUNT_MIN, TICK_COUNT_MAX),
                                &raw_input_buf.to_display_string(),
                                is_input_valid,
                                Message::TickCountInputChanged,
                                tick_count_to_submit
                                    .map(|tc| Message::BasisSelected(Basis::Tick(tc))),
                            )
                        };
                        basis_selection_column = basis_selection_column.push(custom_input);
                        basis_selection_column = basis_selection_column.push(tick_count_grid);
                    }
                }

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
                let Some(exchange) = self.exchange else {
                    return container(text("Exchange information is not available"))
                        .padding(16)
                        .style(style::chart_modal)
                        .into();
                };

                if let Some(ticksize) = selected_ticksize {
                    let mut ticksizes_column =
                        column![].padding(4).spacing(8).align_x(Horizontal::Center);

                    ticksizes_column = ticksizes_column
                        .push(text("Tick size multiplier").size(13))
                        .push(horizontal_rule(1).style(style::split_ruler));

                    let allows_custom_tsizes = exchange.is_depth_client_aggr()
                        || matches!(kind, ModifierKind::Footprint(_, _));

                    let allowed_tm = if allows_custom_tsizes {
                        exchange::TickMultiplier::ALL.to_vec()
                    } else {
                        let base = self.base_ticksize.unwrap_or(0.0);
                        let allow = allowed_multipliers_for_base_tick(base);
                        exchange::TickMultiplier::ALL
                            .iter()
                            .copied()
                            .filter(|tm| allow.contains(&tm.0))
                            .collect()
                    };

                    let tick_multiplier_grid = modifiers_grid(
                        &allowed_tm,
                        Some(ticksize),
                        Message::TicksizeSelected,
                        &create_button,
                        3,
                    );

                    if allows_custom_tsizes {
                        let custom_input = {
                            let tick_multiplier_to_submit = parsed_input.filter(|tm| {
                                tm.0 >= TICK_MULTIPLIER_MIN && tm.0 <= TICK_MULTIPLIER_MAX
                            });

                            numeric_input_box::<_, Message>(
                                "Custom: ",
                                &format!("{}-{}", TICK_MULTIPLIER_MIN, TICK_MULTIPLIER_MAX),
                                &raw_input_buf.to_display_string(),
                                is_input_valid,
                                Message::TicksizeInputChanged,
                                tick_multiplier_to_submit.map(Message::TicksizeSelected),
                            )
                        };

                        ticksizes_column = ticksizes_column.push(custom_input);
                    }
                    ticksizes_column = ticksizes_column.push(tick_multiplier_grid);

                    if let Some(base_ticksize) = self.base_ticksize {
                        ticksizes_column = ticksizes_column.push(
                            row![
                                iced::widget::horizontal_space(),
                                text(format!("Base: {}", base_ticksize)).style(
                                    |theme: &iced::Theme| {
                                        iced::widget::text::Style {
                                            color: Some(
                                                theme.extended_palette().background.strongest.color,
                                            ),
                                        }
                                    }
                                ),
                            ]
                            .padding(padding::top(8).right(4)),
                        );
                    }

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

fn modifiers_grid<'a, T, FMsg>(
    items_source: &[T],
    selected_value: Option<T>,
    to_message: FMsg,
    create_button_fn: &impl Fn(
        iced::widget::text::Text<'a>,
        Option<Message>,
        bool,
    ) -> iced::widget::Button<'a, Message>,
    items_per_row: usize,
) -> iced::widget::Column<'a, Message>
where
    T: Copy + PartialEq + ToString,
    FMsg: Fn(T) -> Message,
{
    let mut grid_column = column![].spacing(4);

    for chunk in items_source.chunks(items_per_row) {
        let mut button_row = row![].spacing(4);

        for &item_value in chunk {
            let is_selected = selected_value == Some(item_value);
            let msg = if is_selected {
                None
            } else {
                Some(to_message(item_value))
            };
            button_row = button_row.push(create_button_fn(
                text(item_value.to_string()),
                msg,
                is_selected,
            ));
        }

        grid_column = grid_column.push(button_row);
    }

    grid_column
}

impl From<&ModifierKind> for SelectedTab {
    fn from(kind: &ModifierKind) -> Self {
        match kind {
            ModifierKind::Candlestick(basis)
            | ModifierKind::Footprint(basis, _)
            | ModifierKind::Heatmap(basis, _)
            | ModifierKind::Orderbook(basis, _) => match basis {
                Basis::Time(_) => SelectedTab::Timeframe,
                Basis::Tick(tc) => SelectedTab::TickCount {
                    raw_input_buf: if tc.is_custom() {
                        NumericInput::from_tick_count(*tc)
                    } else {
                        NumericInput::default()
                    },
                    parsed_input: if tc.is_custom() { Some(*tc) } else { None },
                    is_input_valid: true,
                },
            },
        }
    }
}
