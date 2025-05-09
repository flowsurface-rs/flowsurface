pub mod timeandsales;

use crate::{
    screen::dashboard::pane::Message,
    style, tooltip,
    widget::{create_slider_row, scrollable_content},
};

use data::chart::{KlineChartKind, VisualConfig, heatmap, kline::ClusterKind};
use data::util::format_with_commas;
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

pub fn kline_cfg_view<'a>(
    study_config: &'a study::ChartStudy,
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
                                horizontal_rule(1),
                                color_scaling,
                                horizontal_rule(1),
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
