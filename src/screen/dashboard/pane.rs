use crate::{
    StreamType,
    charts::{
        self, ChartBasis,
        candlestick::CandlestickChart,
        config::{self, VisualConfig},
        footprint::FootprintChart,
        heatmap::HeatmapChart,
        indicators::{CandlestickIndicator, FootprintIndicator, HeatmapIndicator, Indicator},
        timeandsales::TimeAndSales,
    },
    data_providers::{
        Exchange, Kline, MarketType, OpenInterest, TickMultiplier, Ticker, TickerInfo,
        aggr::{ticks::TickCount, time::Timeframe},
    },
    layout::SerializableChartData,
    screen::{
        self, DashboardError, Notification, UserTimezone, create_button,
        modal::{pane_menu, pane_notification},
    },
    style::{self, Icon, get_icon_text},
    window::{self, Window},
};
use iced::{
    Alignment, Element, Length, Renderer, Task, Theme,
    alignment::{Horizontal, Vertical},
    padding,
    widget::{button, center, column, container, pane_grid, row, scrollable, text, tooltip},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
pub enum PaneModal {
    StreamModifier,
    Settings,
    Indicators,
    None,
}

enum ExistingIndicators {
    Heatmap(Vec<HeatmapIndicator>),
    Footprint(Vec<FootprintIndicator>),
    Candlestick(Vec<CandlestickIndicator>),
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
    TicksizeSelected(TickMultiplier, pane_grid::Pane),
    ChartBasisSelected(ChartBasis, pane_grid::Pane),
    ToggleModal(pane_grid::Pane, PaneModal),
    InitPaneContent(
        window::Id,
        String,
        Option<pane_grid::Pane>,
        Vec<StreamType>,
        TickerInfo,
    ),
    ReplacePane(pane_grid::Pane),
    ChartUserUpdate(pane_grid::Pane, charts::Message),
    VisualConfigChanged(Option<pane_grid::Pane>, VisualConfig),
    ToggleIndicator(pane_grid::Pane, String),
    HideNotification(pane_grid::Pane, Notification),
    Popout,
    Merge,
}

pub struct PaneState {
    pub modal: PaneModal,
    pub stream: Vec<StreamType>,
    pub content: PaneContent,
    pub settings: PaneSettings,
}

impl PaneState {
    pub fn new(stream: Vec<StreamType>, settings: PaneSettings) -> Self {
        Self {
            modal: PaneModal::None,
            stream,
            content: PaneContent::Starter,
            settings,
        }
    }

    pub fn from_config(
        content: PaneContent,
        stream: Vec<StreamType>,
        settings: PaneSettings,
    ) -> Self {
        Self {
            modal: PaneModal::None,
            stream,
            content,
            settings,
        }
    }

    /// sets the ticker info, tries to return multiplied tick size, otherwise returns the min tick size
    pub fn set_tickers_info(
        &mut self,
        multiplier: Option<TickMultiplier>,
        ticker_info: TickerInfo,
    ) -> f32 {
        self.settings.ticker_info = Some(ticker_info);

        if let Some(multiplier) = multiplier {
            self.settings.tick_multiply = Some(multiplier);
            multiplier.multiply_with_min_tick_size(ticker_info)
        } else {
            ticker_info.min_ticksize
        }
    }

    pub fn get_ticker_exchange(&self) -> Option<(Exchange, Ticker)> {
        for stream in &self.stream {
            match stream {
                StreamType::DepthAndTrades { exchange, ticker } => {
                    return Some((*exchange, *ticker));
                }
                StreamType::Kline {
                    exchange, ticker, ..
                } => {
                    return Some((*exchange, *ticker));
                }
                _ => {}
            }
        }
        None
    }

    pub fn init_content_task(
        &mut self,
        content: &str,
        exchange: Exchange,
        ticker: (Ticker, TickerInfo),
        pane: pane_grid::Pane,
        window: window::Id,
    ) -> Task<Message> {
        let streams = match content {
            "heatmap" | "time&sales" => {
                vec![StreamType::DepthAndTrades {
                    exchange,
                    ticker: ticker.0,
                }]
            }
            "footprint" => {
                let basis = self
                    .settings
                    .selected_basis
                    .unwrap_or(ChartBasis::Time(Timeframe::M5.into()));

                match basis {
                    ChartBasis::Time(interval) => {
                        vec![
                            StreamType::DepthAndTrades {
                                exchange,
                                ticker: ticker.0,
                            },
                            StreamType::Kline {
                                exchange,
                                ticker: ticker.0,
                                timeframe: interval.into(),
                            },
                        ]
                    }
                    ChartBasis::Tick(_) => {
                        vec![StreamType::DepthAndTrades {
                            exchange,
                            ticker: ticker.0,
                        }]
                    }
                }
            }
            "candlestick" => {
                let basis = self
                    .settings
                    .selected_basis
                    .unwrap_or(ChartBasis::Time(Timeframe::M15.into()));

                match basis {
                    ChartBasis::Time(interval) => {
                        vec![StreamType::Kline {
                            exchange,
                            ticker: ticker.0,
                            timeframe: interval.into(),
                        }]
                    }
                    ChartBasis::Tick(_) => {
                        vec![StreamType::DepthAndTrades {
                            exchange,
                            ticker: ticker.0,
                        }]
                    }
                }
            }
            _ => vec![],
        };

        self.stream = streams.clone();

        Task::done(Message::InitPaneContent(
            window,
            content.to_string(),
            Some(pane),
            streams,
            ticker.1,
        ))
    }

    pub fn set_content(
        &mut self,
        ticker_info: TickerInfo,
        content_str: &str,
    ) -> Result<(), DashboardError> {
        let (existing_indicators, existing_layout) = match (&self.content, content_str) {
            (PaneContent::Heatmap(chart, indicators), "heatmap") => (
                Some(ExistingIndicators::Heatmap(indicators.clone())),
                Some(chart.get_chart_layout()),
            ),
            (PaneContent::Footprint(chart, indicators), "footprint") => (
                Some(ExistingIndicators::Footprint(indicators.clone())),
                Some(chart.get_chart_layout()),
            ),
            (PaneContent::Candlestick(chart, indicators), "candlestick") => (
                Some(ExistingIndicators::Candlestick(indicators.clone())),
                Some(chart.get_chart_layout()),
            ),
            _ => (None, None),
        };

        self.content = match content_str {
            "heatmap" => {
                let tick_size = self.set_tickers_info(Some(TickMultiplier(10)), ticker_info);
                let enabled_indicators = match existing_indicators {
                    Some(ExistingIndicators::Heatmap(indicators)) => indicators,
                    _ => vec![HeatmapIndicator::Volume],
                };
                let layout = existing_layout.unwrap_or(SerializableChartData {
                    crosshair: true,
                    indicators_split: None,
                });

                let config = self.settings.visual_config.and_then(|cfg| cfg.heatmap());

                PaneContent::Heatmap(
                    HeatmapChart::new(
                        layout,
                        tick_size,
                        100,
                        &enabled_indicators,
                        Some(ticker_info),
                        config,
                    ),
                    enabled_indicators,
                )
            }
            "footprint" => {
                let tick_size = self.set_tickers_info(Some(TickMultiplier(50)), ticker_info);

                let basis = self
                    .settings
                    .selected_basis
                    .unwrap_or(ChartBasis::Time(Timeframe::M5.into()));

                let enabled_indicators = match existing_indicators {
                    Some(ExistingIndicators::Footprint(indicators)) => indicators,
                    _ => vec![FootprintIndicator::Volume, FootprintIndicator::OpenInterest],
                };

                let layout = existing_layout.unwrap_or(SerializableChartData {
                    crosshair: true,
                    indicators_split: Some(0.8),
                });

                PaneContent::Footprint(
                    FootprintChart::new(
                        layout,
                        basis,
                        tick_size,
                        vec![],
                        vec![],
                        &enabled_indicators,
                        Some(ticker_info),
                    ),
                    enabled_indicators,
                )
            }
            "candlestick" => {
                let tick_size = self.set_tickers_info(None, ticker_info);

                let basis = self
                    .settings
                    .selected_basis
                    .unwrap_or(ChartBasis::Time(Timeframe::M15.into()));

                let enabled_indicators = match existing_indicators {
                    Some(ExistingIndicators::Candlestick(indicators)) => indicators,
                    _ => vec![
                        CandlestickIndicator::Volume,
                        CandlestickIndicator::OpenInterest,
                    ],
                };

                let layout = existing_layout.unwrap_or(SerializableChartData {
                    crosshair: true,
                    indicators_split: Some(0.8),
                });

                PaneContent::Candlestick(
                    CandlestickChart::new(
                        layout,
                        basis,
                        vec![],
                        vec![],
                        tick_size,
                        &enabled_indicators,
                        Some(ticker_info),
                    ),
                    enabled_indicators,
                )
            }
            "time&sales" => {
                let config = self
                    .settings
                    .visual_config
                    .and_then(|cfg| cfg.time_and_sales());

                PaneContent::TimeAndSales(TimeAndSales::new(config))
            }
            _ => {
                log::error!("content not found: {}", content_str);
                return Err(DashboardError::PaneSet(
                    "content not found: ".to_string() + content_str,
                ));
            }
        };

        Ok(())
    }

    pub fn insert_oi_vec(&mut self, req_id: Option<uuid::Uuid>, oi: Vec<OpenInterest>) {
        match &mut self.content {
            PaneContent::Candlestick(chart, _) => {
                chart.insert_open_interest(req_id, oi);
            }
            PaneContent::Footprint(chart, _) => {
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
        klines: &Vec<Kline>,
    ) {
        match &mut self.content {
            PaneContent::Candlestick(chart, indicators) => {
                if let Some(id) = req_id {
                    chart.insert_new_klines(id, klines);
                } else {
                    let (raw_trades, tick_size) = (chart.get_raw_trades(), chart.get_tick_size());
                    let layout = chart.get_chart_layout();
                    let ticker_info = self.settings.ticker_info;

                    *chart = CandlestickChart::new(
                        layout,
                        ChartBasis::Time(timeframe.into()),
                        klines.clone(),
                        raw_trades,
                        tick_size,
                        indicators,
                        ticker_info,
                    );
                }
            }
            PaneContent::Footprint(chart, indicators) => {
                if let Some(id) = req_id {
                    chart.insert_new_klines(id, klines);
                } else {
                    let (raw_trades, tick_size) = (chart.get_raw_trades(), chart.get_tick_size());
                    let layout = chart.get_chart_layout();
                    let ticker_info = self.settings.ticker_info;

                    *chart = FootprintChart::new(
                        layout,
                        ChartBasis::Time(timeframe.into()),
                        tick_size,
                        klines.clone(),
                        raw_trades,
                        indicators,
                        ticker_info,
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
        timezone: &'a UserTimezone,
        notifications: Option<&'a Vec<screen::Notification>>,
    ) -> pane_grid::Content<'a, Message, Theme, Renderer> {
        let mut stream_info_element = row![]
            .padding(padding::left(8))
            .align_y(Vertical::Center)
            .spacing(8)
            .height(Length::Fixed(32.0));

        if let Some((exchange, ticker)) = self.get_ticker_exchange() {
            let (ticker_str, market) = ticker.get_string();

            stream_info_element = stream_info_element.push(
                row![
                    match exchange {
                        Exchange::BinanceFutures | Exchange::BinanceSpot =>
                            get_icon_text(Icon::BinanceLogo, 14),
                        Exchange::BybitLinear | Exchange::BybitSpot =>
                            get_icon_text(Icon::BybitLogo, 14),
                    },
                    text({
                        if market == MarketType::LinearPerps {
                            ticker_str + " PERP"
                        } else {
                            ticker_str
                        }
                    })
                    .size(14),
                ]
                .spacing(4),
            );
        }

        let mut is_chart = false;
        let is_stream_modifier = self.modal == PaneModal::StreamModifier;

        match self.content {
            PaneContent::Heatmap(_, _) => {
                stream_info_element = stream_info_element.push(
                    button(text(
                        self.settings
                            .tick_multiply
                            .unwrap_or(TickMultiplier(5))
                            .to_string(),
                    ))
                    .style(move |theme, status| {
                        style::button_modifier(theme, status, !is_stream_modifier)
                    })
                    .on_press(Message::ToggleModal(id, PaneModal::StreamModifier)),
                );

                is_chart = true;
            }
            PaneContent::Footprint(_, _) => {
                stream_info_element = stream_info_element.push(
                    button(text(format!(
                        "{} - {}",
                        self.settings
                            .selected_basis
                            .unwrap_or(ChartBasis::Time(Timeframe::M5.into())),
                        self.settings.tick_multiply.unwrap_or(TickMultiplier(10)),
                    )))
                    .style(move |theme, status| {
                        style::button_modifier(theme, status, !is_stream_modifier)
                    })
                    .on_press(Message::ToggleModal(id, PaneModal::StreamModifier)),
                );

                is_chart = true;
            }
            PaneContent::Candlestick(_, _) => {
                stream_info_element = stream_info_element.push(
                    button(text(
                        self.settings
                            .selected_basis
                            .unwrap_or(ChartBasis::Time(Timeframe::M15.into()))
                            .to_string(),
                    ))
                    .style(move |theme, status| {
                        style::button_modifier(theme, status, !is_stream_modifier)
                    })
                    .on_press(Message::ToggleModal(id, PaneModal::StreamModifier)),
                );

                is_chart = true;
            }
            _ => {}
        }

        let content = pane_grid::Content::new(match &self.content {
            PaneContent::Starter => center(text("select a ticker to start").size(16)).into(),
            PaneContent::Heatmap(content, indicators) => {
                view_chart(id, self, content, notifications, indicators, timezone)
            }
            PaneContent::Footprint(content, indicators) => {
                view_chart(id, self, content, notifications, indicators, timezone)
            }
            PaneContent::Candlestick(content, indicators) => {
                view_chart(id, self, content, notifications, indicators, timezone)
            }
            PaneContent::TimeAndSales(content) => {
                view_panel(id, self, content, notifications, timezone)
            }
        })
        .style(move |theme| style::pane_background(theme, is_focused));

        let title_bar = pane_grid::TitleBar::new(stream_info_element)
            .controls(view_controls(
                id,
                panes,
                maximized,
                window != main_window.id,
                is_chart,
            ))
            .style(style::pane_title_bar);

        content.title_bar(title_bar)
    }

    pub fn matches_stream(&self, stream: &StreamType) -> bool {
        self.stream.iter().any(|existing| existing == stream)
    }
}

/// Pane `view()` traits that includes a chart with `Canvas`
///
/// e.g. panes for Heatmap, Footprint, Candlestick charts
trait ChartView {
    fn view<'a, I: Indicator>(
        &'a self,
        pane: pane_grid::Pane,
        state: &'a PaneState,
        indicators: &'a [I],
        notifications: Option<&'a Vec<screen::Notification>>,
        timezone: &'a UserTimezone,
    ) -> Element<'a, Message>;
}

#[derive(Debug, Clone, Copy)]
enum StreamModifier {
    CandlestickChart(ChartBasis),
    FootprintChart(ChartBasis, TickMultiplier),
    HeatmapChart(TickMultiplier),
}

fn handle_chart_view<'a, F>(
    underlay: Element<'a, Message>,
    state: &'a PaneState,
    pane: pane_grid::Pane,
    indicators: &'a [impl Indicator],
    settings_view: F,
    notifications: Option<&'a Vec<screen::Notification>>,
    stream_modifier: StreamModifier,
) -> Element<'a, Message>
where
    F: FnOnce() -> Element<'a, Message>,
{
    let base = if let Some(notifications) = notifications {
        pane_notification(
            underlay,
            screen::notification_modal(notifications, move |notification| {
                Message::HideNotification(pane, notification)
            }),
        )
    } else {
        underlay
    };

    match state.modal {
        PaneModal::StreamModifier => pane_menu(
            base,
            stream_modifier_view(pane, stream_modifier),
            Message::ToggleModal(pane, PaneModal::None),
            padding::left(36),
            Alignment::Start,
        ),
        PaneModal::Indicators => pane_menu(
            base,
            indicators_view(
                pane,
                state
                    .settings
                    .ticker_info
                    .map(|info| info.get_market_type()),
                indicators,
            ),
            Message::ToggleModal(pane, PaneModal::None),
            padding::right(12).left(12),
            Alignment::End,
        ),
        PaneModal::Settings => pane_menu(
            base,
            settings_view(),
            Message::ToggleModal(pane, PaneModal::None),
            padding::right(12).left(12),
            Alignment::End,
        ),
        _ => base,
    }
}

impl ChartView for HeatmapChart {
    fn view<'a, I: Indicator>(
        &'a self,
        pane: pane_grid::Pane,
        state: &'a PaneState,
        indicators: &'a [I],
        notifications: Option<&'a Vec<screen::Notification>>,
        timezone: &'a UserTimezone,
    ) -> Element<'a, Message> {
        let underlay = self
            .view(indicators, timezone)
            .map(move |message| Message::ChartUserUpdate(pane, message));

        let settings_view = || config::heatmap_cfg_view(self.get_visual_config(), pane);

        handle_chart_view(
            underlay,
            state,
            pane,
            indicators,
            settings_view,
            notifications,
            StreamModifier::HeatmapChart(
                state.settings.tick_multiply.unwrap_or(TickMultiplier(10)),
            ),
        )
    }
}

impl ChartView for FootprintChart {
    fn view<'a, I: Indicator>(
        &'a self,
        pane: pane_grid::Pane,
        state: &'a PaneState,
        indicators: &'a [I],
        notifications: Option<&'a Vec<screen::Notification>>,
        timezone: &'a UserTimezone,
    ) -> Element<'a, Message> {
        let underlay = self
            .view(indicators, timezone)
            .map(move |message| Message::ChartUserUpdate(pane, message));

        let settings_view = || blank_settings_view();

        handle_chart_view(
            underlay,
            state,
            pane,
            indicators,
            settings_view,
            notifications,
            StreamModifier::FootprintChart(
                state
                    .settings
                    .selected_basis
                    .unwrap_or(ChartBasis::Time(Timeframe::M5.into())),
                state.settings.tick_multiply.unwrap_or(TickMultiplier(10)),
            ),
        )
    }
}

