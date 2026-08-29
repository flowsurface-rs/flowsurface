use super::{
    Action as ChartAction, CandlestickCommand, ChartCommand, CommandResult, FootprintCommand,
    IndicatorCommand, KlineChart, Message as ChartMessage, SeriesCommand,
};
use crate::widget::study_configurator::Study as _;
use crate::{
    screen::dashboard::pane::ChartAction as PaneChartAction,
    split_column, style,
    theme::ThemeAccents,
    tooltip,
    widget::{
        inspector::{self, InspectorSection},
        labeled_slider,
        series_editor::{self, SeriesStyleEntry, TickerSeriesEditor},
        study_configurator as study, theme_color_control,
    },
};
use data::chart::kline::{
    BarMode, CandlestickColorSlot, CandlestickConfig, FootprintConfig, FootprintImbalanceConfig,
    FootprintPocColorConfig, FootprintPocColorSlot, FootprintPocConfig, HistogramMode,
    IndicatorColorConfig, IndicatorColorSlot, IndicatorConfig, IndicatorSetting,
    IndicatorSettingControl, IndicatorSettingValue, IndicatorSettingsContext, IndicatorStyleConfig,
    KlineIndicator, KlineStudy, KlineStudyId, LineStrokeStyle, MAX_CANDLESTICK_OPACITY_PERCENT,
    MAX_LINE_WIDTH, MIN_CANDLESTICK_OPACITY_PERCENT, MIN_LINE_WIDTH, MarkKind, ThemeAccent,
};
use data::chart::kline_data::{ClusterKind, ClusterScaling};
use data::config::theme::{ThemeColor, ThemeColorShade};
use exchange::unit::MinTicksize;
use exchange::{TickMultiplier, TickerInfo};
use iced::{
    Alignment, Element, Length,
    widget::{
        button, checkbox, column, container, pick_list, radio, row, space, text, text_input,
        tooltip::Position as TooltipPosition,
    },
};

#[derive(Debug, Clone)]
pub enum Message {
    ResetRequested,
    ResetConfirmed,
    ResetCancelled,
    ClusterKindSelected(ClusterKind),
    ClusterScalingSelected(ClusterScaling),
    FootprintImbalanceConfigChanged(FootprintImbalanceConfig),
    FootprintPocConfigChanged(FootprintPocConfig),
    TradeDataEnabledChanged(bool),
    ResetFootprintSettings,
    ResetSeriesSettings(TickerInfo),
    Study(study::Message<KlineStudy>),
    Indicator {
        instance_id: data::chart::kline::IndicatorInstanceId,
        message: KlineIndicatorSettingsMessage,
    },
    CandlestickSettings(CandlestickSettingsMessage),
    SeriesEditor(series_editor::Message),
}

fn setting_section<'a, M>(
    label: &'a str,
    control: impl Into<Element<'a, M>>,
) -> InspectorSection<'a, M>
where
    M: 'a,
{
    InspectorSection::new(
        crate::widget::text::section(label),
        container(control).width(Length::Fill),
    )
}

fn settings_row<'a, M>(label: &'a str, control: impl Into<Element<'a, M>>) -> Element<'a, M>
where
    M: 'a,
{
    setting_section(label, control)
        .with_modal_container()
        .into_element()
}

#[derive(Debug, Clone)]
pub enum KlineIndicatorSettingsMessage {
    TabSelected(IndicatorSettingsTab),
    InputChanged {
        id: &'static str,
        value: String,
        config: Option<IndicatorConfig>,
    },
    DataConfig(IndicatorConfig),
    Mark(MarkKind),
    Color {
        slot: IndicatorColorSlot,
        accent: Option<ThemeAccent>,
        shade: ThemeColorShade,
    },
    FillVisible(bool),
    LineWidth {
        slot: IndicatorColorSlot,
        width: u8,
    },
    StrokeStyle {
        slot: IndicatorColorSlot,
        stroke_style: LineStrokeStyle,
    },
    Reset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorSettingsTab {
    Inputs,
    Style,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndicatorSettingsState {
    instance_id: data::chart::kline::IndicatorInstanceId,
    indicator: data::chart::kline::KlineIndicator,
    tab: IndicatorSettingsTab,
    input_drafts: Vec<(&'static str, String)>,
}

impl IndicatorSettingsState {
    pub fn new(
        instance_id: data::chart::kline::IndicatorInstanceId,
        indicator: data::chart::kline::KlineIndicator,
    ) -> Self {
        Self {
            instance_id,
            indicator,
            tab: IndicatorSettingsTab::Inputs,
            input_drafts: Vec::new(),
        }
    }

    pub fn instance_id(&self) -> data::chart::kline::IndicatorInstanceId {
        self.instance_id
    }

    pub fn indicator(&self) -> data::chart::kline::KlineIndicator {
        self.indicator
    }

    pub fn tab(&self) -> IndicatorSettingsTab {
        self.tab
    }

    pub fn select_tab(&mut self, tab: IndicatorSettingsTab) {
        self.tab = tab;
    }

    pub fn input_draft(&self, setting_id: &'static str) -> Option<&str> {
        self.input_drafts
            .iter()
            .find_map(|(id, value)| (*id == setting_id).then_some(value.as_str()))
    }

    pub fn set_input_draft(&mut self, setting_id: &'static str, value: String) {
        if let Some((_, draft)) = self
            .input_drafts
            .iter_mut()
            .find(|(id, _)| *id == setting_id)
        {
            *draft = value;
        } else {
            self.input_drafts.push((setting_id, value));
        }
    }

    pub fn clear_input_drafts(&mut self) {
        self.input_drafts.clear();
    }
}

pub fn kline_indicator_cfg_view<'a>(
    modal: &'a IndicatorSettingsState,
    data_config: Option<IndicatorConfig>,
    basis: data::chart::Basis,
    color_config: IndicatorColorConfig,
    style_config: IndicatorStyleConfig,
    mark: Option<data::chart::kline::MarkKind>,
    chart: &'a KlineChart,
    theme_accents: ThemeAccents,
) -> Element<'a, Message> {
    let indicator = modal.indicator();
    let instance_id = modal.instance_id();

    let mark_section = indicator_mark_section(instance_id, indicator, mark, chart);
    let fill_section = indicator_fill_section(instance_id, indicator, style_config);
    let channel_style_section = indicator_channel_style_section(
        instance_id,
        indicator,
        color_config,
        style_config,
        mark,
        theme_accents,
    );

    let input_settings = data_config
        .map(|config| config.settings(IndicatorSettingsContext { basis }))
        .unwrap_or_default();
    let has_input_settings = !input_settings.is_empty();
    let selected_tab = if has_input_settings {
        modal.tab()
    } else {
        IndicatorSettingsTab::Style
    };
    let tabs = has_input_settings.then(|| indicator_settings_tabs(selected_tab, instance_id));

    let settings_content: Element<'a, Message> = match selected_tab {
        IndicatorSettingsTab::Inputs => {
            let config = data_config.expect("input settings require an indicator configuration");
            indicator_settings_content(input_settings, config, modal)
        }
        IndicatorSettingsTab::Style => {
            let mut style_settings = column![].spacing(inspector::MODAL_SECTION_SPACING);
            for section in [
                mark_section.map(InspectorSection::into_element),
                fill_section.map(InspectorSection::into_element),
                Some(channel_style_section),
            ]
            .into_iter()
            .flatten()
            {
                style_settings = style_settings.push(section);
            }
            style_settings.into()
        }
    };

