use crate::{
    theme::ThemeAccents,
    widget::{
        classic_slider_row,
        inspector::{self, InspectorSection},
    },
};
use data::chart::kline::{
    KlineStudy, MAX_TPO_VALUE_AREA_PERCENT, MAX_VOLUME_PROFILE_OPACITY_PERCENT,
    MAX_VOLUME_PROFILE_WIDTH_PERCENT, MIN_TPO_VALUE_AREA_PERCENT,
    MIN_VOLUME_PROFILE_OPACITY_PERCENT, MIN_VOLUME_PROFILE_WIDTH_PERCENT, StudyDataSource,
    TpoAnchor, TpoColorConfig, VolumeProfileColorConfig, VolumeProfileSchedule,
    big_trades_step_count, big_trades_step_for_threshold, big_trades_threshold_at_step,
    big_trades_threshold_label,
};
use data::config::theme::ThemeColor;
use exchange::Timeframe;
use iced::{
    Element, Length,
    widget::{checkbox, column, container, pick_list, radio, text},
};

pub fn view(
    study: KlineStudy,
    basis: data::chart::Basis,
    theme_accents: ThemeAccents,
    tab: inspector::InspectorTab,
) -> Option<Element<'static, KlineStudy>> {
    if tab == inspector::InspectorTab::Source
        || (tab == inspector::InspectorTab::Inputs
            && matches!(
                study,
                KlineStudy::VolumeProfile {
                    schedule: VolumeProfileSchedule::VisibleRange,
                    ..
                }
            ))
    {
        return None;
    }

    Some(kline_study_config_view(&study, basis, theme_accents, tab))
}

fn study_setting<'a, M>(
    label: impl iced::widget::text::IntoFragment<'a>,
    setting: impl Into<Element<'a, M>>,
) -> Element<'a, M>
where
    M: 'a,
{
    study_setting_with_title(crate::widget::text::body(label), setting)
}

fn study_setting_with_title<'a, M>(
    title: impl Into<Element<'a, M>>,
    setting: impl Into<Element<'a, M>>,
) -> Element<'a, M>
where
    M: 'a,
{
    let title: Element<'a, M> = title.into();
    let setting: Element<'a, M> = setting.into();
    InspectorSection::new(title, container(setting).width(Length::Fill))
        .with_modal_container()
        .into_element()
}

fn volume_profile_color_section<'a>(
    colors: VolumeProfileColorConfig,
    source: StudyDataSource,
    schedule: VolumeProfileSchedule,
    theme_accents: ThemeAccents,
    on_change: impl Fn(VolumeProfileColorConfig) -> KlineStudy + Copy + 'a,
) -> Element<'a, KlineStudy> {
    let controls = VolumeProfileColorConfig::color_specs_for_source(source, schedule)
        .iter()
        .filter_map(|spec| {
            let slot = spec.slot;
            let enabled = match slot {
                data::chart::kline::VolumeProfileColorSlot::NakedPoc => {
                    colors.show_poc || colors.show_npoc
                }
                data::chart::kline::VolumeProfileColorSlot::FilledPoc => colors.show_poc,
                _ => true,
            };

            if !enabled {
                return None;
            }

            let selected = colors.color_for(slot);
            let setting = study_color_setting(selected, theme_accents, move |color| {
                on_change(colors.with_color(slot, color))
            });

            Some(study_setting(spec.label, setting))
        });

    let mut sections = column![].spacing(inspector::MODAL_SECTION_SPACING);
    if matches!(schedule, VolumeProfileSchedule::Recurring { .. }) {
        sections = sections.push(crate::widget::inspector::modal_container(
            poc_line_controls(colors.show_poc, colors.show_npoc, true, move |all, show| {
                on_change(colors.with_show_poc(all).with_show_npoc(show))
            }),
        ));
    }
    sections.extend(controls).into()
}

pub fn poc_line_controls<'a, M: Clone + 'a>(
    show_poc: bool,
    show_npoc: bool,
    enabled: bool,
    on_change: impl Fn(bool, bool) -> M + Copy + 'a,
) -> Element<'a, M> {
    let shown = show_poc || show_npoc;
    let toggle = checkbox(shown)
        .label("Show POC lines")
        .spacing(8)
        .text_size(crate::style::text_size::SECTION)
        .on_toggle_maybe(enabled.then_some(move |show| on_change(show, show)))
        .width(Length::Fill);
    let mut controls = column![toggle].spacing(8);
    if shown && enabled {
        controls = controls.push(
            container(
                column![
                    radio("All", true, Some(show_poc), move |all| on_change(all, true)),
                    radio("Untouched only", false, Some(show_poc), move |all| {
                        on_change(all, true)
                    }),
                ]
                .spacing(8),
            )
            .padding(iced::Padding::default().left(24)),
        );
    }
    controls.into()
}

