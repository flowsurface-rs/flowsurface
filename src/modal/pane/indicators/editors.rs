use crate::screen::dashboard::pane::{self, Message};
use crate::style;

use data::chart::indicator::{
    BarAnalysisSettings, CumulativeDeltaSettings, KlineIndicatorConfig, KlineVolumeSettings,
    OpenInterestSettings,
};
use iced::{
    Element,
    widget::{checkbox, column, container, pane_grid, slider, text},
};

pub fn kline_indicator_editor<'a>(
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
        .width(iced::Length::Fill)
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
            crate::modal::pane::settings_control::color_picker_section(
                "Buy color",
                buy_color,
                move |color| {
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
                },
            ),
            crate::modal::pane::settings_control::color_picker_section(
                "Sell color",
                sell_color,
                move |color| {
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
                },
            ),
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
            crate::modal::pane::settings_control::color_picker_section(
                "Line color",
                color,
                move |new_color| {
                    Message::PaneEvent(
                        pane,
                        pane::Event::ConfigureKlineIndicator(
                            KlineIndicatorConfig::CumulativeDelta(CumulativeDeltaSettings {
                                custom_color_enabled: true,
                                custom_color: Some(new_color),
                                ..settings
                            }),
                        ),
                    )
                },
            ),
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
            crate::modal::pane::settings_control::color_picker_section(
                "Line color",
                color,
                move |new_color| {
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
                },
            ),
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
