use crate::{
    style::{self, Icon, icon_text},
    theme::ThemeAccents,
    widget::{
        inspector::{self, InspectorSection},
        labeled_slider,
    },
};
use data::chart::heatmap::{CLEANUP_THRESHOLD, HeatmapStudy, ProfileKind};
use data::chart::kline::{
    KlineStudy, MAX_VOLUME_PROFILE_OPACITY_PERCENT, MAX_VOLUME_PROFILE_WIDTH_PERCENT,
    MIN_VOLUME_PROFILE_OPACITY_PERCENT, MIN_VOLUME_PROFILE_WIDTH_PERCENT, VolumeProfileColorConfig,
    VolumeProfileSchedule,
};
use exchange::TickMultiplier;
use exchange::unit::{MinTicksize, PriceStep};
use iced::{
    Element, Length, padding,
    widget::{button, checkbox, column, container, pick_list, row, rule, space, text},
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
        footprint_tick_multiplier: Option<TickMultiplier>,
        min_ticksize: Option<MinTicksize>,
        theme_accents: ThemeAccents,
        on_change: impl Fn(Self) -> Message<Self> + Copy + 'a,
    ) -> Element<'a, Message<Self>>;

    fn view_config_without_reset<'a>(
        &self,
        basis: data::chart::Basis,
        footprint_tick_multiplier: Option<TickMultiplier>,
        min_ticksize: Option<MinTicksize>,
        theme_accents: ThemeAccents,
        on_change: impl Fn(Self) -> Message<Self> + Copy + 'a,
    ) -> Element<'a, Message<Self>> {
        self.view_config(
            basis,
            footprint_tick_multiplier,
            min_ticksize,
            theme_accents,
            on_change,
        )
    }
}

#[derive(Debug, Clone, Copy)]
enum StudyConfigLayout {
    Card,
    Inspector,
}

fn section_for_layout<'a, M>(
    section: InspectorSection<'a, M>,
    layout: StudyConfigLayout,
) -> InspectorSection<'a, M>
where
    M: 'a,
{
    match layout {
        StudyConfigLayout::Card => section,
        StudyConfigLayout::Inspector => section.with_modal_container(),
    }
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
        StudyConfigLayout::Inspector => crate::widget::inspector::modal_container(content),
    }
}

fn theme_color_setting_row<'a, M>(
    label: impl Into<Element<'a, M>>,
    color_choices: impl Into<Element<'a, M>>,
    shade_picklist: impl Into<Element<'a, M>>,
    layout: StudyConfigLayout,
) -> Element<'a, M>
where
    M: 'a,
{
    let label: Element<'a, M> = label.into();
    let color_choices: Element<'a, M> = color_choices.into();
    let shade_picklist: Element<'a, M> = shade_picklist.into();
    section_for_layout(
        InspectorSection::new(label, column![color_choices, shade_picklist].spacing(4)),
        layout,
    )
    .into_element()
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
    theme_accents: ThemeAccents,
    layout: StudyConfigLayout,
    on_change: impl Fn(VolumeProfileColorConfig) -> Message<KlineStudy> + Copy + 'a,
) -> Element<'a, Message<KlineStudy>> {
    let controls = VolumeProfileColorConfig::color_specs_for(schedule)
        .iter()
        .map(|spec| {
            let slot = spec.slot;
            let enabled = match slot {
                data::chart::kline::VolumeProfileColorSlot::NakedPoc => colors.show_npoc,
                data::chart::kline::VolumeProfileColorSlot::FilledPoc => colors.show_poc,
                _ => true,
            };
            let label: Element<'a, Message<KlineStudy>> = match slot {
                data::chart::kline::VolumeProfileColorSlot::NakedPoc => checkbox(colors.show_npoc)
                    .label(spec.label)
                    .text_size(crate::style::text_size::SECTION)
                    .on_toggle(move |show| on_change(colors.with_show_npoc(show)))
                    .width(Length::Fill)
                    .into(),
                data::chart::kline::VolumeProfileColorSlot::FilledPoc => checkbox(colors.show_poc)
                    .label(spec.label)
                    .text_size(crate::style::text_size::SECTION)
                    .on_toggle(move |show| on_change(colors.with_show_poc(show)))
                    .width(Length::Fill)
                    .into(),
                _ => crate::widget::text::section(spec.label)
                    .width(Length::Fill)
                    .into(),
            };

            if !enabled {
                element_for_layout(column![row![label]].spacing(4), layout)
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

                theme_color_setting_row(label, color_choices, shade_picklist, layout)
            }
        });

    let spacing = match layout {
        StudyConfigLayout::Card => 16.0,
        StudyConfigLayout::Inspector => inspector::MODAL_SECTION_SPACING,
    };
    column(controls).spacing(spacing).into()
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
    ticksize_section: Element<'a, Message<KlineStudy>>,
    width_section: Element<'a, Message<KlineStudy>>,
    opacity_section: Element<'a, Message<KlineStudy>>,
    color_section: Element<'a, Message<KlineStudy>>,
    layout: StudyConfigLayout,
) -> Element<'a, Message<KlineStudy>> {
    let width_opacity_section =
        element_for_layout(column![width_section, opacity_section].spacing(8), layout);
    let content = if let Some(schedule) = schedule_section {
        let schedule_row = match layout {
            StudyConfigLayout::Card => row![
                container(schedule).width(Length::Fill),
                container(ticksize_section).width(Length::Fill),
            ]
            .spacing(12)
            .into(),
            StudyConfigLayout::Inspector => inspector::modal_row([schedule, ticksize_section]),
        };

        match layout {
            StudyConfigLayout::Card => column![
                schedule_row,
                rule::horizontal(1.0).style(style::split_ruler),
                width_opacity_section,
                rule::horizontal(1.0).style(style::split_ruler),
                color_section,
            ]
            .spacing(16),
            StudyConfigLayout::Inspector => {
                column![schedule_row, width_opacity_section, color_section,]
                    .spacing(inspector::MODAL_SECTION_SPACING)
            }
        }
    } else {
        match layout {
            StudyConfigLayout::Card => column![
                ticksize_section,
                rule::horizontal(1.0).style(style::split_ruler),
                width_opacity_section,
                rule::horizontal(1.0).style(style::split_ruler),
                color_section,
            ]
            .spacing(16),
            StudyConfigLayout::Inspector => {
                column![ticksize_section, width_opacity_section, color_section,]
                    .spacing(inspector::MODAL_SECTION_SPACING)
            }
        }
    };

    content.into()
}

