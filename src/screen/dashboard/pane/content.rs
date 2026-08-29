pub(super) mod heatmap;
pub(super) mod kline;
pub(super) mod ladder;
pub(super) mod timesales;

use super::state::{Action, Event, Message, settings_message};
use crate::{
    screen::sidebar::{
        kline_indicators,
        pane_settings::{heatmap_shader_cfg_view, ladder_cfg_view, timesales_cfg_view},
    },
    theme::ThemeAccents,
    widget::inspector::{InspectorContent, InspectorSection},
};
use data::{
    chart::{
        Basis,
        kline::{IndicatorUnsupportedReason, KlineIndicator, KlineStudy},
    },
    layout::pane::{ContentKind, Settings, VisualConfig},
};
use exchange::{TickerInfo, Timeframe};
use iced::{
    Alignment,
    widget::{button, pane_grid, row, space},
};
use std::time::Instant;

use heatmap::HeatmapShader;
use kline::{Action as KlineAction, ChartCommand, FootprintCommand, KlineChart};
use ladder::Ladder;
use timesales::TimeAndSales;

pub(super) enum DueTick {
    /// No immediate content work is due, the pane may run its regular invalidation.
    ///
    /// e.g. a viewport fetch deadline has not elapsed.
    NotDue,
    /// Immediate content work was processed.
    ///
    /// e.g. a due viewport fetch was planned.
    /// `None` means planning found no request and still skips regular invalidation.
    Handled(Option<Action>),
}

#[derive(Default)]
pub enum Content {
    #[default]
    Starter,
    ShaderHeatmap {
        chart: Option<Box<HeatmapShader>>,
        studies: Vec<data::chart::heatmap::HeatmapStudy>,
    },
    Kline {
        chart: Option<Box<KlineChart>>,
        layout: Option<Box<data::chart::kline::Config>>,
        kline_indicators: Box<kline_indicators::State>,
    },
    TimeAndSales(Option<Box<TimeAndSales>>),
    Ladder(Option<Box<Ladder>>),
}

impl Content {
    pub(super) fn new_kline(
        current_content: &Content,
        tickers: &[TickerInfo],
        settings: &Settings,
    ) -> Self {
        let kline_indicators = match current_content {
            Content::Kline {
                kline_indicators, ..
            } => kline_indicators.clone(),
            _ => Box::new(kline_indicators::State::default()),
        };
        let Some(_base_ticker) = tickers.first().copied() else {
            return Content::Kline {
                chart: None,
                layout: None,
                kline_indicators,
            };
        };

        let basis = settings
            .selected_basis
            .unwrap_or(Basis::Time(Timeframe::M5));

        let saved_layout = match current_content {
            Content::Kline { chart, layout, .. } => chart
                .as_ref()
                .map(|chart| chart.serializable_config())
                .or_else(|| layout.clone().map(|layout| *layout)),
            _ => None,
        };

        let mut chart = saved_layout.as_ref().map_or_else(
            || KlineChart::new_with_tickers(basis, tickers),
            |layout| KlineChart::new_with_tickers_and_config(basis, tickers, layout),
        );
        if let Some(multiplier) = settings.tick_multiply {
            let _ = chart.apply_command(ChartCommand::Footprint(
                FootprintCommand::SetTickMultiplier(multiplier),
            ));
        }

        Content::Kline {
            chart: Some(Box::new(chart)),
            layout: None,
            kline_indicators,
        }
    }

    pub(super) fn placeholder(kind: ContentKind) -> Self {
        match kind {
            ContentKind::Starter => Content::Starter,
            ContentKind::CandlestickChart => Content::Kline {
                chart: None,
                layout: None,
                kline_indicators: Box::new(kline_indicators::State::default()),
            },
            ContentKind::ShaderHeatmap => Content::ShaderHeatmap {
                chart: None,
                studies: vec![data::chart::heatmap::HeatmapStudy::VolumeProfile(
                    data::chart::heatmap::ProfileKind::default(),
                )],
            },
            ContentKind::TimeAndSales => Content::TimeAndSales(None),
            ContentKind::Ladder => Content::Ladder(None),
        }
    }

