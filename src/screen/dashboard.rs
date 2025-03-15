pub mod pane;
pub mod tickers_table;

pub use pane::{PaneContent, PaneSettings, PaneState};

use crate::{
    StreamType,
    aggr::TickMultiplier,
    charts::{
        ChartBasis, Message as ChartMessage, candlestick::CandlestickChart,
        footprint::FootprintChart,
    },
    fetcher::FetchRange,
    layout::get_data_path,
    screen::{InfoType, notification_modal},
    style,
    window::{self, Window},
};

use exchanges::{
    Kline, OpenInterest, Ticker, TickerInfo, Timeframe, Trade,
    adapter::{Event as ExchangeEvent, Exchange, StreamConfig, binance, bybit},
    depth::Depth,
};

use super::{
    DashboardError, Notification, NotificationManager, UserTimezone, modal::dashboard_notification,
};

use iced::{
    Element, Length, Point, Size, Subscription, Task, Vector,
    widget::{
        PaneGrid, center, container,
        pane_grid::{self, Configuration},
    },
};
use iced_futures::futures::TryFutureExt;
use std::{
    collections::{HashMap, HashSet},
    vec,
};

#[derive(Debug, Clone)]
pub enum Message {
    Pane(window::Id, pane::Message),
    SavePopoutSpecs(HashMap<window::Id, (Point, Size)>),
    ErrorOccurred(window::Id, Option<pane_grid::Pane>, DashboardError),
    ClearLastNotification(window::Id, pane_grid::Pane),
    ClearLastGlobalNotification,

    LayoutFetchAll,
    RefreshStreams,

    // Kline fetching
    FetchEvent(
        Option<uuid::Uuid>,
        Result<Vec<Kline>, String>,
        StreamType,
        pane_grid::Pane,
        window::Id,
    ),
    OIFetchEvent(
        Option<uuid::Uuid>,
        Result<Vec<OpenInterest>, String>,
        StreamType,
        pane_grid::Pane,
        window::Id,
    ),
    DistributeFetchedKlines(StreamType, Result<Vec<Kline>, String>),
    ChartMessage(pane_grid::Pane, window::Id, ChartMessage),

    // Batched trade fetching
    FetchTrades(window::Id, pane_grid::Pane, u64, u64, StreamType),
    DistributeFetchedTrades(window::Id, pane_grid::Pane, Vec<Trade>, StreamType, u64),
}

pub struct Dashboard {
    pub panes: pane_grid::State<PaneState>,
    pub focus: Option<(window::Id, pane_grid::Pane)>,
    pub popout: HashMap<window::Id, (pane_grid::State<PaneState>, (Point, Size))>,
    pub pane_streams: HashMap<Exchange, HashMap<Ticker, HashSet<StreamType>>>,
    notification_manager: NotificationManager,
    pub trade_fetch_enabled: bool,
}

impl Default for Dashboard {
    fn default() -> Self {
        Self::empty()
    }
}

impl Dashboard {
    fn empty() -> Self {
        Self {
            panes: pane_grid::State::with_configuration(Self::default_pane_config()),
            focus: None,
            pane_streams: HashMap::new(),
            notification_manager: NotificationManager::new(),
            popout: HashMap::new(),
            trade_fetch_enabled: false,
        }
    }

    fn default_pane_config() -> Configuration<PaneState> {
        Configuration::Split {
            axis: pane_grid::Axis::Vertical,
            ratio: 0.8,
            a: Box::new(Configuration::Split {
                axis: pane_grid::Axis::Horizontal,
                ratio: 0.4,
                a: Box::new(Configuration::Split {
                    axis: pane_grid::Axis::Vertical,
                    ratio: 0.5,
                    a: Box::new(Configuration::Pane(PaneState {
                        modal: pane::PaneModal::None,
                        stream: vec![],
                        content: PaneContent::Starter,
                        settings: PaneSettings::default(),
                    })),
                    b: Box::new(Configuration::Pane(PaneState {
                        modal: pane::PaneModal::None,
                        stream: vec![],
                        content: PaneContent::Starter,
                        settings: PaneSettings::default(),
                    })),
                }),
                b: Box::new(Configuration::Split {
                    axis: pane_grid::Axis::Vertical,
                    ratio: 0.5,
                    a: Box::new(Configuration::Pane(PaneState {
                        modal: pane::PaneModal::None,
                        stream: vec![],
                        content: PaneContent::Starter,
                        settings: PaneSettings::default(),
                    })),
                    b: Box::new(Configuration::Pane(PaneState {
                        modal: pane::PaneModal::None,
                        stream: vec![],
                        content: PaneContent::Starter,
                        settings: PaneSettings::default(),
                    })),
                }),
            }),
            b: Box::new(Configuration::Pane(PaneState {
                modal: pane::PaneModal::None,
                stream: vec![],
                content: PaneContent::Starter,
                settings: PaneSettings::default(),
            })),
        }
    }

    pub fn from_config(
        panes: Configuration<PaneState>,
        popout_windows: Vec<(Configuration<PaneState>, (Point, Size))>,
        trade_fetch_enabled: bool,
    ) -> Self {
        let panes = pane_grid::State::with_configuration(panes);

        let mut popout = HashMap::new();

        for (pane, specs) in popout_windows {
            popout.insert(
                window::Id::unique(),
                (pane_grid::State::with_configuration(pane), specs),
            );
        }

        Self {
            panes,
            focus: None,
            pane_streams: HashMap::new(),
            notification_manager: NotificationManager::new(),
            popout,
            trade_fetch_enabled,
        }
    }

