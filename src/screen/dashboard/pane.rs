use iced::{
    alignment::{Horizontal, Vertical}, padding, widget::{
        button, center, column, container, pane_grid, row, scrollable, text, tooltip, Container, Slider
    }, Alignment, Element, Length, Renderer, Task, Theme
};
use serde::{Deserialize, Serialize};

use crate::{
    charts::{
        self, candlestick::CandlestickChart, footprint::FootprintChart, heatmap::HeatmapChart, 
        indicators::{CandlestickIndicator, FootprintIndicator, HeatmapIndicator, Indicator}, 
        timeandsales::TimeAndSales
    },
    data_providers::{format_with_commas, Exchange, Kline, MarketType, OpenInterest, TickMultiplier, Ticker, TickerInfo, Timeframe},
    screen::{
        self, create_button, create_notis_column, modal::{pane_menu, pane_notification}, DashboardError, UserTimezone
    },
    style::{self, get_icon_text, Icon},
    window::{self, Window},
    StreamType,
};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
pub enum PaneModal {
    StreamModifier,
    Settings,
    Indicators,
    None,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub enum Axis {
    Horizontal,
    Vertical,
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
    TimeframeSelected(Timeframe, pane_grid::Pane),
    ToggleModal(pane_grid::Pane, PaneModal),
    InitPaneContent(window::Id, String, Option<pane_grid::Pane>, Vec<StreamType>),
    ReplacePane(pane_grid::Pane),
    ChartUserUpdate(pane_grid::Pane, charts::Message),
    SliderChanged(pane_grid::Pane, f32, bool),
    ToggleIndicator(pane_grid::Pane, String),
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

    /// sets the tick size. returns the tick size with the multiplier applied
    pub fn set_tick_size(&mut self, multiplier: TickMultiplier, min_tick_size: f32) -> f32 {
        self.settings.tick_multiply = Some(multiplier);
        self.settings.min_tick_size = Some(min_tick_size);

        multiplier.multiply_with_min_tick_size(min_tick_size)
    }

    /// gets the timeframe if exists, otherwise sets timeframe w given
    pub fn set_timeframe(&mut self, timeframe: Timeframe) -> Timeframe {
        if self.settings.selected_timeframe.is_none() {
            self.settings.selected_timeframe = Some(timeframe);
        }

        timeframe
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
        ticker: Ticker,
        pane: pane_grid::Pane,
        window: window::Id,
    ) -> Task<Message> {
        let streams = match content {
            "heatmap" | "time&sales" => {
                vec![StreamType::DepthAndTrades { exchange, ticker }]
            }
            "footprint" => {
                let timeframe = self
                    .settings
                    .selected_timeframe
                    .unwrap_or(Timeframe::M15);

                vec![
                    StreamType::DepthAndTrades { exchange, ticker },
                    StreamType::Kline {
                        exchange,
                        ticker,
                        timeframe,
                    },
                ]
            }
            "candlestick" => {
                let timeframe = self
                    .settings
                    .selected_timeframe
                    .unwrap_or(Timeframe::M5);

                vec![StreamType::Kline {
                    exchange,
                    ticker,
                    timeframe,
                }]
            }
            _ => vec![],
        };

        self.stream = streams.clone();

        Task::done(Message::InitPaneContent(
            window,
            content.to_string(),
            Some(pane),
            streams,
        ))
    }

    pub fn set_content(
        &mut self, 
        ticker_info: TickerInfo, 
        content_str: &str, 
        timezone: UserTimezone
    ) -> Result<(), DashboardError> {
        self.content = match content_str {
            "heatmap" => {
                let tick_size = self.set_tick_size(
                    TickMultiplier(10),
                    ticker_info.tick_size,
                );

                PaneContent::Heatmap(
                    HeatmapChart::new(
                        tick_size,
                        100,
                        timezone,
                    ),
                    vec![],
                )
            }
            "footprint" => {
                let tick_size = self.set_tick_size(
                    TickMultiplier(50),
                    ticker_info.tick_size,
                );
                let timeframe = self.set_timeframe(Timeframe::M15);
                PaneContent::Footprint(
                    FootprintChart::new(
                        timeframe,
                        tick_size,
                        vec![],
                        vec![],
                        timezone,
                    ),
                    vec![
                        FootprintIndicator::Volume,
                        FootprintIndicator::OpenInterest,
                    ],
                )
            }
            "candlestick" => {
                let tick_size = self.set_tick_size(
                    TickMultiplier(1),
                    ticker_info.tick_size,
                );
                let timeframe = self.set_timeframe(Timeframe::M5);
                PaneContent::Candlestick(
                    CandlestickChart::new(
                        vec![],
                        timeframe,
                        tick_size,
                        timezone,
                    ),
                    vec![
                        CandlestickIndicator::Volume,
                        CandlestickIndicator::OpenInterest,
                    ],
                )
            }
            "time&sales" => PaneContent::TimeAndSales(TimeAndSales::new()),
            _ => {
                log::error!("content not found: {}", content_str);
                return Err(DashboardError::PaneSet("content not found: ".to_string() + content_str));
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
        timezone: UserTimezone,
    ) {
        match &mut self.content {
            PaneContent::Candlestick(chart, _) => {
                if let Some(id) = req_id {
                    chart.insert_new_klines(id, klines);
                } else {
                    let tick_size = chart.get_tick_size();

                    *chart = CandlestickChart::new(klines.clone(), timeframe, tick_size, timezone);
                }
            }
            PaneContent::Footprint(chart, _) => {
                if let Some(id) = req_id {
                    chart.insert_new_klines(id, klines);
                } else {
                    let (raw_trades, tick_size) = (chart.get_raw_trades(), chart.get_tick_size());

                    *chart = FootprintChart::new(
                        timeframe,
                        tick_size,
                        klines.clone(),
                        raw_trades,
                        timezone,
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
                        Exchange::BinanceFutures | Exchange::BinanceSpot => get_icon_text(Icon::BinanceLogo, 14),
                        Exchange::BybitLinear | Exchange::BybitSpot => get_icon_text(Icon::BybitLogo, 14),
                    },
                    text({
                        if market == MarketType::LinearPerps {
                            ticker_str + " PERP"
                        } else {
                            ticker_str
                        }
                    }).size(14),
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
                            .unwrap_or(TickMultiplier(1))
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
                        self.settings.selected_timeframe.unwrap_or(Timeframe::M1),
                        self.settings.tick_multiply.unwrap_or(TickMultiplier(1)),
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
                            .selected_timeframe
                            .unwrap_or(Timeframe::M1)
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
            PaneContent::Starter => 
                center(text("select a ticker to start").size(16)).into(),
            PaneContent::Heatmap(content, indicators) => 
                view_chart(id, self, content, notifications, indicators),
            PaneContent::Footprint(content, indicators) => 
                view_chart(id, self, content, notifications, indicators),
            PaneContent::Candlestick(content, indicators) => 
                view_chart(id, self, content, notifications, indicators),
            PaneContent::TimeAndSales(content) => 
                view_panel(id, self, content, notifications),
        })
        .style(move |theme| style::pane_primary(theme, is_focused));

        let title_bar = pane_grid::TitleBar::new(stream_info_element)
            .controls(view_controls(
                id,
                panes,
                maximized,
                window != main_window.id,
                is_chart,
            ))
            .style(style::title_bar);

        content.title_bar(title_bar)
    }

    pub fn matches_stream(&self, stream_type: &StreamType) -> bool {
        self.stream.iter().any(|stream| stream == stream_type)
    }
}

trait ChartView {
    fn view<'a, I: Indicator>(
        &'a self, 
        pane: pane_grid::Pane, 
        state: &PaneState, 
        indicators: &'a [I],
    ) -> Element<Message>;
}

trait PanelView {
    fn view(&self, pane: pane_grid::Pane, state: &PaneState) -> Element<Message>;
}

impl ChartView for HeatmapChart {
    fn view<'a, I: Indicator>(
        &'a self, 
        pane: pane_grid::Pane, 
        state: &PaneState, 
        indicators: &'a [I],
    ) -> Element<Message> {
        let underlay = self
            .view(indicators)
            .map(move |message| Message::ChartUserUpdate(pane, message));

        match state.modal {
            PaneModal::Settings => {
                let (trade_size_filter, order_size_filter) = self.get_size_filters();
                pane_menu(
                    underlay,
                    size_filter_view(Some(trade_size_filter), Some(order_size_filter), pane),
                    Message::ToggleModal(pane, PaneModal::None),
                    padding::right(12).left(12),
                    Alignment::End,
                )
            }
            PaneModal::StreamModifier => pane_menu(
                underlay,
                stream_modifier_view(
                    pane,
                    state.settings.tick_multiply,
                    None,
                ),
                Message::ToggleModal(pane, PaneModal::None),
                padding::left(36),
                Alignment::Start,
            ),
            PaneModal::Indicators => pane_menu(
                underlay,
                indicators_view::<I>(pane, indicators),
                Message::ToggleModal(pane, PaneModal::None),
                padding::right(12).left(12),
                Alignment::End,
            ),
            _ => underlay,
        }
    }
}

impl ChartView for FootprintChart {
    fn view<'a, I: Indicator>(
        &'a self, 
        pane: pane_grid::Pane, 
        state: &PaneState, 
        indicators: &'a [I],
    ) -> Element<Message> {
        let underlay = self
            .view(indicators)
            .map(move |message| Message::ChartUserUpdate(pane, message));

        match state.modal {
            PaneModal::StreamModifier => pane_menu(
                underlay,
                stream_modifier_view(
                    pane,
                    state.settings.tick_multiply,
                    state.settings.selected_timeframe,
                ),
                Message::ToggleModal(pane, PaneModal::None),
                padding::left(36),
                Alignment::Start,
            ),
            PaneModal::Indicators => pane_menu(
                underlay,
                indicators_view::<I>(pane, indicators),
                Message::ToggleModal(pane, PaneModal::None),
                padding::right(12).left(12),
                Alignment::End,
            ),
            _ => underlay,
        }
    }
}

impl ChartView for CandlestickChart {
    fn view<'a, I: Indicator>(
        &'a self,
        pane: pane_grid::Pane, 
        state: &PaneState, 
        indicators: &'a [I],
    ) -> Element<Message> {
        let underlay = self
            .view(indicators)
            .map(move |message| Message::ChartUserUpdate(pane, message));

        match state.modal {
            PaneModal::StreamModifier => pane_menu(
                underlay,
                stream_modifier_view(
                    pane,
                    None,
                    state.settings.selected_timeframe,
                ),
                Message::ToggleModal(pane, PaneModal::None),
                padding::left(36),
                Alignment::Start,
            ),
            PaneModal::Indicators => pane_menu(
                underlay,
                indicators_view::<I>(pane, indicators),
                Message::ToggleModal(pane, PaneModal::None),
                padding::right(12).left(12),
                Alignment::End,
            ),
            _ => underlay,
        }
    }
}

