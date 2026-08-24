use super::{
    Action as ChartAction, CandlestickCommand, ChartCommand, CommandResult, FootprintCommand,
    IndicatorCommand, KlineChartV2, Message as ChartMessage, SETTINGS_SIDEBAR_HEADER_HEIGHT,
    SeriesCommand,
};
use crate::widget::study_configurator::Study as _;
use crate::{
    split_column, style, tooltip,
    widget::{
        labeled_slider,
        series_editor::{self, SeriesStyleEntry, TickerSeriesEditor},
        study_configurator as study, theme_color_control,
    },
};
use data::chart::kline::ClusterKind;
use data::chart::kline_v2::{
    BarMode, CandlestickColorSlot, CandlestickConfig, FootprintPocColorConfig,
    FootprintPocColorSlot, FootprintPocConfig, HistogramMode, IndicatorColorConfig,
    IndicatorColorSlot, IndicatorConfig, IndicatorSetting, IndicatorSettingControl,
    IndicatorSettingValue, IndicatorSettingsContext, IndicatorStyleConfig,
    KlineIndicator as KlineV2Indicator, KlineStudy, KlineStudyId, LineStrokeStyle,
    MAX_CANDLESTICK_OPACITY_PERCENT, MAX_LINE_WIDTH, MIN_CANDLESTICK_OPACITY_PERCENT,
    MIN_LINE_WIDTH, MarkKind, ThemeAccent,
};
use data::config::theme::{ThemeColor, ThemeColorShade};
use exchange::unit::MinTicksize;
use exchange::{TickMultiplier, TickerInfo};
use iced::{
    Alignment, Element, Length,
    widget::{
        button, checkbox, column, container, pick_list, radio, row, rule, space, text, text_input,
        tooltip::Position as TooltipPosition,
    },
};

#[derive(Debug, Clone)]
pub enum Message {
    ResetRequested,
    ResetConfirmed,
    ResetCancelled,
    ClusterKindSelected(ClusterKind),
    FootprintPocConfigChanged(FootprintPocConfig),
    TradeDataEnabledChanged(bool),
    ResetFootprintSettings,
    ResetSeriesSettings(TickerInfo),
    Study(study::Message<KlineStudy>),
    Indicator {
        instance_id: data::chart::kline_v2::IndicatorInstanceId,
        message: KlineV2IndicatorSettingsMessage,
    },
    CandlestickSettings(CandlestickSettingsMessage),
    SeriesEditor(series_editor::Message),
}

fn cfg_view_container<'a, M, T>(content: T) -> Element<'a, M>
where
    M: 'a,
    T: Into<Element<'a, M>>,
{
    container(crate::widget::scrollable_content(content))
        .width(Length::Fill)
        .height(Length::Fill)
        .padding([4, 0])
        .into()
}

fn settings_row<'a, M>(label: &'a str, control: impl Into<Element<'a, M>>) -> Element<'a, M>
where
    M: 'a,
{
    column![
        text(label).size(crate::style::text_size::SECTION),
        container(control).width(Length::Fill),
    ]
    .width(Length::Fill)
    .spacing(4)
    .into()
}

#[derive(Debug, Clone)]
pub enum KlineV2IndicatorSettingsMessage {
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
    instance_id: data::chart::kline_v2::IndicatorInstanceId,
    indicator: data::chart::kline_v2::KlineIndicator,
    tab: IndicatorSettingsTab,
    input_drafts: Vec<(&'static str, String)>,
}

impl IndicatorSettingsState {
    pub fn new(
        instance_id: data::chart::kline_v2::IndicatorInstanceId,
        indicator: data::chart::kline_v2::KlineIndicator,
    ) -> Self {
        Self {
            instance_id,
            indicator,
            tab: IndicatorSettingsTab::Inputs,
            input_drafts: Vec::new(),
        }
    }

    pub fn instance_id(&self) -> data::chart::kline_v2::IndicatorInstanceId {
        self.instance_id
    }