fn volume_profile_width_section<'a>(
    width_percent: u8,
    reserved_area: &'static str,
    on_change: impl Fn(u8) -> KlineStudy + Copy + 'a,
) -> Element<'a, KlineStudy> {
    let width_percent = width_percent.clamp(
        MIN_VOLUME_PROFILE_WIDTH_PERCENT,
        MAX_VOLUME_PROFILE_WIDTH_PERCENT,
    );
    let control = classic_slider_row(
        None,
        f32::from(MIN_VOLUME_PROFILE_WIDTH_PERCENT)..=f32::from(MAX_VOLUME_PROFILE_WIDTH_PERCENT),
        f32::from(width_percent),
        move |new_width| {
            on_change(new_width.round().clamp(
                f32::from(MIN_VOLUME_PROFILE_WIDTH_PERCENT),
                f32::from(MAX_VOLUME_PROFILE_WIDTH_PERCENT),
            ) as u8)
        },
        move |value| format!("{value:.0}% {reserved_area}"),
        Some(5.0),
    );

    study_setting("Width", control)
}

fn volume_profile_opacity_section<'a>(
    opacity_percent: u8,
    on_change: impl Fn(u8) -> KlineStudy + Copy + 'a,
) -> Element<'a, KlineStudy> {
    let opacity_percent = opacity_percent.clamp(
        MIN_VOLUME_PROFILE_OPACITY_PERCENT,
        MAX_VOLUME_PROFILE_OPACITY_PERCENT,
    );
    let control = classic_slider_row(
        None,
        f32::from(MIN_VOLUME_PROFILE_OPACITY_PERCENT)
            ..=f32::from(MAX_VOLUME_PROFILE_OPACITY_PERCENT),
        f32::from(opacity_percent),
        move |new_opacity| {
            on_change(new_opacity.round().clamp(
                f32::from(MIN_VOLUME_PROFILE_OPACITY_PERCENT),
                f32::from(MAX_VOLUME_PROFILE_OPACITY_PERCENT),
            ) as u8)
        },
        |value| format!("{value:.0}%"),
        Some(5.0),
    );

    study_setting("Opacity", control)
}

fn big_trades_color_settings<'a>(
    buy_color: ThemeColor,
    sell_color: ThemeColor,
    theme_accents: ThemeAccents,
    on_change: impl Fn(ThemeColor, ThemeColor) -> KlineStudy + Copy + 'a,
) -> Element<'a, KlineStudy> {
    column![
        study_setting(
            "Buy bubbles",
            study_color_setting(buy_color, theme_accents, move |color| {
                on_change(color, sell_color)
            }),
        ),
        study_setting(
            "Sell bubbles",
            study_color_setting(sell_color, theme_accents, move |color| {
                on_change(buy_color, color)
            }),
        ),
    ]
    .spacing(inspector::MODAL_SECTION_SPACING)
    .into()
}

fn big_trades_threshold_slider<'a>(
    threshold: u64,
    radius_scale: Option<i32>,
    buy_color: ThemeColor,
    sell_color: ThemeColor,
    visible: bool,
) -> Element<'a, KlineStudy> {
    classic_slider_row(
        Some(text("Size filter")),
        0..=big_trades_step_count().saturating_sub(1) as u32,
        big_trades_step_for_threshold(threshold) as u32,
        move |step| KlineStudy::BigTrades {
            threshold: big_trades_threshold_at_step(step as usize),
            radius_scale,
            buy_color,
            sell_color,
            visible,
        },
        |step| big_trades_threshold_label(big_trades_threshold_at_step(*step as usize)),
        Some(1),
    )
}

