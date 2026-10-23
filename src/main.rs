#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod connector;
mod layout;
mod logger;
mod modal;
mod notify;
mod screen;
mod style;
mod theme;
mod version;
mod widget;
mod window;

use connector::client::DataSources;
use data::config::theme::default_theme;
use data::{layout::WindowSpec, sidebar};
use layout::{LayoutId, configuration};
use modal::dashboard_modal;
use modal::{
    LayoutManager, ThemeEditor,
    audio::AudioStream,
    network_editor::{self, NetworkEditor},
};
use notify::Notifications;
use screen::dashboard::{self, Dashboard};
use theme::ThemeState;
use widget::{
    toast::{self, Toast},
    tooltip,
};

use iced::{
    Alignment, Element, Subscription, Task, keyboard, padding,
    widget::{
        button, column, container, pane_grid, pick_list, row, rule, scrollable, text,
        tooltip::Position as TooltipPosition,
    },
};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

fn main() {
    logger::install_panic_hook();

    if let Err(err) = logger::setup(cfg!(debug_assertions)) {
        logger::report_stderr(&format!("Failed to initialize logger: {err}"));
    }

    if let Err(error) = data::initialize_indicator_catalog() {
        logger::report_stderr(&error);
        std::process::exit(1);
    }

    std::thread::spawn(data::cleanup_old_market_data);

    let daemon = iced::daemon(Flowsurface::new, Flowsurface::update, Flowsurface::view)
        .settings(iced::Settings {
            antialiasing: data::load_antialiasing(),
            fonts: vec![
                Cow::Borrowed(style::AZERET_MONO_BYTES),
                Cow::Borrowed(style::ICONS_BYTES),
            ],
            default_text_size: style::text_size::BODY.into(),
            ..Default::default()
        })
        .title(Flowsurface::title)
        .theme(Flowsurface::theme)
        .scale_factor(Flowsurface::scale_factor)
        .subscription(Flowsurface::subscription);

    if let Err(err) = daemon.run() {
        let message = format!("Runtime error: {err}");
        log::error!("{message}");
        logger::report_stderr(&message);
    }
}

struct Flowsurface {
    main_window: window::Window,
    sidebar: screen::sidebar::Sidebar,
    layout_manager: LayoutManager,
    theme_editor: ThemeEditor,
    network_editor: NetworkEditor,
    network_config: data::Network,
    data_sources: Arc<DataSources>,
    audio_stream: AudioStream,
    volume_size_unit: exchange::SizeUnit,
    applied_volume_size_unit: exchange::SizeUnit,
    ui_scale_factor: data::ScaleFactor,
    timezone: data::UserTimezone,
    theme: ThemeState,
    notifications: Notifications,
    window_scale_factors: window::WindowScaleFactors,
    antialiasing: bool,
    applied_antialiasing: bool,
}

