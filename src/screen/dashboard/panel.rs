pub mod timeandsales;

use crate::style::{Icon, icon_text};
use crate::widget::{column_drag, dragger_row};
use crate::{
    screen::dashboard::pane::Message,
    style, tooltip,
    widget::{self, create_slider_row, pane_modal, scrollable_content},
};

use super::pane;
use data::chart::heatmap::CoalesceKind;
use data::chart::{
    Basis, KlineChartKind, VisualConfig, heatmap, indicators::Indicator, kline::ClusterKind,
};
use data::util::format_with_commas;
use exchange::{TickMultiplier, Ticker};
use exchange::{
    Timeframe,
    adapter::{Exchange, MarketKind},
};
use iced::alignment::Vertical;
use iced::widget::{horizontal_rule, horizontal_space, radio};
use iced::{
    Alignment, Element, Length,
    alignment::Horizontal,
    padding,
    widget::{
        Slider, button, center, column, container, pane_grid, pick_list, row, scrollable, text,
        tooltip::Position as TooltipPosition,
    },
};
use timeandsales::TimeAndSales;

pub trait PanelView {
    fn view(
        &self,
        pane: pane_grid::Pane,
        state: &pane::State,
        timezone: data::UserTimezone,
    ) -> Element<Message>;
}

impl PanelView for TimeAndSales {
    fn view(
        &self,
        pane: pane_grid::Pane,
        state: &pane::State,
        timezone: data::UserTimezone,
    ) -> Element<Message> {
        let underlay = self.view(timezone);

        let settings_view = super::panel::timesales_cfg_view(self.get_config(), pane);

        match state.modal {
            Some(pane::Modal::Settings) => pane_modal(
                underlay,
                settings_view,
                Message::ToggleModal(pane, pane::Modal::Settings),
                padding::right(12).left(12),
                Alignment::End,
            ),
            _ => underlay,
        }
    }
}

pub fn view<'a, C: PanelView>(
    pane: pane_grid::Pane,
    state: &'a pane::State,
    content: &'a C,
    timezone: data::UserTimezone,
) -> Element<'a, Message> {
    let base = center(content.view(pane, state, timezone));

    widget::toast::Manager::new(base, &state.notifications, Alignment::End, move |idx| {
        Message::DeleteNotification(pane, idx)
    })
    .into()
}