    let content: Element<'a, Message> = if let Some(tabs) = tabs {
        column![tabs, settings_content]
            .spacing(inspector::MODAL_SECTION_SPACING)
            .into()
    } else {
        settings_content
    };

    content
}

fn indicator_settings_tabs<'a>(
    selected_tab: IndicatorSettingsTab,
    instance_id: data::chart::kline::IndicatorInstanceId,
) -> Element<'a, Message> {
    let tab_button = move |label, tab| {
        let is_selected = selected_tab == tab;
        let button = button(crate::widget::text::emphasis(label).align_x(iced::Alignment::Center))
            .padding([4, 10])
            .style(move |theme, status| style::button::transparent(theme, status, is_selected))
            .width(iced::Length::Fill);

        if is_selected {
            button
        } else {
            button.on_press(Message::Indicator {
                instance_id,
                message: KlineIndicatorSettingsMessage::TabSelected(tab),
            })
        }
    };

    column![
        container(
            row![
                tab_button("Inputs", IndicatorSettingsTab::Inputs),
                tab_button("Style", IndicatorSettingsTab::Style),
            ]
            .spacing(8)
        )
        .center_x(Length::Fill),
    ]
    .spacing(4)
    .into()
}

fn indicator_settings_content<'a>(
    settings: Vec<IndicatorSetting>,
    config: IndicatorConfig,
    modal: &'a IndicatorSettingsState,
) -> Element<'a, Message> {
    let mut inputs = Vec::new();
    let mut grouped_settings: Option<(&'static str, Vec<Element<'a, Message>>)> = None;

    for setting in settings {
        let group = setting.group;
        let section = indicator_setting_view(config, setting, modal).into_element();

        match group {
            Some(group) => {
                if let Some((active_group, sections)) = grouped_settings.as_mut()
                    && *active_group == group
                {
                    sections.push(section);
                    continue;
                }

                if let Some((_, sections)) = grouped_settings.take() {
                    inputs.push(inspector::modal_container(column(sections).spacing(12)));
                }
                grouped_settings = Some((group, vec![section]));
            }
            None => {
                if let Some((_, sections)) = grouped_settings.take() {
                    inputs.push(inspector::modal_container(column(sections).spacing(12)));
                }
                inputs.push(inspector::modal_container(section));
            }
        }
    }

    if let Some((_, sections)) = grouped_settings {
        inputs.push(inspector::modal_container(column(sections).spacing(12)));
    }

    column(inputs)
        .spacing(inspector::MODAL_SECTION_SPACING)
        .into()
}