    pub fn indicator(&self) -> data::chart::kline_v2::KlineIndicator {
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

pub fn kline_v2_indicator_cfg_view<'a>(
    modal: &'a IndicatorSettingsState,
    data_config: Option<IndicatorConfig>,
    basis: data::chart::Basis,
    color_config: IndicatorColorConfig,
    style_config: IndicatorStyleConfig,
    mark: Option<data::chart::kline_v2::MarkKind>,
    chart: &'a KlineChartV2,
) -> Element<'a, Message> {
    let indicator = modal.indicator();
    let instance_id = modal.instance_id();

    let mark_section = indicator_mark_section(instance_id, indicator, mark, chart);
    let fill_section = indicator_fill_section(instance_id, indicator, style_config);
    let channel_style_section =
        indicator_channel_style_section(instance_id, indicator, color_config, style_config, mark);

    let title = container(text(format!("{}", indicator)).size(crate::style::text_size::SECTION))
        .center_x(Length::Fill);

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
            let mut inputs = column![].spacing(12);
            let config = data_config.expect("input settings require an indicator configuration");
            for (index, setting) in input_settings.into_iter().enumerate() {
                if index > 0 {
                    inputs = inputs.push(rule::horizontal(1.0).style(style::split_ruler));
                }

                inputs = inputs.push(indicator_setting_view(indicator, config, setting, modal));
            }
            inputs.into()
        }
        IndicatorSettingsTab::Style => {
            let mut style_settings = column![].spacing(16);
            for (index, section) in [mark_section, fill_section, Some(channel_style_section)]
                .into_iter()
                .flatten()
                .enumerate()
            {
                if index > 0 {
                    style_settings =
                        style_settings.push(rule::horizontal(1.0).style(style::split_ruler));
                }

                style_settings = style_settings.push(section);
            }
            style_settings.into()
        }
    };

    let content = if let Some(tabs) = tabs {
        column![title, tabs, settings_content].spacing(16)
    } else {
        column![title, settings_content].spacing(16)
    };

    cfg_view_container(content)
}

fn indicator_settings_tabs<'a>(
    selected_tab: IndicatorSettingsTab,
    instance_id: data::chart::kline_v2::IndicatorInstanceId,
) -> Element<'a, Message> {
    let tab_button = move |label, tab| {
        let is_selected = selected_tab == tab;
        let button = button(text(label).size(crate::style::text_size::EMPHASIS))
            .padding([4, 10])
            .style(move |theme, status| style::button::transparent(theme, status, is_selected));

        if is_selected {
            button
        } else {
            button.on_press(Message::Indicator {
                instance_id,
                message: KlineV2IndicatorSettingsMessage::TabSelected(tab),
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
        rule::horizontal(1).style(style::split_ruler),
    ]
    .spacing(4)
    .into()
}

fn indicator_setting_view<'a>(
    _indicator: KlineV2Indicator,
    config: IndicatorConfig,
    setting: IndicatorSetting,
    modal: &'a IndicatorSettingsState,
) -> Element<'a, Message> {
    let IndicatorSetting {
        id,
        label,
        value,
        control,
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

            settings_row(label, input)
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

            settings_row(label, input)
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

            settings_row(label, toggle)
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

                return settings_row(label, picker);
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

            settings_row(label, controls)
        }
        _ => settings_row(label, text("")),
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
    instance_id: data::chart::kline_v2::IndicatorInstanceId,
    config: IndicatorConfig,
    id: &'static str,
    value: String,
    parse_value: impl Fn(&str) -> Option<IndicatorSettingValue>,
) -> Message {
    let config = parse_value(&value).and_then(|value| config.with_setting(id, value));
    Message::Indicator {
        instance_id,
        message: KlineV2IndicatorSettingsMessage::InputChanged { id, value, config },
    }
}

fn indicator_setting_message(
    instance_id: data::chart::kline_v2::IndicatorInstanceId,
    config: IndicatorConfig,
    id: &'static str,
    value: IndicatorSettingValue,
) -> Message {
    let config = config
        .with_setting(id, value)
        .expect("indicator setting schema must match its configuration");
    Message::Indicator {
        instance_id,
        message: KlineV2IndicatorSettingsMessage::DataConfig(config),
    }
}

