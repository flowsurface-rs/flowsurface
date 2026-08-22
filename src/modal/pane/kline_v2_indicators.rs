use crate::screen::dashboard::pane;
use crate::style::{self, Icon, icon_text};
use data::chart::kline_v2::KlineStudy;
use data::chart::{Basis, kline_v2::KlineIndicator};
use exchange::adapter::MarketKind;
use iced::{
    Alignment, Element, Length,
    widget::{
        button, column, container, pane_grid, row, scrollable, space, text, text_input,
        tooltip::Position,
    },
};

#[derive(Debug, Clone)]
pub enum Message {
    SearchChanged(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct State {
    search_query: String,
    search_box_id: iced::widget::Id,
    scroll_id: iced::widget::Id,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActiveIndicator {
    kind: KlineIndicator,
    count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AvailableStudy {
    study: KlineStudy,
    enabled: bool,
    active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AvailableIndicator {
    indicator: KlineIndicator,
    enabled: bool,
    active_count: usize,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    pub fn new() -> Self {
        Self {
            search_query: String::new(),
            search_box_id: iced::widget::Id::unique(),
            scroll_id: iced::widget::Id::unique(),
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::SearchChanged(query) => self.search_query = query,
        }
    }

    pub fn search_box_id(&self) -> iced::widget::Id {
        self.search_box_id.clone()
    }

    fn scroll_id(&self) -> iced::widget::Id {
        self.scroll_id.clone()
    }

    fn search_query(&self) -> &str {
        &self.search_query
    }

    fn matches_query(&self, label: &str) -> bool {
        let query = self.search_query.trim();
        query.is_empty() || label.to_lowercase().contains(&query.to_lowercase())
    }
}

pub fn view<'a>(
    pane_id: pane_grid::Pane,
    content: &pane::Content,
    picker: &'a State,
    active_indicators: &[KlineIndicator],
    market: Option<MarketKind>,
    active_studies: &[KlineStudy],
    basis: Basis,
) -> Element<'a, pane::Message> {
    let active_indicator_counts = group_active_indicators(active_indicators);
    let available_indicators = market
        .into_iter()
        .flat_map(KlineIndicator::for_market)
        .copied()
        .map(|indicator| {
            let enabled = content.allows_kline_v2_indicator(indicator);
            let active_count = active_indicator_counts
                .iter()
                .find(|active| active.kind == indicator)
                .map_or(0, |active| active.count);
            AvailableIndicator {
                indicator,
                enabled,
                active_count,
            }
        })
        .filter(|available| picker.matches_query(&available.indicator.to_string()))
        .collect();

    let available_studies = KlineStudy::ALL
        .into_iter()
        .filter_map(|study| study.normalized_for_basis(basis))
        .map(|study| {
            let enabled = content.allows_kline_v2_study(study);
            let active = active_studies
                .iter()
                .any(|active| active.is_same_type(study));
            AvailableStudy {
                study,
                enabled,
                active,
            }
        })
        .filter(|available| picker.matches_query(&available.study.to_string()))
        .collect();

    let available_column = available_column(pane_id, available_indicators, available_studies);

    let search = row![
        text_input("Search indicators and studies...", picker.search_query())
            .style(|theme, status| style::validated_text_input(theme, status, true))
            .on_input(move |query| {
                pane::Message::PaneEvent(
                    pane_id,
                    pane::Event::KlineV2IndicatorsInteraction(Message::SearchChanged(query)),
                )
            })
            .id(picker.search_box_id())
            .padding(6)
            .width(Length::Fill),
    ]
    .align_y(Alignment::Center)
    .spacing(6);

    let scrollable = scrollable::Scrollable::with_direction(
        available_column,
        scrollable::Direction::Vertical(scrollable::Scrollbar::new().width(4).scroller_width(4)),
    )
    .id(picker.scroll_id())
    .height(Length::Shrink)
    .style(style::scroll_bar);

    container(column![search, scrollable].spacing(12))
        .width(Length::Fixed(240.0))
        .height(Length::Shrink)
        .padding(12)
        .style(style::chart_modal)
        .into()
}

fn available_column(
    pane_id: pane_grid::Pane,
    indicators: Vec<AvailableIndicator>,
    studies: Vec<AvailableStudy>,
) -> Element<'static, pane::Message> {
    column![
        iced::widget::rule::horizontal(1.0).style(style::split_ruler),
        group_section(
            "Indicators",
            indicators
                .into_iter()
                .map(|available| {
                    indicator_row(
                        pane_id,
                        available.indicator,
                        available.enabled,
                        available.active_count,
                    )
                })
                .collect(),
        ),
        iced::widget::rule::horizontal(1.0).style(style::split_ruler),
        group_section(
            "Studies",
            studies
                .into_iter()
                .map(|available| {
                    study_row(
                        pane_id,
                        available.study,
                        available.enabled,
                        available.active,
                    )
                })
                .collect(),
        ),
    ]
    .spacing(8)
    .width(Length::Fill)
    .into()
}

fn group_active_indicators(indicators: &[KlineIndicator]) -> Vec<ActiveIndicator> {
    let mut grouped: Vec<ActiveIndicator> = Vec::new();

    for &kind in indicators {
        if let Some(active) = grouped.iter_mut().find(|active| active.kind == kind) {
            active.count += 1;
        } else {
            grouped.push(ActiveIndicator { kind, count: 1 });
        }
    }

    grouped
}

fn group_section(
    title: &'static str,
    rows: Vec<Element<'static, pane::Message>>,
) -> Element<'static, pane::Message> {
    column![
        text(title).size(crate::style::text_size::SECTION),
        iced::widget::Column::with_children(rows).spacing(2),
    ]
    .spacing(4)
    .into()
}

fn count_badge(count: usize) -> Option<Element<'static, pane::Message>> {
    (count > 1)
        .then(|| format!("x{count}"))
        .map(|label| text(label).size(crate::style::text_size::SMALL).into())
}

