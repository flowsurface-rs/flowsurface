use crate::modal::pane::settings_control::color_picker_section;
use crate::screen::dashboard::pane::{self, Message};
use crate::style::{self, Icon, icon_text};
use crate::widget::{column_drag, dragger_row};

use data::chart::indicator::{
    BarAnalysisSettings, CumulativeDeltaSettings, HeatmapIndicator, HeatmapIndicatorConfig,
    Indicator, KlineIndicator, KlineIndicatorConfig, KlineVolumeSettings, OpenInterestSettings,
    UiIndicator,
};
use iced::{
    Element, Length, padding,
    widget::{button, checkbox, column, container, pane_grid, row, slider, space, text},
};

pub fn view<'a>(
    pane: pane_grid::Pane,
    state: &'a pane::State,
    market_type: Option<exchange::adapter::MarketKind>,
) -> Element<'a, Message> {
    let content_allows_dragging = matches!(state.content, pane::Content::Kline { .. });
    let content = match (&state.content, market_type) {
        (pane::Content::Kline { indicators, .. }, Some(market)) => {
            content_row_kline(pane, state, indicators, market, content_allows_dragging)
        }
        (pane::Content::Heatmap { indicators, .. }, Some(market))
        | (pane::Content::ShaderHeatmap { indicators, .. }, Some(market)) => {
            content_row_heatmap(pane, state, indicators, market, false)
        }
        _ => column![].spacing(4).into(),
    };

    container(content)
        .max_width(320)
        .padding(16)
        .style(style::chart_modal)
        .into()
}

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

fn content_row_kline<'a>(
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

fn content_row_heatmap<'a>(
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

fn kline_indicator_editor<'a>(
    pane: pane_grid::Pane,
    config: KlineIndicatorConfig,
) -> Element<'a, Message> {
    let content = match config {
        KlineIndicatorConfig::Volume(settings) => volume_editor(pane, settings),
        KlineIndicatorConfig::BarAnalysis(settings) => bar_analysis_editor(pane, settings),
        KlineIndicatorConfig::CumulativeDelta(settings) => cumulative_delta_editor(pane, settings),
        KlineIndicatorConfig::OpenInterest(settings) => open_interest_editor(pane, settings),
    };

    container(content)
        .style(style::modal_container)
        .padding(10)
        .width(Length::Fill)
        .into()
}

fn volume_editor<'a>(pane: pane_grid::Pane, settings: KlineVolumeSettings) -> Element<'a, Message> {
    let width = slider(0.2..=1.0, settings.bar_width_factor, move |value| {
        Message::PaneEvent(
            pane,
            pane::Event::ConfigureKlineIndicator(KlineIndicatorConfig::Volume(
                KlineVolumeSettings {
                    bar_width_factor: value,
                    ..settings
                },
            )),
        )
    })
    .step(0.05);

    let mut content = column![
        checkbox(settings.show_tooltip)
            .label("Show tooltip")
            .on_toggle(move |value| {
                Message::PaneEvent(
                    pane,
                    pane::Event::ConfigureKlineIndicator(KlineIndicatorConfig::Volume(
                        KlineVolumeSettings {
                            show_tooltip: value,
                            ..settings
                        },
                    )),
                )
            }),
        text(format!(
            "Bar width: {:.0}%",
            settings.bar_width_factor * 100.0
        )),
        width,
    ]
    .spacing(8);

    let custom_color = if settings.custom_color_enabled {
        let buy_color = settings
            .custom_buy_color
            .unwrap_or(settings.colors.buy_color);
        let sell_color = settings
            .custom_sell_color
            .unwrap_or(settings.colors.sell_color);

        column![
            checkbox(true)
                .label("Custom colors")
                .on_toggle(move |value| {
                    Message::PaneEvent(
                        pane,
                        pane::Event::ConfigureKlineIndicator(KlineIndicatorConfig::Volume(
                            KlineVolumeSettings {
                                custom_color_enabled: value,
                                custom_buy_color: if value { Some(buy_color) } else { None },
                                custom_sell_color: if value { Some(sell_color) } else { None },
                                ..settings
                            },
                        )),
                    )
                }),
            color_picker_section("Buy color", buy_color, move |color| {
                Message::PaneEvent(
                    pane,
                    pane::Event::ConfigureKlineIndicator(KlineIndicatorConfig::Volume(
                        KlineVolumeSettings {
                            custom_color_enabled: true,
                            custom_buy_color: Some(color),
                            custom_sell_color: settings.custom_sell_color.or(Some(sell_color)),
                            ..settings
                        },
                    )),
                )
            }),
            color_picker_section("Sell color", sell_color, move |color| {
                Message::PaneEvent(
                    pane,
                    pane::Event::ConfigureKlineIndicator(KlineIndicatorConfig::Volume(
                        KlineVolumeSettings {
                            custom_color_enabled: true,
                            custom_buy_color: settings.custom_buy_color.or(Some(buy_color)),
                            custom_sell_color: Some(color),
                            ..settings
                        },
                    )),
                )
            }),
        ]
        .spacing(8)
    } else {
        column![
            checkbox(false)
                .label("Custom colors")
                .on_toggle(move |value| {
                    Message::PaneEvent(
                        pane,
                        pane::Event::ConfigureKlineIndicator(KlineIndicatorConfig::Volume(
                            KlineVolumeSettings {
                                custom_color_enabled: value,
                                custom_buy_color: if value {
                                    Some(settings.colors.buy_color)
                                } else {
                                    None
                                },
                                custom_sell_color: if value {
                                    Some(settings.colors.sell_color)
                                } else {
                                    None
                                },
                                ..settings
                            },
                        )),
                    )
                }),
        ]
        .spacing(8)
    };

    content = content.push(custom_color);
    content.into()
}

