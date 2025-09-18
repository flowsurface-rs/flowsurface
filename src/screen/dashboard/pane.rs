use crate::{
    chart::{self, heatmap::HeatmapChart, kline::KlineChart},
    modal::{
        self, ModifierKind,
        pane::{
            settings::{heatmap_cfg_view, kline_cfg_view},
            stack_modal,
        },
    },
    screen::{
        DashboardError,
        dashboard::panel::{self, timeandsales::TimeAndSales},
    },
    style::{self, Icon, icon_text},
    widget::{self, button_with_tooltip, column_drag, link_group_button, toast::Toast},
    window::{self, Window},
};
use data::{
    UserTimezone,
    chart::{
        Basis, ViewConfig, VisualConfig,
        indicator::{HeatmapIndicator, Indicator, KlineIndicator, UiIndicator},
    },
    layout::pane::{LinkGroup, Settings},
};
use exchange::{
    Kline, OpenInterest, TickMultiplier, TickerInfo, Timeframe,
    adapter::{MarketKind, PersistStreamKind, ResolvedStream, StreamKind, StreamTicksize},
};
use iced::{
    Alignment, Element, Length, Renderer, Theme,
    alignment::Vertical,
    padding,
    widget::{button, center, column, container, pane_grid, row, text, tooltip},
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
    LinkGroup,
    Controls,
}

