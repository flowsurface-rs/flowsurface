use super::layout::IndicatorState;
use super::studies::StudyState;
use super::{ChartCommand, IndicatorInputCommand, IndicatorStyleCommand, Message as ChartMessage};
use crate::widget::chart::kline::{IndicatorPresentationContext, ResolvedIndicatorStyleSlot};
use crate::widget::study_configurator::Study as _;
use crate::{
    split_column, style,
    theme::ThemeAccents,
    tooltip,
    widget::{
        classic_slider_row,
        inspector::{self, InspectorSection},
        series_editor::{self, SeriesStyleEntry},
        study_configurator as study, theme_color_control,
    },
};
use data::chart::kline::KlineData;
use data::chart::kline::footprint::{ClusterKind, ClusterScaling};
use data::chart::kline::{
    BaseSeriesSource, CandlestickColorSlot, CandlestickConfig, FootprintConfig,
    FootprintImbalanceConfig, FootprintPocColorConfig, FootprintPocColorSlot, FootprintPocConfig,
    IndicatorAuthor, IndicatorConfig, IndicatorId, IndicatorOutputRef, IndicatorSetting,
    IndicatorSettingControl, IndicatorSettingValue, IndicatorSettingsContext,
    IndicatorStyleOverrides, KlineStudy, KlineStudyId, LegendVisibility, LineStrokeStyle,
    MAX_CANDLESTICK_OPACITY_PERCENT, MAX_LINE_WIDTH, MIN_CANDLESTICK_OPACITY_PERCENT,
    MIN_LINE_WIDTH, MarkKind, ThemeAccent,
};
use data::config::theme::{ThemeColor, ThemeColorShade};
use exchange::TickerInfo;
use iced::{
    Alignment, Element, Length,
    widget::{
        button, checkbox, column, pick_list, radio, row, space, text, text_input,
        tooltip::Position as TooltipPosition,
    },
};
use std::fmt::{Display, Formatter};

pub(super) struct SettingsContext<'a> {
    pub(super) basis: data::chart::Basis,
    pub(super) options: &'a ChartOptions,
    pub(super) data: &'a KlineData,
    pub(super) indicators: &'a IndicatorState,
    pub(super) studies: &'a StudyState,
    pub(super) trade_data_enabled: bool,
}

impl<'a> SettingsContext<'a> {
    pub(super) fn new(
        basis: data::chart::Basis,
        options: &'a ChartOptions,
        data: &'a KlineData,
        indicators: &'a IndicatorState,
        studies: &'a StudyState,
        trade_data_enabled: bool,
    ) -> Self {
        Self {
            basis,
            options,
            data,
            indicators,
            studies,
            trade_data_enabled,
        }
    }

    fn presentation(&self) -> IndicatorPresentationContext {
        self.indicators.presentation_context()
    }

    fn series_style_entry(&self, ticker: TickerInfo) -> Option<SeriesStyleEntry> {
        self.data
            .series()
            .iter()
            .find(|series| series.ticker_info == ticker)
            .map(|series| SeriesStyleEntry {
                ticker: series.ticker_info,
                color: series.color(),
                line_width: series.line_width(),
                stroke_style: series.stroke_style(),
            })
    }

    fn indicator_composition_settings(
        &self,
        id: data::chart::kline::IndicatorInstanceId,
    ) -> IndicatorCompositionSettings {
        IndicatorCompositionSettings {
            source_settings: self.indicator_source_settings(id),
        }
    }

    fn indicator_source_settings(
        &self,
        id: data::chart::kline::IndicatorInstanceId,
    ) -> Vec<IndicatorSourceSetting> {
        let Some(instance) = self.indicators.indicator(id) else {
            return Vec::new();
        };

        instance
            .kind
            .definition()
            .source_inputs
            .iter()
            .map(|input| {
                let (selected, choices) = if let Some(source) =
                    instance.source_bindings.get(&input.id)
                {
                    let Some(source_instance) = self.indicators.indicator(source.instance_id)
                    else {
                        return IndicatorSourceSetting {
                            input_id: input.id.clone(),
                            label: input.label.clone(),
                            selected: None,
                            choices: Vec::new(),
                        };
                    };
                    let choices = source_instance
                        .kind
                        .output_channels()
                        .iter()
                        .filter(|output| {
                            source_instance
                                .kind
                                .definition()
                                .sourceable_numeric_output(&output.id)
                                .is_some()
                        })
                        .map(|output| IndicatorSourceChoice {
                            label: format!(
                                "{} · {}",
                                source_instance.kind.definition().display_name,
                                output.label
                            ),
                            selection: IndicatorSourceSelection::Indicator(
                                IndicatorOutputRef::new(source.instance_id, output.id.clone()),
                            ),
                        })
                        .collect::<Vec<_>>();
                    let selected = choices
                        .iter()
                        .find(|choice| {
                            choice.selection == IndicatorSourceSelection::Indicator(source.clone())
                        })
                        .cloned();
                    (selected, choices)
                } else {
                    let mut choices = vec![IndicatorSourceChoice {
                        label: format!("{} (default)", input.default_label),
                        selection: IndicatorSourceSelection::Declared,
                    }];
                    choices.extend(
                        BaseSeriesSource::ALL
                            .into_iter()
                            .filter(|source| source.label() != input.default_label.as_str())
                            .map(|source| IndicatorSourceChoice {
                                label: source.label().to_owned(),
                                selection: IndicatorSourceSelection::Base(source),
                            }),
                    );
                    let selected = choices
                        .iter()
                        .find(
                            |choice| match instance.base_source_selections.get(&input.id) {
                                Some(source) => {
                                    choice.selection == IndicatorSourceSelection::Base(*source)
                                }
                                None => choice.selection == IndicatorSourceSelection::Declared,
                            },
                        )
                        .cloned();
                    (selected, choices)
                };

                IndicatorSourceSetting {
                    input_id: input.id.clone(),
                    label: input.label.clone(),
                    selected,
                    choices,
                }
            })
            .collect()
    }