fn indicator_setting_view<'a>(
    config: IndicatorConfig,
    setting: IndicatorSetting,
    modal: &'a IndicatorSettingsState,
) -> InspectorSection<'a, Message> {
    let IndicatorSetting {
        id,
        label,
        value,
        control,
        ..
    } = setting;
    let instance_id = modal.instance_id();
    match (control, value) {
        (IndicatorSettingControl::SliderU16 { min, max }, IndicatorSettingValue::U16(value)) => {
            let input_value = modal
                .input_draft(id)
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string());
            let is_valid = input_value
                .parse::<u16>()
                .is_ok_and(|value| (min..=max).contains(&value));
            let input = text_input("", &input_value)
                .on_input(move |value| {
                    indicator_numeric_input_message(instance_id, config, id, value, move |value| {
                        value
                            .parse::<u16>()
                            .ok()
                            .filter(|value| (min..=max).contains(value))
                            .map(IndicatorSettingValue::U16)
                    })
                })
                .style(move |theme, status| style::validated_text_input(theme, status, is_valid))
                .width(Length::Fill);

            setting_section(label, input)
        }
        (
            IndicatorSettingControl::SliderF32 { min, max, step },
            IndicatorSettingValue::F32(value),
        ) => {
            let input_value = modal
                .input_draft(id)
                .map(str::to_owned)
                .unwrap_or_else(|| format_indicator_f32(value, step));
            let is_valid = input_value
                .parse::<f32>()
                .is_ok_and(|value| value.is_finite() && (min..=max).contains(&value));
            let input = text_input("", &input_value)
                .on_input(move |value| {
                    indicator_numeric_input_message(instance_id, config, id, value, move |value| {
                        value
                            .parse::<f32>()
                            .ok()
                            .filter(|value| value.is_finite() && (min..=max).contains(value))
                            .map(IndicatorSettingValue::F32)
                    })
                })
                .style(move |theme, status| style::validated_text_input(theme, status, is_valid))
                .width(Length::Fill);

            setting_section(label, input)
        }
        (IndicatorSettingControl::Toggle, IndicatorSettingValue::Bool(value)) => {
            let toggle = checkbox(value).on_toggle(move |value| {
                indicator_setting_message(
                    instance_id,
                    config,
                    id,
                    IndicatorSettingValue::Bool(value),
                )
            });

            setting_section(label, toggle)
        }
        (IndicatorSettingControl::Choices(choices), IndicatorSettingValue::Choice(selected)) => {
            if choices.len() > 3 {
                let selected = choices.iter().copied().find(|choice| choice.id == selected);
                let picker = pick_list(choices, selected, move |choice| {
                    indicator_setting_message(
                        instance_id,
                        config,
                        id,
                        IndicatorSettingValue::Choice(choice.id),
                    )
                })
                .width(Length::Fill);

                return setting_section(label, picker);
            }

            let mut controls = column![].spacing(4);
            for choice in choices {
                controls = controls.push(radio(
                    choice.label,
                    choice.id,
                    Some(selected),
                    move |value| {
                        indicator_setting_message(
                            instance_id,
                            config,
                            id,
                            IndicatorSettingValue::Choice(value),
                        )
                    },
                ));
            }

            setting_section(label, controls)
        }
        _ => setting_section(label, text("")),
    }
}

fn format_indicator_f32(value: f32, step: f32) -> String {
    if step >= 1.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

fn indicator_numeric_input_message(
    instance_id: data::chart::kline::IndicatorInstanceId,
    config: IndicatorConfig,
    id: &'static str,
    value: String,
    parse_value: impl Fn(&str) -> Option<IndicatorSettingValue>,
) -> Message {
    let config = parse_value(&value).and_then(|value| config.with_setting(id, value));
    Message::Indicator {
        instance_id,
        message: KlineIndicatorSettingsMessage::InputChanged { id, value, config },
    }
}

fn indicator_setting_message(
    instance_id: data::chart::kline::IndicatorInstanceId,
    config: IndicatorConfig,
    id: &'static str,
    value: IndicatorSettingValue,
) -> Message {
    let config = config
        .with_setting(id, value)
        .expect("indicator setting schema must match its configuration");
    Message::Indicator {
        instance_id,
        message: KlineIndicatorSettingsMessage::DataConfig(config),
    }
}

fn indicator_channel_style_section<'a>(
    instance_id: data::chart::kline::IndicatorInstanceId,
    indicator: KlineIndicator,
    color_config: IndicatorColorConfig,
    style_config: IndicatorStyleConfig,
    mark: Option<MarkKind>,
    theme_accents: ThemeAccents,
) -> Element<'a, Message> {
    let active_slots = indicator.active_color_slots(mark);
    let supports_line_styling = mark.is_none_or(MarkKind::supports_line_styling);
    let line_channels = indicator.line_channels();

    let controls = indicator
        .color_specs()
        .iter()
        .filter(|spec| active_slots.is_none_or(|slots| slots.contains(&spec.slot)))
        .map(|spec| {
            let selected = color_config.color_for(spec.slot);
            let send_color = move |accent| Message::Indicator {
                instance_id,
                message: KlineIndicatorSettingsMessage::Color {
                    slot: spec.slot,
                    accent,
                    shade: selected.shade,
                },
            };
            let color_choices = theme_color_control(selected, theme_accents, send_color);
            let shade_picklist =
                pick_list(ThemeColorShade::ALL, Some(selected.shade), move |shade| {
                    Message::Indicator {
                        instance_id,
                        message: KlineIndicatorSettingsMessage::Color {
                            slot: spec.slot,
                            accent: selected.accent,
                            shade,
                        },
                    }
                })
                .width(Length::Fill);

            let mut controls = column![color_choices].width(Length::Fill).spacing(4);

            let has_line_style = supports_line_styling
                && line_channels
                    .iter()
                    .any(|channel| channel.color_slot == spec.slot);
            if has_line_style {
                let slot = spec.slot;
                let width_slider = labeled_slider(
                    "Line width",
                    MIN_LINE_WIDTH..=MAX_LINE_WIDTH,
                    style_config.line_width_for(slot),
                    move |width| Message::Indicator {
                        instance_id,
                        message: KlineIndicatorSettingsMessage::LineWidth { slot, width },
                    },
                    |value| value.to_string(),
                    Some(1),
                );
                let stroke_picker = pick_list(
                    LineStrokeStyle::ALL,
                    Some(style_config.stroke_style_for(slot)),
                    move |stroke_style| Message::Indicator {
                        instance_id,
                        message: KlineIndicatorSettingsMessage::StrokeStyle { slot, stroke_style },
                    },
                )
                .width(Length::Fill);

                controls = controls
                    .push(
                        row![shade_picklist, stroke_picker]
                            .width(Length::Fill)
                            .spacing(4),
                    )
                    .push(width_slider);
            } else {
                controls = controls.push(shade_picklist);
            }

            settings_row(spec.label, controls)
        });

    column(controls)
        .spacing(inspector::MODAL_SECTION_SPACING)
        .into()
}

