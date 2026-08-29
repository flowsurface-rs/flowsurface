use super::content::{
    self, Content, DueTick,
    heatmap::HeatmapShader,
    kline::{
        Action as KlineAction, ChartCommand, CommandResult, DataUpdate, FootprintCommand,
        IndicatorCommand, KlineChart, KlineIndicator, StudyCommand, TickerCommand,
    },
    ladder::Ladder,
    timesales::TimeAndSales,
};

use crate::{
    connector::{
        ResolvedStream,
        fetcher::{FetchSpec, InfoKind},
    },
    modal::{
        self, ModifierKind,
        pane::{Modal, mini_tickers_list::MiniPanel, stack_modal},
    },
    screen::sidebar::{kline_indicators, pane_settings as settings, tickers_table::TickersTable},
    style::{self, Icon, icon_text},
    theme::ThemeAccents,
    widget::{
        self, button_with_tooltip, inspector::InspectorContent, link_group_button,
        press_button_with_tooltip, toast::Toast,
    },
    window::{self, PixelRatio, Window},
};
use data::{
    UserTimezone,
    chart::{Basis, heatmap::HeatmapStudy, kline::KlineStudy},
    layout::pane::{ContentKind, LinkGroup, PaneSetup, Settings, VisualConfig},
    stream::PersistStreamKind,
};
use exchange::{
    Kline, OpenInterest, StreamPairKind, TickMultiplier, TickerInfo, Timeframe,
    adapter::{MarketKind, StreamKind, StreamTicksize},
    unit::PriceStep,
};
use iced::{
    Alignment, Element, Length, Renderer, Theme, padding,
    widget::{button, center, column, container, pane_grid, pick_list, row, text, tooltip},
};
use std::time::Instant;

#[derive(Debug, Clone)]
pub enum Effect {
    RefreshStreams,
    RequestFetch(Vec<FetchSpec>),
    SwitchTickersInGroup(TickerInfo),
    FocusWidget(iced::widget::Id),
    OpenChartSettingsSidebar,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub enum Status {
    #[default]
    Ready,
    Loading(InfoKind),
    Stale(String),
}

pub enum Action {
    Chart(super::ChartAction),
    ResolveStreams(Vec<PersistStreamKind>),
    ResolveContent,
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
    ReplacePane(pane_grid::Pane),
    Popout,
    Merge,
    SwitchLinkGroup(pane_grid::Pane, Option<LinkGroup>),
    VisualConfigChanged(pane_grid::Pane, VisualConfig, bool),
    PaneEvent(pane_grid::Pane, Event),
}

#[derive(Debug, Clone)]
pub enum Event {
    ShowModal(Modal),
    HideModal,
    OpenInspector(crate::screen::sidebar::InspectorMode),
    ToggleInspector(crate::screen::sidebar::InspectorMode),
    SyncKlineSettings,
    ContentSelected(ContentKind),
    KlineInteraction(content::kline::Message),
    KlineIndicatorsInteraction(kline_indicators::Message),
    ToggleKlineIndicator(KlineIndicator),
    ToggleKlineStudy(KlineStudy),
    LadderInteraction(content::ladder::Message),
    TimeAndSalesInteraction(content::timesales::Message),
    DeleteNotification(usize),
    Settings(settings::Message),
    StreamModifierChanged(modal::stream::Message),
    HeatmapShaderInteraction(content::heatmap::Message),
    MiniTickersListInteraction(modal::pane::mini_tickers_list::Message),
}

pub(super) fn settings_message(pane: pane_grid::Pane, message: settings::Message) -> Message {
    match message {
        settings::Message::VisualConfigChanged { config, sync_all } => {
            Message::VisualConfigChanged(pane, config, sync_all)
        }
        message => Message::PaneEvent(pane, Event::Settings(message)),
    }
}

fn inspector_button_event(
    mode: crate::screen::sidebar::InspectorMode,
    active_inspector: Option<crate::screen::sidebar::InspectorMode>,
) -> Event {
    if active_inspector == Some(mode) {
        Event::ToggleInspector(mode)
    } else {
        Event::OpenInspector(mode)
    }
}

pub struct State {
    id: uuid::Uuid,
    pub modal: Option<Modal>,
    pub content: Content,
    pub settings: Settings,
    pub notifications: Vec<Toast>,
    pub streams: ResolvedStream,
    pub status: Status,
    pub link_group: Option<LinkGroup>,
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_config(
        content: Content,
        streams: Vec<PersistStreamKind>,
        settings: Settings,
        link_group: Option<LinkGroup>,
    ) -> Self {
        Self {
            content,
            settings,
            streams: ResolvedStream::waiting(streams),
            link_group,
            ..Default::default()
        }
    }

    pub fn stream_pair(&self) -> Option<TickerInfo> {
        self.streams.find_ready_map(|stream| match stream {
            StreamKind::Kline { ticker_info, .. } => Some(*ticker_info),
            StreamKind::Depth { ticker_info, .. } => Some(*ticker_info),
            StreamKind::Trades { ticker_info, .. } => Some(*ticker_info),
        })
    }

    fn sync_kline_trade_stream(&mut self) -> bool {
        let Some((ticker_info, basis, trade_data_required)) = (match &self.content {
            Content::Kline {
                chart: Some(chart), ..
            } => Some((
                chart.ticker_info(),
                chart.basis(),
                chart.trade_data_required(),
            )),
            _ => None,
        }) else {
            return false;
        };

        self.streams.set_trade_stream(
            ticker_info,
            matches!(basis, Basis::Tick(_)) || trade_data_required,
        )
    }

    pub fn stream_pair_kind(&self) -> Option<StreamPairKind> {
        let ready_streams = self.streams.ready_iter()?;
        let mut unique = vec![];

        for stream in ready_streams {
            let ticker = stream.ticker_info();
            if !unique.contains(&ticker) {
                unique.push(ticker);
            }
        }

        match unique.len() {
            0 => None,
            1 => Some(StreamPairKind::SingleSource(unique[0])),
            _ => Some(StreamPairKind::MultiSource(unique)),
        }
    }

