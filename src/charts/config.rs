use super::{heatmap, timeandsales};
use crate::{data_providers::format_with_commas, screen::dashboard::pane::Message, style, tooltip};

use iced::{
    Alignment, Element, Length,
    widget::{Slider, Text, button, column, container, pane_grid, row, text},
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

pub fn heatmap_cfg_view<'a>(cfg: heatmap::Config, pane: pane_grid::Pane) -> Element<'a, Message> {
    let trade_size_slider = {
        let filter = cfg.trade_size_filter;

        create_slider_row(
            text("Trade size"),
            Slider::new(0.0..=50000.0, filter, move |value| {
                Message::VisualConfigChanged(
                    Some(pane),
                    VisualConfig::Heatmap(heatmap::Config {
                        trade_size_filter: value,
                        ..cfg
                    }),
                )
            })
            .step(500.0)
            .into(),
            text(format!("${}", format_with_commas(filter))).size(13),
        )
    };
    let order_size_slider = {
        let filter = cfg.order_size_filter;

        create_slider_row(
            text("Order size"),
            Slider::new(0.0..=500_000.0, filter, move |value| {
                Message::VisualConfigChanged(
                    Some(pane),
                    VisualConfig::Heatmap(heatmap::Config {
                        order_size_filter: value,
                        ..cfg
                    }),
                )
            })
            .step(1000.0)
            .into(),
            text(format!("${}", format_with_commas(filter))).size(13),
        )
    };
    let circle_scaling_slider = {
        let radius_scale = cfg.trade_size_scale;

        create_slider_row(
            text("Circle radius scaling"),
            Slider::new(10..=200, radius_scale, move |value| {
                Message::VisualConfigChanged(
                    Some(pane),
                    VisualConfig::Heatmap(heatmap::Config {
                        trade_size_scale: value,
                        ..cfg
                    }),
                )
            })
            .step(10)
            .into(),
            text(format!("{}%", cfg.trade_size_scale)).size(13),
        )
    };

    container(
        column![
            column![
                text("Size Filtering").size(14),
                trade_size_slider,
                order_size_slider,
            ]
            .spacing(20)
            .padding(16)
            .align_x(Alignment::Start),
            column![
                text("Trade visualization").size(14),
                iced::widget::checkbox("Dynamic circle radius", cfg.dynamic_sized_trades,)
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
                        circle_scaling_slider
                    } else {
                        container(row![]).into()
                    }
                },
            ]
            .spacing(20)
            .padding(16)
            .width(Length::Fill)
            .align_x(Alignment::Start),
            sync_all_button(VisualConfig::Heatmap(cfg)),
        ]
        .spacing(8),
    )
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
    let trade_size_slider = {
        let filter = cfg.trade_size_filter;

        create_slider_row(
            text("Trade size"),
            Slider::new(0.0..=50000.0, filter, move |value| {
                Message::VisualConfigChanged(
                    Some(pane),
                    VisualConfig::TimeAndSales(timeandsales::Config {
                        trade_size_filter: value,
                        ..cfg
                    }),
                )
            })
            .step(500.0)
            .into(),
            text(format!("${}", format_with_commas(filter))).size(13),
        )
    };

    container(
        column![
            column![text("Size Filtering").size(14), trade_size_slider,]
                .spacing(20)
                .padding(16)
                .align_x(Alignment::Center),
            sync_all_button(VisualConfig::TimeAndSales(cfg)),
        ]
        .spacing(8),
    )
    .width(Length::Shrink)
    .padding(16)
    .max_width(500)
    .style(style::chart_modal)
    .into()
}

fn create_slider_row<'a>(
    label: Text<'a>,
    slider: Element<'a, Message>,
    placeholder: Text<'a>,
) -> Element<'a, Message> {
    container(
        row![
            label,
            column![slider, placeholder,]
                .spacing(2)
                .align_x(Alignment::Center),
        ]
        .align_y(Alignment::Center)
        .spacing(8)
        .padding(8),
    )
    .style(style::modal_container)
    .into()
}

fn sync_all_button<'a>(config: VisualConfig) -> Element<'a, Message> {
    container(tooltip(
        button("Sync all")
            .on_press(Message::VisualConfigChanged(None, config))
            .padding(8),
        Some("Apply configuration to similar panes"),
        tooltip::Position::Top,
    ))
    .padding(16)
    .into()
}
