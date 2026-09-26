pub(super) mod heatmap;
pub(super) mod kline;
pub(super) mod ladder;
pub(super) mod timesales;

use super::{
    ChartAction,
    state::{Event, Message},
};
use crate::{
    screen::sidebar::pane_settings::{
        self, heatmap_shader_cfg_view, ladder_cfg_view, timesales_cfg_view,
    },
    theme::ThemeAccents,
    widget::inspector::{InspectorContent, InspectorSection},
};
use data::{
    PaneSettingsSnapshot, PaneSettingsTransfer,
    chart::Basis,
    layout::pane::{ContentKind, Settings, VisualConfig},
};
use exchange::{TickerInfo, Timeframe};
use iced::{
    Alignment,
    widget::{button, pane_grid, row, space},
};
use std::time::Instant;

use heatmap::HeatmapShader;
use kline::{Action as KlineAction, ChartCommand, KlineChart};
use ladder::Ladder;
use timesales::TimeAndSales;

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
        let Some(_base_ticker) = tickers.first().copied() else {
            return Content::Kline {
                chart: None,
                layout: None,
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
            let _ = chart.apply_command(ChartCommand::SetTickMultiplier { multiplier });
        }
        if let Content::Kline {
            chart: Some(previous),
            ..
        } = current_content
        {
            chart.preserve_indicator_browser_state_from(previous);
        }

        Content::Kline {
            chart: Some(Box::new(chart)),
            layout: None,
        }
    }

    pub(super) fn placeholder(kind: ContentKind) -> Self {
        match kind {
            ContentKind::Starter => Content::Starter,
            ContentKind::CandlestickChart => Content::Kline {
                chart: None,
                layout: None,
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

    pub fn settings_snapshot(
        &self,
        transfer: PaneSettingsTransfer,
    ) -> Option<PaneSettingsSnapshot> {
        match (transfer, self) {
            (PaneSettingsTransfer::Visual(config), Content::ShaderHeatmap { studies, .. }) => Some(
                PaneSettingsSnapshot::Heatmap(data::chart::heatmap::SettingsSnapshot {
                    config: config.as_ref().and_then(VisualConfig::heatmap),
                    studies: studies.clone(),
                }),
            ),
            (PaneSettingsTransfer::Visual(config), Content::TimeAndSales(_)) => {
                Some(PaneSettingsSnapshot::TimeAndSales(
                    config.as_ref().and_then(VisualConfig::time_and_sales),
                ))
            }
            (PaneSettingsTransfer::Visual(config), Content::Ladder(_)) => Some(
                PaneSettingsSnapshot::Ladder(config.as_ref().and_then(VisualConfig::ladder)),
            ),
            (
                PaneSettingsTransfer::Kline,
                Content::Kline {
                    chart: Some(chart), ..
                },
            ) => Some(PaneSettingsSnapshot::Kline(chart.settings_snapshot())),
            _ => None,
        }
    }

    pub fn can_apply_settings_snapshot(&self, snapshot: &PaneSettingsSnapshot) -> bool {
        matches!(
            (self, snapshot),
            (
                Content::ShaderHeatmap { .. },
                PaneSettingsSnapshot::Heatmap(_)
            ) | (
                Content::Kline { chart: Some(_), .. },
                PaneSettingsSnapshot::Kline(_)
            ) | (
                Content::TimeAndSales(_),
                PaneSettingsSnapshot::TimeAndSales(_)
            ) | (Content::Ladder(_), PaneSettingsSnapshot::Ladder(_))
        )
    }

    pub fn apply_settings_snapshot(
        &mut self,
        snapshot: &PaneSettingsSnapshot,
    ) -> Option<KlineAction> {
        match (self, snapshot) {
            (
                Content::ShaderHeatmap {
                    chart,
                    studies: previous,
                },
                PaneSettingsSnapshot::Heatmap(snapshot),
            ) => {
                *previous = snapshot.studies.clone();
                if let Some(chart) = chart {
                    chart.studies = snapshot.studies.clone();
                }
                None
            }
            (
                Content::Kline {
                    chart: Some(chart), ..
                },
                PaneSettingsSnapshot::Kline(snapshot),
            ) => chart.apply_settings_snapshot(snapshot),
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

    fn update_interval(&self) -> Option<u64> {
        match self {
            Content::Kline { .. } => Some(250),
            Content::Ladder(_) | Content::TimeAndSales(_) => Some(100),
            Content::ShaderHeatmap { .. } | Content::Starter => None,
        }
    }

    pub(super) fn invalidate(&mut self, now: Instant) -> Option<ChartAction> {
        match self {
            Content::Starter => None,
            Content::Kline { chart, .. } => {
                chart.as_mut().and_then(|chart| chart.invalidate(Some(now)))
            }
            Content::TimeAndSales(panel) => {
                if let Some(panel) = panel {
                    panel.invalidate(Some(now));
                }
                None
            }
            Content::Ladder(panel) => {
                if let Some(panel) = panel {
                    panel.invalidate(Some(now));
                }
                None
            }
            Content::ShaderHeatmap { chart, .. } => {
                chart.as_mut().and_then(|chart| chart.invalidate(Some(now)))
            }
        }
    }

    pub(super) fn tick(&mut self, now: Instant) -> Option<ChartAction> {
        let invalidate_interval = self.update_interval();
        let last_tick = self.last_tick();

        self.redraw_data_if_due(now);

        match (invalidate_interval, last_tick) {
            (Some(interval_ms), Some(previous_tick_time))
                if interval_ms > 0
                    && now.duration_since(previous_tick_time)
                        >= std::time::Duration::from_millis(interval_ms) =>
            {
                return self.invalidate(now);
            }
            (Some(interval_ms), None) if interval_ms > 0 => return self.invalidate(now),
            (None, _) => return self.invalidate(now),
            _ => {}
        }

        None
    }

    pub(super) fn background_task(&mut self) -> Option<iced::Task<Event>> {
        match self {
            Content::Kline {
                chart: Some(chart), ..
            } => chart
                .rebuild_indicators()
                .map(|task| task.map(Event::KlineInteraction)),
            _ => None,
        }
    }

    pub(super) fn park(&mut self) {
        if let Content::ShaderHeatmap { chart, .. } = self {
            *chart = None;
        }
    }

    pub(super) fn redraw_data_if_due(&mut self, now: Instant) {
        if let Content::Kline {
            chart: Some(chart), ..
        } = self
        {
            chart.redraw_data_if_due(now);
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
                        button("Sync all").on_press(Message::SyncKlineSettings(pane)),
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

fn settings_message(pane: pane_grid::Pane, message: pane_settings::Message) -> Message {
    match message {
        pane_settings::Message::VisualConfigChanged { config, sync_all } => {
            Message::VisualConfigChanged(pane, config, sync_all)
        }
        message => Message::PaneEvent(pane, Event::Settings(message)),
    }
}
