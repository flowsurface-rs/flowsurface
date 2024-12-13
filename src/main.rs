#![windows_subsystem = "windows"]

mod charts;
mod data_providers;
mod layout;
mod logger;
mod screen;
mod style;
mod tickers_table;
mod tooltip;
mod window;

use tooltip::tooltip;
use screen::modal::dashboard_modal;
use layout::{SerializableDashboard, SerializablePane, Sidebar};

use futures::TryFutureExt;
use iced_futures::MaybeSend;
use style::{get_icon_text, Icon, ICON_BYTES};

use screen::{create_button, dashboard, handle_error, Notification, UserTimezone};
use screen::dashboard::{Dashboard, pane, PaneContent, PaneSettings, PaneState};
use data_providers::{
    binance, bybit, Exchange, MarketType, StreamType, TickMultiplier, Ticker, TickerInfo, TickerStats, Timeframe
};
use tickers_table::TickersTable;

use charts::footprint::FootprintChart;
use charts::heatmap::HeatmapChart;
use charts::candlestick::CandlestickChart;
use charts::timeandsales::TimeAndSales;
use window::{window_events, Window, WindowEvent};

use std::future::Future;
use std::{collections::HashMap, vec};

use iced::{
    widget::{button, pick_list, Space, column, container, row, text},
    padding, Alignment, Element, Length, Point, Size, Subscription, Task, Theme,
};
use iced::widget::{center, responsive};
use iced::widget::pane_grid::{self, Configuration};

fn main() {
    logger::setup(false, false).expect("Failed to initialize logger");

    let saved_state = match layout::read_from_file("dashboard_state.json") {
        Ok(state) => {
            let mut de_state = layout::SavedState {
                selected_theme: state.selected_theme,
                layouts: HashMap::new(),
                favorited_tickers: state.favorited_tickers,
                last_active_layout: state.last_active_layout,
                window_size: state.window_size,
                window_position: state.window_position,
                timezone: state.timezone,
                sidebar: state.sidebar,
            };

            fn configuration(pane: SerializablePane) -> Configuration<PaneState> {
                match pane {
                    SerializablePane::Split { axis, ratio, a, b } => Configuration::Split {
                        axis: match axis {
                            pane::Axis::Horizontal => pane_grid::Axis::Horizontal,
                            pane::Axis::Vertical => pane_grid::Axis::Vertical,
                        },
                        ratio,
                        a: Box::new(configuration(*a)),
                        b: Box::new(configuration(*b)),
                    },
                    SerializablePane::Starter => {
                        Configuration::Pane(PaneState::new(vec![], PaneSettings::default()))
                    }
                    SerializablePane::CandlestickChart {
                        stream_type,
                        settings,
                        indicators,
                    } => {
                        let tick_size = settings.tick_multiply
                            .unwrap_or(TickMultiplier(1))
                            .multiply_with_min_tick_size(
                                settings.min_tick_size
                                    .expect("No min tick size found, deleting dashboard_state.json probably fixes this")
                            );

                        let timeframe = settings.selected_timeframe.unwrap_or(Timeframe::M5);

                        Configuration::Pane(PaneState::from_config(
                            PaneContent::Candlestick(
                                CandlestickChart::new(
                                    vec![],
                                    timeframe,
                                    tick_size,
                                    UserTimezone::default(),
                                ),
                                indicators,
                            ),
                            stream_type,
                            settings,
                        ))
                    }
                    SerializablePane::FootprintChart {
                        stream_type,
                        settings,
                        indicators,
                    } => {
                        let tick_size = settings.tick_multiply
                            .unwrap_or(TickMultiplier(50))
                            .multiply_with_min_tick_size(
                                settings.min_tick_size
                                    .expect("No min tick size found, deleting dashboard_state.json probably fixes this")
                            );

                        let timeframe = settings.selected_timeframe.unwrap_or(Timeframe::M15);

                        Configuration::Pane(PaneState::from_config(
                            PaneContent::Footprint(
                                FootprintChart::new(
                                    timeframe,
                                    tick_size,
                                    vec![],
                                    vec![],
                                    UserTimezone::default(),
                                ),
                                indicators,
                            ),
                            stream_type,
                            settings,
                        ))
                    }
                    SerializablePane::HeatmapChart {
                        stream_type,
                        settings,
                        indicators,
                    } => {
                        let tick_size = settings.tick_multiply
                            .unwrap_or(TickMultiplier(10))
                            .multiply_with_min_tick_size(
                                settings.min_tick_size
                                    .expect("No min tick size found, deleting dashboard_state.json probably fixes this")
                            );

                        Configuration::Pane(PaneState::from_config(
                            PaneContent::Heatmap(
                                HeatmapChart::new(
                                    tick_size,
                                    100,
                                    UserTimezone::default(),
                                ),
                                indicators,
                            ),
                            stream_type,
                            settings,
                        ))
                    }
                    SerializablePane::TimeAndSales {
                        stream_type,
                        settings,
                    } => Configuration::Pane(PaneState::from_config(
                        PaneContent::TimeAndSales(TimeAndSales::new()),
                        stream_type,
                        settings,
                    )),
                }
            }

            for (id, dashboard) in &state.layouts {
                let mut popout_windows: Vec<(Configuration<PaneState>, (Point, Size))> = Vec::new();

                for (popout, pos, size) in &dashboard.popout {
                    let configuration = configuration(popout.clone());
                    popout_windows.push((
                        configuration,
                        (Point::new(pos.0, pos.1), Size::new(size.0, size.1)),
                    ));
                }

                let dashboard =
                    Dashboard::from_config(configuration(dashboard.pane.clone()), popout_windows);

                de_state.layouts.insert(*id, dashboard);
            }

            de_state
        }
        Err(e) => {
            log::error!(
                "Failed to load/find layout state: {}. Starting with a new layout.",
                e
            );

            layout::SavedState::default()
        }
    };

    let window_size = saved_state.window_size.unwrap_or((1600.0, 900.0));
    let window_position = saved_state.window_position;

    let window_settings = window::Settings {
        size: iced::Size::new(window_size.0, window_size.1),
        position: {
            if let Some(position) = window_position {
                iced::window::Position::Specific(Point {
                    x: position.0,
                    y: position.1,
                })
            } else {
                iced::window::Position::Centered
            }
        },
        platform_specific: iced::window::settings::PlatformSpecific {
            title_hidden: true,
            titlebar_transparent: true,
            fullsize_content_view: true,
        },
        exit_on_close_request: false,
        min_size: Some(iced::Size::new(800.0, 600.0)),
        ..Default::default()
    };

    let _ = iced::daemon("Flowsurface", State::update, State::view)
        .settings(iced::Settings {
            default_text_size: iced::Pixels(12.0),
            antialiasing: true,
            ..Default::default()
        })
        .theme(State::theme)
        .subscription(State::subscription)
        .font(ICON_BYTES)
        .run_with(move || State::new(saved_state, window_settings));
}