    pub fn settings_snapshot(&self) -> Option<data::chart::kline::SettingsSnapshot> {
        match self {
            Content::Kline {
                chart: Some(chart), ..
            } => Some(chart.settings_snapshot()),
            _ => None,
        }
    }

    pub fn apply_settings_snapshot(
        &mut self,
        snapshot: &data::chart::kline::SettingsSnapshot,
    ) -> Option<KlineAction> {
        match self {
            Content::Kline {
                chart: Some(chart), ..
            } => chart.apply_settings_snapshot(snapshot),
            _ => None,
        }
    }

    pub fn last_tick(&self) -> Option<Instant> {
        match self {
            Content::Kline { chart, .. } => Some(chart.as_ref()?.last_update()),
            Content::TimeAndSales(panel) => Some(panel.as_ref()?.last_update()),
            Content::Ladder(panel) => Some(panel.as_ref()?.last_update()),
            Content::Starter => None,
            Content::ShaderHeatmap { chart, .. } => Some(chart.as_ref()?.last_tick?),
        }
    }

    /// Processes immediate content work before the pane's regular invalidation.
    pub(super) fn take_due_tick(&mut self, now: Instant) -> DueTick {
        match self {
            Content::Kline {
                chart: Some(chart), ..
            } if chart.viewport_fetch_due(now) => {
                DueTick::Handled(chart.take_viewport_fetch_action(now).map(Action::Chart))
            }
            _ => DueTick::NotDue,
        }
    }

    pub fn change_visual_config(&mut self, config: VisualConfig) {
        match (self, config) {
            (Content::ShaderHeatmap { chart: Some(c), .. }, VisualConfig::Heatmap(cfg)) => {
                c.set_visual_config(cfg);
            }
            (Content::TimeAndSales(Some(panel)), VisualConfig::TimeAndSales(cfg)) => {
                panel.config = cfg;
            }
            (Content::Ladder(Some(panel)), VisualConfig::Ladder(cfg)) => {
                panel.config = cfg;
            }
            _ => {}
        }
    }

    pub fn studies(&self) -> Option<data::chart::Study> {
        match &self {
            Content::ShaderHeatmap { studies, .. } => {
                Some(data::chart::Study::Heatmap(studies.clone()))
            }
            Content::TimeAndSales(_)
            | Content::Ladder(_)
            | Content::Starter
            | Content::Kline { .. } => None,
        }
    }

    pub fn update_studies(&mut self, studies: data::chart::Study) {
        if let (
            Content::ShaderHeatmap {
                chart,
                studies: previous,
                ..
            },
            data::chart::Study::Heatmap(studies),
        ) = (self, studies)
        {
            chart
                .as_mut()
                .expect("shader heatmap chart not initialized")
                .studies = studies.clone();
            *previous = studies;
        }
    }

    pub fn kind(&self) -> ContentKind {
        match self {
            Content::Kline { .. } => ContentKind::CandlestickChart,
            Content::TimeAndSales(_) => ContentKind::TimeAndSales,
            Content::Ladder(_) => ContentKind::Ladder,
            Content::Starter => ContentKind::Starter,
            Content::ShaderHeatmap { .. } => ContentKind::ShaderHeatmap,
        }
    }

    pub fn update_theme(&mut self, theme: &iced_core::Theme) {
        if let Content::ShaderHeatmap { chart: Some(c), .. } = self {
            c.update_theme(theme);
        }
    }

    pub(super) fn initialized(&self) -> bool {
        match self {
            Content::ShaderHeatmap { chart, .. } => chart.is_some(),
            Content::Kline { chart, .. } => chart.is_some(),
            Content::TimeAndSales(panel) => panel.is_some(),
            Content::Ladder(panel) => panel.is_some(),
            Content::Starter => true,
        }
    }