impl PanelView for TimeAndSales {
    fn view(
        &self, 
        pane: pane_grid::Pane, 
        state: &PaneState, 
    ) -> Element<Message> {
        let underlay = self.view();

        match state.modal {
            PaneModal::Settings => {
                let trade_size_filter = self.get_size_filter();
                pane_menu(
                    underlay,
                    size_filter_view(Some(trade_size_filter), None, pane),
                    Message::ToggleModal(pane, PaneModal::None),
                    padding::right(12).left(12),
                    Alignment::End,
                )
            }
            _ => underlay,
        }
    }
}

fn indicators_view<I: Indicator> (
    pane: pane_grid::Pane,
    selected: &[I]
) -> Element<Message> {
    let mut content_row = column![
        container(
            text("Indicators").size(14)
        )
        .padding(padding::bottom(8)),
    ]
    .spacing(4);

    for indicator in I::get_available() {
        content_row = content_row.push(
            if selected.contains(indicator) {
                button(text(indicator.to_string()))
                    .on_press(Message::ToggleIndicator(pane, indicator.to_string()))
                    .width(Length::Fill)
                    .style(move |theme, status| style::button_transparent(theme, status, true))
            } else {
                button(text(indicator.to_string()))
                    .on_press(Message::ToggleIndicator(pane, indicator.to_string()))
                    .width(Length::Fill)
                    .style(move |theme, status| style::button_transparent(theme, status, false))
            }
        );
    }

    container(content_row)
        .max_width(200)
        .padding(16)
        .style(style::chart_modal)
        .into()
}

