pub(super) mod heatmap;
pub(super) mod kline;
pub(super) mod ladder;
pub(super) mod timesales;

use super::{
    ChartAction,
    state::{Event, Message},
};
use crate::{
    screen::dashboard::pane::settings::{
        self, heatmap_shader_cfg_view, ladder_cfg_view, timesales_cfg_view,
    },
    widget::inspector::InspectorContent,
};
use data::{
    PaneSettingsSnapshot, PaneSettingsTransfer,
    chart::{Basis, kline::OpenInterestSeries},
    layout::pane::{ContentKind, Settings, VisualConfig},
};
use exchange::{Kline, TickerInfo, Timeframe, Trade, UnixMs, adapter::StreamKind, depth::Depth};
use iced::{
    Alignment,
    task::Handle,
    widget::{button, pane_grid, row, space},
};
use std::time::Instant;

use heatmap::HeatmapContent;
use kline::{Action as KlineAction, ChartCommand, DataUpdate, FetchUpdate, KlineChart};
use ladder::Ladder;
use timesales::TimeAndSales;

#[derive(Default)]
pub enum Content {
    #[default]
    Starter,
    ShaderHeatmap(HeatmapContent),
    Kline {
        chart: Option<Box<KlineChart>>,
        layout: Option<Box<data::chart::kline::Config>>,
    },
    TimeAndSales(Option<Box<TimeAndSales>>),
    Ladder(Option<Box<Ladder>>),
}