fn kline_study_config_view<'a>(
    study: &KlineStudy,
    basis: data::chart::Basis,
    theme_accents: ThemeAccents,
    tab: inspector::InspectorTab,
) -> Element<'a, KlineStudy> {
    match study {
        KlineStudy::VolumeProfile {
            source,
            schedule,
            colors,
            width_percent,
            opacity_percent,
            visible,
        } => {
            let schedule = *schedule;
            let source = *source;
            let colors = *colors;
            let width_percent = *width_percent;
            let opacity_percent = *opacity_percent;
            let visible = *visible;

            if matches!(schedule, VolumeProfileSchedule::Recurring { .. }) && !basis.is_time() {
                return iced::widget::center(text(
                    "Recurring volume profiles require a time-based chart",
                ))
                .into();
            }

            match tab {
                inspector::InspectorTab::Inputs => {
                    let VolumeProfileSchedule::Recurring { interval } = schedule else {
                        return column![].into();
                    };
                    let intervals = VolumeProfileSchedule::recurring_intervals_for(basis);
                    let interval_control: Element<'_, KlineStudy> = if intervals.is_empty() {
                        text("No recurring interval is at least 4x the chart timeframe").into()
                    } else {
                        let selected_interval = intervals.contains(&interval).then_some(interval);
                        pick_list(intervals, selected_interval, move |new_interval| {
                            KlineStudy::VolumeProfile {
                                source,
                                schedule: VolumeProfileSchedule::Recurring {
                                    interval: new_interval,
                                },
                                colors,
                                width_percent,
                                opacity_percent,
                                visible,
                            }
                        })
                        .width(Length::Fill)
                        .into()
                    };

                    column![study_setting("Interval", interval_control)]
                        .spacing(inspector::MODAL_SECTION_SPACING)
                        .into()
                }
                inspector::InspectorTab::Style => {
                    let reserved_area = match schedule {
                        VolumeProfileSchedule::VisibleRange => "of the chart width",
                        VolumeProfileSchedule::Recurring { .. } => "of each profile interval",
                    };
                    let width_section = volume_profile_width_section(
                        width_percent,
                        reserved_area,
                        move |width_percent| KlineStudy::VolumeProfile {
                            source,
                            schedule,
                            colors,
                            width_percent,
                            opacity_percent,
                            visible,
                        },
                    );
                    let opacity_section =
                        volume_profile_opacity_section(opacity_percent, move |opacity_percent| {
                            KlineStudy::VolumeProfile {
                                source,
                                schedule,
                                colors,
                                width_percent,
                                opacity_percent,
                                visible,
                            }
                        });
                    let color_section = volume_profile_color_section(
                        colors,
                        source,
                        schedule,
                        theme_accents,
                        move |colors| KlineStudy::VolumeProfile {
                            source,
                            schedule,
                            colors,
                            width_percent,
                            opacity_percent,
                            visible,
                        },
                    );

                    column![width_section, opacity_section, color_section]
                        .spacing(inspector::MODAL_SECTION_SPACING)
                        .into()
                }
                inspector::InspectorTab::Source => unreachable!(),
            }
        }
        KlineStudy::BigTrades {
            threshold,
            radius_scale,
            buy_color,
            sell_color,
            visible,
        } => {
            let threshold = *threshold;
            let radius_scale = *radius_scale;
            let buy_color = *buy_color;
            let sell_color = *sell_color;
            let visible = *visible;
            match tab {
                inspector::InspectorTab::Inputs => {
                    let slider = big_trades_threshold_slider(
                        threshold,
                        radius_scale,
                        buy_color,
                        sell_color,
                        visible,
                    );
                    let on_radius_change = move |radius_scale| KlineStudy::BigTrades {
                        threshold,
                        radius_scale,
                        buy_color,
                        sell_color,
                        visible,
                    };
                    let mut radius_controls = column![
                        checkbox(radius_scale.is_some())
                            .label("Dynamic circle radius")
                            .on_toggle(move |dynamic| on_radius_change(dynamic.then_some(100))),
                    ]
                    .spacing(8);
                    if let Some(scale) = radius_scale {
                        radius_controls = radius_controls.push(classic_slider_row(
                            Some(crate::widget::text::body("Circle radius scaling")),
                            10..=200,
                            scale,
                            move |value| on_radius_change(Some(value)),
                            |value| format!("{}%", value),
                            Some(10),
                        ));
                    }
                    let radius_section = study_setting("Visualization", radius_controls);

                    column![slider, radius_section]
                        .spacing(inspector::MODAL_SECTION_SPACING)
                        .into()
                }
                inspector::InspectorTab::Style => big_trades_color_settings(
                    buy_color,
                    sell_color,
                    theme_accents,
                    move |buy_color, sell_color| KlineStudy::BigTrades {
                        threshold,
                        radius_scale,
                        buy_color,
                        sell_color,
                        visible,
                    },
                ),
                inspector::InspectorTab::Source => column![].into(),
            }
        }
        KlineStudy::TpoProfile { .. } => tpo_study_config_view(*study, basis, theme_accents, tab),
    }
}

