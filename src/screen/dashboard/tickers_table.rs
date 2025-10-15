use crate::{
    modal::pane::mini_tickers_list::RowSelection,
    style::{self, Icon, icon_text},
    widget::button_with_tooltip,
};
use data::{
    InternalError,
    tickers_table::{
        PriceChangeDirection, Settings, SortOptions, TickerDisplayData, TickerRowData,
        compute_display_data,
    },
};
use exchange::{
    Ticker, TickerInfo, TickerStats,
    adapter::{Exchange, ExchangeInclusive, MarketKind, fetch_ticker_info, fetch_ticker_prices},
};
use iced::{
    Alignment, Element, Length, Renderer, Size, Subscription, Task, Theme,
    alignment::{self, Horizontal, Vertical},
    padding,
    widget::{
        Button, Space, button, column, container, row, rule,
        scrollable::{self, AbsoluteOffset},
        space, text, text_input,
    },
};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::HashMap;

const ACTIVE_UPDATE_INTERVAL: u64 = 13;
const INACTIVE_UPDATE_INTERVAL: u64 = 300;

/// Number of extra cards to render for visibility during scrolling
const OVERSCAN_BUFFER: isize = 3;
const TICKER_CARD_HEIGHT: f32 = 64.0;

const FAVORITES_SEPARATOR_HEIGHT: f32 = 12.0;
const FAVORITES_EMPTY_HINT_HEIGHT: f32 = 32.0;

const TOP_BAR_HEIGHT: f32 = 40.0;
const SORT_AND_FILTER_HEIGHT: f32 = 200.0;

const COMPACT_ROW_HEIGHT: f32 = 28.0;

const EXCHANGE_FILTERS: [(ExchangeInclusive, Exchange, &str); 4] = [
    (ExchangeInclusive::Bybit, Exchange::BybitLinear, "Bybit"),
    (
        ExchangeInclusive::Binance,
        Exchange::BinanceLinear,
        "Binance",
    ),
    (
        ExchangeInclusive::Hyperliquid,
        Exchange::HyperliquidLinear,
        "Hyperliquid",
    ),
    (ExchangeInclusive::Okex, Exchange::OkexLinear, "OKX"),
];

pub fn fetch_tickers_info() -> Task<Message> {
    let fetch_tasks = Exchange::ALL
        .iter()
        .map(|exchange| {
            Task::perform(fetch_ticker_info(*exchange), move |result| match result {
                Ok(ticker_info) => Message::UpdateTickersInfo(*exchange, ticker_info),
                Err(err) => Message::ErrorOccurred(InternalError::Fetch(err.to_string())),
            })
        })
        .collect::<Vec<Task<Message>>>();

    Task::batch(fetch_tasks)
}

pub enum Action {
    TickerSelected(TickerInfo, Option<String>),
    ErrorOccurred(data::InternalError),
    Fetch(Task<Message>),
}

#[derive(Debug, Clone)]
pub enum Message {
    UpdateSearchQuery(String),
    ChangeSortOption(SortOptions),
    ShowSortingOptions,
    TickerSelected(Ticker, Option<String>),
    ExpandTickerCard(Option<Ticker>),
    FavoriteTicker(Ticker),
    Scrolled(scrollable::Viewport),
    ToggleMarketFilter(MarketKind),
    ToggleExchangeFilter(ExchangeInclusive),
    ToggleTable,
    ToggleFavorites,
    FetchForTickerStats(Option<Exchange>),
    UpdateTickersInfo(Exchange, HashMap<Ticker, Option<TickerInfo>>),
    UpdateTickerStats(Exchange, HashMap<Ticker, TickerStats>),
    ErrorOccurred(data::InternalError),
}

pub struct TickersTable {
    ticker_rows: Vec<TickerRowData>,
    pub favorited_tickers: FxHashSet<Ticker>,
    display_cache: FxHashMap<Ticker, TickerDisplayData>,
    search_query: String,
    show_sort_options: bool,
    selected_sort_option: SortOptions,
    pub expand_ticker_card: Option<Ticker>,
    scroll_offset: AbsoluteOffset,
    pub is_shown: bool,
    pub tickers_info: FxHashMap<Ticker, Option<TickerInfo>>,
    selected_exchanges: FxHashSet<ExchangeInclusive>,
    selected_markets: FxHashSet<MarketKind>,
    show_favorites: bool,
}

impl TickersTable {
    pub fn new() -> (Self, Task<Message>) {
        Self::new_with_settings(&Settings::default())
    }

    pub fn new_with_settings(settings: &Settings) -> (Self, Task<Message>) {
        (
            Self {
                ticker_rows: Vec::new(),
                display_cache: FxHashMap::default(),
                favorited_tickers: settings.favorited_tickers.iter().cloned().collect(),
                search_query: String::new(),
                show_sort_options: false,
                selected_sort_option: settings.selected_sort_option,
                expand_ticker_card: None,
                scroll_offset: AbsoluteOffset::default(),
                is_shown: false,
                tickers_info: FxHashMap::default(),
                selected_exchanges: settings.selected_exchanges.iter().cloned().collect(),
                selected_markets: settings.selected_markets.iter().cloned().collect(),
                show_favorites: settings.show_favorites,
            },
            fetch_tickers_info(),
        )
    }

    pub fn settings(&self) -> Settings {
        Settings {
            favorited_tickers: self.favorited_tickers.iter().copied().collect(),
            show_favorites: self.show_favorites,
            selected_sort_option: self.selected_sort_option,
            selected_exchanges: self.selected_exchanges.iter().cloned().collect(),
            selected_markets: self.selected_markets.iter().cloned().collect(),
        }
    }