    fn indicator_attachment_choices(
        &self,
        source_id: data::chart::kline::IndicatorInstanceId,
    ) -> Vec<IndicatorAttachmentChoice> {
        let Some(source_instance) = self.indicators.indicator(source_id) else {
            return Vec::new();
        };
        let Some(source_panel_id) = self.indicators.composition.indicator_panel_id(source_id)
        else {
            return Vec::new();
        };
        if self
            .indicators
            .composition
            .panel(source_panel_id)
            .is_none_or(|panel| {
                panel.presentation != data::chart::kline::composition::PanelPresentation::Plot
            })
        {
            return Vec::new();
        }
        let source_output = source_instance
            .kind
            .output_channels()
            .iter()
            .filter(|output| {
                source_instance
                    .kind
                    .definition()
                    .sourceable_numeric_output(&output.id)
                    .is_some()
            })
            .find(|output| output.id == "value")
            .or_else(|| {
                source_instance
                    .kind
                    .output_channels()
                    .iter()
                    .find(|output| {
                        source_instance
                            .kind
                            .definition()
                            .sourceable_numeric_output(&output.id)
                            .is_some()
                    })
            });
        let Some(source_output) = source_output else {
            return Vec::new();
        };

        IndicatorId::for_market(self.data.base_ticker().market_type())
            .into_iter()
            .filter(|kind| {
                self.indicators
                    .unsupported_reason(kind, self.basis, self.data.base_ticker())
                    .is_none()
                    && kind
                        .definition()
                        .source_inputs
                        .iter()
                        .filter(|input| input.allow_indicator)
                        .count()
                        == 1
                    && matches!(
                        kind.default_view(),
                        data::chart::kline::IndicatorPanelRecipe::Plot { .. }
                    )
                    && kind.definition().has_numeric_plot_layers()
            })
            .filter_map(|kind| {
                let input_id = kind
                    .definition()
                    .source_inputs
                    .iter()
                    .find(|input| input.allow_indicator)?
                    .id
                    .clone();
                Some(IndicatorAttachmentChoice {
                    label: kind.definition().display_name.clone(),
                    kind,
                    input_id,
                    source: IndicatorOutputRef::new(source_id, source_output.id.clone()),
                })
            })
            .collect()
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(super) struct ChartOptions {
    pub(super) trade_data_requested: bool,
    pub(super) legend_visibility: LegendVisibility,
    pub(super) candlestick: CandlestickConfig,
    pub(super) footprint: FootprintConfig,
}

impl ChartOptions {
    pub(super) fn set_trade_data_requested(
        &mut self,
        enabled: bool,
        basis: data::chart::Basis,
    ) -> bool {
        let mut changed = self.trade_data_requested != enabled;
        self.trade_data_requested = enabled;
        if !matches!(basis, data::chart::Basis::Tick(_)) {
            changed |= self.footprint.visible != enabled;
            self.footprint = self.footprint.with_visible(enabled);
        }
        changed
    }
}

#[derive(Default)]
pub(super) struct InspectorState {
    settings: State,
}

pub(super) enum InspectorEffect {
    None,
    Command(ChartCommand),
    StudyConfigurator(study::Message<KlineStudy>),
    OpenUrl(String),
}

impl InspectorState {
    pub(super) fn has_contextual_target(&self) -> bool {
        !matches!(self.settings.target, SettingsTarget::Main)
    }

    pub(super) fn close_contextual_settings(&mut self) {
        self.settings.set_target(SettingsTarget::Main);
    }

    pub(super) fn reconcile_missing_target(&mut self, context: &SettingsContext<'_>) {
        self.settings.reconcile_missing_target(context);
    }

    pub(super) fn is_confirming_reset(&self) -> bool {
        self.settings.reset_confirmation.is_some()
    }

    pub(super) fn dismiss_reset_confirmation(&mut self) -> bool {
        if self.settings.reset_confirmation.is_none() {
            return false;
        }

        self.settings.cancel_reset_confirmation();
        true
    }

    pub(super) fn open_candlestick(&mut self) {
        self.settings.set_target(SettingsTarget::Candlestick);
    }

    pub(super) fn open_series(&mut self, ticker: TickerInfo) {
        self.settings.set_target(SettingsTarget::Series(ticker));
    }

    pub(super) fn open_footprint(&mut self) {
        self.settings.set_target(SettingsTarget::Footprint);
    }

    pub(super) fn open_indicator(
        &mut self,
        instance_id: data::chart::kline::IndicatorInstanceId,
        indicator: IndicatorId,
    ) {
        let mut modal: IndicatorSettingsState = IndicatorSettingsState::new(instance_id, indicator);
        modal.select_tab(self.settings.selected_tab());
        self.settings.set_target(SettingsTarget::Indicator(modal));
    }

    pub(super) fn open_study(&mut self, study_id: KlineStudyId) {
        let mut modal = StudySettingsState::new(study_id);
        modal.select_tab(self.settings.selected_tab());
        self.settings.set_target(SettingsTarget::Study(modal));
    }

    pub(super) fn settings(&self) -> &State {
        &self.settings
    }

    pub(super) fn update(
        &mut self,
        message: Message,
        context: &SettingsContext<'_>,
    ) -> InspectorEffect {
        if let Message::SeriesEditor(message) = message {
            let ticker = match self.settings.target {
                SettingsTarget::Candlestick => context.data.base_ticker(),
                SettingsTarget::Series(ticker) => ticker,
                _ => return InspectorEffect::None,
            };
            let Some(entry) = context.series_style_entry(ticker) else {
                return InspectorEffect::None;
            };
            return InspectorEffect::Command(match entry.change(message) {
                series_editor::Change::ColorChanged(ticker_info, color) => {
                    ChartCommand::SetSeriesColor { ticker_info, color }
                }
                series_editor::Change::LineWidthChanged(ticker_info, width) => {
                    ChartCommand::SetSeriesLineWidth { ticker_info, width }
                }
                series_editor::Change::StrokeStyleChanged(ticker_info, stroke_style) => {
                    ChartCommand::SetSeriesStrokeStyle {
                        ticker_info,
                        stroke_style,
                    }
                }
            });
        }

        self.settings.update(context, message)
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    ResetRequested(ResetScope),
    ResetConfirmed,
    ResetCancelled,
    ClusterKindSelected(ClusterKind),
    ClusterScalingSelected(ClusterScaling),
    FootprintImbalanceConfigChanged(FootprintImbalanceConfig),
    FootprintPocConfigChanged(FootprintPocConfig),
    TradeDataEnabledChanged(bool),
    LegendVisibilityChanged(LegendVisibility),
    ResetFootprintSettings,
    ResetSeriesSettings(TickerInfo),
    StudyTabSelected(inspector::InspectorTab),
    Study(study::Message<KlineStudy>),
    Indicator {
        instance_id: data::chart::kline::IndicatorInstanceId,
        message: IndicatorIdSettingsMessage,
    },
    CandlestickSettings(CandlestickSettingsMessage),
    SeriesEditor(series_editor::Message),
}

fn settings_row<'a, M>(label: &'a str, control: impl Into<Element<'a, M>>) -> Element<'a, M>
where
    M: 'a,
{
    inspector::setting_section(label, control)
        .with_modal_container()
        .into_element()
}

#[derive(Debug, Clone)]
pub enum IndicatorIdSettingsMessage {
    TabSelected(inspector::InspectorTab),
    OpenExternalUrl(String),
    InputChanged {
        id: String,
        value: String,
        config: Option<IndicatorConfig>,
    },
    DataConfig(IndicatorConfig),
    AddOnSource(IndicatorAttachmentChoice),
    SourceSelected {
        input_id: String,
        selection: IndicatorSourceSelection,
    },
    Mark(MarkKind),
    Color {
        slot: String,
        accent: Option<ThemeAccent>,
        shade: ThemeColorShade,
    },
    FillVisible(bool),
    LineWidth {
        slot: String,
        width: u8,
    },
    StrokeStyle {
        slot: String,
        stroke_style: LineStrokeStyle,
    },
    ResetInputs,
    ResetStyle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndicatorAttachmentChoice {
    pub(super) label: String,
    pub(super) kind: IndicatorId,
    pub(super) input_id: String,
    pub(super) source: IndicatorOutputRef,
}

impl Display for IndicatorAttachmentChoice {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.label)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct IndicatorSourceChoice {
    pub(super) label: String,
    pub(super) selection: IndicatorSourceSelection,
}

impl Display for IndicatorSourceChoice {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.label)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndicatorSourceSelection {
    Declared,
    Base(BaseSeriesSource),
    Indicator(IndicatorOutputRef),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct IndicatorSourceSetting {
    pub(super) input_id: String,
    pub(super) label: String,
    pub(super) selected: Option<IndicatorSourceChoice>,
    pub(super) choices: Vec<IndicatorSourceChoice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct IndicatorCompositionSettings {
    pub(super) source_settings: Vec<IndicatorSourceSetting>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndicatorSettingsState {
    instance_id: data::chart::kline::IndicatorInstanceId,
    indicator: data::chart::kline::IndicatorId,
    tab: inspector::InspectorTab,
    input_drafts: Vec<(String, String)>,
}

impl IndicatorSettingsState {
    pub fn new(
        instance_id: data::chart::kline::IndicatorInstanceId,
        indicator: data::chart::kline::IndicatorId,
    ) -> Self {
        Self {
            instance_id,
            indicator,
            tab: inspector::InspectorTab::Inputs,
            input_drafts: Vec::new(),
        }
    }

    pub fn instance_id(&self) -> data::chart::kline::IndicatorInstanceId {
        self.instance_id
    }

    pub fn indicator(&self) -> data::chart::kline::IndicatorId {
        self.indicator.clone()
    }

    pub fn tab(&self) -> inspector::InspectorTab {
        self.tab
    }

    pub fn select_tab(&mut self, tab: inspector::InspectorTab) {
        self.tab = tab;
    }

    pub fn input_draft(&self, setting_id: &str) -> Option<&str> {
        self.input_drafts
            .iter()
            .find_map(|(id, value)| (id == setting_id).then_some(value.as_str()))
    }

    pub fn set_input_draft(&mut self, setting_id: impl Into<String>, value: String) {
        let setting_id = setting_id.into();
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StudySettingsState {
    study_id: KlineStudyId,
    tab: inspector::InspectorTab,
}

impl StudySettingsState {
    fn new(study_id: KlineStudyId) -> Self {
        Self {
            study_id,
            tab: inspector::InspectorTab::Inputs,
        }
    }

    fn study_id(&self) -> KlineStudyId {
        self.study_id
    }

    fn tab(&self) -> inspector::InspectorTab {
        self.tab
    }

    fn select_tab(&mut self, tab: inspector::InspectorTab) {
        self.tab = tab;
    }
}

pub fn kline_indicator_cfg_view<'a>(
    modal: &'a IndicatorSettingsState,
    data_config: Option<IndicatorConfig>,
    composition: IndicatorCompositionSettings,
    basis: data::chart::Basis,
    input_coverage: data::chart::kline::IndicatorInputCoverage,
    style_config: IndicatorStyleOverrides,
    mark: Option<MarkKind>,
    active_style_slots: Vec<ResolvedIndicatorStyleSlot>,
    unavailable_marks: Vec<MarkKind>,
    theme_accents: ThemeAccents,
) -> (inspector::InspectorTab, Element<'a, Message>) {
    let indicator = modal.indicator();
    let instance_id = modal.instance_id();
    let definition = indicator.definition();
    let author = definition.author.clone();
    let description = definition.description.clone();
    let source = definition.source.clone();

    let mark_section =
        indicator_mark_section(instance_id, indicator.clone(), mark, &unavailable_marks);
    let channel_style_section = indicator_channel_style_section(
        instance_id,
        indicator.clone(),
        style_config,
        active_style_slots,
        theme_accents,
    );

    let input_content = data_config.and_then(|config| {
        let settings = config
            .clone()
            .settings_for(
                &indicator,
                IndicatorSettingsContext {
                    basis,
                    input_coverage: input_coverage.clone(),
                },
            )
            .unwrap_or_default();
        indicator_settings_content(settings, &composition, config, &indicator, modal)
    });
    let mut style_sections = Vec::new();
    if let Some(mark_section) = mark_section {
        style_sections.push(mark_section.into_element());
    }
    style_sections.extend(channel_style_section);
    let style_content = if style_sections.is_empty() {
        None
    } else {
        Some(
            column(style_sections)
                .spacing(inspector::MODAL_SECTION_SPACING)
                .into(),
        )
    };

    inspector::tabbed_content(
        modal.tab(),
        move |tab| Message::Indicator {
            instance_id,
            message: IndicatorIdSettingsMessage::TabSelected(tab),
        },
        [
            (inspector::InspectorTab::Inputs, input_content),
            (inspector::InspectorTab::Style, style_content),
            (
                inspector::InspectorTab::Source,
                indicator_source_section(instance_id, author, description, source),
            ),
        ],
    )
}

fn indicator_source_section<'a>(
    instance_id: data::chart::kline::IndicatorInstanceId,
    author: Option<IndicatorAuthor>,
    description: Option<String>,
    source: Option<String>,
) -> Option<Element<'a, Message>> {
    if author.is_none() && description.is_none() && source.is_none() {
        return None;
    }

    let author = author
        .map(|author| indicator_external_link(instance_id, author.name, author.url))
        .unwrap_or_else(|| {
            text("Not provided")
                .style(style::text_color::secondary)
                .into()
        });

    let description = text(description.unwrap_or_else(|| "Not provided".to_owned()))
        .style(style::text_color::secondary);

    let source = source
        .map(|source| indicator_external_link(instance_id, source.clone(), source))
        .unwrap_or_else(|| {
            text("Closed-source or not provided")
                .style(style::text_color::secondary)
                .into()
        });

    Some(
        column![
            inspector::setting_section("Author", author)
                .with_modal_container()
                .into_element(),
            inspector::setting_section("Description", description)
                .with_modal_container()
                .into_element(),
            inspector::setting_section("Source", source)
                .with_modal_container()
                .into_element(),
        ]
        .spacing(inspector::MODAL_SECTION_SPACING)
        .into(),
    )
}

fn indicator_external_link<'a>(
    instance_id: data::chart::kline::IndicatorInstanceId,
    label: String,
    url: String,
) -> Element<'a, Message> {
    button(
        row![
            text(label).style(style::text_color::secondary),
            style::icon_text(style::Icon::ExternalLink, 12).style(style::text_color::secondary),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
    )
    .padding(0)
    .style(style::button::text_link)
    .on_press(Message::Indicator {
        instance_id,
        message: IndicatorIdSettingsMessage::OpenExternalUrl(url),
    })
    .into()
}

fn indicator_settings_content<'a>(
    settings: Vec<IndicatorSetting>,
    composition: &IndicatorCompositionSettings,
    config: IndicatorConfig,
    indicator: &IndicatorId,
    modal: &'a IndicatorSettingsState,
) -> Option<Element<'a, Message>> {
    let mut inputs = Vec::new();
    for source_setting in &composition.source_settings {
        let section = if source_setting.choices.is_empty() {
            inspector::setting_section(source_setting.label.clone(), text("Unavailable"))
        } else {
            let instance_id = modal.instance_id();
            let input_id = source_setting.input_id.clone();
            let picker = pick_list(
                source_setting.choices.clone(),
                source_setting.selected.clone(),
                move |choice| Message::Indicator {
                    instance_id,
                    message: IndicatorIdSettingsMessage::SourceSelected {
                        input_id: input_id.clone(),
                        selection: choice.selection.clone(),
                    },
                },
            )
            .width(Length::Fill);
            inspector::setting_section(source_setting.label.clone(), picker)
        };
        inputs.push(inspector::modal_container(section.into_element()));
    }
    let mut grouped_settings: Option<(String, Vec<Element<'a, Message>>)> = None;

    for setting in settings {
        let group = setting.group.clone();
        let section = indicator_setting_view(config.clone(), indicator.clone(), setting, modal)
            .into_element();

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

    if inputs.is_empty() {
        None
    } else {
        Some(
            column(inputs)
                .spacing(inspector::MODAL_SECTION_SPACING)
                .into(),
        )
    }
}

fn indicator_setting_view<'a>(
    config: IndicatorConfig,
    indicator: IndicatorId,
    setting: IndicatorSetting,
    modal: &'a IndicatorSettingsState,
) -> InspectorSection<'a, Message> {
    let IndicatorSetting {
        id,
        label,
        tooltip,
        value,
        control,
        ..
    } = setting;
    let id = id.to_owned();
    let instance_id = modal.instance_id();
    match (control, value) {
        (IndicatorSettingControl::SliderI64 { min, max }, IndicatorSettingValue::I64(value)) => {
            let placeholder = input_limits_placeholder(min, max);
            let min = min.unwrap_or(i64::MIN);
            let max = max.unwrap_or(i64::MAX);
            let input_value = modal
                .input_draft(&id)
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string());
            let is_valid = input_value
                .parse::<i64>()
                .is_ok_and(|value| (min..=max).contains(&value));
            let setting_indicator = indicator.clone();
            let input = text_input(&placeholder, &input_value)
                .on_input(move |value| {
                    indicator_input_message(
                        instance_id,
                        setting_indicator.clone(),
                        config.clone(),
                        id.clone(),
                        value,
                        move |value| {
                            value
                                .parse::<i64>()
                                .ok()
                                .filter(|value| (min..=max).contains(value))
                                .map(IndicatorSettingValue::I64)
                        },
                    )
                })
                .style(move |theme, status| style::validated_text_input(theme, status, is_valid))
                .width(Length::Fill);

            indicator_setting_section(label, tooltip, input)
        }
        (
            IndicatorSettingControl::SliderF64 { min, max, step },
            IndicatorSettingValue::F64(value),
        ) => {
            let placeholder = input_limits_placeholder(min, max);
            let min = min.unwrap_or(value - 1.0);
            let max = max.unwrap_or(value + 1.0);
            let input_value = modal
                .input_draft(&id)
                .map(str::to_owned)
                .unwrap_or_else(|| format_indicator_value(value, step));
            let is_valid = input_value
                .parse::<f64>()
                .is_ok_and(|value| value.is_finite() && (min..=max).contains(&value));
            let setting_indicator = indicator.clone();
            let input = text_input(&placeholder, &input_value)
                .on_input(move |value| {
                    indicator_input_message(
                        instance_id,
                        setting_indicator.clone(),
                        config.clone(),
                        id.clone(),
                        value,
                        move |value| {
                            value
                                .parse::<f64>()
                                .ok()
                                .filter(|value| value.is_finite() && (min..=max).contains(value))
                                .map(IndicatorSettingValue::F64)
                        },
                    )
                })
                .style(move |theme, status| style::validated_text_input(theme, status, is_valid))
                .width(Length::Fill);

            indicator_setting_section(label, tooltip, input)
        }
        (IndicatorSettingControl::Toggle, IndicatorSettingValue::Bool(value)) => {
            let setting_indicator = indicator.clone();
            let toggle = checkbox(value).on_toggle(move |value| {
                indicator_setting_message(
                    instance_id,
                    setting_indicator.clone(),
                    config.clone(),
                    id.clone(),
                    IndicatorSettingValue::Bool(value),
                )
            });

            indicator_setting_section(label, tooltip, toggle)
        }
        (IndicatorSettingControl::Color, IndicatorSettingValue::Color(value)) => {
            let input_value = modal
                .input_draft(&id)
                .map(str::to_owned)
                .unwrap_or_else(|| format_indicator_color(value));
            let is_valid = parse_indicator_color(&input_value).is_some();
            let setting_indicator = indicator.clone();
            let input = text_input("", &input_value)
                .on_input(move |value| {
                    indicator_input_message(
                        instance_id,
                        setting_indicator.clone(),
                        config.clone(),
                        id.clone(),
                        value,
                        |value| parse_indicator_color(value).map(IndicatorSettingValue::Color),
                    )
                })
                .style(move |theme, status| style::validated_text_input(theme, status, is_valid))
                .width(Length::Fill);

            indicator_setting_section(label, tooltip, input)
        }
        (IndicatorSettingControl::Text, IndicatorSettingValue::Text(value)) => {
            let input_value = modal.input_draft(&id).map(str::to_owned).unwrap_or(value);
            let setting_indicator = indicator.clone();
            let input = text_input("", &input_value)
                .on_input(move |value| {
                    indicator_input_message(
                        instance_id,
                        setting_indicator.clone(),
                        config.clone(),
                        id.clone(),
                        value,
                        |value| Some(IndicatorSettingValue::Text(value.to_owned())),
                    )
                })
                .width(Length::Fill);

            indicator_setting_section(label, tooltip, input)
        }
        (IndicatorSettingControl::Choices(choices), value @ IndicatorSettingValue::Choice(_)) => {
            let selected = value.as_text().unwrap_or_default().to_owned();
            if choices.len() > 3 {
                let selected_choice = choices.iter().find(|choice| choice.id == selected).cloned();
                let picker = pick_list(choices, selected_choice, move |choice| {
                    let value = setting_choice_value(&choice);
                    indicator_setting_message(
                        instance_id,
                        indicator.clone(),
                        config.clone(),
                        id.clone(),
                        value,
                    )
                })
                .width(Length::Fill);

                return indicator_setting_section(label, tooltip, picker);
            }

            let mut controls = column![].spacing(4);
            let selected_choice = choices.iter().position(|choice| choice.id == selected);
            for (choice_index, choice) in choices.into_iter().enumerate() {
                let setting_config = config.clone();
                let setting_indicator = indicator.clone();
                let value = setting_choice_value(&choice);
                let label_text = choice.label.clone();
                let setting_id = id.clone();
                controls = controls.push(radio(
                    label_text,
                    choice_index,
                    selected_choice,
                    move |_| {
                        indicator_setting_message(
                            instance_id,
                            setting_indicator.clone(),
                            setting_config.clone(),
                            setting_id.clone(),
                            value.clone(),
                        )
                    },
                ));
            }

            indicator_setting_section(label, tooltip, controls)
        }
        _ => indicator_setting_section(label, tooltip, text("")),
    }
}

fn indicator_tooltip<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    tooltip: String,
    position: iced::widget::tooltip::Position,
) -> Element<'a, Message> {
    iced::widget::tooltip(
        content,
        iced::widget::container(iced::widget::text(tooltip))
            .style(style::tooltip)
            .padding(8),
        position,
    )
    .into()
}