    pub fn load_layout(&mut self) -> Task<Message> {
        let mut open_popouts_tasks: Vec<Task<Message>> = vec![];
        let mut new_popout: Vec<(
            iced::window::Id,
            (pane_grid::State<PaneState>, (Point, Size)),
        )> = Vec::new();
        let mut keys_to_remove: Vec<(iced::window::Id, (Point, Size))> = Vec::new();

        for (old_window_id, (_, specs)) in &self.popout {
            keys_to_remove.push((*old_window_id, *specs));
        }

        // remove keys and open new windows
        for (old_window_id, (pos, size)) in keys_to_remove {
            let (window, task) = window::open(window::Settings {
                position: window::Position::Specific(pos),
                size,
                exit_on_close_request: false,
                ..window::settings()
            });

            open_popouts_tasks.push(task.then(|_| Task::none()));

            if let Some((removed_pane, specs)) = self.popout.remove(&old_window_id) {
                new_popout.push((window, (removed_pane, specs)));
            }
        }

        // assign new windows to old panes
        for (window, (pane, specs)) in new_popout {
            self.popout.insert(window, (pane, specs));
        }

        Task::batch(open_popouts_tasks).chain(Task::batch(vec![
            Task::done(Message::RefreshStreams),
            Task::done(Message::LayoutFetchAll),
        ]))
    }