fn size_filter_view<'a>(
    trade_size_filter: Option<f32>,
    order_size_filter: Option<f32>,
    pane: pane_grid::Pane,
) -> Element<'a, Message> {
    container(
        column![
            text("Size Filtering").size(14),
            if let Some(trade_filter) = trade_size_filter {
                container(
                    row![
                        text("Trade size"),
                        column![
                            Slider::new(0.0..=50000.0, trade_filter, move |value| {
                                Message::SliderChanged(pane, value, true)
                            })
                            .step(500.0),
                            text(format!("${}", format_with_commas(trade_filter))).size(13),
                        ]
                        .spacing(2)
                        .align_x(Alignment::Center),
                    ]
                    .align_y(Alignment::Center)
                    .spacing(8)
                    .padding(8),
                )
                .style(style::modal_container)
            } else {
                container(row![])
            },
            if let Some(order_filter) = order_size_filter {
                container(
                    row![
                        text("Order size"),
                        column![
                            Slider::new(0.0..=500_000.0, order_filter, move |value| {
                                Message::SliderChanged(pane, value, false)
                            })
                            .step(1000.0),
                            text(format!("${}", format_with_commas(order_filter))).size(13),
                        ]
                        .spacing(2)
                        .align_x(Alignment::Center),
                    ]
                    .align_y(Alignment::Center)
                    .spacing(8)
                    .padding(8),
                )
                .style(style::modal_container)
            } else {
                container(row![])
            },
        ]
        .spacing(20)
        .padding(16)
        .align_x(Alignment::Center),
    )
    .width(Length::Shrink)
    .padding(16)
    .max_width(500)
    .style(style::chart_modal)
    .into()
}

