use crate::modal::pane::indicators::editors::kline_indicator_editor;
use crate::screen::dashboard::pane::{self, Message};
use crate::style::{self, Icon, icon_text};
use crate::widget::{column_drag, dragger_row};

use data::chart::indicator::{
    HeatmapIndicator, HeatmapIndicatorConfig, Indicator, KlineIndicator, KlineIndicatorConfig,
    UiIndicator,
};
use iced::{
    Element, Length, padding,
    widget::{button, column, container, pane_grid, row, space, text},
};

fn indicator_button<'a>(
    pane: pane_grid::Pane,
    label: String,
    ui_indicator: UiIndicator,
    is_selected: bool,
    show_settings: bool,
    is_expanded: bool,
) -> Element<'a, Message> {
    let mut content = row![text(label), space::horizontal()];
    if is_selected {
        content = content.push(container(icon_text(Icon::Checkmark, 12)));
    }

    let main_button = button(content.width(Length::Fill))
        .on_press(Message::PaneEvent(
            pane,
            pane::Event::ToggleIndicator(ui_indicator),
        ))
        .width(Length::FillPortion(1))
        .style(move |theme, status| style::button::modifier(theme, status, is_selected))
        .into();

    if is_selected && show_settings {
        row![
            main_button,
            button(icon_text(Icon::Cog, 12))
                .on_press(Message::PaneEvent(
                    pane,
                    pane::Event::ToggleIndicatorSettings(ui_indicator),
                ))
                .style(move |theme, status| style::button::transparent(theme, status, is_expanded))
        ]
        .spacing(4)
        .align_y(iced::Alignment::Center)
        .into()
    } else {
        main_button
    }
}

fn selected_list_kline<'a>(
    pane: pane_grid::Pane,
    state: &'a pane::State,
    selected: &[KlineIndicatorConfig],
    reorderable: bool,
    chart_kind: data::chart::KlineChartKind,
) -> Element<'a, Message> {
    let elements: Vec<Element<_>> = selected
        .iter()
        .filter(|indicator| is_kline_allowed(chart_kind.clone(), indicator.kind()))
        .map(|indicator| {
            let ui_indicator = UiIndicator::from(indicator.kind());
            let is_expanded = state.expanded_indicator_settings == Some(ui_indicator);
            let row = indicator_button(
                pane,
                indicator.to_string(),
                ui_indicator,
                true,
                indicator.has_settings(),
                is_expanded,
            );

            let mut content = column![row].spacing(6);
            if is_expanded {
                content = content.push(kline_indicator_editor(pane, *indicator));
            }

            dragger_row(container(content).width(Length::Fill).into(), reorderable)
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

fn selected_list_heatmap<'a>(
    pane: pane_grid::Pane,
    selected: &[HeatmapIndicatorConfig],
) -> Element<'a, Message> {
    let elements: Vec<Element<_>> = selected
        .iter()
        .map(|indicator| {
            indicator_button(
                pane,
                indicator.to_string(),
                UiIndicator::from(indicator.kind()),
                true,
                indicator.has_settings(),
                false,
            )
        })
        .collect();

    iced::widget::Column::with_children(elements)
        .spacing(4)
        .into()
}

fn available_list_kline<'a>(
    pane: pane_grid::Pane,
    selected: &[KlineIndicatorConfig],
    market: exchange::adapter::MarketKind,
    chart_kind: data::chart::KlineChartKind,
) -> Element<'a, Message> {
    let elements: Vec<Element<_>> = KlineIndicator::for_market(market)
        .iter()
        .copied()
        .filter(|indicator| is_kline_allowed(chart_kind.clone(), *indicator))
        .filter(|indicator| !selected.iter().any(|cfg| cfg.kind() == *indicator))
        .map(|indicator| {
            indicator_button(
                pane,
                indicator.to_string(),
                UiIndicator::from(indicator),
                false,
                false,
                false,
            )
        })
        .collect();

    iced::widget::Column::with_children(elements)
        .spacing(4)
        .into()
}

fn available_list_heatmap<'a>(
    pane: pane_grid::Pane,
    selected: &[HeatmapIndicatorConfig],
    market: exchange::adapter::MarketKind,
) -> Element<'a, Message> {
    let elements: Vec<Element<_>> = HeatmapIndicator::for_market(market)
        .iter()
        .filter(|indicator| !selected.iter().any(|cfg| cfg.kind() == **indicator))
        .map(|indicator| {
            indicator_button(
                pane,
                indicator.to_string(),
                UiIndicator::from(*indicator),
                false,
                false,
                false,
            )
        })
        .collect();

    iced::widget::Column::with_children(elements)
        .spacing(4)
        .into()
}

pub fn content_row_kline<'a>(
    pane: pane_grid::Pane,
    state: &'a pane::State,
    selected: &[KlineIndicatorConfig],
    market: exchange::adapter::MarketKind,
    allows_drag: bool,
) -> Element<'a, Message> {
    let chart_kind = state
        .content
        .chart_kind()
        .unwrap_or(data::chart::KlineChartKind::Candles);
    let reorderable = allows_drag && selected.len() >= 2;
    let mut col = column![].spacing(4);

    if !selected.is_empty() {
        col = col.push(selected_list_kline(
            pane,
            state,
            selected,
            reorderable,
            chart_kind.clone(),
        ));
    }

    let available = available_list_kline(pane, selected, market, chart_kind);
    col = col.push(available);

    column![
        container(text("Indicators").size(crate::style::text_size::SECTION))
            .padding(padding::bottom(8)),
        col
    ]
    .spacing(4)
    .into()
}

fn is_kline_allowed(chart_kind: data::chart::KlineChartKind, indicator: KlineIndicator) -> bool {
    match chart_kind {
        data::chart::KlineChartKind::Candles => !matches!(indicator, KlineIndicator::BarAnalysis),
        data::chart::KlineChartKind::Footprint { .. } => true,
    }
}

pub fn content_row_heatmap<'a>(
    pane: pane_grid::Pane,
    _state: &'a pane::State,
    selected: &[HeatmapIndicatorConfig],
    market: exchange::adapter::MarketKind,
    _allows_drag: bool,
) -> Element<'a, Message> {
    let mut col = column![].spacing(4);

    if !selected.is_empty() {
        col = col.push(selected_list_heatmap(pane, selected));
    }

    col = col.push(available_list_heatmap(pane, selected, market));

    column![
        container(text("Indicators").size(crate::style::text_size::SECTION))
            .padding(padding::bottom(8)),
        col
    ]
    .spacing(4)
    .into()
}