fn indicator_setting_section<'a>(
    label: String,
    tooltip_text: Option<String>,
    control: impl Into<Element<'a, Message>>,
) -> InspectorSection<'a, Message> {
    let title: Element<'a, Message> = match tooltip_text {
        Some(tooltip_text) => row![
            crate::widget::text::body(label),
            indicator_tooltip(
                button(text("i").size(crate::style::text_size::SMALL).center())
                    .style(style::button::tooltip_indicator)
                    .padding(0)
                    .width(16)
                    .height(16),
                tooltip_text,
                TooltipPosition::Right,
            ),
        ]
        .spacing(4)
        .align_y(Alignment::Center)
        .into(),
        None => crate::widget::text::body(label).into(),
    };

    InspectorSection::new(title, iced::widget::container(control).width(Length::Fill))
}

fn format_indicator_value(value: f64, step: f64) -> String {
    let precision = step
        .to_string()
        .split_once('.')
        .map_or(0, |(_, decimals)| decimals.len());
    format!("{value:.precision$}")
}

fn input_limits_placeholder<T: Display>(min: Option<T>, max: Option<T>) -> String {
    match (min, max) {
        (Some(min), Some(max)) => format!("{min}-{max}"),
        (Some(min), None) => format!("{min}-"),
        (None, Some(max)) => format!("-{max}"),
        (None, None) => String::new(),
    }
}

