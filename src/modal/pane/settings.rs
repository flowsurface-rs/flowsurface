use crate::screen::dashboard::pane::{Event, Message};
use crate::screen::dashboard::panel::timeandsales;
use crate::split_column;
use crate::widget::{classic_slider_row, labeled_slider, theme_color_control};
use crate::{style, tooltip, widget::scrollable_content};

use data::chart::heatmap::HeatmapStudy;
use data::chart::kline::FootprintStudy;
use data::chart::kline_v2::{
    AnchoredVwapConfig, BollingerBandsCalculationConfig, CandlestickColorSlot, CandlestickConfig,
    IndicatorColorConfig, IndicatorColorSlot, IndicatorConfig, KlineIndicator as KlineV2Indicator,
    KlineStudy, MAX_CANDLESTICK_OPACITY_PERCENT, MIN_CANDLESTICK_OPACITY_PERCENT, MarkKind,
    MovingAverageMethod, MovingAveragesCalculationConfig, RsiConfig, RsiSmoothing, ThemeAccent,
    VwapAnchor,
};
use data::chart::{
    KlineChartKind,
    heatmap::{self, CoalesceKind},
    kline::ClusterKind,
};
use data::config::theme::{ThemeColor, ThemeColorShade};
use data::layout::pane::VisualConfig;
use data::panel::ladder;
use data::panel::timeandsales::{StackedBar, StackedBarRatio};
use data::util::format_with_commas;

use crate::widget::series_editor::{SeriesStyleEntry, TickerSeriesEditor};

#[derive(Debug, Clone, Copy)]
pub enum KlineV2IndicatorSettingsMessage {
    DataConfig(IndicatorConfig),
    Mark(data::chart::kline_v2::MarkKind),
    Color {
        slot: IndicatorColorSlot,
        accent: Option<ThemeAccent>,
        shade: ThemeColorShade,
    },
    Reset,
}

#[derive(Debug, Clone, Copy)]
pub enum CandlestickSettingsMessage {
    Config(CandlestickConfig),
    Mark(MarkKind),
    Reset,
}

use iced::widget::{checkbox, space};
use iced::{
    Alignment, Element, Length,
    widget::{
        button, column, container, pane_grid, pick_list, radio, row, slider, text,
        tooltip::Position as TooltipPosition,
    },
};
use std::time::Duration;

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
            |value| format!(">${}", format_with_commas(*value as f64)),
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
            |value| format!(">${}", format_with_commas(*value as f64)),
            Some(5000.0),
        )
    };

    let circle_scaling_slider = cfg.trade_size_scale.map(|radius_scale| {
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
            Some(text(format!("{}%", radius_scale)).size(crate::style::text_size::EMPHASIS)),
        )
    });

    let coalescer_cfg: Option<Element<_>> = if let Some(coalescing) = cfg.coalescing {
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
            Some(
                text(format!("{:.0}%", threshold_pct * 100.0))
                    .size(crate::style::text_size::EMPHASIS),
            ),
        );

        Some(
            container(column![coalescer_kinds, threshold_slider].spacing(8))
                .style(style::modal_container)
                .padding(8)
                .into(),
        )
    } else {
        None
    };

    let size_filters_column = column![
        text("Size filters").size(crate::style::text_size::SECTION),
        column![trade_size_slider, order_size_slider].spacing(8),
    ]
    .spacing(8);

    let noise_filters_column = {
        let merge_checkbox = checkbox(cfg.coalescing.is_some())
            .label("Merge orders if sizes are similar")
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
            });

        let mut col = column![
            text("Noise filters").size(crate::style::text_size::SECTION),
            merge_checkbox
        ]
        .spacing(8);
        if let Some(c) = coalescer_cfg {
            col = col.push(c);
        }
        col
    };

    let trade_viz_column = {
        let dyn_checkbox = checkbox(cfg.trade_size_scale.is_some())
            .label("Dynamic circle radius")
            .on_toggle(move |value| {
                Message::VisualConfigChanged(
                    pane,
                    VisualConfig::Heatmap(heatmap::Config {
                        trade_size_scale: if value { Some(100) } else { None },
                        ..cfg
                    }),
                    false,
                )
            });

        let mut col = column![
            text("Trade visualization").size(crate::style::text_size::SECTION),
            dyn_checkbox
        ]
        .spacing(8);
        if let Some(slider) = circle_scaling_slider {
            col = col.push(slider);
        }
        col
    };

    let study_cfg = study_config.view(studies, basis).map(move |msg| {
        Message::PaneEvent(
            pane,
            Event::StudyConfigurator(study::StudyMessage::Heatmap(msg)),
        )
    });

    let content = split_column![
        size_filters_column,
        noise_filters_column,
        trade_viz_column,
        column![text("Studies").size(crate::style::text_size::SECTION), study_cfg].spacing(8),
        row![
            space::horizontal(),
            sync_all_button(pane, VisualConfig::Heatmap(cfg))
        ]
        ; spacing = 12, align_x = Alignment::Start
    ];

    cfg_view_container(360, content)
}