pub enum Action {
    Chart(chart::Action),
    Panel(panel::Action),
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
    ShowModal(pane_grid::Pane, Modal),
    HideModal(pane_grid::Pane),
    ReplacePane(pane_grid::Pane),
    ChartInteraction(pane_grid::Pane, chart::Message),
    PanelInteraction(pane_grid::Pane, panel::Message),
    VisualConfigChanged(pane_grid::Pane, VisualConfig, bool),
    ToggleIndicator(pane_grid::Pane, UiIndicator),
    Popout,
    Merge,
    DeleteNotification(pane_grid::Pane, usize),
    ReorderIndicator(pane_grid::Pane, column_drag::DragEvent),
    ClusterKindSelected(pane_grid::Pane, data::chart::kline::ClusterKind),
    ClusterScalingSelected(pane_grid::Pane, data::chart::kline::ClusterScaling),
    StreamModifierChanged(pane_grid::Pane, modal::stream::Message),
    StudyConfigurator(pane_grid::Pane, modal::pane::settings::study::StudyMessage),
    SwitchLinkGroup(pane_grid::Pane, Option<LinkGroup>),
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
            streams: ResolvedStream::Waiting(streams),
            link_group,
            ..Default::default()
        }
    }

    pub fn stream_pair(&self) -> Option<TickerInfo> {
        self.streams.find_ready_map(|stream| match stream {
            StreamKind::DepthAndTrades { ticker_info, .. }
            | StreamKind::Kline { ticker_info, .. } => Some(*ticker_info),
        })
    }

    pub fn set_content_and_streams(
        &mut self,
        ticker_info: TickerInfo,
        content_str: &str,
    ) -> Result<Vec<StreamKind>, DashboardError> {
        if (matches!(&self.content, Content::Heatmap { .. }) && content_str != "heatmap")
            || (matches!(&self.content, Content::Kline { .. }) && content_str == "heatmap")
        {
            self.settings.selected_basis = None;
        }
        let ticker = ticker_info.ticker;

        let result = match content_str {
            "heatmap" => {
                let exchange = ticker.exchange;
                let is_depth_client_aggr = exchange.is_depth_client_aggr();

                let tick_multiplier = if let Some(tm) = self.settings.tick_multiply {
                    tm
                } else if is_depth_client_aggr {
                    TickMultiplier(5)
                } else {
                    TickMultiplier(10)
                };
                self.settings.tick_multiply = Some(tick_multiplier);
                let tick_size = tick_multiplier.multiply_with_min_tick_size(ticker_info);

                let content =
                    Content::new_heatmap(&self.content, ticker_info, &self.settings, tick_size);
                let streams = vec![StreamKind::DepthAndTrades {
                    ticker_info,
                    depth_aggr: if is_depth_client_aggr {
                        StreamTicksize::Client
                    } else {
                        StreamTicksize::ServerSide(TickMultiplier(10))
                    },
                }];
                Ok((content, streams))
            }
            "footprint" => {
                let tick_multiplier = if let Some(tm) = self.settings.tick_multiply {
                    tm
                } else {
                    TickMultiplier(50)
                };
                self.settings.tick_multiply = Some(tick_multiplier);
                let tick_size = tick_multiplier.multiply_with_min_tick_size(ticker_info);

                let content = Content::new_kline(
                    content_str,
                    &self.content,
                    ticker_info,
                    &self.settings,
                    tick_size,
                );

                let basis = self.settings.selected_basis.unwrap_or(Timeframe::M5.into());
                let streams = match basis {
                    Basis::Time(timeframe) => vec![
                        StreamKind::DepthAndTrades {
                            ticker_info,
                            depth_aggr: if ticker.exchange.is_depth_client_aggr() {
                                StreamTicksize::Client
                            } else {
                                StreamTicksize::ServerSide(TickMultiplier(50))
                            },
                        },
                        StreamKind::Kline {
                            ticker_info,
                            timeframe,
                        },
                    ],
                    Basis::Tick(_) => {
                        vec![StreamKind::DepthAndTrades {
                            ticker_info,
                            depth_aggr: if ticker.exchange.is_depth_client_aggr() {
                                StreamTicksize::Client
                            } else {
                                StreamTicksize::ServerSide(TickMultiplier(50))
                            },
                        }]
                    }
                };
                Ok((content, streams))
            }
            "candlestick" => {
                self.settings.tick_multiply = None;
                let tick_size = ticker_info.min_ticksize;

                let content = Content::new_kline(
                    content_str,
                    &self.content,
                    ticker_info,
                    &self.settings,
                    tick_size.into(),
                );

                let basis = self
                    .settings
                    .selected_basis
                    .unwrap_or(Timeframe::M15.into());
                let streams = match basis {
                    Basis::Time(timeframe) => {
                        vec![StreamKind::Kline {
                            ticker_info,
                            timeframe,
                        }]
                    }
                    Basis::Tick(_) => {
                        vec![StreamKind::DepthAndTrades {
                            ticker_info,
                            depth_aggr: if ticker.exchange.is_depth_client_aggr() {
                                StreamTicksize::Client
                            } else {
                                StreamTicksize::ServerSide(TickMultiplier(50))
                            },
                        }]
                    }
                };
                Ok((content, streams))
            }
            "time&sales" => {
                let config = self
                    .settings
                    .visual_config
                    .and_then(|cfg| cfg.time_and_sales());
                let content =
                    Content::TimeAndSales(Some(TimeAndSales::new(config, Some(ticker_info))));
                let streams = vec![StreamKind::DepthAndTrades {
                    ticker_info,
                    depth_aggr: if ticker.exchange.is_depth_client_aggr() {
                        StreamTicksize::Client
                    } else {
                        StreamTicksize::ServerSide(TickMultiplier(50))
                    },
                }];
                Ok((content, streams))
            }
            _ => Err(DashboardError::PaneSet(
                "A content must be set first.".to_string(),
            )),
        };

        match result {
            Ok((content, streams)) => {
                self.content = content;
                self.streams.rebuild_ready_from(&streams);
                Ok(streams)
            }
            Err(e) => Err(e),
        }
    }

    pub fn insert_oi_vec(&mut self, req_id: Option<uuid::Uuid>, oi: &[OpenInterest]) {
        match &mut self.content {
            Content::Kline { chart, .. } => {
                let Some(chart) = chart else {
                    panic!("Kline chart wasn't initialized when inserting open interest");
                };
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
        let ticker_info = self.stream_pair();

        match &mut self.content {
            Content::Kline {
                chart, indicators, ..
            } => {
                let Some(chart) = chart else {
                    panic!("chart wasn't initialized when inserting klines");
                };

                if let Some(id) = req_id {
                    chart.insert_new_klines(id, klines);
                } else {
                    let (raw_trades, tick_size) = (chart.raw_trades(), chart.tick_size());
                    let layout = chart.chart_layout();

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
        let mut stream_info_element = if Content::Starter == self.content {
            row![]
        } else {
            row![link_group_button(id, self.link_group, |id| {
                Message::ShowModal(id, Modal::LinkGroup)
            })]
        };

        if let Some(info) = self.stream_pair() {
            let ticker = info.ticker;
            let exchange_icon = icon_text(style::exchange_icon(ticker.exchange), 14);

            let ticker_str = {
                let symbol = ticker.display_symbol_and_type().0;
                match ticker.market_type() {
                    MarketKind::Spot => symbol,
                    MarketKind::LinearPerps | MarketKind::InversePerps => symbol + " PERP",
                }
            };

            stream_info_element = stream_info_element.push(
                row![exchange_icon, text(ticker_str).size(14),]
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

        let compact_controls = if self.modal == Some(Modal::Controls) {
            Some(
                container(self.view_controls(id, panes, maximized, window != main_window.id))
                    .style(style::chart_modal)
                    .into(),
            )
        } else {
            None
        };

        let body = match &self.content {
            Content::Starter => {
                let base: Element<_> = widget::toast::Manager::new(
                    center(text("select a ticker to start").size(16)),
                    &self.notifications,
                    Alignment::End,
                    move |msg| Message::DeleteNotification(id, msg),
                )
                .into();

                if let Some(Modal::LinkGroup) = self.modal {
                    link_group_modal(base, id, self.link_group)
                } else if self.modal == Some(Modal::Controls) {
                    stack_modal(
                        base,
                        container(self.view_controls(
                            id,
                            panes,
                            maximized,
                            window != main_window.id,
                        ))
                        .style(style::chart_modal),
                        Message::HideModal(id),
                        padding::left(12),
                        Alignment::End,
                    )
                } else {
                    base
                }
            }
            Content::TimeAndSales(panel) => {
                if let Some(panel) = panel {
                    let base = panel::view(panel, timezone)
                        .map(move |message| Message::PanelInteraction(id, message));

                    let settings_modal =
                        || modal::pane::settings::timesales_cfg_view(panel.config, id);

                    self.compose_panel_view(base, id, compact_controls, settings_modal)
                } else {
                    center(text("Loading...").size(16)).into()
                }
            }
            Content::Heatmap {
                chart, indicators, ..
            } => {
                if let Some(chart) = chart {
                    let ticker_info = self.stream_pair();
                    let exchange = ticker_info.as_ref().map(|info| info.ticker.exchange);

                    let selected_basis = self
                        .settings
                        .selected_basis
                        .unwrap_or(Basis::default_heatmap_time(ticker_info));
                    let tick_multiply = self.settings.tick_multiply.unwrap_or(TickMultiplier(5));
                    let kind = ModifierKind::Heatmap(selected_basis, tick_multiply);

                    let base_ticksize = tick_multiply.base(chart.tick_size());

                    let modifiers = row![
                        basis_modifier(id, selected_basis, modifier, kind),
                        ticksize_modifier(
                            id,
                            base_ticksize,
                            tick_multiply,
                            modifier,
                            kind,
                            exchange
                        ),
                    ]
                    .spacing(4);

                    stream_info_element = stream_info_element.push(modifiers);

                    let base = chart::view(chart, indicators, timezone)
                        .map(move |message| Message::ChartInteraction(id, message));
                    let settings_modal = || {
                        heatmap_cfg_view(
                            chart.visual_config(),
                            id,
                            chart.study_configurator(),
                            &chart.studies,
                            selected_basis,
                        )
                    };

                    self.compose_chart_view(base, id, indicators, compact_controls, settings_modal)
                } else {
                    center(text("Loading...").size(16)).into()
                }
            }
            Content::Kline {
                chart,
                indicators,
                kind: chart_kind,
                ..
            } => {
                if let Some(chart) = chart {
                    match chart_kind {
                        data::chart::KlineChartKind::Footprint { .. } => {
                            let selected_basis =
                                self.settings.selected_basis.unwrap_or(Timeframe::M5.into());
                            let tick_multiply =
                                self.settings.tick_multiply.unwrap_or(TickMultiplier(10));
                            let kind = ModifierKind::Footprint(selected_basis, tick_multiply);

                            let base_ticksize = tick_multiply.base(chart.tick_size());

                            let exchange =
                                self.stream_pair().as_ref().map(|info| info.ticker.exchange);

                            let modifiers = row![
                                basis_modifier(id, selected_basis, modifier, kind),
                                ticksize_modifier(
                                    id,
                                    base_ticksize,
                                    tick_multiply,
                                    modifier,
                                    kind,
                                    exchange
                                ),
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
                                row![basis_modifier(id, selected_basis, modifier, kind),]
                                    .spacing(4);

                            stream_info_element = stream_info_element.push(modifiers);
                        }
                    }

                    let base = chart::view(chart, indicators, timezone)
                        .map(move |message| Message::ChartInteraction(id, message));
                    let settings_modal = || {
                        kline_cfg_view(
                            chart.study_configurator(),
                            data::chart::kline::Config {},
                            chart_kind,
                            id,
                            chart.basis(),
                        )
                    };

                    self.compose_chart_view(base, id, indicators, compact_controls, settings_modal)
                } else {
                    center(text("Loading...").size(16)).into()
                }
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

        let controls = {
            let compact_control = container(
                button(text("...").size(13).align_y(Alignment::End))
                    .on_press(Message::ShowModal(id, Modal::Controls))
                    .style(move |theme, status| {
                        style::button::transparent(theme, status, self.modal.is_some())
                    }),
            )
            .align_y(Alignment::Center)
            .height(Length::Fixed(32.0))
            .padding(4);

            if self.modal == Some(Modal::Controls) {
                pane_grid::Controls::new(compact_control)
            } else {
                pane_grid::Controls::dynamic(
                    self.view_controls(id, panes, maximized, window != main_window.id),
                    compact_control,
                )
            }
        };

        let title_bar = pane_grid::TitleBar::new(
            stream_info_element
                .padding(padding::left(4).top(1))
                .align_y(Vertical::Center)
                .spacing(8)
                .height(Length::Fixed(32.0)),
        )
        .controls(controls)
        .style(style::pane_title_bar);

        content.title_bar(if self.modal.is_none() {
            title_bar
        } else {
            title_bar.always_show_controls()
        })
    }

    fn view_controls(
        &'_ self,
        pane: pane_grid::Pane,
        total_panes: usize,
        is_maximized: bool,
        is_popout: bool,
    ) -> Element<'_, Message> {
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

        if matches!(
            &self.content,
            Content::Heatmap { .. } | Content::Kline { .. }
        ) {
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
            .padding(padding::right(4).left(4))
            .align_y(Vertical::Center)
            .height(Length::Fixed(32.0))
            .into()
    }

    fn compose_chart_view<'a, F>(
        &'a self,
        base: Element<'a, Message>,
        pane: pane_grid::Pane,
        indicators: &'a [impl Indicator + Copy + Into<UiIndicator>],
        compact_controls: Option<Element<'a, Message>>,
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
                modal::indicators::view(
                    pane,
                    self,
                    indicators,
                    self.stream_pair().map(|i| i.ticker.market_type()),
                ),
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
            Some(Modal::LinkGroup) => link_group_modal(base, pane, self.link_group),
            Some(Modal::Controls) => stack_modal(
                base,
                if let Some(controls) = compact_controls {
                    controls
                } else {
                    column![].into()
                },
                Message::HideModal(pane),
                padding::left(12),
                Alignment::End,
            ),
            None => base,
        }
    }

    fn compose_panel_view<'a, F>(
        &'a self,
        base: Element<'a, Message>,
        pane: pane_grid::Pane,
        compact_controls: Option<Element<'a, Message>>,
        settings_modal: F,
    ) -> Element<'a, Message>
    where
        F: FnOnce() -> Element<'a, Message>,
    {
        let base: Element<_> =
            widget::toast::Manager::new(base, &self.notifications, Alignment::End, move |msg| {
                Message::DeleteNotification(pane, msg)
            })
            .into();

        let stack_padding = padding::right(12).left(12);

        match self.modal {
            Some(Modal::Settings) => stack_modal(
                base,
                settings_modal(),
                Message::HideModal(pane),
                stack_padding,
                Alignment::End,
            ),
            Some(Modal::LinkGroup) => link_group_modal(base, pane, self.link_group),
            Some(Modal::Controls) => stack_modal(
                base,
                if let Some(controls) = compact_controls {
                    controls
                } else {
                    column![].into()
                },
                Message::HideModal(pane),
                padding::left(12),
                Alignment::End,
            ),
            _ => base,
        }
    }

    pub fn matches_stream(&self, stream: &StreamKind) -> bool {
        self.streams.matches_stream(stream)
    }

    pub fn invalidate(&mut self, now: Instant) -> Option<Action> {
        match &mut self.content {
            Content::Heatmap { chart, .. } => chart
                .as_mut()
                .and_then(|c| c.invalidate(Some(now)).map(Action::Chart)),
            Content::Kline { chart, .. } => chart
                .as_mut()
                .and_then(|c| c.invalidate(Some(now)).map(Action::Chart)),
            Content::TimeAndSales(panel) => panel
                .as_mut()
                .and_then(|p| p.invalidate(Some(now)).map(Action::Panel)),
            Content::Starter => None,
        }
    }

    pub fn update_interval(&self) -> Option<u64> {
        match &self.content {
            Content::Kline { .. } => Some(1000),
            Content::Heatmap { chart, .. } => {
                if let Some(chart) = chart {
                    chart.basis_interval()
                } else {
                    None
                }
            }
            Content::TimeAndSales(_) => Some(100),
            Content::Starter => None,
        }
    }

    pub fn last_tick(&self) -> Option<Instant> {
        self.content.last_tick()
    }

    pub fn tick(&mut self, now: Instant) -> Option<Action> {
        let invalidate_interval: Option<u64> = self.update_interval();
        let last_tick: Option<Instant> = self.last_tick();

        if let Some(streams) = self.streams.waiting_to_resolve()
            && !streams.is_empty()
        {
            return Some(Action::ResolveStreams(streams.to_vec()));
        }

        if !self.content.initialized() {
            return Some(Action::ResolveContent);
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
            streams: ResolvedStream::Waiting(vec![]),
            notifications: vec![],
            status: Status::Ready,
            link_group: None,
        }
    }
}

#[derive(Default)]
pub enum Content {
    #[default]
    Starter,
    Heatmap {
        chart: Option<HeatmapChart>,
        indicators: Vec<HeatmapIndicator>,
        layout: data::chart::ViewConfig,
        studies: Vec<data::chart::heatmap::HeatmapStudy>,
    },
    Kline {
        chart: Option<KlineChart>,
        indicators: Vec<KlineIndicator>,
        layout: data::chart::ViewConfig,
        kind: data::chart::KlineChartKind,
    },
    TimeAndSales(Option<TimeAndSales>),
}

impl Content {
    fn new_heatmap(
        current_content: &Content,
        ticker_info: TickerInfo,
        settings: &Settings,
        tick_size: f32,
    ) -> Self {
        let (enabled_indicators, layout, prev_studies) = if let Content::Heatmap {
            chart,
            indicators,
            studies,
            layout,
        } = current_content
        {
            (
                indicators.clone(),
                chart
                    .as_ref()
                    .map(|c| c.chart_layout())
                    .unwrap_or(layout.clone()),
                chart
                    .as_ref()
                    .map_or(studies.clone(), |c| c.studies.clone()),
            )
        } else {
            (
                vec![HeatmapIndicator::Volume],
                ViewConfig {
                    splits: vec![],
                    autoscale: Some(data::chart::Autoscale::CenterLatest),
                },
                vec![],
            )
        };

        let basis = settings
            .selected_basis
            .unwrap_or_else(|| Basis::default_heatmap_time(Some(ticker_info)));
        let config = settings.visual_config.and_then(|cfg| cfg.heatmap());

        let chart = HeatmapChart::new(
            layout.clone(),
            basis,
            tick_size,
            &enabled_indicators,
            Some(ticker_info),
            config,
            prev_studies.clone(),
        );

        Content::Heatmap {
            chart: Some(chart),
            indicators: enabled_indicators,
            layout,
            studies: prev_studies,
        }
    }

    fn new_kline(
        content_str: &str, // "footprint" or "candlestick"
        current_content: &Content,
        ticker_info: TickerInfo,
        settings: &Settings,
        tick_size: f32,
    ) -> Self {
        let (prev_indis, prev_layout, prev_kind_opt) = if let Content::Kline {
            chart,
            indicators,
            kind,
            layout,
        } = current_content
        {
            (
                Some(indicators.clone()),
                Some(chart.as_ref().map_or(layout.clone(), |c| c.chart_layout())),
                Some(chart.as_ref().map_or(kind.clone(), |c| c.kind().clone())),
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
                        scaling: data::chart::kline::ClusterScaling::default(),
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
            .unwrap_or(ViewConfig {
                splits,
                autoscale: Some(data::chart::Autoscale::FitToVisible),
            });

        let chart = KlineChart::new(
            layout.clone(),
            basis,
            tick_size,
            &[],
            vec![],
            &enabled_indicators,
            Some(ticker_info),
            &determined_chart_kind,
        );

        Content::Kline {
            chart: Some(chart),
            indicators: enabled_indicators,
            layout,
            kind: determined_chart_kind,
        }
    }

    pub fn last_tick(&self) -> Option<Instant> {
        match self {
            Content::Heatmap { chart, .. } => Some(chart.as_ref()?.last_update()),
            Content::Kline { chart, .. } => Some(chart.as_ref()?.last_update()),
            Content::TimeAndSales(panel) => Some(panel.as_ref()?.last_update()),
            Content::Starter => None,
        }
    }

    pub fn chart_kind(&self) -> Option<data::chart::KlineChartKind> {
        match self {
            Content::Kline { chart, .. } => Some(chart.as_ref()?.kind().clone()),
            _ => None,
        }
    }

    pub fn toggle_indicator(&mut self, indicator: UiIndicator) {
        match (self, indicator) {
            (
                Content::Heatmap {
                    chart, indicators, ..
                },
                UiIndicator::Heatmap(ind),
            ) => {
                let Some(chart) = chart else {
                    return;
                };

                if indicators.contains(&ind) {
                    indicators.retain(|i| i != &ind);
                } else {
                    indicators.push(ind);
                }
                chart.toggle_indicator(ind);
            }
            (
                Content::Kline {
                    chart, indicators, ..
                },
                UiIndicator::Kline(ind),
            ) => {
                let Some(chart) = chart else {
                    return;
                };

                if indicators.contains(&ind) {
                    indicators.retain(|i| i != &ind);
                } else {
                    indicators.push(ind);
                }
                chart.toggle_indicator(ind);
            }
            _ => panic!("indicator toggle on {indicator:?} pane",),
        }
    }

    pub fn reorder_indicators(&mut self, event: &column_drag::DragEvent) {
        match self {
            Content::Heatmap { indicators, .. } => column_drag::reorder_vec(indicators, event),
            Content::Kline { indicators, .. } => column_drag::reorder_vec(indicators, event),
            Content::TimeAndSales(_) | Content::Starter => {
                panic!("indicator reorder on {} pane", self)
            }
        }
    }

    pub fn change_visual_config(&mut self, config: VisualConfig) {
        match (self, config) {
            (Content::Heatmap { chart: Some(c), .. }, VisualConfig::Heatmap(cfg)) => {
                c.set_visual_config(cfg);
            }
            (Content::TimeAndSales(Some(panel)), VisualConfig::TimeAndSales(cfg)) => {
                panel.config = cfg;
            }
            _ => {}
        }
    }

    pub fn studies(&self) -> Option<data::chart::Study> {
        match &self {
            Content::Heatmap { studies, .. } => Some(data::chart::Study::Heatmap(studies.clone())),
            Content::Kline { kind, .. } => {
                if let data::chart::KlineChartKind::Footprint { studies, .. } = kind {
                    Some(data::chart::Study::Footprint(studies.clone()))
                } else {
                    None
                }
            }
            Content::TimeAndSales(_) | Content::Starter => None,
        }
    }

    pub fn update_studies(&mut self, studies: data::chart::Study) {
        match (self, studies) {
            (
                Content::Heatmap {
                    chart,
                    studies: previous,
                    ..
                },
                data::chart::Study::Heatmap(studies),
            ) => {
                chart
                    .as_mut()
                    .expect("heatmap chart not initialized")
                    .studies = studies.clone();
                *previous = studies;
            }
            (Content::Kline { chart, kind, .. }, data::chart::Study::Footprint(studies)) => {
                chart
                    .as_mut()
                    .expect("kline chart not initialized")
                    .set_studies(studies.clone());
                if let data::chart::KlineChartKind::Footprint {
                    studies: k_studies, ..
                } = kind
                {
                    *k_studies = studies;
                }
            }
            _ => {}
        }
    }

    pub fn identifier_str(&self) -> String {
        match self {
            Content::Starter => "starter".to_string(),
            Content::Heatmap { .. } => "heatmap".to_string(),
            Content::Kline { kind, .. } => match kind {
                data::chart::KlineChartKind::Footprint { .. } => "footprint".to_string(),
                data::chart::KlineChartKind::Candles => "candlestick".to_string(),
            },
            Content::TimeAndSales(_) => "time&sales".to_string(),
        }
    }

    fn initialized(&self) -> bool {
        match self {
            Content::Heatmap { chart, .. } => chart.is_some(),
            Content::Kline { chart, .. } => chart.is_some(),
            Content::TimeAndSales(panel) => panel.is_some(),
            Content::Starter => true,
        }
    }
}

impl std::fmt::Display for Content {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Content::Starter => write!(f, "Starter pane"),
            Content::Heatmap { .. } => write!(f, "Heatmap chart"),
            Content::Kline { kind, .. } => match kind {
                data::chart::KlineChartKind::Footprint { .. } => {
                    write!(f, "Footprint chart")
                }
                data::chart::KlineChartKind::Candles => {
                    write!(f, "Candlestick chart")
                }
            },
            Content::TimeAndSales(_) => write!(f, "Time&Sales"),
        }
    }
}

impl PartialEq for Content {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Content::Starter, Content::Starter)
                | (Content::Heatmap { .. }, Content::Heatmap { .. })
                | (Content::Kline { .. }, Content::Kline { .. })
                | (Content::TimeAndSales(_), Content::TimeAndSales(_))
        )
    }
}

fn link_group_modal(
    base: Element<Message>,
    pane: pane_grid::Pane,
    selected_group: Option<LinkGroup>,
) -> Element<Message> {
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

    let content: Element<_> = container(grid)
        .max_width(240)
        .padding(16)
        .style(style::chart_modal)
        .into();

    stack_modal(
        base,
        content,
        Message::HideModal(pane),
        padding::right(12).left(4),
        Alignment::Start,
    )
}

fn ticksize_modifier<'a>(
    id: pane_grid::Pane,
    base_ticksize: f32,
    multiplier: TickMultiplier,
    modifier: Option<modal::stream::Modifier>,
    kind: ModifierKind,
    exchange: Option<exchange::adapter::Exchange>,
) -> Element<'a, Message> {
    let modifier_modal = Modal::StreamModifier(
        modal::stream::Modifier::new(kind).with_ticksize_view(base_ticksize, multiplier, exchange),
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