    pub fn update(&mut self, message: Message) -> Option<Action> {
        match message {
            Message::UpdateSearchQuery(query) => {
                self.search_query = query.to_uppercase();
            }
            Message::ChangeSortOption(option) => {
                self.change_sort_option(option);
            }
            Message::ShowSortingOptions => {
                self.show_sort_options = !self.show_sort_options;
            }
            Message::ExpandTickerCard(is_ticker) => {
                self.expand_ticker_card = is_ticker;
            }
            Message::FavoriteTicker(ticker) => {
                self.favorite_ticker(ticker);
            }
            Message::Scrolled(viewport) => {
                self.scroll_offset = viewport.absolute_offset();
            }
            Message::ToggleMarketFilter(market) => {
                if self.selected_markets.contains(&market) {
                    self.selected_markets.remove(&market);
                } else {
                    self.selected_markets.insert(market);
                }
            }
            Message::ToggleExchangeFilter(exch) => {
                if self.selected_exchanges.contains(&exch) {
                    self.selected_exchanges.remove(&exch);
                } else {
                    self.selected_exchanges.insert(exch);
                }
            }
            Message::ToggleFavorites => {
                self.show_favorites = !self.show_favorites;
            }
            Message::TickerSelected(ticker, content) => {
                let ticker_info = self.tickers_info.get(&ticker).cloned().flatten();

                if let Some(ticker_info) = ticker_info {
                    return Some(Action::TickerSelected(ticker_info, content));
                } else {
                    log::warn!(
                        "Ticker info not found for {ticker:?} on {:?}",
                        ticker.exchange
                    );
                }
            }
            Message::ToggleTable => {
                self.is_shown = !self.is_shown;

                if self.is_shown {
                    self.display_cache.clear();
                    for row in self.ticker_rows.iter_mut() {
                        row.previous_stats = None;
                        self.display_cache.insert(
                            row.ticker,
                            compute_display_data(&row.ticker, &row.stats, None),
                        );
                    }
                }
            }
            Message::FetchForTickerStats(exchange) => {
                let task = if let Some(exchange) = exchange {
                    Task::perform(fetch_ticker_prices(exchange), move |result| match result {
                        Ok(ticker_rows) => Message::UpdateTickerStats(exchange, ticker_rows),
                        Err(err) => Message::ErrorOccurred(InternalError::Fetch(err.to_string())),
                    })
                } else {
                    let exchanges: FxHashSet<Exchange> =
                        self.tickers_info.keys().map(|t| t.exchange).collect();

                    let fetch_tasks = exchanges
                        .into_iter()
                        .map(|exchange| {
                            Task::perform(fetch_ticker_prices(exchange), move |result| match result
                            {
                                Ok(ticker_rows) => {
                                    Message::UpdateTickerStats(exchange, ticker_rows)
                                }
                                Err(err) => {
                                    Message::ErrorOccurred(InternalError::Fetch(err.to_string()))
                                }
                            })
                        })
                        .collect::<Vec<Task<Message>>>();

                    Task::batch(fetch_tasks)
                };

                return Some(Action::Fetch(task));
            }
            Message::UpdateTickerStats(exchange, stats) => {
                self.update_ticker_rows(exchange, stats);
            }
            Message::UpdateTickersInfo(exchange, info) => {
                self.update_ticker_info(exchange, info);

                let task =
                    Task::perform(fetch_ticker_prices(exchange), move |result| match result {
                        Ok(ticker_rows) => Message::UpdateTickerStats(exchange, ticker_rows),
                        Err(err) => Message::ErrorOccurred(InternalError::Fetch(err.to_string())),
                    });

                return Some(Action::Fetch(task));
            }
            Message::ErrorOccurred(err) => {
                log::error!("Error occurred: {err}");
                return Some(Action::ErrorOccurred(err));
            }
        }
        None
    }

    pub fn view(&self, bounds: Size) -> Element<'_, Message> {
        let (fav_rows, rest_rows) = self.filtered_rows_main();
        let fav_n = fav_rows.len();
        let rest_n = rest_rows.len();
        let has_any_favorites = !self.favorited_tickers.is_empty();

        let top_bar = self.top_bar_row();
        let sort_and_filter = self.sort_and_filter_col(fav_n, rest_n);

        let sep_block_height = self.sep_block_height(fav_n);
        let header_offset = self.header_offset_main();

        let virtual_list = VirtualListConfig {
            row_height: TICKER_CARD_HEIGHT,
            header_offset,
            overscan: OVERSCAN_BUFFER as usize,
            gap: if self.show_favorites {
                Some((fav_n, sep_block_height))
            } else {
                None
            },
        };
        let total_rows = fav_n + rest_n;
        let win = virtual_list.window(self.scroll_offset.y, bounds.height, total_rows);

        let list = self.main_list(
            &virtual_list,
            win,
            &fav_rows,
            &rest_rows,
            sep_block_height,
            has_any_favorites,
        );

        let mut content = column![top_bar]
            .spacing(8)
            .padding(padding::right(8))
            .width(Length::Fill);

        if self.show_sort_options {
            content = content.push(sort_and_filter);
        }
        content = content.push(list);