pub fn heatmap_shader_cfg_view<'a>(
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
            |value| format!(">${}", format_with_commas(*value as f64)),
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
            |value| format!(">${}", format_with_commas(*value as f64)),
            Some(5000.0),
        )
    };

    let circle_scaling_slider = cfg.trade_size_scale.map(|radius_scale| {
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
            Some(text(format!("{}%", radius_scale)).size(crate::style::text_size::EMPHASIS)),
        )
    });

    let size_filters_column = column![
        text("Size filters").size(crate::style::text_size::SECTION),
        column![trade_size_slider, order_size_slider].spacing(8),
    ]
    .spacing(8);

    let trade_viz_column = {
        let dyn_checkbox = checkbox(cfg.trade_size_scale.is_some())
            .label("Dynamic circle radius")
            .on_toggle(move |value| {
                Message::VisualConfigChanged(
                    pane,
                    VisualConfig::Heatmap(heatmap::Config {
                        trade_size_scale: if value { Some(100) } else { None },
                        ..cfg
                    }),
                    false,
                )
            });

        let mut col = column![
            text("Trade visualization").size(crate::style::text_size::SECTION),
            dyn_checkbox
        ]
        .spacing(8);
        if let Some(slider) = circle_scaling_slider {
            col = col.push(slider);
        }
        col
    };

    let study_cfg = study_config.view(studies, basis).map(move |msg| {
        Message::PaneEvent(
            pane,
            Event::StudyConfigurator(study::StudyMessage::Heatmap(msg)),
        )
    });

    let content = split_column![
        size_filters_column,
        trade_viz_column,
        column![text("Studies").size(crate::style::text_size::SECTION), study_cfg].spacing(8),
        row![
            space::horizontal(),
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
        let filter = cfg.trade_size_filter;
        let slider = labeled_slider(
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
            |value| format!(">${}", format_with_commas(*value as f64)),
            Some(500.0),
        );

        column![
            text("Size filter").size(crate::style::text_size::SECTION),
            slider
        ]
        .spacing(8)
    };

    let retention_minutes = (cfg.trade_retention.as_secs_f32() / 60.0).max(1.0);
    let retention_slider = {
        let slider_ui = slider(1.0..=60.0, retention_minutes, move |new_minutes| {
            let mins = new_minutes.round().max(1.0) as u64;
            Message::VisualConfigChanged(
                pane,
                VisualConfig::TimeAndSales(timeandsales::Config {
                    trade_retention: Duration::from_secs(mins * 60),
                    ..cfg
                }),
                false,
            )
        })
        .step(1.0);

        classic_slider_row(
            text("Keep trades for"),
            slider_ui.into(),
            Some(
                text(format!("≈ {} min", retention_minutes.round() as u64))
                    .size(crate::style::text_size::EMPHASIS),
            ),
        )
    };

    let history_column = column![
        row![
            text("History").size(crate::style::text_size::SECTION),
            tooltip(
                button("i").style(style::button::info),
                Some("Affects the stacked bar, colors and how much you can scroll down"),
                TooltipPosition::Top,
            )
        ]
        .spacing(4)
        .align_y(Alignment::Center),
        retention_slider
    ]
    .spacing(8);

    let stacked_bar: Element<_> = {
        let is_shown = cfg.stacked_bar.is_some();

        let enable_checkbox = checkbox(is_shown).label("Show stacked bar").on_toggle({
            move |value| {
                let current_ratio = cfg.stacked_bar.map(|h| h.ratio()).unwrap_or_default();
                Message::VisualConfigChanged(
                    pane,
                    VisualConfig::TimeAndSales(timeandsales::Config {
                        stacked_bar: if value {
                            Some(StackedBar::Compact(current_ratio))
                        } else {
                            None
                        },
                        ..cfg
                    }),
                    false,
                )
            }
        });

        let controls: Option<Element<_>> = cfg.stacked_bar.map(|hist| {
            let ratio = hist.ratio();
            let is_compact = matches!(hist, StackedBar::Compact(_));

            let compact = radio("Compact", true, Some(is_compact), {
                move |_v| {
                    Message::VisualConfigChanged(
                        pane,
                        VisualConfig::TimeAndSales(timeandsales::Config {
                            stacked_bar: Some(StackedBar::Compact(ratio)),
                            ..cfg
                        }),
                        false,
                    )
                }
            })
            .spacing(4);

            let full = radio("Full", false, Some(is_compact), {
                move |_v| {
                    Message::VisualConfigChanged(
                        pane,
                        VisualConfig::TimeAndSales(timeandsales::Config {
                            stacked_bar: Some(StackedBar::Full(ratio)),
                            ..cfg
                        }),
                        false,
                    )
                }
            })
            .spacing(4);

            let metric_picklist = pick_list(StackedBarRatio::ALL, Some(ratio), move |new_ratio| {
                let new_hist = Some(match cfg.stacked_bar {
                    Some(StackedBar::Full(_)) => StackedBar::Full(new_ratio),
                    _ => StackedBar::Compact(new_ratio),
                });
                Message::VisualConfigChanged(
                    pane,
                    VisualConfig::TimeAndSales(timeandsales::Config {
                        stacked_bar: new_hist,
                        ..cfg
                    }),
                    false,
                )
            });

            column![
                iced::widget::rule::horizontal(1),
                text("Mode").size(crate::style::text_size::BODY),
                row![compact, full].spacing(12),
                text("Metric").size(crate::style::text_size::BODY),
                metric_picklist,
            ]
            .spacing(8)
            .into()
        });

        let mut inner = column![enable_checkbox]
            .width(Length::Fill)
            .padding(4)
            .spacing(8);

        if let Some(ctrls) = controls {
            inner = inner.push(ctrls);
        }

        container(inner)
            .style(style::modal_container)
            .padding(8)
            .into()
    };

    let content = split_column![
        trade_size_column,
        history_column,
        stacked_bar,
        row![space::horizontal(), sync_all_button(pane, VisualConfig::TimeAndSales(cfg))],
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
    let display_readout_section = {
        let data_labels_checkbox = tooltip(
            checkbox(cfg.data_labels_always_visible)
                .label("Keep latest label visible")
                .on_toggle(move |value| {
                    Message::VisualConfigChanged(
                        pane,
                        VisualConfig::Kline(data::chart::kline::Config {
                            data_labels_always_visible: value,
                            ..cfg
                        }),
                        false,
                    )
                }),
            Some("Show the latest datapoint label even when not hovering"),
            TooltipPosition::Top,
        );
        column![
            text("Data labels").size(crate::style::text_size::SECTION),
            data_labels_checkbox,
        ]
        .spacing(8)
    };

    let content = match kind {
        KlineChartKind::Candles => {
            split_column![
                display_readout_section,
                row![
                    space::horizontal(),
                    sync_all_button(pane, VisualConfig::Kline(cfg))
                ],
                ; spacing = 12, align_x = Alignment::Start
            ]
        }
        KlineChartKind::Footprint {
            clusters,
            scaling,
            studies,
        } => {
            let cluster_picklist =
                pick_list(ClusterKind::ALL, Some(clusters), move |new_cluster_kind| {
                    Message::PaneEvent(pane, Event::ClusterKindSelected(new_cluster_kind))
                });

            let footprint_summary_checkbox = tooltip(
                checkbox(cfg.show_footprint_summary)
                    .label("Show footprint summary")
                    .on_toggle(move |value| {
                        Message::VisualConfigChanged(
                            pane,
                            VisualConfig::Kline(data::chart::kline::Config {
                                show_footprint_summary: value,
                                ..cfg
                            }),
                            false,
                        )
                    }),
                Some("Show per-bar volume and delta below footprint candles"),
                TooltipPosition::Top,
            );

            let scaling = {
                let picklist = pick_list(
                    data::chart::kline::ClusterScaling::ALL,
                    Some(scaling),
                    move |new_scaling| {
                        Message::PaneEvent(pane, Event::ClusterScalingSelected(new_scaling))
                    },
                );

                if let data::chart::kline::ClusterScaling::Hybrid { weight } = scaling {
                    let hybrid_slider = slider(0.0..=1.0, *weight, move |new_weight| {
                        Message::PaneEvent(
                            pane,
                            Event::ClusterScalingSelected(
                                data::chart::kline::ClusterScaling::Hybrid { weight: new_weight },
                            ),
                        )
                    })
                    .step(0.05);

                    column![
                        picklist,
                        hybrid_slider,
                        text("Blend visible-range and per-candle scaling"),
                    ]
                    .spacing(8)
                } else {
                    column![picklist].spacing(8)
                }
            };

            let available_studies: Vec<_> = data::chart::kline::FootprintStudy::ALL.to_vec();

            let active_studies: Vec<_> = studies
                .iter()
                .copied()
                .filter(|study| {
                    available_studies
                        .iter()
                        .any(|available| available.is_same_type(study))
                })
                .collect();

            let study_cfg = study_config
                .view_available(&active_studies, basis, available_studies, &[])
                .map(move |msg| {
                    Message::PaneEvent(
                        pane,
                        Event::StudyConfigurator(study::StudyMessage::Footprint(msg)),
                    )
                });

            let mut content = split_column![
                display_readout_section,
                column![text("Footprint summary").size(crate::style::text_size::SECTION), footprint_summary_checkbox].spacing(8),
                column![text("Cluster type").size(crate::style::text_size::SECTION), cluster_picklist].spacing(8),
                ; spacing = 12, align_x = Alignment::Start
            ];

            content = content.push(
                column![
                    text("Cluster scaling").size(crate::style::text_size::SECTION),
                    scaling
                ]
                .spacing(8),
            );

            content = content
                .push(
                    column![
                        text("Studies").size(crate::style::text_size::SECTION),
                        study_cfg
                    ]
                    .spacing(8),
                )
                .push(row![
                    space::horizontal(),
                    sync_all_button(pane, VisualConfig::Kline(cfg))
                ]);

            content
        }
    };

    cfg_view_container(360, content)
}

pub fn kline_v2_cfg_view<'a>(
    study_config: &'a study::Configurator<KlineStudy>,
    studies: &'a [KlineStudy],
    cluster_kind: ClusterKind,
    trade_data_enabled: bool,
    pane: pane_grid::Pane,
    basis: data::chart::Basis,
) -> Element<'a, Message> {
    let active_studies = studies.to_vec();
    let available_studies = KlineStudy::ALL
        .into_iter()
        .filter_map(|study| study.normalized_for_basis(basis))
        .collect();
    let disabled_studies: Vec<KlineStudy> = if trade_data_enabled {
        Vec::new()
    } else {
        KlineStudy::ALL
            .iter()
            .copied()
            .filter(|study| study.requires_trade_data())
            .collect()
    };
    let study_cfg = study_config
        .view_available(&active_studies, basis, available_studies, &disabled_studies)
        .map(move |message| {
            Message::PaneEvent(
                pane,
                Event::StudyConfigurator(study::StudyMessage::KlineV2(message)),
            )
        });

    let cluster_picklist = pick_list(ClusterKind::ALL, Some(cluster_kind), move |kind| {
        Message::PaneEvent(pane, Event::ClusterKindSelected(kind))
    });

    let trade_data_required_for_basis = matches!(basis, data::chart::Basis::Tick(_));
    let trade_data_enabled_checkbox = tooltip(
        checkbox(trade_data_enabled)
            .label("Enable trade data")
            .on_toggle_maybe((!trade_data_required_for_basis).then_some(move |enabled| {
                Message::PaneEvent(pane, Event::TradeDataEnabledChanged(enabled))
            })),
        Some(if trade_data_required_for_basis {
            "Trade data is required for tick-based charts"
        } else {
            "Allow footprint studies and indicators that need trade data to subscribe and fetch it"
        }),
        TooltipPosition::Top,
    );

    cfg_view_container(
        360,
        column![
            trade_data_enabled_checkbox,
            text("Footprint cluster type").size(crate::style::text_size::SECTION),
            cluster_picklist,
            text("Studies").size(crate::style::text_size::SECTION),
            study_cfg
        ]
        .spacing(12),
    )
}