    pub fn allows_kline_indicator(&self, indicator: KlineIndicator) -> bool {
        matches!(self, Content::Kline { chart: Some(_), .. })
            && self.kline_indicator_unavailable_reason(indicator).is_none()
    }

    pub fn kline_indicator_unavailable_reason(
        &self,
        indicator: KlineIndicator,
    ) -> Option<IndicatorUnsupportedReason> {
        let Content::Kline {
            chart: Some(chart), ..
        } = self
        else {
            return None;
        };

        if let Some(reason) = chart.indicator_unsupported_reason(indicator) {
            return Some(reason);
        }

        if !chart.trade_data_enabled() && chart.indicator_requires_trade_data(indicator) {
            return Some(IndicatorUnsupportedReason::MissingRequiredInput);
        }

        None
    }

    pub fn allows_kline_study(&self, study: KlineStudy) -> bool {
        matches!(
            self,
            Content::Kline {
                chart: Some(chart), ..
            } if chart.study_is_available(study)
        )
    }

    pub(super) fn pane_settings_content<'a>(
        &'a self,
        pane: pane_grid::Pane,
        ticker_info: Option<TickerInfo>,
        selected_basis: Option<Basis>,
        theme_accents: ThemeAccents,
    ) -> Option<InspectorContent<'a, Message>> {
        match self {
            Self::Kline {
                chart: Some(chart), ..
            } => {
                let mut content = InspectorContent::new(vec![InspectorSection::titled(
                    "Data",
                    chart.pane_settings_view().map(move |message| {
                        Message::PaneEvent(pane, Event::KlineInteraction(message))
                    }),
                )]);
                content.set_footer(
                    row![
                        space::horizontal(),
                        button("Sync all")
                            .on_press(Message::PaneEvent(pane, Event::SyncKlineSettings,)),
                    ]
                    .align_y(Alignment::Center),
                );
                Some(content)
            }
            Self::ShaderHeatmap {
                chart: Some(chart),
                studies,
                ..
            } => {
                let basis = selected_basis.unwrap_or(Basis::default_heatmap_time(ticker_info));

                Some(
                    heatmap_shader_cfg_view(
                        chart.visual_config(),
                        chart.study_configurator(),
                        studies,
                        basis,
                        theme_accents,
                    )
                    .map(move |message| settings_message(pane, message)),
                )
            }
            Self::TimeAndSales(Some(panel)) => Some(
                timesales_cfg_view(panel.config)
                    .map(move |message| settings_message(pane, message)),
            ),
            Self::Ladder(Some(panel)) => Some(
                ladder_cfg_view(panel.config).map(move |message| settings_message(pane, message)),
            ),
            Self::Starter
            | Self::Kline { chart: None, .. }
            | Self::TimeAndSales(None)
            | Self::Ladder(None)
            | Self::ShaderHeatmap { chart: None, .. } => None,
        }
    }

    pub(super) fn chart_settings_content<'a>(
        &'a self,
        pane: pane_grid::Pane,
        theme_accents: ThemeAccents,
    ) -> Option<InspectorContent<'a, Message>> {
        match self {
            Self::Kline {
                chart: Some(chart), ..
            } => chart
                .contextual_settings_section(theme_accents)
                .map(|section| {
                    let section = section.map(move |message| {
                        Message::PaneEvent(pane, Event::KlineInteraction(message))
                    });
                    let mut content = InspectorContent::new(vec![section]);

                    if let Some(footer) = chart.contextual_settings_footer() {
                        content.set_footer(footer.map(move |message| {
                            Message::PaneEvent(pane, Event::KlineInteraction(message))
                        }));
                    }

                    content
                }),
            _ => None,
        }
    }
}

impl std::fmt::Display for Content {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.kind())
    }
}

impl PartialEq for Content {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Content::Starter, Content::Starter)
                | (Content::ShaderHeatmap { .. }, Content::ShaderHeatmap { .. })
                | (Content::Kline { .. }, Content::Kline { .. })
                | (Content::TimeAndSales(_), Content::TimeAndSales(_))
                | (Content::Ladder(_), Content::Ladder(_))
        )
    }
}