fn indicator_channel_style_section<'a>(
    instance_id: data::chart::kline_v2::IndicatorInstanceId,
    indicator: KlineV2Indicator,
    color_config: IndicatorColorConfig,
    style_config: IndicatorStyleConfig,
    mark: Option<MarkKind>,
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
                message: KlineV2IndicatorSettingsMessage::Color {
                    slot: spec.slot,
                    accent,
                    shade: selected.shade,
                },
            };
            let color_choices = theme_color_control(selected, send_color);
            let shade_picklist =
                pick_list(ThemeColorShade::ALL, Some(selected.shade), move |shade| {
                    Message::Indicator {
                        instance_id,
                        message: KlineV2IndicatorSettingsMessage::Color {
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
                        message: KlineV2IndicatorSettingsMessage::LineWidth { slot, width },
                    },
                    |value| value.to_string(),
                    Some(1),
                );
                let stroke_picker = pick_list(
                    LineStrokeStyle::ALL,
                    Some(style_config.stroke_style_for(slot)),
                    move |stroke_style| Message::Indicator {
                        instance_id,
                        message: KlineV2IndicatorSettingsMessage::StrokeStyle {
                            slot,
                            stroke_style,
                        },
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

    column(controls).spacing(16).into()
}

fn indicator_fill_section<'a>(
    instance_id: data::chart::kline_v2::IndicatorInstanceId,
    indicator: KlineV2Indicator,
    style_config: IndicatorStyleConfig,
) -> Option<Element<'a, Message>> {
    if indicator.fill_specs().is_empty() {
        return None;
    }

    let fill_visibility = checkbox(style_config.fill_visible)
        .label("Show background fill")
        .on_toggle(move |visible| Message::Indicator {
            instance_id,
            message: KlineV2IndicatorSettingsMessage::FillVisible(visible),
        });

    Some(fill_visibility.into())
}

fn indicator_mark_section<'a>(
    instance_id: data::chart::kline_v2::IndicatorInstanceId,
    indicator: KlineV2Indicator,
    mark: Option<data::chart::kline_v2::MarkKind>,
    chart: &'a KlineChartV2,
) -> Option<Element<'a, Message>> {
    let mark_specs = indicator.mark_specs();
    if mark_specs.is_empty() {
        return None;
    }

    let signed_overlay_available = chart.indicator_produces_signed_overlay(instance_id);
    let selected = mark.or_else(|| mark_specs.first().map(|spec| spec.mark));
    let send_mark = move |value| Message::Indicator {
        instance_id,
        message: KlineV2IndicatorSettingsMessage::Mark(value),
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
        column![
            text("Display mark").size(crate::style::text_size::SECTION),
            column(controls).spacing(4),
        ]
        .spacing(8)
        .into(),
    )
}
#[derive(Debug, Clone, Copy)]
pub enum CandlestickSettingsMessage {
    Config(CandlestickConfig),
    Mark(MarkKind),
    Reset,
}

pub fn kline_v2_cfg_view(
    cluster_kind: ClusterKind,
    footprint_poc_config: FootprintPocConfig,
    trade_data_enabled: bool,
) -> Element<'static, Message> {
    cfg_view_container(
        column![
            text("Chart settings").size(crate::style::text_size::TITLE),
            footprint_section(cluster_kind),
            footprint_poc_section(footprint_poc_config, trade_data_enabled),
        ]
        .spacing(16),
    )
}

pub(super) fn kline_v2_modal_view(chart: &KlineChartV2) -> Element<'static, ChartMessage> {
    let content: Element<'static, Message> =
        trade_data_section(chart.trade_data_enabled(), chart.basis());
    let content = content.map(ChartMessage::Settings);

    container(crate::widget::scrollable_content(content))
        .max_width(280)
        .max_height(420)
        .padding(16)
        .style(style::chart_modal)
        .into()
}

fn trade_data_section(
    trade_data_enabled: bool,
    basis: data::chart::Basis,
) -> Element<'static, Message> {
    let trade_data_required_for_basis = matches!(basis, data::chart::Basis::Tick(_));
    let trade_data_enabled_checkbox = tooltip(
        checkbox(trade_data_enabled)
            .label("Enable trade data")
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

    column![
        text("Data").size(crate::style::text_size::SECTION),
        trade_data_enabled_checkbox,
    ]
    .spacing(8)
    .into()
}