fn candlestick_color_section<'a>(
    config: CandlestickConfig,
    pane: pane_grid::Pane,
) -> Element<'a, Message> {
    let colors = config.colors;
    let controls = [
        (CandlestickColorSlot::Bullish, "Bullish"),
        (CandlestickColorSlot::Bearish, "Bearish"),
    ]
    .into_iter()
    .map(|(slot, label)| {
        let selected = colors.color_for(slot);
        let color_choices = theme_color_control(selected, move |accent| {
            Message::PaneEvent(
                pane,
                Event::KlineV2CandlestickSettings(CandlestickSettingsMessage::Config(
                    config.with_color(
                        slot,
                        ThemeColor::with_accent(selected.role, accent, selected.shade),
                    ),
                )),
            )
        });
        let shade_picklist = pick_list(ThemeColorShade::ALL, Some(selected.shade), move |shade| {
            Message::PaneEvent(
                pane,
                Event::KlineV2CandlestickSettings(CandlestickSettingsMessage::Config(
                    config.with_color(
                        slot,
                        ThemeColor::with_accent(selected.role, selected.accent, shade),
                    ),
                )),
            )
        });

        column![
            row![text(label).width(Length::Fill), shade_picklist].spacing(8),
            color_choices,
        ]
        .spacing(4)
        .into()
    })
    .collect::<Vec<_>>();

    column![
        text("Colors").size(crate::style::text_size::SECTION),
        column(controls).spacing(8),
    ]
    .spacing(8)
    .into()
}

pub fn candlestick_cfg_view<'a>(
    config: CandlestickConfig,
    mark: MarkKind,
    series_editor: &'a TickerSeriesEditor,
    series: Vec<SeriesStyleEntry>,
    pane: pane_grid::Pane,
) -> Element<'a, Message> {
    let reset = button("Reset to defaults").on_press(Message::PaneEvent(
        pane,
        Event::KlineV2CandlestickSettings(CandlestickSettingsMessage::Reset),
    ));
    let opacity = config.opacity_percent.clamp(
        MIN_CANDLESTICK_OPACITY_PERCENT,
        MAX_CANDLESTICK_OPACITY_PERCENT,
    );
    let opacity_slider = slider(
        f32::from(MIN_CANDLESTICK_OPACITY_PERCENT)..=f32::from(MAX_CANDLESTICK_OPACITY_PERCENT),
        f32::from(opacity),
        move |value| {
            Message::PaneEvent(
                pane,
                Event::KlineV2CandlestickSettings(CandlestickSettingsMessage::Config(
                    config.with_opacity_percent(value.round() as u8),
                )),
            )
        },
    )
    .step(5.0);

    let selected_mark = mark;
    let mark_controls = [("Candles", MarkKind::Candle), ("Line", MarkKind::Line)]
        .into_iter()
        .map(|(label, mark)| {
            radio(label, mark, Some(selected_mark), move |value| {
                Message::PaneEvent(
                    pane,
                    Event::KlineV2CandlestickSettings(CandlestickSettingsMessage::Mark(value)),
                )
            })
            .into()
        });

    let title = row![
        text("Candlesticks settings").size(crate::style::text_size::TITLE),
        space::horizontal(),
        reset,
    ]
    .align_y(Alignment::Center)
    .spacing(12);

    let mut content = column![
        title,
        column![
            text("Display mark").size(crate::style::text_size::SECTION),
            row(mark_controls).spacing(12),
        ]
        .spacing(8),
    ]
    .spacing(16);

    if matches!(mark, MarkKind::Candle) {
        content = content
            .push(candlestick_color_section(config, pane))
            .push(text(format!("Opacity: {opacity}%")))
            .push(opacity_slider);
    } else {
        content = content
            .push(text("Series").size(crate::style::text_size::SECTION))
            .push(series_editor.view(series).map(move |message| {
                Message::PaneEvent(
                    pane,
                    Event::KlineV2Interaction(crate::chart::kline_v2::Message::SeriesEditor(
                        message,
                    )),
                )
            }));
    }

    cfg_view_container(360, content)
}

