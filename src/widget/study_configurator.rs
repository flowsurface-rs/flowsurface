use crate::{
    style,
    theme::ThemeAccents,
    widget::{
        classic_slider_row,
        inspector::{self, InspectorSection},
    },
};
use data::chart::heatmap::{CLEANUP_THRESHOLD, HeatmapStudy, ProfileKind};
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
    widget::{button, checkbox, column, container, pick_list, radio, row, rule, space, text},
};

#[derive(Debug, Clone, Copy)]
pub enum StudyMessage {
    Heatmap(Message<HeatmapStudy>),
}

pub trait Study: Sized + Copy + 'static {
    fn is_same_type(&self, other: &Self) -> bool;
    fn all() -> Vec<Self>;
    fn view_config<'a>(
        &self,
        basis: data::chart::Basis,
        theme_accents: ThemeAccents,
        on_change: impl Fn(Self) -> Message<Self> + Copy + 'a,
    ) -> Element<'a, Message<Self>>;

    fn view_config_without_reset<'a>(
        &self,
        basis: data::chart::Basis,
        theme_accents: ThemeAccents,
        tab: inspector::InspectorTab,
        on_change: impl Fn(Self) -> Message<Self> + Copy + 'a,
    ) -> Option<Element<'a, Message<Self>>> {
        (tab == inspector::InspectorTab::Inputs)
            .then(|| self.view_config(basis, theme_accents, on_change))
    }
}

#[derive(Debug, Clone, Copy)]
enum StudyConfigLayout {
    Card,
    Inspector(inspector::InspectorTab),
}

fn study_setting<'a, M>(
    label: impl iced::widget::text::IntoFragment<'a>,
    setting: impl Into<Element<'a, M>>,
    layout: StudyConfigLayout,
) -> Element<'a, M>
where
    M: 'a,
{
    study_setting_with_title(crate::widget::text::body(label), setting, layout)
}

fn study_setting_with_title<'a, M>(
    title: impl Into<Element<'a, M>>,
    setting: impl Into<Element<'a, M>>,
    layout: StudyConfigLayout,
) -> Element<'a, M>
where
    M: 'a,
{
    let title: Element<'a, M> = title.into();
    let setting: Element<'a, M> = setting.into();
    let section = match layout {
        StudyConfigLayout::Card => {
            InspectorSection::new(title, container(setting).width(Length::Fill))
        }
        StudyConfigLayout::Inspector(_) => {
            InspectorSection::new(title, container(setting).width(Length::Fill))
                .with_modal_container()
        }
    };

    section.into_element()
}

fn element_for_layout<'a, M>(
    content: impl Into<Element<'a, M>>,
    layout: StudyConfigLayout,
) -> Element<'a, M>
where
    M: 'a,
{
    match layout {
        StudyConfigLayout::Card => content.into(),
        StudyConfigLayout::Inspector(_) => crate::widget::inspector::modal_container(content),
    }
}

fn study_window_duration(datapoint_count: usize, interval_ms: u64) -> String {
    let duration_secs = (datapoint_count as u64 * interval_ms) / 1000;
    if duration_secs < 60 {
        format!("{} seconds", duration_secs)
    } else {
        let minutes = duration_secs / 60;
        let seconds = duration_secs % 60;
        if seconds == 0 {
            format!("{} minutes", minutes)
        } else {
            format!("{}m {}s", minutes, seconds)
        }
    }
}

