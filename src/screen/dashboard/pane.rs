use crate::{
    chart::{self, heatmap::HeatmapChart, kline::KlineChart},
    modal::{
        self, ModifierKind,
        pane::{
            settings::{heatmap_cfg_view, kline_cfg_view},
            stack_modal,
        },
    },
    screen::{DashboardError, dashboard::panel::timeandsales::TimeAndSales},
    style::{self, Icon, icon_text},
    widget::{self, button_with_tooltip, column_drag, toast::Toast},
    window::{self, Window},
};
use data::{
    UserTimezone,
    chart::{
        Basis, ChartLayout, VisualConfig,
        indicator::{HeatmapIndicator, Indicator, KlineIndicator},
    },
    layout::pane::Settings,
};
use exchange::{
    Kline, OpenInterest, TickMultiplier, Ticker, TickerInfo, Timeframe,
    adapter::{Exchange, MarketKind, StreamKind},
};
use iced::{
    Alignment, Element, Length, Renderer, Task, Theme,
    alignment::Vertical,
    padding,
    widget::{button, center, pane_grid, row, text, tooltip},
};
use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InfoType {
    FetchingKlines,
    FetchingTrades(usize),
    FetchingOI,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub enum Status {
    #[default]
    Ready,
    Loading(InfoType),
    Stale(String),
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
pub enum Modal {
    StreamModifier(modal::stream::Modifier),
    Settings,
    Indicators,
}

#[derive(Debug, Clone)]
pub enum Message {
    PaneClicked(pane_grid::Pane),
    PaneResized(pane_grid::ResizeEvent),
    PaneDragged(pane_grid::DragEvent),
    ClosePane(pane_grid::Pane),
    SplitPane(pane_grid::Axis, pane_grid::Pane),
    MaximizePane(pane_grid::Pane),
    Restore,
    ShowModal(pane_grid::Pane, Modal),
    HideModal(pane_grid::Pane),
    InitPaneContent(String, Option<pane_grid::Pane>, Vec<StreamKind>, TickerInfo),
    ReplacePane(pane_grid::Pane),
    ChartUserUpdate(pane_grid::Pane, chart::Message),
    VisualConfigChanged(Option<pane_grid::Pane>, VisualConfig),
    ToggleIndicator(pane_grid::Pane, String),
    Popout,
    Merge,
    DeleteNotification(pane_grid::Pane, usize),
    ReorderIndicator(pane_grid::Pane, column_drag::DragEvent),
    ClusterKindSelected(pane_grid::Pane, data::chart::kline::ClusterKind),
    StreamModifierChanged(pane_grid::Pane, modal::stream::Message),
    StudyConfigurator(pane_grid::Pane, modal::pane::settings::study::Message),
}

pub struct State {
    id: uuid::Uuid,
    pub modal: Option<Modal>,
    pub content: Content,
    pub settings: Settings,
    pub notifications: Vec<Toast>,
    pub streams: Vec<StreamKind>,
    pub status: Status,
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_config(content: Content, streams: Vec<StreamKind>, settings: Settings) -> Self {
        Self {
            content,
            settings,
            streams,
            ..Default::default()
        }
    }

    pub fn stream_pair(&self) -> Option<(Exchange, Ticker)> {
        self.streams
            .iter()
            .map(|stream| match stream {
                StreamKind::DepthAndTrades { exchange, ticker }
                | StreamKind::Kline {
                    exchange, ticker, ..
                } => (*exchange, *ticker),
            })
            .next()
    }

    pub fn init_content_task(
        &mut self,
        content: &str,
        exchange: Exchange,
        ticker_info: TickerInfo,
        pane: pane_grid::Pane,
    ) -> Task<Message> {
        if (matches!(&self.content, Content::Heatmap(_, _)) && content != "heatmap")
            || (matches!(&self.content, Content::Kline(_, _)) && content == "heatmap")
        {
            self.settings.selected_basis = None;
        }

        let streams = match content {
            "heatmap" | "time&sales" => {
                vec![StreamKind::DepthAndTrades {
                    exchange,
                    ticker: ticker_info.ticker,
                }]
            }
            "footprint" => {
                let basis = self.settings.selected_basis.unwrap_or(Timeframe::M5.into());

                match basis {
                    Basis::Time(timeframe) => {
                        vec![
                            StreamKind::DepthAndTrades {
                                exchange,
                                ticker: ticker_info.ticker,
                            },
                            StreamKind::Kline {
                                exchange,
                                ticker: ticker_info.ticker,
                                timeframe,
                            },
                        ]
                    }
                    Basis::Tick(_) => {
                        vec![StreamKind::DepthAndTrades {
                            exchange,
                            ticker: ticker_info.ticker,
                        }]
                    }
                }
            }
            "candlestick" => {
                let basis = self
                    .settings
                    .selected_basis
                    .unwrap_or(Timeframe::M15.into());

                match basis {
                    Basis::Time(timeframe) => {
                        vec![StreamKind::Kline {
                            exchange,
                            ticker: ticker_info.ticker,
                            timeframe,
                        }]
                    }
                    Basis::Tick(_) => {
                        vec![StreamKind::DepthAndTrades {
                            exchange,
                            ticker: ticker_info.ticker,
                        }]
                    }
                }
            }
            _ => vec![],
        };

        self.streams.clone_from(&streams);

        Task::done(Message::InitPaneContent(
            content.to_string(),
            Some(pane),
            streams,
            ticker_info,
        ))
    }

    pub fn set_content(
        &mut self,
        ticker_info: TickerInfo,
        content_str: &str,
    ) -> Result<(), DashboardError> {
        self.settings.ticker_info = Some(ticker_info);

        let new_content = match content_str {
            "heatmap" => {
                let tick_multiplier = Some(TickMultiplier(5));
                self.settings.tick_multiply = tick_multiplier;
                let tick_size = tick_multiplier.map_or(ticker_info.min_ticksize, |tm| {
                    tm.multiply_with_min_tick_size(ticker_info)
                });

                Content::new_heatmap(&self.content, ticker_info, &self.settings, tick_size)
            }
            "footprint" | "candlestick" => {
                let tick_multiplier = if content_str == "footprint" {
                    Some(TickMultiplier(50))
                } else {
                    None
                };
                self.settings.tick_multiply = tick_multiplier;
                let tick_size = tick_multiplier.map_or(ticker_info.min_ticksize, |tm| {
                    tm.multiply_with_min_tick_size(ticker_info)
                });

                Content::new_kline(
                    content_str,
                    &self.content,
                    ticker_info,
                    &self.settings,
                    tick_size,
                )
            }
            "time&sales" => {
                self.settings.ticker_info = Some(ticker_info);

                let config = self
                    .settings
                    .visual_config
                    .and_then(|cfg| cfg.time_and_sales());

                Content::TimeAndSales(TimeAndSales::new(config, Some(ticker_info)))
            }
            _ => {
                log::error!("content not found: {}", content_str);
                return Err(DashboardError::PaneSet(format!(
                    "content not found: {}",
                    content_str
                )));
            }
        };

        self.content = new_content;
        Ok(())
    }

    pub fn insert_oi_vec(&mut self, req_id: Option<uuid::Uuid>, oi: &[OpenInterest]) {
        match &mut self.content {
            Content::Kline(chart, _) => {
                chart.insert_open_interest(req_id, oi);
            }
            _ => {
                log::error!("pane content not candlestick");
            }
        }
    }

    pub fn insert_klines_vec(
        &mut self,
        req_id: Option<uuid::Uuid>,
        timeframe: Timeframe,
        klines: &[Kline],
    ) {
        match &mut self.content {
            Content::Kline(chart, indicators) => {
                if let Some(id) = req_id {
                    chart.insert_new_klines(id, klines);
                } else {
                    let (raw_trades, tick_size) = (chart.raw_trades(), chart.tick_size());
                    let layout = chart.chart_layout();
                    let ticker_info = self.settings.ticker_info;

                    *chart = KlineChart::new(
                        layout,
                        Basis::Time(timeframe),
                        tick_size,
                        klines,
                        raw_trades,
                        indicators,
                        ticker_info,
                        chart.kind(),
                    );
                }
            }
            _ => {
                log::error!("pane content not candlestick or footprint");
            }
        }
    }

    pub fn view<'a>(
        &'a self,
        id: pane_grid::Pane,
        panes: usize,
        is_focused: bool,
        maximized: bool,
        window: window::Id,
        main_window: &'a Window,
        timezone: UserTimezone,
    ) -> pane_grid::Content<'a, Message, Theme, Renderer> {
        let mut stream_info_element = row![]
            .padding(padding::left(8))
            .align_y(Vertical::Center)
            .spacing(8)
            .height(Length::Fixed(32.0));

        if let Some((exchange, ticker)) = self.stream_pair() {
            let exchange_info = match exchange {
                Exchange::BinanceSpot | Exchange::BinanceLinear | Exchange::BinanceInverse => {
                    icon_text(Icon::BinanceLogo, 14)
                }
                Exchange::BybitSpot | Exchange::BybitLinear | Exchange::BybitInverse => {
                    icon_text(Icon::BybitLogo, 14)
                }
            };

            let ticker_str = {
                let symbol = ticker.display_symbol_and_type().0;
                match ticker.market_type() {
                    MarketKind::Spot => symbol,
                    MarketKind::LinearPerps | MarketKind::InversePerps => symbol + " PERP",
                }
            };

            stream_info_element = stream_info_element.push(
                row![exchange_info, text(ticker_str).size(14),]
                    .align_y(Vertical::Center)
                    .spacing(4),
            );
        }

        let modifier: Option<modal::stream::Modifier> = self.modal.and_then(|m| {
            if let Modal::StreamModifier(modifier) = m {
                Some(modifier)
            } else {
                None
            }
        });

        let body = match &self.content {
            Content::Starter => center(text("select a ticker to start").size(16)).into(),
            Content::TimeAndSales(panel) => super::panel::view(id, self, panel, timezone),
            Content::Heatmap(chart, indicators) => {
                let selected_basis = self
                    .settings
                    .selected_basis
                    .unwrap_or(Basis::default_heatmap_time(self.settings.ticker_info));
                let tick_multiply = self.settings.tick_multiply.unwrap_or(TickMultiplier(5));
                let kind = ModifierKind::Heatmap(selected_basis, tick_multiply);

                let base_ticksize = tick_multiply.base(chart.tick_size());

                let modifiers = row![
                    basis_modifier(id, selected_basis, modifier, kind),
                    ticksize_modifier(id, base_ticksize, tick_multiply, modifier, kind),
                ]
                .spacing(4);

                stream_info_element = stream_info_element.push(modifiers);

                let base = chart::view(chart, indicators, timezone)
                    .map(move |message| Message::ChartUserUpdate(id, message));
                let settings_modal = || heatmap_cfg_view(chart.visual_config(), id);

                self.compose_chart_view(base, id, indicators, settings_modal)
            }
            Content::Kline(chart, indicators) => {
                let chart_kind = chart.kind();

                match chart_kind {
                    data::chart::KlineChartKind::Footprint { .. } => {
                        let selected_basis =
                            self.settings.selected_basis.unwrap_or(Timeframe::M5.into());
                        let tick_multiply =
                            self.settings.tick_multiply.unwrap_or(TickMultiplier(10));
                        let kind = ModifierKind::Footprint(selected_basis, tick_multiply);

                        let base_ticksize = tick_multiply.base(chart.tick_size());

                        let modifiers = row![
                            basis_modifier(id, selected_basis, modifier, kind),
                            ticksize_modifier(id, base_ticksize, tick_multiply, modifier, kind),
                        ]
                        .spacing(4);

                        stream_info_element = stream_info_element.push(modifiers);
                    }
                    data::chart::KlineChartKind::Candles => {
                        let selected_basis = self
                            .settings
                            .selected_basis
                            .unwrap_or(Timeframe::M15.into());
                        let kind = ModifierKind::Candlestick(selected_basis);

                        let modifiers =
                            row![basis_modifier(id, selected_basis, modifier, kind),].spacing(4);

                        stream_info_element = stream_info_element.push(modifiers);
                    }
                }

                let base = chart::view(chart, indicators, timezone)
                    .map(move |message| Message::ChartUserUpdate(id, message));
                let settings_modal = || kline_cfg_view(chart.study_configurator(), chart_kind, id);

                self.compose_chart_view(base, id, indicators, settings_modal)
            }
        };

        match &self.status {
            Status::Loading(InfoType::FetchingKlines) => {
                stream_info_element = stream_info_element.push(text("Fetching Klines..."));
            }
            Status::Loading(InfoType::FetchingTrades(count)) => {
                stream_info_element =
                    stream_info_element.push(text(format!("Fetching Trades... {count} fetched")));
            }
            Status::Loading(InfoType::FetchingOI) => {
                stream_info_element = stream_info_element.push(text("Fetching Open Interest..."));
            }
            Status::Stale(msg) => {
                stream_info_element = stream_info_element.push(text(msg));
            }
            Status::Ready => {}
        }

        let content = pane_grid::Content::new(body)
            .style(move |theme| style::pane_background(theme, is_focused));

        let title_bar = pane_grid::TitleBar::new(stream_info_element)
            .controls(self.view_controls(id, panes, maximized, window != main_window.id))
            .style(style::pane_title_bar);

        content.title_bar(if self.modal.is_none() {
            title_bar
        } else {
            title_bar.always_show_controls()
        })
    }

    fn view_controls(
        &self,
        pane: pane_grid::Pane,
        total_panes: usize,
        is_maximized: bool,
        is_popout: bool,
    ) -> Element<Message> {
        let modal_btn_style = |modal: Modal| {
            let is_active = self.modal == Some(modal);
            move |theme: &Theme, status: button::Status| {
                style::button::transparent(theme, status, is_active)
            }
        };

        let control_btn_style = |is_active: bool| {
            move |theme: &Theme, status: button::Status| {
                style::button::transparent(theme, status, is_active)
            }
        };

        let tooltip_pos = tooltip::Position::Bottom;
        let mut buttons = row![];

        if !matches!(&self.content, Content::Starter) {
            buttons = buttons.push(button_with_tooltip(
                icon_text(Icon::Cog, 12),
                Message::ShowModal(pane, Modal::Settings),
                None,
                tooltip_pos,
                modal_btn_style(Modal::Settings),
            ));
        }

        if matches!(&self.content, Content::Heatmap(_, _) | Content::Kline(_, _)) {
            buttons = buttons.push(button_with_tooltip(
                icon_text(Icon::ChartOutline, 12),
                Message::ShowModal(pane, Modal::Indicators),
                Some("Indicators"),
                tooltip_pos,
                modal_btn_style(Modal::Indicators),
            ));
        }

        if is_popout {
            buttons = buttons.push(button_with_tooltip(
                icon_text(Icon::Popout, 12),
                Message::Merge,
                Some("Merge"),
                tooltip_pos,
                control_btn_style(is_popout),
            ));
        } else if total_panes > 1 {
            buttons = buttons.push(button_with_tooltip(
                icon_text(Icon::Popout, 12),
                Message::Popout,
                Some("Pop out"),
                tooltip_pos,
                control_btn_style(is_popout),
            ));
        }

        if total_panes > 1 {
            let (resize_icon, message) = if is_maximized {
                (Icon::ResizeSmall, Message::Restore)
            } else {
                (Icon::ResizeFull, Message::MaximizePane(pane))
            };

            buttons = buttons.push(button_with_tooltip(
                icon_text(resize_icon, 12),
                message,
                None,
                tooltip_pos,
                control_btn_style(is_maximized),
            ));

            buttons = buttons.push(button_with_tooltip(
                icon_text(Icon::Close, 12),
                Message::ClosePane(pane),
                None,
                tooltip_pos,
                control_btn_style(false),
            ));
        }

        buttons
            .padding(padding::right(4))
            .align_y(Vertical::Center)
            .height(Length::Fixed(32.0))
            .into()
    }

    fn compose_chart_view<'a, F>(
        &'a self,
        base: Element<'a, Message>,
        pane: pane_grid::Pane,
        indicators: &'a [impl Indicator],
        settings_modal: F,
    ) -> Element<'a, Message>
    where
        F: FnOnce() -> Element<'a, Message>,
    {
        let base =
            widget::toast::Manager::new(base, &self.notifications, Alignment::End, move |msg| {
                Message::DeleteNotification(pane, msg)
            })
            .into();

        let stack_padding = padding::right(12).left(12);

        match self.modal {
            Some(Modal::StreamModifier(modifier)) => stack_modal(
                base,
                modifier
                    .view(self.stream_pair())
                    .map(move |message| Message::StreamModifierChanged(pane, message)),
                Message::HideModal(pane),
                stack_padding,
                Alignment::Start,
            ),
            Some(Modal::Indicators) => stack_modal(
                base,
                modal::indicators::view(pane, self, indicators),
                Message::HideModal(pane),
                stack_padding,
                Alignment::End,
            ),
            Some(Modal::Settings) => stack_modal(
                base,
                settings_modal(),
                Message::HideModal(pane),
                stack_padding,
                Alignment::End,
            ),
            None => base,
        }
    }

    pub fn matches_stream(&self, stream: &StreamKind) -> bool {
        self.streams.iter().any(|existing| existing == stream)
    }

    pub fn invalidate(&mut self, now: Instant) -> Option<chart::Action> {
        self.content.invalidate(now)
    }

    pub fn basis_interval(&self) -> Option<u64> {
        match &self.content {
            Content::Kline(_, _) => Some(1000),
            Content::Heatmap(chart, _) => chart.basis_interval(),
            Content::Starter | Content::TimeAndSales(_) => None,
        }
    }

    pub fn last_tick(&self) -> Option<Instant> {
        self.content.last_tick()
    }

    pub fn tick(&mut self, now: Instant) -> Option<chart::Action> {
        let invalidate_interval: Option<u64> = self.basis_interval();
        let last_tick: Option<Instant> = self.last_tick();

        match (invalidate_interval, last_tick) {
            (Some(interval_ms), Some(previous_tick_time)) => {
                if interval_ms > 0 {
                    let interval_duration = std::time::Duration::from_millis(interval_ms);
                    if now.duration_since(previous_tick_time) >= interval_duration {
                        return self.invalidate(now);
                    }
                }
            }
            (Some(interval_ms), None) => {
                if interval_ms > 0 {
                    return self.invalidate(now);
                }
            }
            (None, _) => {}
        }

        None
    }

    pub fn unique_id(&self) -> uuid::Uuid {
        self.id
    }
}