        scrollable::Scrollable::with_direction(
            content,
            scrollable::Direction::Vertical(
                scrollable::Scrollbar::new().width(8).scroller_width(6),
            ),
        )
        .on_scroll(Message::Scrolled)
        .style(style::scroll_bar)
        .into()
    }

    pub fn view_compact_with<'a, M, FSelect, FSearch, FScroll>(
        &'a self,
        bounds: Size,
        search_query: &'a str,
        scroll_offset: AbsoluteOffset,
        on_select: FSelect,
        on_search: FSearch,
        on_scroll: FScroll,
        selected_tickers: Option<&'a [TickerInfo]>,
        base_ticker: Option<TickerInfo>,
    ) -> Element<'a, M>
    where
        M: 'a + Clone,
        FSelect: 'static + Copy + Fn(RowSelection) -> M,
        FSearch: 'static + Copy + Fn(String) -> M,
        FScroll: 'static + Copy + Fn(scrollable::Viewport) -> M,
    {
        let injected_q = search_query.to_uppercase();

        let mut selected_set: FxHashSet<Ticker> = selected_tickers
            .map(|slice| slice.iter().map(|ti| ti.ticker).collect())
            .unwrap_or_default();
        if let Some(bt) = base_ticker {
            selected_set.insert(bt.ticker);
        }

        let (fav_rows, rest_rows) = self.filtered_rows_compact(&injected_q, &selected_set);

        let base_ticker_id = base_ticker.map(|bt| bt.ticker);
        let selected_list: Vec<TickerInfo> = selected_tickers
            .map(|slice| {
                slice
                    .iter()
                    .copied()
                    .filter(|ti| Some(ti.ticker) != base_ticker_id)
                    .collect()
            })
            .unwrap_or_default();
        let selected_count = selected_list.len() + if base_ticker_id.is_some() { 1 } else { 0 };

        let virtual_list = VirtualListConfig {
            row_height: COMPACT_ROW_HEIGHT,
            header_offset: self.header_offset_compact(selected_count),
            overscan: OVERSCAN_BUFFER as usize,
            gap: None,
        };
        let total_n = fav_rows.len() + rest_rows.len();
        let win = virtual_list.window(scroll_offset.y, bounds.height, total_n);

        let top_bar = self.compact_top_bar(search_query, on_search);
        let selected_section = self.compact_selected_section(base_ticker, selected_list, on_select);

        let list = self.compact_list(&virtual_list, win, &fav_rows, &rest_rows, on_select);

        let mut content = column![top_bar]
            .spacing(8)
            .padding(padding::right(8))
            .width(Length::Fill);
        if let Some(sel) = selected_section {
            content = content
                .push(sel)
                .push(rule::horizontal(1.0).style(style::split_ruler));
        }
        content = content.push(list);

        scrollable::Scrollable::with_direction(
            content,
            scrollable::Direction::Vertical(
                scrollable::Scrollbar::new().width(8).scroller_width(6),
            ),
        )
        .on_scroll(on_scroll)
        .style(style::scroll_bar)
        .into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        iced::time::every(std::time::Duration::from_secs(if self.is_shown {
            ACTIVE_UPDATE_INTERVAL
        } else {
            INACTIVE_UPDATE_INTERVAL
        }))
        .map(|_| Message::FetchForTickerStats(None))
    }

    fn update_table(&mut self, exchange: Exchange, ticker_rows: FxHashMap<Ticker, TickerStats>) {
        self.display_cache
            .retain(|ticker, _| ticker.exchange != exchange);

        for (ticker, new_stats) in ticker_rows {
            let (previous_price, updated_row) = if let Some(row) = self
                .ticker_rows
                .iter_mut()
                .find(|r| r.exchange == exchange && r.ticker == ticker)
            {
                let previous_price = Some(row.stats.mark_price);
                row.previous_stats = Some(row.stats);
                row.stats = new_stats;
                (previous_price, *row)
            } else {
                let new_row = TickerRowData {
                    exchange,
                    ticker,
                    stats: new_stats,
                    previous_stats: None,
                    is_favorited: self.favorited_tickers.contains(&ticker),
                };
                self.ticker_rows.push(new_row);
                (None, new_row)
            };

            self.display_cache.insert(
                ticker,
                compute_display_data(&ticker, &updated_row.stats, previous_price),
            );
        }

        self.sort_ticker_rows();
    }

    fn sort_ticker_rows(&mut self) {
        match self.selected_sort_option {
            SortOptions::VolumeDesc => {
                self.ticker_rows.sort_by(|a, b| {
                    b.stats
                        .daily_volume
                        .partial_cmp(&a.stats.daily_volume)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            SortOptions::VolumeAsc => {
                self.ticker_rows.sort_by(|a, b| {
                    a.stats
                        .daily_volume
                        .partial_cmp(&b.stats.daily_volume)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            SortOptions::ChangeDesc => {
                self.ticker_rows.sort_by(|a, b| {
                    b.stats
                        .daily_price_chg
                        .partial_cmp(&a.stats.daily_price_chg)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            SortOptions::ChangeAsc => {
                self.ticker_rows.sort_by(|a, b| {
                    a.stats
                        .daily_price_chg
                        .partial_cmp(&b.stats.daily_price_chg)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
        }
    }

    fn change_sort_option(&mut self, option: SortOptions) {
        if self.selected_sort_option == option {
            self.selected_sort_option = match self.selected_sort_option {
                SortOptions::VolumeDesc => SortOptions::VolumeAsc,
                SortOptions::VolumeAsc => SortOptions::VolumeDesc,
                SortOptions::ChangeDesc => SortOptions::ChangeAsc,
                SortOptions::ChangeAsc => SortOptions::ChangeDesc,
            };
        } else {
            self.selected_sort_option = option;
        }

        self.sort_ticker_rows();
    }

    fn favorite_ticker(&mut self, ticker: Ticker) {
        if let Some(row) = self.ticker_rows.iter_mut().find(|row| row.ticker == ticker) {
            row.is_favorited = !row.is_favorited;

            if row.is_favorited {
                self.favorited_tickers.insert(ticker);
            } else {
                self.favorited_tickers.remove(&ticker);
            }
        }
    }

    fn ticker_card_container<'a>(
        &self,
        exchange: Exchange,
        ticker: &'a Ticker,
        display_data: &'a TickerDisplayData,
        is_fav: bool,
    ) -> Element<'a, Message> {
        if let Some(selected_ticker) = &self.expand_ticker_card {
            let selected_exchange = selected_ticker.exchange;
            if ticker == selected_ticker && exchange == selected_exchange {
                container(expanded_ticker_card(ticker, display_data, is_fav))
                    .style(style::ticker_card)
                    .into()
            } else {
                ticker_card(ticker, display_data)
            }
        } else {
            ticker_card(ticker, display_data)
        }
    }

    fn market_filter_btn<'a>(&'a self, label: &'a str, market: MarketKind) -> Button<'a, Message> {
        let selected = self.selected_markets.contains(&market);

        button(text(label).align_x(Alignment::Center))
            .on_press(Message::ToggleMarketFilter(market))
            .style(move |theme, status| style::button::transparent(theme, status, selected))
    }

    fn exchange_filter_btn<'a>(
        &'a self,
        exch_inc: ExchangeInclusive,
        logo_exchange: Exchange,
        label: &'a str,
    ) -> Element<'a, Message> {
        let selected = self.selected_exchanges.contains(&exch_inc);

        let content = if selected {
            row![
                icon_text(style::exchange_icon(logo_exchange), 12).align_x(Alignment::Center),
                text(label),
                space::horizontal(),
                container(icon_text(Icon::Checkmark, 12)),
            ]
        } else {
            row![
                icon_text(style::exchange_icon(logo_exchange), 12).align_x(Alignment::Center),
                text(label)
            ]
        };

        let btn = button(content.spacing(4).width(Length::Fill))
            .style(move |theme, status| style::button::modifier(theme, status, selected))
            .on_press(Message::ToggleExchangeFilter(exch_inc))
            .width(Length::Fill);

        container(btn)
            .padding(2)
            .style(style::dragger_row_container)
            .into()
    }

    fn update_ticker_info(
        &mut self,
        _exchange: Exchange,
        info: HashMap<Ticker, Option<TickerInfo>>,
    ) {
        for (ticker, ticker_info) in info.into_iter() {
            self.tickers_info.insert(ticker, ticker_info);
        }
    }

    fn update_ticker_rows(&mut self, exchange: Exchange, stats: HashMap<Ticker, TickerStats>) {
        let stats_fxh = stats.into_iter().collect::<FxHashMap<_, _>>();

        let tickers_set: FxHashSet<_> = self.tickers_info.keys().copied().collect();
        let filtered = stats_fxh
            .into_iter()
            .filter(|(t, _)| t.exchange == exchange && tickers_set.contains(t))
            .collect();

        self.update_table(exchange, filtered);
    }

    fn sep_block_height(&self, fav_n: usize) -> f32 {
        if self.show_favorites {
            FAVORITES_SEPARATOR_HEIGHT
                + if fav_n == 0 {
                    FAVORITES_EMPTY_HINT_HEIGHT
                } else {
                    0.0
                }
        } else {
            0.0
        }
    }

    fn header_offset_main(&self) -> f32 {
        TOP_BAR_HEIGHT
            + if self.show_sort_options {
                SORT_AND_FILTER_HEIGHT
            } else {
                0.0
            }
    }

    fn header_offset_compact(&self, selected_count: usize) -> f32 {
        const GAP: f32 = 8.0;
        const RULE_H: f32 = 1.0;

        let selected_block_height = if selected_count > 0 {
            let rows_h = (selected_count as f32) * COMPACT_ROW_HEIGHT;
            let gaps_h = ((selected_count.saturating_sub(1)) as f32) * 2.0;
            rows_h + gaps_h
        } else {
            0.0
        };

        TOP_BAR_HEIGHT
            + GAP
            + if selected_count > 0 {
                selected_block_height + RULE_H + (2.0 * GAP)
            } else {
                0.0
            }
    }

    fn top_bar_row(&self) -> Element<'_, Message> {
        row![
            text_input("Search for a ticker...", &self.search_query)
                .style(|theme, status| style::validated_text_input(theme, status, true))
                .on_input(Message::UpdateSearchQuery)
                .align_x(Horizontal::Left)
                .padding(6),
            button(
                icon_text(Icon::Sort, 14)
                    .align_x(Horizontal::Center)
                    .align_y(Vertical::Center)
            )
            .height(28)
            .width(28)
            .on_press(Message::ShowSortingOptions)
            .style(move |theme, status| style::button::transparent(
                theme,
                status,
                self.show_sort_options
            )),
            button(
                icon_text(Icon::StarFilled, 12)
                    .align_x(Horizontal::Center)
                    .align_y(Vertical::Center)
            )
            .width(28)
            .height(28)
            .on_press(Message::ToggleFavorites)
            .style(move |theme, status| {
                style::button::transparent(theme, status, self.show_favorites)
            })
        ]
        .align_y(Vertical::Center)
        .spacing(4)
        .into()
    }

    fn sort_and_filter_col(&self, fav_n: usize, rest_n: usize) -> Element<'_, Message> {
        let volume_sort_button =
            sort_button("Volume", SortOptions::VolumeAsc, self.selected_sort_option);
        let volume_sort = volume_sort_button.style(move |theme, status| {
            style::button::transparent(
                theme,
                status,
                matches!(
                    self.selected_sort_option,
                    SortOptions::VolumeAsc | SortOptions::VolumeDesc
                ),
            )
        });

        let change_sort_button =
            sort_button("Change", SortOptions::ChangeAsc, self.selected_sort_option);
        let daily_change = change_sort_button.style(move |theme, status| {
            style::button::transparent(
                theme,
                status,
                matches!(
                    self.selected_sort_option,
                    SortOptions::ChangeAsc | SortOptions::ChangeDesc
                ),
            )
        });

        let spot_market_button = self.market_filter_btn("Spot", MarketKind::Spot);
        let linear_markets_btn = self.market_filter_btn("Linear", MarketKind::LinearPerps);
        let inverse_markets_btn = self.market_filter_btn("Inverse", MarketKind::InversePerps);

        let exchange_filters = {
            let mut col = column![];
            for (exchange_inclusive, exchange_logo, label) in EXCHANGE_FILTERS {
                col = col.push(self.exchange_filter_btn(exchange_inclusive, exchange_logo, label));
            }
            col.spacing(4)
        };

        let total = rest_n + fav_n;

        column![
            rule::horizontal(2.0).style(style::split_ruler),
            row![
                Space::new()
                    .width(Length::FillPortion(2))
                    .height(Length::Shrink),
                volume_sort,
                Space::new()
                    .width(Length::FillPortion(1))
                    .height(Length::Shrink),
                daily_change,
                Space::new()
                    .width(Length::FillPortion(2))
                    .height(Length::Shrink),
            ]
            .spacing(4),
            rule::horizontal(1.0).style(style::split_ruler),
            row![
                spot_market_button.width(Length::Fill),
                linear_markets_btn.width(Length::Fill),
                inverse_markets_btn.width(Length::Fill),
            ]
            .spacing(4),
            rule::horizontal(1.0).style(style::split_ruler),
            exchange_filters,
            rule::horizontal(1.0).style(style::split_ruler),
            text(if total == 0 {
                "No tickers match filters".to_string()
            } else {
                let ticker_str = if total == 1 { "ticker" } else { "tickers" };
                let exchanges = self.selected_exchanges.len();
                let exchange_str = if exchanges == 1 {
                    "exchange"
                } else {
                    "exchanges"
                };
                format!(
                    "Showing {} {} from {} {}",
                    total, ticker_str, exchanges, exchange_str
                )
            })
            .align_x(Alignment::Center),
            rule::horizontal(2.0).style(style::split_ruler),
        ]
        .align_x(Alignment::Center)
        .spacing(8)
        .into()
    }

    fn fav_separator_block(
        &self,
        fav_n: usize,
        sep_block_height: f32,
        has_any_favorites: bool,
    ) -> Element<'_, Message> {
        let col = if fav_n == 0 {
            let hint = if has_any_favorites {
                "No favorited tickers match filters"
            } else {
                "Favorited tickers will appear here"
            };
            column![
                text(hint).size(11),
                rule::horizontal(2.0).style(style::split_ruler),
            ]
            .spacing(8)
            .align_x(Horizontal::Center)
            .width(Length::Fill)
        } else {
            column![rule::horizontal(2.0).style(style::split_ruler),]
                .align_x(Horizontal::Center)
                .spacing(16)
                .width(Length::Fill)
        };

        container(col)
            .width(Length::Fill)
            .height(Length::Fixed(sep_block_height))
            .padding(padding::top(if fav_n == 0 { 12 } else { 4 }))
            .into()
    }

    fn main_list<'a>(
        &'a self,
        vcfg: &VirtualListConfig,
        win: VirtualWindow,
        fav_rows: &[&'a TickerRowData],
        rest_rows: &[&'a TickerRowData],
        sep_block_height: f32,
        has_any_favorites: bool,
    ) -> Element<'a, Message> {
        let fav_n = fav_rows.len();

        let top_space = Space::new()
            .width(Length::Shrink)
            .height(Length::Fixed(win.top_space));
        let bottom_space = Space::new()
            .width(Length::Shrink)
            .height(Length::Fixed(win.bottom_space));

        let mut cards = column![top_space].spacing(4);

        for idx in win.first..win.last {
            match vcfg.virtual_to_item(idx) {
                VirtualItemIndex::Gap => {
                    cards = cards.push(self.fav_separator_block(
                        fav_n,
                        sep_block_height,
                        has_any_favorites,
                    ));
                }
                VirtualItemIndex::Row(data_idx) => {
                    let row_ref = if data_idx < fav_n {
                        fav_rows[data_idx]
                    } else {
                        rest_rows[data_idx - fav_n]
                    };
                    if let Some(display_data) = self.display_cache.get(&row_ref.ticker) {
                        cards = cards.push(self.ticker_card_container(
                            row_ref.exchange,
                            &row_ref.ticker,
                            display_data,
                            row_ref.is_favorited,
                        ));
                    }
                }
            }
        }

        cards = cards.push(bottom_space);
        cards.into()
    }

    fn compact_top_bar<'a, M, FSearch>(
        &'a self,
        search_query: &'a str,
        on_search: FSearch,
    ) -> Element<'a, M>
    where
        M: 'a + Clone,
        FSearch: 'static + Copy + Fn(String) -> M,
    {
        row![
            text_input("Search for a ticker...", search_query)
                .style(|theme, status| crate::style::validated_text_input(theme, status, true))
                .on_input(on_search)
                .align_x(Alignment::Start)
                .padding(6),
        ]
        .align_y(Alignment::Center)
        .spacing(4)
        .into()
    }

    fn compact_selected_section<'a, M, FSelect>(
        &'a self,
        base_ticker: Option<TickerInfo>,
        selected_list: Vec<TickerInfo>,
        on_select: FSelect,
    ) -> Option<Element<'a, M>>
    where
        M: 'a + Clone,
        FSelect: 'static + Copy + Fn(RowSelection) -> M,
    {
        if base_ticker.is_none() && selected_list.is_empty() {
            return None;
        }

        let mut col = column![].spacing(2);

        if let Some(bt) = base_ticker {
            let label = self.label_with_suffix(bt.ticker);
            col = col.push(mini_ticker_card(
                bt.ticker.exchange,
                label,
                None,
                None,
                None,
                on_select,
            ));
        }

        for info in selected_list {
            let label = self.label_with_suffix(info.ticker);
            col = col.push(mini_ticker_card(
                info.ticker.exchange,
                label,
                Some(RowSelection::Switch(info)),
                Some(("Remove", Some(RowSelection::Remove(info)))),
                None,
                on_select,
            ));
        }

        Some(col.into())
    }

    fn compact_list<'a, M, FSelect>(
        &'a self,
        vcfg: &VirtualListConfig,
        win: VirtualWindow,
        fav_rows: &[&'a TickerRowData],
        rest_rows: &[&'a TickerRowData],
        on_select: FSelect,
    ) -> Element<'a, M>
    where
        M: 'a + Clone,
        FSelect: 'static + Copy + Fn(RowSelection) -> M,
    {
        let top_space = Space::new()
            .width(Length::Shrink)
            .height(Length::Fixed(win.top_space));
        let bottom_space = Space::new()
            .width(Length::Shrink)
            .height(Length::Fixed(win.bottom_space));

        let mut list = column![top_space].spacing(2);
        for idx in win.first..win.last {
            let VirtualItemIndex::Row(data_idx) = vcfg.virtual_to_item(idx) else {
                continue;
            };
            let row_ref = if data_idx < fav_rows.len() {
                fav_rows[data_idx]
            } else {
                rest_rows[data_idx - fav_rows.len()]
            };

            let label = self.label_with_suffix(row_ref.ticker);
            let info_opt: Option<TickerInfo> =
                self.tickers_info.get(&row_ref.ticker).cloned().flatten();

            let left_action = info_opt.map(RowSelection::Switch);
            let right = Some(("Add", info_opt.map(RowSelection::Add)));

            let row_el =
                mini_ticker_card(row_ref.exchange, label, left_action, right, None, on_select);

            list = list.push(row_el);
        }
        list = list.push(bottom_space);

        list.into()
    }

    fn label_for(&self, ticker: Ticker) -> String {
        if let Some(dd) = self.display_cache.get(&ticker) {
            dd.display_ticker.clone()
        } else {
            let (s, _) = ticker.display_symbol_and_type();
            s
        }
    }

    fn label_with_suffix(&self, ticker: Ticker) -> String {
        let mut s = self.label_for(ticker);
        s.push_str(market_suffix(ticker.market_type()));
        s
    }

    fn filtered_rows<'a>(
        &'a self,
        search_upper: &str,
        excluded: Option<&FxHashSet<Ticker>>,
    ) -> (Vec<&'a TickerRowData>, Vec<&'a TickerRowData>) {
        let matches_search = |row: &TickerRowData| {
            if search_upper.is_empty() {
                return true;
            }
            let (display_str, _) = row.ticker.display_symbol_and_type();
            let (raw_str, _) = row.ticker.to_full_symbol_and_type();
            display_str.contains(search_upper) || raw_str.contains(search_upper)
        };
        let matches_market =
            |row: &TickerRowData| self.selected_markets.contains(&row.ticker.market_type());
        let matches_exchange = |row: &TickerRowData| {
            self.selected_exchanges
                .contains(&ExchangeInclusive::of(row.exchange))
        };

        let fav_rows = if self.show_favorites {
            self.ticker_rows
                .iter()
                .filter(|row| {
                    row.is_favorited
                        && !excluded.is_some_and(|ex| ex.contains(&row.ticker))
                        && matches_market(row)
                        && matches_exchange(row)
                        && matches_search(row)
                })
                .collect()
        } else {
            Vec::new()
        };

        let rest_rows = self
            .ticker_rows
            .iter()
            .filter(|row| {
                (!self.show_favorites || !row.is_favorited)
                    && !excluded.is_some_and(|ex| ex.contains(&row.ticker))
                    && matches_market(row)
                    && matches_exchange(row)
                    && matches_search(row)
            })
            .collect();

        (fav_rows, rest_rows)
    }

    fn filtered_rows_main(&self) -> (Vec<&TickerRowData>, Vec<&TickerRowData>) {
        self.filtered_rows(&self.search_query, None)
    }

    fn filtered_rows_compact<'a>(
        &'a self,
        injected_q: &str,
        excluded: &FxHashSet<Ticker>,
    ) -> (Vec<&'a TickerRowData>, Vec<&'a TickerRowData>) {
        self.filtered_rows(injected_q, Some(excluded))
    }
}

fn ticker_card<'a>(ticker: &Ticker, display_data: &'a TickerDisplayData) -> Element<'a, Message> {
    let color_column = container(column![])
        .height(Length::Fill)
        .width(Length::Fixed(2.0))
        .style(move |theme| style::ticker_card_bar(theme, display_data.card_color_alpha));

    let price_display = if display_data.price_changed_part.is_empty() {
        row![text(&display_data.price_unchanged_part)]
    } else {
        row![
            text(&display_data.price_unchanged_part),
            text(&display_data.price_changed_part).style(move |theme: &Theme| {
                let palette = theme.extended_palette();
                iced::widget::text::Style {
                    color: Some(match display_data.price_change_direction {
                        PriceChangeDirection::Increased => palette.success.base.color,
                        PriceChangeDirection::Decreased => palette.danger.base.color,
                        PriceChangeDirection::Unchanged => palette.background.base.text,
                    }),
                }
            })
        ]
    };

    let icon = icon_text(style::exchange_icon(ticker.exchange), 12);
    let display_ticker = short_card_label(ticker, display_data);

    container(
        button(
            row![
                color_column,
                column![
                    row![
                        row![icon, text(display_ticker),]
                            .spacing(2)
                            .align_y(alignment::Vertical::Center),
                        Space::new().width(Length::Fill).height(Length::Shrink),
                        text(&display_data.daily_change_pct),
                    ]
                    .spacing(4)
                    .align_y(alignment::Vertical::Center),
                    row![
                        price_display,
                        Space::new().width(Length::Fill).height(Length::Shrink),
                        text(&display_data.volume_display),
                    ]
                    .spacing(4),
                ]
                .padding(padding::left(8).right(8).bottom(4).top(4))
                .spacing(4),
            ]
            .align_y(Alignment::Center),
        )
        .style(style::button::ticker_card)
        .on_press(Message::ExpandTickerCard(Some(*ticker))),
    )
    .height(Length::Fixed(56.0))
    .into()
}

