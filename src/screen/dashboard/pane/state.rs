use super::settings::presets::{self as preset_editor, Action as PresetEditorAction};
use crate::layout::{ChartResourcesEvent, ChartResourcesMessage, SharedChartResources};
use data::layout::presets::Response as PresetResponse;

use super::content::{
    self, Content,
    heatmap::{HeatmapContent, HeatmapShader},
    kline::{Action as KlineAction, ChartCommand},
    ladder::Ladder,
    timesales::TimeAndSales,
};

use crate::{
    connector::{
        client::DataSources,
        fetcher::{self, FetchSpec},
        stream::ResolvedStream,
    },
    modal::{
        self, ModifierKind,
        pane::{
            Modal,
            mini_tickers_list::{MiniPanel, RowSelection},
            stack_modal,
        },
    },
    notify::Notifications,
    screen::{
        dashboard::pane::{Action, settings},
        sidebar::tickers_table::TickersTable,
    },
    style::{self, Icon, icon_text},
    theme::ThemeState,
    widget::{
        self, button_with_tooltip,
        inspector::{InspectorBody, InspectorContent, InspectorFooter},
        link_group_button, press_button_with_tooltip,
    },
    window::{self, PixelRatio, Window},
};
use data::{
    PaneSettingsSnapshot, PaneSettingsTransfer, UserTimezone,
    chart::{
        Basis,
        kline::{ChartSettings, OpenInterestSeries},
    },
    layout::pane::{ContentKind, LinkGroup, PaneSetup, Settings, VisualConfig},
    stream::PersistStreamKind,
};
use exchange::{
    Kline, StreamPairKind, TickMultiplier, TickerInfo, Timeframe,
    adapter::{StreamKind, StreamTicksize},
    unit::PriceStep,
};
use iced::{
    Alignment, Element, Length, Renderer, Theme, padding,
    widget::{
        button, center, column, container, pane_grid, pick_list, responsive, row, rule, text,
        tooltip,
    },
};
use std::time::Instant;

#[derive(Debug, Clone)]
pub enum Effect {
    RefreshStreams,
    RequestFetch(Vec<FetchSpec>),
    SwitchTickersInGroup(TickerInfo),
    FocusWidget(iced::widget::Id),
    OpenUrl(String),
    OpenNetworkSettings,
    SharedChartResources(Box<ChartResourcesMessage>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectorPage {
    PaneSettings,
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
    ReplacePane(pane_grid::Pane),
    Popout,
    Merge,
    SwitchLinkGroup(pane_grid::Pane, Option<LinkGroup>),
    VisualConfigChanged(pane_grid::Pane, VisualConfig, bool),
    ToggleInspector(pane_grid::Pane, InspectorPage),
    SyncKlineSettings(pane_grid::Pane),
    PaneEvent(pane_grid::Pane, Event),
}

#[derive(Debug, Clone)]
pub enum Event {
    PresetEditor(preset_editor::Message),
    ShowModal(Modal),
    HideModal,
    ContentSelected(ContentKind),
    KlineInteraction(content::kline::Message),
    HeatmapShaderInteraction(content::heatmap::Message),
    HeatmapBackground(content::heatmap::BackgroundMessage),
    LadderInteraction(content::ladder::Message),
    TimeAndSalesInteraction(content::timesales::Message),
    DeleteNotification(usize),
    Settings(settings::Message),
    StreamModifierChanged(modal::stream::Message),
    MiniTickersListInteraction(modal::pane::mini_tickers_list::Message),
}

pub struct State {
    preset_editor: preset_editor::State,
    inspector: Option<InspectorPage>,
    id: uuid::Uuid,
    pub modal: Option<Modal>,
    pub content: Content,
    pub settings: Settings,
    pub notifications: Notifications,
    pub streams: ResolvedStream,
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

    fn sync_kline_streams(&mut self) -> bool {
        let Some(desired) = (match &self.content {
            Content::Kline {
                chart: Some(chart), ..
            } => Some(chart.desired_streams()),
            _ => None,
        }) else {
            return false;
        };

        let ResolvedStream::Ready(streams) = &mut self.streams else {
            return false;
        };
        if *streams == desired {
            return false;
        }
        *streams = desired;
        true
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
        if self.content.kind() != kind {
            self.close_inspector();
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
                    let streams = match &content {
                        Content::Kline {
                            chart: Some(chart), ..
                        } => chart.desired_streams(),
                        _ => Vec::new(),
                    };

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
                        tickers.len(),
                    ))));