impl ChartView for CandlestickChart {
    fn view<'a, I: Indicator>(
        &'a self,
        pane: pane_grid::Pane,
        state: &'a PaneState,
        indicators: &'a [I],
        notifications: Option<&'a Vec<screen::Notification>>,
        timezone: &'a UserTimezone,
    ) -> Element<'a, Message> {
        let underlay = self
            .view(indicators, timezone)
            .map(move |message| Message::ChartUserUpdate(pane, message));

        let settings_view = || blank_settings_view();

        handle_chart_view(
            underlay,
            state,
            pane,
            indicators,
            settings_view,
            notifications,
            StreamModifier::CandlestickChart(
                state
                    .settings
                    .selected_basis
                    .unwrap_or(ChartBasis::Time(Timeframe::M15.into())),
            ),
        )
    }
}

/// Pane `view()` traits that doesnt include a chart, `Canvas`
///
/// e.g. Time&Sales pane
trait PanelView {
    fn view(
        &self,
        pane: pane_grid::Pane,
        state: &PaneState,
        timezone: &UserTimezone,
    ) -> Element<Message>;
}

impl PanelView for TimeAndSales {
    fn view(
        &self,
        pane: pane_grid::Pane,
        state: &PaneState,
        timezone: &UserTimezone,
    ) -> Element<Message> {
        let underlay = self.view(timezone);

        match state.modal {
            PaneModal::Settings => pane_menu(
                underlay,
                config::timesales_cfg_view(self.get_config(), pane),
                Message::ToggleModal(pane, PaneModal::None),
                padding::right(12).left(12),
                Alignment::End,
            ),
            _ => underlay,
        }
    }
}