impl Default for State {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            modal: None,
            content: Content::Starter,
            settings: Settings::default(),
            streams: vec![],
            notifications: vec![],
            status: Status::Ready,
        }
    }
}

#[derive(Default)]
pub enum Content {
    #[default]
    Starter,
    Heatmap(HeatmapChart, Vec<HeatmapIndicator>),
    Kline(KlineChart, Vec<KlineIndicator>),
    TimeAndSales(TimeAndSales),
}

impl Content {
    fn new_heatmap(
        current_content: &Content,
        ticker_info: TickerInfo,
        settings: &Settings,
        tick_size: f32,
    ) -> Self {
        let (enabled_indicators, layout) = if let Content::Heatmap(chart, inds) = current_content {
            (inds.clone(), chart.chart_layout())
        } else {
            (
                vec![HeatmapIndicator::Volume],
                ChartLayout {
                    crosshair: true,
                    splits: vec![],
                },
            )
        };

        let basis = settings
            .selected_basis
            .unwrap_or_else(|| Basis::default_heatmap_time(Some(ticker_info)));
        let config = settings.visual_config.and_then(|cfg| cfg.heatmap());

        Content::Heatmap(
            HeatmapChart::new(
                layout,
                basis,
                tick_size,
                &enabled_indicators,
                Some(ticker_info),
                config,
            ),
            enabled_indicators,
        )
    }