fn expanded_ticker_card<'a>(
    ticker: &Ticker,
    display_data: &'a TickerDisplayData,
    is_fav: bool,
) -> Element<'a, Message> {
    let (ticker_str, market) = ticker.display_symbol_and_type();
    let exchange_icon = style::exchange_icon(ticker.exchange);

    column![
        row![
            button(icon_text(Icon::Return, 11))
                .on_press(Message::ExpandTickerCard(None))
                .style(move |theme, status| style::button::transparent(theme, status, false)),
            button(if is_fav {
                icon_text(Icon::StarFilled, 11)
            } else {
                icon_text(Icon::Star, 11)
            })
            .on_press(Message::FavoriteTicker(*ticker))
            .style(move |theme, status| { style::button::transparent(theme, status, false) }),
            space::horizontal(),
            button_with_tooltip(
                icon_text(Icon::Link, 11),
                Message::TickerSelected(*ticker, None),
                Some("Use this ticker on selected pane/group"),
                iced::widget::tooltip::Position::Top,
                move |theme, status| style::button::transparent(theme, status, false)
            ),
        ]
        .spacing(2),
        row![
            icon_text(exchange_icon, 12),
            text(
                ticker_str
                    + " "
                    + &market.to_string()
                    + match market {
                        MarketKind::Spot => "",
                        MarketKind::LinearPerps | MarketKind::InversePerps => " Perp",
                    }
            ),
        ]
        .spacing(2),
        container(
            column![
                row![
                    text("Last Updated Price: ").size(11),
                    Space::new().width(Length::Fill).height(Length::Shrink),
                    text(&display_data.mark_price_display)
                ],
                row![
                    text("Daily Change: ").size(11),
                    Space::new().width(Length::Fill).height(Length::Shrink),
                    text(&display_data.daily_change_pct),
                ],
                row![
                    text("Daily Volume: ").size(11),
                    Space::new().width(Length::Fill).height(Length::Shrink),
                    text(&display_data.volume_display),
                ],
            ]
            .spacing(2)
        )
        .style(|theme: &Theme| {
            let palette = theme.extended_palette();
            iced::widget::container::Style {
                text_color: Some(palette.background.base.text.scale_alpha(0.9)),
                ..Default::default()
            }
        }),
        column![
            init_content_button("Heatmap Chart", "heatmap", *ticker, 180.0),
            init_content_button("Footprint Chart", "footprint", *ticker, 180.0),
            init_content_button("Candlestick Chart", "candlestick", *ticker, 180.0),
            init_content_button("Time&Sales", "time&sales", *ticker, 160.0),
            init_content_button("DOM/Ladder", "ladder", *ticker, 160.0),
            init_content_button("Comparison Chart", "comparison", *ticker, 180.0),
        ]
        .width(Length::Fill)
        .spacing(2)
    ]
    .padding(padding::top(8).right(16).left(16).bottom(16))
    .spacing(12)
    .into()
}

