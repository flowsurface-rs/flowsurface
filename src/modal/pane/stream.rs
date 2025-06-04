use crate::style;

use data::chart::Basis;
use exchange::{TickMultiplier, Ticker, Timeframe, adapter::Exchange};
use iced::{
    Alignment, Element, Length,
    alignment::{Horizontal, Vertical},
    padding,
    widget::{button, column, container, horizontal_rule, pane_grid, row, scrollable, text},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub enum ModifierKind {
    Candlestick(Basis),
    Footprint(Basis, TickMultiplier),
    Heatmap(Basis, TickMultiplier),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum SelectedTab {
    Timeframe,
    TickCount,
}

pub enum Action {
    BasisSelected(Basis, pane_grid::Pane),
    TicksizeSelected(TickMultiplier, pane_grid::Pane),
    TabSelected(SelectedTab),
}

#[derive(Debug, Clone)]
pub enum Message {
    BasisSelected(Basis, pane_grid::Pane),
    TicksizeSelected(TickMultiplier, pane_grid::Pane),
    TabSelected(SelectedTab),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Modifier {
    pub tab: SelectedTab,
    pub kind: ModifierKind,
}

impl Modifier {
    pub fn new(kind: ModifierKind) -> Self {
        let tab = SelectedTab::from(&kind);
        Self { tab, kind }
    }

    pub fn set_tab(&mut self, tab: SelectedTab) {
        self.tab = tab;
    }

    pub fn update(&mut self, message: Message) -> Option<Action> {
        match message {
            Message::TabSelected(tab) => {
                self.tab = tab;
                Some(Action::TabSelected(tab))
            }
            Message::BasisSelected(basis, pane) => Some(Action::BasisSelected(basis, pane)),
            Message::TicksizeSelected(ticksize, pane) => {
                Some(Action::TicksizeSelected(ticksize, pane))
            }
        }
    }

    pub fn view<'a>(
        &self,
        pane: pane_grid::Pane,
        ticker_info: Option<(Exchange, Ticker)>,
    ) -> Element<'a, Message> {
        let kind = self.kind;

        let (selected_basis_from_kind, selected_ticksize) = match kind {
            ModifierKind::Candlestick(basis) => (Some(basis), None),
            ModifierKind::Footprint(basis, ticksize) | ModifierKind::Heatmap(basis, ticksize) => {
                (Some(basis), Some(ticksize))
            }
        };

        let create_button = |content: String, msg: Option<Message>, active: bool| {
            let btn = button(container(text(content)).align_x(Horizontal::Center))
                .width(Length::Fill)
                .style(move |theme, status| style::button::transparent(theme, status, active));

            if let Some(msg) = msg {
                btn.on_press(msg)
            } else {
                btn
            }
        };

        let mut content_row = row![].align_y(Vertical::Center).spacing(16);

        let mut basis_selection_column = column![].padding(4).align_x(Horizontal::Center);

        let is_kline_chart = match kind {
            ModifierKind::Candlestick(_) | ModifierKind::Footprint(_, _) => true,
            ModifierKind::Heatmap(_, _) => false,
        };

        if selected_basis_from_kind.is_some() {
            let (timeframe_tab_is_selected, tick_count_tab_is_selected) = match self.tab {
                SelectedTab::Timeframe => (true, false),
                SelectedTab::TickCount => (false, true),
            };

            let tabs_row = row![
                create_button(
                    "Timeframe".to_string(),
                    if timeframe_tab_is_selected {
                        None
                    } else {
                        Some(Message::TabSelected(SelectedTab::Timeframe))
                    },
                    !timeframe_tab_is_selected,
                ),
                create_button(
                    "Ticks".to_string(),
                    if tick_count_tab_is_selected {
                        None
                    } else {
                        Some(Message::TabSelected(SelectedTab::TickCount))
                    },
                    !tick_count_tab_is_selected,
                ),
            ]
            .padding(padding::bottom(8))
            .spacing(4);
            basis_selection_column = basis_selection_column.push(tabs_row);
            basis_selection_column =
                basis_selection_column.push(horizontal_rule(1).style(style::split_ruler));
        }

        match self.tab {
            SelectedTab::Timeframe => {
                let current_selected_tf = match selected_basis_from_kind {
                    Some(Basis::Time(tf)) => Some(tf),
                    _ => None,
                };

                if is_kline_chart {
                    for chunk in Timeframe::KLINE.chunks(3) {
                        let mut button_row = row![].spacing(4);
                        for timeframe in chunk {
                            let is_selected = current_selected_tf == Some(*timeframe);
                            let msg = if is_selected {
                                None
                            } else {
                                Some(Message::BasisSelected((*timeframe).into(), pane))
                            };
                            button_row = button_row.push(create_button(
                                timeframe.to_string(),
                                msg,
                                !is_selected,
                            ));
                        }
                        basis_selection_column =
                            basis_selection_column.push(button_row.padding(padding::top(8)));
                    }
                } else if let Some((exchange, _)) = ticker_info {
                    let heatmap_timeframes: Vec<_> = Timeframe::HEATMAP
                        .iter()
                        .filter(|tf| !(exchange == Exchange::BybitSpot && *tf == &Timeframe::MS100))
                        .collect();

                    for chunk in heatmap_timeframes.chunks(3) {
                        let mut button_row = row![].spacing(4);
                        for timeframe_ref in chunk {
                            let timeframe = **timeframe_ref;
                            let is_selected = current_selected_tf == Some(timeframe);
                            let msg = if is_selected {
                                None
                            } else {
                                Some(Message::BasisSelected(timeframe.into(), pane))
                            };
                            button_row = button_row.push(create_button(
                                timeframe.to_string(),
                                msg,
                                !is_selected,
                            ));
                        }
                        basis_selection_column =
                            basis_selection_column.push(button_row.padding(padding::top(8)));
                    }
                }
            }
            SelectedTab::TickCount => {
                let current_selected_tick_count = match selected_basis_from_kind {
                    Some(Basis::Tick(tc)) => Some(tc),
                    _ => None,
                };

                for chunk in data::aggr::TickCount::ALL.chunks(3) {
                    let mut button_row = row![].spacing(4);
                    for tick_count in chunk {
                        let current_tick_as_u64 = u64::from(*tick_count);
                        let is_selected = current_selected_tick_count == Some(current_tick_as_u64);
                        let msg = if is_selected {
                            None
                        } else {
                            Some(Message::BasisSelected(
                                Basis::Tick(current_tick_as_u64),
                                pane,
                            ))
                        };
                        button_row = button_row.push(create_button(
                            tick_count.to_string(),
                            msg,
                            !is_selected,
                        ));
                    }
                    basis_selection_column =
                        basis_selection_column.push(button_row.padding(padding::top(8)));
                }
            }
        }

        if selected_basis_from_kind.is_some() {
            content_row = content_row.push(basis_selection_column);
        }

        let mut ticksizes_column = column![].padding(4).align_x(Horizontal::Center);

        if selected_ticksize.is_some() {
            ticksizes_column = ticksizes_column
                .push(container(text("Ticksize Mltp.")).padding(padding::bottom(8)));

            ticksizes_column = ticksizes_column.push(horizontal_rule(1).style(style::split_ruler));

            for chunk in exchange::TickMultiplier::ALL.chunks(3) {
                let mut button_row = row![].spacing(4);
                for ticksize in chunk {
                    let is_selected = selected_ticksize == Some(*ticksize);
                    let msg = if is_selected {
                        None
                    } else {
                        Some(Message::TicksizeSelected(*ticksize, pane))
                    };
                    button_row =
                        button_row.push(create_button(ticksize.to_string(), msg, !is_selected));
                }
                ticksizes_column = ticksizes_column.push(button_row.padding(padding::top(8)));
            }

            content_row = content_row.push(ticksizes_column);
        }

        container(scrollable::Scrollable::with_direction(
            content_row.align_y(Alignment::Start),
            scrollable::Direction::Vertical(
                scrollable::Scrollbar::new().width(4).scroller_width(4),
            ),
        ))
        .max_width(
            if selected_ticksize.is_some() && selected_basis_from_kind.is_some() {
                420
            } else if selected_basis_from_kind.is_some() {
                240
            } else {
                120
            },
        )
        .padding(16)
        .style(style::chart_modal)
        .into()
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