    fn new_kline(
        content_str: &str, // "footprint" or "candlestick"
        current_content: &Content,
        ticker_info: TickerInfo,
        settings: &Settings,
        tick_size: f32,
    ) -> Self {
        let (prev_indis, prev_layout, prev_kind_opt) =
            if let Content::Kline(chart, inds) = current_content {
                (
                    Some(inds.clone()),
                    Some(chart.chart_layout()),
                    Some(chart.kind().clone()),
                )
            } else {
                (None, None, None)
            };

        let (default_tf, determined_chart_kind) = match content_str {
            "footprint" => (
                Timeframe::M5,
                prev_kind_opt
                    .filter(|k| matches!(k, data::chart::KlineChartKind::Footprint { .. }))
                    .unwrap_or_else(|| data::chart::KlineChartKind::Footprint {
                        clusters: data::chart::kline::ClusterKind::default(),
                        studies: vec![],
                    }),
            ),
            _ => (
                // "candlestick"
                Timeframe::M15,
                data::chart::KlineChartKind::Candles,
            ),
        };

        let basis = settings.selected_basis.unwrap_or(Basis::Time(default_tf));

        let enabled_indicators = {
            let available = KlineIndicator::for_market(ticker_info.market_type());
            prev_indis.map_or_else(
                || vec![KlineIndicator::Volume],
                |indis| {
                    indis
                        .into_iter()
                        .filter(|i| available.contains(i))
                        .collect()
                },
            )
        };

        let splits = {
            let main_chart_split: f32 = 0.8;
            let mut splits_vec = vec![main_chart_split];

            if !enabled_indicators.is_empty() {
                let num_indicators = enabled_indicators.len();

                if num_indicators > 0 {
                    let indicator_total_height_ratio = 1.0 - main_chart_split;
                    let height_per_indicator_pane =
                        indicator_total_height_ratio / num_indicators as f32;

                    let mut current_split_pos = main_chart_split;
                    for _ in 0..(num_indicators - 1) {
                        current_split_pos += height_per_indicator_pane;
                        splits_vec.push(current_split_pos);
                    }
                }
            }
            splits_vec
        };

        let layout = prev_layout
            .filter(|l| l.splits.len() == splits.len())
            .unwrap_or(ChartLayout {
                crosshair: true,
                splits,
            });

        Content::Kline(
            KlineChart::new(
                layout,
                basis,
                tick_size,
                &[],
                vec![],
                &enabled_indicators,
                Some(ticker_info),
                &determined_chart_kind,
            ),
            enabled_indicators,
        )
    }