fn mini_ticker_card<'a, M, FSelect>(
    exchange: Exchange,
    label: String,
    left_action: Option<RowSelection>,
    right_label_and_action: Option<(&'static str, Option<RowSelection>)>,
    chip_label: Option<&'static str>,
    on_select: FSelect,
) -> Element<'a, M>
where
    M: 'a + Clone,
    FSelect: 'static + Copy + Fn(RowSelection) -> M,
{
    let icon = icon_text(style::exchange_icon(exchange), 12);

    let left_btn_base = button(
        row![icon, text(label)]
            .spacing(6)
            .align_y(alignment::Vertical::Center)
            .height(Length::Fill),
    )
    .style(|theme, status| style::button::transparent(theme, status, false))
    .width(Length::Fill)
    .height(Length::Fill);

    let left_btn = if let Some(sel) = left_action {
        left_btn_base.on_press(on_select(sel))
    } else {
        left_btn_base
    };

    let right_el: Option<Element<'a, M>> = right_label_and_action.map(|(lbl, action)| {
        let btn_base = button(
            row![text(lbl).size(11)]
                .align_y(alignment::Vertical::Center)
                .height(Length::Fill),
        )
        .style(|theme, status| style::button::transparent(theme, status, false))
        .height(Length::Fill);

        let btn = if let Some(act) = action {
            btn_base.on_press(on_select(act))
        } else {
            btn_base
        };

        btn.into()
    });

    let chip_el: Option<Element<'a, M>> = chip_label.map(|lbl| {
        container(text(lbl).size(11))
            .padding([2, 6])
            .style(style::dragger_row_container)
            .into()
    });

    let mut row_content = row![left_btn]
        .spacing(6)
        .align_y(alignment::Vertical::Center);

    if let Some(chip) = chip_el {
        row_content = row_content.push(chip);
    }
    if let Some(right) = right_el {
        row_content = row_content.push(right);
    }

    container(row_content)
        .style(style::ticker_card)
        .height(Length::Fixed(COMPACT_ROW_HEIGHT))
        .width(Length::Fill)
        .into()
}