pub fn heatmap_cfg_view<'a>(cfg: heatmap::Config, pane: pane_grid::Pane) -> Element<'a, Message> {
    let trade_size_slider = {
        let filter = cfg.trade_size_filter;

        create_slider_row(
            text("Trade"),
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
            text("Order"),
            Slider::new(0.0..=500_000.0, filter, move |value| {
                Message::VisualConfigChanged(
                    Some(pane),
                    VisualConfig::Heatmap(heatmap::Config {
                        order_size_filter: value,
                        ..cfg
                    }),
                )
            })
            .step(5000.0)
            .into(),
            Some(text(format!("${}", format_with_commas(filter))).size(13)),
        )
    };

    let circle_scaling_slider = {
        if let Some(radius_scale) = cfg.trade_size_scale {
            create_slider_row(
                text("Circle radius scaling"),
                Slider::new(10..=200, radius_scale, move |value| {
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

            let threshold_slider = create_slider_row(
                text("Size similarity"),
                Slider::new(0.05..=0.8, threshold_pct, move |value| {
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

    container(scrollable_content(
        column![
            column![
                text("Size filters").size(14),
                row![trade_size_slider, order_size_slider].spacing(8),
            ]
            .spacing(20)
            .padding(16)
            .align_x(Alignment::Start),
            horizontal_rule(1),
            column![
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
            .spacing(20)
            .padding(16)
            .align_x(Alignment::Start),
            horizontal_rule(1),
            column![
                text("Trade visualization").size(14),
                iced::widget::checkbox("Dynamic circle radius", cfg.trade_size_scale.is_some(),)
                    .on_toggle(move |value| {
                        Message::VisualConfigChanged(
                            Some(pane),
                            VisualConfig::Heatmap(heatmap::Config {
                                trade_size_scale: if value { Some(100) } else { None },
                                ..cfg
                            }),
                        )
                    }),
                circle_scaling_slider,
            ]
            .spacing(20)
            .padding(16)
            .width(Length::Fill)
            .align_x(Alignment::Start),
            horizontal_rule(1),
            row![
                horizontal_space(),
                sync_all_button(VisualConfig::Heatmap(cfg))
            ],
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

pub fn indicators_view<I: Indicator>(
    pane: pane_grid::Pane,
    market_type: Option<MarketKind>,
    selected: &[I],
) -> Element<Message> {
    let mut indicators_column = column_drag::Column::new()
        .on_drag(move |event| Message::ReorderIndicator(pane, event))
        .spacing(4);

    if let Some(market) = market_type {
        for indicator in selected {
            let indicator_row = button(
                row![
                    text(indicator.to_string()),
                    horizontal_space(),
                    container(icon_text(Icon::Checkmark, 12)),
                ]
                .width(Length::Fill),
            )
            .on_press(Message::ToggleIndicator(pane, indicator.to_string()))
            .style(move |theme, status| style::button::modifier(theme, status, true));

            indicators_column = indicators_column.push(dragger_row(indicator_row.into()));
        }

        for indicator in I::get_available(market) {
            if !selected.contains(indicator) {
                let indicator_row = button(text(indicator.to_string()))
                    .on_press(Message::ToggleIndicator(pane, indicator.to_string()))
                    .width(Length::Fill)
                    .style(move |theme, status| style::button::modifier(theme, status, false));

                indicators_column = indicators_column.push(dragger_row(indicator_row.into()));
            }
        }
    }

    let content_row = column![
        container(text("Indicators").size(14)).padding(padding::bottom(8)),
        indicators_column
    ]
    .spacing(4);

    container(content_row)
        .max_width(200)
        .padding(16)
        .style(style::chart_modal)
        .into()
}

#[derive(Debug, Clone, Copy)]
pub enum StreamModifier {
    Candlestick(Basis),
    Footprint(Basis, TickMultiplier),
    Heatmap(Basis, TickMultiplier),
}

pub fn stream_modifier_view<'a>(
    pane: pane_grid::Pane,
    modifiers: StreamModifier,
    ticker_info: Option<(Exchange, Ticker)>,
) -> Element<'a, Message> {
    let (selected_basis, selected_ticksize) = match modifiers {
        StreamModifier::Candlestick(basis) => (Some(basis), None),
        StreamModifier::Footprint(basis, ticksize) | StreamModifier::Heatmap(basis, ticksize) => {
            (Some(basis), Some(ticksize))
        }
    };

    let create_button = |content: String, msg: Option<Message>, active: bool| {
        let btn = button(container(text(content)).align_x(Horizontal::Center))
            .width(Length::Fill)
            .style(move |theme, status| style::button::transparent(theme, status, active));

        if let Some(msg) = msg {
            btn.on_press(msg)
        } else {
            btn
        }
    };

    let mut content_row = row![].align_y(Vertical::Center).spacing(16);

    let mut timeframes_column = column![].padding(4).align_x(Horizontal::Center);
    let mut tick_basis_column = column![].padding(4).align_x(Horizontal::Center);

    let is_kline_chart = match modifiers {
        StreamModifier::Candlestick(_) | StreamModifier::Footprint(_, _) => true,
        StreamModifier::Heatmap(_, _) => false,
    };

    if let Some(basis) = selected_basis {
        match basis {
            Basis::Time(selected_timeframe) => {
                timeframes_column = timeframes_column.push(if is_kline_chart {
                    row![
                        create_button("Timeframe".to_string(), None, false,),
                        create_button(
                            "Ticks".to_string(),
                            Some(Message::BasisSelected(Basis::Tick(200), pane,)),
                            true,
                        ),
                    ]
                    .padding(padding::bottom(8))
                    .spacing(4)
                } else {
                    row![text("Aggregation")]
                        .padding(padding::bottom(8))
                        .spacing(4)
                });

                if is_kline_chart {
                    for timeframe in &Timeframe::KLINE {
                        let msg = if *timeframe == selected_timeframe.into() {
                            None
                        } else {
                            Some(Message::BasisSelected(
                                Basis::Time(u64::from(*timeframe)),
                                pane,
                            ))
                        };
                        timeframes_column = timeframes_column.push(create_button(
                            timeframe.to_string(),
                            msg,
                            false,
                        ));
                    }
                } else if let Some((exchange, _)) = ticker_info {
                    for timeframe in &Timeframe::HEATMAP {
                        if exchange == Exchange::BybitSpot && timeframe == &Timeframe::MS100 {
                            continue;
                        }

                        let msg = if *timeframe == selected_timeframe.into() {
                            None
                        } else {
                            Some(Message::BasisSelected(
                                Basis::Time(u64::from(*timeframe)),
                                pane,
                            ))
                        };
                        timeframes_column = timeframes_column.push(create_button(
                            timeframe.to_string(),
                            msg,
                            false,
                        ));
                    }
                }

                content_row =
                    content_row.push(container(timeframes_column).style(style::modal_container));
            }
            Basis::Tick(selected_tick) => {
                tick_basis_column = tick_basis_column.push(
                    row![
                        create_button(
                            "Timeframe".to_string(),
                            Some(Message::BasisSelected(
                                Basis::Time(Timeframe::M5.into()),
                                pane
                            )),
                            true,
                        ),
                        create_button("Ticks".to_string(), None, false,),
                    ]
                    .padding(padding::bottom(8))
                    .spacing(4),
                );

                for tick_count in &data::aggr::TickCount::ALL {
                    let msg = if *tick_count == selected_tick.into() {
                        None
                    } else {
                        Some(Message::BasisSelected(
                            Basis::Tick(u64::from(*tick_count)),
                            pane,
                        ))
                    };
                    tick_basis_column =
                        tick_basis_column.push(create_button(tick_count.to_string(), msg, false));
                }

                content_row =
                    content_row.push(container(tick_basis_column).style(style::modal_container));
            }
        }
    }

    let mut ticksizes_column = column![].padding(4).align_x(Horizontal::Center);

    if selected_ticksize.is_some() {
        ticksizes_column =
            ticksizes_column.push(container(text("Ticksize Mltp.")).padding(padding::bottom(8)));

        for ticksize in &exchange::TickMultiplier::ALL {
            let msg = if selected_ticksize == Some(*ticksize) {
                None
            } else {
                Some(Message::TicksizeSelected(*ticksize, pane))
            };
            ticksizes_column =
                ticksizes_column.push(create_button(ticksize.to_string(), msg, false));
        }

        content_row = content_row.push(container(ticksizes_column).style(style::modal_container));
    }

    container(scrollable::Scrollable::with_direction(
        content_row.align_y(Alignment::Start),
        scrollable::Direction::Vertical(scrollable::Scrollbar::new().width(4).scroller_width(4)),
    ))
    .padding(16)
    .max_width(if selected_ticksize.is_some() && selected_basis.is_some() {
        380
    } else if selected_basis.is_some() {
        200
    } else {
        120
    })
    .style(style::chart_modal)
    .into()
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