pub fn series_cfg_view<'a>(
    series_editor: &'a TickerSeriesEditor,
    series: Vec<SeriesStyleEntry>,
    pane: pane_grid::Pane,
) -> Element<'a, Message> {
    cfg_view_container(
        360,
        column![
            text("Series settings").size(crate::style::text_size::TITLE),
            series_editor.view(series).map(move |message| {
                Message::PaneEvent(
                    pane,
                    Event::KlineV2Interaction(crate::chart::kline_v2::Message::SeriesEditor(
                        message,
                    )),
                )
            }),
        ]
        .spacing(16),
    )
}

pub fn kline_v2_indicator_cfg_view<'a>(
    indicator: KlineV2Indicator,
    data_config: Option<IndicatorConfig>,
    anchored_vwap_config: AnchoredVwapConfig,
    basis: data::chart::Basis,
    color_config: IndicatorColorConfig,
    mark: Option<data::chart::kline_v2::MarkKind>,
    pane: pane_grid::Pane,
) -> Element<'a, Message> {
    let mark_section = indicator_mark_section(indicator, mark, pane);
    let color_section = indicator_color_section(indicator, color_config, pane, mark);

    let reset = button("Reset to default").on_press(Message::PaneEvent(
        pane,
        Event::KlineV2IndicatorSettings(indicator, KlineV2IndicatorSettingsMessage::Reset),
    ));
    let title = row![
        text(format!("{} settings", indicator)).size(crate::style::text_size::TITLE),
        space::horizontal(),
        reset,
    ]
    .align_y(Alignment::Center)
    .spacing(12);
    let mut content = column![title].spacing(16);

    match (indicator, data_config) {
        (KlineV2Indicator::AnchoredVwap, _) => {
            let anchors: Vec<_> = VwapAnchor::STANDARD
                .into_iter()
                .filter(|anchor| anchor.supports_basis(basis))
                .collect();
            let anchor = pick_list(anchors, Some(anchored_vwap_config.anchor), move |anchor| {
                Message::PaneEvent(
                    pane,
                    Event::KlineV2IndicatorSettings(
                        indicator,
                        KlineV2IndicatorSettingsMessage::DataConfig(IndicatorConfig::AnchoredVwap(
                            AnchoredVwapConfig {
                                anchor,
                                ..anchored_vwap_config
                            },
                        )),
                    ),
                )
            });
            let source = pick_list(
                data::chart::kline_v2::KlinePriceSource::ALL,
                Some(anchored_vwap_config.source),
                move |source| {
                    Message::PaneEvent(
                        pane,
                        Event::KlineV2IndicatorSettings(
                            indicator,
                            KlineV2IndicatorSettingsMessage::DataConfig(
                                IndicatorConfig::AnchoredVwap(AnchoredVwapConfig {
                                    source,
                                    ..anchored_vwap_config
                                }),
                            ),
                        ),
                    )
                },
            );
            content = content.push(
                column![
                    text("Anchor").size(crate::style::text_size::SECTION),
                    anchor,
                    text("Price source").size(crate::style::text_size::SECTION),
                    source,
                ]
                .spacing(8),
            );
        }
        (KlineV2Indicator::Rsi, Some(IndicatorConfig::Rsi(config))) => {
            let rsi_period = labeled_slider(
                "Period",
                2..=100,
                config.period,
                move |period| {
                    Message::PaneEvent(
                        pane,
                        Event::KlineV2IndicatorSettings(
                            indicator,
                            KlineV2IndicatorSettingsMessage::DataConfig(IndicatorConfig::Rsi(
                                RsiConfig { period, ..config },
                            )),
                        ),
                    )
                },
                |value| value.to_string(),
                None,
            );
            let smoothing_enabled = matches!(config.smoothing, RsiSmoothing::Ema(_));
            let smoothing_toggle =
                checkbox(smoothing_enabled)
                    .label("EMA signal")
                    .on_toggle(move |enabled| {
                        let smoothing = if enabled {
                            RsiSmoothing::Ema(match config.smoothing {
                                RsiSmoothing::Ema(period) => period.max(2),
                                RsiSmoothing::None => 9,
                            })
                        } else {
                            RsiSmoothing::None
                        };
                        Message::PaneEvent(
                            pane,
                            Event::KlineV2IndicatorSettings(
                                indicator,
                                KlineV2IndicatorSettingsMessage::DataConfig(IndicatorConfig::Rsi(
                                    RsiConfig {
                                        smoothing,
                                        ..config
                                    },
                                )),
                            ),
                        )
                    });
            let smoothing_period = match config.smoothing {
                RsiSmoothing::Ema(period) => Some(labeled_slider(
                    "EMA period",
                    2..=50,
                    period,
                    move |period| {
                        Message::PaneEvent(
                            pane,
                            Event::KlineV2IndicatorSettings(
                                indicator,
                                KlineV2IndicatorSettingsMessage::DataConfig(IndicatorConfig::Rsi(
                                    RsiConfig {
                                        smoothing: RsiSmoothing::Ema(period),
                                        ..config
                                    },
                                )),
                            ),
                        )
                    },
                    |value| value.to_string(),
                    None,
                )),
                RsiSmoothing::None => None,
            };
            let upper_band = labeled_slider(
                "Upper band",
                1.0..=99.0,
                config.upper_band,
                move |upper_band| {
                    Message::PaneEvent(
                        pane,
                        Event::KlineV2IndicatorSettings(
                            indicator,
                            KlineV2IndicatorSettingsMessage::DataConfig(IndicatorConfig::Rsi(
                                RsiConfig {
                                    upper_band,
                                    ..config
                                },
                            )),
                        ),
                    )
                },
                |value| format!("{value:.0}"),
                Some(1.0),
            );
            let lower_band = labeled_slider(
                "Lower band",
                1.0..=99.0,
                config.lower_band,
                move |lower_band| {
                    Message::PaneEvent(
                        pane,
                        Event::KlineV2IndicatorSettings(
                            indicator,
                            KlineV2IndicatorSettingsMessage::DataConfig(IndicatorConfig::Rsi(
                                RsiConfig {
                                    lower_band,
                                    ..config
                                },
                            )),
                        ),
                    )
                },
                |value| format!("{value:.0}"),
                Some(1.0),
            );
            content = content.push(
                column![
                    text("Calculation").size(crate::style::text_size::SECTION),
                    rsi_period,
                    smoothing_toggle,
                    smoothing_period.unwrap_or_else(|| space::vertical().height(0).into()),
                    text("Bands").size(crate::style::text_size::SECTION),
                    upper_band,
                    lower_band,
                ]
                .spacing(8),
            );
        }
        (KlineV2Indicator::BollingerBands, Some(IndicatorConfig::BollingerBands(config))) => {
            let period = labeled_slider(
                "Period",
                2..=200,
                config.period,
                move |period| {
                    Message::PaneEvent(
                        pane,
                        Event::KlineV2IndicatorSettings(
                            indicator,
                            KlineV2IndicatorSettingsMessage::DataConfig(
                                IndicatorConfig::BollingerBands(BollingerBandsCalculationConfig {
                                    period,
                                    ..config
                                }),
                            ),
                        ),
                    )
                },
                |value| value.to_string(),
                None,
            );
            let method = row![
                radio(
                    "SMA",
                    MovingAverageMethod::Sma,
                    Some(config.method),
                    move |method| {
                        Message::PaneEvent(
                            pane,
                            Event::KlineV2IndicatorSettings(
                                indicator,
                                KlineV2IndicatorSettingsMessage::DataConfig(
                                    IndicatorConfig::BollingerBands(
                                        BollingerBandsCalculationConfig { method, ..config },
                                    ),
                                ),
                            ),
                        )
                    },
                ),
                radio(
                    "EMA",
                    MovingAverageMethod::Ema,
                    Some(config.method),
                    move |method| {
                        Message::PaneEvent(
                            pane,
                            Event::KlineV2IndicatorSettings(
                                indicator,
                                KlineV2IndicatorSettingsMessage::DataConfig(
                                    IndicatorConfig::BollingerBands(
                                        BollingerBandsCalculationConfig { method, ..config },
                                    ),
                                ),
                            ),
                        )
                    },
                ),
                radio(
                    "VWMA",
                    MovingAverageMethod::Vwma,
                    Some(config.method),
                    move |method| {
                        Message::PaneEvent(
                            pane,
                            Event::KlineV2IndicatorSettings(
                                indicator,
                                KlineV2IndicatorSettingsMessage::DataConfig(
                                    IndicatorConfig::BollingerBands(
                                        BollingerBandsCalculationConfig { method, ..config },
                                    ),
                                ),
                            ),
                        )
                    },
                ),
            ]
            .spacing(12);
            let std_dev_multiplier = labeled_slider(
                "Std. deviation",
                0.0..=5.0,
                config.std_dev_multiplier,
                move |std_dev_multiplier| {
                    Message::PaneEvent(
                        pane,
                        Event::KlineV2IndicatorSettings(
                            indicator,
                            KlineV2IndicatorSettingsMessage::DataConfig(
                                IndicatorConfig::BollingerBands(BollingerBandsCalculationConfig {
                                    std_dev_multiplier,
                                    ..config
                                }),
                            ),
                        ),
                    )
                },
                |value| format!("{value:.1}"),
                Some(0.1),
            );
            content = content.push(
                column![
                    text("Calculation").size(crate::style::text_size::SECTION),
                    period,
                    method,
                    std_dev_multiplier,
                ]
                .spacing(8),
            );
        }
        (KlineV2Indicator::MovingAverages, Some(IndicatorConfig::MovingAverages(config))) => {
            let method = row![
                radio(
                    "SMA",
                    MovingAverageMethod::Sma,
                    Some(config.method),
                    move |method| {
                        Message::PaneEvent(
                            pane,
                            Event::KlineV2IndicatorSettings(
                                indicator,
                                KlineV2IndicatorSettingsMessage::DataConfig(
                                    IndicatorConfig::MovingAverages(
                                        MovingAveragesCalculationConfig { method, ..config },
                                    ),
                                ),
                            ),
                        )
                    },
                ),
                radio(
                    "EMA",
                    MovingAverageMethod::Ema,
                    Some(config.method),
                    move |method| {
                        Message::PaneEvent(
                            pane,
                            Event::KlineV2IndicatorSettings(
                                indicator,
                                KlineV2IndicatorSettingsMessage::DataConfig(
                                    IndicatorConfig::MovingAverages(
                                        MovingAveragesCalculationConfig { method, ..config },
                                    ),
                                ),
                            ),
                        )
                    },
                ),
                radio(
                    "VWMA",
                    MovingAverageMethod::Vwma,
                    Some(config.method),
                    move |method| {
                        Message::PaneEvent(
                            pane,
                            Event::KlineV2IndicatorSettings(
                                indicator,
                                KlineV2IndicatorSettingsMessage::DataConfig(
                                    IndicatorConfig::MovingAverages(
                                        MovingAveragesCalculationConfig { method, ..config },
                                    ),
                                ),
                            ),
                        )
                    },
                ),
            ]
            .spacing(12);
            let fast = labeled_slider(
                "Fast period",
                1..=500,
                config.fast_period,
                move |value| {
                    Message::PaneEvent(
                        pane,
                        Event::KlineV2IndicatorSettings(
                            indicator,
                            KlineV2IndicatorSettingsMessage::DataConfig(
                                IndicatorConfig::MovingAverages(MovingAveragesCalculationConfig {
                                    fast_period: value,
                                    ..config
                                }),
                            ),
                        ),
                    )
                },
                |value| value.to_string(),
                None,
            );
            let medium = labeled_slider(
                "Medium period",
                1..=500,
                config.medium_period,
                move |value| {
                    Message::PaneEvent(
                        pane,
                        Event::KlineV2IndicatorSettings(
                            indicator,
                            KlineV2IndicatorSettingsMessage::DataConfig(
                                IndicatorConfig::MovingAverages(MovingAveragesCalculationConfig {
                                    medium_period: value,
                                    ..config
                                }),
                            ),
                        ),
                    )
                },
                |value| value.to_string(),
                None,
            );
            let slow = labeled_slider(
                "Slow period",
                1..=500,
                config.slow_period,
                move |value| {
                    Message::PaneEvent(
                        pane,
                        Event::KlineV2IndicatorSettings(
                            indicator,
                            KlineV2IndicatorSettingsMessage::DataConfig(
                                IndicatorConfig::MovingAverages(MovingAveragesCalculationConfig {
                                    slow_period: value,
                                    ..config
                                }),
                            ),
                        ),
                    )
                },
                |value| value.to_string(),
                None,
            );
            content = content.push(
                column![
                    text("Average method").size(crate::style::text_size::SECTION),
                    method,
                    text("Periods").size(crate::style::text_size::SECTION),
                    fast,
                    medium,
                    slow,
                ]
                .spacing(8),
            );
        }
        _ => {}
    }

    if let Some(mark_section) = mark_section {
        content = content.push(mark_section);
    }
    content = content.push(color_section);

    cfg_view_container(400, content)
}