fn volume_profile_color_section<'a>(
    colors: VolumeProfileColorConfig,
    source: StudyDataSource,
    schedule: VolumeProfileSchedule,
    theme_accents: ThemeAccents,
    layout: StudyConfigLayout,
    on_change: impl Fn(VolumeProfileColorConfig) -> Message<KlineStudy> + Copy + 'a,
) -> Element<'a, Message<KlineStudy>> {
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
            let color_choices =
                crate::widget::theme_color_control(selected, theme_accents, send_color);
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
            )
            .width(Length::Fill);

            let setting = column![color_choices, shade_picklist]
                .spacing(4)
                .width(Length::Fill);

            Some(study_setting(spec.label, setting, layout))
        });

    let spacing = match layout {
        StudyConfigLayout::Card => 16.0,
        StudyConfigLayout::Inspector(_) => inspector::MODAL_SECTION_SPACING,
    };
    let mut sections = column![].spacing(spacing);
    if matches!(schedule, VolumeProfileSchedule::Recurring { .. }) {
        sections = sections.push(element_for_layout(
            poc_line_controls(colors.show_poc, colors.show_npoc, true, move |all, show| {
                on_change(colors.with_show_poc(all).with_show_npoc(show))
            }),
            layout,
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
    layout: StudyConfigLayout,
    on_change: impl Fn(u8) -> Message<KlineStudy> + Copy + 'a,
) -> Element<'a, Message<KlineStudy>> {
    let width_percent = width_percent.clamp(
        MIN_VOLUME_PROFILE_WIDTH_PERCENT,
        MAX_VOLUME_PROFILE_WIDTH_PERCENT,
    );
    let control = classic_slider_row(
        match layout {
            StudyConfigLayout::Card => Some(text("Width")),
            StudyConfigLayout::Inspector(_) => None,
        },
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

    match layout {
        StudyConfigLayout::Card => control,
        StudyConfigLayout::Inspector(_) => study_setting("Width", control, layout),
    }
}

fn volume_profile_opacity_section<'a>(
    opacity_percent: u8,
    layout: StudyConfigLayout,
    on_change: impl Fn(u8) -> Message<KlineStudy> + Copy + 'a,
) -> Element<'a, Message<KlineStudy>> {
    let opacity_percent = opacity_percent.clamp(
        MIN_VOLUME_PROFILE_OPACITY_PERCENT,
        MAX_VOLUME_PROFILE_OPACITY_PERCENT,
    );
    let control = classic_slider_row(
        match layout {
            StudyConfigLayout::Card => Some(text("Opacity")),
            StudyConfigLayout::Inspector(_) => None,
        },
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

    match layout {
        StudyConfigLayout::Card => control,
        StudyConfigLayout::Inspector(_) => study_setting("Opacity", control, layout),
    }
}

fn volume_profile_settings_sections<'a>(
    schedule_section: Option<Element<'a, Message<KlineStudy>>>,
    width_section: Element<'a, Message<KlineStudy>>,
    opacity_section: Element<'a, Message<KlineStudy>>,
    color_section: Element<'a, Message<KlineStudy>>,
    layout: StudyConfigLayout,
) -> Element<'a, Message<KlineStudy>> {
    match layout {
        StudyConfigLayout::Card => {
            let width_opacity_section = column![width_section, opacity_section].spacing(8);
            let content = if let Some(schedule) = schedule_section {
                let schedule_row = container(schedule).width(Length::Fill);

                column![
                    schedule_row,
                    rule::horizontal(1.0).style(style::split_ruler),
                    width_opacity_section,
                    rule::horizontal(1.0).style(style::split_ruler),
                    color_section,
                ]
                .spacing(16)
            } else {
                column![
                    width_opacity_section,
                    rule::horizontal(1.0).style(style::split_ruler),
                    color_section,
                ]
                .spacing(16)
            };

            content.into()
        }
        StudyConfigLayout::Inspector(tab) => {
            let settings = match tab {
                inspector::InspectorTab::Inputs => {
                    let mut inputs = column![].spacing(inspector::MODAL_SECTION_SPACING);
                    if let Some(schedule) = schedule_section {
                        inputs = inputs.push(schedule);
                    }
                    inputs
                }
                inspector::InspectorTab::Style => {
                    column![width_section, opacity_section, color_section,]
                        .spacing(inspector::MODAL_SECTION_SPACING)
                }
                inspector::InspectorTab::Source => unreachable!(),
            };

            settings.into()
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
        _theme_accents: ThemeAccents,
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
            HeatmapStudy::VolumeProfile(kind) => {
                let fixed_window = match kind {
                    ProfileKind::FixedWindow(_) => *kind,
                    ProfileKind::VisibleRange => ProfileKind::FixedWindow(CLEANUP_THRESHOLD / 5),
                };
                let fixed_window_label = match kind {
                    ProfileKind::FixedWindow(datapoint_count) => format!(
                        "Fixed window ({})",
                        study_window_duration(*datapoint_count, interval_ms)
                    ),
                    ProfileKind::VisibleRange => "Fixed window".to_owned(),
                };
                let mut controls = column![
                    radio(
                        "Visible range",
                        ProfileKind::VisibleRange,
                        Some(*kind),
                        move |kind| { on_change(HeatmapStudy::VolumeProfile(kind)) }
                    ),
                    radio(fixed_window_label, fixed_window, Some(*kind), move |kind| {
                        on_change(HeatmapStudy::VolumeProfile(kind))
                    }),
                ]
                .spacing(4);

                if let ProfileKind::FixedWindow(datapoint_count) = kind {
                    let min_range = CLEANUP_THRESHOLD / 20;
                    let slider = classic_slider_row(
                        Some(text("Window")),
                        min_range as f32..=CLEANUP_THRESHOLD as f32,
                        *datapoint_count as f32,
                        move |new_datapoint_count| {
                            on_change(HeatmapStudy::VolumeProfile(ProfileKind::FixedWindow(
                                new_datapoint_count as usize,
                            )))
                        },
                        |value| format!("{} datapoints", *value as usize),
                        Some(40.0),
                    );

                    controls = controls.push(slider);
                }

                controls.padding(8).into()
            }
        }
    }
}