fn indicator_fill_section<'a>(
    instance_id: data::chart::kline::IndicatorInstanceId,
    indicator: KlineIndicator,
    style_config: IndicatorStyleConfig,
) -> Option<InspectorSection<'a, Message>> {
    if indicator.fill_specs().is_empty() {
        return None;
    }

    let fill_visibility = checkbox(style_config.fill_visible)
        .label("Show background fill")
        .on_toggle(move |visible| Message::Indicator {
            instance_id,
            message: KlineIndicatorSettingsMessage::FillVisible(visible),
        });

    Some(InspectorSection::titled("Fill", fill_visibility).with_modal_container())
}

fn indicator_mark_section<'a>(
    instance_id: data::chart::kline::IndicatorInstanceId,
    indicator: KlineIndicator,
    mark: Option<data::chart::kline::MarkKind>,
    chart: &'a KlineChart,
) -> Option<InspectorSection<'a, Message>> {
    let mark_specs = indicator.mark_specs();
    if mark_specs.is_empty() {
        return None;
    }

    let signed_overlay_available = chart.indicator_produces_signed_overlay(instance_id);
    let selected = mark.or_else(|| mark_specs.first().map(|spec| spec.mark));
    let send_mark = move |value| Message::Indicator {
        instance_id,
        message: KlineIndicatorSettingsMessage::Mark(value),
    };
    let controls = mark_specs
        .iter()
        .filter(|spec| {
            let is_signed_overlay = matches!(
                spec.mark,
                MarkKind::Bar(BarMode::Histogram(HistogramMode::SignedOverlay))
            );
            !is_signed_overlay || signed_overlay_available
        })
        .map(|spec| radio(spec.label, spec.mark, selected, send_mark).into());

    Some(
        InspectorSection::titled("Display mark", column(controls).spacing(4))
            .with_modal_container(),
    )
}
#[derive(Debug, Clone, Copy)]
pub enum CandlestickSettingsMessage {
    Config(CandlestickConfig),
    Mark(MarkKind),
    Reset,
}

pub fn kline_cfg_view(
    footprint: FootprintConfig,
    trade_data_enabled: bool,
    theme_accents: ThemeAccents,
) -> Element<'static, Message> {
    column![
        footprint_section(footprint),
        footprint_imbalance_section(footprint.imbalance, trade_data_enabled, theme_accents),
        footprint_poc_section(footprint.poc, trade_data_enabled, theme_accents),
    ]
    .spacing(inspector::MODAL_SECTION_SPACING)
    .into()
}

pub(super) fn data_settings_content(
    trade_data_enabled: bool,
    basis: data::chart::Basis,
) -> Element<'static, Message> {
    let trade_data_required_for_basis = matches!(basis, data::chart::Basis::Tick(_));
    let trade_data_enabled_checkbox = tooltip(
        checkbox(trade_data_enabled)
            .label("Trade data")
            .on_toggle_maybe(
                (!trade_data_required_for_basis).then_some(Message::TradeDataEnabledChanged),
            ),
        Some(if trade_data_required_for_basis {
            "Trade data is required for tick-based charts"
        } else {
            "Allow Footprint and indicators that need trade data to subscribe and fetch it"
        }),
        TooltipPosition::Top,
    );

    column![trade_data_enabled_checkbox].into()
}

fn footprint_section(config: FootprintConfig) -> Element<'static, Message> {
    let cluster_picklist = pick_list(ClusterKind::ALL, Some(config.cluster_kind), move |kind| {
        Message::ClusterKindSelected(kind)
    });

    let scaling_picklist = pick_list(
        ClusterScaling::ALL,
        Some(config.cluster_scaling),
        Message::ClusterScalingSelected,
    );
    let scaling = if let ClusterScaling::Hybrid { weight } = config.cluster_scaling {
        column![
            scaling_picklist,
            labeled_slider(
                "Visible-range weight",
                0.0..=1.0,
                weight,
                move |weight| Message::ClusterScalingSelected(ClusterScaling::Hybrid { weight }),
                |weight| format!("{:.0}%", weight * 100.0),
                Some(0.05),
            ),
            text("Blend visible-range and per-candle scaling"),
        ]
        .spacing(8)
    } else {
        column![scaling_picklist].spacing(8)
    };

    column![
        InspectorSection::titled("Cluster type", cluster_picklist)
            .with_modal_container()
            .into_element(),
        InspectorSection::titled("Cluster scaling", scaling)
            .with_modal_container()
            .into_element(),
    ]
    .spacing(inspector::MODAL_SECTION_SPACING)
    .into()
}