fn sort_button(
    label: &str,
    sort_option: SortOptions,
    current_sort: SortOptions,
) -> Button<'_, Message, Theme, Renderer> {
    let (asc_variant, desc_variant) = match sort_option {
        SortOptions::VolumeAsc => (SortOptions::VolumeAsc, SortOptions::VolumeDesc),
        SortOptions::ChangeAsc => (SortOptions::ChangeAsc, SortOptions::ChangeDesc),
        _ => (sort_option, sort_option), // fallback
    };

    button(
        row![
            text(label),
            icon_text(
                if current_sort == desc_variant {
                    Icon::SortDesc
                } else {
                    Icon::SortAsc
                },
                14
            )
        ]
        .spacing(4)
        .align_y(Vertical::Center),
    )
    .on_press(Message::ChangeSortOption(asc_variant))
}

fn init_content_button<'a>(
    label: &'a str,
    content: &str,
    ticker: Ticker,
    width: f32,
) -> Button<'a, Message, Theme, Renderer> {
    button(text(label).align_x(Horizontal::Center))
        .on_press(Message::TickerSelected(ticker, Some(content.to_string())))
        .width(Length::Fixed(width))
}

fn short_card_label(ticker: &Ticker, display_data: &TickerDisplayData) -> String {
    if display_data.display_ticker.len() >= 11 {
        format!("{}...", &display_data.display_ticker[..9])
    } else {
        format!(
            "{}{}",
            display_data.display_ticker,
            match ticker.market_type() {
                MarketKind::Spot => "",
                MarketKind::LinearPerps | MarketKind::InversePerps => "P",
            }
        )
    }
}

