#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod chart;
mod layout;
mod logger;
mod screen;
mod style;
mod widget;
mod window;

use layout::{Layout, LayoutManager};
use screen::{
    create_button,
    dashboard::{
        self, Dashboard, pane,
        theme_editor::{self, ThemeEditor},
        tickers_table::{self, TickersTable},
    },
};
use style::{Icon, icon_text};
use widget::{
    confirm_dialog_container, dashboard_modal, main_dialog_modal,
    notification::{self, Toast},
    tooltip,
};
use window::{Window, window_events};

use data::{InternalError, config::theme::default_theme, layout::WindowSpec, sidebar};
use exchange::adapter::{Exchange, StreamType, fetch_ticker_info};
use iced::{
    Alignment, Element, Length, Subscription, Task, padding,
    widget::{
        Space, button, center, column, container, pane_grid, pick_list, responsive, row, text,
        tooltip::Position as TooltipPosition,
    },
};
use std::{borrow::Cow, collections::HashMap, vec};

fn main() {
    logger::setup(cfg!(debug_assertions)).expect("Failed to initialize logger");

    std::thread::spawn(data::cleanup_old_market_data);

    let _ = iced::daemon(Flowsurface::new, Flowsurface::update, Flowsurface::view)
        .settings(iced::Settings {
            antialiasing: true,
            fonts: vec![
                Cow::Borrowed(style::AZERET_MONO_BYTES),
                Cow::Borrowed(style::ICONS_BYTES),
            ],
            default_text_size: iced::Pixels(12.0),
            ..Default::default()
        })
        .title(Flowsurface::title)
        .theme(Flowsurface::theme)
        .scale_factor(Flowsurface::scale_factor)
        .subscription(Flowsurface::subscription)
        .run();
}

struct Flowsurface {
    main_window: Window,
    layout_manager: LayoutManager,
    tickers_table: TickersTable,
    confirm_dialog: Option<(String, Box<Message>)>,
    scale_factor: data::ScaleFactor,
    timezone: data::UserTimezone,
    sidebar: data::Sidebar,
    theme: data::Theme,
    notifications: Vec<Toast>,
    audio_stream: audio::AudioStream,
    theme_editor: ThemeEditor,
}

#[derive(Debug, Clone)]
enum Message {
    LoadLayout(Layout),
    Layouts(layout::Message),

    MarketWsEvent(exchange::Event),
    AudioStream(audio::Message),
    FetchTickersInfo,
    ToggleTradeFetch(bool),

    Dashboard(Option<uuid::Uuid>, dashboard::Message),
    TickersTable(tickers_table::Message),

    WindowEvent(window::Event),
    ExitRequested(HashMap<window::Id, WindowSpec>),
    ErrorOccurred(InternalError),

    ThemeSelected(data::Theme),
    ScaleFactorChanged(data::ScaleFactor),
    SetTimezone(data::UserTimezone),
    SetSidebarPosition(sidebar::Position),
    ToggleSidebarMenu(sidebar::Menu),
    ToggleDialogModal(Option<(String, Box<Message>)>),
    DataFolderRequested,

    AddNotification(Toast),
    DeleteNotification(usize),

    ThemeEditor(theme_editor::Message),
}