#[derive(thiserror::Error, Debug, Clone)]
enum InternalError {
    #[error("Fetch error: {0}")]
    Fetch(String),
}

#[derive(Debug, Clone, PartialEq)]
enum DashboardModal {
    Layout,
    Settings,
    None,
}

#[derive(Debug, Clone)]
enum Message {
    Notification(Notification),
    ErrorOccurred(InternalError),

    ToggleModal(DashboardModal),

    MarketWsEvent(Exchange, data_providers::Event),

    WindowEvent(WindowEvent),
    SaveAndExit(HashMap<window::Id, (Point, Size)>),

    ToggleLayoutLock,
    ResetCurrentLayout,
    LayoutSelected(layout::LayoutId),
    ThemeSelected(Theme),
    Dashboard(dashboard::Message),
    SetTickersInfo(Exchange, HashMap<Ticker, Option<TickerInfo>>),
    SetTimezone(UserTimezone),
    SidebarPosition(layout::Sidebar),

    TickersTable(tickers_table::Message),
    ToggleTickersDashboard,
    UpdateTickersTable(Exchange, HashMap<Ticker, TickerStats>),
    FetchAndUpdateTickersTable,

    LoadLayout(layout::LayoutId),
}

struct State {
    theme: Theme,
    layouts: HashMap<layout::LayoutId, Dashboard>,
    last_active_layout: layout::LayoutId,
    main_window: Window,
    active_modal: DashboardModal,
    sidebar_location: Sidebar,
    notification: Option<Notification>,
    ticker_info_map: HashMap<Exchange, HashMap<Ticker, Option<TickerInfo>>>,
    show_tickers_dashboard: bool,
    tickers_table: TickersTable,
}