fn format_indicator_color(value: u32) -> String {
    format!("#{value:08X}")
}

fn parse_indicator_color(value: &str) -> Option<u32> {
    let value = value.strip_prefix('#').unwrap_or(value);
    (value.len() == 8)
        .then(|| u32::from_str_radix(value, 16).ok())
        .flatten()
}

fn setting_choice_value(
    choice: &data::chart::kline::IndicatorSettingChoice,
) -> IndicatorSettingValue {
    IndicatorSettingValue::Choice(choice.id.clone())
}

fn indicator_input_message(
    instance_id: data::chart::kline::IndicatorInstanceId,
    indicator: IndicatorId,
    config: IndicatorConfig,
    id: String,
    value: String,
    parse_value: impl Fn(&str) -> Option<IndicatorSettingValue>,
) -> Message {
    let config =
        parse_value(&value).and_then(|value| config.with_setting_for(&indicator, &id, value));
    Message::Indicator {
        instance_id,
        message: IndicatorIdSettingsMessage::InputChanged { id, value, config },
    }
}

fn indicator_setting_message(
    instance_id: data::chart::kline::IndicatorInstanceId,
    indicator: IndicatorId,
    config: IndicatorConfig,
    id: String,
    value: IndicatorSettingValue,
) -> Message {
    let config = config
        .with_setting_for(&indicator, &id, value)
        .expect("indicator setting schema must match its configuration");
    Message::Indicator {
        instance_id,
        message: IndicatorIdSettingsMessage::DataConfig(config),
    }
}