impl Flowsurface {
    fn new() -> (Self, Task<Message>) {
        let saved_state = layout::load_saved_state();

        let main_window_cfg = window::Settings {
            size: saved_state
                .main_window
                .map_or_else(window::default_size, |w| w.size()),
            position: saved_state.main_window.map(|w| w.position()).map_or(
                iced::window::Position::Centered,
                iced::window::Position::Specific,
            ),
            exit_on_close_request: false,
            ..window::settings()
        };

        let active_layout = saved_state.layout_manager.active_layout();
        let (main_window_id, open_main_window) = window::open(main_window_cfg);

        let theme = saved_state.theme.clone();

        (
            Self {
                main_window: Window::new(main_window_id),
                layout_manager: saved_state.layout_manager,
                tickers_table: TickersTable::new(saved_state.favorited_tickers),
                confirm_dialog: None,
                timezone: saved_state.timezone,
                scale_factor: saved_state.scale_factor,
                sidebar: saved_state.sidebar,
                theme: theme.clone(),
                notifications: vec![],
                audio_stream: audio::AudioStream::new(saved_state.audio_cfg),
                theme_editor: theme_editor::ThemeEditor::new(saved_state.custom_theme),
            },
            open_main_window
                .then(|_| Task::none())
                .chain(Task::batch(vec![
                    Task::done(Message::LoadLayout(active_layout)),
                    Task::done(Message::SetTimezone(saved_state.timezone)),
                    Task::done(Message::FetchTickersInfo),
                ])),
        )
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::FetchTickersInfo => {
                let fetch_tasks = Exchange::ALL
                    .iter()
                    .map(|exchange| {
                        Task::perform(fetch_ticker_info(*exchange), move |result| match result {
                            Ok(ticker_info) => Message::TickersTable(
                                tickers_table::Message::UpdateTickersInfo(*exchange, ticker_info),
                            ),
                            Err(err) => {
                                Message::ErrorOccurred(InternalError::Fetch(err.to_string()))
                            }
                        })
                    })
                    .collect::<Vec<Task<Message>>>();

                return Task::batch(fetch_tasks);
            }
            Message::MarketWsEvent(event) => {
                let main_window_id = self.main_window.id;

                if let Some(dashboard) = self.active_dashboard_mut() {
                    match event {
                        exchange::Event::Connected(exchange, _) => {
                            log::info!("a stream connected to {exchange} WS");
                        }
                        exchange::Event::Disconnected(exchange, reason) => {
                            log::info!("a stream disconnected from {exchange} WS: {reason:?}");
                        }
                        exchange::Event::DepthReceived(
                            stream,
                            depth_update_t,
                            depth,
                            trades_buffer,
                        ) => {
                            let task = dashboard
                                .update_depth_and_trades(
                                    &stream,
                                    depth_update_t,
                                    &depth,
                                    &trades_buffer,
                                    main_window_id,
                                )
                                .map(move |msg| Message::Dashboard(None, msg));

                            if let Err(err) =
                                self.audio_stream.try_play_sound(&stream, &trades_buffer)
                            {
                                log::error!("Failed to play sound: {err}");
                            }

                            return task;
                        }
                        exchange::Event::KlineReceived(stream, kline) => {
                            return dashboard
                                .update_latest_klines(&stream, &kline, main_window_id)
                                .map(move |msg| Message::Dashboard(None, msg));
                        }
                    }
                }
            }
            Message::WindowEvent(event) => match event {
                window::Event::CloseRequested(window) => {
                    let main_window = self.main_window.id;

                    let Some(dashboard) = self.active_dashboard_mut() else {
                        return iced::exit();
                    };

                    if window != main_window {
                        dashboard.popout.remove(&window);
                        return window::close(window);
                    }

                    let mut opened_windows = dashboard
                        .popout
                        .keys()
                        .copied()
                        .collect::<Vec<window::Id>>();

                    opened_windows.push(main_window);

                    return window::collect_window_specs(opened_windows, Message::ExitRequested);
                }
            },
            Message::ExitRequested(windows) => {
                let dashboard = self.active_dashboard_mut().expect("No active dashboard");

                dashboard
                    .popout
                    .iter_mut()
                    .for_each(|(id, (_, window_spec))| {
                        if let Some(new_window_spec) = windows.get(id) {
                            *window_spec = *new_window_spec;
                        }
                    });

                let mut ser_layouts = Vec::new();

                for id in &self.layout_manager.layout_order {
                    if let Some((layout, dashboard)) = self.layout_manager.layouts.get(id) {
                        let serialized_dashboard = data::Dashboard::from(dashboard);

                        ser_layouts.push(data::Layout {
                            name: layout.name.clone(),
                            dashboard: serialized_dashboard,
                        });
                    }
                }

                let layouts = data::Layouts {
                    layouts: ser_layouts,
                    active_layout: self.layout_manager.active_layout.name.clone(),
                };

                let main_window = windows
                    .iter()
                    .find(|(id, _)| **id == self.main_window.id)
                    .map(|(_, spec)| *spec);

                let audio_cfg = data::AudioStream::from(&self.audio_stream);

                let layout = data::State::from_parts(
                    layouts,
                    self.theme.clone(),
                    self.theme_editor.custom_theme.clone().map(data::Theme),
                    self.tickers_table.favorited_tickers(),
                    main_window,
                    self.timezone,
                    self.sidebar,
                    self.scale_factor,
                    audio_cfg,
                );

                match serde_json::to_string(&layout) {
                    Ok(layout_str) => {
                        let file_name = data::SAVED_STATE_PATH;

                        if let Err(e) = data::write_json_to_file(&layout_str, file_name) {
                            log::error!("Failed to write layout state to file: {}", e);
                        } else {
                            log::info!("Successfully wrote layout state to {file_name}");
                        }
                    }
                    Err(e) => log::error!("Failed to serialize layout: {}", e),
                }

                return iced::exit();
            }
            Message::ErrorOccurred(err) => {
                return match err {
                    InternalError::Fetch(err) | InternalError::Layout(err) => {
                        Task::done(Message::AddNotification(Toast::error(err)))
                    }
                };
            }
            Message::ThemeSelected(theme) => {
                self.theme = theme.clone();
            }
            Message::Dashboard(id, message) => {
                let main_window = self.main_window;
                let layout_id = id.unwrap_or(self.layout_manager.active_layout.id);

                if let Some(dashboard) = self.layout_manager.mut_dashboard(&layout_id) {
                    let (task, event) = dashboard.update(message, &main_window, &layout_id);

                    let event_task = match event {
                        Some(dashboard::Event::DistributeFetchedData(
                            layout_id,
                            pane_uid,
                            data,
                            stream,
                        )) => dashboard
                            .distribute_fetched_data(main_window.id, pane_uid, data, stream)
                            .map(move |msg| Message::Dashboard(Some(layout_id), msg)),
                        Some(dashboard::Event::Notification(toast)) => {
                            Task::done(Message::AddNotification(toast))
                        }
                        None => Task::none(),
                    };

                    return task
                        .map(move |msg| Message::Dashboard(Some(layout_id), msg))
                        .chain(event_task);
                }
            }
            Message::TickersTable(message) => {
                let action = self.tickers_table.update(message);

                match action {
                    tickers_table::Action::TickerSelected(ticker_info, exchange, content) => {
                        let main_window_id = self.main_window.id;

                        if let Some(dashboard) = self.active_dashboard_mut() {
                            let task = dashboard.init_pane_task(
                                main_window_id,
                                ticker_info,
                                exchange,
                                &content,
                            );

                            return task.map(move |msg| Message::Dashboard(None, msg));
                        }
                    }
                    tickers_table::Action::Fetch(task) => {
                        return task.map(Message::TickersTable);
                    }
                    tickers_table::Action::ErrorOccurred(err) => {
                        return Task::done(Message::ErrorOccurred(err));
                    }
                    tickers_table::Action::None => {}
                }
            }
            Message::SetTimezone(tz) => {
                self.timezone = tz;
            }
            Message::SetSidebarPosition(position) => {
                self.sidebar.set_position(position);
            }
            Message::ScaleFactorChanged(value) => {
                self.scale_factor = value;
            }
            Message::ToggleSidebarMenu(menu) => {
                let new_menu = if self.sidebar.is_menu_active(menu) {
                    sidebar::Menu::None
                } else {
                    menu
                };
                self.sidebar.set_menu(new_menu);
            }
            Message::ToggleTradeFetch(checked) => {
                self.layout_manager
                    .iter_dashboards_mut()
                    .for_each(|dashboard| {
                        dashboard.toggle_trade_fetch(checked, &self.main_window);
                    });

                if checked {
                    self.confirm_dialog = None;
                }
            }
            Message::ToggleDialogModal(dialog) => {
                self.confirm_dialog = dialog;
            }
            Message::Layouts(message) => {
                let action = self.layout_manager.update(message);

                match action {
                    layout::Action::Select(layout) => {
                        if let Some(dashboard) = self.active_dashboard() {
                            let active_popout_keys =
                                dashboard.popout.keys().copied().collect::<Vec<_>>();

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
                            .map(move |msg| Message::Dashboard(None, msg))
                            .chain(window_tasks)
                            .chain(Task::done(Message::LoadLayout(layout)));
                        } else {
                            return Task::done(Message::ErrorOccurred(InternalError::Layout(
                                "Couldn't get active dashboard".to_string(),
                            )));
                        }
                    }
                    layout::Action::None => {}
                }
            }
            Message::LoadLayout(layout) => {
                self.layout_manager.active_layout = layout.clone();
                if let Some(dashboard) = self.active_dashboard_mut() {
                    dashboard.focus = None;
                    return dashboard
                        .load_layout()
                        .map(move |msg| Message::Dashboard(None, msg));
                }
            }
            Message::AddNotification(toast) => {
                self.notifications.push(toast);
            }
            Message::DeleteNotification(index) => {
                if index < self.notifications.len() {
                    self.notifications.remove(index);
                }
            }
            Message::AudioStream(message) => {
                let action = self.audio_stream.update(message);

                match action {
                    audio::Action::None => {}
                }
            }
            Message::DataFolderRequested => {
                if let Err(err) = data::open_data_folder() {
                    return Task::done(Message::AddNotification(Toast::error(format!(
                        "Failed to open data folder: {}",
                        err
                    ))));
                }
            }
            Message::ThemeEditor(msg) => {
                let action = self.theme_editor.update(msg, self.theme.clone().into());

                match action {
                    theme_editor::Action::Exit => {
                        self.sidebar.set_menu(sidebar::Menu::Settings);
                    }
                    theme_editor::Action::UpdateTheme(theme) => {
                        self.theme = data::Theme(theme);

                        let main_window = self.main_window.id;

                        if let Some(dashboard) = self.active_dashboard_mut() {
                            dashboard.invalidate_all_panes(main_window)
                        } else {
                            return Task::done(Message::ErrorOccurred(InternalError::Layout(
                                "No active dashboard".to_string(),
                            )));
                        }
                    }
                    theme_editor::Action::None => {}
                }
            }
        }
        Task::none()
    }

    fn view(&self, id: window::Id) -> Element<'_, Message> {
        let Some(dashboard) = self.active_dashboard() else {
            return center(
                column![
                    text("No dashboard available").size(20),
                    button("Add new dashboard")
                        .on_press(Message::Layouts(layout::Message::AddLayout))
                ]
                .align_x(Alignment::Center)
                .spacing(8),
            )
            .into();
        };

        let sidebar_pos = self.sidebar.position;

        let content = if id == self.main_window.id {
            let tooltip_position = if sidebar_pos == sidebar::Position::Left {
                TooltipPosition::Right
            } else {
                TooltipPosition::Left
            };

            let sidebar = {
                let is_table_open = self.tickers_table.is_open();

                let nav_buttons = {
                    let settings_modal_button = {
                        let is_active = self.sidebar.is_menu_active(sidebar::Menu::Settings)
                            || self.sidebar.is_menu_active(sidebar::Menu::ThemeEditor);

                        create_button(
                            icon_text(Icon::Cog, 14)
                                .width(24)
                                .align_x(Alignment::Center),
                            Message::ToggleSidebarMenu(sidebar::Menu::Settings),
                            None,
                            tooltip_position,
                            move |theme, status| {
                                style::button::transparent(theme, status, is_active)
                            },
                        )
                    };
                    let layout_modal_button = {
                        let is_active = self.sidebar.is_menu_active(sidebar::Menu::Layout);

                        create_button(
                            icon_text(Icon::Layout, 14)
                                .width(24)
                                .align_x(Alignment::Center),
                            Message::ToggleSidebarMenu(sidebar::Menu::Layout),
                            None,
                            tooltip_position,
                            move |theme, status| {
                                style::button::transparent(theme, status, is_active)
                            },
                        )
                    };
                    let ticker_search_button = {
                        create_button(
                            icon_text(Icon::Search, 14)
                                .width(24)
                                .align_x(Alignment::Center),
                            Message::TickersTable(tickers_table::Message::ToggleTable),
                            None,
                            tooltip_position,
                            move |theme, status| {
                                style::button::transparent(theme, status, is_table_open)
                            },
                        )
                    };

                    let audio_btn = {
                        let is_active = self.sidebar.is_menu_active(sidebar::Menu::Audio);

                        let icon = match self.audio_stream.volume().unwrap_or(0.0) {
                            v if v >= 40.0 => Icon::SpeakerHigh,
                            v if v > 0.0 => Icon::SpeakerLow,
                            _ => Icon::SpeakerOff,
                        };

                        create_button(
                            icon_text(icon, 14).width(24).align_x(Alignment::Center),
                            Message::ToggleSidebarMenu(sidebar::Menu::Audio),
                            None,
                            tooltip_position,
                            move |theme, status| {
                                style::button::transparent(theme, status, is_active)
                            },
                        )
                    };

                    column![
                        ticker_search_button,
                        layout_modal_button,
                        audio_btn,
                        Space::with_height(Length::Fill),
                        settings_modal_button,
                    ]
                    .width(32)
                    .spacing(8)
                };

                let tickers_table = {
                    if is_table_open {
                        column![responsive(move |size| {
                            self.tickers_table.view(size).map(Message::TickersTable)
                        })]
                        .width(200)
                    } else {
                        column![]
                    }
                };

                match sidebar_pos {
                    sidebar::Position::Left => {
                        row![nav_buttons, tickers_table,]
                    }
                    sidebar::Position::Right => {
                        row![tickers_table, nav_buttons,]
                    }
                }
                .spacing(if is_table_open { 8 } else { 4 })
            };

            let dashboard_view = dashboard
                .view(&self.main_window, self.timezone)
                .map(move |msg| Message::Dashboard(None, msg));

            let base = column![
                {
                    #[cfg(target_os = "macos")]
                    {
                        iced::widget::center(
                            text("FLOWSURFACE")
                                .font(iced::Font {
                                    weight: iced::font::Weight::Bold,
                                    ..Default::default()
                                })
                                .size(16)
                                .style(style::title_text)
                                .align_x(Alignment::Center),
                        )
                        .height(20)
                        .align_y(Alignment::Center)
                        .padding(padding::right(8).top(4))
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        column![]
                    }
                },
                match sidebar_pos {
                    sidebar::Position::Left => row![sidebar, dashboard_view,],
                    sidebar::Position::Right => row![dashboard_view, sidebar],
                }
                .spacing(4)
                .padding(8),
            ];

            match self.sidebar.active_menu {
                sidebar::Menu::Settings => {
                    let settings_modal = {
                        let theme_picklist = {
                            let mut themes: Vec<iced::Theme> = iced_core::Theme::ALL.to_vec();

                            let default_theme = iced_core::Theme::Custom(default_theme().into());
                            themes.push(default_theme);

                            if let Some(custom_theme) = self.theme_editor.custom_theme.clone() {
                                themes.push(custom_theme);
                            }

                            pick_list(themes, Some(self.theme.clone().0), |theme| {
                                Message::ThemeSelected(data::Theme(theme))
                            })
                        };

                        let toggle_theme_editor = button(text("Theme editor"))
                            .on_press(Message::ToggleSidebarMenu(sidebar::Menu::ThemeEditor));

                        let timezone_picklist = pick_list(
                            [data::UserTimezone::Utc, data::UserTimezone::Local],
                            Some(self.timezone),
                            Message::SetTimezone,
                        );

                        let sidebar_pos = pick_list(
                            [sidebar::Position::Left, sidebar::Position::Right],
                            Some(sidebar_pos),
                            Message::SetSidebarPosition,
                        );

                        let scale_factor = {
                            let current_value: f64 = self.scale_factor.into();

                            let decrease_btn = if current_value > 0.8 {
                                button(text("-")).on_press(Message::ScaleFactorChanged(
                                    (current_value - 0.1).into(),
                                ))
                            } else {
                                button(text("-"))
                            };

                            let increase_btn = if current_value < 1.8 {
                                button(text("+")).on_press(Message::ScaleFactorChanged(
                                    (current_value + 0.1).into(),
                                ))
                            } else {
                                button(text("+"))
                            };

                            container(
                                row![
                                    decrease_btn,
                                    text(format!("{:.0}%", current_value * 100.0)).size(14),
                                    increase_btn,
                                ]
                                .align_y(Alignment::Center)
                                .spacing(8)
                                .padding(4),
                            )
                            .style(style::modal_container)
                        };

                        let trade_fetch_checkbox = {
                            let is_active = dashboard.trade_fetch_enabled;

                            let checkbox =
                                iced::widget::checkbox("Fetch trades (Binance)", is_active)
                                    .on_toggle(|checked| {
                                        if checked {
                                            Message::ToggleDialogModal(Some((
                                        "This might be unreliable and take some time to complete"
                                            .to_string(),
                                        Box::new(Message::ToggleTradeFetch(true)),
                                    )))
                                        } else {
                                            Message::ToggleTradeFetch(false)
                                        }
                                    });

                            tooltip(
                                checkbox,
                                Some("Try to fetch trades for footprint charts"),
                                TooltipPosition::Top,
                            )
                        };

                        let open_data_folder = {
                            let button = button(text("Open data folder"))
                                .on_press(Message::DataFolderRequested);

                            tooltip(
                                button,
                                Some("Open the folder where the data & config is stored"),
                                TooltipPosition::Top,
                            )
                        };

                        container(
                            column![
                                column![open_data_folder,].spacing(8),
                                column![text("Sidebar position").size(14), sidebar_pos,].spacing(8),
                                column![text("Time zone").size(14), timezone_picklist,].spacing(8),
                                column![text("Theme").size(14), theme_picklist,].spacing(8),
                                column![text("Interface scale").size(14), scale_factor,].spacing(8),
                                column![
                                    text("Experimental").size(14),
                                    trade_fetch_checkbox,
                                    toggle_theme_editor
                                ]
                                .spacing(8),
                            ]
                            .spacing(20),
                        )
                        .align_x(Alignment::Start)
                        .max_width(400)
                        .padding(24)
                        .style(style::dashboard_modal)
                    };

                    let (align_x, padding) = match sidebar_pos {
                        sidebar::Position::Left => (Alignment::Start, padding::left(48).top(8)),
                        sidebar::Position::Right => (Alignment::End, padding::right(48).top(8)),
                    };

                    let base_content = dashboard_modal(
                        base,
                        settings_modal,
                        Message::ToggleSidebarMenu(sidebar::Menu::None),
                        padding,
                        Alignment::End,
                        align_x,
                    );

                    if let Some((dialog, on_confirm)) = &self.confirm_dialog {
                        let dialog_content = confirm_dialog_container(
                            dialog,
                            *on_confirm.to_owned(),
                            Message::ToggleDialogModal(None),
                        );

                        main_dialog_modal(
                            base_content,
                            dialog_content,
                            Message::ToggleDialogModal(None),
                        )
                    } else {
                        base_content
                    }
                }
                sidebar::Menu::Layout => {
                    let reset_pane_button = tooltip(
                        button(text("Reset").align_x(Alignment::Center))
                            .width(iced::Length::Fill)
                            .on_press(Message::Dashboard(
                                None,
                                dashboard::Message::Pane(
                                    id,
                                    pane::Message::ReplacePane(
                                        if let Some(focus) = dashboard.focus {
                                            focus.1
                                        } else {
                                            *dashboard.panes.iter().next().unwrap().0
                                        },
                                    ),
                                ),
                            )),
                        Some("Reset selected pane"),
                        TooltipPosition::Top,
                    );
                    let split_pane_button = tooltip(
                        button(text("Split").align_x(Alignment::Center))
                            .width(iced::Length::Fill)
                            .on_press(Message::Dashboard(
                                None,
                                dashboard::Message::Pane(
                                    id,
                                    pane::Message::SplitPane(
                                        pane_grid::Axis::Horizontal,
                                        if let Some(focus) = dashboard.focus {
                                            focus.1
                                        } else {
                                            *dashboard.panes.iter().next().unwrap().0
                                        },
                                    ),
                                ),
                            )),
                        Some("Split selected pane horizontally"),
                        TooltipPosition::Top,
                    );

                    let manage_layout_modal = {
                        container(
                            column![
                                column![
                                    text("Panes").size(14),
                                    if dashboard.focus.is_some() {
                                        row![reset_pane_button, split_pane_button,]
                                            .padding(padding::left(8).right(8))
                                            .spacing(8)
                                    } else {
                                        row![text("No pane selected"),]
                                    },
                                ]
                                .align_x(Alignment::Center)
                                .spacing(8),
                                column![
                                    text("Layouts").size(14),
                                    self.layout_manager.view().map(Message::Layouts),
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

                    let (align_x, padding) = match sidebar_pos {
                        sidebar::Position::Left => (Alignment::Start, padding::left(48).top(40)),
                        sidebar::Position::Right => (Alignment::End, padding::right(48).top(40)),
                    };

                    dashboard_modal(
                        base,
                        manage_layout_modal,
                        Message::ToggleSidebarMenu(sidebar::Menu::None),
                        padding,
                        Alignment::Start,
                        align_x,
                    )
                }
                sidebar::Menu::Audio => {
                    let (align_x, padding) = match sidebar_pos {
                        sidebar::Position::Left => (Alignment::Start, padding::left(48).top(64)),
                        sidebar::Position::Right => (Alignment::End, padding::right(48).top(64)),
                    };

                    let depth_streams_list: Vec<(Exchange, exchange::Ticker)> = dashboard
                        .pane_streams
                        .iter()
                        .flat_map(|(exchange, streams)| {
                            streams
                                .iter()
                                .filter(|(ticker, stream_types)| {
                                    stream_types.contains(&StreamType::DepthAndTrades {
                                        exchange: *exchange,
                                        ticker: **ticker,
                                    })
                                })
                                .map(|(ticker, _)| (*exchange, *ticker))
                        })
                        .collect();

                    dashboard_modal(
                        base,
                        self.audio_stream
                            .view(depth_streams_list)
                            .map(Message::AudioStream),
                        Message::ToggleSidebarMenu(sidebar::Menu::None),
                        padding,
                        Alignment::Start,
                        align_x,
                    )
                }
                sidebar::Menu::ThemeEditor => {
                    let (align_x, padding) = match sidebar_pos {
                        sidebar::Position::Left => (Alignment::Start, padding::left(48).top(8)),
                        sidebar::Position::Right => (Alignment::End, padding::right(48).top(8)),
                    };

                    dashboard_modal(
                        base,
                        self.theme_editor
                            .view(&self.theme.0)
                            .map(Message::ThemeEditor),
                        Message::ToggleSidebarMenu(sidebar::Menu::None),
                        padding,
                        Alignment::End,
                        align_x,
                    )
                }
                sidebar::Menu::None => base.into(),
            }
        } else {
            container(
                dashboard
                    .view_window(id, &self.main_window, self.timezone)
                    .map(move |msg| Message::Dashboard(None, msg)),
            )
            .padding(padding::top(style::TITLE_PADDING_TOP))
            .into()
        };

        notification::Manager::new(
            content,
            &self.notifications,
            match sidebar_pos {
                sidebar::Position::Left => Alignment::End,
                sidebar::Position::Right => Alignment::Start,
            },
            Message::DeleteNotification,
        )
        .into()
    }

    fn theme(&self, _window: window::Id) -> iced_core::Theme {
        self.theme.clone().into()
    }

    fn title(&self, _window: window::Id) -> String {
        format!("Flowsurface [{}]", self.layout_manager.active_layout.name)
    }

    fn scale_factor(&self, _window: window::Id) -> f64 {
        self.scale_factor.into()
    }

    fn subscription(&self) -> Subscription<Message> {
        let window_events = window_events().map(Message::WindowEvent);

        let Some(dashboard) = self.active_dashboard() else {
            return window_events;
        };

        let exchange_streams = dashboard.market_subscriptions(Message::MarketWsEvent);

        let tickers_table_fetch = self.tickers_table.subscription().map(Message::TickersTable);

        Subscription::batch(vec![exchange_streams, tickers_table_fetch, window_events])
    }

    fn active_dashboard(&self) -> Option<&Dashboard> {
        self.layout_manager.active_dashboard()
    }

    fn active_dashboard_mut(&mut self) -> Option<&mut Dashboard> {
        self.layout_manager.active_dashboard_mut()
    }
}