fn volume_profile_ticksize_section<'a>(
    tick_multiplier: Option<TickMultiplier>,
    footprint_tick_multiplier: Option<TickMultiplier>,
    min_ticksize: Option<MinTicksize>,
    layout: StudyConfigLayout,
    on_change: impl Fn(Option<TickMultiplier>) -> Message<KlineStudy> + Copy + 'a,
) -> Element<'a, Message<KlineStudy>> {
    let floor = footprint_tick_multiplier.map_or(0, |multiplier| multiplier.0);
    let options = TickMultiplier::ALL
        .iter()
        .copied()
        .filter(|multiplier| multiplier.0 >= floor)
        .collect::<Vec<_>>();
    let selected = tick_multiplier
        .map(|multiplier| TickMultiplier(multiplier.0.max(floor)))
        .unwrap_or_else(|| {
            footprint_tick_multiplier.unwrap_or_else(|| {
                TickMultiplier::ALL
                    .first()
                    .copied()
                    .expect("TickMultiplier::ALL is never empty")
            })
        });
    let picker = pick_list(options, Some(selected), move |option| {
        on_change(Some(option))
    });

    let mut setting = column![picker.width(Length::Fill)]
        .spacing(4)
        .width(Length::Fill);

    if let Some(min_tick) = min_ticksize {
        let step = selected.multiply_step(PriceStep::from(min_tick));
        let step_text = row![
            space::horizontal(),
            crate::widget::text::small(format!("Step: {}", step.to_ui_string())),
        ];
        setting = setting.push(step_text);
    }

    section_for_layout(
        InspectorSection::new(crate::widget::text::section("Tick size"), setting),
        layout,
    )
    .into_element()
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
        _footprint_tick_multiplier: Option<TickMultiplier>,
        _min_ticksize: Option<MinTicksize>,
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
    footprint_tick_multiplier: Option<TickMultiplier>,
    min_ticksize: Option<MinTicksize>,
    theme_accents: ThemeAccents,
    on_change: impl Fn(KlineStudy) -> Message<KlineStudy> + Copy + 'a,
    include_reset: bool,
    layout: StudyConfigLayout,
) -> Element<'a, Message<KlineStudy>> {
    match study {
        KlineStudy::VolumeProfile {
            schedule,
            colors,
            width_percent,
            opacity_percent,
            visible,
            tick_multiplier,
        } => {
            let schedule = *schedule;
            let colors = *colors;
            let width_percent = *width_percent;
            let opacity_percent = *opacity_percent;
            let visible = *visible;
            let tick_multiplier = *tick_multiplier;
            let reset_row = || {
                row![
                    space::horizontal(),
                    button("Reset defaults")
                        .on_press(on_change((*study).default_for_type_for_basis(basis))),
                ]
            };
            let color_section = volume_profile_color_section(
                colors,
                schedule,
                theme_accents,
                layout,
                move |colors| {
                    on_change(KlineStudy::VolumeProfile {
                        schedule,
                        colors,
                        width_percent,
                        opacity_percent,
                        visible,
                        tick_multiplier,
                    })
                },
            );
            let ticksize_section = volume_profile_ticksize_section(
                tick_multiplier,
                footprint_tick_multiplier,
                min_ticksize,
                layout,
                move |new_multiplier| {
                    on_change(KlineStudy::VolumeProfile {
                        schedule,
                        colors,
                        width_percent,
                        opacity_percent,
                        visible,
                        tick_multiplier: new_multiplier,
                    })
                },
            );

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
                                tick_multiplier,
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
                                tick_multiplier,
                            })
                        });

                    let mut content = column![];
                    if include_reset {
                        content = content.push(reset_row());
                    }
                    content = content.push(volume_profile_settings_sections(
                        None,
                        ticksize_section,
                        width_section,
                        opacity_section,
                        color_section,
                        layout,
                    ));

                    let content = match layout {
                        StudyConfigLayout::Card => content.padding(8),
                        StudyConfigLayout::Inspector => content,
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
                        move |width_percent| {
                            on_change(KlineStudy::VolumeProfile {
                                schedule,
                                colors,
                                width_percent,
                                opacity_percent,
                                visible,
                                tick_multiplier,
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
                                tick_multiplier,
                            })
                        });
                    if intervals.is_empty() {
                        let mut content = column![];
                        if include_reset {
                            content = content.push(reset_row());
                        }
                        content = content.push(volume_profile_settings_sections(
                            Some(element_for_layout(
                                text("No recurring interval is at least 4x the chart timeframe"),
                                layout,
                            )),
                            ticksize_section,
                            width_section,
                            opacity_section,
                            color_section,
                            layout,
                        ));

                        let content = match layout {
                            StudyConfigLayout::Card => content.padding(8),
                            StudyConfigLayout::Inspector => content,
                        };
                        return content.spacing(4).into();
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
                                tick_multiplier,
                            })
                        })
                        .width(Length::Fill);

                    let mut content = column![];
                    if include_reset {
                        content = content.push(reset_row());
                    }
                    content = content.push(volume_profile_settings_sections(
                        Some(
                            section_for_layout(
                                InspectorSection::new(
                                    crate::widget::text::section("Interval"),
                                    interval_picker,
                                ),
                                layout,
                            )
                            .into_element(),
                        ),
                        ticksize_section,
                        width_section,
                        opacity_section,
                        color_section,
                        layout,
                    ));

                    let content = match layout {
                        StudyConfigLayout::Card => content.padding(8),
                        StudyConfigLayout::Inspector => content,
                    };
                    content.spacing(4).into()
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
        footprint_tick_multiplier: Option<TickMultiplier>,
        min_ticksize: Option<MinTicksize>,
        theme_accents: ThemeAccents,
        on_change: impl Fn(Self) -> Message<Self> + Copy + 'a,
    ) -> Element<'a, Message<Self>> {
        kline_study_config_view(
            self,
            basis,
            footprint_tick_multiplier,
            min_ticksize,
            theme_accents,
            on_change,
            true,
            StudyConfigLayout::Card,
        )
    }

    fn view_config_without_reset<'a>(
        &self,
        basis: data::chart::Basis,
        footprint_tick_multiplier: Option<TickMultiplier>,
        min_ticksize: Option<MinTicksize>,
        theme_accents: ThemeAccents,
        on_change: impl Fn(Self) -> Message<Self> + Copy + 'a,
    ) -> Element<'a, Message<Self>> {
        kline_study_config_view(
            self,
            basis,
            footprint_tick_multiplier,
            min_ticksize,
            theme_accents,
            on_change,
            false,
            StudyConfigLayout::Inspector,
        )
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
        footprint_tick_multiplier: Option<TickMultiplier>,
        min_ticksize: Option<MinTicksize>,
        theme_accents: ThemeAccents,
    ) -> Element<'a, Message<S>> {
        self.view_available(
            active_studies,
            basis,
            footprint_tick_multiplier,
            min_ticksize,
            theme_accents,
            S::all(),
            &[],
        )
    }

    pub fn view_available<'a>(
        &self,
        active_studies: &[S],
        basis: data::chart::Basis,
        footprint_tick_multiplier: Option<TickMultiplier>,
        min_ticksize: Option<MinTicksize>,
        theme_accents: ThemeAccents,
        available_studies: Vec<S>,
        disabled: &[S],
    ) -> Element<'a, Message<S>> {
        let mut content = column![].spacing(4);

        for available_study in available_studies {
            content = content.push(self.create_study_row(
                available_study,
                active_studies,
                basis,
                footprint_tick_multiplier,
                min_ticksize,
                theme_accents,
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
        footprint_tick_multiplier: Option<TickMultiplier>,
        min_ticksize: Option<MinTicksize>,
        theme_accents: ThemeAccents,
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
            column = column.push(config.view_config(
                basis,
                footprint_tick_multiplier,
                min_ticksize,
                theme_accents,
                Message::StudyValueChanged,
            ));
        }

        container(column).style(style::modal_container).into()
    }
}
