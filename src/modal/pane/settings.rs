use crate::screen::dashboard::pane::Message;
use crate::screen::dashboard::panel::timeandsales;
use crate::widget::{classic_slider_row, labeled_slider};
use crate::{style, tooltip, widget::scrollable_content};
use data::chart::timeandsales::StackedBarRatio;
use data::chart::{
    KlineChartKind, VisualConfig,
    heatmap::{self, CoalesceKind},
    kline::ClusterKind,
};
use data::util::format_with_commas;
use iced::{
    Alignment, Element, Length,
    widget::{
        button, column, container, horizontal_rule, horizontal_space, pane_grid, pick_list, radio,
        row, slider, text, tooltip::Position as TooltipPosition,
    },
};

fn cfg_view_container<'a, T>(max_width: u32, content: T) -> Element<'a, Message>
where
    T: Into<Element<'a, Message>>,
{
    container(scrollable_content(content))
        .width(Length::Shrink)
        .padding(28)
        .max_width(max_width)
        .style(style::chart_modal)
        .into()
}

pub fn heatmap_cfg_view<'a>(cfg: heatmap::Config, pane: pane_grid::Pane) -> Element<'a, Message> {
    let trade_size_slider = {
        let filter = cfg.trade_size_filter;

        labeled_slider(
            "Trade",
            0.0..=50000.0,
            filter,
            move |value| {
                Message::VisualConfigChanged(
                    Some(pane),
                    VisualConfig::Heatmap(heatmap::Config {
                        trade_size_filter: value,
                        ..cfg
                    }),
                )
            },
            |value| format!(">${}", format_with_commas(*value)),
            Some(500.0),
        )
    };

    let order_size_slider = {
        let filter = cfg.order_size_filter;

        labeled_slider(
            "Order",
            0.0..=500_000.0,
            filter,
            move |value| {
                Message::VisualConfigChanged(
                    Some(pane),
                    VisualConfig::Heatmap(heatmap::Config {
                        order_size_filter: value,
                        ..cfg
                    }),
                )
            },
            |value| format!(">${}", format_with_commas(*value)),
            Some(5000.0),
        )
    };

    let circle_scaling_slider = {
        if let Some(radius_scale) = cfg.trade_size_scale {
            classic_slider_row(
                text("Circle radius scaling"),
                slider(10..=200, radius_scale, move |value| {
                    Message::VisualConfigChanged(
                        Some(pane),
                        VisualConfig::Heatmap(heatmap::Config {
                            trade_size_scale: Some(value),
                            ..cfg
                        }),
                    )
                })
                .step(10)
                .into(),
                Some(text(format!("{}%", radius_scale)).size(13)),
            )
        } else {
            container(row![]).into()
        }
    };

    let coalescer_cfg: Element<_> = {
        if let Some(coalescing) = cfg.coalescing {
            let threshold_pct = coalescing.threshold();

            let coalescer_kinds = {
                let average = radio(
                    "Average",
                    CoalesceKind::Average(threshold_pct),
                    Some(coalescing),
                    move |value| {
                        Message::VisualConfigChanged(
                            Some(pane),
                            VisualConfig::Heatmap(heatmap::Config {
                                coalescing: Some(value),
                                ..cfg
                            }),
                        )
                    },
                )
                .spacing(4);

                let first = radio(
                    "First",
                    CoalesceKind::First(threshold_pct),
                    Some(coalescing),
                    move |value| {
                        Message::VisualConfigChanged(
                            Some(pane),
                            VisualConfig::Heatmap(heatmap::Config {
                                coalescing: Some(value),
                                ..cfg
                            }),
                        )
                    },
                )
                .spacing(4);

                let max = radio(
                    "Max",
                    CoalesceKind::Max(threshold_pct),
                    Some(coalescing),
                    move |value| {
                        Message::VisualConfigChanged(
                            Some(pane),
                            VisualConfig::Heatmap(heatmap::Config {
                                coalescing: Some(value),
                                ..cfg
                            }),
                        )
                    },
                )
                .spacing(4);

                row![
                    text("Merge method: "),
                    row![average, first, max].spacing(12)
                ]
                .spacing(12)
            };

            let threshold_slider = classic_slider_row(
                text("Size similarity"),
                slider(0.05..=0.8, threshold_pct, move |value| {
                    Message::VisualConfigChanged(
                        Some(pane),
                        VisualConfig::Heatmap(heatmap::Config {
                            coalescing: Some(coalescing.with_threshold(value)),
                            ..cfg
                        }),
                    )
                })
                .step(0.05)
                .into(),
                Some(text(format!("{:.0}%", threshold_pct * 100.0)).size(13)),
            );

            container(column![coalescer_kinds, threshold_slider,].spacing(8))
                .style(style::modal_container)
                .padding(8)
                .into()
        } else {
            row![].into()
        }
    };

    let size_filters_column = column![
        text("Size filters").size(14),
        column![trade_size_slider, order_size_slider].spacing(8),
    ]
    .spacing(8);

    let noise_filters_column = column![
        text("Noise filters").size(14),
        iced::widget::checkbox(
            "Merge orders if sizes are similar",
            cfg.coalescing.is_some(),
        )
        .on_toggle(move |value| {
            Message::VisualConfigChanged(
                Some(pane),
                VisualConfig::Heatmap(heatmap::Config {
                    coalescing: if value {
                        Some(CoalesceKind::Average(0.15))
                    } else {
                        None
                    },
                    ..cfg
                }),
            )
        }),
        coalescer_cfg,
    ]
    .spacing(8);

    let trade_viz_column = column![
        text("Trade visualization").size(14),
        iced::widget::checkbox("Dynamic circle radius", cfg.trade_size_scale.is_some(),).on_toggle(
            move |value| {
                Message::VisualConfigChanged(
                    Some(pane),
                    VisualConfig::Heatmap(heatmap::Config {
                        trade_size_scale: if value { Some(100) } else { None },
                        ..cfg
                    }),
                )
            }
        ),
        circle_scaling_slider,
    ]
    .spacing(8);

    let content = column![
        size_filters_column,
        horizontal_rule(1).style(style::split_ruler),
        noise_filters_column,
        horizontal_rule(1).style(style::split_ruler),
        trade_viz_column,
        horizontal_rule(1).style(style::split_ruler),
        row![
            horizontal_space(),
            sync_all_button(VisualConfig::Heatmap(cfg))
        ],
    ]
    .align_x(Alignment::Start)
    .spacing(12);

    cfg_view_container(360, content)
}