    pub fn set_content_and_streams(
        &mut self,
        tickers: Vec<TickerInfo>,
        kind: ContentKind,
    ) -> Vec<StreamKind> {
        if !(self.content.kind() == kind) {
            self.settings.selected_basis = None;
            self.settings.tick_multiply = None;
        }

        let base_ticker = tickers[0];
        let prev_base_ticker = self.stream_pair();

        let derived_plan = PaneSetup::new(
            kind,
            base_ticker,
            prev_base_ticker,
            self.settings.selected_basis,
            self.settings.tick_multiply,
        );

        self.settings.selected_basis = derived_plan.basis;
        self.settings.tick_multiply = derived_plan.tick_multiplier;

        let (content, streams) = {
            let kline_stream = |ti: TickerInfo, tf: Timeframe| StreamKind::Kline {
                ticker_info: ti,
                timeframe: tf,
            };
            let depth_stream = |derived_plan: &PaneSetup| StreamKind::Depth {
                ticker_info: derived_plan.ticker_info,
                depth_aggr: derived_plan.depth_aggr,
                push_freq: derived_plan.push_freq,
            };
            let trades_stream = |derived_plan: &PaneSetup| StreamKind::Trades {
                ticker_info: derived_plan.ticker_info,
            };

            match kind {
                ContentKind::CandlestickChart => {
                    let tickers = if matches!(derived_plan.basis, Some(Basis::Tick(_))) {
                        vec![tickers[0]]
                    } else {
                        tickers.to_vec()
                    };

                    let content = Content::new_kline(&self.content, &tickers, &self.settings);
                    let trade_data_required = match &content {
                        Content::Kline {
                            chart: Some(chart), ..
                        } => chart.trade_data_required(),
                        _ => false,
                    };

                    let streams = by_basis_default(
                        derived_plan.basis,
                        Timeframe::M5,
                        |tf| {
                            let mut streams = Vec::new();
                            if trade_data_required {
                                streams.push(StreamKind::Trades {
                                    ticker_info: tickers[0],
                                });
                            }
                            streams.extend(tickers.iter().copied().map(|ti| kline_stream(ti, tf)));
                            streams
                        },
                        || {
                            tickers
                                .iter()
                                .copied()
                                .map(|ti| StreamKind::Trades { ticker_info: ti })
                                .collect()
                        },
                    );

                    (content, streams)
                }
                ContentKind::TimeAndSales => {
                    let config = self
                        .settings
                        .visual_config
                        .clone()
                        .and_then(|cfg| cfg.time_and_sales());
                    let content = Content::TimeAndSales(Some(Box::new(TimeAndSales::new(
                        config,
                        derived_plan.ticker_info,
                    ))));

                    let temp = PaneSetup {
                        push_freq: exchange::PushFrequency::ServerDefault,
                        ..derived_plan
                    };

                    let streams = vec![trades_stream(&temp)];

                    (content, streams)
                }
                ContentKind::Ladder => {
                    let config = self
                        .settings
                        .visual_config
                        .clone()
                        .and_then(|cfg| cfg.ladder());
                    let content = Content::Ladder(Some(Box::new(Ladder::new(
                        config,
                        derived_plan.ticker_info,
                        derived_plan.price_step,
                    ))));

                    let streams = vec![depth_stream(&derived_plan), trades_stream(&derived_plan)];

                    (content, streams)
                }
                ContentKind::ShaderHeatmap => {
                    let basis = derived_plan
                        .basis
                        .unwrap_or(Basis::default_heatmap_time(Some(derived_plan.ticker_info)));

                    let studies = if let Content::ShaderHeatmap { chart, studies } = &self.content {
                        chart
                            .as_ref()
                            .map_or(studies.clone(), |c| c.studies.clone())
                    } else {
                        vec![HeatmapStudy::VolumeProfile(
                            data::chart::heatmap::ProfileKind::default(),
                        )]
                    };

                    let config = self
                        .settings
                        .visual_config
                        .clone()
                        .and_then(|cfg| cfg.heatmap());

                    let content = Content::ShaderHeatmap {
                        chart: Some(Box::new(HeatmapShader::new(
                            basis,
                            derived_plan.price_step,
                            base_ticker,
                            studies.clone(),
                            config,
                        ))),
                        studies,
                    };

                    let streams = vec![depth_stream(&derived_plan), trades_stream(&derived_plan)];

                    (content, streams)
                }
                ContentKind::Starter => unreachable!(),
            }
        };

        self.content = content;
        self.streams = ResolvedStream::Ready(streams.clone());

        streams
    }

    pub fn insert_hist_oi(
        &mut self,
        req_id: Option<uuid::Uuid>,
        ticker_info: TickerInfo,
        oi: &[OpenInterest],
    ) {
        match &mut self.content {
            Content::Kline { chart, .. } => {
                let Some(chart) = chart else {
                    panic!("Kline chart chart wasn't initialized when inserting open interest");
                };
                chart.apply_data(DataUpdate::HistoricalOpenInterest {
                    req_id,
                    ticker_info,
                    data: oi,
                });
            }
            _ => {
                log::error!("pane content not candlestick");
            }
        }
    }

    pub fn insert_hist_klines(
        &mut self,
        req_id: Option<uuid::Uuid>,
        timeframe: Timeframe,
        ticker_info: TickerInfo,
        klines: &[Kline],
    ) {
        match &mut self.content {
            Content::Kline { chart, .. } => {
                if chart.is_none() {
                    let mut tickers = Vec::new();

                    if let Some(ready_streams) = self.streams.ready_iter() {
                        for stream in ready_streams {
                            let info = stream.ticker_info();
                            if !tickers.contains(&info) {
                                tickers.push(info);
                            }
                        }
                    }

                    if tickers.is_empty() {
                        tickers.push(ticker_info);
                    }

                    *chart = Some(Box::new(KlineChart::new_with_tickers(
                        Basis::Time(timeframe),
                        &tickers,
                    )));
                }

                let Some(chart) = chart else {
                    panic!("Kline chart chart wasn't initialized when inserting klines");
                };

                if chart.basis() != Basis::Time(timeframe) {
                    log::warn!(
                        "Ignoring stale kline fetch for timeframe {:?}; chart basis = {:?}",
                        timeframe,
                        chart.basis()
                    );
                    return;
                }

                if let Some(id) = req_id {
                    chart.apply_data(DataUpdate::HistoricalKlines {
                        req_id: Some(id),
                        ticker_info,
                        klines,
                    });
                } else {
                    chart.apply_data(DataUpdate::HistoricalKlines {
                        req_id: None,
                        ticker_info,
                        klines,
                    });
                }
            }
            _ => {
                log::error!("pane content not candlestick");
            }
        }
    }

    fn has_stream(&self) -> bool {
        match &self.streams {
            ResolvedStream::Ready(streams) => !streams.is_empty(),
            ResolvedStream::Waiting { streams, .. } => !streams.is_empty(),
            ResolvedStream::Blocked { streams, .. } => !streams.is_empty(),
        }
    }