fn big_trades_color_settings<'a>(
    buy_color: ThemeColor,
    sell_color: ThemeColor,
    theme_accents: ThemeAccents,
    on_change: impl Fn(ThemeColor, ThemeColor) -> Message<KlineStudy> + Copy + 'a,
) -> Element<'a, Message<KlineStudy>> {
    let buy_accent_control =
        crate::widget::theme_color_control(buy_color, theme_accents, move |accent| {
            on_change(
                ThemeColor::with_accent(buy_color.role, accent, buy_color.shade),
                sell_color,
            )
        });
    let buy_shade_control = pick_list(
        data::config::theme::ThemeColorShade::ALL,
        Some(buy_color.shade),
        move |shade| {
            on_change(
                ThemeColor::with_accent(buy_color.role, buy_color.accent, shade),
                sell_color,
            )
        },
    )
    .width(Length::Fill);
    let sell_accent_control =
        crate::widget::theme_color_control(sell_color, theme_accents, move |accent| {
            on_change(
                buy_color,
                ThemeColor::with_accent(sell_color.role, accent, sell_color.shade),
            )
        });
    let sell_shade_control = pick_list(
        data::config::theme::ThemeColorShade::ALL,
        Some(sell_color.shade),
        move |shade| {
            on_change(
                buy_color,
                ThemeColor::with_accent(sell_color.role, sell_color.accent, shade),
            )
        },
    )
    .width(Length::Fill);

    column![
        study_setting(
            "Buy bubbles",
            column![buy_accent_control, buy_shade_control].spacing(4),
            StudyConfigLayout::Inspector(inspector::InspectorTab::Style),
        ),
        study_setting(
            "Sell bubbles",
            column![sell_accent_control, sell_shade_control].spacing(4),
            StudyConfigLayout::Inspector(inspector::InspectorTab::Style),
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
    on_change: impl Fn(KlineStudy) -> Message<KlineStudy> + Copy + 'a,
) -> Element<'a, Message<KlineStudy>> {
    classic_slider_row(
        Some(text("Size filter")),
        0..=big_trades_step_count().saturating_sub(1) as u32,
        big_trades_step_for_threshold(threshold) as u32,
        move |step| {
            on_change(KlineStudy::BigTrades {
                threshold: big_trades_threshold_at_step(step as usize),
                radius_scale,
                buy_color,
                sell_color,
                visible,
            })
        },
        |step| big_trades_threshold_label(big_trades_threshold_at_step(*step as usize)),
        Some(1),
    )
}