fn indicator_channel_style_section<'a>(
    instance_id: data::chart::kline::IndicatorInstanceId,
    indicator: IndicatorId,
    style_config: IndicatorStyleOverrides,
    active_style_slots: Vec<ResolvedIndicatorStyleSlot>,
    theme_accents: ThemeAccents,
) -> Option<Element<'a, Message>> {
    let fill_style_slots = indicator
        .fill_specs()
        .iter()
        .flat_map(|fill| {
            std::iter::once(fill.color_slot.as_str())
                .chain(fill.true_color_slot.as_deref())
                .chain(fill.false_color_slot.as_deref())
        })
        .collect::<Vec<_>>();
    let mut sections = Vec::new();
    let mut fill_sections = Vec::new();
    let mut fill_section_index = None;

    for spec in indicator
        .color_specs()
        .iter()
        .filter(|spec| active_style_slots.iter().any(|slot| slot.id == spec.id))
    {
        let controls = indicator_style_controls(
            instance_id,
            spec,
            &style_config,
            &active_style_slots,
            theme_accents,
        );
        if fill_style_slots.iter().any(|slot| *slot == spec.id) {
            fill_section_index.get_or_insert(sections.len());
            fill_sections.push((spec.label.clone(), controls));
        } else {
            sections.push(settings_row(&spec.label, controls));
        }
    }

    if !fill_style_slots.is_empty() {
        let fill_title = checkbox(style_config.fill_visible)
            .label("Background fill")
            .spacing(8)
            .text_size(crate::style::text_size::SECTION)
            .on_toggle(move |visible| Message::Indicator {
                instance_id,
                message: IndicatorIdSettingsMessage::FillVisible(visible),
            })
            .width(Length::Fill);
        let fill_section = if style_config.fill_visible && !fill_sections.is_empty() {
            let setting = if fill_sections.len() == 1 {
                fill_sections
                    .into_iter()
                    .next()
                    .expect("fill section must contain its style controls")
                    .1
            } else {
                column(fill_sections.into_iter().map(|(label, controls)| {
                    InspectorSection::new(crate::widget::text::body(label), controls).into_element()
                }))
                .spacing(inspector::MODAL_SECTION_SPACING)
                .into()
            };
            InspectorSection::new(fill_title, setting)
                .with_modal_container()
                .into_element()
        } else {
            inspector::modal_container(fill_title)
        };
        sections.insert(fill_section_index.unwrap_or(sections.len()), fill_section);
    }

    if sections.is_empty() {
        None
    } else {
        Some(
            column(sections)
                .spacing(inspector::MODAL_SECTION_SPACING)
                .into(),
        )
    }
}

