#![windows_subsystem = "windows"]

mod style;
mod charts;
mod window;
mod layout;
mod logger;
mod screen;
mod widget;
mod tooltip;
mod tickers_table;
mod data_providers;

use tooltip::tooltip;
use tickers_table::TickersTable;
use layout::{Layout, LayoutManager, SerializableDashboard, Sidebar};
use style::{get_icon_text, Icon, ICON_BYTES};
use screen::{
    create_button, dashboard::{self, pane, Dashboard}, handle_error, 
    modal::{confirmation_modal, dashboard_modal}, 
    Notification, UserTimezone
};
use data_providers::{
    binance, bybit, Exchange, StreamType, Ticker, TickerInfo, TickerStats,
};
use window::{window_events, Window, WindowEvent};
use iced::{
    widget::{button, center, column, container, pane_grid, pick_list, responsive, row, text, Space}, 
    Alignment, Element, Length, Point, Size, Subscription, Task, Theme, padding,
};
use iced_futures::MaybeSend;
use futures::TryFutureExt;
use std::{collections::HashMap, vec, future::Future};

fn main() {
    logger::setup(false, false).expect("Failed to initialize logger");

    std::thread::spawn(|| layout::cleanup_old_data());

    let saved_state: layout::SavedState = layout::load_saved_state("dashboard_state.json");

    std::env::set_var(
        "ICED_PRESENT_MODE", 
        saved_state.present_mode.get_env_name(),
    );

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
        platform_specific: {
            #[cfg(target_os = "macos")]
            {
                iced::window::settings::PlatformSpecific {
                    title_hidden: true,
                    titlebar_transparent: true,
                    fullsize_content_view: true,
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                Default::default()
            }
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
        .scale_factor(State::scale_factor)
        .theme(State::theme)
        .subscription(State::subscription)
        .font(ICON_BYTES)
        .run_with(move || State::new(saved_state, window_settings));
}

#[derive(Debug, Clone)]
enum Message {
    Notification(Notification),
    ErrorOccurred(InternalError),

    ToggleModal(SidebarModal),

    MarketWsEvent(data_providers::Event),
    ToggleTradeFetch(bool),

    WindowEvent(WindowEvent),
    SaveAndExit(HashMap<window::Id, (Point, Size)>),

    ToggleLayoutLock,
    LayoutSelected(Layout),
    ThemeSelected(Theme),
    Dashboard(dashboard::Message),
    SetTickersInfo(Exchange, HashMap<Ticker, Option<TickerInfo>>),
    SetTimezone(UserTimezone),
    SidebarPosition(layout::Sidebar),

    TickersTable(tickers_table::Message),
    ToggleTickersDashboard,
    UpdateTickersTable(Exchange, HashMap<Ticker, TickerStats>),
    FetchAndUpdateTickersTable,

    LoadLayout(Layout),
    ToggleDialogModal(Option<(String, Box<Message>)>),

    PresentModeSelected(screen::PresentMode),
    ChangePresentMode(screen::PresentMode),

    ScaleFactorChanged(f64),

    ManageLayouts(layout::Message),
}

struct State {
    theme: Theme,
    main_window: Window,
    timezone: UserTimezone,
    layout_locked: bool,
    confirmation_dialog: Option<(String, Box<Message>)>,
    layouts: LayoutManager,
    active_modal: SidebarModal,
    sidebar_location: Sidebar,
    notification: Option<Notification>,
    show_tickers_dashboard: bool,
    scale_factor: layout::ScaleFactor,
    tickers_table: TickersTable,
    tickers_info: HashMap<Exchange, HashMap<Ticker, Option<TickerInfo>>>,
    present_mode: screen::PresentMode,
}

impl State {
    fn new(
        saved_state: layout::SavedState,
        window_settings: window::Settings,
    ) -> (Self, Task<Message>) {
        let (main_window, open_main_window) = window::open(window_settings);

        let active_layout = saved_state.layout_manager.active_layout.clone();

        let exchange_fetch_tasks = {
            Exchange::MARKET_TYPES.iter()
                .map(|(exchange, market_type)| {                    
                    match exchange {
                        Exchange::BinanceFutures | Exchange::BinanceSpot => {
                            fetch_ticker_info(*exchange, binance::fetch_ticksize(*market_type))
                        }
                        Exchange::BybitLinear | Exchange::BybitSpot => {
                            fetch_ticker_info(*exchange, bybit::fetch_ticksize(*market_type))
                        }
                    }
                })
                .collect::<Vec<Task<Message>>>()
        };

        (
            Self {
                theme: saved_state.selected_theme.theme,
                layouts: saved_state.layout_manager,
                main_window: Window::new(main_window),
                active_modal: SidebarModal::None,
                notification: None,
                tickers_info: HashMap::new(),
                show_tickers_dashboard: false,
                sidebar_location: saved_state.sidebar,
                tickers_table: TickersTable::new(saved_state.favorited_tickers),
                confirmation_dialog: None,
                layout_locked: false,
                timezone: saved_state.timezone,
                scale_factor: saved_state.scale_factor,
                present_mode: saved_state.present_mode,
            },
            open_main_window
                .then(|_| Task::none())
                .chain(Task::batch(vec![
                    Task::done(Message::LoadLayout(active_layout)),
                    Task::done(Message::SetTimezone(saved_state.timezone)),
                    Task::done(Message::FetchAndUpdateTickersTable),
                    Task::batch(exchange_fetch_tasks),
                ])),
        )
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SetTickersInfo(exchange, tickers_info) => {
                log::info!("Received tickers info for {exchange}, len: {}", tickers_info.len());
                self.tickers_info.insert(exchange, tickers_info);
            }
            Message::MarketWsEvent(event) => {
                let main_window_id = self.main_window.id;
                if let Some(dashboard) = self.get_active_dashboard_mut() {
                    match event {
                        data_providers::Event::Connected(exchange, _) => {
                            log::info!("a stream connected to {exchange} WS");
                        }
                        data_providers::Event::Disconnected(exchange, reason) => {
                            log::info!("a stream disconnected from {exchange} WS: {reason:?}");
                        }
                        data_providers::Event::DepthReceived(
                            stream,
                            depth_update_t,
                            depth,
                            trades_buffer,
                        ) => {
                            return dashboard
                                .update_depth_and_trades(
                                    &stream,
                                    depth_update_t,
                                    depth,
                                    trades_buffer,
                                    main_window_id,
                                )
                                .map(Message::Dashboard);
                        }
                        data_providers::Event::KlineReceived(
                            stream,
                            kline, 
                        ) => {
                            return dashboard
                                .update_latest_klines(
                                    &stream,
                                    &kline,
                                    main_window_id,
                                )
                                .map(Message::Dashboard);
                        }
                    }
                }                
            }
            Message::ToggleLayoutLock => {
                self.layout_locked = !self.layout_locked;
                if let Some(dashboard) = self.get_active_dashboard_mut() {
                    dashboard.focus = None;
                }
            }
            Message::WindowEvent(event) => match event {
                WindowEvent::CloseRequested(window) => {
                    let main_window = self.main_window.id;

                    let dashboard = match self.get_active_dashboard_mut() {
                        Some(dashboard) => dashboard,
                        None => {
                            return iced::exit(); 
                        }
                    };

                    if window != main_window {
                        dashboard.popout.remove(&window);
                        return window::close(window);
                    }

                    let mut opened_windows: Vec<window::Id> = dashboard
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
            }
            Message::SaveAndExit(windows) => {
                let dashboard = self.get_active_dashboard_mut().expect("No active dashboard");

                dashboard.popout
                    .iter_mut()
                    .for_each(|(id, (_, (pos, size)))| {
                        if let Some((new_pos, new_size)) = windows.get(id) {
                            *pos = *new_pos;
                            *size = *new_size;
                        }
                    });

                let mut ser_layouts = Vec::new();

                for id in &self.layouts.layout_order {
                    if let Some((layout, dashboard)) = self.layouts.layouts.get(id) {
                        let serialized_dashboard = SerializableDashboard::from(dashboard);
                        
                        ser_layouts.push(
                            layout::SerializableLayout {
                                name: layout.name.clone(),
                                dashboard: serialized_dashboard,
                            }
                        );
                    }
                }

                let layouts = layout::SerializableLayouts {
                    layouts: ser_layouts,
                    active_layout: self.layouts.active_layout.name.clone(),
                };

                let favorited_tickers = self.tickers_table.get_favorited_tickers();

                let (size, position) = windows
                    .iter()
                    .find(|(id, _)| **id == self.main_window.id)
                    .map(|(_, (position, size))| (*size, *position))
                    .unzip();

                let layout = layout::SerializableState::from_parts(
                    layouts,
                    self.theme.clone(),
                    favorited_tickers,
                    size,
                    position,
                    self.timezone,
                    self.sidebar_location,
                    self.present_mode,
                    self.scale_factor,
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
                    self.active_modal = SidebarModal::None;
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
            Message::LayoutSelected(new_layout_id) => {
                if let Some(dashboard) = self.get_active_dashboard() {
                    let active_popout_keys = dashboard
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
            }
            Message::LoadLayout(layout) => {
                self.layouts.active_layout = layout.clone();
                if let Some(dashboard) = self.get_active_dashboard_mut() {
                    dashboard.focus = None;
                    return dashboard.load_layout()
                        .map(Message::Dashboard);
                }
            }
            Message::Dashboard(message) => {
                let main_window = self.main_window;
                if let Some(dashboard) = self.get_active_dashboard_mut() {
                    return dashboard.update(message, &main_window)
                        .map(Message::Dashboard);
                }
            }
            Message::ToggleTickersDashboard => {
                self.show_tickers_dashboard = !self.show_tickers_dashboard;
            }
            Message::UpdateTickersTable(exchange, tickers_info) => {
                self.tickers_table.update_table(exchange, tickers_info);
            }
            Message::FetchAndUpdateTickersTable => {
                let fetch_tasks = {
                    Exchange::MARKET_TYPES.iter()
                        .map(|(exchange, market_type)| {                    
                            match exchange {
                                Exchange::BinanceFutures | Exchange::BinanceSpot => {
                                    fetch_ticker_prices(*exchange, binance::fetch_ticker_prices(*market_type))
                                }
                                Exchange::BybitLinear | Exchange::BybitSpot => {
                                    fetch_ticker_prices(*exchange, bybit::fetch_ticker_prices(*market_type))
                                }
                            }
                        })
                        .collect::<Vec<Task<Message>>>()
                };

                return Task::batch(fetch_tasks)
            }
            Message::TickersTable(message) => {
                if let tickers_table::Message::TickerSelected(ticker, exchange, content) = message {
                    let main_window_id = self.main_window.id;

                    let ticker_info = self.tickers_info.get(&exchange).and_then(|info| {
                        info.get(&ticker).cloned().flatten()
                    });

                    if let Some(dashboard) = self.get_active_dashboard_mut() {
                        if let Some(ticker_info) = ticker_info {
                            let task = dashboard
                                .init_pane_task(main_window_id, (ticker, ticker_info), exchange, &content);

                            return task.map(Message::Dashboard);
                        } else {
                            return Task::done(Message::ErrorOccurred(InternalError::Fetch(
                                format!(
                                    "Couldn't find ticker info for {ticker} on {exchange}, try restarting the app"
                                ),
                            )));
                        }
                    }
                } else {
                    return self.tickers_table.update(message)
                        .map(Message::TickersTable);
                }
            }
            Message::SetTimezone(tz) => {
                self.timezone = tz;
            }
            Message::SidebarPosition(pos) => {
                self.sidebar_location = pos;
            }
            Message::ToggleTradeFetch(checked) => {
                self.layouts.iter_dashboards_mut().for_each(|dashboard| {
                    dashboard.toggle_trade_fetch(checked, &self.main_window);
                });
                    
                if checked {
                    self.confirmation_dialog = None;
                }
            }
            Message::ToggleDialogModal(dialog) => {
                self.confirmation_dialog = dialog;
            }
            Message::PresentModeSelected(mode) => {
                if mode != self.present_mode {
                    return Task::done(Message::ToggleDialogModal(
                        Some((
                            "This will take effect after restarting the app".to_string(),
                            Box::new(Message::ChangePresentMode(mode)),
                        )),
                    ));
                }
            }
            Message::ChangePresentMode(mode) => {
                self.present_mode = mode;
                self.confirmation_dialog = None;
            }
            Message::ScaleFactorChanged(value) => {
                self.scale_factor = layout::ScaleFactor::from(value);
            }
            Message::ManageLayouts(msg) => {
                if let layout::Message::SelectActive(layout) = msg {
                    return Task::done(Message::LayoutSelected(layout));
                } else {
                    return self.layouts.update(msg)
                        .map(Message::ManageLayouts);
                }
            }
        }
        Task::none()
    }

    fn view(&self, id: window::Id) -> Element<'_, Message> {
        let dashboard = match self.get_active_dashboard() {
            Some(dashboard) => dashboard,
            None => {
                return center(
                    column![
                        text("No dashboard available").size(20),
                        button("Add new dashboard")
                            .on_press(Message::ManageLayouts(
                                layout::Message::AddLayout)
                            )
                    ]
                    .align_x(Alignment::Center)
                    .spacing(8)
                )
                .into();
            }    
        };

        if id != self.main_window.id {
            container(
                dashboard
                .view_window(
                    id, 
                    &self.main_window, 
                    self.layout_locked,
                    &self.timezone,
                )
                .map(Message::Dashboard)
            )
            .padding(padding::top(if cfg!(target_os = "macos") { 20 } else { 0 }))
            .into()
        } else {
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
                                if self.layout_locked {
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
                        let is_active = matches!(self.active_modal, SidebarModal::Settings);

                        create_button(
                            get_icon_text(Icon::Cog, 14)
                                .width(24)
                                .align_x(Alignment::Center),
                            Message::ToggleModal(if is_active {
                                SidebarModal::None
                            } else {
                                SidebarModal::Settings
                            }),
                            Some("Settings"),
                            tooltip_position,
                            move |theme: &Theme, status: button::Status| {
                                style::button_transparent(theme, status, is_active)
                            },
                        )
                    };
                    let layout_modal_button = {
                        let is_active = matches!(self.active_modal, SidebarModal::Layout);
                
                        create_button(
                            get_icon_text(Icon::Layout, 14)
                                .width(24)
                                .align_x(Alignment::Center),
                            Message::ToggleModal(if is_active {
                                SidebarModal::None
                            } else {
                                SidebarModal::Layout
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
                .view(
                    &self.main_window, 
                    self.layout_locked,
                    &self.timezone,
                )
                .map(Message::Dashboard);

            let base = column![
                {
                    #[cfg(target_os = "macos")] {
                        iced::widget::center(
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
                        .padding(padding::right(8).top(4))
                    }
                    #[cfg(not(target_os = "macos"))] {
                        column![]
                    }
                },
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
                SidebarModal::Settings => {
                    let settings_modal = {
                        let mut all_themes: Vec<Theme> = Theme::ALL.to_vec();
                        all_themes.push(Theme::Custom(style::custom_theme().into()));

                        let trade_fetch_checkbox = {
                            let is_active = dashboard.trade_fetch_enabled;
                    
                            let checkbox = iced::widget::checkbox("Fetch trades (Binance)", is_active)
                                .on_toggle(|checked| {
                                        if checked {
                                            Message::ToggleDialogModal(
                                                Some((
                                                    "This might be unreliable and take some time to complete"
                                                    .to_string(),
                                                    Box::new(Message::ToggleTradeFetch(true)),
                                                )),
                                            )
                                        } else {
                                            Message::ToggleTradeFetch(false)
                                        }
                                    }
                                );
                    
                            tooltip(
                                checkbox,
                                Some("Try to fetch trades for footprint charts"),
                                tooltip::Position::Top,
                            )
                        };

                        let present_mode_picklist = pick_list(
                            screen::PresentMode::ALL,
                            Some(self.present_mode),
                            Message::PresentModeSelected,
                        );

                        let theme_picklist = pick_list(
                            all_themes, 
                            Some(self.theme.clone()), 
                            Message::ThemeSelected,
                        );
        
                        let timezone_picklist = pick_list(
                            [UserTimezone::Utc, UserTimezone::Local],
                            Some(self.timezone),
                            Message::SetTimezone,
                        );

                        let sidebar_pos = pick_list(
                            [Sidebar::Left, Sidebar::Right],
                            Some(self.sidebar_location),
                            Message::SidebarPosition,
                        );

                        let scale_factor = {
                            let current_value: f64 = self.scale_factor.into();
                        
                            let decrease_btn = if current_value > 0.8 {
                                button(text("-")).on_press(Message::ScaleFactorChanged(current_value - 0.1))
                            } else {
                                button(text("-"))
                            };
                        
                            let increase_btn = if current_value < 1.8 {
                                button(text("+")).on_press(Message::ScaleFactorChanged(current_value + 0.1))
                            } else {
                                button(text("+"))
                            };
                        
                            container(
                                row![
                                    decrease_btn,
                                    text(format!(
                                        "{:.0}%", 
                                        current_value * 100.0)
                                    )
                                    .size(14),
                                    increase_btn,
                                ]
                                .align_y(Alignment::Center)
                                .spacing(8)
                                .padding(8),
                            )
                            .style(style::modal_container)
                        };

                        container(
                            column![
                                column![
                                    text("Sidebar").size(14),
                                    sidebar_pos,
                                ].spacing(8),
                                column![text("Time zone").size(14), timezone_picklist,].spacing(8),
                                column![text("Theme").size(14), theme_picklist,].spacing(8),
                                column![text("Interface scale").size(14), scale_factor,].spacing(8),
                                column![
                                    text("Experimental").size(14),
                                    trade_fetch_checkbox,
                                    present_mode_picklist,
                                ].spacing(8),
                            ]
                            .spacing(20),
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

                    let base_content = dashboard_modal(
                        base,
                        settings_modal,
                        Message::ToggleModal(SidebarModal::None),
                        padding,
                        Alignment::End,
                        align_x,
                    );

                    if let Some((dialog, message)) = &self.confirmation_dialog {
                        let dialog_content = self.confirm_dialog(
                            dialog,
                            *message.to_owned(),
                        );
    
                        confirmation_modal(
                            base_content, 
                            dialog_content, 
                            Message::ToggleDialogModal(None)
                        )
                    } else {
                        base_content
                    }
                }
                SidebarModal::Layout => {
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
                                    self.layouts.view().map(Message::ManageLayouts),
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
                        base,
                        manage_layout_modal,
                        Message::ToggleModal(SidebarModal::None),
                        padding,
                        Alignment::Start,
                        align_x,
                    )
                }
                SidebarModal::None => base.into(),
            }
        }
    }

    fn confirm_dialog<'a>(
        &'a self, 
        dialog: &'a str, 
        on_confirm: Message,
    ) -> Element<'a, Message> {
        let dialog_content = container(
            column![
                text(dialog).size(14),
                row![
                    button(text("Cancel"))
                        .style(|theme, status| style::button_transparent(theme, status, false))
                        .on_press(Message::ToggleDialogModal(None)),
                    button(text("Confirm"))
                        .on_press(on_confirm),
                ]
                .spacing(8),
            ]
            .align_x(Alignment::Center)
            .spacing(16),
        )
        .padding(24)
        .style(style::dashboard_modal);

        dialog_content.into()
    }

    fn theme(&self, _window: window::Id) -> Theme {
        self.theme.clone()
    }

    fn scale_factor(&self, _window: window::Id) -> f64 {
        self.scale_factor.into()
    }

    fn subscription(&self) -> Subscription<Message> {
        let window_events = window_events().map(Message::WindowEvent);

        let dashboard = match self.get_active_dashboard() {
            Some(dashboard) => dashboard,
            None => {
                return window_events;
            }
        }; 

        let exchange_streams = dashboard
            .get_market_subscriptions(Message::MarketWsEvent);
        
        let tickers_table_fetch = iced::time::every(std::time::Duration::from_secs(
            if self.show_tickers_dashboard { 25 } else { 300 },
        ))
        .map(|_| Message::FetchAndUpdateTickersTable);
        
        Subscription::batch(vec![
            exchange_streams,
            tickers_table_fetch,
            window_events,
        ])
    }

    fn get_active_dashboard(&self) -> Option<&Dashboard> {
        self.layouts.get_active_dashboard()
    }

    fn get_active_dashboard_mut(&mut self) -> Option<&mut Dashboard> {
        self.layouts.get_active_dashboard_mut()
    }
}

#[derive(thiserror::Error, Debug, Clone)]
enum InternalError {
    #[error("Fetch error: {0}")]
    Fetch(String),
}

#[derive(Debug, Clone, PartialEq)]
enum SidebarModal {
    Layout,
    Settings,
    None,
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