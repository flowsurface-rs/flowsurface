use crate::{
    split_column,
    style::{self, Icon, icon_text},
    widget::labeled_slider,
};
use data::chart::heatmap::{CLEANUP_THRESHOLD, HeatmapStudy, ProfileKind};
use data::chart::kline::FootprintStudy;
use data::chart::kline_v2::{
    KlineStudy, MAX_VOLUME_PROFILE_OPACITY_PERCENT, MAX_VOLUME_PROFILE_WIDTH_PERCENT,
    MIN_VOLUME_PROFILE_OPACITY_PERCENT, MIN_VOLUME_PROFILE_WIDTH_PERCENT, VolumeProfileColorConfig,
    VolumeProfileSchedule,
};
use iced::{
    Element, Length, padding,
    widget::{button, checkbox, column, container, pick_list, row, rule, space, text},
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

    fn view_config_without_reset<'a>(
        &self,
        basis: data::chart::Basis,
        on_change: impl Fn(Self) -> Message<Self> + Copy + 'a,
    ) -> Element<'a, Message<Self>> {
        self.view_config(basis, on_change)
    }
}

fn theme_color_setting_row<'a, M>(
    label: impl Into<Element<'a, M>>,
    color_choices: impl Into<Element<'a, M>>,
    shade_picklist: impl Into<Element<'a, M>>,
) -> Element<'a, M>
where
    M: 'a,
{
    let label: Element<'a, M> = label.into();
    let color_choices: Element<'a, M> = color_choices.into();
    let shade_picklist: Element<'a, M> = shade_picklist.into();
    column![label, color_choices, shade_picklist]
        .width(Length::Fill)
        .spacing(4)
        .into()
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
                )
                .width(Length::Fill);

                theme_color_setting_row(label, color_choices, shade_picklist)
            }
        });

    column(controls).spacing(16).into()
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
    labeled_slider(
        "Width",
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
    )
}

fn volume_profile_opacity_section<'a>(
    opacity_percent: u8,
    on_change: impl Fn(u8) -> Message<KlineStudy> + Copy + 'a,
) -> Element<'a, Message<KlineStudy>> {
    let opacity_percent = opacity_percent.clamp(
        MIN_VOLUME_PROFILE_OPACITY_PERCENT,
        MAX_VOLUME_PROFILE_OPACITY_PERCENT,
    );
    labeled_slider(
        "Opacity",
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
    )
}

fn volume_profile_settings_sections<'a>(
    schedule_section: Option<Element<'a, Message<KlineStudy>>>,
    width_section: Element<'a, Message<KlineStudy>>,
    opacity_section: Element<'a, Message<KlineStudy>>,
    color_section: Element<'a, Message<KlineStudy>>,
) -> Element<'a, Message<KlineStudy>> {
    if let Some(schedule) = schedule_section {
        column![
            schedule,
            rule::horizontal(1.0).style(style::split_ruler),
            column![width_section, opacity_section].spacing(8),
            rule::horizontal(1.0).style(style::split_ruler),
            color_section,
        ]
        .spacing(16)
        .into()
    } else {
        column![
            column![width_section, opacity_section].spacing(8),
            rule::horizontal(1.0).style(style::split_ruler),
            color_section,
        ]
        .spacing(16)
        .into()
    }
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
                let slider_ui = labeled_slider(
                    "Lookback",
                    10.0..=400.0,
                    lookback as f32,
                    move |new_value| {
                        on_change(FootprintStudy::NPoC {
                            lookback: new_value as usize,
                        })
                    },
                    |value| format!("{value:.0} datapoints"),
                    Some(10.0),
                );

                column![slider_ui].padding(8).spacing(4).into()
            }
            FootprintStudy::Imbalance {
                threshold,
                color_scale,
                ignore_zeros,
            } => {
                let qty_threshold = {
                    let threshold_slider = labeled_slider(
                        "Ask:Bid threshold",
                        100.0..=800.0,
                        threshold as f32,
                        move |new_value| {
                            on_change(FootprintStudy::Imbalance {
                                threshold: new_value as usize,
                                color_scale,
                                ignore_zeros,
                            })
                        },
                        |value| format!("{value:.0}%"),
                        Some(25.0),
                    );

                    column![threshold_slider].padding(8).spacing(4)
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
                        let scaling_slider = labeled_slider(
                            "Opaque color at",
                            50.0..=2000.0,
                            color_scale_value as f32,
                            move |new_value| {
                                on_change(FootprintStudy::Imbalance {
                                    threshold,
                                    color_scale: Some(new_value as usize),
                                    ignore_zeros,
                                })
                            },
                            |value| format!("{value:.0}x"),
                            Some(50.0),
                        );

                        column![color_scale_checkbox, scaling_slider]
                            .padding(8)
                            .spacing(8)
                    } else {
                        column![color_scale_checkbox].padding(8)
                    }
                };

                let ignore_zeros_checkbox = {
                    let cbox =
                        checkbox(ignore_zeros)
                            .label("Ignore zeros")
                            .on_toggle(move |is_checked| {
                                on_change(FootprintStudy::Imbalance {
                                    threshold,
                                    color_scale,
                                    ignore_zeros: is_checked,
                                })
                            });

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
                    let min_range = CLEANUP_THRESHOLD / 20;
                    let slider = labeled_slider(
                        "Window",
                        min_range as f32..=CLEANUP_THRESHOLD as f32,
                        *datapoint_count as f32,
                        move |new_datapoint_count| {
                            on_change(HeatmapStudy::VolumeProfile(ProfileKind::FixedWindow(
                                new_datapoint_count as usize,
                            )))
                        },
                        move |value| {
                            let datapoint_count = *value as usize;
                            format!(
                                "{} datapoints ({})",
                                datapoint_count,
                                study_window_duration(datapoint_count, interval_ms)
                            )
                        },
                        Some(40.0),
                    );

                    let switch_kind = button(text("Switch to visible range")).on_press(on_change(
                        HeatmapStudy::VolumeProfile(ProfileKind::VisibleRange),
                    ));

                    column![row![space::horizontal(), switch_kind,], slider,]
                        .padding(8)
                        .spacing(4)
                        .into()
                }
                ProfileKind::VisibleRange => {
                    let switch_kind = button(text("Switch to fixed window")).on_press(on_change(
                        HeatmapStudy::VolumeProfile(ProfileKind::FixedWindow(
                            CLEANUP_THRESHOLD / 5_usize,
                        )),
                    ));

                    column![row![space::horizontal(), switch_kind,],]
                        .padding(8)
                        .spacing(4)
                        .into()
                }
            },
        }
    }
}