    pub(crate) fn view<'a>(
        &'a self,
        id: pane_grid::Pane,
        panes: usize,
        is_focused: bool,
        maximized: bool,
        window: window::Id,
        main_window: &'a Window,
        timezone: UserTimezone,
        pixel_ratio: PixelRatio,
        tickers_table: &'a TickersTable,
        active_inspector: Option<crate::screen::sidebar::InspectorMode>,
        theme_accents: ThemeAccents,
    ) -> pane_grid::Content<'a, Message, Theme, Renderer> {
        let mut top_left_buttons = if Content::Starter == self.content {
            row![]
        } else {
            row![link_group_button(id, self.link_group, |id| {
                Message::PaneEvent(id, Event::ShowModal(Modal::LinkGroup))
            })]
        };

        if let Some(kind) = self.stream_pair_kind() {
            let (base_ti, extra) = match kind {
                StreamPairKind::MultiSource(list) => (list[0], list.len().saturating_sub(1)),
                StreamPairKind::SingleSource(ti) => (ti, 0),
            };

            let exchange_icon = icon_text(style::venue_icon(base_ti.ticker.exchange.venue()), 14);
            let mut label = {
                let symbol = base_ti.ticker.display_symbol_and_type().0;
                match base_ti.ticker.market_type() {
                    MarketKind::Spot => symbol,
                    MarketKind::LinearPerps | MarketKind::InversePerps => symbol + " PERP",
                }
            };
            if extra > 0 {
                label = format!("{label} +{extra}");
            }

            let content = row![
                exchange_icon.align_y(Alignment::Center).line_height(1.4),
                text(label)
                    .size(crate::style::text_size::SECTION)
                    .align_y(Alignment::Center)
                    .line_height(1.4)
            ]
            .align_y(Alignment::Center)
            .spacing(4);

            let tickers_list_btn = button(content)
                .on_press(Message::PaneEvent(
                    id,
                    Event::ShowModal(Modal::MiniTickersList(MiniPanel::new())),
                ))
                .style(|theme, status| {
                    style::button::modifier(
                        theme,
                        status,
                        !matches!(self.modal, Some(Modal::MiniTickersList(_))),
                    )
                })
                .height(widget::PANE_CONTROL_BTN_HEIGHT);

            top_left_buttons = top_left_buttons.push(tickers_list_btn);
        } else if !matches!(self.content, Content::Starter) && !self.has_stream() {
            let content = row![
                text("Choose a ticker")
                    .size(crate::style::text_size::EMPHASIS)
                    .align_y(Alignment::Center)
                    .line_height(1.4)
            ]
            .align_y(Alignment::Center);

            let tickers_list_btn = button(content)
                .on_press(Message::PaneEvent(
                    id,
                    Event::ShowModal(Modal::MiniTickersList(MiniPanel::new())),
                ))
                .style(|theme, status| {
                    style::button::modifier(
                        theme,
                        status,
                        !matches!(self.modal, Some(Modal::MiniTickersList(_))),
                    )
                })
                .height(widget::PANE_CONTROL_BTN_HEIGHT);

            top_left_buttons = top_left_buttons.push(tickers_list_btn);
        }

        let modifier: Option<modal::stream::Modifier> = self.modal.clone().and_then(|m| {
            if let Modal::StreamModifier(modifier) = m {
                Some(modifier)
            } else {
                None
            }
        });

        let compact_controls = if self.modal == Some(Modal::Controls) {
            Some(
                container(self.view_controls(
                    id,
                    panes,
                    maximized,
                    window != main_window.id,
                    active_inspector,
                ))
                .style(style::chart_modal)
                .into(),
            )
        } else {
            None
        };

