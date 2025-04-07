use crate::{
    StreamType,
    charts::{
        self, candlestick::CandlestickChart, config, footprint::FootprintChart,
        heatmap::HeatmapChart, timeandsales::TimeAndSales,
    },
    screen::{DashboardError, create_button},
    style::{self, Icon, get_icon_text},
    widget::{self, notification::Toast, pane_modal},
    window::{self, Window},
};
use data::{
    UserTimezone,
    aggr::ticks::TickCount,
    chart::{
        Basis, ChartLayout, VisualConfig,
        indicators::{CandlestickIndicator, FootprintIndicator, HeatmapIndicator, Indicator},
    },
    layout::pane::PaneSettings,
};
use exchange::{
    Kline, OpenInterest, TickMultiplier, Ticker, TickerInfo, Timeframe,
    adapter::{Exchange, MarketType},
};
use iced::{
    Alignment, Element, Length, Renderer, Task, Theme,
    alignment::{Horizontal, Vertical},
    padding,
    widget::{
        button, center, column, container, horizontal_space, pane_grid, row, scrollable, text,
        tooltip,
    },
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InfoType {
    FetchingKlines,
    FetchingTrades(usize),
    FetchingOI,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Ready,
    Loading(InfoType),
    Stale(String),
}

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
    BasisSelected(Basis, pane_grid::Pane),
    ToggleModal(pane_grid::Pane, PaneModal),
    InitPaneContent(String, Option<pane_grid::Pane>, Vec<StreamType>, TickerInfo),
    ReplacePane(pane_grid::Pane),
    ChartUserUpdate(pane_grid::Pane, charts::Message),
    VisualConfigChanged(Option<pane_grid::Pane>, VisualConfig),
    ToggleIndicator(pane_grid::Pane, String),
    Popout,
    Merge,
    DeleteNotification(pane_grid::Pane, usize),
}

pub struct PaneState {
    pub id: uuid::Uuid,
    pub modal: PaneModal,
    pub content: PaneContent,
    pub settings: PaneSettings,
    pub notifications: Vec<Toast>,
    pub streams: Vec<StreamType>,
    pub status: Status,
}