// Modal views, overlay
fn indicators_view<I: Indicator>(
    pane: pane_grid::Pane,
    market_type: Option<MarketType>,
    selected: &[I],
) -> Element<Message> {
    let mut content_row =
        column![container(text("Indicators").size(14)).padding(padding::bottom(8)),].spacing(4);

    for indicator in I::get_available(market_type) {
        content_row = content_row.push(if selected.contains(indicator) {
            button(text(indicator.to_string()))
                .on_press(Message::ToggleIndicator(pane, indicator.to_string()))
                .width(Length::Fill)
                .style(move |theme, status| style::button_transparent(theme, status, true))
        } else {
            button(text(indicator.to_string()))
                .on_press(Message::ToggleIndicator(pane, indicator.to_string()))
                .width(Length::Fill)
                .style(move |theme, status| style::button_transparent(theme, status, false))
        });
    }

    container(content_row)
        .max_width(200)
        .padding(16)
        .style(style::chart_modal)
        .into()
}

fn stream_modifier_view<'a>(
    pane: pane_grid::Pane,
    modifiers: StreamModifier,
) -> Element<'a, Message> {
    let (selected_basis, selected_ticksize) = match modifiers {
        StreamModifier::CandlestickChart(basis) => (Some(basis), None),
        StreamModifier::FootprintChart(basis, ticksize) => (Some(basis), Some(ticksize)),
        StreamModifier::HeatmapChart(ticksize) => (None, Some(ticksize)),
    };

    let create_button = |content: String, msg: Option<Message>, active: bool| {
        let btn = button(container(text(content)).align_x(Horizontal::Center))
            .width(Length::Fill)
            .style(move |theme, status| style::button_transparent(theme, status, active));

        if let Some(msg) = msg {
            btn.on_press(msg)
        } else {
            btn
        }
    };

    let mut content_row = row![].align_y(Vertical::Center).spacing(16);

    let mut timeframes_column = column![].padding(4).align_x(Horizontal::Center);

    let mut tick_basis_column = column![].padding(4).align_x(Horizontal::Center);

    if let Some(basis) = selected_basis {
        match basis {
            ChartBasis::Time(selected_timeframe) => {
                timeframes_column = timeframes_column.push(
                    row![
                        create_button("Timeframe".to_string(), None, false,),
                        create_button(
                            "Ticks".to_string(),
                            Some(Message::ChartBasisSelected(ChartBasis::Tick(200), pane,)),
                            true,
                        ),
                    ]
                    .padding(padding::bottom(8))
                    .spacing(4),
                );

                for timeframe in &Timeframe::ALL {
                    let msg = if *timeframe == selected_timeframe.into() {
                        None
                    } else {
                        Some(Message::ChartBasisSelected(
                            ChartBasis::Time(u64::from(*timeframe)),
                            pane,
                        ))
                    };
                    timeframes_column =
                        timeframes_column.push(create_button(timeframe.to_string(), msg, false));
                }

                content_row =
                    content_row.push(container(timeframes_column).style(style::modal_container));
            }
            ChartBasis::Tick(selected_tick) => {
                tick_basis_column = tick_basis_column.push(
                    row![
                        create_button(
                            "Timeframe".to_string(),
                            Some(Message::ChartBasisSelected(
                                ChartBasis::Time(Timeframe::M5.into()),
                                pane
                            )),
                            true,
                        ),
                        create_button("Ticks".to_string(), None, false,),
                    ]
                    .padding(padding::bottom(8))
                    .spacing(4),
                );

                for tick_count in &TickCount::ALL {
                    let msg = if *tick_count == selected_tick.into() {
                        None
                    } else {
                        Some(Message::ChartBasisSelected(
                            ChartBasis::Tick(u64::from(*tick_count)),
                            pane,
                        ))
                    };
                    tick_basis_column =
                        tick_basis_column.push(create_button(tick_count.to_string(), msg, false));
                }

                content_row =
                    content_row.push(container(tick_basis_column).style(style::modal_container));
            }
        }
    }

    let mut ticksizes_column = column![].padding(4).align_x(Horizontal::Center);

    if selected_ticksize.is_some() {
        ticksizes_column =
            ticksizes_column.push(container(text("Ticksize Mltp.")).padding(padding::bottom(8)));

        for ticksize in &TickMultiplier::ALL {
            let msg = if selected_ticksize == Some(*ticksize) {
                None
            } else {
                Some(Message::TicksizeSelected(*ticksize, pane))
            };
            ticksizes_column =
                ticksizes_column.push(create_button(ticksize.to_string(), msg, false));
        }

        content_row = content_row.push(container(ticksizes_column).style(style::modal_container));
    }

    container(scrollable::Scrollable::with_direction(
        content_row,
        scrollable::Direction::Vertical(scrollable::Scrollbar::new().width(4).scroller_width(4)),
    ))
    .padding(16)
    .max_width(if selected_ticksize.is_some() && selected_basis.is_some() {
        380
    } else if selected_basis.is_some() {
        200
    } else {
        120
    })
    .style(style::chart_modal)
    .into()
}