pub fn timesales_cfg_view<'a>(
    cfg: timeandsales::Config,
    pane: pane_grid::Pane,
) -> Element<'a, Message> {
    let trade_size_column = {
        let slider = {
            let filter = cfg.trade_size_filter;

            labeled_slider(
                "Trade",
                0.0..=50000.0,
                filter,
                move |value| {
                    Message::VisualConfigChanged(
                        Some(pane),
                        VisualConfig::TimeAndSales(timeandsales::Config {
                            trade_size_filter: value,
                            ..cfg
                        }),
                    )
                },
                |value| format!(">${}", format_with_commas(*value)),
                Some(500.0),
            )
        };

        column![text("Size filter").size(14), slider,].spacing(8)
    };

    let storage_buffer_column = {
        let slider = {
            let buffer_size = cfg.buffer_filter as f32;

            labeled_slider(
                "Count",
                400.0..=5000.0,
                buffer_size,
                move |value| {
                    Message::VisualConfigChanged(
                        Some(pane),
                        VisualConfig::TimeAndSales(timeandsales::Config {
                            buffer_filter: value as usize,
                            ..cfg
                        }),
                    )
                },
                |value| format!("{}", *value as usize),
                Some(100.0),
            )
        };

        column![
            row![
                text("Max trades stored").size(14),
                tooltip(
                    button("i").style(style::button::info),
                    Some("Affects the stacked bar, colors and how much you can scroll down"),
                    TooltipPosition::Top,
                ),
            ]
            .align_y(Alignment::Center)
            .spacing(4),
            row![slider,]
        ]
        .spacing(4)
    };

    let stacked_bar_ratio = {
        let ratio = cfg.stacked_bar_ratio;

        let ratio_picklist = pick_list(StackedBarRatio::ALL, Some(ratio), move |new_ratio| {
            Message::VisualConfigChanged(
                Some(pane),
                VisualConfig::TimeAndSales(timeandsales::Config {
                    stacked_bar_ratio: new_ratio,
                    ..cfg
                }),
            )
        });

        column![text("Stacked bar ratio").size(14), ratio_picklist].spacing(8)
    };

    let content = column![
        trade_size_column,
        horizontal_rule(1).style(style::split_ruler),
        storage_buffer_column,
        horizontal_rule(1).style(style::split_ruler),
        stacked_bar_ratio,
        horizontal_rule(1).style(style::split_ruler),
        row![
            horizontal_space(),
            sync_all_button(VisualConfig::TimeAndSales(cfg))
        ],
    ]
    .align_x(Alignment::Start)
    .spacing(12);

    cfg_view_container(320, content)
}