        let uninitialized_base = |kind: ContentKind| -> Element<'a, Message> {
            match &self.streams {
                ResolvedStream::Waiting { streams, .. } if !streams.is_empty() => {
                    center(text("Waiting for metadata…").size(crate::style::text_size::TITLE))
                        .into()
                }
                ResolvedStream::Ready(streams) if !streams.is_empty() => center(
                    text("Waiting for pane initialization...").size(crate::style::text_size::TITLE),
                )
                .into(),
                ResolvedStream::Blocked {
                    streams, reason, ..
                } => {
                    let blocked_exchanges = streams
                        .iter()
                        .map(|s| s.exchange())
                        .collect::<std::collections::BTreeSet<_>>();

                    center(
                        column![
                            text(format!(
                                "Couldn't resolve streams for {}",
                                blocked_exchanges
                                    .iter()
                                    .map(|v| v.to_string())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ))
                            .size(crate::style::text_size::SECTION),
                            text((if reason.is_empty() { "" } else { reason }).to_string())
                                .size(crate::style::text_size::BODY),
                        ]
                        .spacing(8)
                        .align_x(Alignment::Center),
                    )
                    .into()
                }
                _ => {
                    let content = column![
                        text(kind.to_string()).size(crate::style::text_size::TITLE),
                        text("No ticker selected").size(crate::style::text_size::SECTION)
                    ]
                    .spacing(8)
                    .align_x(Alignment::Center);

                    center(content).into()
                }
            }
        };

        let body = match &self.content {
            Content::Starter => {
                let content_picklist =
                    pick_list(ContentKind::ALL, Some(ContentKind::Starter), move |kind| {
                        Message::PaneEvent(id, Event::ContentSelected(kind))
                    });

                let base: Element<_> = widget::toast::Manager::new(
                    center(
                        column![
                            text("Choose a view to get started")
                                .size(crate::style::text_size::TITLE),
                            content_picklist
                        ]
                        .align_x(Alignment::Center)
                        .spacing(12),
                    ),
                    &self.notifications,
                    Alignment::End,
                    move |msg| Message::PaneEvent(id, Event::DeleteNotification(msg)),
                )
                .into();

                self.compose_stack_view(base, id, compact_controls, None, tickers_table)
            }
            Content::TimeAndSales(panel) => {
                if let Some(panel) = panel {
                    let base = panel.view(timezone).map(move |message| {
                        Message::PaneEvent(id, Event::TimeAndSalesInteraction(message))
                    });

                    self.compose_stack_view(base, id, compact_controls, None, tickers_table)
                } else {
                    let base = uninitialized_base(ContentKind::TimeAndSales);
                    self.compose_stack_view(base, id, compact_controls, None, tickers_table)
                }
            }
            Content::Ladder(panel) => {
                if let Some(panel) = panel {
                    let basis = self
                        .settings
                        .selected_basis
                        .unwrap_or(Basis::default_heatmap_time(self.stream_pair()));
                    let tick_multiply = self.settings.tick_multiply.unwrap_or(TickMultiplier(1));

                    let stream_pair = self.stream_pair();

                    let price_step = stream_pair
                        .map(|ti| {
                            tick_multiply.unscale_step_or_min_tick(panel.step, ti.min_ticksize)
                        })
                        .unwrap_or_else(|| tick_multiply.unscale_step(panel.step));

                    let exchange = stream_pair.map(|ti| ti.ticker.exchange);
                    let min_ticksize = stream_pair.map(|ti| ti.min_ticksize);

                    let modifiers = ticksize_modifier(
                        id,
                        price_step,
                        min_ticksize,
                        tick_multiply,
                        modifier,
                        ModifierKind::Orderbook(basis, tick_multiply),
                        exchange,
                    );

                    top_left_buttons = top_left_buttons.push(modifiers);

                    let base = panel.view(timezone).map(move |message| {
                        Message::PaneEvent(id, Event::LadderInteraction(message))
                    });

                    self.compose_stack_view(base, id, compact_controls, None, tickers_table)
                } else {
                    let base = uninitialized_base(ContentKind::Ladder);
                    self.compose_stack_view(base, id, compact_controls, None, tickers_table)
                }
            }
            Content::Kline { chart, .. } => {
                if let Some(chart) = chart {
                    let selected_basis = chart.basis();
                    let tick_multiplier = chart.footprint_tick_multiplier();
                    let kind = ModifierKind::Candlestick(selected_basis, tick_multiplier);
                    let ticker_info = chart.ticker_info();

                    let mut modifiers =
                        row![basis_modifier(id, selected_basis, modifier, kind)].spacing(4);
                    if chart.trade_data_enabled() {
                        modifiers = modifiers.push(ticksize_modifier(
                            id,
                            tick_multiplier.multiply_with_min_tick_step(ticker_info),
                            Some(ticker_info.min_ticksize),
                            tick_multiplier,
                            modifier,
                            kind,
                            Some(ticker_info.exchange()),
                        ));
                    }

                    top_left_buttons = top_left_buttons.push(modifiers);

                    let base =
                        chart
                            .view(timezone, pixel_ratio, theme_accents)
                            .map(move |message| {
                                Message::PaneEvent(id, Event::KlineInteraction(message))
                            });

                    let selected_tickers =
                        chart.basis().is_time().then_some(chart.selected_tickers());

                    self.compose_stack_view(
                        base,
                        id,
                        compact_controls,
                        selected_tickers,
                        tickers_table,
                    )
                } else {
                    let base = uninitialized_base(ContentKind::CandlestickChart);
                    self.compose_stack_view(base, id, compact_controls, None, tickers_table)
                }
            }
            Content::ShaderHeatmap { chart, .. } => {
                if let Some(chart) = chart {
                    let base = HeatmapShader::view(chart, timezone).map(move |message| {
                        Message::PaneEvent(id, Event::HeatmapShaderInteraction(message))
                    });

                    let ticker_info = self.stream_pair();
                    let exchange = ticker_info.as_ref().map(|info| info.ticker.exchange);

                    let basis = self
                        .settings
                        .selected_basis
                        .unwrap_or(Basis::default_heatmap_time(ticker_info));
                    let tick_multiply = self.settings.tick_multiply.unwrap_or(TickMultiplier(5));

                    let kind = ModifierKind::Heatmap(basis, tick_multiply);

                    let price_step = ticker_info
                        .map(|ti| {
                            tick_multiply
                                .unscale_step_or_min_tick(chart.tick_size(), ti.min_ticksize)
                        })
                        .unwrap_or_else(|| tick_multiply.unscale_step(chart.tick_size()));
                    let min_ticksize = ticker_info.map(|ti| ti.min_ticksize);

                    let modifiers = row![
                        basis_modifier(id, basis, modifier, kind),
                        ticksize_modifier(
                            id,
                            price_step,
                            min_ticksize,
                            tick_multiply,
                            modifier,
                            kind,
                            exchange
                        ),
                    ]
                    .spacing(4);

                    top_left_buttons = top_left_buttons.push(modifiers);

                    self.compose_stack_view(base, id, compact_controls, None, tickers_table)
                } else {
                    let base = uninitialized_base(ContentKind::ShaderHeatmap);
                    self.compose_stack_view(base, id, compact_controls, None, tickers_table)
                }
            }
        };

        match &self.status {
            Status::Loading(InfoKind::FetchingKlines) => {
                top_left_buttons = top_left_buttons.push(text("Fetching Klines..."));
            }
            Status::Loading(InfoKind::FetchingTrades(count)) => {
                top_left_buttons =
                    top_left_buttons.push(text(format!("Fetching Trades... {count} fetched")));
            }
            Status::Loading(InfoKind::FetchingOI) => {
                top_left_buttons = top_left_buttons.push(text("Fetching Open Interest..."));
            }
            Status::Stale(msg) => {
                top_left_buttons = top_left_buttons.push(text(msg));
            }
            Status::Ready => {}
        }

        let content = pane_grid::Content::new(body)
            .style(move |theme| style::pane_background(theme, is_focused));

        let top_right_buttons = {
            let compact_control = container(
                button(
                    text("...")
                        .size(crate::style::text_size::EMPHASIS)
                        .align_y(Alignment::End),
                )
                .on_press(Message::PaneEvent(id, Event::ShowModal(Modal::Controls)))
                .style(move |theme, status| {
                    style::button::transparent(theme, status, self.modal == Some(Modal::Controls))
                }),
            )
            .align_y(Alignment::Center)
            .padding(4);

            if self.modal == Some(Modal::Controls) {
                pane_grid::Controls::new(compact_control)
            } else {
                pane_grid::Controls::dynamic(
                    self.view_controls(
                        id,
                        panes,
                        maximized,
                        window != main_window.id,
                        active_inspector,
                    ),
                    compact_control,
                )
            }
        };

        let title_bar = pane_grid::TitleBar::new(
            top_left_buttons
                .padding(padding::left(4))
                .align_y(Alignment::Center)
                .spacing(8)
                .height(Length::Fixed(32.0)),
        )
        .controls(top_right_buttons)
        .style(style::pane_title_bar);

        content.title_bar(if self.modal.is_none() && active_inspector.is_none() {
            title_bar
        } else {
            title_bar.always_show_controls()
        })
    }

    pub fn update(&mut self, msg: Event) -> Option<Effect> {
        match msg {
            Event::ShowModal(requested_modal) => {
                return self.show_modal_with_focus(requested_modal);
            }
            Event::HideModal => {
                self.modal = None;
            }
            Event::ContentSelected(kind) => {
                self.content = Content::placeholder(kind);
                self.settings.visual_config = None;

                if !matches!(kind, ContentKind::Starter) {
                    self.streams = ResolvedStream::waiting(vec![]);
                    let modal = Modal::MiniTickersList(MiniPanel::new());

                    if let Some(effect) = self.show_modal_with_focus(modal) {
                        return Some(effect);
                    }
                }
            }
            Event::OpenInspector(_) | Event::ToggleInspector(_) | Event::SyncKlineSettings => {}
            Event::KlineInteraction(message) => {
                let action = match &mut self.content {
                    Content::Kline { chart: Some(c), .. } => c.update(message),
                    _ => None,
                };
                let trade_stream_changed = self.sync_kline_trade_stream();

                if let Some(action) = action {
                    match action {
                        KlineAction::RequestFetch(fetch) => {
                            return Some(Effect::RequestFetch(fetch));
                        }
                        KlineAction::RemoveSeries(ti) => {
                            if let Content::Kline { chart: Some(c), .. } = &mut self.content
                                && let CommandResult::Streams(streams) =
                                    c.apply_command(ChartCommand::Ticker(TickerCommand::Remove(ti)))
                            {
                                self.streams = ResolvedStream::Ready(streams);
                            }

                            return Some(Effect::RefreshStreams);
                        }
                        KlineAction::OpenChartSettingsSidebar => {
                            return Some(Effect::OpenChartSettingsSidebar);
                        }
                    }
                }

                if trade_stream_changed {
                    return Some(Effect::RefreshStreams);
                }
            }
            Event::KlineIndicatorsInteraction(message) => {
                if let Content::Kline {
                    kline_indicators, ..
                } = &mut self.content
                {
                    kline_indicators.update(message);
                }
            }
            Event::Settings(message) => return self.update_settings(message),
            Event::ToggleKlineIndicator(indicator) => {
                let result = match &mut self.content {
                    Content::Kline {
                        chart: Some(chart), ..
                    } => chart
                        .apply_command(ChartCommand::Indicator(IndicatorCommand::Add(indicator))),
                    _ => CommandResult::None,
                };
                let trade_stream_changed = self.sync_kline_trade_stream();
                if let CommandResult::Action(Some(super::ChartAction::RequestFetch(fetch))) = result
                {
                    return Some(Effect::RequestFetch(fetch));
                }
                if trade_stream_changed {
                    return Some(Effect::RefreshStreams);
                }
            }
            Event::ToggleKlineStudy(study) => {
                let result = match &mut self.content {
                    Content::Kline {
                        chart: Some(chart), ..
                    } => chart.apply_command(ChartCommand::Study(StudyCommand::Toggle(study))),
                    _ => CommandResult::None,
                };
                let trade_stream_changed = self.sync_kline_trade_stream();
                if let CommandResult::Action(Some(super::ChartAction::RequestFetch(fetch))) = result
                {
                    return Some(Effect::RequestFetch(fetch));
                }
                if trade_stream_changed {
                    return Some(Effect::RefreshStreams);
                }
            }
            Event::LadderInteraction(message) => {
                if let Content::Ladder(Some(panel)) = &mut self.content {
                    panel.update(message);
                }
            }
            Event::TimeAndSalesInteraction(message) => {
                if let Content::TimeAndSales(Some(panel)) = &mut self.content {
                    panel.update(message);
                }
            }
            Event::DeleteNotification(idx) => {
                if idx < self.notifications.len() {
                    self.notifications.remove(idx);
                }
            }
            Event::StreamModifierChanged(message) => {
                if let Some(Modal::StreamModifier(mut modifier)) = self.modal.take() {
                    let mut effect: Option<Effect> = None;

                    if let Some(action) = modifier.update(message) {
                        match action {
                            modal::stream::Action::TabSelected(tab) => {
                                modifier.tab = tab;
                            }
                            modal::stream::Action::TicksizeSelected(tm) => {
                                modifier.update_kind_with_multiplier(tm);
                                self.settings.tick_multiply = Some(tm);

                                if let Some(ticker) = self.stream_pair() {
                                    match &mut self.content {
                                        Content::Ladder(Some(p)) => {
                                            p.set_tick_size(tm.multiply_with_min_tick_step(ticker));
                                        }
                                        Content::ShaderHeatmap {
                                            chart: Some(c),
                                            studies,
                                        } => {
                                            let saved_config = c.config;
                                            **c = HeatmapShader::new(
                                                c.basis,
                                                tm.multiply_with_min_tick_step(ticker),
                                                c.ticker_info,
                                                studies.clone(),
                                                Some(saved_config),
                                            );
                                        }
                                        Content::Kline { chart: Some(c), .. } => {
                                            let _ = c.apply_command(ChartCommand::Footprint(
                                                FootprintCommand::SetTickMultiplier(tm),
                                            ));
                                        }
                                        _ => {}
                                    }
                                }

                                let is_client = self
                                    .stream_pair()
                                    .map(|ti| ti.exchange().is_depth_client_aggr())
                                    .unwrap_or(false);

                                if let Some(mut it) = self.streams.ready_iter_mut() {
                                    for s in &mut it {
                                        if let StreamKind::Depth { depth_aggr, .. } = s {
                                            *depth_aggr = if is_client {
                                                StreamTicksize::Client
                                            } else {
                                                StreamTicksize::ServerSide(tm)
                                            };
                                        }
                                    }
                                }
                                if !is_client {
                                    effect = Some(Effect::RefreshStreams);
                                }
                            }
                            modal::stream::Action::BasisSelected(new_basis) => {
                                modifier.update_kind_with_basis(new_basis);
                                self.settings.selected_basis = Some(new_basis);

                                match &mut self.content {
                                    Content::ShaderHeatmap { chart: Some(c), .. } => {
                                        let saved_config = c.config;
                                        let saved_studies = c.studies.clone();
                                        **c = HeatmapShader::new(
                                            new_basis,
                                            c.tick_size(),
                                            c.ticker_info,
                                            saved_studies,
                                            Some(saved_config),
                                        );

                                        if let Some(stream_type) =
                                            self.streams.ready_iter_mut().and_then(|mut it| {
                                                it.find(|s| matches!(s, StreamKind::Depth { .. }))
                                            })
                                            && let StreamKind::Depth {
                                                push_freq,
                                                ticker_info,
                                                ..
                                            } = stream_type
                                            && ticker_info.exchange().is_custom_push_freq()
                                        {
                                            match new_basis {
                                                Basis::Time(tf) => {
                                                    *push_freq = exchange::PushFrequency::Custom(tf)
                                                }
                                                Basis::Tick(_) => {
                                                    *push_freq =
                                                        exchange::PushFrequency::ServerDefault
                                                }
                                            }
                                        }

                                        effect = Some(Effect::RefreshStreams);
                                    }
                                    Content::Kline { chart: Some(c), .. } => {
                                        let action = match c
                                            .apply_command(ChartCommand::SetBasis(new_basis))
                                        {
                                            CommandResult::Action(action) => action,
                                            _ => None,
                                        };

                                        let streams = match new_basis {
                                            Basis::Time(tf) => {
                                                let mut streams = Vec::new();
                                                if c.trade_data_required() {
                                                    streams.push(StreamKind::Trades {
                                                        ticker_info: c.ticker_info(),
                                                    });
                                                }
                                                streams.extend(
                                                    c.selected_tickers().iter().copied().map(
                                                        |ti| StreamKind::Kline {
                                                            ticker_info: ti,
                                                            timeframe: tf,
                                                        },
                                                    ),
                                                );
                                                streams
                                            }
                                            Basis::Tick(_) => {
                                                let base_ticker = c.ticker_info();
                                                let secondary_tickers: Vec<TickerInfo> = c
                                                    .selected_tickers()
                                                    .iter()
                                                    .copied()
                                                    .filter(|ticker| *ticker != base_ticker)
                                                    .collect();

                                                for ticker in secondary_tickers {
                                                    let _ = c.apply_command(ChartCommand::Ticker(
                                                        TickerCommand::Remove(ticker),
                                                    ));
                                                }

                                                vec![StreamKind::Trades {
                                                    ticker_info: base_ticker,
                                                }]
                                            }
                                        };
                                        self.streams = ResolvedStream::Ready(streams);

                                        if let Some(super::ChartAction::RequestFetch(fetch)) =
                                            action
                                        {
                                            effect = Some(Effect::RequestFetch(fetch));
                                        } else {
                                            self.status = Status::Ready;
                                            effect = Some(Effect::RefreshStreams);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }

                    self.modal = Some(Modal::StreamModifier(modifier));

                    if let Some(e) = effect {
                        return Some(e);
                    }
                }
            }
            Event::HeatmapShaderInteraction(message) => {
                if let Content::ShaderHeatmap { chart: Some(c), .. } = &mut self.content {
                    c.update(message);
                }
            }
            Event::MiniTickersListInteraction(message) => {
                if let Some(Modal::MiniTickersList(ref mut mini_panel)) = self.modal
                    && let Some(action) = mini_panel.update(message)
                {
                    self.modal = Some(Modal::MiniTickersList(mini_panel.clone()));

                    let crate::modal::pane::mini_tickers_list::Action::RowSelected(sel) = action;
                    match sel {
                        crate::modal::pane::mini_tickers_list::RowSelection::Add(ti) => {
                            if let Content::Kline { chart: Some(c), .. } = &mut self.content {
                                if matches!(c.basis(), Basis::Tick(_)) {
                                    return None;
                                }

                                let CommandResult::Streams(streams) =
                                    c.apply_command(ChartCommand::Ticker(TickerCommand::Add(ti)))
                                else {
                                    return None;
                                };
                                self.streams = ResolvedStream::Ready(streams);
                                return Some(Effect::RefreshStreams);
                            }
                        }
                        crate::modal::pane::mini_tickers_list::RowSelection::Remove(ti) => {
                            if let Content::Kline { chart: Some(c), .. } = &mut self.content {
                                let CommandResult::Streams(streams) = c
                                    .apply_command(ChartCommand::Ticker(TickerCommand::Remove(ti)))
                                else {
                                    return None;
                                };
                                self.streams = ResolvedStream::Ready(streams);
                                return Some(Effect::RefreshStreams);
                            }
                        }
                        crate::modal::pane::mini_tickers_list::RowSelection::Switch(ti) => {
                            return Some(Effect::SwitchTickersInGroup(ti));
                        }
                    }
                }
            }
        }
        None
    }

    fn update_settings(&mut self, message: settings::Message) -> Option<Effect> {
        match message {
            settings::Message::Study(study_msg) => match study_msg {
                crate::widget::study_configurator::StudyMessage::Heatmap(m) => {
                    if let Content::ShaderHeatmap { chart, studies } = &mut self.content
                        && let Some(c) = chart
                    {
                        c.update_study_configurator(m);
                        *studies = c.studies.clone();
                    }
                }
            },
            settings::Message::VisualConfigChanged { .. } => {}
        }

        None
    }

    fn settings_inspector_mode(&self) -> Option<crate::screen::sidebar::InspectorMode> {
        match &self.content {
            Content::Kline { chart: Some(_), .. }
            | Content::ShaderHeatmap { chart: Some(_), .. }
            | Content::TimeAndSales(Some(_))
            | Content::Ladder(Some(_)) => Some(crate::screen::sidebar::InspectorMode::PaneSettings),
            Content::Starter
            | Content::Kline { chart: None, .. }
            | Content::ShaderHeatmap { chart: None, .. }
            | Content::TimeAndSales(None)
            | Content::Ladder(None) => None,
        }
    }

    pub(crate) fn supports_inspector(&self, mode: crate::screen::sidebar::InspectorMode) -> bool {
        match &self.content {
            Content::Kline {
                chart: Some(chart), ..
            } => {
                matches!(mode, crate::screen::sidebar::InspectorMode::PaneSettings)
                    || (mode == crate::screen::sidebar::InspectorMode::ChartSettings
                        && chart.has_contextual_settings())
                    || mode == crate::screen::sidebar::InspectorMode::Indicators
            }
            Content::ShaderHeatmap { chart: Some(_), .. } => {
                mode == crate::screen::sidebar::InspectorMode::PaneSettings
            }
            Content::TimeAndSales(Some(_)) | Content::Ladder(Some(_)) => {
                mode == crate::screen::sidebar::InspectorMode::PaneSettings
            }
            Content::Starter
            | Content::Kline { chart: None, .. }
            | Content::ShaderHeatmap { chart: None, .. }
            | Content::TimeAndSales(None)
            | Content::Ladder(None) => false,
        }
    }

    pub(crate) fn inspector_mode_after_focus(
        &self,
        current: crate::screen::sidebar::InspectorMode,
    ) -> Option<crate::screen::sidebar::InspectorMode> {
        if self.supports_inspector(current) {
            return Some(current);
        }

        (current == crate::screen::sidebar::InspectorMode::ChartSettings)
            .then_some(crate::screen::sidebar::InspectorMode::PaneSettings)
            .filter(|mode| self.supports_inspector(*mode))
    }

    fn view_controls(
        &'_ self,
        pane: pane_grid::Pane,
        total_panes: usize,
        is_maximized: bool,
        is_popout: bool,
        active_inspector: Option<crate::screen::sidebar::InspectorMode>,
    ) -> Element<'_, Message> {
        let control_btn_style = |is_active: bool| {
            move |theme: &Theme, status: button::Status| {
                style::button::transparent(theme, status, is_active)
            }
        };

        let treat_as_starter =
            matches!(&self.content, Content::Starter) || !self.content.initialized();

        let tooltip_pos = tooltip::Position::Bottom;
        let mut buttons = row![];

        if let Some(settings_mode) = self.settings_inspector_mode() {
            buttons = buttons.push(press_button_with_tooltip(
                icon_text(Icon::Cog, 12),
                Message::PaneEvent(
                    pane,
                    inspector_button_event(settings_mode, active_inspector),
                ),
                None,
                tooltip_pos,
                control_btn_style(active_inspector == Some(settings_mode)),
            ));
        }
        if !treat_as_starter && matches!(&self.content, Content::Kline { .. }) {
            buttons = buttons.push(press_button_with_tooltip(
                icon_text(Icon::ChartOutline, 12),
                Message::PaneEvent(
                    pane,
                    inspector_button_event(
                        crate::screen::sidebar::InspectorMode::Indicators,
                        active_inspector,
                    ),
                ),
                Some("Indicators"),
                tooltip_pos,
                control_btn_style(
                    active_inspector == Some(crate::screen::sidebar::InspectorMode::Indicators),
                ),
            ));
        }
        if let Content::Kline {
            chart: Some(chart), ..
        } = &self.content
        {
            buttons = buttons.push(press_button_with_tooltip(
                icon_text(Icon::Edit, 12),
                Message::PaneEvent(
                    pane,
                    Event::KlineInteraction(content::kline::Message::ToggleDrawingSidebar),
                ),
                Some("Drawing tools"),
                tooltip_pos,
                control_btn_style(chart.drawing_sidebar_visible()),
            ));
        }

        if window::MULTI_WINDOW_SUPPORTED {
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
            .padding(padding::right(4).left(4))
            .spacing(2)
            .align_y(Alignment::Center)
            .height(Length::Fixed(32.0))
            .into()
    }

    fn compose_stack_view<'a>(
        &'a self,
        base: Element<'a, Message>,
        pane: pane_grid::Pane,
        compact_controls: Option<Element<'a, Message>>,
        selected_tickers: Option<&'a [TickerInfo]>,
        tickers_table: &'a TickersTable,
    ) -> Element<'a, Message> {
        let base =
            widget::toast::Manager::new(base, &self.notifications, Alignment::End, move |msg| {
                Message::PaneEvent(pane, Event::DeleteNotification(msg))
            })
            .into();

        let on_blur = Message::PaneEvent(pane, Event::HideModal);

        match &self.modal {
            Some(Modal::LinkGroup) => {
                let content = link_group_modal(pane, self.link_group);

                stack_modal(
                    base,
                    content,
                    on_blur,
                    padding::right(12).left(4),
                    Alignment::Start,
                )
            }
            Some(Modal::StreamModifier(modifier)) => stack_modal(
                base,
                modifier.view(self.stream_pair_kind()).map(move |message| {
                    Message::PaneEvent(pane, Event::StreamModifierChanged(message))
                }),
                Message::PaneEvent(pane, Event::HideModal),
                padding::right(12).left(48),
                Alignment::Start,
            ),
            Some(Modal::MiniTickersList(panel)) => {
                let mini_list = panel
                    .view(tickers_table, selected_tickers, self.stream_pair())
                    .map(move |msg| {
                        Message::PaneEvent(pane, Event::MiniTickersListInteraction(msg))
                    });

                let content: Element<_> = container(mini_list)
                    .max_width(260)
                    .padding(16)
                    .style(style::chart_modal)
                    .into();

                stack_modal(
                    base,
                    content,
                    Message::PaneEvent(pane, Event::HideModal),
                    padding::left(12),
                    Alignment::Start,
                )
            }
            Some(Modal::Controls) => stack_modal(
                base,
                if let Some(controls) = compact_controls {
                    controls
                } else {
                    column![].into()
                },
                on_blur,
                padding::left(12),
                Alignment::End,
            ),
            None => base,
        }
    }

    pub fn matches_stream(&self, stream: &StreamKind) -> bool {
        self.streams.matches_stream(stream)
    }

    pub(crate) fn dismiss_settings_sidebar_state(&mut self) {
        if let Content::Kline {
            chart: Some(chart), ..
        } = &mut self.content
        {
            chart.dismiss_settings_reset_confirmation();
        }
    }

    pub(crate) fn indicators_sidebar_focus_id(&self) -> Option<iced::widget::Id> {
        match &self.content {
            Content::Kline {
                kline_indicators,
                chart: Some(_),
                ..
            } => Some(kline_indicators.search_box_id()),
            _ => None,
        }
    }

    pub(crate) fn pane_settings_content<'a>(
        &'a self,
        pane: pane_grid::Pane,
        theme_accents: ThemeAccents,
    ) -> Option<InspectorContent<'a, Message>> {
        self.content.pane_settings_content(
            pane,
            self.stream_pair(),
            self.settings.selected_basis,
            theme_accents,
        )
    }

    pub(crate) fn chart_settings_content<'a>(
        &'a self,
        pane: pane_grid::Pane,
        theme_accents: ThemeAccents,
    ) -> Option<InspectorContent<'a, Message>> {
        self.content.chart_settings_content(pane, theme_accents)
    }

    pub(crate) fn indicators_sidebar_view<'a>(
        &'a self,
        pane: pane_grid::Pane,
    ) -> Option<Element<'a, Message>> {
        match &self.content {
            Content::Kline {
                chart: Some(chart),
                kline_indicators,
                ..
            } => {
                let active_indicators = chart
                    .selected_indicators()
                    .into_iter()
                    .filter_map(|id| chart.indicator_kind(id))
                    .collect::<Vec<_>>();

                Some(kline_indicators::view(
                    pane,
                    &self.content,
                    kline_indicators,
                    &active_indicators,
                    Some(chart.ticker_info().ticker.market_type()),
                    chart.studies(),
                    chart.basis(),
                ))
            }
            _ => None,
        }
    }

    fn show_modal_with_focus(&mut self, requested_modal: Modal) -> Option<Effect> {
        let should_toggle_close = match (&self.modal, &requested_modal) {
            (Some(Modal::StreamModifier(open)), Modal::StreamModifier(req)) => {
                open.view_mode == req.view_mode
            }
            (Some(open), req) => core::mem::discriminant(open) == core::mem::discriminant(req),
            _ => false,
        };

        if should_toggle_close {
            self.modal = None;
            return None;
        }

        let focus_widget_id = match &requested_modal {
            Modal::MiniTickersList(m) => Some(m.search_box_id.clone()),
            _ => None,
        };

        self.modal = Some(requested_modal);
        focus_widget_id.map(Effect::FocusWidget)
    }

    pub fn invalidate(&mut self, now: Instant) -> Option<Action> {
        match &mut self.content {
            Content::Starter => None,
            Content::Kline { chart, .. } => chart
                .as_mut()
                .and_then(|c| c.invalidate(Some(now)).map(Action::Chart)),
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
            Content::ShaderHeatmap { chart, .. } => chart
                .as_mut()
                .and_then(|c| c.invalidate(Some(now)).map(Action::Chart)),
        }
    }

    pub fn park_for_inactive_layout(&mut self) {
        if let Content::ShaderHeatmap { chart, .. } = &mut self.content {
            *chart = None;
            self.status = Status::Ready;
        }
    }

    pub fn update_interval(&self) -> Option<u64> {
        match &self.content {
            Content::Kline { .. } => Some(1000),
            Content::Ladder(_) | Content::TimeAndSales(_) => Some(100),
            Content::ShaderHeatmap { .. } => None,
            Content::Starter => None,
        }
    }

    pub fn last_tick(&self) -> Option<Instant> {
        self.content.last_tick()
    }

    pub fn tick(&mut self, now: Instant) -> Option<Action> {
        let invalidate_interval: Option<u64> = self.update_interval();
        let last_tick: Option<Instant> = self.last_tick();

        if let Some(streams) = self.streams.due_streams_to_resolve(now) {
            return Some(Action::ResolveStreams(streams));
        }

        if !self.content.initialized() {
            return Some(Action::ResolveContent);
        }

        if let DueTick::Handled(action) = self.content.take_due_tick(now) {
            return action;
        }

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
            (None, _) => {
                return self.invalidate(now);
            }
        }

        None
    }

    pub fn unique_id(&self) -> uuid::Uuid {
        self.id
    }

    pub fn apply_settings_snapshot(
        &mut self,
        snapshot: &data::chart::kline::SettingsSnapshot,
    ) -> Option<Effect> {
        let action = self.content.apply_settings_snapshot(snapshot);
        let trade_stream_changed = self.sync_kline_trade_stream();

        if let Some(KlineAction::RequestFetch(fetch)) = action {
            return Some(Effect::RequestFetch(fetch));
        }
        trade_stream_changed.then_some(Effect::RefreshStreams)
    }

    pub fn apply_synced_settings(&mut self, studies: &Option<data::chart::Study>) {
        if let Some(studies) = studies {
            self.content.update_studies(studies.clone());
        }
    }
}