fn blank_settings_view<'a>() -> Element<'a, Message> {
    container(text(
        "This chart type doesn't have any configurations, WIP...",
    ))
    .padding(16)
    .width(Length::Shrink)
    .max_width(500)
    .style(style::chart_modal)
    .into()
}

// Main pane content views, underlays
fn view_panel<'a, C: PanelView>(
    pane: pane_grid::Pane,
    state: &'a PaneState,
    content: &'a C,
    notifications: Option<&'a Vec<screen::Notification>>,
    timezone: &'a UserTimezone,
) -> Element<'a, Message> {
    let base = center(content.view(pane, state, timezone));

    if let Some(notifications) = notifications {
        pane_notification(
            base,
            screen::notification_modal(notifications, move |notification| {
                Message::HideNotification(pane, notification)
            }),
        )
    } else {
        base.into()
    }
}

fn view_chart<'a, C: ChartView, I: Indicator>(
    pane: pane_grid::Pane,
    state: &'a PaneState,
    content: &'a C,
    notifications: Option<&'a Vec<screen::Notification>>,
    indicators: &'a [I],
    timezone: &'a UserTimezone,
) -> Element<'a, Message> {
    content.view(pane, state, indicators, notifications, timezone)
}

// Pane controls, title bar
fn view_controls<'a>(
    pane: pane_grid::Pane,
    total_panes: usize,
    is_maximized: bool,
    is_popout: bool,
    is_chart: bool,
) -> Element<'a, Message> {
    let button_style =
        |theme: &Theme, status: button::Status| style::button_transparent(theme, status, false);
    let tooltip_pos = tooltip::Position::Bottom;

    let mut buttons = row![create_button(
        get_icon_text(Icon::Cog, 12),
        Message::ToggleModal(pane, PaneModal::Settings),
        None,
        tooltip_pos,
        button_style,
    )];

    if is_chart {
        buttons = buttons.push(create_button(
            get_icon_text(Icon::ChartOutline, 12),
            Message::ToggleModal(pane, PaneModal::Indicators),
            Some("Indicators"),
            tooltip_pos,
            button_style,
        ));
    }

    if is_popout {
        buttons = buttons.push(create_button(
            get_icon_text(Icon::Popout, 12),
            Message::Merge,
            Some("Merge"),
            tooltip_pos,
            button_style,
        ));
    } else if total_panes > 1 {
        buttons = buttons.push(create_button(
            get_icon_text(Icon::Popout, 12),
            Message::Popout,
            Some("Pop out"),
            tooltip_pos,
            button_style,
        ));
    }

    if total_panes > 1 {
        let (resize_icon, message) = if is_maximized {
            (Icon::ResizeSmall, Message::Restore)
        } else {
            (Icon::ResizeFull, Message::MaximizePane(pane))
        };

        buttons = buttons.push(create_button(
            get_icon_text(resize_icon, 12),
            message,
            None,
            tooltip_pos,
            button_style,
        ));

        buttons = buttons.push(create_button(
            get_icon_text(Icon::Close, 12),
            Message::ClosePane(pane),
            None,
            tooltip_pos,
            button_style,
        ));
    }

    buttons
        .padding(padding::right(4))
        .align_y(Vertical::Center)
        .height(Length::Fixed(32.0))
        .into()
}