fn footprint_section(cluster_kind: ClusterKind) -> Element<'static, Message> {
    let cluster_picklist = pick_list(ClusterKind::ALL, Some(cluster_kind), move |kind| {
        Message::ClusterKindSelected(kind)
    });

    column![
        text("Cluster type").size(crate::style::text_size::SECTION),
        cluster_picklist
    ]
    .spacing(8)
    .into()
}

fn footprint_poc_section(
    config: FootprintPocConfig,
    trade_data_enabled: bool,
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
            (!trade_data_enabled).then_some("Enable trade data to use Footprint POC/NPOC"),
            TooltipPosition::Top,
        );

        if !checked {
            column![toggle].spacing(4).into()
        } else {
            let selected = config.colors.color_for(slot);
            let color_choices = theme_color_control(selected, move |accent| {
                Message::FootprintPocConfigChanged(config.with_colors(config.colors.with_color(
                    slot,
                    ThemeColor::with_accent(selected.role, accent, selected.shade),
                )))
            });
            let shade_picklist =
                pick_list(ThemeColorShade::ALL, Some(selected.shade), move |shade| {
                    Message::FootprintPocConfigChanged(config.with_colors(
                        config.colors.with_color(
                            slot,
                            ThemeColor::with_accent(selected.role, selected.accent, shade),
                        ),
                    ))
                })
                .width(Length::Fill);

            column![toggle, color_choices, shade_picklist]
                .width(Length::Fill)
                .spacing(4)
                .into()
        }
    });

    column(controls).spacing(16).into()
}

pub fn footprint_cfg_view(
    cluster_kind: ClusterKind,
    footprint_poc_config: FootprintPocConfig,
    trade_data_enabled: bool,
) -> Element<'static, Message> {
    cfg_view_container(split_column![
        footprint_section(cluster_kind),
        footprint_poc_section(footprint_poc_config, trade_data_enabled);
        spacing = 16, align_x = Alignment::Start
    ])
}

fn study_cfg_view(
    study: KlineStudy,
    basis: data::chart::Basis,
    footprint_tick_multiplier: TickMultiplier,
    min_ticksize: MinTicksize,
) -> Element<'static, Message> {
    let content = study
        .view_config_without_reset(
            basis,
            Some(footprint_tick_multiplier),
            Some(min_ticksize),
            study::Message::StudyValueChanged,
        )
        .map(Message::Study);

    cfg_view_container(
        column![
            container(text(study.to_string()).size(crate::style::text_size::SECTION))
                .center_x(Length::Fill),
            content,
        ]
        .spacing(12),
    )
}