#[derive(Debug, Clone)]
enum Message {
    Sidebar(screen::sidebar::Message),
    MarketWsEvent(exchange::Event),
    Dashboard {
        /// If `None`, the active layout is used for the event.
        layout_id: Option<uuid::Uuid>,
        event: dashboard::Message,
    },
    Tick(std::time::Instant),
    WindowEvent(window::Event),
    ExitRequested(HashMap<window::Id, WindowSpec>),
    SaveStateRequested(HashMap<window::Id, WindowSpec>),
    GoBack,
    DataFolderRequested,
    OpenUrlRequested(Cow<'static, str>),
    ThemeSelected(iced_core::Theme),
    ScaleFactorChanged(data::ScaleFactor),
    SetTimezone(data::UserTimezone),
    SetMetadataCaching(bool),
    RefreshMetadata,
    ApplyVolumeSizeUnit(exchange::SizeUnit),
    ApplyAntialiasing(bool),
    RemoveNotification(usize),
    WindowScaleFactorUpdated {
        window: window::Id,
        scale_factor: window::WindowScaleFactor,
    },
    ThemeEditor(modal::theme_editor::Message),
    NetworkEditor(modal::network_editor::Message),
    Layouts(modal::layout_manager::Message),
    AudioStream(modal::audio::Message),
}

impl Flowsurface {
    fn new() -> (Self, Task<Message>) {
        let saved_state = layout::load_saved_state();

        let (main_window_id, open_main_window) = {
            let (position, size) = saved_state.window();
            let config = window::Settings {
                size,
                position,
                exit_on_close_request: false,
                ..window::settings()
            };
            window::open(config)
        };

        let data_sources = Arc::new(DataSources::new(&saved_state.network));

        let (sidebar, launch_sidebar) = screen::sidebar::Sidebar::new(
            &saved_state.sidebar,
            saved_state.cache_market_metadata,
            data_sources.exchange.clone(),
        );

        let (audio_stream, audio_init_err) = AudioStream::new(saved_state.audio_cfg);

        let mut state = Self {
            main_window: window::Window::new(main_window_id),
            layout_manager: saved_state.layout_manager,
            theme_editor: ThemeEditor::new(saved_state.custom_theme),
            window_scale_factors: window::WindowScaleFactors::new(main_window_id),
            audio_stream,
            sidebar,
            data_sources,
            timezone: saved_state.timezone,
            ui_scale_factor: saved_state.scale_factor,
            volume_size_unit: saved_state.volume_size_unit,
            applied_volume_size_unit: saved_state.volume_size_unit,
            theme: ThemeState::new(saved_state.theme),
            notifications: Notifications::new(),
            network_config: saved_state.network.clone(),
            network_editor: NetworkEditor::new(&saved_state.network, None),
            antialiasing: saved_state.antialiasing,
            applied_antialiasing: saved_state.antialiasing,
        };

        for diagnostic in data::chart::kline::indicators::catalog().diagnostics() {
            state.notifications.push(Toast::warn(format!(
                "Indicator source quarantined: {diagnostic}"
            )));
        }

        if let Some(err) = audio_init_err {
            state
                .notifications
                .push(Toast::error(format!("Audio disabled: {err}")));
        }

        if state.layout_manager.layouts.is_empty() {
            log::error!("No layouts available after loading state; creating a default layout");
            state.layout_manager.ensure_layout();
        }

        let active_layout_id = state
            .layout_manager
            .active_layout_id()
            .or_else(|| {
                state
                    .layout_manager
                    .layouts
                    .first()
                    .map(|layout| &layout.id)
            })
            .map(|layout| layout.unique);

        let load_layout = active_layout_id
            .map(|uid| state.load_layout(uid, main_window_id))
            .unwrap_or_else(|| {
                log::error!("No active layout could be selected at startup");
                Task::none()
            });

        let open_main_window = open_main_window.then(|window_id| {
            window::scale_factor(window_id, |window, scale_factor| {
                Message::WindowScaleFactorUpdated {
                    window,
                    scale_factor,
                }
            })
        });

        (
            state,
            open_main_window
                .chain(load_layout)
                .chain(launch_sidebar.map(Message::Sidebar)),
        )
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::MarketWsEvent(event) => {
                let main_window_id = self.main_window.id;
                if self.layout_manager.active_layout_id().is_none() {
                    return Task::none();
                }
                let dashboard = self.active_dashboard_mut();

                match event {
                    exchange::Event::Connected(_streams) => {}
                    exchange::Event::Disconnected(_streams, reason) => {
                        log::info!("a stream disconnected from WS: {reason:?}");
                    }
                    exchange::Event::DepthReceived(stream, update_t, depth) => {
                        let task =
                            dashboard.ingest_live_depth(&stream, update_t, depth, main_window_id);
                        return self.map_dashboard_task(task, None);
                    }
                    exchange::Event::TradesReceived(stream, update_t, buffer) => {
                        let task = dashboard.ingest_live_trades(
                            &stream,
                            &buffer,
                            update_t,
                            main_window_id,
                        );

                        if let Some(msg) = self.audio_stream.try_play_sound(&stream, &buffer) {
                            self.notifications.push(Toast::error(msg));
                        }

                        return self.map_dashboard_task(task, None);
                    }
                    exchange::Event::KlineReceived(stream, kline) => {
                        let task = dashboard.ingest_live_kline(&stream, &kline, main_window_id);
                        return self.map_dashboard_task(task, None);
                    }
                }
            }
            Message::Tick(now) => {
                let main_window_id = self.main_window.id;
                let data_sources = Arc::clone(&self.data_sources);
                if self.layout_manager.active_layout_id().is_none() {
                    return Task::none();
                }

                let task = self
                    .active_dashboard_mut()
                    .tick(now, &data_sources, main_window_id);
                return self.map_dashboard_task(task, None);
            }
            Message::WindowEvent(event) => return self.handle_window_event(event),
            Message::WindowScaleFactorUpdated {
                window,
                scale_factor,
            } => {
                self.window_scale_factors.set(window, scale_factor);
            }
            Message::ExitRequested(windows) => {
                self.save_state_to_disk(&windows);
                return iced::exit();
            }
            Message::SaveStateRequested(windows) => {
                self.save_state_to_disk(&windows);
            }
            Message::GoBack => {
                let main_window = self.main_window.id;

                if self.sidebar.active_menu().is_some() {
                    self.sidebar.set_menu(None);
                } else {
                    if self.active_dashboard_mut().go_back(main_window) {
                        return Task::none();
                    } else if self.active_dashboard().focus.is_some() {
                        self.active_dashboard_mut().clear_focus();
                    } else {
                        self.sidebar.hide_tickers_table();
                    }
                }
            }
            Message::ThemeSelected(theme) => {
                self.theme_editor.reset_input();
                self.replace_theme(theme);
            }
            Message::Dashboard {
                layout_id: id,
                event: msg,
            } => {
                let Some(active_layout) = self.layout_manager.active_layout_id() else {
                    log::error!("No active layout to handle dashboard message");
                    return Task::none();
                };

                let main_window = self.main_window;
                let active_layout_id = active_layout.unique;
                let layout_id = id.unwrap_or(active_layout_id);

                let Some((main_task, event)) = self.layout_manager.update_dashboard(
                    msg,
                    &main_window,
                    &layout_id,
                    &self.data_sources,
                ) else {
                    return Task::none();
                };

                let additional_task = match event {
                    Some(dashboard::Event::OpenUrlRequested(url)) => {
                        Task::done(Message::OpenUrlRequested(Cow::Owned(url)))
                    }
                    Some(dashboard::Event::DistributeFetchedData {
                        pane_id,
                        data,
                        stream,
                    }) => {
                        let task = self
                            .layout_manager
                            .mut_dashboard(layout_id)
                            .map(|dashboard| {
                                dashboard.distribute_fetched_data(
                                    main_window.id,
                                    pane_id,
                                    data,
                                    stream,
                                )
                            })
                            .unwrap_or_else(Task::none);
                        self.map_dashboard_task(task, Some(layout_id))
                    }
                    Some(dashboard::Event::Notification(toast)) => {
                        self.notifications.push(toast);
                        Task::none()
                    }
                    Some(dashboard::Event::ResolveStreams { pane_id, streams }) => {
                        let tickers_info = self.sidebar.tickers_info();

                        let resolved_streams =
                            streams.into_iter().try_fold(vec![], |mut acc, persist| {
                                let resolver =
                                    |t: &exchange::Ticker| tickers_info.get(t).and_then(|opt| *opt);

                                match persist.into_stream_kinds(resolver) {
                                    Ok(mut resolved) => {
                                        acc.append(&mut resolved);
                                        Ok(acc)
                                    }
                                    Err(err) => Err(err),
                                }
                            });

                        match resolved_streams {
                            Ok(resolved) => {
                                if resolved.is_empty() {
                                    Task::none()
                                } else {
                                    let task = self
                                        .layout_manager
                                        .mut_dashboard(layout_id)
                                        .map(|dashboard| {
                                            dashboard.resolve_streams(
                                                main_window.id,
                                                pane_id,
                                                resolved,
                                            )
                                        })
                                        .unwrap_or_else(Task::none);
                                    self.map_dashboard_task(task, Some(layout_id))
                                }
                            }
                            Err(err) => {
                                if self.sidebar.is_metadata_loading() {
                                    // Metadata fetches are still in flight
                                    log::debug!(
                                        "Deferring stream resolution for pane {pane_id}: metadata still loading ({err})"
                                    );
                                } else if let Some(dashboard) =
                                    self.layout_manager.mut_dashboard(layout_id)
                                {
                                    log::debug!("Blocking streams for pane {pane_id}: {err}");
                                    dashboard.block_streams(
                                        main_window.id,
                                        pane_id,
                                        format!("Metadata not available: {err}"),
                                    );
                                }
                                Task::none()
                            }
                        }
                    }
                    Some(dashboard::Event::OpenNetworkSettings) => {
                        return self.update(Message::Sidebar(
                            screen::sidebar::Message::ToggleSidebarMenu(Some(
                                sidebar::Menu::Network,
                            )),
                        ));
                    }
                    Some(dashboard::Event::RequestPalette) => {
                        let theme = self.theme.current().clone();

                        if let Some(dashboard) = self.layout_manager.mut_dashboard(layout_id) {
                            dashboard.theme_updated(main_window.id, &theme);
                        }

                        Task::none()
                    }
                    Some(dashboard::Event::ChartPresets {
                        window,
                        pane,
                        command,
                    }) => {
                        self.layout_manager.update_presets(
                            command,
                            main_window.id,
                            layout_id,
                            window,
                            pane,
                        );

                        Task::none()
                    }
                    None => Task::none(),
                };

                return self
                    .map_dashboard_task(main_task, Some(layout_id))
                    .chain(additional_task);
            }
            Message::RemoveNotification(index) => {
                self.notifications.remove(index);
            }
            Message::SetTimezone(tz) => {
                self.timezone = tz;
            }
            Message::SetMetadataCaching(enabled) => {
                self.sidebar.set_cache_enabled(enabled);
            }
            Message::RefreshMetadata => {
                if let Some(task) = self.sidebar.force_refresh_metadata() {
                    return task.map(Message::Sidebar);
                }
            }
            Message::ScaleFactorChanged(value) => {
                self.ui_scale_factor = value;
            }
            Message::Layouts(message) => {
                let action = self.layout_manager.update(message);

                match action {
                    Some(modal::layout_manager::Action::Select(layout)) => {
                        let active_popout_keys = self
                            .active_dashboard()
                            .popout
                            .keys()
                            .copied()
                            .collect::<Vec<_>>();

                        let window_tasks = Task::batch(
                            active_popout_keys
                                .iter()
                                .map(|&popout_id| window::close::<window::Id>(popout_id))
                                .collect::<Vec<_>>(),
                        )
                        .discard();

                        let old_layout_id = self
                            .layout_manager
                            .active_layout_id()
                            .map(|layout| layout.unique)
                            .expect("No active layout");

                        return self
                            .map_dashboard_task(
                                window::collect_window_specs(
                                    active_popout_keys,
                                    dashboard::Message::SavePopoutSpecs,
                                ),
                                Some(old_layout_id),
                            )
                            .chain(window_tasks)
                            .chain(self.load_layout(layout, self.main_window.id));
                    }
                    Some(modal::layout_manager::Action::Clone(id)) => {
                        let manager = &mut self.layout_manager;

                        let source_data = manager.get(id).map(|layout| {
                            (
                                layout.id.name.clone(),
                                data::Dashboard::from(&layout.dashboard),
                            )
                        });

                        if let Some((name, ser_dashboard)) = source_data {
                            let new_uid = uuid::Uuid::new_v4();
                            let new_layout = LayoutId {
                                unique: new_uid,
                                name: manager.ensure_unique_name(&name, new_uid),
                            };

                            let mut popout_windows = Vec::new();

                            for (pane, window_spec) in &ser_dashboard.popout {
                                let configuration = configuration(pane.clone());
                                popout_windows.push((configuration, *window_spec));
                            }

                            let dashboard = Dashboard::from_config(
                                configuration(ser_dashboard.pane.clone()),
                                popout_windows,
                                ser_dashboard.chart_settings,
                            );

                            manager.insert_layout(new_layout.clone(), dashboard);
                        }
                    }
                    None => {}
                }
            }
            Message::AudioStream(message) => {
                if let Some(event) = self.audio_stream.update(message) {
                    match event {
                        modal::audio::UpdateEvent::RetryFailed(err) => {
                            self.notifications
                                .push(Toast::error(format!("Audio still unavailable: {err}")));
                        }
                        modal::audio::UpdateEvent::RetrySucceeded => {
                            self.notifications.push(Toast::info(
                                "Audio output re-initialized successfully".to_string(),
                            ));
                        }
                    }
                }
            }
            Message::DataFolderRequested => {
                if let Err(err) = data::open_data_folder() {
                    self.notifications
                        .push(Toast::error(format!("Failed to open data folder: {err}")));
                }
            }
            Message::OpenUrlRequested(url) => {
                if let Err(err) = data::open_url(url.as_ref()) {
                    self.notifications
                        .push(Toast::error(format!("Failed to open link: {err}")));
                }
            }
            Message::ThemeEditor(msg) => {
                let action = self.theme_editor.update(msg, self.theme.current());

                match action {
                    Some(modal::theme_editor::Action::Exit) => {
                        self.sidebar.set_menu(Some(sidebar::Menu::Settings));
                    }
                    Some(modal::theme_editor::Action::UpdateTheme(theme)) => {
                        self.replace_theme(theme);
                    }
                    None => {}
                }
            }
            Message::NetworkEditor(msg) => {
                let action = self.network_editor.update(msg);

                match action {
                    Some(network_editor::Action::ApplyProxy(ref proxy)) => {
                        if let Some(proxy) = proxy {
                            data::config::auth::save_proxy_auth(proxy);
                        } else if let Some(ref old_proxy) = self.network_config.proxy {
                            data::config::auth::delete_proxy_auth(old_proxy);
                        }
                        self.network_config.proxy = proxy.clone();

                        let main_window = self.main_window.id;
                        let dashboard = self.active_dashboard_mut();

                        let mut active_windows = dashboard
                            .popout
                            .keys()
                            .copied()
                            .collect::<Vec<window::Id>>();
                        active_windows.push(main_window);

                        return window::collect_window_specs(
                            active_windows,
                            Message::SaveStateRequested,
                        );
                    }
                    Some(network_editor::Action::ApplyServerConfig {
                        mode,
                        url,
                        auth_token,
                    }) => {
                        if let Some(ref url) = url
                            && let Some(ref token) = auth_token
                        {
                            data::config::auth::save_server_token(url, token);
                        } else if let Some(ref old_url) = self.network_config.server_url {
                            data::config::auth::delete_server_token(old_url);
                        }
                        self.network_config.server_url = url;
                        self.network_config.server_auth_token = auth_token;
                        self.network_config.trade_fetch_mode = mode;

                        let main_window = self.main_window.id;
                        let dashboard = self.active_dashboard_mut();

                        let mut active_windows = dashboard
                            .popout
                            .keys()
                            .copied()
                            .collect::<Vec<window::Id>>();
                        active_windows.push(main_window);

                        return window::collect_window_specs(
                            active_windows,
                            Message::SaveStateRequested,
                        );
                    }
                    Some(network_editor::Action::Exit) => {
                        self.sidebar.set_menu(Some(sidebar::Menu::Settings));
                    }
                    None => {}
                }
            }
            Message::Sidebar(message) => {
                let (task, action) = self.sidebar.update(message);

                match action {
                    Some(screen::sidebar::Action::MenuChanged(Some(sidebar::Menu::Network))) => {
                        self.network_editor = NetworkEditor::new(
                            &self.network_config,
                            self.network_editor.pending_apply().cloned(),
                        );
                    }
                    Some(screen::sidebar::Action::MenuChanged(_)) => {}
                    Some(screen::sidebar::Action::TickerSelected(ticker_info, content)) => {
                        let main_window_id = self.main_window.id;
                        let data_sources = Arc::clone(&self.data_sources);
                        if self.layout_manager.active_layout_id().is_none() {
                            return task.map(Message::Sidebar);
                        }

                        let dashboard_task = {
                            if let Some(kind) = content {
                                self.active_dashboard_mut().init_focused_pane(
                                    &data_sources,
                                    main_window_id,
                                    ticker_info,
                                    kind,
                                )
                            } else {
                                self.active_dashboard_mut().switch_tickers_in_group(
                                    &data_sources,
                                    main_window_id,
                                    ticker_info,
                                )
                            }
                        };

                        return self.map_dashboard_task(dashboard_task, None);
                    }
                    Some(screen::sidebar::Action::ErrorOccurred(err)) => {
                        self.notifications.push(Toast::error(err.to_string()));
                    }
                    None => {}
                }

                return task.map(Message::Sidebar);
            }
            Message::ApplyVolumeSizeUnit(pref) => {
                self.volume_size_unit = pref;

                let mut active_windows: Vec<window::Id> =
                    self.active_dashboard().popout.keys().copied().collect();
                active_windows.push(self.main_window.id);

                return window::collect_window_specs(active_windows, |windows| {
                    Message::SaveStateRequested(windows)
                });
            }
            Message::ApplyAntialiasing(enabled) => {
                self.antialiasing = enabled;

                let mut active_windows: Vec<window::Id> =
                    self.active_dashboard().popout.keys().copied().collect();
                active_windows.push(self.main_window.id);

                return window::collect_window_specs(active_windows, |windows| {
                    Message::SaveStateRequested(windows)
                });
            }
        }
        Task::none()
    }

