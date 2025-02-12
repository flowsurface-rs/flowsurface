use crate::{
    data_providers::format_with_commas, 
    screen::dashboard::pane::Message, 
    style, tooltip
};
use super::{heatmap, timeandsales};

use iced::{
    widget::{
        button, column, container, pane_grid, row, text, Slider
    }, 
    Alignment, Element, Length
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub enum VisualConfig {
    Heatmap(heatmap::Config),
    TimeAndSales(timeandsales::Config),
}

impl VisualConfig {
    pub fn heatmap(&self) -> Option<heatmap::Config> {
        match self {
            Self::Heatmap(cfg) => Some(*cfg),
            _ => None,
        }
    }

    pub fn time_and_sales(&self) -> Option<timeandsales::Config> {
        match self {
            Self::TimeAndSales(cfg) => Some(*cfg),
            _ => None,
        }
    }
}

pub fn heatmap_cfg_view<'a>(
    cfg: heatmap::Config,
    pane: pane_grid::Pane,
) -> Element<'a, Message> {
    let trade_filter = cfg.trade_size_filter;
    let order_filter = cfg.order_size_filter;

    container(column![
        column![
            text("Size Filtering").size(14),
            container(
                row![
                    text("Trade size"),
                    column![
                        Slider::new(0.0..=50000.0, trade_filter, move |value| {
                            Message::VisualConfigChanged(
                                Some(pane), 
                                VisualConfig::Heatmap(heatmap::Config {
                                    trade_size_filter: value,
                                    ..cfg
                                }),
                            )
                        })
                        .step(500.0),
                        text(format!("${}", format_with_commas(trade_filter))).size(13),
                    ]
                    .spacing(2)
                    .align_x(Alignment::Center),
                ]
                .align_y(Alignment::Center)
                .spacing(8)
                .padding(8),
            )
            .style(style::modal_container),
            container(
                row![
                    text("Order size"),
                    column![
                        Slider::new(0.0..=500_000.0, order_filter, move |value| {
                            Message::VisualConfigChanged(
                                Some(pane), 
                                VisualConfig::Heatmap(heatmap::Config {
                                    order_size_filter: value,
                                    ..cfg
                                }),
                            )
                        })
                        .step(1000.0),
                        text(format!("${}", format_with_commas(order_filter))).size(13),
                    ]
                    .spacing(2)
                    .align_x(Alignment::Center),
                ]
                .align_y(Alignment::Center)
                .spacing(8)
                .padding(8),
            )
            .style(style::modal_container),
        ]
        .spacing(20)
        .padding(16)
        .align_x(Alignment::Center),
        column![
            column![
                text("Trade visualization").size(14),
                iced::widget::checkbox(
                    "Dynamic circle sizing",
                    cfg.dynamic_sized_trades,
                )
                .on_toggle(move |value| {
                    Message::VisualConfigChanged(
                        Some(pane), 
                        VisualConfig::Heatmap(heatmap::Config {
                            dynamic_sized_trades: value,
                            ..cfg
                        }),
                    )
                }),
                {
                    if cfg.dynamic_sized_trades {
                        column![
                            text("Circle size scaling"),
                            column![
                                Slider::new(10..=200, cfg.trade_size_scale, move |value| {
                                    Message::VisualConfigChanged(
                                        Some(pane),
                                        VisualConfig::Heatmap(heatmap::Config {
                                            trade_size_scale: value,
                                            ..cfg
                                        }),
                                    )
                                })
                                .step(10),
                                text(format!("{}%", cfg.trade_size_scale)).size(13),
                            ]
                            .spacing(2)
                            .align_x(Alignment::Center),
                        ]
                    } else {
                        column![]
                    }
                }
            ]
            .spacing(8)
            .align_x(Alignment::Center)
        ]
        .spacing(20)
        .padding(16)
        .width(Length::Fill)
        .align_x(Alignment::Center),
        container(
            tooltip(
                button("Sync all")
                    .on_press(Message::VisualConfigChanged(
                        None,
                        VisualConfig::Heatmap(cfg),
                    ))
                    .padding(8),
                Some("Apply current config to similar panes"),
                tooltip::Position::Top,
            ),
        )
        .padding(16),
    ].spacing(8))
    .width(Length::Shrink)
    .padding(16)
    .max_width(500)
    .style(style::chart_modal)
    .into()
}

pub fn timesales_cfg_view<'a>(
    cfg: timeandsales::Config,
    pane: pane_grid::Pane,
) -> Element<'a, Message> {
    let trade_filter = cfg.trade_size_filter;

    container(column![
        column![
            text("Size Filtering").size(14),
            container(
                row![
                    text("Trade size"),
                    column![
                        Slider::new(0.0..=50000.0, trade_filter, move |value| {
                            Message::VisualConfigChanged(
                                Some(pane), 
                                VisualConfig::TimeAndSales(timeandsales::Config {
                                    trade_size_filter: value,
                                    ..cfg
                                }),
                            )
                        })
                        .step(500.0),
                        text(format!("${}", format_with_commas(trade_filter))).size(13),
                    ]
                    .spacing(2)
                    .align_x(Alignment::Center),
                ]
                .align_y(Alignment::Center)
                .spacing(8)
                .padding(8),
            )
            .style(style::modal_container),
        ]
        .spacing(20)
        .padding(16)
        .align_x(Alignment::Center),
        container(
            tooltip(
                button("Sync all")
                    .on_press(Message::VisualConfigChanged(
                        None,
                        VisualConfig::TimeAndSales(cfg),
                    ))
                    .padding(8),
                Some("Apply current config to similar panes"),
                tooltip::Position::Top,
            )
        )
        .padding(16),
    ].spacing(8))
    .width(Length::Shrink)
    .padding(16)
    .max_width(500)
    .style(style::chart_modal)
    .into()
}