fn indicator_style_controls<'a>(
    instance_id: data::chart::kline::IndicatorInstanceId,
    spec: &data::chart::kline::IndicatorColorSpec,
    style_config: &IndicatorStyleOverrides,
    active_style_slots: &[ResolvedIndicatorStyleSlot],
    theme_accents: ThemeAccents,
) -> Element<'a, Message> {
    let selected = style_config
        .color(&spec.id)
        .unwrap_or_else(|| spec.default_color());
    let color_slot = spec.id.to_owned();
    let shade_slot = color_slot.clone();
    let send_color = move |accent| Message::Indicator {
        instance_id,
        message: IndicatorIdSettingsMessage::Color {
            slot: color_slot.clone(),
            accent,
            shade: selected.shade,
        },
    };
    let color_choices = theme_color_control(selected, theme_accents, send_color);
    let shade_picklist = pick_list(ThemeColorShade::ALL, Some(selected.shade), move |shade| {
        Message::Indicator {
            instance_id,
            message: IndicatorIdSettingsMessage::Color {
                slot: shade_slot.clone(),
                accent: selected.accent,
                shade,
            },
        }
    })
    .width(Length::Fill);

    let mut controls = column![color_choices].width(Length::Fill).spacing(4);

    let has_line_style = active_style_slots
        .iter()
        .find(|slot| slot.id == spec.id)
        .is_some_and(|slot| slot.supports_line_styling);
    if has_line_style {
        let slot = spec.id.to_owned();
        let width_slot = slot.clone();
        let stroke_slot = slot.clone();
        let width_slider = classic_slider_row(
            Some(text("Width")),
            MIN_LINE_WIDTH..=MAX_LINE_WIDTH,
            style_config
                .line_width(&width_slot)
                .unwrap_or(MIN_LINE_WIDTH),
            move |width| Message::Indicator {
                instance_id,
                message: IndicatorIdSettingsMessage::LineWidth {
                    slot: width_slot.clone(),
                    width,
                },
            },
            |value| value.to_string(),
            Some(1),
        );
        let stroke_picker = pick_list(
            LineStrokeStyle::ALL,
            Some(style_config.stroke_style(&stroke_slot).unwrap_or_default()),
            move |stroke_style| Message::Indicator {
                instance_id,
                message: IndicatorIdSettingsMessage::StrokeStyle {
                    slot: stroke_slot.clone(),
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

    controls.into()
}

fn indicator_mark_section<'a>(
    instance_id: data::chart::kline::IndicatorInstanceId,
    indicator: IndicatorId,
    mark: Option<MarkKind>,
    unavailable_marks: &[MarkKind],
) -> Option<InspectorSection<'a, Message>> {
    let mut selectable_mark_specs: Vec<&data::chart::kline::IndicatorMarkSpec> = Vec::new();
    for spec in indicator.mark_specs() {
        if unavailable_marks.contains(&spec.mark)
            || selectable_mark_specs
                .iter()
                .any(|candidate| candidate.mark == spec.mark)
        {
            continue;
        }
        selectable_mark_specs.push(spec);
    }
    if selectable_mark_specs.len() < 2 {
        return None;
    }

    let selected = mark
        .filter(|mark| selectable_mark_specs.iter().any(|spec| spec.mark == *mark))
        .or_else(|| selectable_mark_specs.first().map(|spec| spec.mark));
    let send_mark = move |value| Message::Indicator {
        instance_id,
        message: IndicatorIdSettingsMessage::Mark(value),
    };
    let controls = selectable_mark_specs
        .into_iter()
        .map(|spec| radio(spec.label.clone(), spec.mark, selected, send_mark).into());

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
            classic_slider_row(
                Some(text("Visible-range weight")),
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
            TooltipPosition::Right,
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
        TooltipPosition::Right,
    );

    if !config.show {
        return inspector::modal_container(toggle);
    }

    let threshold = classic_slider_row(
        Some(text("Ask:Bid threshold")),
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

fn footprint_cfg_view(
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
    modal: StudySettingsState,
    study: KlineStudy,
    basis: data::chart::Basis,
    theme_accents: ThemeAccents,
) -> (inspector::InspectorTab, Element<'static, Message>) {
    let content_for_tab = |tab| {
        study
            .view_config_without_reset(basis, theme_accents, tab, study::Message::StudyValueChanged)
            .map(|content| content.map(Message::Study))
    };

    inspector::tabbed_content(
        modal.tab(),
        Message::StudyTabSelected,
        [
            (
                inspector::InspectorTab::Inputs,
                content_for_tab(inspector::InspectorTab::Inputs),
            ),
            (
                inspector::InspectorTab::Style,
                content_for_tab(inspector::InspectorTab::Style),
            ),
            (
                inspector::InspectorTab::Source,
                content_for_tab(inspector::InspectorTab::Source),
            ),
        ],
    )
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

fn candlestick_cfg_view<'a>(
    ticker: TickerInfo,
    config: CandlestickConfig,
    mark: MarkKind,
    base_series: Option<SeriesStyleEntry>,
    theme_accents: ThemeAccents,
) -> Element<'a, Message> {
    let opacity = config.opacity_percent.clamp(
        MIN_CANDLESTICK_OPACITY_PERCENT,
        MAX_CANDLESTICK_OPACITY_PERCENT,
    );
    let opacity_section = classic_slider_row(
        Some(text("Opacity")),
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
                    series.view(theme_accents).map(Message::SeriesEditor),
                )
                .with_modal_container()
                .into_element(),
            );
        }
        content
    };

    content.into()
}

fn series_cfg_view<'a>(
    ticker: TickerInfo,
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
                series.view(theme_accents).map(Message::SeriesEditor),
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
    Study(StudySettingsState),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetScope {
    Inputs,
    Style,
    All,
}

impl ResetScope {
    fn label(self) -> &'static str {
        match self {
            Self::Inputs => "Reset inputs",
            Self::Style => "Reset style",
            Self::All => "Reset settings",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct State {
    target: SettingsTarget,
    reset_confirmation: Option<ResetScope>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            target: SettingsTarget::Main,
            reset_confirmation: None,
        }
    }
}

impl State {
    fn reset_message(&self, context: &SettingsContext<'_>, scope: ResetScope) -> Option<Message> {
        match &self.target {
            SettingsTarget::Main | SettingsTarget::Footprint => {
                Some(Message::ResetFootprintSettings)
            }
            SettingsTarget::Indicator(modal) => Some(Message::Indicator {
                instance_id: modal.instance_id(),
                message: match scope {
                    ResetScope::Inputs => IndicatorIdSettingsMessage::ResetInputs,
                    ResetScope::Style => IndicatorIdSettingsMessage::ResetStyle,
                    ResetScope::All => return None,
                },
            }),
            SettingsTarget::Candlestick => Some(Message::CandlestickSettings(
                CandlestickSettingsMessage::Reset,
            )),
            SettingsTarget::Series(ticker) => Some(Message::ResetSeriesSettings(*ticker)),
            SettingsTarget::Study(modal) => {
                let study = context.studies.study(modal.study_id())?;
                let defaults = match scope {
                    ResetScope::Inputs => study.with_default_inputs(context.basis),
                    ResetScope::Style => study.with_default_style(),
                    ResetScope::All => return None,
                };
                Some(Message::Study(study::Message::StudyValueChanged(defaults)))
            }
        }
    }

    fn cancel_reset_confirmation(&mut self) {
        self.reset_confirmation = None;
    }

    fn confirmed_reset_message(&mut self, context: &SettingsContext<'_>) -> Option<Message> {
        let scope = self.reset_confirmation.take()?;
        self.reset_message(context, scope)
    }