    fn view(&self, id: window::Id) -> Element<'_, Message> {
        let dashboard = self.active_dashboard();
        let sidebar_pos = self.sidebar.position();

        let pixel_ratio = self
            .window_scale_factors
            .pixel_ratio_for_window(id, self.ui_scale_factor);

        let tickers_table = &self.sidebar.tickers_table;

        let content = if id == self.main_window.id {
            let sidebar_view = self
                .sidebar
                .view(self.audio_stream.volume())
                .map(Message::Sidebar);
            let dashboard_view = dashboard
                .view(
                    &self.main_window,
                    tickers_table,
                    self.timezone,
                    pixel_ratio,
                    &self.theme,
                    self.layout_manager.presets(),
                )
                .map(move |msg| Message::Dashboard {
                    layout_id: None,
                    event: msg,
                });

            let header_title = {
                #[cfg(target_os = "macos")]
                {
                    iced::widget::center(
                        text("FLOWSURFACE")
                            .font(iced::Font {
                                weight: iced::font::Weight::Bold,
                                ..Default::default()
                            })
                            .size(crate::style::text_size::TITLE)
                            .style(style::title_text),
                    )
                    .height(20)
                    .align_y(Alignment::Center)
                    .padding(padding::top(4))
                }
                #[cfg(not(target_os = "macos"))]
                {
                    column![]
                }
            };

            let dashboard_row = match sidebar_pos {
                sidebar::Position::Left => row![sidebar_view, dashboard_view],
                sidebar::Position::Right => row![dashboard_view, sidebar_view],
            };

            let base = column![header_title, dashboard_row.spacing(4).padding(8),];

            if let Some(menu) = self.sidebar.active_menu() {
                self.view_with_modal(base.into(), dashboard, menu)
            } else {
                base.into()
            }
        } else {
            container(
                dashboard
                    .view_window(
                        id,
                        &self.main_window,
                        tickers_table,
                        self.timezone,
                        pixel_ratio,
                        &self.theme,
                        self.layout_manager.presets(),
                    )
                    .map(move |msg| Message::Dashboard {
                        layout_id: None,
                        event: msg,
                    }),
            )
            .padding(padding::top(style::TITLE_PADDING_TOP))
            .into()
        };