    pub fn invalidate(&mut self, now: Instant) -> Option<chart::Action> {
        match self {
            Content::Heatmap(chart, _) => chart.invalidate(Some(now)),
            Content::Kline(chart, _) => chart.invalidate(Some(now)),
            Content::Starter | Content::TimeAndSales(_) => None,
        }
    }

    pub fn last_tick(&self) -> Option<Instant> {
        match self {
            Content::Heatmap(chart, _) => Some(chart.last_update()),
            Content::Kline(chart, _) => Some(chart.last_update()),
            Content::Starter | Content::TimeAndSales(_) => None,
        }
    }

    pub fn chart_kind(&self) -> Option<data::chart::KlineChartKind> {
        match self {
            Content::Kline(chart, _) => Some(chart.kind().clone()),
            _ => None,
        }
    }

    pub fn toggle_indicator(&mut self, indicator_str: &str) {
        match self {
            Content::Heatmap(chart, indicators) => {
                let indicator = match indicator_str {
                    "Volume" => HeatmapIndicator::Volume,
                    "VPSR" => HeatmapIndicator::SessionVolumeProfile,
                    _ => {
                        panic!("heatmap indicator requested to toggle not found: {indicator_str}",);
                    }
                };

                if indicators.contains(&indicator) {
                    indicators.retain(|i| i != &indicator);
                } else {
                    indicators.push(indicator);
                }

                chart.toggle_indicator(indicator);
            }
            Content::Kline(chart, indicators) => {
                let indicator = match indicator_str {
                    "Volume" => KlineIndicator::Volume,
                    "Open Interest" => KlineIndicator::OpenInterest,
                    _ => {
                        panic!("kline indicator requested to toggle not found: {indicator_str}",);
                    }
                };

                if indicators.contains(&indicator) {
                    indicators.retain(|i| i != &indicator);
                } else {
                    indicators.push(indicator);
                }

                chart.toggle_indicator(indicator);
            }
            Content::Starter | Content::TimeAndSales(_) => {
                panic!("indicator toggle on {} pane", self)
            }
        }
    }

