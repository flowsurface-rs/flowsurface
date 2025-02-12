use crate::{
    data_providers::format_with_commas, 
    screen::dashboard::pane::Message, 
    style
};
use super::{heatmap, timeandsales};

use iced::{
    widget::{
        column, container, pane_grid, row, text, Slider
    }, 
    Alignment, Element, Length
};

#[derive(Debug, Clone)]
pub enum VisualConfig {
    Heatmap(heatmap::Config),
    TimeAndSales(timeandsales::Config),
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
                                pane, 
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
                                pane, 
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
                        pane, 
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
                                        pane, 
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
                                pane, 
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
    ].spacing(8))
    .width(Length::Shrink)
    .padding(16)
    .max_width(500)
    .style(style::chart_modal)
    .into()
}