fn kline_study_config_view<'a>(
    study: &KlineStudy,
    basis: data::chart::Basis,
    theme_accents: ThemeAccents,
    on_change: impl Fn(KlineStudy) -> Message<KlineStudy> + Copy + 'a,
    include_reset: bool,
    layout: StudyConfigLayout,
) -> Element<'a, Message<KlineStudy>> {
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
            let reset_row = || {
                row![
                    space::horizontal(),
                    button("Reset defaults")
                        .on_press(on_change((*study).default_for_type_for_basis(basis))),
                ]
            };
            let color_section = volume_profile_color_section(
                colors,
                source,
                schedule,
                theme_accents,
                layout,
                move |colors| {
                    on_change(KlineStudy::VolumeProfile {
                        source,
                        schedule,
                        colors,
                        width_percent,
                        opacity_percent,
                        visible,
                    })
                },
            );

            match schedule {
                VolumeProfileSchedule::VisibleRange => {
                    let width_section = volume_profile_width_section(
                        width_percent,
                        "of the chart width",
                        layout,
                        move |width_percent| {
                            on_change(KlineStudy::VolumeProfile {
                                source,
                                schedule,
                                colors,
                                width_percent,
                                opacity_percent,
                                visible,
                            })
                        },
                    );
                    let opacity_section = volume_profile_opacity_section(
                        opacity_percent,
                        layout,
                        move |opacity_percent| {
                            on_change(KlineStudy::VolumeProfile {
                                source,
                                schedule,
                                colors,
                                width_percent,
                                opacity_percent,
                                visible,
                            })
                        },
                    );

                    let mut content = column![];
                    if include_reset {
                        content = content.push(reset_row());
                    }
                    content = content.push(volume_profile_settings_sections(
                        None,
                        width_section,
                        opacity_section,
                        color_section,
                        layout,
                    ));

                    let content = match layout {
                        StudyConfigLayout::Card => content.padding(8),
                        StudyConfigLayout::Inspector(_) => content,
                    };
                    content.spacing(4).into()
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
                        layout,
                        move |width_percent| {
                            on_change(KlineStudy::VolumeProfile {
                                source,
                                schedule,
                                colors,
                                width_percent,
                                opacity_percent,
                                visible,
                            })
                        },
                    );
                    let opacity_section = volume_profile_opacity_section(
                        opacity_percent,
                        layout,
                        move |opacity_percent| {
                            on_change(KlineStudy::VolumeProfile {
                                source,
                                schedule,
                                colors,
                                width_percent,
                                opacity_percent,
                                visible,
                            })
                        },
                    );
                    if intervals.is_empty() {
                        let mut content = column![];
                        if include_reset {
                            content = content.push(reset_row());
                        }
                        content = content.push(volume_profile_settings_sections(
                            Some(match layout {
                                StudyConfigLayout::Card => element_for_layout(
                                    text(
                                        "No recurring interval is at least 4x the chart timeframe",
                                    ),
                                    layout,
                                ),
                                StudyConfigLayout::Inspector(_) => study_setting(
                                    "Interval",
                                    text(
                                        "No recurring interval is at least 4x the chart timeframe",
                                    ),
                                    layout,
                                ),
                            }),
                            width_section,
                            opacity_section,
                            color_section,
                            layout,
                        ));

                        let content = match layout {
                            StudyConfigLayout::Card => content.padding(8),
                            StudyConfigLayout::Inspector(_) => content,
                        };
                        return content.spacing(4).into();
                    }

                    let selected_interval = intervals.contains(&interval).then_some(interval);
                    let interval_picker =
                        pick_list(intervals, selected_interval, move |new_interval| {
                            on_change(KlineStudy::VolumeProfile {
                                source,
                                schedule: VolumeProfileSchedule::Recurring {
                                    interval: new_interval,
                                },
                                colors,
                                width_percent,
                                opacity_percent,
                                visible,
                            })
                        })
                        .width(Length::Fill);

                    let mut content = column![];
                    if include_reset {
                        content = content.push(reset_row());
                    }
                    content = content.push(volume_profile_settings_sections(
                        Some(study_setting("Interval", interval_picker, layout)),
                        width_section,
                        opacity_section,
                        color_section,
                        layout,
                    ));

                    let content = match layout {
                        StudyConfigLayout::Card => content.padding(8),
                        StudyConfigLayout::Inspector(_) => content,
                    };
                    content.spacing(4).into()
                }
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
            let slider = big_trades_threshold_slider(
                threshold,
                radius_scale,
                buy_color,
                sell_color,
                visible,
                on_change,
            );
            let on_radius_change = move |radius_scale| {
                on_change(KlineStudy::BigTrades {
                    threshold,
                    radius_scale,
                    buy_color,
                    sell_color,
                    visible,
                })
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
            let radius_section = study_setting("Visualization", radius_controls, layout);
            match layout {
                StudyConfigLayout::Card => {
                    let mut content = column![];
                    if include_reset {
                        content = content.push(row![
                            space::horizontal(),
                            button("Reset defaults")
                                .on_press(on_change((*study).default_for_type_for_basis(basis))),
                        ]);
                    }
                    content
                        .push(slider)
                        .push(radius_section)
                        .padding(8)
                        .spacing(4)
                        .into()
                }
                StudyConfigLayout::Inspector(tab) => match tab {
                    inspector::InspectorTab::Inputs => column![slider, radius_section]
                        .spacing(inspector::MODAL_SECTION_SPACING)
                        .into(),
                    inspector::InspectorTab::Style => big_trades_color_settings(
                        buy_color,
                        sell_color,
                        theme_accents,
                        move |buy_color, sell_color| {
                            on_change(KlineStudy::BigTrades {
                                threshold,
                                radius_scale,
                                buy_color,
                                sell_color,
                                visible,
                            })
                        },
                    ),
                    inspector::InspectorTab::Source => column![].into(),
                },
            }
        }
        KlineStudy::TpoProfile {
            source,
            anchor,
            letter_period,
            value_area_percent,
            colors,
            visible,
        } => tpo_study_config_view(
            *study,
            *source,
            *anchor,
            *letter_period,
            *value_area_percent,
            *colors,
            *visible,
            basis,
            theme_accents,
            on_change,
            include_reset,
            layout,
        ),
    }
}