fn stream_modifier_view<'a>(
    pane: pane_grid::Pane,
    selected_ticksize: Option<TickMultiplier>,
    selected_timeframe: Option<Timeframe>,
) -> iced::Element<'a, Message> {
    let create_button = |content: String, msg: Option<Message>| {
        let btn = button(text(content))
            .width(Length::Fill)
            .style(move |theme, status| style::button_transparent(theme, status, false));
            
        if let Some(msg) = msg {
            btn.on_press(msg)
        } else {
            btn
        }
    };

    let mut content_row = row![]
        .align_y(Vertical::Center)
        .spacing(16);

    let mut timeframes_column = column![]
        .padding(4)
        .align_x(Horizontal::Center);

    if selected_timeframe.is_some() {
        timeframes_column =
            timeframes_column.push(container(text("Timeframe"))
                .padding(padding::bottom(8)));

        for timeframe in &Timeframe::ALL {
            let msg = if selected_timeframe == Some(*timeframe) {
                None
            } else {
                Some(Message::TimeframeSelected(*timeframe, pane))
            };
            timeframes_column = timeframes_column.push(
                create_button(timeframe.to_string(), msg)
            );
        }

        content_row = content_row.push(timeframes_column);
    }

    let mut ticksizes_column = column![]
        .padding(4)
        .align_x(Horizontal::Center);

    if selected_ticksize.is_some() {
        ticksizes_column =
            ticksizes_column.push(container(text("Ticksize Mltp."))
                .padding(padding::bottom(8)));

        for ticksize in &TickMultiplier::ALL {
            let msg = if selected_ticksize == Some(*ticksize) {
                None
            } else {
                Some(Message::TicksizeSelected(*ticksize, pane))
            };
            ticksizes_column = ticksizes_column.push(
                create_button(ticksize.to_string(), msg)
            );
        }

        content_row = content_row.push(ticksizes_column);
    }

    container(
        scrollable::Scrollable::with_direction(
            content_row, 
            scrollable::Direction::Vertical(
                scrollable::Scrollbar::new().width(4).scroller_width(4),
            )
        ))
        .padding(16)
        .max_width(
            if selected_ticksize.is_some() && selected_timeframe.is_some() {
                240
            } else {
                120
            },
        )
        .style(style::chart_modal)
        .into()
}

