pub mod pane;
pub mod panel;
pub mod theme_editor;
pub mod tickers_table;

use crate::{
    StreamKind, chart, style,
    widget::toast::Toast,
    window::{self, Window},
};
use data::{UserTimezone, chart::Basis, layout::WindowSpec};

use exchange::{
    Kline, TickMultiplier, Ticker, TickerInfo, Timeframe, Trade,
    adapter::{self, Exchange, StreamConfig, StreamError, UniqueStreams, binance, bybit},
    depth::Depth,
    fetcher::{FetchRange, FetchedData},
};

use super::DashboardError;

use iced::{
    Element, Length, Subscription, Task, Vector,
    task::{Straw, sipper},
    widget::{
        PaneGrid, center, container,
        pane_grid::{self, Configuration},
    },
};
use iced_futures::futures::TryFutureExt;
use std::{collections::HashMap, path::PathBuf, time::Instant, vec};

#[derive(Debug, Clone)]
pub enum Message {
    Pane(window::Id, pane::Message),
    SavePopoutSpecs(HashMap<window::Id, WindowSpec>),
    ErrorOccurred(Option<uuid::Uuid>, DashboardError),
    Notification(Toast),

    LayoutFetchAll,
    RefreshStreams,

    ChartRequestedFetch(pane_grid::Pane, window::Id, uuid::Uuid, FetchRange),
    DistributeFetchedData(uuid::Uuid, uuid::Uuid, FetchedData, StreamKind),
    ChangePaneStatus(uuid::Uuid, pane::Status),
}

pub struct Dashboard {
    pub panes: pane_grid::State<pane::State>,
    pub focus: Option<(window::Id, pane_grid::Pane)>,
    pub popout: HashMap<window::Id, (pane_grid::State<pane::State>, WindowSpec)>,
    pub streams: UniqueStreams,
    pub trade_fetch_enabled: bool,
}

