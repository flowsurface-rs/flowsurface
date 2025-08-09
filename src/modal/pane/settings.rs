use crate::screen::dashboard::pane::Message;
use crate::screen::dashboard::panel::timeandsales;
use crate::split_column;
use crate::widget::{classic_slider_row, labeled_slider};
use crate::{style, tooltip, widget::scrollable_content};
use data::chart::heatmap::HeatmapStudy;
use data::chart::kline::FootprintStudy;
use data::chart::{
    KlineChartKind, VisualConfig,
    heatmap::{self, CoalesceKind},
    kline::ClusterKind,
    timeandsales::StackedBarRatio,
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

pub fn heatmap_cfg_view<'a>(
    cfg: heatmap::Config,
    pane: pane_grid::Pane,
    study_config: &'a study::Configurator<HeatmapStudy>,
    studies: &'a [HeatmapStudy],
    basis: data::chart::Basis,
) -> Element<'a, Message> {
    let trade_size_slider = {
        let filter = cfg.trade_size_filter;

        labeled_slider(
            "Trade",
            0.0..=50000.0,
            filter,
            move |value| {
                Message::VisualConfigChanged(
                    pane,
                    VisualConfig::Heatmap(heatmap::Config {
                        trade_size_filter: value,
                        ..cfg
                    }),
                    false,
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
                    pane,
                    VisualConfig::Heatmap(heatmap::Config {
                        order_size_filter: value,
                        ..cfg
                    }),
                    false,
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
                        pane,
                        VisualConfig::Heatmap(heatmap::Config {
                            trade_size_scale: Some(value),
                            ..cfg
                        }),
                        false,
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
                            pane,
                            VisualConfig::Heatmap(heatmap::Config {
                                coalescing: Some(value),
                                ..cfg
                            }),
                            false,
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
                            pane,
                            VisualConfig::Heatmap(heatmap::Config {
                                coalescing: Some(value),
                                ..cfg
                            }),
                            false,
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
                            pane,
                            VisualConfig::Heatmap(heatmap::Config {
                                coalescing: Some(value),
                                ..cfg
                            }),
                            false,
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
                        pane,
                        VisualConfig::Heatmap(heatmap::Config {
                            coalescing: Some(coalescing.with_threshold(value)),
                            ..cfg
                        }),
                        false,
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
                pane,
                VisualConfig::Heatmap(heatmap::Config {
                    coalescing: if value {
                        Some(CoalesceKind::Average(0.15))
                    } else {
                        None
                    },
                    ..cfg
                }),
                false,
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
                    pane,
                    VisualConfig::Heatmap(heatmap::Config {
                        trade_size_scale: if value { Some(100) } else { None },
                        ..cfg
                    }),
                    false,
                )
            }
        ),
        circle_scaling_slider,
    ]
    .spacing(8);

    let study_cfg = study_config
        .view(studies, basis)
        .map(move |msg| Message::StudyConfigurator(pane, study::StudyMessage::Heatmap(msg)));

    let content = split_column![
        size_filters_column,
        noise_filters_column,
        trade_viz_column,
        column![text("Studies").size(14), study_cfg].spacing(8),
        row![
            horizontal_space(),
            sync_all_button(pane, VisualConfig::Heatmap(cfg))
        ]
        ; spacing = 12, align_x = Alignment::Start
    ];

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
                        pane,
                        VisualConfig::TimeAndSales(timeandsales::Config {
                            trade_size_filter: value,
                            ..cfg
                        }),
                        false,
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
                        pane,
                        VisualConfig::TimeAndSales(timeandsales::Config {
                            buffer_filter: value as usize,
                            ..cfg
                        }),
                        false,
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
                pane,
                VisualConfig::TimeAndSales(timeandsales::Config {
                    stacked_bar_ratio: new_ratio,
                    ..cfg
                }),
                false,
            )
        });

        column![text("Stacked bar ratio").size(14), ratio_picklist].spacing(8)
    };

    let content = split_column![
        trade_size_column,
        storage_buffer_column,
        stacked_bar_ratio,
        row![
            horizontal_space(),
            sync_all_button(pane, VisualConfig::TimeAndSales(cfg))
        ],
        ; spacing = 12, align_x = Alignment::Start
    ];

    cfg_view_container(320, content)
}

