use super::{format_with_commas, study::ChartStudy};
use crate::{
    screen::dashboard::pane::Message,
    style, tooltip,
    widget::{create_slider_row, scrollable_content},
};

use data::chart::{KlineChartKind, VisualConfig, heatmap, kline::ClusterKind, timeandsales};
use iced::{
    Alignment, Element, Length,
    widget::{
        Slider, button, column, container, pane_grid, pick_list, row, text,
        tooltip::Position as TooltipPosition,
    },
};

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
            Some(text(format!("${}", format_with_commas(filter))).size(13)),
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
            Some(text(format!("${}", format_with_commas(filter))).size(13)),
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
            Some(text(format!("{}%", cfg.trade_size_scale)).size(13)),
        )
    };

    container(scrollable_content(
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
    ))
    .width(Length::Shrink)
    .padding(16)
    .max_width(500)
    .style(style::chart_modal)
    .into()
}

pub fn kline_cfg_view<'a>(
    study_config: &'a ChartStudy,
    kind: &'a KlineChartKind,
    pane: pane_grid::Pane,
) -> Element<'a, Message> {
    match kind {
        KlineChartKind::Candles => container(text(
            "This chart type doesn't have any configurations, WIP...",
        ))
        .padding(16)
        .width(Length::Shrink)
        .max_width(500)
        .style(style::chart_modal)
        .into(),
        KlineChartKind::Footprint { clusters, studies } => {
            let cluster_picklist =
                pick_list(ClusterKind::ALL, Some(clusters), move |new_cluster_kind| {
                    Message::ClusterKindSelected(pane, new_cluster_kind)
                });

            let study_cfg = study_config
                .view(studies)
                .map(move |msg| Message::StudyConfigurator(pane, msg));

            container(scrollable_content(
                column![
                    column![text("Clustering type").size(14), cluster_picklist].spacing(4),
                    column![text("Footprint studies").size(14), study_cfg].spacing(4),
                ]
                .spacing(20)
                .padding(16)
                .align_x(Alignment::Start),
            ))
            .width(Length::Shrink)
            .max_width(320)
            .padding(16)
            .style(style::chart_modal)
            .into()
        }
    }
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
                    }),
                )
            })
            .step(500.0)
            .into(),
            Some(text(format!("${}", format_with_commas(filter))).size(13)),
        )
    };

    container(scrollable_content(
        column![
            column![text("Size Filtering").size(14), trade_size_slider,]
                .spacing(20)
                .padding(16)
                .align_x(Alignment::Center),
            sync_all_button(VisualConfig::TimeAndSales(cfg)),
        ]
        .spacing(8),
    ))
    .width(Length::Shrink)
    .padding(16)
    .max_width(500)
    .style(style::chart_modal)
    .into()
}

fn sync_all_button<'a>(config: VisualConfig) -> Element<'a, Message> {
    container(tooltip(
        button("Sync all")
            .on_press(Message::VisualConfigChanged(None, config))
            .padding(8),
        Some("Apply configuration to similar panes"),
        TooltipPosition::Top,
    ))
    .padding(16)
    .into()
}
