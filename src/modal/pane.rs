use iced::{
    Alignment, Element, Length, padding,
    widget::{container, mouse_area, opaque},
};

pub mod indicators;
pub mod mini_tickers_list;
pub mod settings;
pub mod stream;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorSettingsTab {
    Inputs,
    Style,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndicatorSettingsModal {
    instance_id: data::chart::kline_v2::IndicatorInstanceId,
    indicator: data::chart::kline_v2::KlineIndicator,
    tab: IndicatorSettingsTab,
    input_drafts: Vec<(&'static str, String)>,
}

impl IndicatorSettingsModal {
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

#[derive(Debug, Clone, PartialEq)]
pub enum Modal {
    StreamModifier(super::stream::Modifier),
    MiniTickersList(mini_tickers_list::MiniPanel),
    Settings,
    IndicatorSettings(IndicatorSettingsModal),
    CandlestickSettings,
    SeriesSettings(exchange::TickerInfo),
    Indicators,
    LinkGroup,
    Controls,
}

pub fn stack_modal<'a, Message>(
    base: impl Into<Element<'a, Message>>,
    content: impl Into<Element<'a, Message>>,
    on_blur: Message,
    padding: padding::Padding,
    alignment: Alignment,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    iced::widget::stack![
        base.into(),
        mouse_area(
            container(opaque(content))
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(padding)
                .align_x(alignment)
        )
        .on_press(on_blur)
    ]
    .into()
}