    fn set_target(&mut self, target: SettingsTarget) {
        self.target = target;
        self.cancel_reset_confirmation();
    }

    fn selected_tab(&self) -> inspector::InspectorTab {
        match &self.target {
            SettingsTarget::Indicator(modal) => modal.tab(),
            SettingsTarget::Study(modal) => modal.tab(),
            SettingsTarget::Main
            | SettingsTarget::Candlestick
            | SettingsTarget::Series(_)
            | SettingsTarget::Footprint => inspector::InspectorTab::Inputs,
        }
    }

    fn reconcile_missing_target(&mut self, context: &SettingsContext<'_>) {
        let missing = match &self.target {
            SettingsTarget::Indicator(modal) => context
                .indicators
                .indicator_kind(modal.instance_id())
                .is_none(),
            SettingsTarget::Study(modal) => context.studies.study(modal.study_id()).is_none(),
            SettingsTarget::Series(ticker) => context.series_style_entry(*ticker).is_none(),
            SettingsTarget::Main | SettingsTarget::Candlestick | SettingsTarget::Footprint => false,
        };
        if missing {
            self.target = SettingsTarget::Main;
            self.cancel_reset_confirmation();
        }
    }

    pub(super) fn contextual_title(&self, context: SettingsContext<'_>) -> Option<String> {
        Some(match &self.target {
            SettingsTarget::Main => return None,
            SettingsTarget::Indicator(modal) => modal.indicator().to_string(),
            SettingsTarget::Candlestick => "Base ticker".to_owned(),
            SettingsTarget::Series(_) => "Comparison ticker".to_owned(),
            SettingsTarget::Footprint => "Footprint".to_owned(),
            SettingsTarget::Study(modal) => context
                .studies
                .study(modal.study_id())
                .map_or_else(|| "Study".to_owned(), |study| study.to_string()),
        })
    }