fn candlestick_color_section<'a>(config: CandlestickConfig) -> Element<'a, Message> {
    let colors = config.colors;
    let controls = [
        (CandlestickColorSlot::Bullish, "Bullish"),
        (CandlestickColorSlot::Bearish, "Bearish"),
    ]
    .into_iter()
    .map(|(slot, label)| {
        let selected = colors.color_for(slot);
        let color_choices = theme_color_control(selected, move |accent| {
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
    config: CandlestickConfig,
    mark: MarkKind,
    series_editor: &'a TickerSeriesEditor,
    base_series: Option<SeriesStyleEntry>,
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
    let mark_controls = [("Candles", MarkKind::Candle), ("Line", MarkKind::Line)]
        .into_iter()
        .map(|(label, mark)| {
            radio(label, mark, Some(selected_mark), move |value| {
                Message::CandlestickSettings(CandlestickSettingsMessage::Mark(value))
            })
            .into()
        });

    let display_section = column![
        text("Display mark").size(crate::style::text_size::SECTION),
        row(mark_controls).spacing(12),
    ]
    .spacing(8);

    let content = if matches!(mark, MarkKind::Candle) {
        split_column![
            display_section,
            candlestick_color_section(config),
            opacity_section;
            spacing = 16, align_x = Alignment::Start
        ]
    } else {
        let mut content = column![display_section].spacing(16);
        if let Some(series) = base_series {
            content = content.push(
                column![
                    text("Line style").size(crate::style::text_size::SECTION),
                    series_editor
                        .view(&series)
                        .map(move |message| { Message::SeriesEditor(message) }),
                ]
                .spacing(8),
            );
        }
        content
    };

    cfg_view_container(content)
}

pub fn series_cfg_view<'a>(
    ticker: TickerInfo,
    series_editor: &'a TickerSeriesEditor,
    series: Option<SeriesStyleEntry>,
) -> Element<'a, Message> {
    let mut content = column![
        text(ticker.ticker.symbol_and_exchange_string()).size(crate::style::text_size::BODY),
    ]
    .spacing(16);

    if let Some(series) = series {
        content = content.push(series_editor.view(&series).map(Message::SeriesEditor));
    }

    cfg_view_container(content)
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

const COMPACT_SIDEBAR_WIDTH: f32 = 180.0;
const CHART_SETTINGS_SIDEBAR_WIDTH: f32 = 270.0;

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
    pub(super) fn sidebar_width(&self) -> f32 {
        match self.target {
            SettingsTarget::Main | SettingsTarget::Footprint | SettingsTarget::Study(_) => {
                CHART_SETTINGS_SIDEBAR_WIDTH
            }
            SettingsTarget::Indicator(_)
            | SettingsTarget::Candlestick
            | SettingsTarget::Series(_) => COMPACT_SIDEBAR_WIDTH,
        }
    }

    pub(super) fn sidebar_title(&self) -> &'static str {
        match self.target {
            SettingsTarget::Main => "Chart",
            SettingsTarget::Indicator(_) => "Indicator",
            SettingsTarget::Candlestick => "Candles",
            SettingsTarget::Series(_) => "Series",
            SettingsTarget::Footprint => "Footprint",
            SettingsTarget::Study(_) => "Study",
        }
    }

    pub(super) fn can_reset(&self, chart: &KlineChartV2) -> bool {
        self.reset_message(chart).is_some()
    }

    pub(super) fn is_confirming_reset(&self) -> bool {
        self.reset_confirmation
    }

    fn reset_message(&self, chart: &KlineChartV2) -> Option<Message> {
        match &self.target {
            SettingsTarget::Main | SettingsTarget::Footprint => {
                Some(Message::ResetFootprintSettings)
            }
            SettingsTarget::Indicator(modal) => Some(Message::Indicator {
                instance_id: modal.instance_id(),
                message: KlineV2IndicatorSettingsMessage::Reset,
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

    fn request_reset_confirmation(&mut self, chart: &KlineChartV2) {
        self.reset_confirmation = self.can_reset(chart);
    }

    fn cancel_reset_confirmation(&mut self) {
        self.reset_confirmation = false;
    }

    fn confirmed_reset_message(&mut self, chart: &KlineChartV2) -> Option<Message> {
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

    pub(super) fn open_main(&mut self) {
        self.set_target(SettingsTarget::Main);
    }

    pub(super) fn open_indicator(
        &mut self,
        instance_id: data::chart::kline_v2::IndicatorInstanceId,
        indicator: KlineV2Indicator,
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

    pub(super) fn view<'a>(&'a self, chart: &'a KlineChartV2) -> Element<'a, ChartMessage> {
        let content = match &self.target {
            SettingsTarget::Main => kline_v2_cfg_view(
                chart.footprint_cluster_kind(),
                chart.footprint_poc_config(),
                chart.trade_data_enabled(),
            ),
            SettingsTarget::Indicator(modal) => kline_v2_indicator_cfg_view(
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
            ),
            SettingsTarget::Candlestick => candlestick_cfg_view(
                chart.candlestick_config(),
                chart.primary_mark(),
                chart.series_editor(),
                chart.series_style_entry(chart.ticker_info()),
            ),
            SettingsTarget::Series(ticker) => series_cfg_view(
                *ticker,
                chart.series_editor(),
                chart.series_style_entry(*ticker),
            ),
            SettingsTarget::Footprint => footprint_cfg_view(
                chart.footprint_cluster_kind(),
                chart.footprint_poc_config(),
                chart.trade_data_enabled(),
            ),
            SettingsTarget::Study(study_id) => chart
                .study(*study_id)
                .map(|study| {
                    study_cfg_view(
                        study,
                        chart.basis(),
                        chart.footprint_tick_multiplier(),
                        chart.ticker_info().min_ticksize,
                    )
                })
                .unwrap_or_else(|| {
                    kline_v2_cfg_view(
                        chart.footprint_cluster_kind(),
                        chart.footprint_poc_config(),
                        chart.trade_data_enabled(),
                    )
                }),
        };

        content.map(ChartMessage::Settings)
    }

    pub(super) fn update(
        &mut self,
        chart: &mut KlineChartV2,
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
                    KlineV2IndicatorSettingsMessage::TabSelected(tab) => {
                        if let SettingsTarget::Indicator(modal) = &mut self.target
                            && modal.instance_id() == instance_id
                        {
                            modal.select_tab(tab);
                        }
                        None
                    }
                    KlineV2IndicatorSettingsMessage::InputChanged { id, value, config } => {
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
                    KlineV2IndicatorSettingsMessage::DataConfig(config) => {
                        Some(ChartCommand::Indicator(IndicatorCommand::SetDataConfig {
                            id: instance_id,
                            config,
                        }))
                    }
                    KlineV2IndicatorSettingsMessage::Color {
                        slot,
                        accent,
                        shade,
                    } => Some(ChartCommand::Indicator(IndicatorCommand::SetColor {
                        id: instance_id,
                        slot,
                        accent,
                        shade,
                    })),
                    KlineV2IndicatorSettingsMessage::FillVisible(visible) => Some(
                        ChartCommand::Indicator(IndicatorCommand::SetFillVisibility {
                            id: instance_id,
                            visible,
                        }),
                    ),
                    KlineV2IndicatorSettingsMessage::LineWidth { slot, width } => {
                        Some(ChartCommand::Indicator(IndicatorCommand::SetLineWidth {
                            id: instance_id,
                            slot,
                            width,
                        }))
                    }
                    KlineV2IndicatorSettingsMessage::StrokeStyle { slot, stroke_style } => {
                        Some(ChartCommand::Indicator(IndicatorCommand::SetStrokeStyle {
                            id: instance_id,
                            slot,
                            stroke_style,
                        }))
                    }
                    KlineV2IndicatorSettingsMessage::Mark(mark) => {
                        Some(ChartCommand::Indicator(IndicatorCommand::SetMark {
                            id: instance_id,
                            mark,
                        }))
                    }
                    KlineV2IndicatorSettingsMessage::Reset => {
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
                    && let CommandResult::Action(Some(crate::chart::Action::RequestFetch(fetch))) =
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
            Message::FootprintPocConfigChanged(config) => {
                let _ = chart.apply_command(ChartCommand::Footprint(
                    FootprintCommand::SetPocConfig(config),
                ));
            }
            Message::TradeDataEnabledChanged(enabled) => {
                let _ = chart.apply_command(ChartCommand::Footprint(
                    FootprintCommand::SetTradeDataEnabled(enabled),
                ));
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

    pub(super) fn sidebar_view<'a>(&'a self, chart: &'a KlineChartV2) -> Element<'a, ChartMessage> {
        let close = button(style::icon_text(style::Icon::Close, 14))
            .style(|theme, status| style::button::cancel(theme, status, false))
            .padding([4, 8])
            .on_press(ChartMessage::CloseSettingsSidebar);
        let header = row![
            text(self.sidebar_title()).size(style::text_size::TITLE),
            space::horizontal(),
            close,
        ]
        .align_y(Alignment::Center)
        .height(Length::Fixed(SETTINGS_SIDEBAR_HEADER_HEIGHT))
        .spacing(8);

        let settings_content = self.view(chart);
        let settings_content = if self.can_reset(chart) {
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
            column![
                settings_content,
                rule::horizontal(1.0),
                container(
                    row![space::horizontal(), reset]
                        .align_y(Alignment::Center)
                        .padding([4, 0]),
                )
                .width(Length::Fill),
            ]
            .height(Length::Fill)
            .into()
        } else {
            settings_content
        };

        container(column![header, rule::horizontal(1.0), settings_content,])
            .width(Length::Fixed(self.sidebar_width()))
            .height(Length::Fill)
            .padding([4, 8])
            .style(style::chart_sidebar_container)
            .into()
    }
}