    pub fn reorder_indicators(&mut self, event: &column_drag::DragEvent) {
        match self {
            Content::Heatmap(_, indicator) => column_drag::reorder_vec(indicator, event),
            Content::Kline(_, indicator) => column_drag::reorder_vec(indicator, event),
            Content::TimeAndSales(_) | Content::Starter => {
                panic!("indicator reorder on {} pane", self)
            }
        }
    }

    pub fn change_visual_config(&mut self, config: VisualConfig) {
        match (self, config) {
            (Content::Heatmap(chart, _), VisualConfig::Heatmap(cfg)) => {
                chart.set_visual_config(cfg);
            }
            (Content::TimeAndSales(panel), VisualConfig::TimeAndSales(cfg)) => {
                panel.config = cfg;
            }
            _ => {}
        }
    }
}

impl std::fmt::Display for Content {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Content::Starter => write!(f, "Starter pane"),
            Content::Heatmap(_, _) => write!(f, "Heatmap chart"),
            Content::Kline(chart, _) => match chart.kind() {
                data::chart::KlineChartKind::Footprint { .. } => write!(f, "Footprint chart"),
                data::chart::KlineChartKind::Candles => write!(f, "Candlestick chart"),
            },
            Content::TimeAndSales(_) => write!(f, "Time&Sales pane"),
        }
    }
}