fn footprint_poc_section(
    config: FootprintPocConfig,
    trade_data_enabled: bool,
    theme_accents: ThemeAccents,
) -> Element<'static, Message> {
    let controls = FootprintPocColorConfig::color_specs().iter().map(|spec| {
        let slot = spec.slot;
        let checked = match slot {
            FootprintPocColorSlot::Poc => config.show_poc,
            FootprintPocColorSlot::Npoc => config.show_npoc,
        };
        let toggle = match slot {
            FootprintPocColorSlot::Poc => checkbox(checked)
                .label(spec.label)
                .spacing(8)
                .text_size(crate::style::text_size::SECTION)
                .on_toggle_maybe(trade_data_enabled.then_some(move |show| {
                    Message::FootprintPocConfigChanged(config.with_show_poc(show))
                })),
            FootprintPocColorSlot::Npoc => checkbox(checked)
                .label(spec.label)
                .spacing(8)
                .text_size(crate::style::text_size::SECTION)
                .on_toggle_maybe(trade_data_enabled.then_some(move |show| {
                    Message::FootprintPocConfigChanged(config.with_show_npoc(show))
                })),
        }
        .width(Length::Fill);
        let toggle = tooltip(
            toggle,
            (!trade_data_enabled).then_some("Enable trade data in pane settings to use."),
            TooltipPosition::Top,
        );

        if !checked {
            return inspector::modal_container(toggle);
        }

        let selected = config.colors.color_for(slot);
        let color_choices = theme_color_control(selected, theme_accents, move |accent| {
            Message::FootprintPocConfigChanged(config.with_colors(config.colors.with_color(
                slot,
                ThemeColor::with_accent(selected.role, accent, selected.shade),
            )))
        });
        let shade_picklist = pick_list(ThemeColorShade::ALL, Some(selected.shade), move |shade| {
            Message::FootprintPocConfigChanged(config.with_colors(config.colors.with_color(
                slot,
                ThemeColor::with_accent(selected.role, selected.accent, shade),
            )))
        })
        .width(Length::Fill);

        let setting = column![color_choices, shade_picklist]
            .width(Length::Fill)
            .spacing(4);

        InspectorSection::new(toggle, setting)
            .with_modal_container()
            .into_element()
    });

    column(controls)
        .spacing(inspector::MODAL_SECTION_SPACING)
        .into()
}

fn footprint_imbalance_section(
    config: FootprintImbalanceConfig,
    trade_data_enabled: bool,
    theme_accents: ThemeAccents,
) -> Element<'static, Message> {
    let toggle = checkbox(config.show)
        .label("Imbalance")
        .spacing(8)
        .text_size(crate::style::text_size::SECTION)
        .on_toggle_maybe(trade_data_enabled.then_some(move |show| {
            Message::FootprintImbalanceConfigChanged(config.with_show(show))
        }))
        .width(Length::Fill);
    let toggle = tooltip(
        toggle,
        (!trade_data_enabled).then_some("Enable trade data in pane settings to use."),
        TooltipPosition::Top,
    );

    if !config.show {
        return inspector::modal_container(toggle);
    }

    let threshold = labeled_slider(
        "Ask:Bid threshold",
        100.0..=800.0,
        config.threshold_percent as f32,
        move |threshold_percent| {
            Message::FootprintImbalanceConfigChanged(
                config.with_threshold_percent(threshold_percent.round() as usize),
            )
        },
        |value| format!("{value:.0}%"),
        Some(25.0),
    );
    let imbalance = InspectorSection::new(toggle, threshold).into_element();
    let ignore_zeros = checkbox(config.ignore_zeros)
        .label("Ignore zeros")
        .on_toggle(move |ignore_zeros| {
            Message::FootprintImbalanceConfigChanged(config.with_ignore_zeros(ignore_zeros))
        });
    let color_choices = theme_color_control(config.color, theme_accents, move |accent| {
        Message::FootprintImbalanceConfigChanged(config.with_color(ThemeColor::with_accent(
            config.color.role,
            accent,
            config.color.shade,
        )))
    });
    let shade_picklist =
        pick_list(
            ThemeColorShade::ALL,
            Some(config.color.shade),
            move |shade| {
                Message::FootprintImbalanceConfigChanged(config.with_color(
                    ThemeColor::with_accent(config.color.role, config.color.accent, shade),
                ))
            },
        )
        .width(Length::Fill);

    let marker_color = InspectorSection::titled(
        "Marker color",
        column![color_choices, shade_picklist].spacing(8),
    )
    .into_element();

    inspector::modal_container(
        split_column![
            imbalance,
            ignore_zeros,
            marker_color;
            spacing = inspector::MODAL_SECTION_SPACING
        ]
        .width(Length::Fill),
    )
}

pub fn footprint_cfg_view(
    footprint: FootprintConfig,
    trade_data_enabled: bool,
    theme_accents: ThemeAccents,
) -> Element<'static, Message> {
    column![
        footprint_section(footprint),
        footprint_imbalance_section(footprint.imbalance, trade_data_enabled, theme_accents),
        footprint_poc_section(footprint.poc, trade_data_enabled, theme_accents),
    ]
    .spacing(inspector::MODAL_SECTION_SPACING)
    .align_x(Alignment::Start)
    .into()
}

fn study_cfg_view(
    study: KlineStudy,
    basis: data::chart::Basis,
    footprint_tick_multiplier: TickMultiplier,
    min_ticksize: MinTicksize,
    theme_accents: ThemeAccents,
) -> Element<'static, Message> {
    study
        .view_config_without_reset(
            basis,
            Some(footprint_tick_multiplier),
            Some(min_ticksize),
            theme_accents,
            study::Message::StudyValueChanged,
        )
        .map(Message::Study)
}

fn candlestick_color_section<'a>(
    config: CandlestickConfig,
    theme_accents: ThemeAccents,
) -> Element<'a, Message> {
    let colors = config.colors;
    let controls = [
        (CandlestickColorSlot::Bullish, "Bullish"),
        (CandlestickColorSlot::Bearish, "Bearish"),
    ]
    .into_iter()
    .map(|(slot, label)| {
        let selected = colors.color_for(slot);
        let color_choices = theme_color_control(selected, theme_accents, move |accent| {
            Message::CandlestickSettings(CandlestickSettingsMessage::Config(config.with_color(
                slot,
                ThemeColor::with_accent(selected.role, accent, selected.shade),
            )))
        });
        let shade_picklist = pick_list(ThemeColorShade::ALL, Some(selected.shade), move |shade| {
            Message::CandlestickSettings(CandlestickSettingsMessage::Config(config.with_color(
                slot,
                ThemeColor::with_accent(selected.role, selected.accent, shade),
            )))
        })
        .width(Length::Fill);

        let controls = column![color_choices, shade_picklist]
            .width(Length::Fill)
            .spacing(4);

        settings_row(label, controls)
    });

    column(controls).spacing(8).into()
}