fn indicator_row(
    pane_id: pane_grid::Pane,
    indicator: KlineIndicator,
    enabled: bool,
    active_count: usize,
) -> Element<'static, pane::Message> {
    let active = active_count > 0;
    let mut content = row![text(indicator.to_string()), space::horizontal()].spacing(4);
    if let Some(badge) = count_badge(active_count) {
        content = content.push(badge);
    }
    if active {
        content = content.push(icon_text(Icon::Checkmark, 12));
    }

    let row = picker_button(
        content.align_y(Alignment::Center).width(Length::Fill),
        false,
        enabled.then_some(pane::Message::PaneEvent(
            pane_id,
            pane::Event::ToggleKlineV2Indicator(indicator),
        )),
    );

    availability_tooltip(row, enabled, "Enable trade data to use this indicator")
}

fn study_row(
    pane_id: pane_grid::Pane,
    study: KlineStudy,
    enabled: bool,
    active: bool,
) -> Element<'static, pane::Message> {
    let mut content = row![text(study.to_string()), space::horizontal()];
    if active {
        content = content.push(icon_text(Icon::Checkmark, 12));
    }

    let message = if active || !enabled {
        None
    } else {
        Some(pane::Message::PaneEvent(
            pane_id,
            pane::Event::ToggleKlineV2Study(study),
        ))
    };

    let row = picker_button(
        content.align_y(Alignment::Center).width(Length::Fill),
        active,
        message,
    );

    availability_tooltip(row, enabled, "Enable trade data to use this study")
}

fn picker_button(
    content: impl Into<Element<'static, pane::Message>>,
    selected: bool,
    message: Option<pane::Message>,
) -> button::Button<'static, pane::Message> {
    button(content)
        .width(Length::Fill)
        .style(move |theme, status| {
            let visual_status = if selected {
                button::Status::Disabled
            } else {
                status
            };
            style::button::modifier(theme, visual_status, selected)
        })
        .on_press_maybe(message)
}

fn availability_tooltip(
    row: button::Button<'static, pane::Message>,
    enabled: bool,
    message: &'static str,
) -> Element<'static, pane::Message> {
    if enabled {
        row.into()
    } else {
        crate::tooltip(row, Some(message), Position::Top)
    }
}