impl PaneState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_config(
        content: PaneContent,
        streams: Vec<StreamType>,
        settings: PaneSettings,
    ) -> Self {
        Self {
            streams,
            content,
            settings,
            ..Default::default()
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
        for stream in &self.streams {
            match stream {
                StreamType::DepthAndTrades { exchange, ticker }
                | StreamType::Kline {
                    exchange, ticker, ..
                } => {
                    return Some((*exchange, *ticker));
                }
                StreamType::None => {}
            }
        }
        None
    }

    pub fn init_content_task(
        &mut self,
        content: &str,
        exchange: Exchange,
        ticker_info: TickerInfo,
        pane: pane_grid::Pane,
    ) -> Task<Message> {
        let streams = match content {
            "heatmap" | "time&sales" => {
                vec![StreamType::DepthAndTrades {
                    exchange,
                    ticker: ticker_info.ticker,
                }]
            }
            "footprint" => {
                let basis = self
                    .settings
                    .selected_basis
                    .unwrap_or(Basis::Time(Timeframe::M5.into()));

                match basis {
                    Basis::Time(interval) => {
                        vec![
                            StreamType::DepthAndTrades {
                                exchange,
                                ticker: ticker_info.ticker,
                            },
                            StreamType::Kline {
                                exchange,
                                ticker: ticker_info.ticker,
                                timeframe: interval.into(),
                            },
                        ]
                    }
                    Basis::Tick(_) => {
                        vec![StreamType::DepthAndTrades {
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
                    .unwrap_or(Basis::Time(Timeframe::M15.into()));

                match basis {
                    Basis::Time(interval) => {
                        vec![StreamType::Kline {
                            exchange,
                            ticker: ticker_info.ticker,
                            timeframe: interval.into(),
                        }]
                    }
                    Basis::Tick(_) => {
                        vec![StreamType::DepthAndTrades {
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
                let layout = existing_layout.unwrap_or(ChartLayout {
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
                    .unwrap_or(Basis::Time(Timeframe::M5.into()));

                let enabled_indicators = match existing_indicators {
                    Some(ExistingIndicators::Footprint(indicators)) => indicators,
                    _ => vec![FootprintIndicator::Volume, FootprintIndicator::OpenInterest],
                };

                let layout = existing_layout.unwrap_or(ChartLayout {
                    crosshair: true,
                    indicators_split: Some(0.8),
                });

                PaneContent::Footprint(
                    FootprintChart::new(
                        layout,
                        basis,
                        tick_size,
                        &[],
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
                    .unwrap_or(Basis::Time(Timeframe::M15.into()));

                let enabled_indicators = match existing_indicators {
                    Some(ExistingIndicators::Candlestick(indicators)) => indicators,
                    _ => vec![
                        CandlestickIndicator::Volume,
                        CandlestickIndicator::OpenInterest,
                    ],
                };

                let layout = existing_layout.unwrap_or(ChartLayout {
                    crosshair: true,
                    indicators_split: Some(0.8),
                });

                PaneContent::Candlestick(
                    CandlestickChart::new(
                        layout,
                        basis,
                        &[],
                        vec![],
                        tick_size,
                        &enabled_indicators,
                        Some(ticker_info),
                    ),
                    enabled_indicators,
                )
            }
            "time&sales" => {
                let _ = self.set_tickers_info(None, ticker_info);

                let config = self
                    .settings
                    .visual_config
                    .and_then(|cfg| cfg.time_and_sales());

                PaneContent::TimeAndSales(TimeAndSales::new(config, Some(ticker_info)))
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

    pub fn insert_oi_vec(&mut self, req_id: Option<uuid::Uuid>, oi: &[OpenInterest]) {
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
                        Basis::Time(timeframe.into()),
                        klines,
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
                        Basis::Time(timeframe.into()),
                        tick_size,
                        klines,
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
        timezone: UserTimezone,
    ) -> pane_grid::Content<'a, Message, Theme, Renderer> {
        let mut stream_info_element = row![]
            .padding(padding::left(8))
            .align_y(Vertical::Center)
            .spacing(8)
            .height(Length::Fixed(32.0));

        if let Some((exchange, ticker)) = self.get_ticker_exchange() {
            let exchange_info = match exchange {
                Exchange::BinanceSpot | Exchange::BinanceLinear | Exchange::BinanceInverse => {
                    get_icon_text(Icon::BinanceLogo, 14)
                }
                Exchange::BybitSpot | Exchange::BybitLinear | Exchange::BybitInverse => {
                    get_icon_text(Icon::BybitLogo, 14)
                }
            };

            let ticker_str = {
                let symbol = ticker.display_symbol_and_type().0;
                match ticker.get_market_type() {
                    MarketType::Spot => symbol,
                    MarketType::LinearPerps | MarketType::InversePerps => symbol + " PERP",
                }
            };

            stream_info_element = stream_info_element.push(
                row![exchange_info, text(ticker_str).size(14),]
                    .align_y(Vertical::Center)
                    .spacing(4),
            );
        }

        let is_stream_modifier = self.modal == PaneModal::StreamModifier;

        match self.content {
            PaneContent::Starter => {}
            PaneContent::Heatmap(_, _) => {
                stream_info_element = stream_info_element.push(
                    button(text(
                        self.settings
                            .tick_multiply
                            .unwrap_or(TickMultiplier(5))
                            .to_string(),
                    ))
                    .style(move |theme, status| {
                        style::button::modifier(theme, status, !is_stream_modifier)
                    })
                    .on_press(Message::ToggleModal(id, PaneModal::StreamModifier)),
                );
            }
            PaneContent::Footprint(_, _) => {
                stream_info_element = stream_info_element.push(
                    button(text(format!(
                        "{} - {}",
                        self.settings
                            .selected_basis
                            .unwrap_or(Basis::Time(Timeframe::M5.into())),
                        self.settings.tick_multiply.unwrap_or(TickMultiplier(10)),
                    )))
                    .style(move |theme, status| {
                        style::button::modifier(theme, status, !is_stream_modifier)
                    })
                    .on_press(Message::ToggleModal(id, PaneModal::StreamModifier)),
                );
            }
            PaneContent::Candlestick(_, _) => {
                stream_info_element = stream_info_element.push(
                    button(text(
                        self.settings
                            .selected_basis
                            .unwrap_or(Basis::Time(Timeframe::M15.into()))
                            .to_string(),
                    ))
                    .style(move |theme, status| {
                        style::button::modifier(theme, status, !is_stream_modifier)
                    })
                    .on_press(Message::ToggleModal(id, PaneModal::StreamModifier)),
                );
            }
            PaneContent::TimeAndSales(_) => {}
        }

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

        let content = pane_grid::Content::new(match &self.content {
            PaneContent::Starter => center(text("select a ticker to start").size(16)).into(),
            PaneContent::Heatmap(content, indicators) => {
                view_chart(id, self, content, indicators, timezone)
            }
            PaneContent::Footprint(content, indicators) => {
                view_chart(id, self, content, indicators, timezone)
            }
            PaneContent::Candlestick(content, indicators) => {
                view_chart(id, self, content, indicators, timezone)
            }
            PaneContent::TimeAndSales(content) => view_panel(id, self, content, timezone),
        })
        .style(move |theme| style::pane_background(theme, is_focused));

        let title_bar = pane_grid::TitleBar::new(stream_info_element)
            .controls(self.view_controls(id, panes, maximized, window != main_window.id))
            .style(style::pane_title_bar);

        content.title_bar(if self.modal == PaneModal::None {
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
        let modal_btn_style = |modal: PaneModal| {
            let is_active = self.modal == modal;
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

        if !matches!(&self.content, PaneContent::Starter) {
            buttons = buttons.push(create_button(
                get_icon_text(Icon::Cog, 12),
                Message::ToggleModal(pane, PaneModal::Settings),
                None,
                tooltip_pos,
                modal_btn_style(PaneModal::Settings),
            ))
        }

        if matches!(
            &self.content,
            PaneContent::Heatmap(_, _)
                | PaneContent::Footprint(_, _)
                | PaneContent::Candlestick(_, _)
        ) {
            buttons = buttons.push(create_button(
                get_icon_text(Icon::ChartOutline, 12),
                Message::ToggleModal(pane, PaneModal::Indicators),
                Some("Indicators"),
                tooltip_pos,
                modal_btn_style(PaneModal::Indicators),
            ));
        }

        if is_popout {
            buttons = buttons.push(create_button(
                get_icon_text(Icon::Popout, 12),
                Message::Merge,
                Some("Merge"),
                tooltip_pos,
                control_btn_style(is_popout),
            ));
        } else if total_panes > 1 {
            buttons = buttons.push(create_button(
                get_icon_text(Icon::Popout, 12),
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

            buttons = buttons.push(create_button(
                get_icon_text(resize_icon, 12),
                message,
                None,
                tooltip_pos,
                control_btn_style(is_maximized),
            ));

            buttons = buttons.push(create_button(
                get_icon_text(Icon::Close, 12),
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

    pub fn matches_stream(&self, stream: &StreamType) -> bool {
        self.streams.iter().any(|existing| existing == stream)
    }
}

impl Default for PaneState {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            modal: PaneModal::None,
            content: PaneContent::Starter,
            settings: PaneSettings::default(),
            streams: vec![],
            notifications: vec![],
            status: Status::Ready,
        }
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
        timezone: UserTimezone,
    ) -> Element<'a, Message>;
}

#[derive(Debug, Clone, Copy)]
enum StreamModifier {
    CandlestickChart(Basis),
    FootprintChart(Basis, TickMultiplier),
    HeatmapChart(TickMultiplier),
}

fn handle_chart_view<'a, F>(
    base: Element<'a, Message>,
    state: &'a PaneState,
    pane: pane_grid::Pane,
    indicators: &'a [impl Indicator],
    settings_view: F,
    stream_modifier: StreamModifier,
) -> Element<'a, Message>
where
    F: FnOnce() -> Element<'a, Message>,
{
    let base = widget::notification::Manager::new(
        base,
        &state.notifications,
        Alignment::End,
        move |msg| Message::DeleteNotification(pane, msg),
    )
    .into();

    match state.modal {
        PaneModal::StreamModifier => pane_modal(
            base,
            stream_modifier_view(pane, stream_modifier),
            Message::ToggleModal(pane, PaneModal::None),
            padding::left(36),
            Alignment::Start,
        ),
        PaneModal::Indicators => pane_modal(
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
        PaneModal::Settings => pane_modal(
            base,
            settings_view(),
            Message::ToggleModal(pane, PaneModal::None),
            padding::right(12).left(12),
            Alignment::End,
        ),
        PaneModal::None => base,
    }
}

impl ChartView for HeatmapChart {
    fn view<'a, I: Indicator>(
        &'a self,
        pane: pane_grid::Pane,
        state: &'a PaneState,
        indicators: &'a [I],
        timezone: UserTimezone,
    ) -> Element<'a, Message> {
        let base = self
            .view(indicators, timezone)
            .map(move |message| Message::ChartUserUpdate(pane, message));

        let settings_view = || config::heatmap_cfg_view(self.get_visual_config(), pane);

        handle_chart_view(
            base,
            state,
            pane,
            indicators,
            settings_view,
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
        timezone: UserTimezone,
    ) -> Element<'a, Message> {
        let base = self
            .view(indicators, timezone)
            .map(move |message| Message::ChartUserUpdate(pane, message));

        let settings_view = || blank_settings_view();

        handle_chart_view(
            base,
            state,
            pane,
            indicators,
            settings_view,
            StreamModifier::FootprintChart(
                state
                    .settings
                    .selected_basis
                    .unwrap_or(Basis::Time(Timeframe::M5.into())),
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
        timezone: UserTimezone,
    ) -> Element<'a, Message> {
        let base = self
            .view(indicators, timezone)
            .map(move |message| Message::ChartUserUpdate(pane, message));

        let settings_view = || blank_settings_view();

        handle_chart_view(
            base,
            state,
            pane,
            indicators,
            settings_view,
            StreamModifier::CandlestickChart(
                state
                    .settings
                    .selected_basis
                    .unwrap_or(Basis::Time(Timeframe::M15.into())),
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
        timezone: UserTimezone,
    ) -> Element<Message>;
}

impl PanelView for TimeAndSales {
    fn view(
        &self,
        pane: pane_grid::Pane,
        state: &PaneState,
        timezone: UserTimezone,
    ) -> Element<Message> {
        let underlay = self.view(timezone);

        let settings_view = config::timesales_cfg_view(self.get_config(), pane);

        match state.modal {
            PaneModal::Settings => pane_modal(
                underlay,
                settings_view,
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
            button(row![
                text(indicator.to_string()),
                horizontal_space(),
                container(get_icon_text(Icon::Checkmark, 12)),
            ])
            .on_press(Message::ToggleIndicator(pane, indicator.to_string()))
            .width(Length::Fill)
            .style(move |theme, status| style::button::modifier(theme, status, true))
        } else {
            button(text(indicator.to_string()))
                .on_press(Message::ToggleIndicator(pane, indicator.to_string()))
                .width(Length::Fill)
                .style(move |theme, status| style::button::modifier(theme, status, false))
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
            .style(move |theme, status| style::button::transparent(theme, status, active));

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
            Basis::Time(selected_timeframe) => {
                timeframes_column = timeframes_column.push(
                    row![
                        create_button("Timeframe".to_string(), None, false,),
                        create_button(
                            "Ticks".to_string(),
                            Some(Message::BasisSelected(Basis::Tick(200), pane,)),
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
                        Some(Message::BasisSelected(
                            Basis::Time(u64::from(*timeframe)),
                            pane,
                        ))
                    };
                    timeframes_column =
                        timeframes_column.push(create_button(timeframe.to_string(), msg, false));
                }

                content_row =
                    content_row.push(container(timeframes_column).style(style::modal_container));
            }
            Basis::Tick(selected_tick) => {
                tick_basis_column = tick_basis_column.push(
                    row![
                        create_button(
                            "Timeframe".to_string(),
                            Some(Message::BasisSelected(
                                Basis::Time(Timeframe::M5.into()),
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
                        Some(Message::BasisSelected(
                            Basis::Tick(u64::from(*tick_count)),
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
    timezone: UserTimezone,
) -> Element<'a, Message> {
    let base = center(content.view(pane, state, timezone));

    widget::notification::Manager::new(base, &state.notifications, Alignment::End, move |idx| {
        Message::DeleteNotification(pane, idx)
    })
    .into()
}

fn view_chart<'a, C: ChartView, I: Indicator>(
    pane: pane_grid::Pane,
    state: &'a PaneState,
    content: &'a C,
    indicators: &'a [I],
    timezone: UserTimezone,
) -> Element<'a, Message> {
    content.view(pane, state, indicators, timezone)
}

pub enum PaneContent {
    Starter,
    Heatmap(HeatmapChart, Vec<HeatmapIndicator>),
    Footprint(FootprintChart, Vec<FootprintIndicator>),
    Candlestick(CandlestickChart, Vec<CandlestickIndicator>),
    TimeAndSales(TimeAndSales),
}

impl PaneContent {
    pub fn toggle_indicator(&mut self, indicator_str: &str) {
        match self {
            PaneContent::Heatmap(chart, indicators) => {
                let indicator = match indicator_str {
                    "Volume" => HeatmapIndicator::Volume,
                    "VPSR" => HeatmapIndicator::SessionVolumeProfile,
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
                let indicator = match indicator_str {
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
                let indicator = match indicator_str {
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