#[allow(dead_code)]
impl State {
    fn new(
        saved_state: layout::SavedState,
        window_settings: window::Settings,
    ) -> (Self, Task<Message>) {
        let (main_window, open_main_window) = window::open(window_settings);

        let last_active_layout = saved_state.last_active_layout;

        let mut ticker_info_map = HashMap::new();
        let mut ticksizes_tasks = Vec::new();

        for exchange in &Exchange::ALL {
            ticker_info_map.insert(*exchange, HashMap::new());

            let fetch_ticksize = match exchange {
                Exchange::BinanceFutures => {
                    fetch_ticker_info(*exchange, binance::fetch_ticksize(MarketType::LinearPerps))
                }
                Exchange::BybitLinear => {
                    fetch_ticker_info(*exchange, bybit::fetch_ticksize(MarketType::LinearPerps))
                }
                Exchange::BinanceSpot => {
                    fetch_ticker_info(*exchange, binance::fetch_ticksize(MarketType::Spot))
                }
                Exchange::BybitSpot => {
                    fetch_ticker_info(*exchange, bybit::fetch_ticksize(MarketType::Spot))
                }
            };
            ticksizes_tasks.push(fetch_ticksize);
        }

        let bybit_tickers_fetch = fetch_ticker_prices(
            Exchange::BybitLinear,
            bybit::fetch_ticker_prices(MarketType::LinearPerps),
        );
        let binance_tickers_fetch = fetch_ticker_prices(
            Exchange::BinanceFutures,
            binance::fetch_ticker_prices(MarketType::LinearPerps),
        );
        let binance_spot_tickers_fetch = fetch_ticker_prices(
            Exchange::BinanceSpot,
            binance::fetch_ticker_prices(MarketType::Spot),
        );
        let bybit_spot_tickers_fetch = fetch_ticker_prices(
            Exchange::BybitSpot,
            bybit::fetch_ticker_prices(MarketType::Spot),
        );

        let batch_fetch_tasks = Task::batch(vec![
            bybit_tickers_fetch,
            binance_tickers_fetch,
            binance_spot_tickers_fetch,
            bybit_spot_tickers_fetch,
            Task::batch(ticksizes_tasks),
        ]);

        (
            Self {
                theme: saved_state.selected_theme.theme,
                layouts: saved_state.layouts,
                last_active_layout,
                main_window: Window::new(main_window),
                active_modal: DashboardModal::None,
                notification: None,
                ticker_info_map,
                show_tickers_dashboard: false,
                sidebar_location: saved_state.sidebar,
                tickers_table: TickersTable::new(saved_state.favorited_tickers),
            },
            open_main_window
                .then(|_| Task::none())
                .chain(Task::batch(vec![
                    Task::done(Message::LoadLayout(last_active_layout)),
                    Task::done(Message::SetTimezone(saved_state.timezone)),
                    batch_fetch_tasks,
                ])),
        )
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SetTickersInfo(exchange, tickers_info) => {
                log::info!("Received tickers info for {exchange}, len: {}", tickers_info.len());

                self.ticker_info_map.insert(exchange, tickers_info);

                self.layouts.values_mut().for_each(|dashboard| {
                    dashboard.set_tickers_info(self.ticker_info_map.clone());
                });
            }
            Message::MarketWsEvent(exchange, event) => {
                let main_window_id = self.main_window.id;
                let dashboard = self.get_mut_dashboard(self.last_active_layout);

                match event {
                    data_providers::Event::Connected(_) => {
                        log::info!("a stream connected to {exchange} WS");
                    }
                    data_providers::Event::Disconnected(reason) => {
                        log::info!("a stream disconnected from {exchange} WS: {reason:?}");
                    }
                    data_providers::Event::DepthReceived(
                        ticker,
                        depth_update_t,
                        depth,
                        trades_buffer,
                    ) => {
                        return dashboard
                            .update_depth_and_trades(
                                &StreamType::DepthAndTrades { exchange, ticker },
                                depth_update_t,
                                depth,
                                trades_buffer,
                                main_window_id,
                            )
                            .map(Message::Dashboard);
                    }
                    data_providers::Event::KlineReceived(ticker, kline, timeframe) => {
                        return dashboard
                            .update_latest_klines(
                                &StreamType::Kline {
                                    exchange,
                                    ticker,
                                    timeframe,
                                },
                                &kline,
                                main_window_id,
                            )
                            .map(Message::Dashboard);
                    }
                }
            }
            Message::ToggleLayoutLock => {
                let dashboard = self.get_mut_dashboard(self.last_active_layout);

                dashboard.layout_lock = !dashboard.layout_lock;
                dashboard.focus = None;
            }
            Message::WindowEvent(event) => match event {
                WindowEvent::CloseRequested(window) => {
                    if window != self.main_window.id {
                        self.get_mut_dashboard(self.last_active_layout)
                            .popout
                            .remove(&window);

                        return window::close(window);
                    }

                    let mut opened_windows: Vec<window::Id> = self
                        .get_dashboard(self.last_active_layout)
                        .popout
                        .keys()
                        .copied()
                        .collect::<Vec<_>>();

                    opened_windows.push(self.main_window.id);

                    return window::collect_window_specs(
                        opened_windows, 
                        Message::SaveAndExit
                    );
                }
            },
            Message::SaveAndExit(windows) => {
                self.get_mut_dashboard(self.last_active_layout)
                    .popout
                    .iter_mut()
                    .for_each(|(id, (_, (pos, size)))| {
                        if let Some((new_pos, new_size)) = windows.get(id) {
                            *pos = *new_pos;
                            *size = *new_size;
                        }
                    });

                let mut layouts = HashMap::new();

                for (id, dashboard) in &self.layouts {
                    let serialized_dashboard = SerializableDashboard::from(dashboard);
                    layouts.insert(*id, serialized_dashboard);
                }

                let favorited_tickers = self.tickers_table.get_favorited_tickers();

                let size: Option<Size> = windows
                    .iter()
                    .find(|(id, _)| **id == self.main_window.id)
                    .map(|(_, (_, size))| *size);

                let position: Option<Point> = windows
                    .iter()
                    .find(|(id, _)| **id == self.main_window.id)
                    .map(|(_, (position, _))| *position);

                let user_tz = {
                    let dashboard = self.get_dashboard(self.last_active_layout);
                    dashboard.get_timezone()
                };

                let layout = layout::SerializableState::from_parts(
                    layouts,
                    self.theme.clone(),
                    favorited_tickers,
                    self.last_active_layout,
                    size,
                    position,
                    user_tz,
                    self.sidebar_location,
                );

                match serde_json::to_string(&layout) {
                    Ok(layout_str) => {
                        if let Err(e) =
                            layout::write_json_to_file(&layout_str, "dashboard_state.json")
                        {
                            log::error!("Failed to write layout state to file: {}", e);
                        } else {
                            log::info!("Successfully wrote layout state to dashboard_state.json");
                        }
                    }
                    Err(e) => log::error!("Failed to serialize layout: {}", e),
                }

                return iced::exit();
            }
            Message::ToggleModal(modal) => {
                if modal == self.active_modal {
                    self.active_modal = DashboardModal::None;
                } else {
                    self.active_modal = modal;
                }
            }
            Message::Notification(notification) => {
                self.notification = Some(notification);
            }
            Message::ErrorOccurred(err) => {
                return match err {
                    InternalError::Fetch(err) => handle_error(
                        &err, 
                        "Failed to fetch data",
                        Message::Notification,
                    ),
                };
            }
            Message::ThemeSelected(theme) => {
                self.theme = theme;
            }
            Message::ResetCurrentLayout => {
                let dashboard = self.get_mut_dashboard(self.last_active_layout);

                let active_popout_keys = dashboard.popout.keys().copied().collect::<Vec<_>>();

                let window_tasks = Task::batch(
                    active_popout_keys
                        .iter()
                        .map(|&popout_id| window::close(popout_id))
                        .collect::<Vec<_>>(),
                )
                .then(|_: Task<window::Id>| Task::none());

                return window_tasks.chain(dashboard.reset_layout().map(Message::Dashboard));
            }
            Message::LayoutSelected(new_layout_id) => {
                let active_popout_keys = self
                    .get_dashboard(self.last_active_layout)
                    .popout
                    .keys()
                    .copied()
                    .collect::<Vec<_>>();

                let window_tasks = Task::batch(
                    active_popout_keys
                        .iter()
                        .map(|&popout_id| window::close(popout_id))
                        .collect::<Vec<_>>(),
                )
                .then(|_: Task<window::Id>| Task::none());

                return window::collect_window_specs(
                    active_popout_keys,
                    dashboard::Message::SavePopoutSpecs,
                )
                .map(Message::Dashboard)
                .chain(window_tasks)
                .chain(Task::done(Message::LoadLayout(new_layout_id)));
            }
            Message::LoadLayout(layout_id) => {
                self.last_active_layout = layout_id;

                return self
                    .get_mut_dashboard(layout_id)
                    .load_layout()
                    .map(Message::Dashboard);
            }
            Message::Dashboard(message) => {
                if let Some(dashboard) = self.layouts.get_mut(&self.last_active_layout) {
                    let command = dashboard.update(message, &self.main_window);

                    return Task::batch(vec![command.map(Message::Dashboard)]);
                }
            }
            Message::ToggleTickersDashboard => {
                self.show_tickers_dashboard = !self.show_tickers_dashboard;
            }
            Message::UpdateTickersTable(exchange, tickers_info) => {
                self.tickers_table.update_table(exchange, tickers_info);
            }
            Message::FetchAndUpdateTickersTable => {
                let bybit_linear_fetch = fetch_ticker_prices(
                    Exchange::BybitLinear,
                    bybit::fetch_ticker_prices(MarketType::LinearPerps),
                );
                let binance_linear_fetch = fetch_ticker_prices(
                    Exchange::BinanceFutures,
                    binance::fetch_ticker_prices(MarketType::LinearPerps),
                );
                let binance_spot_fetch = fetch_ticker_prices(
                    Exchange::BinanceSpot,
                    binance::fetch_ticker_prices(MarketType::Spot),
                );
                let bybit_spot_fetch = fetch_ticker_prices(
                    Exchange::BybitSpot,
                    bybit::fetch_ticker_prices(MarketType::Spot),
                );

                return Task::batch(vec![
                    bybit_linear_fetch, 
                    binance_linear_fetch, 
                    binance_spot_fetch,
                    bybit_spot_fetch,
                ]);
            }
            Message::TickersTable(message) => {
                if let tickers_table::Message::TickerSelected(ticker, exchange, content) = message {
                    let main_window_id = self.main_window.id;

                    let command = self
                        .get_mut_dashboard(self.last_active_layout)
                        .init_pane_task(main_window_id, ticker, exchange, &content);

                    return Task::batch(vec![command.map(Message::Dashboard)]);
                } else {
                    let command = self.tickers_table.update(message);

                    return Task::batch(vec![command.map(Message::TickersTable)]);
                }
            }
            Message::SetTimezone(tz) => {
                self.layouts.values_mut().for_each(|dashboard| {
                    dashboard.set_timezone(self.main_window.id, tz);
                });
            }
            Message::SidebarPosition(pos) => {
                self.sidebar_location = pos;
            }
        }
        Task::none()
    }

    fn view(&self, id: window::Id) -> Element<'_, Message> {
        let dashboard = self.get_dashboard(self.last_active_layout);

        if id != self.main_window.id {
            return container(
                dashboard
                .view_window(id, &self.main_window)
                .map(Message::Dashboard)
            )
            .padding(padding::top(if cfg!(target_os = "macos") { 20 } else { 0 }))
            .into();
        } else {
            let branding_logo = center(
                text("FLOWSURFACE")
                    .font(
                        iced::Font {
                            weight: iced::font::Weight::Bold,
                            ..Default::default()
                        }
                    )
                    .size(16)
                    .style(style::branding_text)
                    .align_x(Alignment::Center)
                )
            .height(20)
            .align_y(Alignment::Center)
            .padding(padding::right(8).top(4));

            let tooltip_position = if self.sidebar_location == Sidebar::Left {
                tooltip::Position::Right
            } else {
                tooltip::Position::Left
            };
            
            let sidebar = {
                let nav_buttons = {
                    let layout_lock_button = {
                        create_button(
                            get_icon_text(
                                if dashboard.layout_lock {
                                    Icon::Locked
                                } else {
                                    Icon::Unlocked
                                }, 
                                14,
                            ).width(24).align_x(Alignment::Center),
                            Message::ToggleLayoutLock,
                            Some("Layout Lock"),
                            tooltip_position,
                            |theme: &Theme, status: button::Status| 
                                style::button_transparent(theme, status, false),
                        )
                    };
                    let settings_modal_button = {
                        let is_active = matches!(self.active_modal, DashboardModal::Settings);

                        create_button(
                            get_icon_text(Icon::Cog, 14)
                                .width(24)
                                .align_x(Alignment::Center),
                            Message::ToggleModal(if is_active {
                                DashboardModal::None
                            } else {
                                DashboardModal::Settings
                            }),
                            Some("Settings"),
                            tooltip_position,
                            move |theme: &Theme, status: button::Status| {
                                style::button_transparent(theme, status, is_active)
                            },
                        )
                    };
                    let layout_modal_button = {
                        let is_active = matches!(self.active_modal, DashboardModal::Layout);
                
                        create_button(
                            get_icon_text(Icon::Layout, 14)
                                .width(24)
                                .align_x(Alignment::Center),
                            Message::ToggleModal(if is_active {
                                DashboardModal::None
                            } else {
                                DashboardModal::Layout
                            }),
                            Some("Manage Layouts"),
                            tooltip_position,
                            move |theme: &Theme, status: button::Status| {
                                style::button_transparent(theme, status, is_active)
                            },
                        )
                    };
                    let ticker_search_button = {
                        let is_active = self.show_tickers_dashboard;
                
                        create_button(
                            get_icon_text(Icon::Search, 14)
                                .width(24)
                                .align_x(Alignment::Center),
                            Message::ToggleTickersDashboard,
                            Some("Search Tickers"),
                            tooltip_position,
                            move |theme: &Theme, status: button::Status| {
                                style::button_transparent(theme, status, is_active)
                            },
                        )
                    };

                    column![
                        ticker_search_button,
                        layout_modal_button,
                        layout_lock_button,
                        Space::with_height(Length::Fill),
                        settings_modal_button,
                    ]
                    .width(32)
                    .spacing(4)
                };

                let tickers_table = {
                    if self.show_tickers_dashboard {
                        column![
                            responsive(move |size| {
                                self.tickers_table.view(size).map(Message::TickersTable)
                            })
                        ]
                        .width(200)
                    } else {
                        column![]
                    }
                };

                match self.sidebar_location {
                    Sidebar::Left => {
                        row![
                            nav_buttons,
                            tickers_table,
                        ]
                    }
                    Sidebar::Right => {
                        row![
                            tickers_table,
                            nav_buttons,
                        ]
                    }
                }
                .spacing(4)
            };

            let dashboard_view = dashboard
                .view(&self.main_window)
                .map(Message::Dashboard);

            let content = column![
                branding_logo,
                match self.sidebar_location {
                    Sidebar::Left => row![
                        sidebar,
                        dashboard_view,
                    ],
                    Sidebar::Right => row![
                        dashboard_view,
                        sidebar
                    ],
                }
                .spacing(4)
                .padding(8),
            ];

            match self.active_modal {
                DashboardModal::Settings => {
                    let mut all_themes: Vec<Theme> = Theme::ALL.to_vec();
                    all_themes.push(Theme::Custom(style::custom_theme().into()));
    
                    let theme_picklist =
                        pick_list(all_themes, Some(self.theme.clone()), Message::ThemeSelected);
    
                    let timezone_picklist = pick_list(
                        [UserTimezone::Utc, UserTimezone::Local],
                        Some(dashboard.get_timezone()),
                        Message::SetTimezone,
                    );
                    let sidebar_pos = pick_list(
                        [Sidebar::Left, Sidebar::Right],
                        Some(self.sidebar_location),
                        Message::SidebarPosition,
                    );
                    let settings_modal = {
                        container(
                            column![
                                column![
                                    text("Sidebar").size(14),
                                    sidebar_pos,
                                ].spacing(4),
                                column![text("Time zone").size(14), timezone_picklist,].spacing(4),
                                column![text("Theme").size(14), theme_picklist,].spacing(4),
                            ]
                            .spacing(16),
                        )
                        .align_x(Alignment::Start)
                        .max_width(500)
                        .padding(24)
                        .style(style::dashboard_modal)
                    };

                    let (align_x, padding) = match self.sidebar_location {
                        Sidebar::Left => (Alignment::Start, padding::left(48).top(8)),
                        Sidebar::Right => (Alignment::End, padding::right(48).top(8)),
                    };
    
                    dashboard_modal(
                        content,
                        settings_modal,
                        Message::ToggleModal(DashboardModal::None),
                        padding,
                        Alignment::End,
                        align_x,
                    )
                }
                DashboardModal::Layout => {
                    let layout_picklist = pick_list(
                        &layout::LayoutId::ALL[..],
                        Some(self.last_active_layout),
                        move |layout: layout::LayoutId| Message::LayoutSelected(layout),
                    );
                    let reset_layout_button = tooltip(
                        button(text("Reset").align_x(Alignment::Center))
                            .width(iced::Length::Fill)
                            .on_press(Message::ResetCurrentLayout),
                        Some("Reset current layout"),
                        tooltip::Position::Top,
                    );
                    let info_text = tooltip(
                        button(text("i")).style(move |theme, status| {
                            style::button_transparent(theme, status, false)
                        }),
                        Some("Layouts won't be saved if app exited abruptly"),
                        tooltip::Position::Top,
                    );
    
                    // Pane management
                    let reset_pane_button = tooltip(
                        button(text("Reset").align_x(Alignment::Center))
                            .width(iced::Length::Fill)
                            .on_press(Message::Dashboard(dashboard::Message::Pane(
                                id,
                                pane::Message::ReplacePane(if let Some(focus) = dashboard.focus {
                                    focus.1
                                } else {
                                    *dashboard.panes.iter().next().unwrap().0
                                }),
                            ))),
                        Some("Reset selected pane"),
                        tooltip::Position::Top,
                    );
                    let split_pane_button = tooltip(
                        button(text("Split").align_x(Alignment::Center))
                            .width(iced::Length::Fill)
                            .on_press(Message::Dashboard(dashboard::Message::Pane(
                                id,
                                pane::Message::SplitPane(
                                    pane_grid::Axis::Horizontal,
                                    if let Some(focus) = dashboard.focus {
                                        focus.1
                                    } else {
                                        *dashboard.panes.iter().next().unwrap().0
                                    },
                                ),
                            ))),
                        Some("Split selected pane horizontally"),
                        tooltip::Position::Top,
                    );
                    let manage_layout_modal = {
                        container(
                            column![
                                column![
                                    text("Panes").size(14),
                                    if dashboard.focus.is_some() {
                                        row![reset_pane_button, split_pane_button,].spacing(8)
                                    } else {
                                        row![text("No pane selected"),]
                                    },
                                ]
                                .align_x(Alignment::Center)
                                .spacing(8),
                                column![
                                    text("Layouts").size(14),
                                    row![info_text, layout_picklist, reset_layout_button,].spacing(8),
                                ]
                                .align_x(Alignment::Center)
                                .spacing(8),
                            ]
                            .align_x(Alignment::Center)
                            .spacing(32),
                        )
                        .width(280)
                        .padding(24)
                        .style(style::dashboard_modal)
                    };

                    let (align_x, padding) = match self.sidebar_location {
                        Sidebar::Left => (Alignment::Start, padding::left(48).top(40)),
                        Sidebar::Right => (Alignment::End, padding::right(48).top(40)),
                    };
    
                    dashboard_modal(
                        content,
                        manage_layout_modal,
                        Message::ToggleModal(DashboardModal::None),
                        padding,
                        Alignment::Start,
                        align_x,
                    )
                }
                DashboardModal::None => content.into(),
            }
        }
    }

    fn theme(&self, _window: window::Id) -> Theme {
        self.theme.clone()
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut market_subscriptions: Vec<Subscription<Message>> = Vec::new();

        self.get_dashboard(self.last_active_layout)
            .pane_streams
            .iter()
            .for_each(|(exchange, stream)| {
                let mut depth_streams: Vec<Subscription<Message>> = Vec::new();
                let mut kline_streams: Vec<(Ticker, Timeframe)> = Vec::new();

                let exchange: Exchange = *exchange;

                stream
                    .values()
                    .flat_map(|stream_types| stream_types.iter())
                    .for_each(|stream_type| match stream_type {
                        StreamType::Kline {
                            ticker, timeframe, ..
                        } => {
                            kline_streams.push((*ticker, *timeframe));
                        }
                        StreamType::DepthAndTrades { ticker, .. } => {
                            let ticker: Ticker = *ticker;

                            let depth_stream = match exchange {
                                Exchange::BinanceFutures => Subscription::run_with_id(
                                    ticker,
                                    binance::connect_market_stream(ticker),
                                )
                                .map(move |event| Message::MarketWsEvent(exchange, event)),
                                Exchange::BybitLinear => Subscription::run_with_id(
                                    ticker,
                                    bybit::connect_market_stream(ticker),
                                )
                                .map(move |event| Message::MarketWsEvent(exchange, event)),
                                Exchange::BinanceSpot => Subscription::run_with_id(
                                    ticker,
                                    binance::connect_market_stream(ticker),
                                )
                                .map(move |event| Message::MarketWsEvent(exchange, event)),
                                Exchange::BybitSpot => Subscription::run_with_id(
                                    ticker,
                                    bybit::connect_market_stream(ticker),
                                )
                                .map(move |event| Message::MarketWsEvent(exchange, event)),
                            };
                            depth_streams.push(depth_stream);
                        }
                        StreamType::None => {}
                    });

                if !kline_streams.is_empty() {
                    let kline_streams_id: Vec<(Ticker, Timeframe)> = kline_streams.clone();

                    let kline_subscription = match exchange {
                        Exchange::BinanceFutures => Subscription::run_with_id(
                            kline_streams_id,
                            binance::connect_kline_stream(kline_streams, MarketType::LinearPerps),
                        )
                        .map(move |event| Message::MarketWsEvent(exchange, event)),
                        Exchange::BybitLinear => Subscription::run_with_id(
                            kline_streams_id,
                            bybit::connect_kline_stream(kline_streams, MarketType::LinearPerps),
                        )
                        .map(move |event| Message::MarketWsEvent(exchange, event)),
                        Exchange::BinanceSpot => Subscription::run_with_id(
                            kline_streams_id,
                            binance::connect_kline_stream(kline_streams, MarketType::Spot),
                        )
                        .map(move |event| Message::MarketWsEvent(exchange, event)),
                        Exchange::BybitSpot => Subscription::run_with_id(
                            kline_streams_id,
                            bybit::connect_kline_stream(kline_streams, MarketType::Spot),
                        )
                        .map(move |event| Message::MarketWsEvent(exchange, event)),
                    };
                    market_subscriptions.push(kline_subscription);
                }

                if !depth_streams.is_empty() {
                    market_subscriptions.push(Subscription::batch(depth_streams));
                }
            });

        let tickers_table_fetch = iced::time::every(std::time::Duration::from_secs(
            if self.show_tickers_dashboard { 25 } else { 300 },
        ))
        .map(|_| Message::FetchAndUpdateTickersTable);

        let window_events = window_events().map(Message::WindowEvent);

        Subscription::batch(vec![
            Subscription::batch(market_subscriptions),
            tickers_table_fetch,
            window_events,
        ])
    }

    fn get_mut_dashboard(&mut self, layout_id: layout::LayoutId) -> &mut Dashboard {
        self.layouts.get_mut(&layout_id).expect("No active layout")
    }

    fn get_dashboard(&self, layout_id: layout::LayoutId) -> &Dashboard {
        self.layouts.get(&layout_id).expect("No active layout")
    }
}

fn fetch_ticker_info<F>(exchange: Exchange, fetch_fn: F) -> Task<Message>
where
    F: Future<
            Output = Result<
                HashMap<Ticker, Option<data_providers::TickerInfo>>,
                data_providers::StreamError,
            >,
        > + MaybeSend
        + 'static,
{
    Task::perform(
        fetch_fn.map_err(|err| format!("{err}")),
        move |ticksize| match ticksize {
            Ok(ticksize) => Message::SetTickersInfo(exchange, ticksize),
            Err(err) => Message::ErrorOccurred(InternalError::Fetch(err)),
        },
    )
}

fn fetch_ticker_prices<F>(exchange: Exchange, fetch_fn: F) -> Task<Message>
where
    F: Future<Output = Result<HashMap<Ticker, TickerStats>, data_providers::StreamError>>
        + MaybeSend
        + 'static,
{
    Task::perform(
        fetch_fn.map_err(|err| format!("{err}")),
        move |tickers_table| match tickers_table {
            Ok(tickers_table) => Message::UpdateTickersTable(exchange, tickers_table),
            Err(err) => Message::ErrorOccurred(InternalError::Fetch(err)),
        },
    )
}