fn kline_study_config_view<'a>(
    study: &KlineStudy,
    basis: data::chart::Basis,
    on_change: impl Fn(KlineStudy) -> Message<KlineStudy> + Copy + 'a,
    include_reset: bool,
) -> Element<'a, Message<KlineStudy>> {
    match study {
        KlineStudy::VolumeProfile {
            schedule,
            colors,
            width_percent,
            opacity_percent,
            visible,
        } => {
            let schedule = *schedule;
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
            let color_section = volume_profile_color_section(colors, schedule, move |colors| {
                on_change(KlineStudy::VolumeProfile {
                    schedule,
                    colors,
                    width_percent,
                    opacity_percent,
                    visible,
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
                                visible,
                            })
                        },
                    );
                    let opacity_section =
                        volume_profile_opacity_section(opacity_percent, move |opacity_percent| {
                            on_change(KlineStudy::VolumeProfile {
                                schedule,
                                colors,
                                width_percent,
                                opacity_percent,
                                visible,
                            })
                        });

                    let mut content = column![];
                    if include_reset {
                        content = content.push(reset_row());
                    }
                    content = content.push(volume_profile_settings_sections(
                        None,
                        width_section,
                        opacity_section,
                        color_section,
                    ));

                    content.padding(8).spacing(4).into()
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
                                visible,
                            })
                        },
                    );
                    let opacity_section =
                        volume_profile_opacity_section(opacity_percent, move |opacity_percent| {
                            on_change(KlineStudy::VolumeProfile {
                                schedule,
                                colors,
                                width_percent,
                                opacity_percent,
                                visible,
                            })
                        });
                    if intervals.is_empty() {
                        let mut content = column![];
                        if include_reset {
                            content = content.push(reset_row());
                        }
                        content = content.push(volume_profile_settings_sections(
                            Some(
                                text("No recurring interval is at least 4x the chart timeframe")
                                    .into(),
                            ),
                            width_section,
                            opacity_section,
                            color_section,
                        ));

                        return content.padding(8).spacing(4).into();
                    }

                    let selected_interval = intervals.contains(&interval).then_some(interval);
                    let interval_picker =
                        pick_list(intervals, selected_interval, move |new_interval| {
                            on_change(KlineStudy::VolumeProfile {
                                schedule: VolumeProfileSchedule::Recurring {
                                    interval: new_interval,
                                },
                                colors,
                                width_percent,
                                opacity_percent,
                                visible,
                            })
                        });

                    let mut content = column![];
                    if include_reset {
                        content = content.push(reset_row());
                    }
                    content = content.push(volume_profile_settings_sections(
                        Some(column![text("Interval"), interval_picker].spacing(4).into()),
                        width_section,
                        opacity_section,
                        color_section,
                    ));

                    content.padding(8).spacing(4).into()
                }
            }
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
        kline_study_config_view(self, basis, on_change, true)
    }

    fn view_config_without_reset<'a>(
        &self,
        basis: data::chart::Basis,
        on_change: impl Fn(Self) -> Message<Self> + Copy + 'a,
    ) -> Element<'a, Message<Self>> {
        kline_study_config_view(self, basis, on_change, false)
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
                    (!is_disabled).then_some(move |checked| Message::StudyToggled(study, checked)),
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
