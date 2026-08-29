use crate::screen::dashboard::pane;
use crate::style::{self, Icon, icon_text};
use crate::widget::inspector::{InspectorContent, InspectorSection};
use data::chart::kline::{IndicatorUnsupportedReason, KlineStudy};
use data::chart::{Basis, kline::KlineIndicator};
use exchange::adapter::MarketKind;
use iced::{
    Alignment, Element, Length,
    widget::{
        button, column, container, pane_grid, row, scrollable, space, text_input, tooltip::Position,
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
    available: bool,
    active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AvailableIndicator {
    indicator: KlineIndicator,
    unavailable_reason: Option<IndicatorUnsupportedReason>,
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
    let available_indicators: Vec<_> = market
        .into_iter()
        .flat_map(KlineIndicator::for_market)
        .copied()
        .map(|indicator| {
            let unavailable_reason = content.kline_indicator_unavailable_reason(indicator);
            let active_count = active_indicator_counts
                .iter()
                .find(|active| active.kind == indicator)
                .map_or(0, |active| active.count);
            AvailableIndicator {
                indicator,
                unavailable_reason,
                active_count,
            }
        })
        .filter(|available| picker.matches_query(&available.indicator.to_string()))
        .collect();

    let available_studies = KlineStudy::ALL
        .into_iter()
        .filter_map(|study| study.normalized_for_basis(basis))
        .map(|study| {
            let available = content.allows_kline_study(study);
            let active = active_studies
                .iter()
                .any(|active| active.is_same_type(study));
            AvailableStudy {
                study,
                available,
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
                    pane::Event::KlineIndicatorsInteraction(Message::SearchChanged(query)),
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
    .height(Length::Fill)
    .style(style::scroll_bar);

    container(column![search, scrollable].spacing(12))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn available_column(
    pane_id: pane_grid::Pane,
    indicators: Vec<AvailableIndicator>,
    studies: Vec<AvailableStudy>,
) -> Element<'static, pane::Message> {
    let indicator_rows = indicators
        .into_iter()
        .map(|available| {
            indicator_row(
                pane_id,
                available.indicator,
                available.unavailable_reason,
                available.active_count,
            )
        })
        .collect::<Vec<_>>();
    let study_rows = studies
        .into_iter()
        .map(|available| {
            study_row(
                pane_id,
                available.study,
                available.available,
                available.active,
            )
        })
        .collect::<Vec<_>>();

    let mut sections = Vec::with_capacity(2);
    if !indicator_rows.is_empty() {
        sections.push(InspectorSection::titled(
            "Indicators",
            iced::widget::Column::with_children(indicator_rows)
                .spacing(2)
                .width(Length::Fill),
        ));
    }
    if !study_rows.is_empty() {
        sections.push(InspectorSection::titled(
            "Studies",
            iced::widget::Column::with_children(study_rows)
                .spacing(2)
                .width(Length::Fill),
        ));
    }

    InspectorContent::new(sections).into_element()
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

fn count_badge(count: usize) -> Option<Element<'static, pane::Message>> {
    (count > 0)
        .then(|| format!("x{count}"))
        .map(|label| crate::widget::text::muted(label).into())
}

fn indicator_row(
    pane_id: pane_grid::Pane,
    indicator: KlineIndicator,
    unavailable_reason: Option<IndicatorUnsupportedReason>,
    active_count: usize,
) -> Element<'static, pane::Message> {
    let mut content = row![
        iced::widget::text(indicator.to_string()).size(crate::style::text_size::BODY),
        space::horizontal()
    ]
    .spacing(4);

    if let Some(badge) = count_badge(active_count) {
        content = content.push(badge);
    }

    let active = active_count > 0;
    let toggle = unavailable_reason
        .is_none()
        .then_some(pane::Message::PaneEvent(
            pane_id,
            pane::Event::ToggleKlineIndicator(indicator),
        ));

    let row = picker_button(
        content.align_y(Alignment::Center).width(Length::Fill),
        toggle,
        active,
    );

    availability_tooltip(
        row,
        unavailable_reason.map(|reason| reason.indicator_unavailable_tooltip()),
    )
}

fn study_row(
    pane_id: pane_grid::Pane,
    study: KlineStudy,
    available: bool,
    active: bool,
) -> Element<'static, pane::Message> {
    let mut content = row![
        iced::widget::text(study.to_string()).size(crate::style::text_size::BODY),
        space::horizontal()
    ];

    if active {
        content = content.push(icon_text(Icon::Checkmark, 12));
    }

    let message = if active || !available {
        None
    } else {
        Some(pane::Message::PaneEvent(
            pane_id,
            pane::Event::ToggleKlineStudy(study),
        ))
    };

    let row = picker_button(
        content.align_y(Alignment::Center).width(Length::Fill),
        message,
        active,
    );

    availability_tooltip(
        row,
        (!available).then_some("Enable trade data to use this study"),
    )
}

fn picker_button(
    content: impl Into<Element<'static, pane::Message>>,
    message: Option<pane::Message>,
    active: bool,
) -> button::Button<'static, pane::Message> {
    button(content)
        .width(Length::Fill)
        .style(move |theme, status| style::button::indicator_row(theme, status, active))
        .on_press_maybe(message)
}

fn availability_tooltip(
    row: button::Button<'static, pane::Message>,
    message: Option<&'static str>,
) -> Element<'static, pane::Message> {
    match message {
        Some(message) => crate::tooltip(row, Some(message), Position::Top),
        None => row.into(),
    }
}