        toast::Manager::new(
            content,
            self.notifications.toasts(),
            match sidebar_pos {
                sidebar::Position::Left => Alignment::Start,
                sidebar::Position::Right => Alignment::End,
            },
            Message::RemoveNotification,
        )
        .into()
    }

    fn theme(&self, _window: window::Id) -> iced_core::Theme {
        self.theme.current().clone()
    }

    fn title(&self, _window: window::Id) -> String {
        if let Some(id) = self.layout_manager.active_layout_id() {
            format!("Flowsurface [{}]", id.name)
        } else {
            "Flowsurface".to_string()
        }
    }

    fn scale_factor(&self, _window: window::Id) -> f32 {
        self.ui_scale_factor.into()
    }

    fn subscription(&self) -> Subscription<Message> {
        let window_events = window::events().map(Message::WindowEvent);
        let sidebar = self.sidebar.subscription().map(Message::Sidebar);

        let exchange_streams = self
            .active_dashboard()
            .market_subscriptions(&self.data_sources.exchange)
            .map(Message::MarketWsEvent);

        let tick = iced::window::frames().map(Message::Tick);

        let hotkeys = keyboard::listen().filter_map(|event| {
            let keyboard::Event::KeyPressed { key, .. } = event else {
                return None;
            };
            match key {
                keyboard::Key::Named(keyboard::key::Named::Escape) => Some(Message::GoBack),
                _ => None,
            }
        });

        Subscription::batch(vec![
            exchange_streams,
            sidebar,
            window_events,
            tick,
            hotkeys,
        ])
    }

    fn active_dashboard(&self) -> &Dashboard {
        let active_layout = self
            .layout_manager
            .active_layout_id()
            .expect("No active layout");
        self.layout_manager
            .get(active_layout.unique)
            .map(|layout| &layout.dashboard)
            .expect("No active dashboard")
    }

    fn active_dashboard_mut(&mut self) -> &mut Dashboard {
        let active_layout = self
            .layout_manager
            .active_layout_id()
            .expect("No active layout");
        self.layout_manager
            .get_mut(active_layout.unique)
            .map(|layout| &mut layout.dashboard)
            .expect("No active dashboard")
    }

    fn map_dashboard_task(
        &self,
        task: Task<dashboard::Message>,
        layout_id: Option<uuid::Uuid>,
    ) -> Task<Message> {
        let Some(layout_id) = layout_id.or_else(|| {
            self.layout_manager
                .active_layout_id()
                .map(|layout| layout.unique)
        }) else {
            return Task::none();
        };

        task.map(move |event| Message::Dashboard {
            layout_id: Some(layout_id),
            event,
        })
    }

    fn replace_theme(&mut self, theme: iced_core::Theme) {
        if !self.theme.replace(theme.clone()) {
            return;
        }

        let main_window = self.main_window.id;
        self.active_dashboard_mut()
            .theme_updated(main_window, &theme);
    }

    fn handle_window_event(&mut self, event: window::Event) -> Task<Message> {
        match event {
            window::Event::Opened(window_id) => {
                window::scale_factor(window_id, |window, scale_factor| {
                    Message::WindowScaleFactorUpdated {
                        window,
                        scale_factor,
                    }
                })
            }
            window::Event::Rescaled(window_id, scale_factor) => {
                self.window_scale_factors.set(window_id, scale_factor);
                Task::none()
            }
            window::Event::CloseRequested(window_id) => {
                let main_window = self.main_window.id;

                if window_id != main_window {
                    self.window_scale_factors.remove(window_id);
                    self.active_dashboard_mut().popout.remove(&window_id);
                    return window::close(window_id);
                }

                let dashboard = self.active_dashboard_mut();

                let mut active_windows = dashboard
                    .popout
                    .keys()
                    .copied()
                    .collect::<Vec<window::Id>>();
                active_windows.push(main_window);

                window::collect_window_specs(active_windows, Message::ExitRequested)
            }
        }
    }

    fn load_layout(&mut self, layout_uid: uuid::Uuid, main_window: window::Id) -> Task<Message> {
        if let Err(err) = self.layout_manager.set_active_layout(layout_uid) {
            log::error!("Failed to set active layout: {}", err);
            return Task::none();
        }

        self.layout_manager
            .park_inactive_layouts(layout_uid, main_window);

        let task = self
            .layout_manager
            .get_mut(layout_uid)
            .map(|layout| layout.dashboard.load_layout(main_window));

        task.map(|task| self.map_dashboard_task(task, Some(layout_uid)))
            .unwrap_or_else(|| {
                log::error!("Active layout missing after selection: {}", layout_uid);
                Task::none()
            })
    }

    fn view_with_modal<'a>(
        &'a self,
        base: Element<'a, Message>,
        dashboard: &'a Dashboard,
        menu: sidebar::Menu,
    ) -> Element<'a, Message> {
        let sidebar_pos = self.sidebar.position();

        match menu {
            sidebar::Menu::Settings => {
                let settings_modal = {
                    let theme_picklist = {
                        let mut themes: Vec<iced::Theme> = iced_core::Theme::ALL.to_vec();

                        let default_theme = iced_core::Theme::Custom(default_theme().into());
                        themes.push(default_theme);

                        if let Some(custom_theme) = &self.theme_editor.custom_theme {
                            themes.push(custom_theme.clone());
                        }

                        pick_list(themes, Some(self.theme.current().clone()), |theme| {
                            Message::ThemeSelected(theme)
                        })
                    };

                    let toggle_theme_editor = button(text("Theme editor")).on_press(
                        Message::Sidebar(screen::sidebar::Message::ToggleSidebarMenu(Some(
                            sidebar::Menu::ThemeEditor,
                        ))),
                    );

                    let toggle_network_editor = button(text("Network")).on_press(Message::Sidebar(
                        screen::sidebar::Message::ToggleSidebarMenu(Some(sidebar::Menu::Network)),
                    ));

                    let timezone_picklist = pick_list(
                        [data::UserTimezone::Utc, data::UserTimezone::Local],
                        Some(self.timezone),
                        Message::SetTimezone,
                    );

                    let restart_required_marker = || {
                        let btn = button(text("!").style(|theme: &iced::Theme| {
                            iced::widget::text::Style {
                                color: Some(theme.extended_palette().warning.base.color),
                            }
                        }))
                        .style(move |theme, status| style::button::modifier(theme, status, true));
                        tooltip(btn, Some("Restart required"), TooltipPosition::Top)
                    };

                    let size_in_quote_ccy_checkbox = {
                        let is_active = match self.volume_size_unit {
                            exchange::SizeUnit::Quote => true,
                            exchange::SizeUnit::Base => false,
                        };
                        let restart_required =
                            self.volume_size_unit != self.applied_volume_size_unit;

                        let checkbox = iced::widget::checkbox(is_active)
                            .label("Size in quote currency")
                            .on_toggle(|checked| {
                                Message::ApplyVolumeSizeUnit(if checked {
                                    exchange::SizeUnit::Quote
                                } else {
                                    exchange::SizeUnit::Base
                                })
                            });

                        let checkbox = tooltip(
                            checkbox,
                            Some(
                                "Display sizes/volumes in quote currency (USD).\n- Has no effect on inverse perps or open interest\n- Restart required to apply.",
                            ),
                            TooltipPosition::Top,
                        );

                        let mut row = row![checkbox]
                            .align_y(iced::Alignment::Center)
                            .spacing(8)
                            .height(24);
                        if restart_required {
                            row = row.push(restart_required_marker());
                        }
                        row
                    };

                    let cache_metadata_checkbox = {
                        let cache_enabled = self.sidebar.cache_enabled();

                        let checkbox = tooltip(
                            iced::widget::checkbox(cache_enabled)
                                .label("Cache ticker metadata")
                                .on_toggle(Message::SetMetadataCaching),
                            Some("Cache ticker metadata for faster startup."),
                            TooltipPosition::Top,
                        );

                        if !cache_enabled {
                            column![row![checkbox].height(24).align_y(iced::Alignment::Center)]
                        } else {
                            let loading = self.sidebar.is_metadata_loading();

                            let refresh_btn = {
                                let btn = button(crate::style::icon_text(
                                    crate::style::Icon::Refresh,
                                    12,
                                ))
                                .style(move |theme, status| {
                                    style::button::modifier(theme, status, loading)
                                });
                                let btn = if loading {
                                    btn
                                } else {
                                    btn.on_press(Message::RefreshMetadata)
                                };

                                tooltip(
                                    btn,
                                    if loading { None } else { Some("Refresh now") },
                                    TooltipPosition::Top,
                                )
                            };

                            let last_updated = {
                                let last_update = self.sidebar.last_metadata_update();

                                let label = if loading {
                                    "Refreshing metadata…".to_string()
                                } else {
                                    match last_update {
                                        Some(at) => {
                                            format!(
                                                "Metadata last update: {}",
                                                data::util::relative_time_label(at)
                                            )
                                        }
                                        None => "Metadata not fetched yet".to_string(),
                                    }
                                };

                                text(label)
                                    .size(crate::style::text_size::SMALL)
                                    .style(style::secondary_text)
                            };

                            column![
                                row![checkbox, refresh_btn]
                                    .align_y(iced::Alignment::Center)
                                    .spacing(8),
                                last_updated,
                            ]
                            .spacing(4)
                        }
                    };

                    let sidebar_pos_picklist = pick_list(
                        [sidebar::Position::Left, sidebar::Position::Right],
                        Some(sidebar_pos),
                        |pos| Message::Sidebar(screen::sidebar::Message::SetSidebarPosition(pos)),
                    );

                    let scale_factor = {
                        let current_value: f32 = self.ui_scale_factor.into();

                        let decrease_btn = if current_value > data::config::MIN_SCALE {
                            button(text("-"))
                                .on_press(Message::ScaleFactorChanged((current_value - 0.1).into()))
                        } else {
                            button(text("-"))
                        };

                        let increase_btn = if current_value < data::config::MAX_SCALE {
                            button(text("+"))
                                .on_press(Message::ScaleFactorChanged((current_value + 0.1).into()))
                        } else {
                            button(text("+"))
                        };

                        container(
                            row![
                                decrease_btn,
                                text(format!("{:.0}%", current_value * 100.0))
                                    .size(crate::style::text_size::SECTION),
                                increase_btn,
                            ]
                            .align_y(Alignment::Center)
                            .spacing(8)
                            .padding(4),
                        )
                        .style(style::modal_container)
                    };

                    let antialiasing_checkbox = {
                        let restart_required = self.antialiasing != self.applied_antialiasing;
                        let checkbox = iced::widget::checkbox(self.antialiasing)
                            .label("Antialiasing")
                            .on_toggle(Message::ApplyAntialiasing);

                        let checkbox = tooltip(
                            checkbox,
                            Some(
                                "Disabling may increase performance, but expect rough edges\n- Restart required to apply.",
                            ),
                            TooltipPosition::Top,
                        );

                        let mut row = row![checkbox]
                            .align_y(iced::Alignment::Center)
                            .spacing(8)
                            .height(24);
                        if restart_required {
                            row = row.push(restart_required_marker());
                        }
                        row
                    };

                    let open_data_folder = {
                        let button =
                            button(text("Open data folder")).on_press(Message::DataFolderRequested);

                        tooltip(
                            button,
                            Some("Open the folder where the data & config is stored"),
                            TooltipPosition::Top,
                        )
                    };

                    let version_info = {
                        let (version_label, commit_label) = version::app_build_version_parts();

                        let github_link_button =
                            button(text(version_label).size(crate::style::text_size::EMPHASIS))
                                .padding(0)
                                .style(style::button::text_link)
                                .on_press(Message::OpenUrlRequested(Cow::Borrowed(
                                    version::GITHUB_REPOSITORY_URL,
                                )));

                        let github_button: Element<'_, Message> = iced::widget::tooltip(
                            github_link_button,
                            container(
                                row![
                                    text("GitHub"),
                                    style::icon_text(style::Icon::ExternalLink, 12),
                                ]
                                .spacing(4)
                                .align_y(Alignment::Center),
                            )
                            .style(style::tooltip)
                            .padding(8),
                            TooltipPosition::Top,
                        )
                        .into();

                        if let (Some(commit_label), Some(commit_url)) =
                            (commit_label, version::build_commit_url())
                        {
                            let commit_button =
                                button(text(commit_label).size(crate::style::text_size::SMALL))
                                    .padding(0)
                                    .style(style::button::text_link_secondary)
                                    .on_press(Message::OpenUrlRequested(Cow::Owned(commit_url)));

                            column![github_button, commit_button]
                                .spacing(2)
                                .align_x(Alignment::End)
                                .into()
                        } else {
                            github_button
                        }
                    };

                    let footer = column![
                        container(version_info)
                            .width(iced::Length::Fill)
                            .align_x(Alignment::End),
                    ]
                    .spacing(8);

                    let column_content = split_column![
                        column![open_data_folder,].spacing(8),
                        column![text("Sidebar position").size(crate::style::text_size::SECTION), sidebar_pos_picklist,].spacing(12),
                        column![text("Time zone").size(crate::style::text_size::SECTION), timezone_picklist,].spacing(12),
                        column![text("Market data").size(crate::style::text_size::SECTION), column![size_in_quote_ccy_checkbox, cache_metadata_checkbox].spacing(4)].spacing(12),
                        column![text("Theme").size(crate::style::text_size::SECTION), theme_picklist,].spacing(12),
                        column![text("Interface scale").size(crate::style::text_size::SECTION), scale_factor,].spacing(12),
                        column![text("Rendering").size(crate::style::text_size::SECTION), antialiasing_checkbox,].spacing(12),
                        column![
                            text("Experimental").size(crate::style::text_size::SECTION),
                            column![toggle_theme_editor, toggle_network_editor].spacing(8),
                        ]
                        .spacing(12),
                        footer,
                        ; spacing = 16, align_x = Alignment::Start
                    ];

                    let content = scrollable::Scrollable::with_direction(
                        column_content,
                        scrollable::Direction::Vertical(
                            scrollable::Scrollbar::new().width(8).scroller_width(6),
                        ),
                    );

                    container(content)
                        .align_x(Alignment::Start)
                        .max_width(240)
                        .padding(24)
                        .style(style::dashboard_modal)
                };

                let (align_x, padding) = match sidebar_pos {
                    sidebar::Position::Left => (Alignment::Start, padding::left(44).bottom(4)),
                    sidebar::Position::Right => (Alignment::End, padding::right(44).bottom(4)),
                };

                dashboard_modal(
                    base,
                    settings_modal,
                    Message::Sidebar(screen::sidebar::Message::ToggleSidebarMenu(None)),
                    padding,
                    Alignment::End,
                    align_x,
                )
            }
            sidebar::Menu::Layout => {
                let main_window = self.main_window.id;

                let manage_pane = if let Some((window_id, pane_id)) = dashboard.focus {
                    let selected_pane_str =
                        if let Some(state) = dashboard.get_pane(main_window, window_id, pane_id) {
                            let link_group_name: String =
                                state.link_group.as_ref().map_or_else(String::new, |g| {
                                    " - Group ".to_string() + &g.to_string()
                                });

                            state.content.to_string() + &link_group_name
                        } else {
                            "".to_string()
                        };

                    let is_main_window = window_id == main_window;

                    let reset_pane_button = {
                        let btn = button(text("Reset").align_x(Alignment::Center))
                            .width(iced::Length::Fill);
                        if is_main_window {
                            let dashboard_msg = Message::Dashboard {
                                layout_id: None,
                                event: dashboard::Message::Pane(
                                    main_window,
                                    dashboard::pane::Message::ReplacePane(pane_id),
                                ),
                            };

                            btn.on_press(dashboard_msg)
                        } else {
                            btn
                        }
                    };
                    let split_pane_button = {
                        let btn = button(text("Split").align_x(Alignment::Center))
                            .width(iced::Length::Fill);
                        if is_main_window {
                            let dashboard_msg = Message::Dashboard {
                                layout_id: None,
                                event: dashboard::Message::Pane(
                                    main_window,
                                    dashboard::pane::Message::SplitPane(
                                        pane_grid::Axis::Horizontal,
                                        pane_id,
                                    ),
                                ),
                            };
                            btn.on_press(dashboard_msg)
                        } else {
                            btn
                        }
                    };

                    column![
                        text(selected_pane_str),
                        row![
                            tooltip(
                                reset_pane_button,
                                if is_main_window {
                                    Some("Reset selected pane")
                                } else {
                                    None
                                },
                                TooltipPosition::Top,
                            ),
                            tooltip(
                                split_pane_button,
                                if is_main_window {
                                    Some("Split selected pane horizontally")
                                } else {
                                    None
                                },
                                TooltipPosition::Top,
                            ),
                        ]
                        .spacing(8)
                    ]
                    .spacing(8)
                } else {
                    let reset_pane_button =
                        button(text("Reset").align_x(Alignment::Center)).width(iced::Length::Fill);
                    let split_pane_button =
                        button(text("Split").align_x(Alignment::Center)).width(iced::Length::Fill);

                    column![
                        text("No pane selected"),
                        row![
                            tooltip(reset_pane_button, None, TooltipPosition::Top),
                            tooltip(split_pane_button, None, TooltipPosition::Top),
                        ]
                        .spacing(8)
                    ]
                    .spacing(8)
                };

                let manage_layout_modal = {
                    let col = column![
                        manage_pane,
                        rule::horizontal(1.0).style(style::split_ruler),
                        self.layout_manager.view().map(Message::Layouts)
                    ]
                    .spacing(20);

                    container(col.align_x(Alignment::Center))
                        .width(260)
                        .padding(24)
                        .style(style::dashboard_modal)
                };

                let (align_x, padding) = match sidebar_pos {
                    sidebar::Position::Left => (Alignment::Start, padding::left(44).top(40)),
                    sidebar::Position::Right => (Alignment::End, padding::right(44).top(40)),
                };

                dashboard_modal(
                    base,
                    manage_layout_modal,
                    Message::Sidebar(screen::sidebar::Message::ToggleSidebarMenu(None)),
                    padding,
                    Alignment::Start,
                    align_x,
                )
            }
            sidebar::Menu::Audio => {
                let (align_x, padding) = match sidebar_pos {
                    sidebar::Position::Left => (Alignment::Start, padding::left(44).top(76)),
                    sidebar::Position::Right => (Alignment::End, padding::right(44).top(76)),
                };

                let trade_streams_list = dashboard.streams.trade_streams(None);

                dashboard_modal(
                    base,
                    self.audio_stream
                        .view(trade_streams_list)
                        .map(Message::AudioStream),
                    Message::Sidebar(screen::sidebar::Message::ToggleSidebarMenu(None)),
                    padding,
                    Alignment::Start,
                    align_x,
                )
            }
            sidebar::Menu::ThemeEditor => {
                let (align_x, padding) = match sidebar_pos {
                    sidebar::Position::Left => (Alignment::Start, padding::left(44).bottom(4)),
                    sidebar::Position::Right => (Alignment::End, padding::right(44).bottom(4)),
                };

                dashboard_modal(
                    base,
                    self.theme_editor
                        .view(self.theme.current())
                        .map(Message::ThemeEditor),
                    Message::Sidebar(screen::sidebar::Message::ToggleSidebarMenu(None)),
                    padding,
                    Alignment::End,
                    align_x,
                )
            }
            sidebar::Menu::Network => {
                let (align_x, padding) = match sidebar_pos {
                    sidebar::Position::Left => (Alignment::Start, padding::left(44).bottom(4)),
                    sidebar::Position::Right => (Alignment::End, padding::right(44).bottom(4)),
                };

                dashboard_modal(
                    base,
                    self.network_editor
                        .view(&self.network_config)
                        .map(Message::NetworkEditor),
                    Message::Sidebar(screen::sidebar::Message::ToggleSidebarMenu(None)),
                    padding,
                    Alignment::End,
                    align_x,
                )
            }
        }
    }

    fn save_state_to_disk(&mut self, windows: &HashMap<window::Id, WindowSpec>) {
        self.active_dashboard_mut()
            .popout
            .iter_mut()
            .for_each(|(id, (_, window_spec))| {
                if let Some(new_window_spec) = windows.get(id) {
                    *window_spec = *new_window_spec;
                }
            });

        self.sidebar.sync_tickers_table_settings();

        let mut ser_layouts = vec![];
        for layout in &self.layout_manager.layouts {
            if let Some(layout) = self.layout_manager.get(layout.id.unique) {
                let serialized_dashboard = data::Dashboard::from(&layout.dashboard);
                ser_layouts.push(data::Layout {
                    name: layout.id.name.clone(),
                    dashboard: serialized_dashboard,
                });
            }
        }

        let layouts = data::Layouts {
            chart_presets: self.layout_manager.presets().entries().to_vec(),
            layouts: ser_layouts,
            active_layout: self
                .layout_manager
                .active_layout_id()
                .map(|layout| layout.name.to_string())
                .clone(),
        };

        let main_window_spec = windows
            .iter()
            .find(|(id, _)| **id == self.main_window.id)
            .map(|(_, spec)| *spec);

        let audio_cfg = data::AudioStream::from(&self.audio_stream);

        let state = data::State::from_parts(
            layouts,
            self.theme.data_theme(),
            self.theme_editor.custom_theme.clone().map(data::Theme),
            main_window_spec,
            self.timezone,
            self.sidebar.state.clone(),
            self.ui_scale_factor,
            audio_cfg,
            self.network_config.for_persistence(),
            self.volume_size_unit,
            self.sidebar.cache_enabled(),
            self.antialiasing,
        );

        match serde_json::to_string(&state) {
            Ok(layout_str) => {
                let file_name = data::SAVED_STATE_PATH;
                if let Err(e) = data::write_json_to_file(&layout_str, file_name) {
                    log::error!("Failed to write layout state to file: {}", e);
                } else {
                    log::info!("Persisted state to {file_name}");
                }
            }
            Err(e) => log::error!("Failed to serialize layout: {}", e),
        }

        self.sidebar.persist_metadata_cache();
    }
}