pub fn candlestick_cfg_view<'a>(
    ticker: TickerInfo,
    config: CandlestickConfig,
    mark: MarkKind,
    series_editor: &'a TickerSeriesEditor,
    base_series: Option<SeriesStyleEntry>,
    theme_accents: ThemeAccents,
) -> Element<'a, Message> {
    let opacity = config.opacity_percent.clamp(
        MIN_CANDLESTICK_OPACITY_PERCENT,
        MAX_CANDLESTICK_OPACITY_PERCENT,
    );
    let opacity_section = labeled_slider(
        "Opacity",
        f32::from(MIN_CANDLESTICK_OPACITY_PERCENT)..=f32::from(MAX_CANDLESTICK_OPACITY_PERCENT),
        f32::from(opacity),
        move |value| {
            Message::CandlestickSettings(CandlestickSettingsMessage::Config(
                config.with_opacity_percent(value.round() as u8),
            ))
        },
        |value| format!("{value:.0}%"),
        Some(5.0),
    );

    let selected_mark = mark;
    let mark_controls = row([("Candles", MarkKind::Candle), ("Line", MarkKind::Line)]
        .into_iter()
        .map(|(label, mark)| {
            radio(label, mark, Some(selected_mark), move |value| {
                Message::CandlestickSettings(CandlestickSettingsMessage::Mark(value))
            })
            .into()
        }))
    .spacing(12);

    let ticker_label = crate::widget::text::body(ticker.ticker.symbol_and_exchange_string());
    let display_setting: Element<'a, Message> = if matches!(mark, MarkKind::Candle) {
        column![mark_controls, opacity_section].spacing(8).into()
    } else {
        mark_controls.into()
    };
    let display_section = InspectorSection::titled("Display mark", display_setting)
        .with_modal_container()
        .into_element();

    let content = if matches!(mark, MarkKind::Candle) {
        column![
            ticker_label,
            display_section,
            candlestick_color_section(config, theme_accents),
        ]
        .spacing(inspector::MODAL_SECTION_SPACING)
        .align_x(Alignment::Start)
    } else {
        let mut content =
            column![ticker_label, display_section].spacing(inspector::MODAL_SECTION_SPACING);
        if let Some(series) = base_series {
            content = content.push(
                InspectorSection::titled(
                    "Line style",
                    series_editor
                        .view(&series, theme_accents)
                        .map(Message::SeriesEditor),
                )
                .with_modal_container()
                .into_element(),
            );
        }
        content
    };

    content.into()
}

pub fn series_cfg_view<'a>(
    ticker: TickerInfo,
    series_editor: &'a TickerSeriesEditor,
    series: Option<SeriesStyleEntry>,
    theme_accents: ThemeAccents,
) -> Element<'a, Message> {
    let mut content = column![crate::widget::text::body(
        ticker.ticker.symbol_and_exchange_string()
    ),]
    .spacing(inspector::MODAL_SECTION_SPACING);

    if let Some(series) = series {
        content = content.push(
            InspectorSection::titled(
                "Line",
                series_editor
                    .view(&series, theme_accents)
                    .map(Message::SeriesEditor),
            )
            .with_modal_container()
            .into_element(),
        );
    }

    content.into()
}
#[derive(Debug, Clone, PartialEq)]
enum SettingsTarget {
    Main,
    Indicator(IndicatorSettingsState),
    Candlestick,
    Series(TickerInfo),
    Footprint,
    Study(KlineStudyId),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct State {
    target: SettingsTarget,
    reset_confirmation: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            target: SettingsTarget::Main,
            reset_confirmation: false,
        }
    }
}

impl State {
    pub(super) fn has_contextual_target(&self, chart: &KlineChart) -> bool {
        match &self.target {
            SettingsTarget::Main => false,
            SettingsTarget::Indicator(modal) => chart.indicator_kind(modal.instance_id()).is_some(),
            SettingsTarget::Study(study_id) => chart.study(*study_id).is_some(),
            SettingsTarget::Series(ticker) => chart.series_style_entry(*ticker).is_some(),
            SettingsTarget::Candlestick | SettingsTarget::Footprint => true,
        }
    }

    pub(super) fn can_reset(&self, chart: &KlineChart) -> bool {
        self.reset_message(chart).is_some()
    }

    pub(super) fn is_confirming_reset(&self) -> bool {
        self.reset_confirmation
    }

    fn reset_message(&self, chart: &KlineChart) -> Option<Message> {
        match &self.target {
            SettingsTarget::Main | SettingsTarget::Footprint => {
                Some(Message::ResetFootprintSettings)
            }
            SettingsTarget::Indicator(modal) => Some(Message::Indicator {
                instance_id: modal.instance_id(),
                message: KlineIndicatorSettingsMessage::Reset,
            }),
            SettingsTarget::Candlestick => Some(Message::CandlestickSettings(
                CandlestickSettingsMessage::Reset,
            )),
            SettingsTarget::Series(ticker) => Some(Message::ResetSeriesSettings(*ticker)),
            SettingsTarget::Study(study_id) => chart.study(*study_id).map(|study| {
                Message::Study(study::Message::StudyValueChanged(
                    study.default_for_type_for_basis(chart.basis()),
                ))
            }),
        }
    }