pub fn kline_cfg_view<'a>(
    study_config: &'a study::Configurator<FootprintStudy>,
    cfg: data::chart::kline::Config,
    kind: &'a KlineChartKind,
    pane: pane_grid::Pane,
    basis: data::chart::Basis,
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

            let study_cfg = study_config.view(studies, basis).map(move |msg| {
                Message::StudyConfigurator(pane, study::StudyMessage::Footprint(msg))
            });

            split_column![
                column![text("Cluster type").size(14), cluster_picklist].spacing(8),
                column![text("Studies").size(14), study_cfg].spacing(8),
                row![
                    horizontal_space(),
                    sync_all_button(pane, VisualConfig::Kline(cfg))
                ],
                ; spacing = 12, align_x = Alignment::Start
            ]
        }
    };

    cfg_view_container(360, content)
}

fn sync_all_button<'a>(pane: pane_grid::Pane, config: VisualConfig) -> Element<'a, Message> {
    tooltip(
        button("Sync all").on_press(Message::VisualConfigChanged(pane, config, true)),
        Some("Apply configuration to similar panes"),
        TooltipPosition::Top,
    )
}

pub mod study {
    use crate::{
        split_column,
        style::{self, Icon, icon_text},
    };
    use data::chart::heatmap::{CLEANUP_THRESHOLD, HeatmapStudy, ProfileKind};
    use data::chart::kline::FootprintStudy;
    use iced::{
        Element, padding,
        widget::{button, column, container, horizontal_rule, horizontal_space, row, slider, text},
    };

    #[derive(Debug, Clone, Copy)]
    pub enum StudyMessage {
        Footprint(Message<FootprintStudy>),
        Heatmap(Message<HeatmapStudy>),
    }