impl Default for State {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            modal: None,
            content: Content::Starter,
            settings: Settings::default(),
            streams: ResolvedStream::waiting(vec![]),
            notifications: vec![],
            status: Status::Ready,
            link_group: None,
        }
    }
}

fn link_group_modal<'a>(
    pane: pane_grid::Pane,
    selected_group: Option<LinkGroup>,
) -> Element<'a, Message> {
    let mut grid = column![].spacing(4);
    let rows = LinkGroup::ALL.chunks(3);

    for row_groups in rows {
        let mut button_row = row![].spacing(4);

        for &group in row_groups {
            let is_selected = selected_group == Some(group);
            let btn_content = text(group.to_string()).font(style::AZERET_MONO);

            let btn = if is_selected {
                button_with_tooltip(
                    btn_content.align_x(iced::Alignment::Center),
                    Message::SwitchLinkGroup(pane, None),
                    Some("Unlink"),
                    tooltip::Position::Bottom,
                    move |theme, status| style::button::menu_body(theme, status, true),
                )
            } else {
                button(btn_content.align_x(iced::Alignment::Center))
                    .on_press(Message::SwitchLinkGroup(pane, Some(group)))
                    .style(move |theme, status| style::button::menu_body(theme, status, false))
                    .into()
            };

            button_row = button_row.push(btn);
        }

        grid = grid.push(button_row);
    }

    container(grid)
        .max_width(240)
        .padding(16)
        .style(style::chart_modal)
        .into()
}