fn tpo_study_config_view<'a>(
    study: KlineStudy,
    basis: data::chart::Basis,
    theme_accents: ThemeAccents,
    tab: inspector::InspectorTab,
) -> Element<'a, KlineStudy> {
    let KlineStudy::TpoProfile {
        source,
        anchor,
        letter_period,
        value_area_percent,
        colors,
        visible,
    } = study
    else {
        unreachable!();
    };

    if !basis.is_time() {
        return iced::widget::center(text("TPO profiles require a time-based chart")).into();
    }

    match tab {
        inspector::InspectorTab::Inputs => {
            let anchor_picker = pick_list(TpoAnchor::ALL, Some(anchor), move |anchor| {
                KlineStudy::TpoProfile {
                    source,
                    anchor,
                    letter_period,
                    value_area_percent,
                    colors,
                    visible,
                }
            })
            .width(Length::Fill);

            let periods = [
                Timeframe::M15,
                Timeframe::M30,
                Timeframe::H1,
                Timeframe::H2,
                Timeframe::H4,
            ];
            let period_picker = pick_list(periods, Some(letter_period), move |period| {
                KlineStudy::TpoProfile {
                    source,
                    anchor,
                    letter_period: period,
                    value_area_percent,
                    colors,
                    visible,
                }
            })
            .width(Length::Fill);

            let value_area_target = classic_slider_row(
                Some(text("% of TPOs")),
                f32::from(MIN_TPO_VALUE_AREA_PERCENT)..=f32::from(MAX_TPO_VALUE_AREA_PERCENT),
                f32::from(
                    value_area_percent
                        .clamp(MIN_TPO_VALUE_AREA_PERCENT, MAX_TPO_VALUE_AREA_PERCENT),
                ),
                move |value| KlineStudy::TpoProfile {
                    source,
                    anchor,
                    letter_period,
                    value_area_percent: (value.round() as u8)
                        .clamp(MIN_TPO_VALUE_AREA_PERCENT, MAX_TPO_VALUE_AREA_PERCENT),
                    colors,
                    visible,
                },
                |value| format!("{value:.0}%"),
                Some(5.0),
            );

            column![
                study_setting("Anchor (UTC)", anchor_picker),
                study_setting("Bracket duration", period_picker),
                study_setting("Value area target", value_area_target),
            ]
            .spacing(inspector::MODAL_SECTION_SPACING)
            .into()
        }
        inspector::InspectorTab::Style => {
            let color_controls = [
                (TpoColorSlot::ValueAreaCells, "Value area"),
                (TpoColorSlot::OutsideValueArea, "Outside value area"),
                (TpoColorSlot::Poc, "POC"),
                (TpoColorSlot::InitialBalance, "Initial balance"),
                (TpoColorSlot::SinglePrint, "Single print"),
                (TpoColorSlot::PoorExtremes, "Poor high/low"),
            ]
            .into_iter()
            .map(|(slot, label)| {
                let enabled = match slot {
                    TpoColorSlot::OutsideValueArea | TpoColorSlot::ValueAreaCells => true,
                    TpoColorSlot::Poc => colors.show_poc,
                    TpoColorSlot::InitialBalance => colors.show_initial_balance,
                    TpoColorSlot::SinglePrint => colors.show_single_print,
                    TpoColorSlot::PoorExtremes => colors.show_poor_high || colors.show_poor_low,
                };
                if !enabled {
                    return crate::widget::inspector::modal_container(tpo_visibility_toggle(
                        study, label, slot,
                    ));
                }

                let selected = tpo_color_for(colors, slot);
                let setting = study_color_setting(selected, theme_accents, move |color| {
                    KlineStudy::TpoProfile {
                        source,
                        anchor,
                        letter_period,
                        value_area_percent,
                        colors: tpo_color_with(colors, slot, color),
                        visible,
                    }
                });
                let title = match slot {
                    TpoColorSlot::OutsideValueArea | TpoColorSlot::ValueAreaCells => {
                        crate::widget::text::body(label).into()
                    }
                    TpoColorSlot::Poc
                    | TpoColorSlot::InitialBalance
                    | TpoColorSlot::SinglePrint
                    | TpoColorSlot::PoorExtremes => tpo_visibility_toggle(study, label, slot),
                };
                study_setting_with_title(title, setting)
            })
            .collect::<Vec<_>>();

            column![column(color_controls).spacing(8)]
                .spacing(inspector::MODAL_SECTION_SPACING)
                .into()
        }
        inspector::InspectorTab::Source => column![].into(),
    }
}