fn market_suffix(m: MarketKind) -> &'static str {
    match m {
        MarketKind::Spot => "",
        MarketKind::LinearPerps | MarketKind::InversePerps => "P",
    }
}

#[derive(Clone, Copy, Debug)]
struct VirtualListConfig {
    row_height: f32,
    header_offset: f32,
    overscan: usize,
    /// Optional gap inserted at a specific virtual index (`usize`=idx), with a fixed height(`f32`).
    /// Used for the “favorites” separator in the full view. None for compact view.
    gap: Option<(usize, f32)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VirtualItemIndex {
    Row(usize),
    Gap,
}

#[derive(Clone, Copy, Debug)]
struct VirtualWindow {
    first: usize,
    last: usize,
    top_space: f32,
    bottom_space: f32,
}

impl VirtualListConfig {
    fn virtual_count(&self, total_rows: usize) -> usize {
        total_rows + self.gap.map(|_| 1).unwrap_or(0)
    }

    fn total_height(&self, total_rows: usize) -> f32 {
        (total_rows as f32) * self.row_height + self.gap.map(|(_, h)| h).unwrap_or(0.0)
    }

    fn index_start_y(&self, idx: usize) -> f32 {
        match self.gap {
            None => (idx as f32) * self.row_height,
            Some((gap_idx, gap_h)) => {
                let pre_gap_h = (gap_idx as f32) * self.row_height;
                if idx <= gap_idx {
                    (idx as f32) * self.row_height
                } else {
                    pre_gap_h + gap_h + ((idx - gap_idx - 1) as f32) * self.row_height
                }
            }
        }
    }