fn bar_analysis_editor<'a>(
    pane: pane_grid::Pane,
    settings: BarAnalysisSettings,
) -> Element<'a, Message> {
    column![
        checkbox(settings.show_buy_sell)
            .label("Show bid/ask rows")
            .on_toggle(move |value| {
                Message::PaneEvent(
                    pane,
                    pane::Event::ConfigureKlineIndicator(KlineIndicatorConfig::BarAnalysis(
                        BarAnalysisSettings {
                            show_buy_sell: value,
                            ..settings
                        },
                    )),
                )
            }),
        checkbox(settings.show_volume)
            .label("Show volume row")
            .on_toggle(move |value| {
                Message::PaneEvent(
                    pane,
                    pane::Event::ConfigureKlineIndicator(KlineIndicatorConfig::BarAnalysis(
                        BarAnalysisSettings {
                            show_volume: value,
                            ..settings
                        },
                    )),
                )
            }),
        checkbox(settings.show_delta)
            .label("Show delta row")
            .on_toggle(move |value| {
                Message::PaneEvent(
                    pane,
                    pane::Event::ConfigureKlineIndicator(KlineIndicatorConfig::BarAnalysis(
                        BarAnalysisSettings {
                            show_delta: value,
                            ..settings
                        },
                    )),
                )
            }),
        checkbox(settings.show_delta_pct)
            .label("Show delta % row")
            .on_toggle(move |value| {
                Message::PaneEvent(
                    pane,
                    pane::Event::ConfigureKlineIndicator(KlineIndicatorConfig::BarAnalysis(
                        BarAnalysisSettings {
                            show_delta_pct: value,
                            ..settings
                        },
                    )),
                )
            }),
    ]
    .spacing(8)
    .into()
}