#[derive(Debug, Clone, Copy)]
enum TpoColorSlot {
    OutsideValueArea,
    Poc,
    ValueAreaCells,
    InitialBalance,
    SinglePrint,
    PoorExtremes,
}

fn tpo_color_for(colors: TpoColorConfig, slot: TpoColorSlot) -> data::config::theme::ThemeColor {
    match slot {
        TpoColorSlot::OutsideValueArea => colors.letters,
        TpoColorSlot::Poc => colors.poc,
        TpoColorSlot::ValueAreaCells => colors.value_area,
        TpoColorSlot::InitialBalance => colors.initial_balance,
        TpoColorSlot::SinglePrint => colors.single_print,
        TpoColorSlot::PoorExtremes => colors.poor_high,
    }
}

fn tpo_color_with(
    colors: TpoColorConfig,
    slot: TpoColorSlot,
    color: data::config::theme::ThemeColor,
) -> TpoColorConfig {
    match slot {
        TpoColorSlot::OutsideValueArea => TpoColorConfig {
            letters: color,
            ..colors
        },
        TpoColorSlot::Poc => TpoColorConfig {
            poc: color,
            ..colors
        },
        TpoColorSlot::ValueAreaCells => TpoColorConfig {
            value_area: color,
            ..colors
        },
        TpoColorSlot::InitialBalance => TpoColorConfig {
            initial_balance: color,
            ..colors
        },
        TpoColorSlot::SinglePrint => TpoColorConfig {
            single_print: color,
            ..colors
        },
        TpoColorSlot::PoorExtremes => TpoColorConfig {
            poor_high: color,
            poor_low: color,
            ..colors
        },
    }
}

fn tpo_visibility_toggle<'a>(
    study: KlineStudy,
    label: &'static str,
    slot: TpoColorSlot,
) -> Element<'a, KlineStudy> {
    let KlineStudy::TpoProfile { colors, .. } = study else {
        unreachable!();
    };

    let enabled = match slot {
        TpoColorSlot::Poc => colors.show_poc,
        TpoColorSlot::InitialBalance => colors.show_initial_balance,
        TpoColorSlot::SinglePrint => colors.show_single_print,
        TpoColorSlot::PoorExtremes => colors.show_poor_high || colors.show_poor_low,
        TpoColorSlot::OutsideValueArea | TpoColorSlot::ValueAreaCells => unreachable!(),
    };

    checkbox(enabled)
        .label(label)
        .spacing(8)
        .text_size(crate::style::text_size::SECTION)
        .on_toggle(move |enabled| {
            let mut study = study;
            let KlineStudy::TpoProfile { colors, .. } = &mut study else {
                unreachable!();
            };

            *colors = match slot {
                TpoColorSlot::Poc => colors.with_show_poc(enabled),
                TpoColorSlot::InitialBalance => colors.with_show_initial_balance(enabled),
                TpoColorSlot::SinglePrint => colors.with_show_single_print(enabled),
                TpoColorSlot::PoorExtremes => colors.with_show_poor_extremes(enabled),
                TpoColorSlot::OutsideValueArea | TpoColorSlot::ValueAreaCells => unreachable!(),
            };

            study
        })
        .width(Length::Fill)
        .into()
}

fn study_color_setting<'a>(
    color: data::config::theme::ThemeColor,
    theme_accents: ThemeAccents,
    on_change: impl Fn(data::config::theme::ThemeColor) -> KlineStudy + Copy + 'a,
) -> Element<'a, KlineStudy> {
    let selected = color;
    let accents = crate::widget::theme_color_control(selected, theme_accents, move |accent| {
        on_change(data::config::theme::ThemeColor::with_accent(
            selected.role,
            accent,
            selected.shade,
        ))
    });
    let shades = pick_list(
        data::config::theme::ThemeColorShade::ALL,
        Some(selected.shade),
        move |shade| {
            on_change(data::config::theme::ThemeColor::with_accent(
                selected.role,
                selected.accent,
                shade,
            ))
        },
    )
    .width(Length::Fill);
    column![accents, shades].spacing(4).into()
}