fn ticksize_modifier<'a>(
    id: pane_grid::Pane,
    price_step: PriceStep,
    min_ticksize: Option<exchange::unit::MinTicksize>,
    multiplier: TickMultiplier,
    modifier: Option<modal::stream::Modifier>,
    kind: ModifierKind,
    exchange: Option<exchange::adapter::Exchange>,
) -> Element<'a, Message> {
    let modifier_modal =
        Modal::StreamModifier(modal::stream::Modifier::new(kind).with_ticksize_view(
            price_step,
            min_ticksize,
            multiplier,
            exchange,
        ));

    let is_active = modifier.is_some_and(|m| {
        matches!(
            m.view_mode,
            modal::stream::ViewMode::TicksizeSelection { .. }
        )
    });

    button(text(multiplier.to_string()).align_y(Alignment::Center))
        .style(move |theme, status| style::button::modifier(theme, status, !is_active))
        .on_press(Message::PaneEvent(id, Event::ShowModal(modifier_modal)))
        .height(widget::PANE_CONTROL_BTN_HEIGHT)
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

    button(text(selected_basis.to_string()).align_y(Alignment::Center))
        .style(move |theme, status| style::button::modifier(theme, status, !is_active))
        .on_press(Message::PaneEvent(id, Event::ShowModal(modifier_modal)))
        .height(widget::PANE_CONTROL_BTN_HEIGHT)
        .into()
}

fn by_basis_default<T>(
    basis: Option<Basis>,
    default_tf: Timeframe,
    on_time: impl FnOnce(Timeframe) -> T,
    on_tick: impl FnOnce() -> T,
) -> T {
    match basis.unwrap_or(Basis::Time(default_tf)) {
        Basis::Time(tf) => on_time(tf),
        Basis::Tick(_) => on_tick(),
    }
}