                    let streams = tickers
                        .iter()
                        .map(|ticker_info| StreamKind::Trades {
                            ticker_info: *ticker_info,
                        })
                        .collect();

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

                    let config = self
                        .settings
                        .visual_config
                        .clone()
                        .and_then(|cfg| cfg.heatmap());

                    let indicators = self
                        .content
                        .heatmap_indicator_config()
                        .cloned()
                        .unwrap_or_default();
                    let content = Content::ShaderHeatmap(HeatmapContent::Active(Box::new(
                        HeatmapShader::new(
                            basis,
                            derived_plan.price_step,
                            base_ticker,
                            config,
                            indicators,
                        ),
                    )));

                    let streams = vec![depth_stream(&derived_plan), trades_stream(&derived_plan)];

                    (content, streams)
                }
                ContentKind::Starter => unreachable!(),
            }
        };

        self.content = content;
        self.reconcile_inspector();
        self.streams = ResolvedStream::Ready(streams.clone());

        streams
    }

    pub fn insert_hist_open_interest(
        &mut self,
        req_id: Option<uuid::Uuid>,
        ticker_info: TickerInfo,
        series: &OpenInterestSeries,
    ) {
        self.content
            .insert_historical_open_interest(req_id, ticker_info, series);
    }

    pub fn insert_hist_klines(
        &mut self,
        req_id: Option<uuid::Uuid>,
        timeframe: Timeframe,
        ticker_info: TickerInfo,
        klines: &[Kline],
        is_batches_done: bool,
        has_data: bool,
    ) {
        self.content.insert_historical_klines(
            self.streams
                .ready_iter()
                .into_iter()
                .flatten()
                .map(|stream| stream.ticker_info()),
            req_id,
            timeframe,
            ticker_info,
            klines,
            is_batches_done,
            has_data,
        );
    }

    fn has_stream(&self) -> bool {
        match &self.streams {
            ResolvedStream::Ready(streams) => !streams.is_empty(),
            ResolvedStream::Waiting { streams, .. } => !streams.is_empty(),
            ResolvedStream::Blocked { streams, .. } => !streams.is_empty(),
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
        pixel_ratio: PixelRatio,
        tickers_table: &'a TickersTable,
        theme: &'a ThemeState,
        chart_resources: &'a SharedChartResources,
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
            let mut label = base_ti.ticker.display_symbol_and_type().0;
            if base_ti.is_perps() {
                label.push_str(" PERP");
            }
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
                container(self.view_controls(id, panes, maximized, window != main_window.id))
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

        let base = match &self.content {
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
                    self.notifications.toasts(),
                    Alignment::End,
                    move |msg| Message::PaneEvent(id, Event::DeleteNotification(msg)),
                )
                .into();

                base
            }
            Content::TimeAndSales(panel) => {
                if let Some(panel) = panel {
                    panel.view(timezone).map(move |message| {
                        Message::PaneEvent(id, Event::TimeAndSalesInteraction(message))
                    })
                } else {
                    uninitialized_base(ContentKind::TimeAndSales)
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

                    let exchange = stream_pair.map(|ti| ti.ticker.exchange);
                    let min_ticksize = stream_pair.map(|ti| ti.min_ticksize);

                    let modifiers = ticksize_modifier(
                        id,
                        panel.tick_size(),
                        min_ticksize,
                        tick_multiply,
                        modifier,
                        ModifierKind::Orderbook(basis, tick_multiply),
                        exchange,
                    );

                    top_left_buttons = top_left_buttons.push(modifiers);

                    panel.view(timezone).map(move |message| {
                        Message::PaneEvent(id, Event::LadderInteraction(message))
                    })
                } else {
                    uninitialized_base(ContentKind::Ladder)
                }
            }
            Content::Kline { chart, .. } => {
                if let Some(chart) = chart {
                    let selected_basis = chart.basis();
                    let tick_multiplier = chart.tick_multiplier();
                    let kind = ModifierKind::Candlestick(selected_basis, tick_multiplier);
                    let ticker_info = chart.base_ticker_info();

                    let modifiers = row![
                        basis_modifier(id, selected_basis, modifier, kind),
                        ticksize_modifier(
                            id,
                            tick_multiplier.multiply_with_min_tick_step(ticker_info),
                            Some(ticker_info.min_ticksize),
                            tick_multiplier,
                            modifier,
                            kind,
                            Some(ticker_info.exchange()),
                        ),
                    ]
                    .spacing(4);

                    top_left_buttons = top_left_buttons.push(modifiers);

                    chart
                        .view(timezone, pixel_ratio, theme, self.inspector.is_some())
                        .map(move |message| {
                            Message::PaneEvent(id, Event::KlineInteraction(message))
                        })
                } else {
                    uninitialized_base(ContentKind::CandlestickChart)
                }
            }
            Content::ShaderHeatmap(heatmap) => {
                if let HeatmapContent::Active(chart) = heatmap {
                    let base =
                        HeatmapShader::view(chart, timezone, pixel_ratio).map(move |message| {
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

                    let min_ticksize = ticker_info.map(|ti| ti.min_ticksize);

                    let modifiers = row![
                        basis_modifier(id, basis, modifier, kind),
                        ticksize_modifier(
                            id,
                            chart.tick_size(),
                            min_ticksize,
                            tick_multiply,
                            modifier,
                            kind,
                            exchange
                        ),
                    ]
                    .spacing(4);

                    top_left_buttons = top_left_buttons.push(modifiers);

                    base
                } else {
                    uninitialized_base(ContentKind::ShaderHeatmap)
                }
            }
        };

        let body =
            self.compose_stack_view(base, id, compact_controls, tickers_table, chart_resources);

        let body = column![
            iced::widget::rule::horizontal(1.0).style(style::split_ruler),
            body,
        ]
        .width(Length::Fill)
        .height(Length::Fill);

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
                    self.view_controls(id, panes, maximized, window != main_window.id),
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

        content.title_bar(if self.modal.is_none() && self.inspector.is_none() {
            title_bar
        } else {
            title_bar.always_show_controls()
        })
    }

    pub fn update(
        &mut self,
        msg: Event,
        chart_settings: &mut ChartSettings,
        chart_resources: &SharedChartResources,
    ) -> Option<Effect> {
        let effect = self.update_event(msg, chart_settings, chart_resources);
        self.reconcile_inspector();
        effect
    }

    fn update_event(
        &mut self,
        msg: Event,
        chart_settings: &mut ChartSettings,
        chart_resources: &SharedChartResources,
    ) -> Option<Effect> {
        match msg {
            Event::PresetEditor(message) => {
                let Content::Kline {
                    chart: Some(chart), ..
                } = &self.content
                else {
                    return None;
                };
                let action = self
                    .preset_editor
                    .update(message, chart_resources.presets(), chart);
                return self.apply_preset_editor_action(action);
            }
            Event::ShowModal(requested_modal) => {
                return self.show_modal_with_focus(requested_modal);
            }
            Event::HideModal => {
                self.modal = None;
            }
            Event::ContentSelected(kind) => {
                self.close_inspector();
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
            Event::KlineInteraction(message) => {
                if matches!(
                    message,
                    content::kline::Message::Chart(
                        widget::chart::kline::KlineWidgetEvent::EscapePressed
                    )
                ) && (self.modal.take().is_some() || self.go_back())
                {
                    return None;
                }
                let action = match &mut self.content {
                    Content::Kline { chart: Some(c), .. } => c.update(message, chart_settings),
                    _ => None,
                };
                return self.finish_kline_action(action);
            }
            Event::Settings(message) => return self.update_settings(message),
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
                self.notifications.remove(idx);
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
                                        Content::ShaderHeatmap(HeatmapContent::Active(c)) => {
                                            let saved_config = c.config;
                                            **c = HeatmapShader::new(
                                                c.basis,
                                                tm.multiply_with_min_tick_step(ticker),
                                                c.ticker_info,
                                                Some(saved_config),
                                                c.indicator_config().clone(),
                                            );
                                        }
                                        Content::Kline { chart: Some(c), .. } => {
                                            let _ =
                                                c.apply_command(ChartCommand::SetTickMultiplier {
                                                    multiplier: tm,
                                                });
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
                                    Content::ShaderHeatmap(HeatmapContent::Active(c)) => {
                                        let saved_config = c.config;
                                        **c = HeatmapShader::new(
                                            new_basis,
                                            c.tick_size(),
                                            c.ticker_info,
                                            Some(saved_config),
                                            c.indicator_config().clone(),
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
                                        let action =
                                            c.apply_command(ChartCommand::SetBasis(new_basis));
                                        let stream_plan_changed = self.sync_kline_streams();

                                        if let Some(super::ChartAction::RequestFetch(fetch)) =
                                            action
                                        {
                                            effect = Some(Effect::RequestFetch(fetch));
                                        } else if stream_plan_changed {
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
                if let Content::ShaderHeatmap(HeatmapContent::Active(c)) = &mut self.content {
                    c.update(message);
                }
            }
            Event::HeatmapBackground(message) => {
                if let Content::ShaderHeatmap(HeatmapContent::Active(chart)) = &mut self.content {
                    chart.update_background(message);
                }
            }
            Event::MiniTickersListInteraction(message) => {
                if let Some(Modal::MiniTickersList(ref mut mini_panel)) = self.modal
                    && let Some(action) = mini_panel.update(message)
                {
                    self.modal = Some(Modal::MiniTickersList(mini_panel.clone()));

                    let crate::modal::pane::mini_tickers_list::Action::RowSelected(sel) = action;
                    match sel {
                        RowSelection::Add(ti) => {
                            if matches!(self.content, Content::TimeAndSales(Some(_))) {
                                return self.change_timesales_ticker(sel);
                            }
                            if let Content::Kline { chart: Some(c), .. } = &mut self.content {
                                if matches!(c.basis(), Basis::Tick(_)) {
                                    return None;
                                }

                                let _ =
                                    c.apply_command(ChartCommand::AddTicker { ticker_info: ti });
                                if self.sync_kline_streams() {
                                    return Some(Effect::RefreshStreams);
                                }
                            }
                        }
                        RowSelection::Remove(ti) => {
                            if matches!(self.content, Content::TimeAndSales(Some(_))) {
                                return self.change_timesales_ticker(sel);
                            }
                            if let Content::Kline { chart: Some(c), .. } = &mut self.content {
                                let _ =
                                    c.apply_command(ChartCommand::RemoveTicker { ticker_info: ti });
                                if self.sync_kline_streams() {
                                    return Some(Effect::RefreshStreams);
                                }
                            }
                        }
                        RowSelection::Switch(ti) => {
                            return Some(Effect::SwitchTickersInGroup(ti));
                        }
                    }
                }
            }
        }
        None
    }

    fn change_timesales_ticker(&mut self, selection: RowSelection) -> Option<Effect> {
        let ResolvedStream::Ready(streams) = &mut self.streams else {
            return None;
        };
        match selection {
            RowSelection::Add(ticker_info) => {
                if streams
                    .iter()
                    .any(|stream| stream.ticker_info().ticker == ticker_info.ticker)
                {
                    return None;
                }
                streams.push(StreamKind::Trades { ticker_info });
            }
            RowSelection::Remove(ticker_info) => {
                if streams
                    .first()
                    .is_some_and(|stream| stream.ticker_info().ticker == ticker_info.ticker)
                {
                    return None;
                }
                let previous_len = streams.len();
                streams.retain(|stream| stream.ticker_info().ticker != ticker_info.ticker);
                if streams.len() == previous_len {
                    return None;
                }
            }
            RowSelection::Switch(_) => return None,
        }

        if let Content::TimeAndSales(Some(panel)) = &mut self.content {
            let tickers: Vec<_> = streams.iter().map(|stream| stream.ticker_info()).collect();
            panel.set_sources(&tickers);
        }
        Some(Effect::RefreshStreams)
    }

    fn update_settings(&mut self, message: settings::Message) -> Option<Effect> {
        match message {
            settings::Message::HeatmapIndicator(message) => {
                if let Content::ShaderHeatmap(HeatmapContent::Active(chart)) = &mut self.content {
                    chart.update_indicator_settings(message);
                }
            }
            settings::Message::VisualConfigChanged { .. } => {}
        }

        None
    }

    fn supports_inspector(&self, page: InspectorPage) -> bool {
        match &self.content {
            Content::Kline { chart: Some(_), .. } => true,
            Content::ShaderHeatmap(HeatmapContent::Active(_))
            | Content::TimeAndSales(Some(_))
            | Content::Ladder(Some(_)) => page == InspectorPage::PaneSettings,
            _ => false,
        }
    }

    fn set_inspector(&mut self, page: Option<InspectorPage>) {
        self.inspector = page.filter(|page| self.supports_inspector(*page));
    }

    pub fn toggle_inspector(&mut self, page: InspectorPage) -> Option<iced::widget::Id> {
        self.set_inspector((self.inspector != Some(page)).then_some(page));
        (self.inspector == Some(InspectorPage::Indicators))
            .then(|| self.indicators_sidebar_focus_id())
            .flatten()
    }

    pub fn close_inspector(&mut self) -> bool {
        let was_open = self.inspector.is_some();
        if was_open {
            self.set_inspector(None);
        }
        was_open
    }

    fn reconcile_inspector(&mut self) {
        if self
            .inspector
            .is_some_and(|page| !self.supports_inspector(page))
        {
            self.close_inspector();
        }
    }

    fn inspector_view<'a>(
        &'a self,
        pane: pane_grid::Pane,
        chart_resources: &'a SharedChartResources,
    ) -> Option<Element<'a, Message>> {
        let page = self.inspector?;
        if !self.supports_inspector(page) {
            return None;
        }

        let title = match page {
            InspectorPage::PaneSettings => "Pane settings".to_owned(),
            InspectorPage::Indicators => match &self.content {
                Content::Kline {
                    chart: Some(chart), ..
                } => chart.indicator_browser_title().to_owned(),
                _ => return None,
            },
        };

        let close = button(icon_text(Icon::Close, 14))
            .style(|theme, status| style::button::transparent(theme, status, false))
            .on_press(Message::ToggleInspector(pane, page));
        let header = row![
            widget::text::title(title)
                .wrapping(iced::widget::text::Wrapping::Word)
                .width(Length::Fill),
            close,
        ]
        .align_y(Alignment::Center)
        .spacing(8);

        Some(
            container(column![
                header,
                rule::horizontal(1.0).style(style::split_ruler),
                responsive(move |size| {
                    self.inspector_body_view(pane, page, size.height, chart_resources)
                        .unwrap_or_else(|| column![].into())
                }),
            ])
            .width(240)
            .height(Length::Fill)
            .padding([4, 8])
            .style(style::chart_sidebar_container)
            .into(),
        )
    }

    fn inspector_body_view<'a>(
        &'a self,
        pane: pane_grid::Pane,
        page: InspectorPage,
        available_height: f32,
        chart_resources: &'a SharedChartResources,
    ) -> Option<Element<'a, Message>> {
        let (body, footer) = match page {
            InspectorPage::PaneSettings => {
                let (sections, footer) = self
                    .pane_settings_content(pane, chart_resources)?
                    .into_parts();
                (
                    sections,
                    footer.map(|footer| (InspectorFooter::HEIGHT, footer)),
                )
            }
            InspectorPage::Indicators => (
                InspectorBody::view_without_scroll(
                    self.indicators_sidebar_view(pane, chart_resources)?,
                ),
                None,
            ),
        };
        let body = if page == InspectorPage::Indicators {
            body
        } else {
            let footer_height = footer.as_ref().map_or(0.0, |(height, _)| *height);
            InspectorBody::view(body, (available_height - footer_height).max(0.0))
        };
        let mut content = column![body];
        if let Some((height, footer)) = footer {
            content = content.push(InspectorFooter::view(footer, height));
        }
        Some(content.into())
    }

    pub fn indicators_sidebar_view<'a>(
        &'a self,
        pane: pane_grid::Pane,
        chart_resources: &'a SharedChartResources,
    ) -> Option<Element<'a, Message>> {
        let Content::Kline {
            chart: Some(chart), ..
        } = &self.content
        else {
            return None;
        };

        Some(
            chart
                .indicator_browser_view(chart_resources.indicator_catalog())
                .map(move |message| Message::PaneEvent(pane, Event::KlineInteraction(message))),
        )
    }

    pub fn indicators_sidebar_focus_id(&self) -> Option<iced::widget::Id> {
        match &self.content {
            Content::Kline {
                chart: Some(chart), ..
            } => Some(chart.indicator_browser_focus_id()),
            _ => None,
        }
    }

    fn view_controls(
        &'_ self,
        pane: pane_grid::Pane,
        total_panes: usize,
        is_maximized: bool,
        is_popout: bool,
    ) -> Element<'_, Message> {
        let control_btn_style = |is_active: bool| {
            move |theme: &Theme, status: button::Status| {
                style::button::transparent(theme, status, is_active)
            }
        };

        let tooltip_pos = tooltip::Position::Bottom;
        let mut buttons = row![];

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
                control_btn_style(chart.left_sidebar_visible()),
            ));
        }

        for (page, icon, label) in [
            (InspectorPage::Indicators, Icon::ChartOutline, "Indicators"),
            (InspectorPage::PaneSettings, Icon::Cog, "Pane settings"),
        ] {
            if self.supports_inspector(page) {
                buttons = buttons.push(press_button_with_tooltip(
                    icon_text(icon, 12),
                    Message::ToggleInspector(pane, page),
                    Some(label),
                    tooltip_pos,
                    control_btn_style(self.inspector == Some(page)),
                ));
            }
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
        tickers_table: &'a TickersTable,
        chart_resources: &'a SharedChartResources,
    ) -> Element<'a, Message> {
        let base = if let Some(inspector) = self.inspector_view(pane, chart_resources) {
            row![
                base,
                rule::vertical(1.0).style(style::split_ruler),
                inspector
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else {
            base
        };
        let base = widget::toast::Manager::new(
            base,
            self.notifications.toasts(),
            Alignment::End,
            move |msg| Message::PaneEvent(pane, Event::DeleteNotification(msg)),
        )
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
                let mini_list = match (&self.content, &self.streams) {
                    (
                        Content::Kline {
                            chart: Some(chart), ..
                        },
                        _,
                    ) if chart.basis().is_time() => panel.view(
                        tickers_table,
                        Some(chart.series().iter().map(|series| series.ticker_info)),
                        self.stream_pair(),
                    ),
                    (Content::TimeAndSales(Some(_)), ResolvedStream::Ready(streams)) => panel.view(
                        tickers_table,
                        Some(streams.iter().map(|stream| stream.ticker_info())),
                        self.stream_pair(),
                    ),
                    _ => panel.view(
                        tickers_table,
                        None::<std::iter::Empty<TickerInfo>>,
                        self.stream_pair(),
                    ),
                }
                .map(move |msg| Message::PaneEvent(pane, Event::MiniTickersListInteraction(msg)));

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

    pub fn go_back(&mut self) -> bool {
        if self.inspector == Some(InspectorPage::PaneSettings) && self.preset_editor.cancel() {
            return true;
        }
        if self.inspector == Some(InspectorPage::Indicators)
            && let Content::Kline {
                chart: Some(chart), ..
            } = &mut self.content
            && chart.indicator_browser_go_back()
        {
            return true;
        }
        if let Content::Kline {
            chart: Some(chart), ..
        } = &mut self.content
            && chart.go_back()
        {
            return true;
        }
        if self.close_inspector() {
            return true;
        }
        if let Content::Kline {
            chart: Some(chart), ..
        } = &mut self.content
        {
            return chart.handle_escape();
        }
        false
    }

    pub fn pane_settings_content<'a>(
        &'a self,
        pane: pane_grid::Pane,
        chart_resources: &'a SharedChartResources,
    ) -> Option<InspectorContent<'a, Message>> {
        let mut content = self.content.pane_settings_content(
            pane,
            self.stream_pair(),
            self.settings.selected_basis,
        )?;
        if let Content::Kline {
            chart: Some(chart), ..
        } = &self.content
        {
            content.prepend(
                crate::widget::inspector::InspectorSection::titled(
                    "Presets",
                    self.preset_editor
                        .view(chart_resources.presets(), chart)
                        .map(move |message| Message::PaneEvent(pane, Event::PresetEditor(message))),
                )
                .with_modal_container(),
            );
        }
        Some(content)
    }

    fn apply_preset_editor_action(&mut self, action: PresetEditorAction) -> Option<Effect> {
        match action {
            PresetEditorAction::None => None,
            PresetEditorAction::FocusWidget(id) => Some(Effect::FocusWidget(id)),
            PresetEditorAction::Command(command) => Some(Effect::SharedChartResources(Box::new(
                ChartResourcesMessage::Presets(command),
            ))),
            PresetEditorAction::Apply { id, application } => {
                let Content::Kline {
                    chart: Some(chart), ..
                } = &mut self.content
                else {
                    return None;
                };
                let action = chart.apply_preset(id, application);
                self.reconcile_inspector();
                self.finish_kline_action(action)
            }
        }
    }

    pub fn chart_resources_updated(&mut self, event: ChartResourcesEvent) {
        let ChartResourcesEvent::Presets(response) = event;
        let Content::Kline {
            chart: Some(chart), ..
        } = &mut self.content
        else {
            return;
        };
        if let Ok(PresetResponse::Created(id)) = &response {
            chart.selected_preset = Some(*id);
        }
        self.preset_editor.command_completed(response);
    }

    fn finish_kline_action(&mut self, action: Option<KlineAction>) -> Option<Effect> {
        let effect = match action {
            Some(KlineAction::RequestFetch(fetch)) => Some(Effect::RequestFetch(fetch)),
            Some(KlineAction::OpenNetworkSettings) => Some(Effect::OpenNetworkSettings),
            Some(KlineAction::OpenUrl(url)) => Some(Effect::OpenUrl(url)),
            Some(KlineAction::FocusWidget(id)) => Some(Effect::FocusWidget(id)),
            Some(KlineAction::SharedChartResources(message)) => {
                Some(Effect::SharedChartResources(message))
            }
            Some(KlineAction::RemoveSeries(ticker_info)) => {
                if let Content::Kline {
                    chart: Some(chart), ..
                } = &mut self.content
                {
                    let _ = chart.apply_command(ChartCommand::RemoveTicker { ticker_info });
                }
                Some(Effect::RefreshStreams)
            }
            None => None,
        };
        let streams_changed = self.sync_kline_streams();
        effect.or_else(|| streams_changed.then_some(Effect::RefreshStreams))
    }

    pub fn clear_chart_preset(&mut self, id: uuid::Uuid) {
        if let Content::Kline { chart, layout, .. } = &mut self.content {
            if let Some(chart) = chart
                && chart.selected_preset == Some(id)
            {
                chart.selected_preset = None;
            }
            if let Some(config) = layout
                && config.selected_preset == Some(id)
            {
                config.selected_preset = None;
            }
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
        self.content.invalidate(now).map(Action::Chart)
    }

    pub fn park_for_inactive_layout(&mut self) {
        self.content.park();
    }

    pub fn tick(
        &mut self,
        now: Instant,
        pane: pane_grid::Pane,
    ) -> (Option<Action>, iced::Task<Message>) {
        let action = self.tick_action(now);
        let task = self
            .content
            .background_task()
            .map(|task| task.map(move |event| Message::PaneEvent(pane, event)))
            .unwrap_or_else(iced::Task::none);
        (action, task)
    }

    fn tick_action(&mut self, now: Instant) -> Option<Action> {
        if let Some(streams) = self.streams.due_streams_to_resolve(now) {
            return Some(Action::ResolveStreams(streams));
        }

        if !self.content.initialized() {
            return Some(Action::ResolveContent);
        }

        self.content.tick(now).map(Action::Chart)
    }

    pub fn unique_id(&self) -> uuid::Uuid {
        self.id
    }

    pub fn request_fetch(
        &mut self,
        data_sources: &DataSources,
        requests: Vec<FetchSpec>,
    ) -> iced::Task<fetcher::FetchUpdate> {
        let pane_id = self.unique_id();
        let ready_streams = self
            .streams
            .ready_iter()
            .map(|iter| iter.copied().collect::<Vec<_>>())
            .unwrap_or_default();

        fetcher::request_fetch_many(
            data_sources,
            pane_id,
            &ready_streams,
            requests
                .into_iter()
                .map(|request| (request.req_id, request.fetch, request.stream)),
            |req_id, handle| {
                self.content.apply_fetch_handle(req_id, handle);
            },
        )
    }

    pub fn settings_snapshot(
        &self,
        transfer: PaneSettingsTransfer,
    ) -> Option<PaneSettingsSnapshot> {
        self.content.settings_snapshot(transfer)
    }

    pub fn apply_settings_snapshot(&mut self, snapshot: &PaneSettingsSnapshot) -> Option<Effect> {
        if !self.content.can_apply_settings_snapshot(snapshot) {
            return None;
        }

        if !matches!(snapshot, PaneSettingsSnapshot::Kline(_)) {
            let visual_config = snapshot.visual_config();
            self.settings.visual_config = visual_config.clone();
            if let Some(config) = visual_config {
                self.content.change_visual_config(config);
            }
        }

        let action = self.content.apply_settings_snapshot(snapshot);
        self.reconcile_inspector();
        self.finish_kline_action(action)
    }
}

impl Default for State {
    fn default() -> Self {
        Self {
            preset_editor: preset_editor::State::default(),
            inspector: None,
            id: uuid::Uuid::new_v4(),
            modal: None,
            content: Content::Starter,
            settings: Settings::default(),
            streams: ResolvedStream::waiting(vec![]),
            notifications: Notifications::new(),
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