fn tpo_study_config_view<'a>(
    study: KlineStudy,
    source: StudyDataSource,
    anchor: TpoAnchor,
    letter_period: Timeframe,
    value_area_percent: u8,
    colors: TpoColorConfig,
    visible: bool,
    basis: data::chart::Basis,
    theme_accents: ThemeAccents,
    on_change: impl Fn(KlineStudy) -> Message<KlineStudy> + Copy + 'a,
    include_reset: bool,
    layout: StudyConfigLayout,
) -> Element<'a, Message<KlineStudy>> {
    if !basis.is_time() {
        return iced::widget::center(text("TPO profiles require a time-based chart")).into();
    }

    let anchor_picker = pick_list(TpoAnchor::ALL, Some(anchor), move |anchor| {
        on_change(KlineStudy::TpoProfile {
            source,
            anchor,
            letter_period,
            value_area_percent,
            colors,
            visible,
        })
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
        on_change(KlineStudy::TpoProfile {
            source,
            anchor,
            letter_period: period,
            value_area_percent,
            colors,
            visible,
        })
    })
    .width(Length::Fill);

    let value_area_target = classic_slider_row(
        Some(text("% of TPOs")),
        f32::from(MIN_TPO_VALUE_AREA_PERCENT)..=f32::from(MAX_TPO_VALUE_AREA_PERCENT),
        f32::from(value_area_percent.clamp(MIN_TPO_VALUE_AREA_PERCENT, MAX_TPO_VALUE_AREA_PERCENT)),
        move |value| {
            on_change(KlineStudy::TpoProfile {
                source,
                anchor,
                letter_period,
                value_area_percent: (value.round() as u8)
                    .clamp(MIN_TPO_VALUE_AREA_PERCENT, MAX_TPO_VALUE_AREA_PERCENT),
                colors,
                visible,
            })
        },
        |value| format!("{value:.0}%"),
        Some(5.0),
    );

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
            return element_for_layout(
                tpo_visibility_toggle(colors, label, slot, study, on_change),
                layout,
            );
        }

        let selected = tpo_color_for(colors, slot);
        let setting = tpo_color_setting(selected, theme_accents, move |color| {
            on_change(KlineStudy::TpoProfile {
                source,
                anchor,
                letter_period,
                value_area_percent,
                colors: tpo_color_with(colors, slot, color),
                visible,
            })
        });
        let title = match slot {
            TpoColorSlot::OutsideValueArea | TpoColorSlot::ValueAreaCells => {
                crate::widget::text::body(label).into()
            }
            TpoColorSlot::Poc
            | TpoColorSlot::InitialBalance
            | TpoColorSlot::SinglePrint
            | TpoColorSlot::PoorExtremes => {
                tpo_visibility_toggle(colors, label, slot, study, on_change)
            }
        };
        study_setting_with_title(title, setting, layout)
    })
    .collect::<Vec<_>>();

    let mut content = column![];
    if include_reset {
        content = content.push(row![
            space::horizontal(),
            button("Reset defaults").on_press(on_change(study.default_for_type_for_basis(basis)))
        ]);
    }

    match layout {
        StudyConfigLayout::Card => {
            content = content.push(study_setting("Anchor (UTC)", anchor_picker, layout));
            content = content.push(study_setting("Bracket duration", period_picker, layout));
            content = content.push(value_area_target);
            content = content.push(rule::horizontal(1.0).style(style::split_ruler));
            content = content.push(column(color_controls).spacing(8));
            content.padding(8).spacing(12).into()
        }
        StudyConfigLayout::Inspector(tab) => {
            match tab {
                inspector::InspectorTab::Inputs => {
                    content = content.push(study_setting("Anchor (UTC)", anchor_picker, layout));
                    content =
                        content.push(study_setting("Bracket duration", period_picker, layout));
                    content = content.push(study_setting(
                        "Value area target",
                        value_area_target,
                        layout,
                    ));
                }
                inspector::InspectorTab::Style => {
                    content = content.push(column(color_controls).spacing(8));
                }
                inspector::InspectorTab::Source => {}
            }
            content.spacing(inspector::MODAL_SECTION_SPACING).into()
        }
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
    colors: TpoColorConfig,
    label: &'static str,
    slot: TpoColorSlot,
    study: KlineStudy,
    on_change: impl Fn(KlineStudy) -> Message<KlineStudy> + Copy + 'a,
) -> Element<'a, Message<KlineStudy>> {
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
            let colors = match slot {
                TpoColorSlot::Poc => colors.with_show_poc(enabled),
                TpoColorSlot::InitialBalance => colors.with_show_initial_balance(enabled),
                TpoColorSlot::SinglePrint => colors.with_show_single_print(enabled),
                TpoColorSlot::PoorExtremes => colors.with_show_poor_extremes(enabled),
                TpoColorSlot::OutsideValueArea | TpoColorSlot::ValueAreaCells => unreachable!(),
            };
            on_change(match study {
                KlineStudy::TpoProfile {
                    source,
                    anchor,
                    letter_period,
                    value_area_percent,
                    visible,
                    ..
                } => KlineStudy::TpoProfile {
                    source,
                    anchor,
                    letter_period,
                    value_area_percent,
                    colors,
                    visible,
                },
                KlineStudy::VolumeProfile { .. } | KlineStudy::BigTrades { .. } => unreachable!(),
            })
        })
        .width(Length::Fill)
        .into()
}

