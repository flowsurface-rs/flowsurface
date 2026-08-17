use crate::screen::dashboard::pane::{self, Message};
use crate::style::{self, Icon, icon_text};
use crate::widget::{column_drag, dragger_row};

use data::chart::indicator::UiIndicator;
use iced::{
    Element, Length, padding,
    widget::{button, column, container, pane_grid, row, space, text},
};
use std::fmt::Display;

pub trait IndicatorItem: PartialEq + Display + Copy + 'static {
    fn for_market(market: exchange::adapter::MarketKind) -> &'static [Self];
    fn toggle_event(self) -> pane::Event;
    fn is_allowed(content: &pane::Content, indicator: Self) -> bool;
}

impl IndicatorItem for data::chart::indicator::KlineIndicator {
    fn for_market(market: exchange::adapter::MarketKind) -> &'static [Self] {
        <Self as data::chart::indicator::Indicator>::for_market(market)
    }

    fn toggle_event(self) -> pane::Event {
        pane::Event::ToggleIndicator(UiIndicator::Kline(self))
    }

    fn is_allowed(content: &pane::Content, indicator: Self) -> bool {
        content.allows_indicator(UiIndicator::Kline(indicator))
    }
}

impl IndicatorItem for data::chart::indicator::HeatmapIndicator {
    fn for_market(market: exchange::adapter::MarketKind) -> &'static [Self] {
        <Self as data::chart::indicator::Indicator>::for_market(market)
    }

    fn toggle_event(self) -> pane::Event {
        pane::Event::ToggleIndicator(UiIndicator::Heatmap(self))
    }

    fn is_allowed(content: &pane::Content, indicator: Self) -> bool {
        content.allows_indicator(UiIndicator::Heatmap(indicator))
    }
}

impl IndicatorItem for crate::chart::kline_v2::KlineIndicator {
    fn for_market(market: exchange::adapter::MarketKind) -> &'static [Self] {
        <Self as crate::chart::kline_v2::Indicator>::for_market(market)
    }

    fn toggle_event(self) -> pane::Event {
        pane::Event::ToggleKlineV2Indicator(self)
    }

    fn is_allowed(content: &pane::Content, indicator: Self) -> bool {
        content.allows_kline_v2_indicator(indicator)
    }
}

pub fn view<'a, I>(
    pane: pane_grid::Pane,
    state: &'a pane::State,
    selected: &[I],
    market_type: Option<exchange::adapter::MarketKind>,
) -> Element<'a, Message>
where
    I: IndicatorItem,
{
    let content_allows_dragging = matches!(state.content, pane::Content::Kline { .. });
    let content_row = if let Some(market) = market_type {
        content_row(
            pane,
            &state.content,
            selected,
            market,
            content_allows_dragging,
        )
    } else {
        column![].spacing(4).into()
    };

    container(content_row)
        .max_width(200)
        .padding(16)
        .style(style::chart_modal)
        .into()
}

fn build_indicator_row<'a, I>(
    pane: pane_grid::Pane,
    indicator: &I,
    is_selected: bool,
) -> Element<'a, Message>
where
    I: IndicatorItem,
{
    let content = if is_selected {
        row![
            text(indicator.to_string()),
            space::horizontal(),
            container(icon_text(Icon::Checkmark, 12)),
        ]
        .width(Length::Fill)
    } else {
        row![text(indicator.to_string())].width(Length::Fill)
    };

    button(content)
        .on_press(Message::PaneEvent(pane, indicator.toggle_event()))
        .width(Length::Fill)
        .style(move |theme, status| style::button::modifier(theme, status, is_selected))
        .into()
}

fn selected_list<'a, I>(
    pane: pane_grid::Pane,
    selected: &[I],
    reorderable: bool,
) -> Element<'a, Message>
where
    I: IndicatorItem,
{
    let elements: Vec<Element<_>> = selected
        .iter()
        .map(|indicator| {
            let base = build_indicator_row(pane, indicator, true);
            dragger_row(base, reorderable)
        })
        .collect();

    if reorderable {
        let mut draggable_column = column_drag::Column::new()
            .on_drag(move |event| Message::PaneEvent(pane, pane::Event::ReorderIndicator(event)))
            .spacing(4);
        for element in elements {
            draggable_column = draggable_column.push(element);
        }
        draggable_column.into()
    } else {
        iced::widget::Column::with_children(elements)
            .spacing(4)
            .into()
    }
}

fn available_list<'a, I>(pane: pane_grid::Pane, available: &[I]) -> Element<'a, Message>
where
    I: IndicatorItem,
{
    let elements: Vec<Element<_>> = available
        .iter()
        .map(|indicator| {
            let base = build_indicator_row(pane, indicator, false);
            dragger_row(base, false)
        })
        .collect();

    iced::widget::Column::with_children(elements)
        .spacing(4)
        .into()
}

fn content_row<'a, I>(
    pane: pane_grid::Pane,
    content: &pane::Content,
    selected: &[I],
    market: exchange::adapter::MarketKind,
    allows_drag: bool,
) -> Element<'a, Message>
where
    I: IndicatorItem,
{
    let reorderable = allows_drag && selected.len() >= 2;

    let selected: Vec<I> = selected
        .iter()
        .copied()
        .filter(|indicator| I::is_allowed(content, *indicator))
        .collect();

    let selected_list = if !selected.is_empty() {
        Some(selected_list(pane, &selected, reorderable))
    } else {
        None
    };

    let available: Vec<I> = I::for_market(market)
        .iter()
        .filter(|indicator| !selected.contains(indicator) && I::is_allowed(content, **indicator))
        .cloned()
        .collect();
    let available_list = if !available.is_empty() {
        Some(available_list(pane, &available))
    } else {
        None
    };

    let mut col = iced::widget::Column::new();
    if let Some(sel) = selected_list {
        col = col.push(sel);
    }
    if let Some(avail) = available_list {
        col = col.push(avail);
    }

    column![
        container(text("Indicators").size(crate::style::text_size::SECTION))
            .padding(padding::bottom(8)),
        col.spacing(4)
    ]
    .spacing(4)
    .into()
}