fn indicator_color_section<'a>(
    indicator: KlineV2Indicator,
    color_config: IndicatorColorConfig,
    pane: pane_grid::Pane,
    mark: Option<MarkKind>,
) -> Element<'a, Message> {
    let active_slots = indicator.active_color_slots(mark);
    let controls = indicator
        .color_specs()
        .iter()
        .filter(|spec| active_slots.is_none_or(|slots| slots.contains(&spec.slot)))
        .map(|spec| {
            let selected = color_config.color_for(spec.slot);
            let send_color = move |accent| {
                Message::PaneEvent(
                    pane,
                    Event::KlineV2IndicatorSettings(
                        indicator,
                        KlineV2IndicatorSettingsMessage::Color {
                            slot: spec.slot,
                            accent,
                            shade: selected.shade,
                        },
                    ),
                )
            };
            let color_choices = theme_color_control(selected, send_color);
            let shade_picklist: Element<'a, Message> =
                pick_list(ThemeColorShade::ALL, Some(selected.shade), move |shade| {
                    Message::PaneEvent(
                        pane,
                        Event::KlineV2IndicatorSettings(
                            indicator,
                            KlineV2IndicatorSettingsMessage::Color {
                                slot: spec.slot,
                                accent: selected.accent,
                                shade,
                            },
                        ),
                    )
                })
                .into();
            column![
                row![text(spec.label).width(Length::Fill), shade_picklist].spacing(8),
                color_choices,
            ]
            .spacing(4)
            .into()
        });

    column![
        text("Colors").size(crate::style::text_size::SECTION),
        column(controls).spacing(8),
    ]
    .spacing(8)
    .into()
}