fn cumulative_delta_editor<'a>(
    pane: pane_grid::Pane,
    settings: CumulativeDeltaSettings,
) -> Element<'a, Message> {
    let line_width = slider(0.5..=4.0, settings.line_width, move |value| {
        Message::PaneEvent(
            pane,
            pane::Event::ConfigureKlineIndicator(KlineIndicatorConfig::CumulativeDelta(
                CumulativeDeltaSettings {
                    line_width: value,
                    ..settings
                },
            )),
        )
    })
    .step(0.25);

    let min_run = slider(
        1.0..=8.0,
        settings.min_directional_run as f32,
        move |value| {
            Message::PaneEvent(
                pane,
                pane::Event::ConfigureKlineIndicator(KlineIndicatorConfig::CumulativeDelta(
                    CumulativeDeltaSettings {
                        min_directional_run: value.round() as usize,
                        ..settings
                    },
                )),
            )
        },
    )
    .step(1.0);

    let mut content = column![
        checkbox(settings.show_points)
            .label("Show points")
            .on_toggle(move |value| {
                Message::PaneEvent(
                    pane,
                    pane::Event::ConfigureKlineIndicator(KlineIndicatorConfig::CumulativeDelta(
                        CumulativeDeltaSettings {
                            show_points: value,
                            ..settings
                        },
                    )),
                )
            }),
        text(format!(
            "Minimum directional run: {}",
            settings.min_directional_run
        )),
        min_run,
        text(format!("Line width: {:.2}", settings.line_width)),
        line_width
    ]
    .spacing(8);

    let custom_color = if settings.custom_color_enabled {
        let color = settings.custom_color.unwrap_or(iced::Color::WHITE);
        column![
            checkbox(true)
                .label("Custom color")
                .on_toggle(move |value| {
                    Message::PaneEvent(
                        pane,
                        pane::Event::ConfigureKlineIndicator(
                            KlineIndicatorConfig::CumulativeDelta(CumulativeDeltaSettings {
                                custom_color_enabled: value,
                                custom_color: if value { Some(color) } else { None },
                                ..settings
                            }),
                        ),
                    )
                }),
            color_picker_section("Line color", color, move |new_color| {
                Message::PaneEvent(
                    pane,
                    pane::Event::ConfigureKlineIndicator(KlineIndicatorConfig::CumulativeDelta(
                        CumulativeDeltaSettings {
                            custom_color_enabled: true,
                            custom_color: Some(new_color),
                            ..settings
                        },
                    )),
                )
            }),
        ]
        .spacing(8)
    } else {
        column![
            checkbox(false)
                .label("Custom color")
                .on_toggle(move |value| {
                    Message::PaneEvent(
                        pane,
                        pane::Event::ConfigureKlineIndicator(
                            KlineIndicatorConfig::CumulativeDelta(CumulativeDeltaSettings {
                                custom_color_enabled: value,
                                custom_color: if value {
                                    Some(iced::Color::WHITE)
                                } else {
                                    None
                                },
                                ..settings
                            }),
                        ),
                    )
                }),
        ]
        .spacing(8)
    };

    content = content.push(custom_color);
    content.into()
}

fn open_interest_editor<'a>(
    pane: pane_grid::Pane,
    settings: OpenInterestSettings,
) -> Element<'a, Message> {
    let line_width = slider(0.5..=4.0, settings.line_width, move |value| {
        Message::PaneEvent(
            pane,
            pane::Event::ConfigureKlineIndicator(KlineIndicatorConfig::OpenInterest(
                OpenInterestSettings {
                    line_width: value,
                    ..settings
                },
            )),
        )
    })
    .step(0.25);

    let mut content = column![
        checkbox(settings.show_points)
            .label("Show points")
            .on_toggle(move |value| {
                Message::PaneEvent(
                    pane,
                    pane::Event::ConfigureKlineIndicator(KlineIndicatorConfig::OpenInterest(
                        OpenInterestSettings {
                            show_points: value,
                            ..settings
                        },
                    )),
                )
            }),
        text(format!("Line width: {:.2}", settings.line_width)),
        line_width
    ]
    .spacing(8);

    let custom_color = if settings.custom_color_enabled {
        let color = settings.custom_color.unwrap_or(iced::Color::WHITE);
        column![
            checkbox(true)
                .label("Custom color")
                .on_toggle(move |value| {
                    Message::PaneEvent(
                        pane,
                        pane::Event::ConfigureKlineIndicator(KlineIndicatorConfig::OpenInterest(
                            OpenInterestSettings {
                                custom_color_enabled: value,
                                custom_color: if value { Some(color) } else { None },
                                ..settings
                            },
                        )),
                    )
                }),
            color_picker_section("Line color", color, move |new_color| {
                Message::PaneEvent(
                    pane,
                    pane::Event::ConfigureKlineIndicator(KlineIndicatorConfig::OpenInterest(
                        OpenInterestSettings {
                            custom_color_enabled: true,
                            custom_color: Some(new_color),
                            ..settings
                        },
                    )),
                )
            }),
        ]
        .spacing(8)
    } else {
        column![
            checkbox(false)
                .label("Custom color")
                .on_toggle(move |value| {
                    Message::PaneEvent(
                        pane,
                        pane::Event::ConfigureKlineIndicator(KlineIndicatorConfig::OpenInterest(
                            OpenInterestSettings {
                                custom_color_enabled: value,
                                custom_color: if value {
                                    Some(iced::Color::WHITE)
                                } else {
                                    None
                                },
                                ..settings
                            },
                        )),
                    )
                }),
        ]
        .spacing(8)
    };

    content = content.push(custom_color);
    content.into()
}