fn tpo_color_setting<'a>(
    color: data::config::theme::ThemeColor,
    theme_accents: ThemeAccents,
    on_change: impl Fn(data::config::theme::ThemeColor) -> Message<KlineStudy> + Copy + 'a,
) -> Element<'a, Message<KlineStudy>> {
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
        theme_accents: ThemeAccents,
        on_change: impl Fn(Self) -> Message<Self> + Copy + 'a,
    ) -> Element<'a, Message<Self>> {
        kline_study_config_view(
            self,
            basis,
            theme_accents,
            on_change,
            true,
            StudyConfigLayout::Card,
        )
    }

    fn view_config_without_reset<'a>(
        &self,
        basis: data::chart::Basis,
        theme_accents: ThemeAccents,
        tab: inspector::InspectorTab,
        on_change: impl Fn(Self) -> Message<Self> + Copy + 'a,
    ) -> Option<Element<'a, Message<Self>>> {
        if tab == inspector::InspectorTab::Source
            || (tab == inspector::InspectorTab::Inputs
                && matches!(
                    self,
                    KlineStudy::VolumeProfile {
                        schedule: VolumeProfileSchedule::VisibleRange,
                        ..
                    }
                ))
        {
            return None;
        }

        Some(kline_study_config_view(
            self,
            basis,
            theme_accents,
            on_change,
            false,
            StudyConfigLayout::Inspector(tab),
        ))
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Message<S: Study> {
    StudyToggled(S, bool),
    StudyValueChanged(S),
}

pub fn view<'a, S: Study + ToString>(
    active_studies: &[S],
    basis: data::chart::Basis,
    theme_accents: ThemeAccents,
) -> Element<'a, Message<S>> {
    let mut content = column![].spacing(4);
    for study in S::all() {
        let study_config = active_studies
            .iter()
            .find(|active| active.is_same_type(&study))
            .copied();
        let checkbox = checkbox(study_config.is_some())
            .label(study_config.unwrap_or(study).to_string())
            .on_toggle(move |checked| Message::StudyToggled(study, checked));
        let checkbox_row = row![checkbox]
            .height(36)
            .align_y(iced::Alignment::Center)
            .padding([0, 8]);
        let mut controls = column![checkbox_row];
        if let Some(config) = study_config {
            controls =
                controls.push(config.view_config(basis, theme_accents, Message::StudyValueChanged));
        }
        content = content.push(
            container(controls)
                .width(Length::Fill)
                .style(style::inspector_control),
        );
    }
    content.into()
}