    fn request_reset_confirmation(&mut self, chart: &KlineChart) {
        self.reset_confirmation = self.can_reset(chart);
    }

    fn cancel_reset_confirmation(&mut self) {
        self.reset_confirmation = false;
    }

    pub(super) fn dismiss_reset_confirmation(&mut self) -> bool {
        if !self.reset_confirmation {
            return false;
        }

        self.cancel_reset_confirmation();
        true
    }

    fn confirmed_reset_message(&mut self, chart: &KlineChart) -> Option<Message> {
        if !self.reset_confirmation {
            return None;
        }

        self.reset_confirmation = false;
        self.reset_message(chart)
    }

    fn set_target(&mut self, target: SettingsTarget) {
        self.target = target;
        self.cancel_reset_confirmation();
    }

    pub(super) fn open_indicator(
        &mut self,
        instance_id: data::chart::kline::IndicatorInstanceId,
        indicator: KlineIndicator,
    ) {
        self.set_target(SettingsTarget::Indicator(IndicatorSettingsState::new(
            instance_id,
            indicator,
        )));
    }

    pub(super) fn open_candlestick(&mut self) {
        self.set_target(SettingsTarget::Candlestick);
    }

    pub(super) fn open_series(&mut self, ticker: TickerInfo) {
        self.set_target(SettingsTarget::Series(ticker));
    }

    pub(super) fn open_footprint(&mut self) {
        self.set_target(SettingsTarget::Footprint);
    }

    pub(super) fn open_study(&mut self, study_id: KlineStudyId) {
        self.set_target(SettingsTarget::Study(study_id));
    }