impl Content {
    pub fn heatmap_indicator_config(
        &self,
    ) -> Option<&data::chart::heatmap::indicators::HeatmapIndicatorConfig> {
        match self {
            Self::ShaderHeatmap(heatmap) => Some(heatmap.indicator_config()),
            _ => None,
        }
    }

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
            || KlineChart::new(basis, tickers),
            |layout| KlineChart::new_with_config(basis, tickers, layout),
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
            ContentKind::ShaderHeatmap => {
                Content::ShaderHeatmap(HeatmapContent::Parked(Default::default()))
            }
            ContentKind::TimeAndSales => Content::TimeAndSales(None),
            ContentKind::Ladder => Content::Ladder(None),
        }
    }

    pub fn settings_snapshot(
        &self,
        transfer: PaneSettingsTransfer,
    ) -> Option<PaneSettingsSnapshot> {
        match (transfer, self) {
            (PaneSettingsTransfer::Visual(config), Content::ShaderHeatmap(heatmap)) => Some(
                PaneSettingsSnapshot::Heatmap(data::chart::heatmap::SettingsSnapshot {
                    config: config.as_ref().and_then(VisualConfig::heatmap),
                    indicators: heatmap.indicator_config().clone(),
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
            (Content::ShaderHeatmap(_), PaneSettingsSnapshot::Heatmap(_))
                | (
                    Content::Kline { chart: Some(_), .. },
                    PaneSettingsSnapshot::Kline(_)
                )
                | (
                    Content::TimeAndSales(_),
                    PaneSettingsSnapshot::TimeAndSales(_)
                )
                | (Content::Ladder(_), PaneSettingsSnapshot::Ladder(_))
        )
    }

    pub fn apply_settings_snapshot(
        &mut self,
        snapshot: &PaneSettingsSnapshot,
    ) -> Option<KlineAction> {
        match (self, snapshot) {
            (Content::ShaderHeatmap(heatmap), PaneSettingsSnapshot::Heatmap(snapshot)) => {
                heatmap.set_indicator_config(snapshot.indicators.clone());
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
            Content::Kline { chart, .. } => Some(chart.as_ref()?.last_maintenance_at()),
            Content::TimeAndSales(panel) => Some(panel.as_ref()?.last_update()),
            Content::Ladder(panel) => Some(panel.as_ref()?.last_update()),
            Content::Starter => None,
            Content::ShaderHeatmap(HeatmapContent::Active(chart)) => chart.last_tick,
            Content::ShaderHeatmap(HeatmapContent::Parked(_)) => None,
        }
    }

    fn update_interval(&self) -> Option<u64> {
        match self {
            Content::Kline { .. } => Some(250),
            Content::Ladder(_) | Content::TimeAndSales(_) => Some(100),
            Content::ShaderHeatmap(_) | Content::Starter => None,
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
            Content::ShaderHeatmap(HeatmapContent::Active(chart)) => chart.invalidate(Some(now)),
            Content::ShaderHeatmap(HeatmapContent::Parked(_)) => None,
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
            Content::ShaderHeatmap(HeatmapContent::Active(chart)) => chart
                .rebuild_indicators()
                .map(|task| task.map(Event::HeatmapBackground)),
            _ => None,
        }
    }

    pub(super) fn park(&mut self) {
        if let Content::ShaderHeatmap(heatmap) = self {
            heatmap.park();
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

    pub fn ingest_live_kline(&mut self, stream: &StreamKind, kline: &Kline) {
        if let Self::Kline {
            chart: Some(chart), ..
        } = self
            && let StreamKind::Kline {
                timeframe,
                ticker_info,
            } = stream
        {
            chart.apply_data(DataUpdate::LiveKline {
                timeframe: *timeframe,
                ticker_info,
                kline,
            });
        }
    }

    pub fn ingest_live_trades(&mut self, stream: &StreamKind, trades: &[Trade], update_t: UnixMs) {
        match self {
            Self::ShaderHeatmap(HeatmapContent::Active(chart)) => {
                chart.insert_trades(trades, update_t);
            }
            Self::ShaderHeatmap(HeatmapContent::Parked(_)) => {}
            Self::Kline {
                chart: Some(chart), ..
            } => {
                let ticker_info = stream.ticker_info();
                chart.apply_data(DataUpdate::LiveTrades {
                    ticker_info: &ticker_info,
                    trades,
                });
            }
            Self::TimeAndSales(panel) => {
                if let Some(panel) = panel {
                    panel.insert_buffer(stream.ticker_info(), trades);
                }
            }
            Self::Ladder(panel) => {
                if let Some(panel) = panel {
                    panel.insert_trades(trades);
                }
            }
            _ => log::error!("No chart found for the stream: {stream:?}"),
        }
    }

    pub fn ingest_live_depth(&mut self, stream: &StreamKind, depth: &Depth, update_t: UnixMs) {
        match self {
            Self::ShaderHeatmap(HeatmapContent::Active(chart)) => {
                chart.insert_depth(depth, update_t);
            }
            Self::ShaderHeatmap(HeatmapContent::Parked(_)) => {}
            Self::Ladder(panel) => {
                if let Some(panel) = panel {
                    panel.insert_depth(depth, update_t);
                }
            }
            _ => log::error!("No chart found for the stream: {stream:?}"),
        }
    }

    pub fn insert_historical_trades(
        &mut self,
        ticker_info: TickerInfo,
        trades: &[Trade],
        is_batches_done: bool,
        req_id: Option<uuid::Uuid>,
    ) -> Result<(), &'static str> {
        match self {
            Self::Kline {
                chart: Some(chart), ..
            } => {
                chart.apply_data(DataUpdate::HistoricalTrades {
                    ticker_info,
                    trades,
                    is_batches_done,
                    req_id,
                });
                Ok(())
            }
            Self::Kline { chart: None, .. } if req_id.is_some() => Ok(()),
            Self::Kline { chart: None, .. } => Err("fetched trades but no chart found"),
            _ if req_id.is_some() => Ok(()),
            _ => Err("No matching chart found for fetched trades"),
        }
    }

    pub(super) fn insert_historical_klines(
        &mut self,
        tickers: impl IntoIterator<Item = TickerInfo>,
        req_id: Option<uuid::Uuid>,
        timeframe: Timeframe,
        ticker_info: TickerInfo,
        klines: &[Kline],
        is_batches_done: bool,
        has_data: bool,
    ) {
        let Self::Kline { chart, .. } = self else {
            if req_id.is_none() {
                log::error!("pane content not candlestick");
            }
            return;
        };

        if chart.is_none() {
            if req_id.is_some() {
                return;
            }

            let mut initial_tickers = Vec::new();
            for ticker in tickers {
                if !initial_tickers.contains(&ticker) {
                    initial_tickers.push(ticker);
                }
            }
            if initial_tickers.is_empty() {
                initial_tickers.push(ticker_info);
            }

            *chart = Some(Box::new(KlineChart::new(
                Basis::Time(timeframe),
                &initial_tickers,
            )));
        }

        if let Some(chart) = chart.as_mut() {
            chart.apply_data(DataUpdate::HistoricalKlines {
                req_id,
                timeframe,
                ticker_info,
                klines,
                is_batches_done,
                has_data,
            });
        }
    }

    pub(super) fn insert_historical_open_interest(
        &mut self,
        req_id: Option<uuid::Uuid>,
        ticker_info: TickerInfo,
        series: &OpenInterestSeries,
    ) {
        let Self::Kline { chart, .. } = self else {
            if req_id.is_none() {
                log::error!("pane content not candlestick");
            }
            return;
        };

        if let Some(chart) = chart.as_mut() {
            chart.apply_data(DataUpdate::HistoricalOpenInterest {
                req_id,
                ticker_info,
                series,
            });
        }
    }

    pub fn apply_fetch_failure(&mut self, req_id: uuid::Uuid) -> bool {
        if let Self::Kline {
            chart: Some(chart), ..
        } = self
        {
            chart.apply_fetch_update(FetchUpdate::Failed(req_id))
        } else {
            false
        }
    }

    pub fn apply_trade_fetch_no_data(
        &mut self,
        ticker_info: TickerInfo,
        req_id: uuid::Uuid,
    ) -> bool {
        if let Self::Kline {
            chart: Some(chart), ..
        } = self
        {
            chart.apply_fetch_update(FetchUpdate::TradeFetchNoData {
                ticker_info,
                req_id,
            })
        } else {
            false
        }
    }

    pub(super) fn apply_fetch_handle(&mut self, req_id: uuid::Uuid, handle: Handle) -> bool {
        if let Self::Kline {
            chart: Some(chart), ..
        } = self
        {
            chart.apply_fetch_update(FetchUpdate::FetchHandle { req_id, handle })
        } else {
            false
        }
    }

    pub fn change_visual_config(&mut self, config: VisualConfig) {
        match (self, config) {
            (Content::ShaderHeatmap(HeatmapContent::Active(c)), VisualConfig::Heatmap(cfg)) => {
                c.set_visual_config(cfg);
            }
            (Content::TimeAndSales(Some(panel)), VisualConfig::TimeAndSales(cfg)) => {
                panel.set_config(cfg);
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
            Content::ShaderHeatmap(_) => ContentKind::ShaderHeatmap,
        }
    }

    pub fn update_theme(&mut self, theme: &iced_core::Theme) {
        if let Content::ShaderHeatmap(HeatmapContent::Active(c)) = self {
            c.update_theme(theme);
        }
    }

    pub(super) fn initialized(&self) -> bool {
        match self {
            Content::ShaderHeatmap(heatmap) => matches!(heatmap, HeatmapContent::Active(_)),
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
    ) -> Option<InspectorContent<'a, Message>> {
        match self {
            Self::Kline {
                chart: Some(chart), ..
            } => {
                let mut content = chart
                    .pane_settings_view()
                    .map(move |message| Message::PaneEvent(pane, Event::KlineInteraction(message)));
                content.set_footer(
                    row![
                        space::horizontal(),
                        crate::tooltip(
                            button("Sync all").on_press(Message::SyncKlineSettings(pane)),
                            Some("Apply configuration to similar panes"),
                            iced::widget::tooltip::Position::Left,
                        ),
                    ]
                    .align_y(Alignment::Center),
                );
                Some(content)
            }
            Self::ShaderHeatmap(HeatmapContent::Active(chart)) => {
                let basis = selected_basis.unwrap_or(Basis::default_heatmap_time(ticker_info));

                Some(
                    heatmap_shader_cfg_view(
                        chart.visual_config(),
                        basis,
                        chart.ticker_info.exchange().venue(),
                        chart.indicator_config(),
                        chart.depth_distance_input(),
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
            | Self::ShaderHeatmap(HeatmapContent::Parked(_)) => None,
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
                | (Content::ShaderHeatmap(_), Content::ShaderHeatmap(_))
                | (Content::Kline { .. }, Content::Kline { .. })
                | (Content::TimeAndSales(_), Content::TimeAndSales(_))
                | (Content::Ladder(_), Content::Ladder(_))
        )
    }
}

fn settings_message(pane: pane_grid::Pane, message: settings::Message) -> Message {
    match message {
        settings::Message::VisualConfigChanged { config, sync_all } => {
            Message::VisualConfigChanged(pane, config, sync_all)
        }
        message => Message::PaneEvent(pane, Event::Settings(message)),
    }
}