pub fn kline_cfg_view<'a>(
    study_config: &'a study::ChartStudy,
    kind: &'a KlineChartKind,
    pane: pane_grid::Pane,
) -> Element<'a, Message> {
    let content = match kind {
        KlineChartKind::Candles => column![text(
            "This chart type doesn't have any configurations, WIP..."
        )],
        KlineChartKind::Footprint { clusters, studies } => {
            let cluster_picklist =
                pick_list(ClusterKind::ALL, Some(clusters), move |new_cluster_kind| {
                    Message::ClusterKindSelected(pane, new_cluster_kind)
                });

            let study_cfg = study_config
                .view(studies)
                .map(move |msg| Message::StudyConfigurator(pane, msg));

            column![
                column![text("Clustering type").size(14), cluster_picklist].spacing(8),
                column![text("Footprint studies").size(14), study_cfg].spacing(8),
            ]
            .spacing(12)
            .align_x(Alignment::Start)
        }
    };

    cfg_view_container(360, content)
}

fn sync_all_button<'a>(config: VisualConfig) -> Element<'a, Message> {
    tooltip(
        button("Sync all").on_press(Message::VisualConfigChanged(None, config)),
        Some("Apply configuration to similar panes"),
        TooltipPosition::Top,
    )
}

pub mod study {
    use crate::style::{self, Icon, icon_text};
    use data::chart::kline::FootprintStudy;
    use iced::{
        Element, padding,
        widget::{button, column, container, horizontal_rule, horizontal_space, row, slider, text},
    };

    #[derive(Debug, Clone, Copy)]
    pub enum Message {
        CardToggled(FootprintStudy),
        StudyToggled(FootprintStudy, bool),
        StudyValueChanged(FootprintStudy),
    }

    pub enum Action {
        ToggleStudy(FootprintStudy, bool),
        ConfigureStudy(FootprintStudy),
    }

    #[derive(Default)]
    pub struct ChartStudy {
        expanded_card: Option<FootprintStudy>,
    }

    impl ChartStudy {
        pub fn new() -> Self {
            Self {
                expanded_card: None,
            }
        }

        pub fn update(&mut self, message: Message) -> Option<Action> {
            match message {
                Message::CardToggled(study) => {
                    let should_collapse = self
                        .expanded_card
                        .as_ref()
                        .is_some_and(|expanded| expanded.is_same_type(&study));

                    if should_collapse {
                        self.expanded_card = None;
                    } else {
                        self.expanded_card = Some(study);
                    }
                }
                Message::StudyToggled(study, is_checked) => {
                    return Some(Action::ToggleStudy(study, is_checked));
                }
                Message::StudyValueChanged(study) => {
                    return Some(Action::ConfigureStudy(study));
                }
            }

            None
        }

        pub fn view(&self, studies: &Vec<FootprintStudy>) -> Element<Message> {
            let mut content = column![].spacing(4);

            for available_study in FootprintStudy::ALL {
                let (is_selected, study_config) = {
                    let mut is_selected = false;
                    let mut study_config = None;

                    for s in studies {
                        if s.is_same_type(&available_study) {
                            is_selected = true;
                            study_config = Some(*s);
                            break;
                        }
                    }
                    (is_selected, study_config)
                };

                content =
                    content.push(self.create_study_row(available_study, is_selected, study_config));
            }

            content.into()
        }