fn view_panel<'a, C: PanelView>(
    pane: pane_grid::Pane,
    state: &'a PaneState,
    content: &'a C,
    notifications: Option<&'a Vec<screen::Notification>>,
) -> Element<'a, Message> {
    let base: Container<'_, Message> = center(content.view(pane, state));

    if let Some(notifications) = notifications {
        if !notifications.is_empty() {
            pane_notification(base, create_notis_column(notifications))
        } else {
            base.into()
        }
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
) -> Element<'a, Message> {
    let base: Container<'_, Message> = center(content.view(pane, state, indicators));

    if let Some(notifications) = notifications {
        if !notifications.is_empty() {
            pane_notification(base, create_notis_column(notifications))
        } else {
            base.into()
        }
    } else {
        base.into()
    }
}

fn view_controls<'a>(
    pane: pane_grid::Pane,
    total_panes: usize,
    is_maximized: bool,
    is_popout: bool,
    is_chart: bool,
) -> Element<'a, Message> {
    let button_style = |theme: &Theme, status: button::Status| 
        style::button_transparent(theme, status, false);
    let tooltip_pos = tooltip::Position::Bottom;

    let mut buttons = row![
        create_button(
            get_icon_text(Icon::Cog, 12),
            Message::ToggleModal(pane, PaneModal::Settings),
            None,
            tooltip_pos,
            button_style,
        )
    ];

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
    Heatmap(HeatmapChart, Vec<HeatmapIndicator>),
    Footprint(FootprintChart, Vec<FootprintIndicator>),
    Candlestick(CandlestickChart, Vec<CandlestickIndicator>),
    TimeAndSales(TimeAndSales),
    Starter,
}

impl PaneContent {
    pub fn change_timezone(&mut self, timezone: UserTimezone) {
        match self {
            PaneContent::Heatmap(chart, _) => chart.change_timezone(timezone),
            PaneContent::Footprint(chart, _) => chart.change_timezone(timezone),
            PaneContent::Candlestick(chart, _) => chart.change_timezone(timezone),
            _ => {}
        }
    }

    pub fn toggle_indicator(&mut self, indicator_str: String) {
        match self {
            PaneContent::Footprint(_, indicators) => {
                let indicator = match indicator_str.as_str() {
                    "Volume" => FootprintIndicator::Volume,
                    "Open Interest" => FootprintIndicator::OpenInterest,
                    _ => {
                        log::error!("indicator not found: {}", indicator_str);
                        return
                    },
                };

                if indicators.contains(&indicator) {
                    indicators.retain(|i| i != &indicator);
                } else {
                    indicators.push(indicator);
                }
            }
            PaneContent::Candlestick(_, indicators) => {
                let indicator = match indicator_str.as_str() {
                    "Volume" => CandlestickIndicator::Volume,
                    "Open Interest" => CandlestickIndicator::OpenInterest,
                    _ => {
                        log::error!("indicator not found: {}", indicator_str);
                        return
                    },
                };

                if indicators.contains(&indicator) {
                    indicators.retain(|i| i != &indicator);
                } else {
                    indicators.push(indicator);
                }
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default)]
pub struct PaneSettings {
    pub min_tick_size: Option<f32>,
    pub trade_size_filter: Option<f32>,
    pub tick_multiply: Option<TickMultiplier>,
    pub selected_timeframe: Option<Timeframe>,
}