impl Default for Dashboard {
    fn default() -> Self {
        Self {
            panes: pane_grid::State::with_configuration(Self::default_pane_config()),
            focus: None,
            streams: UniqueStreams::default(),
            popout: HashMap::new(),
            trade_fetch_enabled: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Notification(Toast),
    DistributeFetchedData(uuid::Uuid, uuid::Uuid, FetchedData, StreamKind),
}

impl Dashboard {
    fn default_pane_config() -> Configuration<pane::State> {
        Configuration::Split {
            axis: pane_grid::Axis::Vertical,
            ratio: 0.8,
            a: Box::new(Configuration::Split {
                axis: pane_grid::Axis::Horizontal,
                ratio: 0.4,
                a: Box::new(Configuration::Split {
                    axis: pane_grid::Axis::Vertical,
                    ratio: 0.5,
                    a: Box::new(Configuration::Pane(pane::State::default())),
                    b: Box::new(Configuration::Pane(pane::State::default())),
                }),
                b: Box::new(Configuration::Split {
                    axis: pane_grid::Axis::Vertical,
                    ratio: 0.5,
                    a: Box::new(Configuration::Pane(pane::State::default())),
                    b: Box::new(Configuration::Pane(pane::State::default())),
                }),
            }),
            b: Box::new(Configuration::Pane(pane::State::default())),
        }
    }

    pub fn from_config(
        panes: Configuration<pane::State>,
        popout_windows: Vec<(Configuration<pane::State>, WindowSpec)>,
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
            streams: UniqueStreams::default(),
            popout,
            trade_fetch_enabled,
        }
    }

    pub fn load_layout(&mut self) -> Task<Message> {
        let mut open_popouts_tasks: Vec<Task<Message>> = vec![];
        let mut new_popout = Vec::new();
        let mut keys_to_remove = Vec::new();

        for (old_window_id, (_, specs)) in &self.popout {
            keys_to_remove.push((*old_window_id, *specs));
        }

        // remove keys and open new windows
        for (old_window_id, window_spec) in keys_to_remove {
            let (window, task) = window::open(window::Settings {
                position: window::Position::Specific(window_spec.position()),
                size: window_spec.size(),
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

    pub fn update(
        &mut self,
        message: Message,
        main_window: &Window,
        layout_id: &uuid::Uuid,
    ) -> (Task<Message>, Option<Event>) {
        match message {
            Message::SavePopoutSpecs(specs) => {
                for (window_id, new_spec) in specs {
                    if let Some((_, spec)) = self.popout.get_mut(&window_id) {
                        *spec = new_spec;
                    }
                }
            }
            Message::ErrorOccurred(pane_uid, err) => match pane_uid {
                Some(id) => {
                    if let Some(pane_state) = self.get_mut_pane_state_by_uuid(main_window.id, id) {
                        pane_state.notifications.push(Toast::error(err.to_string()));
                    }
                }
                _ => {
                    return (
                        Task::done(Message::Notification(Toast::error(err.to_string()))),
                        None,
                    );
                }
            },
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
                        let focus_pane = if let Some((new_pane, _)) =
                            self.panes.split(axis, pane, pane::State::new())
                        {
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
                            *pane = pane::State::new();
                        }

                        return (Task::done(Message::RefreshStreams), None);
                    }
                    pane::Message::ToggleModal(pane, modal_type) => {
                        if let Some(pane) = self.get_mut_pane(main_window.id, window, pane) {
                            if Some(modal_type) == pane.modal {
                                pane.modal = None;
                            } else {
                                pane.modal = Some(modal_type);
                            }
                        };
                    }
                    pane::Message::ChartUserUpdate(pane, msg) => {
                        if let Some(pane_state) = self.get_mut_pane(main_window.id, window, pane) {
                            match pane_state.content {
                                pane::Content::Heatmap(ref mut chart, _) => {
                                    chart.update(&msg);
                                }
                                pane::Content::Kline(ref mut chart, _) => {
                                    chart.update(&msg);
                                }
                                _ => {}
                            }
                        }
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
                        content_str,
                        is_pane,
                        pane_stream,
                        ticker_info,
                    ) => {
                        let pane;
                        if let Some(parent_pane) = is_pane {
                            pane = parent_pane;
                        } else {
                            pane = self
                                .panes
                                .iter()
                                .next()
                                .map(|(pane, _)| *pane)
                                .expect("No pane found");
                        }

                        let err_occurred =
                            |pane_uid, err| Task::done(Message::ErrorOccurred(pane_uid, err));

                        // prepare unique streams for websocket
                        for stream in &pane_stream {
                            self.streams.add(*stream);
                        }

                        // set pane's stream and content identifiers
                        if let Some(pane_state) = self.get_mut_pane(main_window.id, window, pane) {
                            if let Err(err) = pane_state.set_content(ticker_info, &content_str) {
                                return (err_occurred(Some(pane_state.id), err), None);
                            }

                            // get fetch tasks for pane's content
                            for stream in &pane_stream {
                                if let StreamKind::Kline { .. } = stream {
                                    return (
                                        kline_fetch_task(
                                            *layout_id,
                                            pane_state.id,
                                            *stream,
                                            None,
                                            None,
                                        ),
                                        None,
                                    );
                                }
                            }
                        } else {
                            return (
                                err_occurred(
                                    None,
                                    DashboardError::PaneSet("No pane found".to_string()),
                                ),
                                None,
                            );
                        }
                    }
                    pane::Message::TicksizeSelected(tick_multiply, pane) => {
                        return (
                            self.set_pane_ticksize(main_window.id, window, pane, tick_multiply),
                            None,
                        );
                    }
                    pane::Message::BasisSelected(new_basis, pane) => {
                        if let Some(state) = self.get_mut_pane(main_window.id, window, pane) {
                            state.settings.selected_basis = Some(new_basis);

                            match state.content {
                                pane::Content::Heatmap(ref mut chart, _) => {
                                    chart.set_basis(new_basis);
                                    return (Task::done(Message::RefreshStreams), None);
                                }
                                _ => {}
                            }

                            if let Some((exchange, ticker)) = state.stream_pair() {
                                let chart_kind = state.content.chart_kind().unwrap_or_default();

                                match &chart_kind {
                                    data::chart::KlineChartKind::Candles => match new_basis {
                                        Basis::Time(interval) => {
                                            state.streams = vec![StreamKind::Kline {
                                                exchange,
                                                ticker,
                                                timeframe: interval.into(),
                                            }];
                                        }
                                        Basis::Tick(_) => {
                                            state.streams = vec![StreamKind::DepthAndTrades {
                                                exchange,
                                                ticker,
                                            }];
                                        }
                                    },
                                    data::chart::KlineChartKind::Footprint { .. } => {
                                        match new_basis {
                                            Basis::Time(interval) => {
                                                state.streams = vec![
                                                    StreamKind::Kline {
                                                        exchange,
                                                        ticker,
                                                        timeframe: interval.into(),
                                                    },
                                                    StreamKind::DepthAndTrades { exchange, ticker },
                                                ];
                                            }
                                            Basis::Tick(_) => {
                                                state.streams = vec![StreamKind::DepthAndTrades {
                                                    exchange,
                                                    ticker,
                                                }];
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        match new_basis {
                            Basis::Time(timeframe) => {
                                match self.set_pane_timeframe(
                                    main_window.id,
                                    window,
                                    pane,
                                    timeframe.into(),
                                ) {
                                    Ok((stream, pane_uid)) => {
                                        if let StreamKind::Kline { .. } = stream {
                                            let task = kline_fetch_task(
                                                *layout_id, pane_uid, *stream, None, None,
                                            );

                                            return (
                                                Task::done(Message::RefreshStreams).chain(task),
                                                None,
                                            );
                                        }
                                    }
                                    Err(err) => {
                                        return (
                                            Task::done(Message::ErrorOccurred(None, err)),
                                            None,
                                        );
                                    }
                                }
                            }
                            Basis::Tick(size) => {
                                if let Some(pane_state) =
                                    self.get_mut_pane(main_window.id, window, pane)
                                {
                                    if let pane::Content::Kline(chart, _) = &mut pane_state.content
                                    {
                                        chart.set_tick_basis(size);
                                    }
                                }
                            }
                        }

                        return (Task::done(Message::RefreshStreams), None);
                    }
                    pane::Message::Popout => return (self.popout_pane(main_window), None),
                    pane::Message::Merge => return (self.merge_pane(main_window), None),
                    pane::Message::ToggleIndicator(pane, indicator_str) => {
                        if let Some(pane_state) = self.get_mut_pane(main_window.id, window, pane) {
                            pane_state.content.toggle_indicator(&indicator_str);
                        }
                    }
                    pane::Message::DeleteNotification(pane, idx) => {
                        if let Some(pane_state) = self.get_mut_pane(main_window.id, window, pane) {
                            pane_state.notifications.remove(idx);
                        }
                    }
                    pane::Message::ReorderIndicator(pane, event) => {
                        if let Some(pane_state) = self.get_mut_pane(main_window.id, window, pane) {
                            pane_state.content.reorder_indicators(&event);
                        }
                    }
                    pane::Message::ClusterKindSelected(pane, cluster_kind) => {
                        if let Some(pane_state) = self.get_mut_pane(main_window.id, window, pane) {
                            if let pane::Content::Kline(chart, _) = &mut pane_state.content {
                                chart.set_cluster_kind(cluster_kind);
                            }
                        }
                    }
                    pane::Message::StudyConfigurator(pane, msg) => {
                        if let Some(pane_state) = self.get_mut_pane(main_window.id, window, pane) {
                            if let pane::Content::Kline(chart, _) = &mut pane_state.content {
                                chart.update_study_configurator(msg);
                            }
                        }
                    }
                }
            }
            Message::LayoutFetchAll => {
                let mut fetched_panes = vec![];

                self.iter_all_panes(main_window.id)
                    .for_each(|(window, pane, pane_state)| {
                        if let pane::Content::Kline(_, _) = pane_state.content {
                            if pane_state
                                .settings
                                .selected_basis
                                .is_some_and(|basis| basis.is_time())
                            {
                                fetched_panes.push((window, pane));
                            }
                        }
                    });

                return (
                    Task::batch(self.klines_fetch_all_task(*layout_id, main_window.id)),
                    None,
                );
            }
            Message::RefreshStreams => {
                let all_pane_streams = self
                    .iter_all_panes(main_window.id)
                    .flat_map(|(_, _, pane_state)| &pane_state.streams);

                self.streams = UniqueStreams::from(all_pane_streams);
            }
            Message::ChartRequestedFetch(pane, window, req_id, fetch) => match fetch {
                FetchRange::Kline(from, to) => {
                    let kline_stream =
                        self.get_mut_pane(main_window.id, window, pane)
                            .and_then(|state| {
                                state
                                    .streams
                                    .iter()
                                    .find(|stream| matches!(stream, StreamKind::Kline { .. }))
                                    .map(|stream| (*stream, state.id))
                            });

                    if let Some((stream, pane_uid)) = kline_stream {
                        return (
                            kline_fetch_task(
                                *layout_id,
                                pane_uid,
                                stream,
                                Some(req_id),
                                Some((from, to)),
                            ),
                            None,
                        );
                    }
                }
                FetchRange::OpenInterest(from, to) => {
                    let kline_stream =
                        self.get_mut_pane(main_window.id, window, pane)
                            .and_then(|state| {
                                state
                                    .streams
                                    .iter()
                                    .find(|stream| matches!(stream, StreamKind::Kline { .. }))
                                    .map(|stream| (*stream, state.id))
                            });

                    if let Some((stream, pane_uid)) = kline_stream {
                        return (
                            oi_fetch_task(
                                *layout_id,
                                pane_uid,
                                stream,
                                Some(req_id),
                                Some((from, to)),
                            ),
                            None,
                        );
                    }
                }
                FetchRange::Trades(from_time, to_time) => {
                    if !self.trade_fetch_enabled {
                        return (Task::none(), None);
                    }

                    let trade_info =
                        self.get_pane(main_window.id, window, pane)
                            .and_then(|state| {
                                state.streams.iter().find_map(|stream| {
                                    if let StreamKind::DepthAndTrades { exchange, ticker } = stream
                                    {
                                        Some((*exchange, *ticker, state.id, *stream))
                                    } else {
                                        None
                                    }
                                })
                            });

                    if let Some((exchange, ticker, pane_uid, stream)) = trade_info {
                        let is_binance = matches!(
                            exchange,
                            Exchange::BinanceSpot
                                | Exchange::BinanceLinear
                                | Exchange::BinanceInverse
                        );

                        if is_binance {
                            let data_path = data::data_path(Some("market_data/binance/"));
                            let dashboard_id = *layout_id;

                            let (task, handle) = Task::sip(
                                fetch_trades_batched(ticker, from_time, to_time, data_path),
                                move |batch| {
                                    let data = FetchedData::Trades(batch, to_time);
                                    Message::DistributeFetchedData(
                                        dashboard_id,
                                        pane_uid,
                                        data,
                                        stream,
                                    )
                                },
                                move |result| match result {
                                    Ok(()) => {
                                        Message::ChangePaneStatus(pane_uid, pane::Status::Ready)
                                    }
                                    Err(err) => Message::ErrorOccurred(
                                        Some(pane_uid),
                                        DashboardError::Fetch(err.to_string()),
                                    ),
                                },
                            )
                            .abortable();

                            if let Some(state) = self.get_mut_pane(main_window.id, window, pane) {
                                if let pane::Content::Kline(chart, _) = &mut state.content {
                                    chart.set_handle(handle.abort_on_drop());
                                }
                            };

                            return (task, None);
                        }
                    }
                }
            },
            Message::ChangePaneStatus(pane_uid, status) => {
                if let Some(pane_state) = self.get_mut_pane_state_by_uuid(main_window.id, pane_uid)
                {
                    pane_state.status = status;
                }
            }
            Message::DistributeFetchedData(layout_id, pane_uid, data, stream) => {
                return (
                    Task::none(),
                    Some(Event::DistributeFetchedData(
                        layout_id, pane_uid, data, stream,
                    )),
                );
            }
            Message::Notification(toast) => {
                return (Task::none(), Some(Event::Notification(toast)));
            }
        }

        (Task::none(), None)
    }

    fn new_pane(
        &mut self,
        axis: pane_grid::Axis,
        main_window: &Window,
        pane_state: Option<pane::State>,
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
                let result = self.panes.split(axis, pane, pane_state.unwrap_or_default());

                if let Some((pane, _)) = result {
                    return self.focus_pane(main_window.id, pane);
                }
            } else {
                let (state, pane) = pane_grid::State::new(pane_state.unwrap_or_default());
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
                let result = self.panes.split(axis, pane, pane::State::new());

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
                    min_size: Some(iced::Size::new(400.0, 300.0)),
                    ..window::settings()
                });

                let (state, id) = pane_grid::State::new(pane);
                self.popout.insert(window, (state, WindowSpec::default()));

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
    ) -> Option<&pane::State> {
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
    ) -> Option<&mut pane::State> {
        if main_window == window {
            self.panes.get_mut(pane)
        } else {
            self.popout
                .get_mut(&window)
                .and_then(|(panes, _)| panes.get_mut(pane))
        }
    }

    fn get_mut_pane_state_by_uuid(
        &mut self,
        main_window: window::Id,
        uuid: uuid::Uuid,
    ) -> Option<&mut pane::State> {
        self.iter_all_panes_mut(main_window)
            .find(|(_, _, state)| state.id == uuid)
            .map(|(_, _, state)| state)
    }

    fn iter_all_panes(
        &self,
        main_window: window::Id,
    ) -> impl Iterator<Item = (window::Id, pane_grid::Pane, &pane::State)> {
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
    ) -> impl Iterator<Item = (window::Id, pane_grid::Pane, &mut pane::State)> {
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
        timezone: UserTimezone,
    ) -> Element<'a, Message> {
        let focus = self.focus;

        let pane_grid: Element<_> = PaneGrid::new(&self.panes, |id, pane, maximized| {
            let is_focused = focus == Some((main_window.id, id));
            pane.view(
                id,
                self.panes.len(),
                is_focused,
                maximized,
                main_window.id,
                main_window,
                timezone,
            )
        })
        .on_click(pane::Message::PaneClicked)
        .on_resize(8, pane::Message::PaneResized)
        .on_drag(pane::Message::PaneDragged)
        .spacing(6)
        .style(style::pane_grid)
        .into();

        container(pane_grid.map(move |message| Message::Pane(main_window.id, message))).into()
    }

    pub fn view_window<'a>(
        &'a self,
        window: window::Id,
        main_window: &'a Window,
        timezone: UserTimezone,
    ) -> Element<'a, Message> {
        if let Some((state, _)) = self.popout.get(&window) {
            let content = container(
                PaneGrid::new(state, |id, pane, _maximized| {
                    let is_focused = self.focus == Some((window, id));
                    pane.view(
                        id,
                        state.len(),
                        is_focused,
                        false,
                        window,
                        main_window,
                        timezone,
                    )
                })
                .on_click(pane::Message::PaneClicked),
            )
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
                    pane::Content::Kline(ref mut chart, _) => {
                        chart.change_tick_size(
                            new_tick_multiply.multiply_with_min_tick_size(ticker_info),
                        );

                        chart.reset_request_handler();
                    }
                    pane::Content::Heatmap(ref mut chart, _) => {
                        chart.change_tick_size(
                            new_tick_multiply.multiply_with_min_tick_size(ticker_info),
                        );
                    }
                    _ => {
                        return Task::done(Message::ErrorOccurred(
                            Some(pane_state.id),
                            DashboardError::PaneSet(
                                "No chart found to change ticksize".to_string(),
                            ),
                        ));
                    }
                }
            } else {
                return Task::done(Message::ErrorOccurred(
                    Some(pane_state.id),
                    DashboardError::PaneSet("No min ticksize found".to_string()),
                ));
            }
        } else {
            return Task::done(Message::ErrorOccurred(
                None,
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
    ) -> Result<(&StreamKind, uuid::Uuid), DashboardError> {
        if let Some(pane_state) = self.get_mut_pane(main_window, window, pane) {
            pane_state.settings.selected_basis = Some(Basis::Time(new_timeframe.to_milliseconds()));

            if let Some(stream_type) = pane_state
                .streams
                .iter_mut()
                .find(|stream_type| matches!(stream_type, StreamKind::Kline { .. }))
            {
                if let StreamKind::Kline { timeframe, .. } = stream_type {
                    *timeframe = new_timeframe;
                }

                if let pane::Content::Kline(_, _) = &pane_state.content {
                    return Ok((stream_type, pane_state.id));
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
        ticker_info: TickerInfo,
        exchange: Exchange,
        content: &str,
    ) -> Task<Message> {
        if let Some((window, selected_pane)) = self.focus {
            if let Some(pane_state) = self.get_mut_pane(main_window, window, selected_pane) {
                return pane_state
                    .init_content_task(content, exchange, ticker_info, selected_pane)
                    .map(move |msg| Message::Pane(window, msg));
            }
        } else {
            return Task::done(Message::Notification(Toast::warn(
                "Select a pane first".to_string(),
            )));
        }

        Task::none()
    }

    pub fn toggle_trade_fetch(&mut self, is_enabled: bool, main_window: &Window) {
        self.trade_fetch_enabled = is_enabled;

        self.iter_all_panes_mut(main_window.id)
            .for_each(|(_, _, pane_state)| {
                if let pane::Content::Kline(chart, _) = &mut pane_state.content {
                    chart.reset_request_handler();
                }
            });
    }

    pub fn distribute_fetched_data(
        &mut self,
        main_window: window::Id,
        pane_uid: uuid::Uuid,
        data: FetchedData,
        stream_type: StreamKind,
    ) -> Task<Message> {
        match data {
            FetchedData::Trades(trades, to_time) => {
                let last_trade_time = trades.last().map_or(0, |trade| trade.time);

                if last_trade_time < to_time {
                    if let Err(reason) =
                        self.insert_fetched_trades(main_window, pane_uid, &trades, false)
                    {
                        return Task::done(Message::ErrorOccurred(Some(pane_uid), reason));
                    }
                } else {
                    let filtered_batch = trades
                        .iter()
                        .filter(|trade| trade.time <= to_time)
                        .copied()
                        .collect::<Vec<_>>();

                    if let Err(reason) =
                        self.insert_fetched_trades(main_window, pane_uid, &filtered_batch, true)
                    {
                        return Task::done(Message::ErrorOccurred(Some(pane_uid), reason));
                    }
                }
            }
            FetchedData::Klines(klines, req_id) => {
                if let Some(pane_state) = self.get_mut_pane_state_by_uuid(main_window, pane_uid) {
                    pane_state.status = pane::Status::Ready;

                    if let StreamKind::Kline { timeframe, .. } = stream_type {
                        pane_state.insert_klines_vec(req_id, timeframe, &klines);
                    }
                }
            }
            FetchedData::OI(oi, req_id) => {
                if let Some(pane_state) = self.get_mut_pane_state_by_uuid(main_window, pane_uid) {
                    pane_state.status = pane::Status::Ready;

                    if let StreamKind::Kline { .. } = stream_type {
                        pane_state.insert_oi_vec(req_id, &oi);
                    }
                }
            }
        }

        Task::none()
    }

    fn insert_fetched_trades(
        &mut self,
        main_window: window::Id,
        pane_uid: uuid::Uuid,
        trades: &[Trade],
        is_batches_done: bool,
    ) -> Result<(), DashboardError> {
        let pane_state = self
            .get_mut_pane_state_by_uuid(main_window, pane_uid)
            .ok_or_else(|| {
                DashboardError::Unknown(
                    "No matching pane state found for fetched trades".to_string(),
                )
            })?;

        match &mut pane_state.status {
            pane::Status::Loading(pane::InfoType::FetchingTrades(count)) => {
                *count += trades.len();
            }
            _ => {
                pane_state.status =
                    pane::Status::Loading(pane::InfoType::FetchingTrades(trades.len()));
            }
        }

        match &mut pane_state.content {
            pane::Content::Kline(chart, _) => {
                chart.insert_raw_trades(trades.to_owned(), is_batches_done);

                if is_batches_done {
                    pane_state.status = pane::Status::Ready;
                }

                Ok(())
            }
            _ => Err(DashboardError::Unknown(
                "No matching chart found for fetched trades".to_string(),
            )),
        }
    }

    pub fn update_latest_klines(
        &mut self,
        stream: &StreamKind,
        kline: &Kline,
        main_window: window::Id,
    ) -> Task<Message> {
        let mut tasks = vec![];
        let mut found_match = false;

        self.iter_all_panes_mut(main_window)
            .for_each(|(window, pane, pane_state)| {
                if pane_state.matches_stream(stream) {
                    let action = match &mut pane_state.content {
                        pane::Content::Kline(chart, _) => chart.update_latest_kline(kline),
                        _ => None,
                    };

                    match action {
                        Some(chart::Action::ErrorOccurred(err)) => {
                            tasks.push(Task::done(Message::ErrorOccurred(
                                Some(pane_state.id),
                                DashboardError::Unknown(err.to_string()),
                            )));
                        }
                        Some(chart::Action::FetchRequested(req_id, range)) => {
                            tasks.push(Task::done(Message::ChartRequestedFetch(
                                pane, window, req_id, range,
                            )));
                        }
                        None => {}
                    }

                    found_match = true;
                }
            });

        if !found_match {
            log::warn!("{stream:?} stream had no matching panes - dropping");
            tasks.push(Task::done(Message::RefreshStreams));
        }

        Task::batch(tasks)
    }

    pub fn update_depth_and_trades(
        &mut self,
        stream: &StreamKind,
        depth_update_t: u64,
        depth: &Depth,
        trades_buffer: &[Trade],
        main_window: window::Id,
    ) -> Task<Message> {
        let mut found_match = false;

        self.iter_all_panes_mut(main_window)
            .for_each(|(_, _, pane_state)| {
                if pane_state.matches_stream(stream) {
                    match &mut pane_state.content {
                        pane::Content::Heatmap(chart, _) => {
                            chart.insert_datapoint(trades_buffer, depth_update_t, depth);
                        }
                        pane::Content::Kline(chart, _) => {
                            chart.insert_trades_buffer(trades_buffer, depth_update_t);
                        }
                        pane::Content::TimeAndSales(chart) => {
                            chart.update(trades_buffer);
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

    pub fn invalidate_all_panes(&mut self, main_window: window::Id) {
        self.iter_all_panes_mut(main_window)
            .for_each(|(_, _, state)| state.invalidate(Instant::now()));
    }

    pub fn tick(&mut self, now: Instant, main_window: window::Id) {
        self.iter_all_panes_mut(main_window)
            .for_each(|(_, _, state)| state.tick(now));
    }

    pub fn market_subscriptions(&self) -> Subscription<exchange::Event> {
        let stream_specs = self.streams.combined();

        let unique_streams = stream_specs
            .iter()
            .flat_map(|(exchange, specs)| {
                let mut subs = vec![];

                if !specs.depth.is_empty() {
                    let depth_subs = specs
                        .depth
                        .iter()
                        .map(|(_, ticker)| depth_subscription(*exchange, *ticker))
                        .collect::<Vec<_>>();

                    if !depth_subs.is_empty() {
                        subs.push(Subscription::batch(depth_subs));
                    }
                }

                let kline_params = specs
                    .kline
                    .iter()
                    .map(|(_, ticker, timeframe)| (*ticker, *timeframe))
                    .collect::<Vec<_>>();

                if !kline_params.is_empty() {
                    subs.push(kline_subscription(*exchange, kline_params));
                }

                subs
            })
            .collect::<Vec<Subscription<exchange::Event>>>();

        Subscription::batch(unique_streams)
    }

    fn klines_fetch_all_task(
        &self,
        layout_id: uuid::Uuid,
        main_window_id: window::Id,
    ) -> Vec<Task<Message>> {
        let mut tasks: Vec<Task<Message>> = vec![];

        for (exchange, ticker, timeframe) in self.streams.kline_streams(None) {
            let stream_kind = StreamKind::Kline {
                exchange,
                ticker,
                timeframe,
            };

            let matching_panes = self
                .iter_all_panes(main_window_id)
                .filter(|(_, _, pane_state)| pane_state.matches_stream(&stream_kind))
                .map(|(_, _, state)| state.id)
                .collect::<Vec<uuid::Uuid>>();

            if matching_panes.is_empty() {
                let fetch_task = Task::perform(
                    adapter::fetch_klines(exchange, ticker, timeframe, None)
                        .map_err(|err| format!("{err}")),
                    move |result| match result {
                        Ok(_) => Message::Notification(Toast::warn(format!(
                            "Fetched klines for stream with no matching panes: {exchange:?} {:?} {timeframe:?}",
                            ticker.to_full_symbol_and_type(),
                        ))),
                        Err(err) => Message::Notification(Toast::error(format!(
                            "Failed to fetch klines for stream: {exchange:?} {:?} {timeframe:?} {err}",
                            ticker.to_full_symbol_and_type(),
                        ))),
                    },
                );
                tasks.push(fetch_task);
            } else {
                for pane_uid in matching_panes {
                    tasks.push(kline_fetch_task(
                        layout_id,
                        pane_uid,
                        stream_kind,
                        None,
                        None,
                    ));
                }
            }
        }

        tasks
    }
}

fn oi_fetch_task(
    layout_id: uuid::Uuid,
    pane_uid: uuid::Uuid,
    stream: StreamKind,
    req_id: Option<uuid::Uuid>,
    range: Option<(u64, u64)>,
) -> Task<Message> {
    let update_status = Task::done(Message::ChangePaneStatus(
        pane_uid,
        pane::Status::Loading(pane::InfoType::FetchingOI),
    ));

    let fetch_task = match stream {
        StreamKind::Kline {
            exchange,
            timeframe,
            ticker,
        } => Task::perform(
            adapter::fetch_open_interest(exchange, ticker, timeframe, range)
                .map_err(|err| format!("{err}")),
            move |result| match result {
                Ok(oi) => {
                    let data = FetchedData::OI(oi, req_id);
                    Message::DistributeFetchedData(layout_id, pane_uid, data, stream)
                }
                Err(err) => Message::ErrorOccurred(Some(pane_uid), DashboardError::Fetch(err)),
            },
        ),
        _ => Task::none(),
    };

    update_status.chain(fetch_task)
}

fn kline_fetch_task(
    layout_id: uuid::Uuid,
    pane_uid: uuid::Uuid,
    stream: StreamKind,
    req_id: Option<uuid::Uuid>,
    range: Option<(u64, u64)>,
) -> Task<Message> {
    let update_status = Task::done(Message::ChangePaneStatus(
        pane_uid,
        pane::Status::Loading(pane::InfoType::FetchingKlines),
    ));

    let fetch_task = match stream {
        StreamKind::Kline {
            exchange,
            ticker,
            timeframe,
        } => Task::perform(
            adapter::fetch_klines(exchange, ticker, timeframe, range)
                .map_err(|err| format!("{err}")),
            move |result| match result {
                Ok(klines) => {
                    let data = FetchedData::Klines(klines, req_id);
                    Message::DistributeFetchedData(layout_id, pane_uid, data, stream)
                }
                Err(err) => Message::ErrorOccurred(Some(pane_uid), DashboardError::Fetch(err)),
            },
        ),
        _ => Task::none(),
    };

    update_status.chain(fetch_task)
}

pub fn fetch_trades_batched(
    ticker: Ticker,
    from_time: u64,
    to_time: u64,
    data_path: PathBuf,
) -> impl Straw<(), Vec<Trade>, StreamError> {
    sipper(async move |mut progress| {
        let mut latest_trade_t = from_time;

        while latest_trade_t < to_time {
            match binance::fetch_trades(ticker, latest_trade_t, data_path.clone()).await {
                Ok(batch) => {
                    if batch.is_empty() {
                        break;
                    }

                    latest_trade_t = batch.last().map_or(latest_trade_t, |trade| trade.time);

                    let () = progress.send(batch).await;
                }
                Err(err) => return Err(err),
            }
        }

        Ok(())
    })
}

pub fn depth_subscription(exchange: Exchange, ticker: Ticker) -> Subscription<exchange::Event> {
    let config = StreamConfig::new(ticker, exchange);
    match exchange {
        Exchange::BinanceSpot | Exchange::BinanceInverse | Exchange::BinanceLinear => {
            Subscription::run_with(config, move |cfg| binance::connect_market_stream(cfg.id))
        }
        Exchange::BybitSpot | Exchange::BybitLinear | Exchange::BybitInverse => {
            Subscription::run_with(config, move |cfg| bybit::connect_market_stream(cfg.id))
        }
    }
}

pub fn kline_subscription(
    exchange: Exchange,
    kline_subs: Vec<(Ticker, Timeframe)>,
) -> Subscription<exchange::Event> {
    let config = StreamConfig::new(kline_subs, exchange);
    match exchange {
        Exchange::BinanceSpot | Exchange::BinanceInverse | Exchange::BinanceLinear => {
            Subscription::run_with(config, move |cfg| {
                binance::connect_kline_stream(cfg.id.clone(), cfg.market_type)
            })
        }
        Exchange::BybitSpot | Exchange::BybitInverse | Exchange::BybitLinear => {
            Subscription::run_with(config, move |cfg| {
                bybit::connect_kline_stream(cfg.id.clone(), cfg.market_type)
            })
        }
    }
}