        fn create_study_row(
            &self,
            study: FootprintStudy,
            is_selected: bool,
            study_config: Option<FootprintStudy>,
        ) -> Element<Message> {
            let checkbox = iced::widget::checkbox(study.to_string(), is_selected)
                .on_toggle(move |checked| Message::StudyToggled(study, checked));

            let mut checkbox_row = row![checkbox, horizontal_space()]
                .height(36)
                .align_y(iced::Alignment::Center)
                .padding(4)
                .spacing(4);

            let is_expanded = self
                .expanded_card
                .as_ref()
                .is_some_and(|expanded| expanded.is_same_type(&study));

            if is_selected {
                checkbox_row = checkbox_row.push(
                    button(icon_text(Icon::Cog, 12))
                        .on_press(Message::CardToggled(study))
                        .style(move |theme, status| {
                            style::button::transparent(theme, status, is_expanded)
                        }),
                );
            }

            let mut column = column![checkbox_row].padding(padding::left(4));

            if is_expanded && study_config.is_some() {
                let config = study_config.unwrap();

                match config {
                    FootprintStudy::NPoC { lookback } => {
                        let slider_ui = slider(10.0..=400.0, lookback as f32, move |new_value| {
                            let updated_study = FootprintStudy::NPoC {
                                lookback: new_value as usize,
                            };
                            Message::StudyValueChanged(updated_study)
                        })
                        .step(10.0);

                        column = column.push(
                            column![text(format!("Lookback: {lookback} datapoints")), slider_ui]
                                .padding(8)
                                .spacing(4),
                        );
                    }
                    FootprintStudy::Imbalance {
                        threshold,
                        color_scale,
                        ignore_zeros,
                    } => {
                        let qty_threshold = {
                            let info_text = text(format!("Ask:Bid threshold: {threshold}%"));

                            let threshold_slider =
                                slider(100.0..=800.0, threshold as f32, move |new_value| {
                                    let updated_study = FootprintStudy::Imbalance {
                                        threshold: new_value as usize,
                                        color_scale,
                                        ignore_zeros,
                                    };
                                    Message::StudyValueChanged(updated_study)
                                })
                                .step(25.0);

                            column![info_text, threshold_slider,].padding(8).spacing(4)
                        };

                        let color_scaling = {
                            let color_scale_enabled = color_scale.is_some();
                            let color_scale_value = color_scale.unwrap_or(100);

                            let color_scale_checkbox = iced::widget::checkbox(
                                "Dynamic color scaling",
                                color_scale_enabled,
                            )
                            .on_toggle(move |is_enabled| {
                                let updated_study = FootprintStudy::Imbalance {
                                    threshold,
                                    color_scale: if is_enabled {
                                        Some(color_scale_value)
                                    } else {
                                        None
                                    },
                                    ignore_zeros,
                                };
                                Message::StudyValueChanged(updated_study)
                            });

                            if color_scale_enabled {
                                let scaling_slider = column![
                                    text(format!("Opaque color at: {color_scale_value}x")),
                                    slider(
                                        50.0..=2000.0,
                                        color_scale_value as f32,
                                        move |new_value| {
                                            let updated_study = FootprintStudy::Imbalance {
                                                threshold,
                                                color_scale: Some(new_value as usize),
                                                ignore_zeros,
                                            };
                                            Message::StudyValueChanged(updated_study)
                                        }
                                    )
                                    .step(50.0)
                                ]
                                .spacing(2);

                                column![color_scale_checkbox, scaling_slider]
                                    .padding(8)
                                    .spacing(8)
                            } else {
                                column![color_scale_checkbox].padding(8)
                            }
                        };

                        let ignore_zeros_checkbox = {
                            let cbox = iced::widget::checkbox("Ignore zeros", ignore_zeros)
                                .on_toggle(move |is_checked| {
                                    let updated_study = FootprintStudy::Imbalance {
                                        threshold,
                                        color_scale,
                                        ignore_zeros: is_checked,
                                    };
                                    Message::StudyValueChanged(updated_study)
                                });

                            column![cbox].padding(8).spacing(4)
                        };

                        column = column.push(
                            column![
                                qty_threshold,
                                horizontal_rule(1).style(style::split_ruler),
                                color_scaling,
                                horizontal_rule(1).style(style::split_ruler),
                                ignore_zeros_checkbox,
                            ]
                            .padding(8)
                            .spacing(4),
                        );
                    }
                }
            }

            container(column).style(style::modal_container).into()
        }
    }
}