    pub trait Study: Sized + Copy + 'static {
        fn is_same_type(&self, other: &Self) -> bool;
        fn all() -> Vec<Self>;
        fn view_config<'a>(
            &self,
            basis: data::chart::Basis,
            on_change: impl Fn(Self) -> Message<Self> + Copy + 'a,
        ) -> Element<'a, Message<Self>>;
    }

    impl Study for FootprintStudy {
        fn is_same_type(&self, other: &Self) -> bool {
            std::mem::discriminant(self) == std::mem::discriminant(other)
        }

        fn all() -> Vec<Self> {
            FootprintStudy::ALL.to_vec()
        }

        fn view_config<'a>(
            &self,
            _basis: data::chart::Basis,
            on_change: impl Fn(Self) -> Message<Self> + Copy + 'a,
        ) -> Element<'a, Message<Self>> {
            match *self {
                FootprintStudy::NPoC { lookback } => {
                    let slider_ui = slider(10.0..=400.0, lookback as f32, move |new_value| {
                        on_change(FootprintStudy::NPoC {
                            lookback: new_value as usize,
                        })
                    })
                    .step(10.0);

                    column![text(format!("Lookback: {lookback} datapoints")), slider_ui]
                        .padding(8)
                        .spacing(4)
                        .into()
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
                                on_change(FootprintStudy::Imbalance {
                                    threshold: new_value as usize,
                                    color_scale,
                                    ignore_zeros,
                                })
                            })
                            .step(25.0);

                        column![info_text, threshold_slider,].padding(8).spacing(4)
                    };

                    let color_scaling = {
                        let color_scale_enabled = color_scale.is_some();
                        let color_scale_value = color_scale.unwrap_or(100);

                        let color_scale_checkbox =
                            iced::widget::checkbox("Dynamic color scaling", color_scale_enabled)
                                .on_toggle(move |is_enabled| {
                                    on_change(FootprintStudy::Imbalance {
                                        threshold,
                                        color_scale: if is_enabled {
                                            Some(color_scale_value)
                                        } else {
                                            None
                                        },
                                        ignore_zeros,
                                    })
                                });

                        if color_scale_enabled {
                            let scaling_slider = column![
                                text(format!("Opaque color at: {color_scale_value}x")),
                                slider(50.0..=2000.0, color_scale_value as f32, move |new_value| {
                                    on_change(FootprintStudy::Imbalance {
                                        threshold,
                                        color_scale: Some(new_value as usize),
                                        ignore_zeros,
                                    })
                                })
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
                        let cbox = iced::widget::checkbox("Ignore zeros", ignore_zeros).on_toggle(
                            move |is_checked| {
                                on_change(FootprintStudy::Imbalance {
                                    threshold,
                                    color_scale,
                                    ignore_zeros: is_checked,
                                })
                            },
                        );

                        column![cbox].padding(8).spacing(4)
                    };

                    split_column![qty_threshold, color_scaling, ignore_zeros_checkbox]
                        .padding(4)
                        .into()
                }
            }
        }
    }

    impl Study for HeatmapStudy {
        fn is_same_type(&self, other: &Self) -> bool {
            std::mem::discriminant(self) == std::mem::discriminant(other)
        }

        fn all() -> Vec<Self> {
            HeatmapStudy::ALL.to_vec()
        }

        fn view_config<'a>(
            &self,
            basis: data::chart::Basis,
            on_change: impl Fn(Self) -> Message<Self> + Copy + 'a,
        ) -> Element<'a, Message<Self>> {
            let interval_ms = match basis {
                data::chart::Basis::Time(interval) => interval.to_milliseconds(),
                data::chart::Basis::Tick(_) => {
                    return iced::widget::center(text(
                        "Heatmap studies are not supported for tick-based charts",
                    ))
                    .into();
                }
            };

            match self {
                HeatmapStudy::VolumeProfile(kind) => match kind {
                    ProfileKind::FixedWindow(datapoint_count) => {
                        let duration_secs = (*datapoint_count as u64 * interval_ms) / 1000;
                        let min_range = CLEANUP_THRESHOLD / 20;

                        let duration_text = if duration_secs < 60 {
                            format!("{} seconds", duration_secs)
                        } else {
                            let minutes = duration_secs / 60;
                            let seconds = duration_secs % 60;
                            if seconds == 0 {
                                format!("{} minutes", minutes)
                            } else {
                                format!("{}m {}s", minutes, seconds)
                            }
                        };

                        let slider = slider(
                            min_range as f32..=CLEANUP_THRESHOLD as f32,
                            *datapoint_count as f32,
                            move |new_datapoint_count| {
                                on_change(HeatmapStudy::VolumeProfile(ProfileKind::FixedWindow(
                                    new_datapoint_count as usize,
                                )))
                            },
                        )
                        .step(40.0);

                        let switch_kind = button(text("Switch to visible range")).on_press(
                            on_change(HeatmapStudy::VolumeProfile(ProfileKind::VisibleRange)),
                        );

                        column![
                            row![horizontal_space(), switch_kind,],
                            text(format!(
                                "Window: {} datapoints ({})",
                                datapoint_count, duration_text
                            )),
                            slider,
                        ]
                        .padding(8)
                        .spacing(4)
                        .into()
                    }
                    ProfileKind::VisibleRange => {
                        let switch_kind = button(text("Switch to fixed window")).on_press(
                            on_change(HeatmapStudy::VolumeProfile(ProfileKind::FixedWindow(
                                CLEANUP_THRESHOLD / 5_usize,
                            ))),
                        );

                        column![row![horizontal_space(), switch_kind,],]
                            .padding(8)
                            .spacing(4)
                            .into()
                    }
                },
            }
        }
    }

    #[derive(Debug, Clone, Copy)]
    pub enum Message<S: Study> {
        CardToggled(S),
        StudyToggled(S, bool),
        StudyValueChanged(S),
    }

    pub enum Action<S: Study> {
        ToggleStudy(S, bool),
        ConfigureStudy(S),
    }

    pub struct Configurator<S: Study> {
        expanded_card: Option<S>,
    }

    impl<S: Study> Default for Configurator<S> {
        fn default() -> Self {
            Self {
                expanded_card: None,
            }
        }
    }

    impl<S: Study + ToString> Configurator<S> {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn update(&mut self, message: Message<S>) -> Option<Action<S>> {
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

        pub fn view<'a>(
            &self,
            active_studies: &'a [S],
            basis: data::chart::Basis,
        ) -> Element<'a, Message<S>> {
            let mut content = column![].spacing(4);

            for available_study in S::all() {
                content =
                    content.push(self.create_study_row(available_study, active_studies, basis));
            }

            content.into()
        }

        fn create_study_row<'a>(
            &self,
            study: S,
            active_studies: &'a [S],
            basis: data::chart::Basis,
        ) -> Element<'a, Message<S>> {
            let (is_selected, study_config) = {
                let mut is_selected = false;
                let mut study_config = None;

                for s in active_studies {
                    if s.is_same_type(&study) {
                        is_selected = true;
                        study_config = Some(*s);
                        break;
                    }
                }
                (is_selected, study_config)
            };

            let label = study_config.map_or(study.to_string(), |s| s.to_string());
            let checkbox = iced::widget::checkbox(label, is_selected)
                .on_toggle(move |checked| Message::StudyToggled(study, checked));

            let mut checkbox_row = row![checkbox, horizontal_space()]
                .height(36)
                .align_y(iced::Alignment::Center)
                .padding(padding::left(8).right(4))
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

            let mut column = column![checkbox_row];

            if is_expanded && let Some(config) = study_config {
                column = column.push(config.view_config(basis, Message::StudyValueChanged));
            }

            container(column).style(style::modal_container).into()
        }
    }
}