    pub fn update(&mut self, message: Message, main_window: &Window) -> Task<Message> {
        match message {
            Message::SavePopoutSpecs(specs) => {
                for (window_id, (position, size)) in specs {
                    if let Some((_, specs)) = self.popout.get_mut(&window_id) {
                        *specs = (position, size);
                    }
                }
            }
            Message::ClearLastNotification(window, pane) => {
                self.notification_manager.remove_last(&window, &pane);
            }
            Message::ClearLastGlobalNotification => {
                self.notification_manager.global_notifications.pop();
            }
            Message::ErrorOccurred(window, pane, err) => {
                if let Some(pane) = pane {
                    self.notification_manager.handle_error(window, pane, err);

                    return Task::perform(
                        async { std::thread::sleep(std::time::Duration::from_secs(15)) },
                        move |()| Message::ClearLastNotification(window, pane),
                    );
                }
            }
            Message::Pane(window, message) => {
                match message {
                    pane::Message::PaneClicked(pane) => {
                        self.focus = Some((window, pane));
                    }
                    pane::Message::PaneResized(pane_grid::ResizeEvent { split, ratio }) => {
                        self.panes.resize(split, ratio);
                    }
                    pane::Message::PaneDragged(event) => {
                        if let pane_grid::DragEvent::Dropped { pane, target } = event {
                            self.panes.drop(pane, target);
                            self.focus = None;
                        }
                    }
                    pane::Message::SplitPane(axis, pane) => {
                        let focus_pane = if let Some((new_pane, _)) = self.panes.split(
                            axis,
                            pane,
                            PaneState::new(vec![], PaneSettings::default()),
                        ) {
                            Some(new_pane)
                        } else {
                            None
                        };

                        if Some(focus_pane).is_some() {
                            self.focus = Some((window, focus_pane.unwrap()));
                        }
                    }
                    pane::Message::ClosePane(pane) => {
                        if let Some((_, sibling)) = self.panes.close(pane) {
                            self.focus = Some((window, sibling));
                        }
                    }
                    pane::Message::MaximizePane(pane) => {
                        self.panes.maximize(pane);
                    }
                    pane::Message::Restore => {
                        self.panes.restore();
                    }
                    pane::Message::ReplacePane(pane) => {
                        if let Some(pane) = self.panes.get_mut(pane) {
                            *pane = PaneState::new(vec![], PaneSettings::default());
                        }
                    }
                    pane::Message::ToggleModal(pane, modal_type) => {
                        if let Some(pane) = self.get_mut_pane(main_window.id, window, pane) {
                            if modal_type == pane.modal {
                                pane.modal = pane::PaneModal::None;
                            } else {
                                pane.modal = modal_type;
                            }
                        };
                    }
                    pane::Message::ChartUserUpdate(pane, chart_message) => {
                        return self.update_chart_state(
                            pane,
                            window,
                            &chart_message,
                            main_window.id,
                        );
                    }
                    pane::Message::VisualConfigChanged(pane, cfg) => {
                        if let Some(pane) = pane {
                            if let Some(state) = self.get_mut_pane(main_window.id, window, pane) {
                                state.settings.visual_config = Some(cfg);
                                state.content.change_visual_config(cfg);
                            }
                        } else {
                            self.iter_all_panes_mut(main_window.id)
                                .for_each(|(_, _, state)| {
                                    state.settings.visual_config = Some(cfg);
                                    state.content.change_visual_config(cfg);
                                });
                        }
                    }
                    pane::Message::InitPaneContent(
                        window,
                        content_str,
                        is_pane,
                        pane_stream,
                        ticker_info,
                    ) => {
                        let pane;
                        if let Some(parent_pane) = is_pane {
                            pane = parent_pane;
                        } else {
                            pane = self.panes.iter().next().map(|(pane, _)| *pane).unwrap();
                        }

                        let err_occurred =
                            |err| Task::done(Message::ErrorOccurred(window, Some(pane), err));

                        // set pane's stream and content identifiers
                        if let Some(pane_state) = self.get_mut_pane(main_window.id, window, pane) {
                            if let Err(err) = pane_state.set_content(ticker_info, &content_str) {
                                return err_occurred(err);
                            }
                        } else {
                            return err_occurred(DashboardError::PaneSet(
                                "No pane found".to_string(),
                            ));
                        }

                        // prepare unique streams for websocket
                        for stream in &pane_stream {
                            match stream {
                                StreamType::Kline {
                                    exchange, ticker, ..
                                }
                                | StreamType::DepthAndTrades { exchange, ticker } => {
                                    self.pane_streams
                                        .entry(*exchange)
                                        .or_default()
                                        .entry(*ticker)
                                        .or_default()
                                        .insert(*stream);
                                }
                                _ => {}
                            }
                        }

                        // get fetch tasks for pane's content
                        for stream in &pane_stream {
                            if let StreamType::Kline { .. } = stream {
                                return get_kline_fetch_task(window, pane, *stream, None, None);
                            }
                        }
                    }
                    pane::Message::TicksizeSelected(tick_multiply, pane) => {
                        self.notification_manager.clear(&window, &pane);

                        return self.set_pane_ticksize(main_window.id, window, pane, tick_multiply);
                    }
                    pane::Message::ChartBasisSelected(basis, pane) => {
                        self.notification_manager.clear(&window, &pane);

                        if let Some(state) = self.get_mut_pane(main_window.id, window, pane) {
                            state.settings.selected_basis = Some(basis);

                            if let Some((exchange, ticker)) = state.get_ticker_exchange() {
                                match &state.content {
                                    PaneContent::Candlestick(_, _) => match basis {
                                        ChartBasis::Time(interval) => {
                                            state.stream = vec![StreamType::Kline {
                                                exchange,
                                                ticker,
                                                timeframe: interval.into(),
                                            }];
                                        }
                                        ChartBasis::Tick(_) => {
                                            state.stream = vec![StreamType::DepthAndTrades {
                                                exchange,
                                                ticker,
                                            }];
                                        }
                                    },
                                    PaneContent::Footprint(_, _) => match basis {
                                        ChartBasis::Time(interval) => {
                                            state.stream = vec![
                                                StreamType::Kline {
                                                    exchange,
                                                    ticker,
                                                    timeframe: interval.into(),
                                                },
                                                StreamType::DepthAndTrades { exchange, ticker },
                                            ];
                                        }
                                        ChartBasis::Tick(_) => {
                                            state.stream = vec![StreamType::DepthAndTrades {
                                                exchange,
                                                ticker,
                                            }];
                                        }
                                    },
                                    _ => {}
                                }
                            }
                        }

                        match basis {
                            ChartBasis::Time(timeframe) => {
                                match self.set_pane_timeframe(
                                    main_window.id,
                                    window,
                                    pane,
                                    timeframe.into(),
                                ) {
                                    Ok(stream_type) => {
                                        if let StreamType::Kline { .. } = stream_type {
                                            let task = get_kline_fetch_task(
                                                window,
                                                pane,
                                                *stream_type,
                                                None,
                                                None,
                                            );

                                            self.notification_manager.push(
                                                window,
                                                pane,
                                                Notification::Info(InfoType::FetchingKlines),
                                            );

                                            return Task::done(Message::RefreshStreams).chain(task);
                                        }
                                    }
                                    Err(err) => {
                                        return Task::done(Message::ErrorOccurred(
                                            window,
                                            Some(pane),
                                            err,
                                        ));
                                    }
                                }
                            }
                            ChartBasis::Tick(size) => {
                                if let Some(pane_state) =
                                    self.get_mut_pane(main_window.id, window, pane)
                                {
                                    match &mut pane_state.content {
                                        PaneContent::Footprint(chart, _) => {
                                            chart.set_tick_basis(size);
                                        }
                                        PaneContent::Candlestick(chart, _) => {
                                            chart.set_tick_basis(size);
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }

                        return Task::done(Message::RefreshStreams);
                    }
                    pane::Message::Popout => return self.popout_pane(main_window),
                    pane::Message::Merge => return self.merge_pane(main_window),
                    pane::Message::ToggleIndicator(pane, indicator_str) => {
                        if let Some(pane_state) = self.get_mut_pane(main_window.id, window, pane) {
                            pane_state.content.toggle_indicator(indicator_str);
                        }
                    }
                    pane::Message::HideNotification(pane, notification) => {
                        self.notification_manager
                            .find_and_remove(window, pane, notification);
                    }
                }
            }
            Message::FetchEvent(req_id, klines, pane_stream, pane_id, window) => {
                self.notification_manager.remove_info_type(
                    window,
                    &pane_id,
                    &InfoType::FetchingKlines,
                );

                match klines {
                    Ok(klines) => {
                        if let StreamType::Kline { timeframe, .. } = pane_stream {
                            if let Some(pane_state) =
                                self.get_mut_pane(main_window.id, window, pane_id)
                            {
                                pane_state.insert_klines_vec(req_id, timeframe, &klines);
                            }
                        }
                    }
                    Err(err) => {
                        return Task::done(Message::ErrorOccurred(
                            window,
                            Some(pane_id),
                            DashboardError::Fetch(err),
                        ));
                    }
                }
            }
            Message::OIFetchEvent(req_id, oi, pane_stream, pane_id, window) => {
                self.notification_manager
                    .remove_info_type(window, &pane_id, &InfoType::FetchingOI);

                if let Some(pane_state) = self.get_mut_pane(main_window.id, window, pane_id) {
                    match oi {
                        Ok(oi) => {
                            if let StreamType::Kline { .. } = pane_stream {
                                pane_state.insert_oi_vec(req_id, oi);
                            }
                        }
                        Err(err) => {
                            return Task::done(Message::ErrorOccurred(
                                window,
                                Some(pane_id),
                                DashboardError::Fetch(err),
                            ));
                        }
                    }
                }
            }
            Message::LayoutFetchAll => {
                let mut fetched_panes = vec![];

                self.iter_all_panes(main_window.id)
                    .for_each(|(window, pane, pane_state)| match pane_state.content {
                        PaneContent::Candlestick(_, _) | PaneContent::Footprint(_, _) => {
                            if pane_state
                                .settings
                                .selected_basis
                                .is_some_and(|basis| basis.is_time())
                            {
                                fetched_panes.push((window, pane));
                            }
                        }
                        _ => {}
                    });

                for (window, pane) in fetched_panes {
                    self.notification_manager.push(
                        window,
                        pane,
                        Notification::Info(InfoType::FetchingKlines),
                    );
                }

                return Task::batch(klines_fetch_all_task(&self.pane_streams));
            }
            Message::DistributeFetchedKlines(stream_type, klines) => match klines {
                Ok(klines) => {
                    let mut inserted_panes = vec![];

                    self.iter_all_panes_mut(main_window.id)
                        .for_each(|(window, pane, state)| {
                            if state.matches_stream(&stream_type) {
                                if let StreamType::Kline { timeframe, .. } = stream_type {
                                    match &mut state.content {
                                        PaneContent::Candlestick(chart, indicators) => {
                                            let (raw_trades, tick_size) =
                                                (chart.get_raw_trades(), chart.get_tick_size());

                                            *chart = CandlestickChart::new(
                                                chart.get_chart_layout(),
                                                state.settings.selected_basis.unwrap_or(
                                                    ChartBasis::Time(timeframe.to_milliseconds()),
                                                ),
                                                klines.clone(),
                                                raw_trades,
                                                tick_size,
                                                indicators,
                                                state.settings.ticker_info,
                                            );
                                        }
                                        PaneContent::Footprint(chart, indicators) => {
                                            let (raw_trades, tick_size) =
                                                (chart.get_raw_trades(), chart.get_tick_size());

                                            *chart = FootprintChart::new(
                                                chart.get_chart_layout(),
                                                state.settings.selected_basis.unwrap_or(
                                                    ChartBasis::Time(timeframe.to_milliseconds()),
                                                ),
                                                tick_size,
                                                klines.clone(),
                                                raw_trades,
                                                indicators,
                                                state.settings.ticker_info,
                                            );
                                        }
                                        _ => {}
                                    }

                                    inserted_panes.push((window, pane));
                                }
                            }
                        });

                    for (window, pane) in inserted_panes {
                        self.notification_manager.remove_info_type(
                            window,
                            &pane,
                            &InfoType::FetchingKlines,
                        );
                    }
                }
                Err(err) => {
                    log::error!("{err}");
                }
            },
            Message::FetchTrades(window_id, pane, from_time, to_time, stream_type) => {
                if let StreamType::DepthAndTrades { exchange, ticker } = stream_type {
                    if exchange == Exchange::BinanceFutures || exchange == Exchange::BinanceSpot {
                        let data_path = get_data_path(&format!("market_data/binance/",));

                        return Task::perform(
                            binance::fetch_trades(ticker, from_time, data_path),
                            move |result| match result {
                                Ok(trades) => Message::DistributeFetchedTrades(
                                    window_id,
                                    pane,
                                    trades,
                                    stream_type,
                                    to_time,
                                ),
                                Err(err) => Message::ErrorOccurred(
                                    window_id,
                                    Some(pane),
                                    DashboardError::Fetch(err.to_string()),
                                ),
                            },
                        );
                    } else {
                        self.notification_manager.remove_info_type(
                            window_id,
                            &pane,
                            &InfoType::FetchingTrades(0),
                        );

                        return Task::done(Message::ErrorOccurred(
                            window_id,
                            Some(pane),
                            DashboardError::Fetch(format!(
                                "No trade fetch support for {exchange:?}"
                            )),
                        ));
                    }
                }
            }
            Message::DistributeFetchedTrades(window_id, pane, trades, stream_type, to_time) => {
                let last_trade_time = trades.last().map_or(0, |trade| trade.time);

                self.notification_manager
                    .increment_fetching_trades(window_id, &pane, trades.len());

                if last_trade_time < to_time {
                    match self.insert_fetched_trades(
                        main_window.id,
                        window_id,
                        pane,
                        &trades,
                        false,
                    ) {
                        Ok(()) => {
                            return Task::done(Message::FetchTrades(
                                window_id,
                                pane,
                                last_trade_time,
                                to_time,
                                stream_type,
                            ));
                        }
                        Err(err) => {
                            self.notification_manager.remove_info_type(
                                window_id,
                                &pane,
                                &InfoType::FetchingTrades(0),
                            );

                            return Task::done(Message::ErrorOccurred(window_id, Some(pane), err));
                        }
                    }
                } else {
                    self.notification_manager.remove_info_type(
                        window_id,
                        &pane,
                        &InfoType::FetchingTrades(0),
                    );

                    match self.insert_fetched_trades(main_window.id, window_id, pane, &trades, true)
                    {
                        Ok(()) => {}
                        Err(err) => {
                            return Task::done(Message::ErrorOccurred(window_id, Some(pane), err));
                        }
                    }
                }
            }
            Message::RefreshStreams => {
                self.pane_streams = self.get_all_diff_streams(main_window.id);
            }
            Message::ChartMessage(pane, window, message) => {
                if let ChartMessage::NewDataRange(req_id, fetch) = message {
                    match fetch {
                        FetchRange::Kline(from, to) => {
                            let kline_stream = self
                                .get_pane(main_window.id, window, pane)
                                .and_then(|pane| {
                                    pane.stream
                                        .iter()
                                        .find(|stream| matches!(stream, StreamType::Kline { .. }))
                                });

                            if let Some(stream) = kline_stream {
                                let stream = *stream;

                                self.notification_manager.push(
                                    window,
                                    pane,
                                    Notification::Info(InfoType::FetchingKlines),
                                );

                                return get_kline_fetch_task(
                                    window,
                                    pane,
                                    stream,
                                    Some(req_id),
                                    Some((from, to)),
                                );
                            }
                        }
                        FetchRange::OpenInterest(from, to) => {
                            let kline_stream = self
                                .get_pane(main_window.id, window, pane)
                                .and_then(|pane| {
                                    pane.stream
                                        .iter()
                                        .find(|stream| matches!(stream, StreamType::Kline { .. }))
                                });

                            if let Some(stream) = kline_stream {
                                let stream = *stream;

                                self.notification_manager.push(
                                    window,
                                    pane,
                                    Notification::Info(InfoType::FetchingOI),
                                );

                                return get_oi_fetch_task(
                                    window,
                                    pane,
                                    stream,
                                    Some(req_id),
                                    Some((from, to)),
                                );
                            }
                        }
                        FetchRange::Trades(from, to) => {
                            if !self.trade_fetch_enabled {
                                return Task::none();
                            }

                            let trade_stream = self
                                .get_pane(main_window.id, window, pane)
                                .and_then(|pane| {
                                    pane.stream.iter().find(|stream| {
                                        matches!(stream, StreamType::DepthAndTrades { .. })
                                    })
                                });

                            if let Some(stream) = trade_stream {
                                let stream = *stream;

                                self.notification_manager.push(
                                    window,
                                    pane,
                                    Notification::Info(InfoType::FetchingTrades(0)),
                                );

                                return Task::done(Message::FetchTrades(
                                    window, pane, from, to, stream,
                                ));
                            }
                        }
                    }
                }
            }
        }

        Task::none()
    }

    fn new_pane(
        &mut self,
        axis: pane_grid::Axis,
        main_window: &Window,
        pane_state: Option<PaneState>,
    ) -> Task<Message> {
        if self
            .focus
            .filter(|(window, _)| *window == main_window.id)
            .is_some()
        {
            // If there is any focused pane on main window, split it
            return self.split_pane(axis, main_window);
        } else {
            // If there is no focused pane, split the last pane or create a new empty grid
            let pane = self.panes.iter().last().map(|(pane, _)| pane).copied();

            if let Some(pane) = pane {
                let result = self.panes.split(
                    axis,
                    pane,
                    pane_state.unwrap_or(PaneState::new(vec![], PaneSettings::default())),
                );

                if let Some((pane, _)) = result {
                    return self.focus_pane(main_window.id, pane);
                }
            } else {
                let (state, pane) = pane_grid::State::new(
                    pane_state.unwrap_or(PaneState::new(vec![], PaneSettings::default())),
                );
                self.panes = state;

                return self.focus_pane(main_window.id, pane);
            }
        }

        Task::none()
    }

    fn focus_pane(&mut self, window: window::Id, pane: pane_grid::Pane) -> Task<Message> {
        if self.focus != Some((window, pane)) {
            self.focus = Some((window, pane));
        }

        Task::none()
    }

    fn split_pane(&mut self, axis: pane_grid::Axis, main_window: &Window) -> Task<Message> {
        if let Some((window, pane)) = self.focus {
            if window == main_window.id {
                let result =
                    self.panes
                        .split(axis, pane, PaneState::new(vec![], PaneSettings::default()));

                if let Some((pane, _)) = result {
                    return self.focus_pane(main_window.id, pane);
                }
            }
        }

        Task::none()
    }

    fn popout_pane(&mut self, main_window: &Window) -> Task<Message> {
        if let Some((_, id)) = self.focus.take() {
            if let Some((pane, _)) = self.panes.close(id) {
                let (window, task) = window::open(window::Settings {
                    position: main_window
                        .position
                        .map(|point| window::Position::Specific(point + Vector::new(20.0, 20.0)))
                        .unwrap_or_default(),
                    exit_on_close_request: false,
                    ..window::settings()
                });

                let (state, id) = pane_grid::State::new(pane);
                self.popout.insert(
                    window,
                    (state, (Point::new(0.0, 0.0), Size::new(1024.0, 768.0))),
                );

                return task.then(move |window| {
                    Task::done(Message::Pane(window, pane::Message::PaneClicked(id)))
                });
            }
        }

        Task::none()
    }

    fn merge_pane(&mut self, main_window: &Window) -> Task<Message> {
        if let Some((window, pane)) = self.focus.take() {
            if let Some(pane_state) = self
                .popout
                .remove(&window)
                .and_then(|(mut panes, _)| panes.panes.remove(&pane))
            {
                let task =
                    self.new_pane(pane_grid::Axis::Horizontal, main_window, Some(pane_state));

                return Task::batch(vec![window::close(window), task]);
            }
        }

        Task::none()
    }

    fn get_pane(
        &self,
        main_window: window::Id,
        window: window::Id,
        pane: pane_grid::Pane,
    ) -> Option<&PaneState> {
        if main_window == window {
            self.panes.get(pane)
        } else {
            self.popout
                .get(&window)
                .and_then(|(panes, _)| panes.get(pane))
        }
    }

    fn get_mut_pane(
        &mut self,
        main_window: window::Id,
        window: window::Id,
        pane: pane_grid::Pane,
    ) -> Option<&mut PaneState> {
        if main_window == window {
            self.panes.get_mut(pane)
        } else {
            self.popout
                .get_mut(&window)
                .and_then(|(panes, _)| panes.get_mut(pane))
        }
    }

    fn iter_all_panes(
        &self,
        main_window: window::Id,
    ) -> impl Iterator<Item = (window::Id, pane_grid::Pane, &PaneState)> {
        self.panes
            .iter()
            .map(move |(pane, state)| (main_window, *pane, state))
            .chain(self.popout.iter().flat_map(|(window_id, (panes, _))| {
                panes.iter().map(|(pane, state)| (*window_id, *pane, state))
            }))
    }

    fn iter_all_panes_mut(
        &mut self,
        main_window: window::Id,
    ) -> impl Iterator<Item = (window::Id, pane_grid::Pane, &mut PaneState)> {
        self.panes
            .iter_mut()
            .map(move |(pane, state)| (main_window, *pane, state))
            .chain(self.popout.iter_mut().flat_map(|(window_id, (panes, _))| {
                panes
                    .iter_mut()
                    .map(|(pane, state)| (*window_id, *pane, state))
            }))
    }

    pub fn view<'a>(
        &'a self,
        main_window: &'a Window,
        layout_locked: bool,
        timezone: &'a UserTimezone,
    ) -> Element<'a, Message> {
        let focus = self.focus;

        let mut pane_grid = PaneGrid::new(&self.panes, |id, pane, maximized| {
            let is_focused = !layout_locked && focus == Some((main_window.id, id));
            pane.view(
                id,
                self.panes.len(),
                is_focused,
                maximized,
                main_window.id,
                main_window,
                timezone,
                self.notification_manager.get(&main_window.id, &id),
            )
        })
        .spacing(6)
        .style(style::pane_grid);

        if !layout_locked {
            pane_grid = pane_grid
                .on_click(pane::Message::PaneClicked)
                .on_resize(8, pane::Message::PaneResized)
                .on_drag(pane::Message::PaneDragged);
        }

        let pane_grid: Element<_> = pane_grid.into();
        let base = container(pane_grid.map(move |message| Message::Pane(main_window.id, message)));

        if self.notification_manager.global_notifications.is_empty() {
            base.into()
        } else {
            dashboard_notification(
                base,
                notification_modal(&self.notification_manager.global_notifications, move |_| {
                    Message::ClearLastGlobalNotification
                }),
            )
        }
    }

    pub fn view_window<'a>(
        &'a self,
        window: window::Id,
        main_window: &'a Window,
        layout_locked: bool,
        timezone: &'a UserTimezone,
    ) -> Element<'a, Message> {
        if let Some((state, _)) = self.popout.get(&window) {
            let content = container({
                let mut pane_grid = PaneGrid::new(state, |id, pane, _maximized| {
                    let is_focused = self.focus == Some((window, id));
                    pane.view(
                        id,
                        state.len(),
                        is_focused,
                        false,
                        window,
                        main_window,
                        timezone,
                        self.notification_manager.get(&window, &id),
                    )
                });

                if !layout_locked {
                    pane_grid = pane_grid.on_click(pane::Message::PaneClicked);
                }
                pane_grid
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(8);

            Element::new(content).map(move |message| Message::Pane(window, message))
        } else {
            Element::new(center("No pane found for window"))
                .map(move |message| Message::Pane(window, message))
        }
    }

    fn set_pane_ticksize(
        &mut self,
        main_window: window::Id,
        window: window::Id,
        pane: pane_grid::Pane,
        new_tick_multiply: TickMultiplier,
    ) -> Task<Message> {
        if let Some(pane_state) = self.get_mut_pane(main_window, window, pane) {
            pane_state.settings.tick_multiply = Some(new_tick_multiply);

            if let Some(ticker_info) = pane_state.settings.ticker_info {
                match pane_state.content {
                    PaneContent::Footprint(ref mut chart, _) => {
                        chart.change_tick_size(
                            new_tick_multiply.multiply_with_min_tick_size(ticker_info),
                        );

                        chart.reset_request_handler();
                    }
                    PaneContent::Heatmap(ref mut chart, _) => {
                        chart.change_tick_size(
                            new_tick_multiply.multiply_with_min_tick_size(ticker_info),
                        );
                    }
                    _ => {
                        return Task::done(Message::ErrorOccurred(
                            window,
                            Some(pane),
                            DashboardError::PaneSet(
                                "No chart found to change ticksize".to_string(),
                            ),
                        ));
                    }
                }
            } else {
                return Task::done(Message::ErrorOccurred(
                    window,
                    Some(pane),
                    DashboardError::PaneSet("No min ticksize found".to_string()),
                ));
            }
        } else {
            return Task::done(Message::ErrorOccurred(
                window,
                Some(pane),
                DashboardError::PaneSet("No pane found to change ticksize".to_string()),
            ));
        }

        Task::none()
    }

    fn set_pane_timeframe(
        &mut self,
        main_window: window::Id,
        window: window::Id,
        pane: pane_grid::Pane,
        new_timeframe: Timeframe,
    ) -> Result<&StreamType, DashboardError> {
        if let Some(pane_state) = self.get_mut_pane(main_window, window, pane) {
            pane_state.settings.selected_basis =
                Some(ChartBasis::Time(new_timeframe.to_milliseconds()));

            if let Some(stream_type) = pane_state
                .stream
                .iter_mut()
                .find(|stream_type| matches!(stream_type, StreamType::Kline { .. }))
            {
                if let StreamType::Kline { timeframe, .. } = stream_type {
                    *timeframe = new_timeframe;
                }

                match &pane_state.content {
                    PaneContent::Candlestick(_, _) => {
                        return Ok(stream_type);
                    }
                    PaneContent::Footprint(_, _) => {
                        return Ok(stream_type);
                    }
                    _ => {}
                }
            }
        }
        Err(DashboardError::Unknown(
            "Couldn't get the pane to change its timeframe".to_string(),
        ))
    }

    pub fn init_pane_task(
        &mut self,
        main_window: window::Id,
        ticker: (Ticker, TickerInfo),
        exchange: Exchange,
        content: &str,
    ) -> Task<Message> {
        if let Some((window, selected_pane)) = self.focus {
            if let Some(pane_state) = self.get_mut_pane(main_window, window, selected_pane) {
                return pane_state
                    .init_content_task(content, exchange, ticker, selected_pane, window)
                    .map(move |message| Message::Pane(window, message));
            }
        } else {
            self.notification_manager
                .global_notifications
                .push(Notification::Warn("Select a pane first".to_string()));

            return Task::perform(
                async { std::thread::sleep(std::time::Duration::from_secs(8)) },
                move |()| Message::ClearLastGlobalNotification,
            );
        }

        Task::none()
    }

    pub fn toggle_trade_fetch(&mut self, is_enabled: bool, main_window: &Window) {
        self.trade_fetch_enabled = is_enabled;

        self.iter_all_panes_mut(main_window.id)
            .for_each(|(_, _, pane_state)| {
                if let PaneContent::Footprint(chart, _) = &mut pane_state.content {
                    chart.reset_request_handler();
                }
            });
    }

    fn insert_fetched_trades(
        &mut self,
        main_window: window::Id,
        window: window::Id,
        pane: pane_grid::Pane,
        trades: &[Trade],
        is_batches_done: bool,
    ) -> Result<(), DashboardError> {
        self.get_mut_pane(main_window, window, pane).map_or_else(
            || {
                Err(DashboardError::Unknown(
                    "Couldnt get the pane for fetched trades".to_string(),
                ))
            },
            |pane_state| match &mut pane_state.content {
                PaneContent::Footprint(chart, _) => {
                    chart.insert_raw_trades(trades.to_owned(), is_batches_done);
                    Ok(())
                }
                _ => Err(DashboardError::Unknown(
                    "No matching chart found for fetched trades".to_string(),
                )),
            },
        )
    }

    pub fn update_latest_klines(
        &mut self,
        stream: &StreamType,
        kline: &Kline,
        main_window: window::Id,
    ) -> Task<Message> {
        let mut tasks = vec![];

        let mut found_match = false;

        self.iter_all_panes_mut(main_window)
            .for_each(|(window, pane, pane_state)| {
                if pane_state.matches_stream(stream) {
                    match &mut pane_state.content {
                        PaneContent::Candlestick(chart, _) => tasks.push(
                            chart
                                .update_latest_kline(kline)
                                .map(move |message| Message::ChartMessage(pane, window, message)),
                        ),
                        PaneContent::Footprint(chart, _) => tasks.push(
                            chart
                                .update_latest_kline(kline)
                                .map(move |message| Message::ChartMessage(pane, window, message)),
                        ),
                        _ => {}
                    }
                    found_match = true;
                }
            });

        if !found_match {
            log::error!("No matching pane found for the stream: {stream:?}");
            tasks.push(Task::done(Message::RefreshStreams));
        }

        Task::batch(tasks)
    }

    pub fn update_depth_and_trades(
        &mut self,
        stream: &StreamType,
        depth_update_t: u64,
        depth: Depth,
        trades_buffer: Box<[Trade]>,
        main_window: window::Id,
    ) -> Task<Message> {
        let mut found_match = false;

        self.iter_all_panes_mut(main_window)
            .for_each(|(_, _, pane_state)| {
                if pane_state.matches_stream(stream) {
                    match &mut pane_state.content {
                        PaneContent::Heatmap(chart, _) => {
                            chart.insert_datapoint(&trades_buffer, depth_update_t, &depth);
                        }
                        PaneContent::Footprint(chart, _) => {
                            chart.insert_trades_buffer(&trades_buffer, depth_update_t);
                        }
                        PaneContent::TimeAndSales(chart) => {
                            chart.update(&trades_buffer);
                        }
                        PaneContent::Candlestick(chart, _) => {
                            chart.insert_trades_buffer(&trades_buffer);
                        }
                        _ => {
                            log::error!("No chart found for the stream: {stream:?}");
                        }
                    }
                    found_match = true;
                }
            });

        if found_match {
            Task::none()
        } else {
            log::error!("No matching pane found for the stream: {stream:?}");
            Task::done(Message::RefreshStreams)
        }
    }

    fn update_chart_state(
        &mut self,
        pane: pane_grid::Pane,
        window: window::Id,
        chart_message: &ChartMessage,
        main_window: window::Id,
    ) -> Task<Message> {
        if let Some(pane_state) = self.get_mut_pane(main_window, window, pane) {
            match pane_state.content {
                PaneContent::Heatmap(ref mut chart, _) => chart
                    .update(chart_message)
                    .map(move |message| Message::ChartMessage(pane, window, message)),
                PaneContent::Footprint(ref mut chart, _) => chart
                    .update(chart_message)
                    .map(move |message| Message::ChartMessage(pane, window, message)),
                PaneContent::Candlestick(ref mut chart, _) => chart
                    .update(chart_message)
                    .map(move |message| Message::ChartMessage(pane, window, message)),
                _ => Task::done(Message::ErrorOccurred(
                    window,
                    Some(pane),
                    DashboardError::Unknown("No chart found".to_string()),
                )),
            }
        } else {
            Task::done(Message::ErrorOccurred(
                window,
                Some(pane),
                DashboardError::Unknown("No pane found to update its state".to_string()),
            ))
        }
    }

    pub fn get_market_subscriptions<M>(
        &self,
        market_msg: impl Fn(ExchangeEvent) -> M + Clone + Send + 'static,
    ) -> Subscription<M>
    where
        M: 'static,
    {
        let mut market_subscriptions = Vec::with_capacity(
            self.pane_streams.len() * 2, // worst case: both kline and depth per exchange
        );

        self.pane_streams.iter().for_each(|(exchange, stream)| {
            let (depth_count, kline_count) = stream
                .values()
                .flat_map(|stream_types| stream_types.iter())
                .fold((0, 0), |(depths, klines), stream_type| match stream_type {
                    StreamType::DepthAndTrades { .. } => (depths + 1, klines),
                    StreamType::Kline { .. } => (depths, klines + 1),
                    StreamType::None => (depths, klines),
                });

            if depth_count > 0 {
                let mut depth_streams = Vec::with_capacity(depth_count);

                stream
                    .values()
                    .flat_map(|stream_types| stream_types.iter())
                    .filter_map(|stream_type| match stream_type {
                        StreamType::DepthAndTrades { ticker, .. } => {
                            let config = StreamConfig::new(*ticker, *exchange);
                            Some(match exchange {
                                Exchange::BinanceSpot | Exchange::BinanceFutures => {
                                    Subscription::run_with(config, move |cfg| {
                                        binance::connect_market_stream(cfg.id)
                                    })
                                    .map(market_msg.clone())
                                }
                                Exchange::BybitSpot | Exchange::BybitLinear => {
                                    Subscription::run_with(config, move |cfg| {
                                        bybit::connect_market_stream(cfg.id)
                                    })
                                    .map(market_msg.clone())
                                }
                            })
                        }
                        _ => None,
                    })
                    .for_each(|stream| depth_streams.push(stream));

                market_subscriptions.push(Subscription::batch(depth_streams));
            }

            if kline_count > 0 {
                let kline_streams: Vec<_> = stream
                    .values()
                    .flat_map(|stream_types| stream_types.iter())
                    .filter_map(|stream_type| match stream_type {
                        StreamType::Kline {
                            ticker, timeframe, ..
                        } => Some((*ticker, *timeframe)),
                        _ => None,
                    })
                    .collect();

                let config = StreamConfig::new(kline_streams, *exchange);

                market_subscriptions.push(match exchange {
                    Exchange::BinanceSpot | Exchange::BinanceFutures => {
                        Subscription::run_with(config, move |cfg| {
                            binance::connect_kline_stream(cfg.id.clone(), cfg.market_type)
                        })
                        .map(market_msg.clone())
                    }
                    Exchange::BybitSpot | Exchange::BybitLinear => {
                        Subscription::run_with(config, move |cfg| {
                            bybit::connect_kline_stream(cfg.id.clone(), cfg.market_type)
                        })
                        .map(market_msg.clone())
                    }
                });
            }
        });

        Subscription::batch(market_subscriptions)
    }

    fn get_all_diff_streams(
        &mut self,
        main_window: window::Id,
    ) -> HashMap<Exchange, HashMap<Ticker, HashSet<StreamType>>> {
        let mut pane_streams = HashMap::new();

        self.iter_all_panes_mut(main_window)
            .for_each(|(_, _, pane_state)| {
                for stream_type in &pane_state.stream {
                    match stream_type {
                        StreamType::Kline {
                            exchange,
                            ticker,
                            timeframe,
                        } => {
                            let exchange = *exchange;
                            let ticker = *ticker;
                            let timeframe = *timeframe;

                            let exchange_map =
                                pane_streams.entry(exchange).or_insert(HashMap::new());
                            let ticker_map = exchange_map.entry(ticker).or_insert(HashSet::new());
                            ticker_map.insert(StreamType::Kline {
                                exchange,
                                ticker,
                                timeframe,
                            });
                        }
                        StreamType::DepthAndTrades { exchange, ticker } => {
                            let exchange = *exchange;
                            let ticker = *ticker;

                            let exchange_map =
                                pane_streams.entry(exchange).or_insert(HashMap::new());
                            let ticker_map = exchange_map.entry(ticker).or_insert(HashSet::new());
                            ticker_map.insert(StreamType::DepthAndTrades { exchange, ticker });
                        }
                        _ => {}
                    }
                }
            });

        self.pane_streams.clone_from(&pane_streams);

        pane_streams
    }
}

fn get_oi_fetch_task(
    window_id: window::Id,
    pane: pane_grid::Pane,
    stream: StreamType,
    req_id: Option<uuid::Uuid>,
    from_to_time: Option<(u64, u64)>,
) -> Task<Message> {
    match stream {
        StreamType::Kline {
            exchange,
            ticker,
            timeframe,
        } => match exchange {
            Exchange::BinanceFutures => Task::perform(
                binance::fetch_historical_oi(ticker, from_to_time, timeframe)
                    .map_err(|err| format!("{err}")),
                move |oi| Message::OIFetchEvent(req_id, oi, stream, pane, window_id),
            ),
            Exchange::BybitLinear => Task::perform(
                bybit::fetch_historical_oi(ticker, from_to_time, timeframe)
                    .map_err(|err| format!("{err}")),
                move |oi| Message::OIFetchEvent(req_id, oi, stream, pane, window_id),
            ),
            _ => {
                log::error!("No OI fetch support for {exchange:?}");
                Task::none()
            }
        },
        _ => Task::none(),
    }
}

fn get_kline_fetch_task(
    window_id: window::Id,
    pane: pane_grid::Pane,
    stream: StreamType,
    req_id: Option<uuid::Uuid>,
    range: Option<(u64, u64)>,
) -> Task<Message> {
    match stream {
        StreamType::Kline {
            exchange,
            ticker,
            timeframe,
        } => match exchange {
            Exchange::BinanceFutures | Exchange::BinanceSpot => Task::perform(
                binance::fetch_klines(ticker, timeframe, range).map_err(|err| format!("{err}")),
                move |klines| Message::FetchEvent(req_id, klines, stream, pane, window_id),
            ),
            Exchange::BybitLinear | Exchange::BybitSpot => Task::perform(
                bybit::fetch_klines(ticker, timeframe, range).map_err(|err| format!("{err}")),
                move |klines| Message::FetchEvent(req_id, klines, stream, pane, window_id),
            ),
        },
        _ => Task::none(),
    }
}

fn klines_fetch_all_task(
    streams: &HashMap<Exchange, HashMap<Ticker, HashSet<StreamType>>>,
) -> Vec<Task<Message>> {
    let mut tasks: Vec<Task<Message>> = vec![];

    for (exchange, stream) in streams {
        let mut kline_fetches = Vec::new();

        for stream_types in stream.values() {
            for stream_type in stream_types {
                if let StreamType::Kline {
                    ticker, timeframe, ..
                } = stream_type
                {
                    kline_fetches.push((*ticker, *timeframe));
                }
            }
        }

        for (ticker, timeframe) in kline_fetches {
            let (ticker, timeframe) = (ticker, timeframe);
            let exchange = *exchange;

            match exchange {
                Exchange::BinanceFutures | Exchange::BinanceSpot => {
                    let fetch_klines = Task::perform(
                        binance::fetch_klines(ticker, timeframe, None)
                            .map_err(|err| format!("{err}")),
                        move |klines| {
                            Message::DistributeFetchedKlines(
                                StreamType::Kline {
                                    exchange,
                                    ticker,
                                    timeframe,
                                },
                                klines,
                            )
                        },
                    );
                    tasks.push(fetch_klines);
                }
                Exchange::BybitLinear | Exchange::BybitSpot => {
                    let fetch_klines = Task::perform(
                        bybit::fetch_klines(ticker, timeframe, None)
                            .map_err(|err| format!("{err}")),
                        move |klines| {
                            Message::DistributeFetchedKlines(
                                StreamType::Kline {
                                    exchange,
                                    ticker,
                                    timeframe,
                                },
                                klines,
                            )
                        },
                    );
                    tasks.push(fetch_klines);
                }
            }
        }
    }

    tasks
}