pub enum PaneContent {
    Starter,
    Heatmap(HeatmapChart, Vec<HeatmapIndicator>),
    Footprint(FootprintChart, Vec<FootprintIndicator>),
    Candlestick(CandlestickChart, Vec<CandlestickIndicator>),
    TimeAndSales(TimeAndSales),
}

impl PaneContent {
    pub fn toggle_indicator(&mut self, indicator_str: String) {
        match self {
            PaneContent::Heatmap(chart, indicators) => {
                let indicator = match indicator_str.as_str() {
                    "Volume" => HeatmapIndicator::Volume,
                    _ => {
                        log::error!("indicator not found: {}", indicator_str);
                        return;
                    }
                };

                if indicators.contains(&indicator) {
                    indicators.retain(|i| i != &indicator);
                } else {
                    indicators.push(indicator);
                }

                chart.toggle_indicator(indicator);
            }
            PaneContent::Footprint(chart, indicators) => {
                let indicator = match indicator_str.as_str() {
                    "Volume" => FootprintIndicator::Volume,
                    "Open Interest" => FootprintIndicator::OpenInterest,
                    _ => {
                        log::error!("indicator not found: {}", indicator_str);
                        return;
                    }
                };

                if indicators.contains(&indicator) {
                    indicators.retain(|i| i != &indicator);
                } else {
                    indicators.push(indicator);
                }

                chart.toggle_indicator(indicator);
            }
            PaneContent::Candlestick(chart, indicators) => {
                let indicator = match indicator_str.as_str() {
                    "Volume" => CandlestickIndicator::Volume,
                    "Open Interest" => CandlestickIndicator::OpenInterest,
                    _ => {
                        log::error!("indicator not found: {}", indicator_str);
                        return;
                    }
                };

                if indicators.contains(&indicator) {
                    indicators.retain(|i| i != &indicator);
                } else {
                    indicators.push(indicator);
                }

                chart.toggle_indicator(indicator);
            }
            _ => {}
        }
    }

    pub fn change_visual_config(&mut self, config: VisualConfig) {
        match (self, config) {
            (PaneContent::Heatmap(chart, _), VisualConfig::Heatmap(cfg)) => {
                chart.set_visual_config(cfg);
            }
            (PaneContent::TimeAndSales(panel), VisualConfig::TimeAndSales(cfg)) => {
                panel.set_config(cfg);
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct PaneSettings {
    pub ticker_info: Option<TickerInfo>,
    pub tick_multiply: Option<TickMultiplier>,
    pub visual_config: Option<VisualConfig>,
    pub selected_basis: Option<ChartBasis>,
}