fn indicator_mark_section<'a>(
    indicator: KlineV2Indicator,
    mark: Option<data::chart::kline_v2::MarkKind>,
    pane: pane_grid::Pane,
) -> Option<Element<'a, Message>> {
    let mark_specs = indicator.mark_specs();
    if mark_specs.is_empty() {
        return None;
    }

    let selected = mark.or_else(|| mark_specs.first().map(|spec| spec.mark));
    let send_mark = move |value| {
        Message::PaneEvent(
            pane,
            Event::KlineV2IndicatorSettings(
                indicator,
                KlineV2IndicatorSettingsMessage::Mark(value),
            ),
        )
    };
    let controls = mark_specs
        .iter()
        .map(|spec| radio(spec.label, spec.mark, selected, send_mark).into());

    Some(
        column![
            text("Display mark").size(crate::style::text_size::SECTION),
            row(controls).spacing(12),
        ]
        .spacing(8)
        .into(),
    )
}

pub fn ladder_cfg_view<'a>(cfg: ladder::Config, pane: pane_grid::Pane) -> Element<'a, Message> {
    let display_options = {
        let spread = checkbox(cfg.show_spread)
            .label("Show Spread")
            .on_toggle(move |value| {
                Message::VisualConfigChanged(
                    pane,
                    VisualConfig::Ladder(ladder::Config {
                        show_spread: value,
                        ..cfg
                    }),
                    false,
                )
            });

        let chase_tracker = checkbox(cfg.show_chase_tracker)
            .label("Show Chase Tracker")
            .on_toggle(move |value| {
                Message::VisualConfigChanged(
                    pane,
                    VisualConfig::Ladder(ladder::Config {
                        show_chase_tracker: value,
                        ..cfg
                    }),
                    false,
                )
            });

        column![
            text("Display Options").size(crate::style::text_size::SECTION),
            column![
                spread,
                row![
                    chase_tracker,
                    tooltip(
                        button("i").style(style::button::info),
                        Some("Highlights consecutive best-price moves and fades when momentum stalls.\nCalculated using raw ungrouped data."),
                        TooltipPosition::Top,
                    )
                ]
                .align_y(Alignment::Center)
                .spacing(4)
            ]
            .spacing(4)
        ]
        .spacing(8)
    };

    let retention_slider = {
        let retention_minutes = (cfg.trade_retention.as_secs_f32() / 60.0).max(1.0);

        let slider_ui = slider(1.0..=60.0, retention_minutes, move |new_minutes| {
            let mins = new_minutes.round().max(1.0) as u64;
            Message::VisualConfigChanged(
                pane,
                VisualConfig::Ladder(ladder::Config {
                    trade_retention: Duration::from_secs(mins * 60),
                    ..cfg
                }),
                false,
            )
        })
        .step(1.0);

        classic_slider_row(
            text("Keep trades for"),
            slider_ui.into(),
            Some(
                text(format!("≈ {} min", retention_minutes.round() as u64))
                    .size(crate::style::text_size::EMPHASIS),
            ),
        )
    };

    let history_column = column![
        text("History").size(crate::style::text_size::SECTION),
        retention_slider
    ]
    .spacing(8);

    let content = split_column![
        display_options,
        history_column,
        row![
            space::horizontal(),
            sync_all_button(pane, VisualConfig::Ladder(cfg))
        ],
        ; spacing = 12, align_x = Alignment::Start
    ];

    cfg_view_container(320, content)
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
    use data::chart::kline_v2::{
        FootprintPocColorConfig, FootprintPocConfig, KlineStudy,
        MAX_VOLUME_PROFILE_OPACITY_PERCENT, MAX_VOLUME_PROFILE_WIDTH_PERCENT,
        MIN_VOLUME_PROFILE_OPACITY_PERCENT, MIN_VOLUME_PROFILE_WIDTH_PERCENT,
        VolumeProfileColorConfig, VolumeProfileSchedule,
    };
    use iced::{
        Element, Length, padding,
        widget::{button, checkbox, column, container, pick_list, row, slider, space, text},
    };

    #[derive(Debug, Clone, Copy)]
    pub enum StudyMessage {
        Footprint(Message<FootprintStudy>),
        Heatmap(Message<HeatmapStudy>),
        KlineV2(Message<KlineStudy>),
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

    fn volume_profile_color_section<'a>(
        colors: VolumeProfileColorConfig,
        schedule: VolumeProfileSchedule,
        on_change: impl Fn(VolumeProfileColorConfig) -> Message<KlineStudy> + Copy + 'a,
    ) -> Element<'a, Message<KlineStudy>> {
        let controls = VolumeProfileColorConfig::color_specs_for(schedule)
            .iter()
            .map(|spec| {
                let slot = spec.slot;
                let enabled = match slot {
                    data::chart::kline_v2::VolumeProfileColorSlot::NakedPoc => colors.show_npoc,
                    data::chart::kline_v2::VolumeProfileColorSlot::FilledPoc => colors.show_poc,
                    _ => true,
                };
                let label: Element<'a, Message<KlineStudy>> = match slot {
                    data::chart::kline_v2::VolumeProfileColorSlot::NakedPoc => {
                        checkbox(colors.show_npoc)
                            .label(spec.label)
                            .on_toggle(move |show| on_change(colors.with_show_npoc(show)))
                            .width(Length::Fill)
                            .into()
                    }
                    data::chart::kline_v2::VolumeProfileColorSlot::FilledPoc => {
                        checkbox(colors.show_poc)
                            .label(spec.label)
                            .on_toggle(move |show| on_change(colors.with_show_poc(show)))
                            .width(Length::Fill)
                            .into()
                    }
                    _ => text(spec.label).width(Length::Fill).into(),
                };

                if !enabled {
                    column![row![label]].spacing(4).into()
                } else {
                    let selected = colors.color_for(slot);
                    let send_color = move |accent| {
                        on_change(colors.with_color(
                            slot,
                            data::config::theme::ThemeColor::with_accent(
                                selected.role,
                                accent,
                                selected.shade,
                            ),
                        ))
                    };
                    let color_choices = crate::widget::theme_color_control(selected, send_color);
                    let shade_picklist = pick_list(
                        data::config::theme::ThemeColorShade::ALL,
                        Some(selected.shade),
                        move |shade| {
                            on_change(colors.with_color(
                                slot,
                                data::config::theme::ThemeColor::with_accent(
                                    selected.role,
                                    selected.accent,
                                    shade,
                                ),
                            ))
                        },
                    );

                    column![row![label, shade_picklist].spacing(8), color_choices,]
                        .spacing(4)
                        .into()
                }
            });

        column![
            text("Colors").size(crate::style::text_size::SECTION),
            column(controls).spacing(8),
        ]
        .spacing(8)
        .into()
    }

    fn footprint_poc_color_section<'a>(
        config: FootprintPocConfig,
        on_change: impl Fn(FootprintPocConfig) -> Message<KlineStudy> + Copy + 'a,
    ) -> Element<'a, Message<KlineStudy>> {
        let controls = FootprintPocColorConfig::color_specs().iter().map(|spec| {
            let slot = spec.slot;
            let enabled = match slot {
                data::chart::kline_v2::FootprintPocColorSlot::Poc => config.show_poc,
                data::chart::kline_v2::FootprintPocColorSlot::Npoc => config.show_npoc,
            };
            let label: Element<'a, Message<KlineStudy>> = match slot {
                data::chart::kline_v2::FootprintPocColorSlot::Poc => checkbox(config.show_poc)
                    .label(spec.label)
                    .on_toggle(move |show| on_change(config.with_show_poc(show)))
                    .width(Length::Fill)
                    .into(),
                data::chart::kline_v2::FootprintPocColorSlot::Npoc => checkbox(config.show_npoc)
                    .label(spec.label)
                    .on_toggle(move |show| on_change(config.with_show_npoc(show)))
                    .width(Length::Fill)
                    .into(),
            };

            if !enabled {
                column![row![label]].spacing(4).into()
            } else {
                let selected = config.colors.color_for(slot);
                let send_color = move |accent| {
                    on_change(config.with_colors(config.colors.with_color(
                        slot,
                        data::config::theme::ThemeColor::with_accent(
                            selected.role,
                            accent,
                            selected.shade,
                        ),
                    )))
                };
                let color_choices = crate::widget::theme_color_control(selected, send_color);
                let shade_picklist = pick_list(
                    data::config::theme::ThemeColorShade::ALL,
                    Some(selected.shade),
                    move |shade| {
                        on_change(config.with_colors(config.colors.with_color(
                            slot,
                            data::config::theme::ThemeColor::with_accent(
                                selected.role,
                                selected.accent,
                                shade,
                            ),
                        )))
                    },
                );

                column![row![label, shade_picklist].spacing(8), color_choices,]
                    .spacing(4)
                    .into()
            }
        });

        column![
            text("Colors").size(crate::style::text_size::SECTION),
            column(controls).spacing(8),
        ]
        .spacing(8)
        .into()
    }

    fn volume_profile_width_section<'a>(
        width_percent: u8,
        reserved_area: &'static str,
        on_change: impl Fn(u8) -> Message<KlineStudy> + Copy + 'a,
    ) -> Element<'a, Message<KlineStudy>> {
        let width_percent = width_percent.clamp(
            MIN_VOLUME_PROFILE_WIDTH_PERCENT,
            MAX_VOLUME_PROFILE_WIDTH_PERCENT,
        );
        let width_slider = slider(
            f32::from(MIN_VOLUME_PROFILE_WIDTH_PERCENT)
                ..=f32::from(MAX_VOLUME_PROFILE_WIDTH_PERCENT),
            f32::from(width_percent),
            move |new_width| {
                on_change(new_width.round().clamp(
                    f32::from(MIN_VOLUME_PROFILE_WIDTH_PERCENT),
                    f32::from(MAX_VOLUME_PROFILE_WIDTH_PERCENT),
                ) as u8)
            },
        )
        .step(5.0);

        column![
            text(format!("Width: {width_percent}% {reserved_area}")),
            width_slider,
        ]
        .spacing(4)
        .into()
    }

    fn volume_profile_opacity_section<'a>(
        opacity_percent: u8,
        on_change: impl Fn(u8) -> Message<KlineStudy> + Copy + 'a,
    ) -> Element<'a, Message<KlineStudy>> {
        let opacity_percent = opacity_percent.clamp(
            MIN_VOLUME_PROFILE_OPACITY_PERCENT,
            MAX_VOLUME_PROFILE_OPACITY_PERCENT,
        );
        let opacity_slider = slider(
            f32::from(MIN_VOLUME_PROFILE_OPACITY_PERCENT)
                ..=f32::from(MAX_VOLUME_PROFILE_OPACITY_PERCENT),
            f32::from(opacity_percent),
            move |new_opacity| {
                on_change(new_opacity.round().clamp(
                    f32::from(MIN_VOLUME_PROFILE_OPACITY_PERCENT),
                    f32::from(MAX_VOLUME_PROFILE_OPACITY_PERCENT),
                ) as u8)
            },
        )
        .step(5.0);

        column![text(format!("Opacity: {opacity_percent}%")), opacity_slider,]
            .spacing(4)
            .into()
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

                        let color_scale_checkbox = checkbox(color_scale_enabled)
                            .label("Dynamic color scaling")
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
                        let cbox = checkbox(ignore_zeros).label("Ignore zeros").on_toggle(
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
                            row![space::horizontal(), switch_kind,],
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

                        column![row![space::horizontal(), switch_kind,],]
                            .padding(8)
                            .spacing(4)
                            .into()
                    }
                },
            }
        }
    }

    impl Study for KlineStudy {
        fn is_same_type(&self, other: &Self) -> bool {
            (*self).is_same_type(*other)
        }

        fn all() -> Vec<Self> {
            KlineStudy::ALL.to_vec()
        }

        fn view_config<'a>(
            &self,
            basis: data::chart::Basis,
            on_change: impl Fn(Self) -> Message<Self> + Copy + 'a,
        ) -> Element<'a, Message<Self>> {
            match self {
                KlineStudy::VolumeProfile {
                    schedule,
                    colors,
                    width_percent,
                    opacity_percent,
                } => {
                    let schedule = *schedule;
                    let colors = *colors;
                    let width_percent = *width_percent;
                    let opacity_percent = *opacity_percent;
                    let reset = button("Reset to defaults")
                        .on_press(on_change((*self).default_for_type_for_basis(basis)));
                    let color_section =
                        volume_profile_color_section(colors, schedule, move |colors| {
                            on_change(KlineStudy::VolumeProfile {
                                schedule,
                                colors,
                                width_percent,
                                opacity_percent,
                            })
                        });

                    match schedule {
                        VolumeProfileSchedule::VisibleRange => {
                            let width_section = volume_profile_width_section(
                                width_percent,
                                "of the chart width",
                                move |width_percent| {
                                    on_change(KlineStudy::VolumeProfile {
                                        schedule,
                                        colors,
                                        width_percent,
                                        opacity_percent,
                                    })
                                },
                            );
                            let opacity_section = volume_profile_opacity_section(
                                opacity_percent,
                                move |opacity_percent| {
                                    on_change(KlineStudy::VolumeProfile {
                                        schedule,
                                        colors,
                                        width_percent,
                                        opacity_percent,
                                    })
                                },
                            );

                            column![
                                row![space::horizontal(), reset],
                                text("Uses the currently visible chart range"),
                                width_section,
                                opacity_section,
                                color_section,
                            ]
                            .padding(8)
                            .spacing(4)
                            .into()
                        }
                        VolumeProfileSchedule::Recurring { interval } => {
                            if !basis.is_time() {
                                return iced::widget::center(text(
                                    "Recurring volume profiles require a time-based chart",
                                ))
                                .into();
                            }

                            let intervals = VolumeProfileSchedule::recurring_intervals_for(basis);
                            let width_section = volume_profile_width_section(
                                width_percent,
                                "of each profile interval",
                                move |width_percent| {
                                    on_change(KlineStudy::VolumeProfile {
                                        schedule,
                                        colors,
                                        width_percent,
                                        opacity_percent,
                                    })
                                },
                            );
                            let opacity_section = volume_profile_opacity_section(
                                opacity_percent,
                                move |opacity_percent| {
                                    on_change(KlineStudy::VolumeProfile {
                                        schedule,
                                        colors,
                                        width_percent,
                                        opacity_percent,
                                    })
                                },
                            );
                            if intervals.is_empty() {
                                return column![
                                    row![space::horizontal(), reset],
                                    text(
                                        "No recurring interval is at least 4× the chart timeframe",
                                    ),
                                    width_section,
                                    opacity_section,
                                    color_section,
                                ]
                                .padding(8)
                                .spacing(4)
                                .into();
                            }

                            let selected_interval =
                                intervals.contains(&interval).then_some(interval);
                            let interval_picker =
                                pick_list(intervals, selected_interval, move |new_interval| {
                                    on_change(KlineStudy::VolumeProfile {
                                        schedule: VolumeProfileSchedule::Recurring {
                                            interval: new_interval,
                                        },
                                        colors,
                                        width_percent,
                                        opacity_percent,
                                    })
                                });

                            column![
                                row![space::horizontal(), reset],
                                text("Interval"),
                                interval_picker,
                                width_section,
                                opacity_section,
                                color_section
                            ]
                            .padding(8)
                            .spacing(4)
                            .into()
                        }
                    }
                }
                KlineStudy::FootprintPoc { config } => {
                    let config = *config;
                    let reset = button("Reset to defaults")
                        .on_press(on_change((*self).default_for_type_for_basis(basis)));
                    let color_section = footprint_poc_color_section(config, move |config| {
                        on_change(KlineStudy::FootprintPoc { config })
                    });

                    column![
                        row![space::horizontal(), reset],
                        color_section,
                        text("POC/NPOC lines remain active until a later footprint touches the POC price."),
                    ]
                    .padding(8)
                    .spacing(4)
                    .into()
                }
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
            active_studies: &[S],
            basis: data::chart::Basis,
        ) -> Element<'a, Message<S>> {
            self.view_available(active_studies, basis, S::all(), &[])
        }

        pub fn view_available<'a>(
            &self,
            active_studies: &[S],
            basis: data::chart::Basis,
            available_studies: Vec<S>,
            disabled: &[S],
        ) -> Element<'a, Message<S>> {
            let mut content = column![].spacing(4);

            for available_study in available_studies {
                content = content.push(self.create_study_row(
                    available_study,
                    active_studies,
                    basis,
                    disabled,
                ));
            }

            content.into()
        }

        fn create_study_row<'a>(
            &self,
            study: S,
            active_studies: &[S],
            basis: data::chart::Basis,
            disabled: &[S],
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

            let is_disabled = disabled.iter().any(|d| d.is_same_type(&study));

            let checkbox = crate::tooltip(
                checkbox(is_selected)
                    .label(study_config.map_or(study.to_string(), |s| s.to_string()))
                    .on_toggle_maybe(
                        (!is_disabled)
                            .then_some(move |checked| Message::StudyToggled(study, checked)),
                    ),
                is_disabled.then_some("Requires trade data to be enabled"),
                iced::widget::tooltip::Position::Top,
            );

            let mut checkbox_row = row![checkbox, space::horizontal()]
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