    pub(super) fn contextual_content<'a>(
        &'a self,
        context: SettingsContext<'a>,
        theme_accents: ThemeAccents,
    ) -> Option<(Option<inspector::InspectorTab>, Element<'a, ChartMessage>)> {
        let presentation = context.presentation();

        let content = match &self.target {
            SettingsTarget::Main => return None,
            SettingsTarget::Indicator(modal) => {
                let (tab, content) = kline_indicator_cfg_view(
                    modal,
                    context
                        .indicators
                        .indicator_data_config(modal.instance_id()),
                    context.indicator_composition_settings(modal.instance_id()),
                    context.basis,
                    presentation.input_coverage.clone(),
                    context
                        .indicators
                        .indicator_style_config(modal.instance_id())
                        .unwrap_or_default(),
                    presentation
                        .presentation(modal.instance_id())
                        .and_then(|presentation| presentation.effective_mark),
                    presentation.style_slots_for(modal.instance_id(), &modal.indicator()),
                    modal
                        .indicator()
                        .mark_specs()
                        .iter()
                        .filter(|spec| {
                            !context
                                .indicators
                                .indicator_mark_selectable(modal.instance_id(), spec.mark)
                        })
                        .map(|spec| spec.mark)
                        .collect(),
                    theme_accents,
                );
                return Some((Some(tab), content.map(ChartMessage::Settings)));
            }
            SettingsTarget::Candlestick => candlestick_cfg_view(
                context.data.base_ticker(),
                context.options.candlestick,
                context
                    .indicators
                    .primary_mark()
                    .unwrap_or(MarkKind::Candle),
                context.series_style_entry(context.data.base_ticker()),
                theme_accents,
            ),
            SettingsTarget::Series(ticker) => {
                series_cfg_view(*ticker, context.series_style_entry(*ticker), theme_accents)
            }
            SettingsTarget::Footprint => footprint_cfg_view(
                context.options.footprint,
                context.trade_data_enabled,
                theme_accents,
            ),
            SettingsTarget::Study(modal) => {
                let study = context.studies.study(modal.study_id())?;
                let (tab, content) = study_cfg_view(*modal, study, context.basis, theme_accents);
                return Some((Some(tab), content.map(ChartMessage::Settings)));
            }
        };

        Some((None, content.map(ChartMessage::Settings)))
    }

    pub(super) fn footer_view<'a>(
        &'a self,
        context: SettingsContext<'a>,
        tab: Option<inspector::InspectorTab>,
    ) -> Option<(f32, Element<'a, ChartMessage>)> {
        if matches!(self.target, SettingsTarget::Main) {
            return None;
        }

        let scope = match tab {
            Some(inspector::InspectorTab::Inputs) => ResetScope::Inputs,
            Some(inspector::InspectorTab::Style) => ResetScope::Style,
            Some(inspector::InspectorTab::Source) => return None,
            None => ResetScope::All,
        };
        self.reset_message(&context, scope)?;
        let confirming_reset = self.reset_confirmation == Some(scope);

        let reset: Element<'_, ChartMessage> = if confirming_reset {
            row![
                text(format!("{}?", scope.label()))
                    .size(style::text_size::EMPHASIS)
                    .align_y(Alignment::Center),
                row![
                    button(style::icon_text(style::Icon::Checkmark, 12))
                        .style(|theme, status| style::button::confirm(theme, status, true))
                        .on_press(ChartMessage::Settings(Message::ResetConfirmed)),
                    button(style::icon_text(style::Icon::Close, 12))
                        .style(|theme, status| style::button::cancel(theme, status, true))
                        .on_press(ChartMessage::Settings(Message::ResetCancelled)),
                ]
                .spacing(8)
            ]
            .align_y(Alignment::Center)
            .spacing(24)
            .padding([0, 12])
            .into()
        } else {
            button(text("Reset"))
                .on_press(ChartMessage::Settings(Message::ResetRequested(scope)))
                .into()
        };

        if confirming_reset {
            return Some((
                inspector::InspectorFooter::HEIGHT,
                iced::widget::container(reset)
                    .align_x(Alignment::End)
                    .width(Length::Fill)
                    .padding(0)
                    .style(crate::style::inspector_section)
                    .into(),
            ));
        }
        if tab == Some(inspector::InspectorTab::Inputs)
            && let SettingsTarget::Indicator(modal) = &self.target
        {
            let choices = context.indicator_attachment_choices(modal.instance_id());
            if !choices.is_empty() {
                let instance_id = modal.instance_id();
                let attach = pick_list(choices, None::<IndicatorAttachmentChoice>, move |choice| {
                    ChartMessage::Settings(Message::Indicator {
                        instance_id,
                        message: IndicatorIdSettingsMessage::AddOnSource(choice),
                    })
                })
                .placeholder("Attach an indicator")
                .width(Length::Fill);
                return Some((
                    inspector::InspectorFooter::HEIGHT,
                    row![attach, reset]
                        .align_y(Alignment::Center)
                        .spacing(8)
                        .into(),
                ));
            }
        }

        Some((
            inspector::InspectorFooter::HEIGHT,
            row![space::horizontal(), reset]
                .align_y(Alignment::Center)
                .into(),
        ))
    }

    fn update(&mut self, context: &SettingsContext<'_>, message: Message) -> InspectorEffect {
        if !matches!(
            &message,
            Message::ResetRequested(_) | Message::ResetConfirmed | Message::ResetCancelled
        ) {
            self.cancel_reset_confirmation();
        }

        match message {
            Message::ResetRequested(scope) => {
                self.reset_confirmation = self.reset_message(context, scope).map(|_| scope);
            }
            Message::ResetConfirmed => {
                let Some(message) = self.confirmed_reset_message(context) else {
                    return InspectorEffect::None;
                };
                return self.update(context, message);
            }
            Message::ResetCancelled => self.cancel_reset_confirmation(),
            Message::Indicator {
                instance_id,
                message,
            } => {
                let command = match message {
                    IndicatorIdSettingsMessage::TabSelected(tab) => {
                        if let SettingsTarget::Indicator(modal) = &mut self.target
                            && modal.instance_id() == instance_id
                        {
                            modal.select_tab(tab);
                        }
                        None
                    }
                    IndicatorIdSettingsMessage::OpenExternalUrl(url) => {
                        return InspectorEffect::OpenUrl(url);
                    }
                    IndicatorIdSettingsMessage::InputChanged { id, value, config } => {
                        if let SettingsTarget::Indicator(modal) = &mut self.target
                            && modal.instance_id() == instance_id
                        {
                            modal.set_input_draft(id, value);
                        }
                        config.map(|config| {
                            ChartCommand::indicator_input(IndicatorInputCommand::DataConfig {
                                id: instance_id,
                                config,
                            })
                        })
                    }
                    IndicatorIdSettingsMessage::DataConfig(config) => Some(
                        ChartCommand::indicator_input(IndicatorInputCommand::DataConfig {
                            id: instance_id,
                            config,
                        }),
                    ),
                    IndicatorIdSettingsMessage::AddOnSource(choice) => {
                        Some(ChartCommand::AddIndicatorOnSource {
                            kind: choice.kind,
                            input_id: choice.input_id,
                            source: choice.source,
                        })
                    }
                    IndicatorIdSettingsMessage::SourceSelected {
                        input_id,
                        selection,
                    } => match selection {
                        IndicatorSourceSelection::Declared => Some(ChartCommand::indicator_input(
                            IndicatorInputCommand::BaseSource {
                                id: instance_id,
                                input_id,
                                source: None,
                            },
                        )),
                        IndicatorSourceSelection::Base(source) => Some(
                            ChartCommand::indicator_input(IndicatorInputCommand::BaseSource {
                                id: instance_id,
                                input_id,
                                source: Some(source),
                            }),
                        ),
                        IndicatorSourceSelection::Indicator(source) => Some(
                            ChartCommand::indicator_input(IndicatorInputCommand::SourceBinding {
                                id: instance_id,
                                input_id,
                                source,
                            }),
                        ),
                    },
                    IndicatorIdSettingsMessage::Mark(mark) => context
                        .indicators
                        .indicator_mark_selectable(instance_id, mark)
                        .then_some(ChartCommand::indicator_style(IndicatorStyleCommand::Mark {
                            id: instance_id,
                            mark,
                        })),
                    IndicatorIdSettingsMessage::Color {
                        slot,
                        accent,
                        shade,
                    } => Some(ChartCommand::indicator_style(
                        IndicatorStyleCommand::Color {
                            id: instance_id,
                            slot,
                            accent,
                            shade,
                        },
                    )),
                    IndicatorIdSettingsMessage::FillVisible(visible) => Some(
                        ChartCommand::indicator_style(IndicatorStyleCommand::FillVisibility {
                            id: instance_id,
                            visible,
                        }),
                    ),
                    IndicatorIdSettingsMessage::LineWidth { slot, width } => Some(
                        ChartCommand::indicator_style(IndicatorStyleCommand::LineWidth {
                            id: instance_id,
                            slot,
                            width,
                        }),
                    ),
                    IndicatorIdSettingsMessage::StrokeStyle { slot, stroke_style } => Some(
                        ChartCommand::indicator_style(IndicatorStyleCommand::StrokeStyle {
                            id: instance_id,
                            slot,
                            stroke_style,
                        }),
                    ),
                    IndicatorIdSettingsMessage::ResetInputs => {
                        if let SettingsTarget::Indicator(modal) = &mut self.target
                            && modal.instance_id() == instance_id
                        {
                            modal.clear_input_drafts();
                        }
                        Some(ChartCommand::indicator_input(IndicatorInputCommand::Reset(
                            instance_id,
                        )))
                    }
                    IndicatorIdSettingsMessage::ResetStyle => Some(ChartCommand::indicator_style(
                        IndicatorStyleCommand::Reset(instance_id),
                    )),
                };

                if let Some(command) = command {
                    return InspectorEffect::Command(command);
                }
            }
            Message::StudyTabSelected(tab) => {
                if let SettingsTarget::Study(modal) = &mut self.target {
                    modal.select_tab(tab);
                }
            }
            Message::CandlestickSettings(message) => {
                let command = match message {
                    CandlestickSettingsMessage::Config(config) => {
                        ChartCommand::SetCandlestickConfig { config }
                    }
                    CandlestickSettingsMessage::Mark(mark) => {
                        ChartCommand::SetCandlestickMark { mark }
                    }
                    CandlestickSettingsMessage::Reset => ChartCommand::ResetCandlestickSettings,
                };
                return InspectorEffect::Command(command);
            }
            Message::ClusterKindSelected(kind) => {
                return InspectorEffect::Command(ChartCommand::SetFootprintClusterKind { kind });
            }
            Message::ClusterScalingSelected(scaling) => {
                return InspectorEffect::Command(ChartCommand::SetFootprintClusterScaling {
                    scaling,
                });
            }
            Message::FootprintImbalanceConfigChanged(config) => {
                return InspectorEffect::Command(ChartCommand::SetFootprintImbalanceConfig {
                    config,
                });
            }
            Message::FootprintPocConfigChanged(config) => {
                return InspectorEffect::Command(ChartCommand::SetFootprintPocConfig { config });
            }
            Message::TradeDataEnabledChanged(enabled) => {
                return InspectorEffect::Command(ChartCommand::SetTradeDataEnabled(enabled));
            }
            Message::LegendVisibilityChanged(visibility) => {
                return InspectorEffect::Command(ChartCommand::SetLegendVisibility(visibility));
            }
            Message::ResetFootprintSettings => {
                return InspectorEffect::Command(ChartCommand::ResetFootprintSettings);
            }
            Message::ResetSeriesSettings(ticker_info) => {
                return InspectorEffect::Command(ChartCommand::ResetSeriesSettings { ticker_info });
            }
            Message::Study(message) => return InspectorEffect::StudyConfigurator(message),
            Message::SeriesEditor(_) => return InspectorEffect::None,
        }

        InspectorEffect::None
    }
}