    pub(super) fn contextual_section<'a>(
        &'a self,
        chart: &'a KlineChart,
        theme_accents: ThemeAccents,
    ) -> Option<InspectorSection<'a, ChartMessage>> {
        let title: Element<'a, ChartMessage> = match &self.target {
            SettingsTarget::Main => return None,
            SettingsTarget::Indicator(modal) => {
                crate::widget::text::section(modal.indicator().to_string()).into()
            }
            SettingsTarget::Candlestick => crate::widget::text::section("Base ticker").into(),
            SettingsTarget::Series(_) => crate::widget::text::section("Comparison ticker").into(),
            SettingsTarget::Footprint => crate::widget::text::section("Footprint").into(),
            SettingsTarget::Study(study_id) => chart
                .study(*study_id)
                .map(|study| -> Element<'a, ChartMessage> {
                    crate::widget::text::section(study.to_string()).into()
                })
                .unwrap_or_else(|| crate::widget::text::section("Study").into()),
        };

        let content = match &self.target {
            SettingsTarget::Main => unreachable!(),
            SettingsTarget::Indicator(modal) => kline_indicator_cfg_view(
                modal,
                chart.indicator_data_config(modal.instance_id()),
                chart.basis(),
                chart
                    .indicator_color_config(modal.instance_id())
                    .unwrap_or_default(),
                chart
                    .indicator_style_config(modal.instance_id())
                    .unwrap_or_default(),
                chart.indicator_mark(modal.instance_id()),
                chart,
                theme_accents,
            ),
            SettingsTarget::Candlestick => candlestick_cfg_view(
                chart.ticker_info(),
                chart.candlestick_config(),
                chart.primary_mark(),
                chart.series_editor(),
                chart.series_style_entry(chart.ticker_info()),
                theme_accents,
            ),
            SettingsTarget::Series(ticker) => series_cfg_view(
                *ticker,
                chart.series_editor(),
                chart.series_style_entry(*ticker),
                theme_accents,
            ),
            SettingsTarget::Footprint => footprint_cfg_view(
                chart.footprint_config(),
                chart.trade_data_enabled(),
                theme_accents,
            ),
            SettingsTarget::Study(study_id) => chart
                .study(*study_id)
                .map(|study| {
                    study_cfg_view(
                        study,
                        chart.basis(),
                        chart.footprint_tick_multiplier(),
                        chart.ticker_info().min_ticksize,
                        theme_accents,
                    )
                })
                .unwrap_or_else(|| {
                    kline_cfg_view(
                        chart.footprint_config(),
                        chart.trade_data_enabled(),
                        theme_accents,
                    )
                }),
        };

        Some(InspectorSection::new(
            title,
            content.map(ChartMessage::Settings),
        ))
    }

    pub(super) fn footer_view<'a>(
        &'a self,
        chart: &'a KlineChart,
    ) -> Option<Element<'a, ChartMessage>> {
        if matches!(self.target, SettingsTarget::Main) {
            return None;
        }

        if !self.can_reset(chart) {
            return None;
        }

        let reset: Element<'_, ChartMessage> = if self.is_confirming_reset() {
            row![
                text("Reset settings?"),
                button(style::icon_text(style::Icon::Checkmark, 12))
                    .style(|theme, status| style::button::confirm(theme, status, true))
                    .on_press(ChartMessage::Settings(Message::ResetConfirmed)),
                button(style::icon_text(style::Icon::Close, 12))
                    .style(|theme, status| style::button::cancel(theme, status, true))
                    .on_press(ChartMessage::Settings(Message::ResetCancelled)),
            ]
            .align_y(Alignment::Center)
            .spacing(4)
            .into()
        } else {
            button(style::icon_text(style::Icon::TrashBin, 12))
                .style(|theme, status| style::button::transparent(theme, status, false))
                .on_press(ChartMessage::Settings(Message::ResetRequested))
                .into()
        };

        Some(
            row![space::horizontal(), reset]
                .align_y(Alignment::Center)
                .into(),
        )
    }

    pub(super) fn update(
        &mut self,
        chart: &mut KlineChart,
        message: Message,
    ) -> Option<ChartAction> {
        if !matches!(
            &message,
            Message::ResetRequested | Message::ResetConfirmed | Message::ResetCancelled
        ) {
            self.cancel_reset_confirmation();
        }

        match message {
            Message::ResetRequested => self.request_reset_confirmation(chart),
            Message::ResetConfirmed => {
                let message = self.confirmed_reset_message(chart)?;
                return self.update(chart, message);
            }
            Message::ResetCancelled => self.cancel_reset_confirmation(),
            Message::Indicator {
                instance_id,
                message,
            } => {
                let command = match message {
                    KlineIndicatorSettingsMessage::TabSelected(tab) => {
                        if let SettingsTarget::Indicator(modal) = &mut self.target
                            && modal.instance_id() == instance_id
                        {
                            modal.select_tab(tab);
                        }
                        None
                    }
                    KlineIndicatorSettingsMessage::InputChanged { id, value, config } => {
                        if let SettingsTarget::Indicator(modal) = &mut self.target
                            && modal.instance_id() == instance_id
                        {
                            modal.set_input_draft(id, value);
                        }
                        config.map(|config| {
                            ChartCommand::Indicator(IndicatorCommand::SetDataConfig {
                                id: instance_id,
                                config,
                            })
                        })
                    }
                    KlineIndicatorSettingsMessage::DataConfig(config) => {
                        Some(ChartCommand::Indicator(IndicatorCommand::SetDataConfig {
                            id: instance_id,
                            config,
                        }))
                    }
                    KlineIndicatorSettingsMessage::Color {
                        slot,
                        accent,
                        shade,
                    } => Some(ChartCommand::Indicator(IndicatorCommand::SetColor {
                        id: instance_id,
                        slot,
                        accent,
                        shade,
                    })),
                    KlineIndicatorSettingsMessage::FillVisible(visible) => Some(
                        ChartCommand::Indicator(IndicatorCommand::SetFillVisibility {
                            id: instance_id,
                            visible,
                        }),
                    ),
                    KlineIndicatorSettingsMessage::LineWidth { slot, width } => {
                        Some(ChartCommand::Indicator(IndicatorCommand::SetLineWidth {
                            id: instance_id,
                            slot,
                            width,
                        }))
                    }
                    KlineIndicatorSettingsMessage::StrokeStyle { slot, stroke_style } => {
                        Some(ChartCommand::Indicator(IndicatorCommand::SetStrokeStyle {
                            id: instance_id,
                            slot,
                            stroke_style,
                        }))
                    }
                    KlineIndicatorSettingsMessage::Mark(mark) => {
                        Some(ChartCommand::Indicator(IndicatorCommand::SetMark {
                            id: instance_id,
                            mark,
                        }))
                    }
                    KlineIndicatorSettingsMessage::Reset => {
                        if let SettingsTarget::Indicator(modal) = &mut self.target
                            && modal.instance_id() == instance_id
                        {
                            modal.clear_input_drafts();
                        }
                        Some(ChartCommand::Indicator(IndicatorCommand::ResetSettings(
                            instance_id,
                        )))
                    }
                };

                if let Some(command) = command
                    && let CommandResult::Action(Some(PaneChartAction::RequestFetch(fetch))) =
                        chart.apply_command(command)
                {
                    return Some(ChartAction::RequestFetch(fetch));
                }
            }
            Message::CandlestickSettings(message) => {
                let command = match message {
                    CandlestickSettingsMessage::Config(config) => {
                        ChartCommand::Candlestick(CandlestickCommand::SetConfig(config))
                    }
                    CandlestickSettingsMessage::Mark(mark) => {
                        ChartCommand::Candlestick(CandlestickCommand::SetMark(mark))
                    }
                    CandlestickSettingsMessage::Reset => {
                        ChartCommand::Candlestick(CandlestickCommand::ResetSettings)
                    }
                };
                let _ = chart.apply_command(command);
            }
            Message::ClusterKindSelected(kind) => {
                let _ = chart.apply_command(ChartCommand::Footprint(
                    FootprintCommand::SetClusterKind(kind),
                ));
            }
            Message::ClusterScalingSelected(scaling) => {
                let _ = chart.apply_command(ChartCommand::Footprint(
                    FootprintCommand::SetClusterScaling(scaling),
                ));
            }
            Message::FootprintImbalanceConfigChanged(config) => {
                let _ = chart.apply_command(ChartCommand::Footprint(
                    FootprintCommand::SetImbalanceConfig(config),
                ));
            }
            Message::FootprintPocConfigChanged(config) => {
                let _ = chart.apply_command(ChartCommand::Footprint(
                    FootprintCommand::SetPocConfig(config),
                ));
            }
            Message::TradeDataEnabledChanged(enabled) => {
                let _ = chart.apply_command(ChartCommand::SetTradeDataEnabled(enabled));
            }
            Message::ResetFootprintSettings => {
                let _ =
                    chart.apply_command(ChartCommand::Footprint(FootprintCommand::ResetSettings));
            }
            Message::ResetSeriesSettings(ticker_info) => {
                let _ = chart.apply_command(ChartCommand::Series(SeriesCommand::ResetSettings {
                    ticker_info,
                }));
            }
            Message::Study(message) => chart.update_study_configurator(message),
            Message::SeriesEditor(message) => {
                return chart.update(ChartMessage::SeriesEditor(message));
            }
        }

        None
    }
}
