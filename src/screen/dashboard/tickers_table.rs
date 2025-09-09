use crate::{
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
        Button, Space, button, column, container, horizontal_rule, horizontal_space, row,
        scrollable::{self, AbsoluteOffset},
        text, text_input,
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

    pub fn update_table(
        &mut self,
        exchange: Exchange,
        ticker_rows: FxHashMap<Ticker, TickerStats>,
    ) {
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
                container(create_expanded_ticker_card(ticker, display_data, is_fav))
                    .style(style::ticker_card)
                    .into()
            } else {
                create_ticker_card(ticker, display_data)
            }
        } else {
            create_ticker_card(ticker, display_data)
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
                horizontal_space(),
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

    pub fn update_ticker_info(
        &mut self,
        _exchange: Exchange,
        info: HashMap<Ticker, Option<TickerInfo>>,
    ) {
        for (ticker, ticker_info) in info.into_iter() {
            self.tickers_info.insert(ticker, ticker_info);
        }
    }

    pub fn update_ticker_rows(&mut self, exchange: Exchange, stats: HashMap<Ticker, TickerStats>) {
        let stats_fxh = stats.into_iter().collect::<FxHashMap<_, _>>();

        let tickers_set: FxHashSet<_> = self.tickers_info.keys().copied().collect();
        let filtered = stats_fxh
            .into_iter()
            .filter(|(t, _)| t.exchange == exchange && tickers_set.contains(t))
            .collect();

        self.update_table(exchange, filtered);
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
        let matches_search = |row: &TickerRowData| {
            if self.search_query.is_empty() {
                return true;
            }
            let (display_str, _) = row.ticker.display_symbol_and_type();
            let (raw_str, _) = row.ticker.to_full_symbol_and_type();
            display_str.contains(&self.search_query) || raw_str.contains(&self.search_query)
        };
        let matches_market =
            |row: &TickerRowData| self.selected_markets.contains(&row.ticker.market_type());
        let matches_exchange = |row: &TickerRowData| {
            self.selected_exchanges
                .contains(&ExchangeInclusive::of(row.exchange))
        };

        let rest_rows: Vec<&TickerRowData> = self
            .ticker_rows
            .iter()
            .filter(|row| {
                (!self.show_favorites || !row.is_favorited)
                    && matches_market(row)
                    && matches_exchange(row)
                    && matches_search(row)
            })
            .collect();

        let mut fav_rows: Vec<&TickerRowData> = Vec::new();
        if self.show_favorites {
            fav_rows = self
                .ticker_rows
                .iter()
                .filter(|row| {
                    row.is_favorited
                        && matches_market(row)
                        && matches_exchange(row)
                        && matches_search(row)
                })
                .collect();
        }

        let fav_n = fav_rows.len();
        let rest_n = rest_rows.len();
        let has_separator = self.show_favorites;
        let has_any_favorites = !self.favorited_tickers.is_empty();

        let top_bar = row![
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
        .spacing(4);

        let sort_and_filter = {
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

            let exchange_filters = column![
                self.exchange_filter_btn(ExchangeInclusive::Bybit, Exchange::BybitLinear, "Bybit"),
                self.exchange_filter_btn(
                    ExchangeInclusive::Binance,
                    Exchange::BinanceLinear,
                    "Binance"
                ),
                self.exchange_filter_btn(
                    ExchangeInclusive::Hyperliquid,
                    Exchange::HyperliquidLinear,
                    "Hyperliquid"
                ),
            ]
            .spacing(4);

            column![
                horizontal_rule(2.0).style(style::split_ruler),
                row![
                    Space::new(Length::FillPortion(2), Length::Shrink),
                    volume_sort,
                    Space::new(Length::FillPortion(1), Length::Shrink),
                    daily_change,
                    Space::new(Length::FillPortion(2), Length::Shrink)
                ]
                .spacing(4),
                horizontal_rule(1.0).style(style::split_ruler),
                row![
                    spot_market_button.width(Length::Fill),
                    linear_markets_btn.width(Length::Fill),
                    inverse_markets_btn.width(Length::Fill),
                ]
                .spacing(4),
                horizontal_rule(1.0).style(style::split_ruler),
                exchange_filters,
                horizontal_rule(1.0).style(style::split_ruler),
                text({
                    let total = rest_n + fav_n;
                    if total == 0 {
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
                    }
                })
                .align_x(Alignment::Center),
                horizontal_rule(2.0).style(style::split_ruler),
            ]
            .align_x(Alignment::Center)
            .spacing(8)
        };

        let sep_block_height: f32 = if has_separator {
            FAVORITES_SEPARATOR_HEIGHT
                + if fav_n == 0 {
                    FAVORITES_EMPTY_HINT_HEIGHT
                } else {
                    0.0
                }
        } else {
            0.0
        };

        let fav_block_height = (fav_n as f32) * TICKER_CARD_HEIGHT;
        let total_height =
            fav_block_height + sep_block_height + (rest_n as f32) * TICKER_CARD_HEIGHT;

        let virtual_count = fav_n + rest_n + if has_separator { 1 } else { 0 };

        let index_start_y = |idx: usize| -> f32 {
            if !has_separator {
                return (idx as f32) * TICKER_CARD_HEIGHT;
            }
            if idx <= fav_n {
                (idx as f32) * TICKER_CARD_HEIGHT
            } else {
                fav_block_height
                    + sep_block_height
                    + ((idx - fav_n - 1) as f32) * TICKER_CARD_HEIGHT
            }
        };

        let pos_to_index = |y: f32| -> usize {
            let header_offset = TOP_BAR_HEIGHT
                + if self.show_sort_options {
                    SORT_AND_FILTER_HEIGHT
                } else {
                    0.0
                };
            let rel_y = (y - header_offset).max(0.0);

            if !has_separator {
                return (rel_y / TICKER_CARD_HEIGHT).floor().max(0.0) as usize;
            }
            if rel_y < fav_block_height {
                (rel_y / TICKER_CARD_HEIGHT).floor().max(0.0) as usize
            } else if rel_y < fav_block_height + sep_block_height {
                fav_n
            } else {
                let off = rel_y - fav_block_height - sep_block_height;
                fav_n + 1 + (off / TICKER_CARD_HEIGHT).floor().max(0.0) as usize
            }
        };

        let scroll_y = self.scroll_offset.y.max(0.0);
        let scroll_bottom = scroll_y + bounds.height;

        let mut first_visible_index =
            pos_to_index(scroll_y).saturating_sub(OVERSCAN_BUFFER as usize);
        if first_visible_index > virtual_count {
            first_visible_index = virtual_count;
        }

        let last_visible_index =
            (pos_to_index(scroll_bottom) + 1 + OVERSCAN_BUFFER as usize).min(virtual_count);

        let top_space = Space::new(
            Length::Shrink,
            Length::Fixed(index_start_y(first_visible_index)),
        );
        let bottom_space = Space::new(
            Length::Shrink,
            Length::Fixed((total_height - index_start_y(last_visible_index)).max(0.0)),
        );

        let mut ticker_cards = column![top_space].spacing(4);

        for idx in first_visible_index..last_visible_index {
            if has_separator && idx == fav_n {
                let sep_block = {
                    let col = if fav_n == 0 {
                        let hint = if has_any_favorites {
                            "No favorited tickers match filters"
                        } else {
                            "Favorited tickers will appear here"
                        };
                        column![
                            text(hint).size(11),
                            horizontal_rule(2.0).style(style::split_ruler),
                        ]
                        .spacing(8)
                        .align_x(Horizontal::Center)
                        .width(Length::Fill)
                    } else {
                        column![horizontal_rule(2.0).style(style::split_ruler),]
                            .align_x(Horizontal::Center)
                            .spacing(16)
                            .width(Length::Fill)
                    };

                    container(col)
                        .width(Length::Fill)
                        .height(Length::Fixed(sep_block_height))
                }
                .padding(padding::top(if fav_n == 0 { 12 } else { 4 }));

                ticker_cards = ticker_cards.push(sep_block);
                continue;
            }

            let row = if idx < fav_n {
                fav_rows[idx]
            } else {
                let base = if has_separator { fav_n + 1 } else { fav_n };
                rest_rows[idx - base]
            };

            if let Some(display_data) = self.display_cache.get(&row.ticker) {
                let card = self.ticker_card_container(
                    row.exchange,
                    &row.ticker,
                    display_data,
                    row.is_favorited,
                );
                ticker_cards = ticker_cards.push(card);
            }
        }

        ticker_cards = ticker_cards.push(bottom_space);

        let mut content = column![top_bar]
            .spacing(8)
            .padding(padding::right(8))
            .width(Length::Fill);

        if self.show_sort_options {
            content = content.push(sort_and_filter);
        }
        content = content.push(ticker_cards);

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

    pub fn subscription(&self) -> Subscription<Message> {
        iced::time::every(std::time::Duration::from_secs(if self.is_shown {
            ACTIVE_UPDATE_INTERVAL
        } else {
            INACTIVE_UPDATE_INTERVAL
        }))
        .map(|_| Message::FetchForTickerStats(None))
    }
}

fn create_ticker_card<'a>(
    ticker: &Ticker,
    display_data: &'a TickerDisplayData,
) -> Element<'a, Message> {
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

    container(
        button(
            row![
                color_column,
                column![
                    row![
                        row![icon, text(&display_data.display_ticker),]
                            .spacing(2)
                            .align_y(alignment::Vertical::Center),
                        Space::new(Length::Fill, Length::Shrink),
                        text(&display_data.daily_change_pct),
                    ]
                    .spacing(4)
                    .align_y(alignment::Vertical::Center),
                    row![
                        price_display,
                        Space::new(Length::Fill, Length::Shrink),
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

fn create_expanded_ticker_card<'a>(
    ticker: &Ticker,
    display_data: &'a TickerDisplayData,
    is_fav: bool,
) -> Element<'a, Message> {
    let (ticker_str, market) = ticker.display_symbol_and_type();

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
            horizontal_space(),
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
            match ticker.exchange {
                Exchange::BybitInverse | Exchange::BybitLinear | Exchange::BybitSpot =>
                    icon_text(Icon::BybitLogo, 12),
                Exchange::BinanceInverse | Exchange::BinanceLinear | Exchange::BinanceSpot =>
                    icon_text(Icon::BinanceLogo, 12),
                Exchange::HyperliquidLinear | Exchange::HyperliquidSpot =>
                    icon_text(Icon::HyperliquidLogo, 12),
            },
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
                    Space::new(Length::Fill, Length::Shrink),
                    text(&display_data.mark_price_display)
                ],
                row![
                    text("Daily Change: ").size(11),
                    Space::new(Length::Fill, Length::Shrink),
                    text(&display_data.daily_change_pct),
                ],
                row![
                    text("Daily Volume: ").size(11),
                    Space::new(Length::Fill, Length::Shrink),
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
        ]
        .width(Length::Fill)
        .spacing(2)
    ]
    .padding(padding::top(8).right(16).left(16).bottom(16))
    .spacing(12)
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