fn ticksize_modifier<'a>(
    id: pane_grid::Pane,
    base_ticksize: f32,
    multiplier: TickMultiplier,
    modifier: Option<modal::stream::Modifier>,
    kind: ModifierKind,
) -> Element<'a, Message> {
    let modifier_modal = Modal::StreamModifier(
        modal::stream::Modifier::new(kind).with_ticksize_view(base_ticksize, multiplier),
    );

    let is_active = modifier.is_some_and(|m| {
        matches!(
            m.view_mode,
            modal::stream::ViewMode::TicksizeSelection { .. }
        )
    });

    button(text(multiplier.to_string()))
        .style(move |theme, status| style::button::modifier(theme, status, !is_active))
        .on_press(Message::ShowModal(id, modifier_modal))
        .into()
}

fn basis_modifier<'a>(
    id: pane_grid::Pane,
    selected_basis: Basis,
    modifier: Option<modal::stream::Modifier>,
    kind: ModifierKind,
) -> Element<'a, Message> {
    let modifier_modal = Modal::StreamModifier(
        modal::stream::Modifier::new(kind).with_view_mode(modal::stream::ViewMode::BasisSelection),
    );

    let is_active =
        modifier.is_some_and(|m| m.view_mode == modal::stream::ViewMode::BasisSelection);

    button(text(selected_basis.to_string()))
        .style(move |theme, status| style::button::modifier(theme, status, !is_active))
        .on_press(Message::ShowModal(id, modifier_modal))
        .into()
}