    fn pos_to_index(&self, y_abs: f32) -> usize {
        let y = (y_abs - self.header_offset).max(0.0);
        match self.gap {
            None => (y / self.row_height).floor().max(0.0) as usize,
            Some((gap_idx, gap_h)) => {
                let pre_gap_h = (gap_idx as f32) * self.row_height;
                if y < pre_gap_h {
                    (y / self.row_height).floor().max(0.0) as usize
                } else if y < pre_gap_h + gap_h {
                    gap_idx
                } else {
                    let off = y - pre_gap_h - gap_h;
                    gap_idx + 1 + (off / self.row_height).floor().max(0.0) as usize
                }
            }
        }
    }

    fn virtual_to_item(&self, idx: usize) -> VirtualItemIndex {
        if let Some((gap_idx, _)) = self.gap {
            if idx == gap_idx {
                VirtualItemIndex::Gap
            } else if idx < gap_idx {
                VirtualItemIndex::Row(idx)
            } else {
                VirtualItemIndex::Row(idx - 1)
            }
        } else {
            VirtualItemIndex::Row(idx)
        }
    }

    fn window(&self, scroll_y: f32, viewport_h: f32, total_rows: usize) -> VirtualWindow {
        let vcount = self.virtual_count(total_rows);
        let scroll_y = scroll_y.max(0.0);
        let scroll_bottom = scroll_y + viewport_h;

        let mut first = self.pos_to_index(scroll_y).saturating_sub(self.overscan);
        if first > vcount {
            first = vcount;
        }
        let last = (self.pos_to_index(scroll_bottom) + 1 + self.overscan).min(vcount);

        let total_h = self.total_height(total_rows);
        let top_space = self.index_start_y(first);
        let bottom_space = (total_h - self.index_start_y(last)).max(0.0);

        VirtualWindow {
            first,
            last,
            top_space,
            bottom_space,
        }
    }
}
