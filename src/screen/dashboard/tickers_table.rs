use std::collections::{HashMap, HashSet};

use crate::{
    style::{self, ICONS_FONT, Icon, icon_text},
    widget::{button_with_tooltip, tooltip},
};
use data::InternalError;
use exchange::{
    Ticker, TickerInfo, TickerStats,
    adapter::{Exchange, MarketKind, fetch_ticker_info, fetch_ticker_prices},
};
use iced::{
    Alignment, Element, Length, Renderer, Size, Subscription, Task, Theme,
    alignment::{self, Horizontal, Vertical},
    padding,
    widget::{
        Button, Space, Text, button, column, container, horizontal_rule, horizontal_space, row,
        scrollable::{self, AbsoluteOffset},
        text, text_input,
    },
};

const ACTIVE_UPDATE_INTERVAL: u64 = 13;
const INACTIVE_UPDATE_INTERVAL: u64 = 300;

/// Number of extra cards to render for visibility during scrolling
const OVERSCAN_BUFFER: isize = 3;
const TICKER_CARD_HEIGHT: f32 = 64.0;

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

#[derive(Debug, Clone, PartialEq)]
pub enum TickerTab {
    All,
    Bybit,
    Binance,
    Hyperliquid,
    Favorites,
}

#[derive(Clone, Copy)]
struct TickerRowData {
    exchange: Exchange,
    ticker: Ticker,
    stats: TickerStats,
    previous_stats: Option<TickerStats>,
    is_favorited: bool,
}

#[derive(Clone)]
struct TickerDisplayData {
    display_ticker: String,
    daily_change_pct: String,
    volume_display: String,
    mark_price_display: String,
    price_unchanged_part: String,
    price_changed_part: String,
    price_change_direction: PriceChangeDirection,
    card_color_alpha: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SortOptions {
    VolumeAsc,
    VolumeDesc,
    ChangeAsc,
    ChangeDesc,
}

#[derive(Debug, Clone)]
pub enum Message {
    ChangeTickersTableTab(TickerTab),
    UpdateSearchQuery(String),
    ChangeSortOption(SortOptions),
    ShowSortingOptions,
    TickerSelected(Ticker, Exchange, Option<String>),
    ExpandTickerCard(Option<(Ticker, Exchange)>),
    FavoriteTicker(Exchange, Ticker),
    Scrolled(scrollable::Viewport),
    SetMarketFilter(Option<MarketKind>),
    ToggleTable,
    FetchForTickerStats(Option<Exchange>),
    UpdateTickersInfo(Exchange, HashMap<Ticker, Option<TickerInfo>>),
    UpdateTickerStats(Exchange, HashMap<Ticker, TickerStats>),
    ErrorOccurred(data::InternalError),
}

pub struct TickersTable {
    ticker_rows: Vec<TickerRowData>,
    pub favorited_tickers: HashSet<(Exchange, Ticker)>,
    display_cache: HashMap<(Exchange, Ticker), TickerDisplayData>,
    selected_tab: TickerTab,
    search_query: String,
    show_sort_options: bool,
    selected_sort_option: SortOptions,
    selected_market: Option<MarketKind>,
    pub expand_ticker_card: Option<(Ticker, Exchange)>,
    scroll_offset: AbsoluteOffset,
    pub is_shown: bool,
    tickers_info: HashMap<Exchange, HashMap<Ticker, Option<TickerInfo>>>,
}

impl TickersTable {
    pub fn new(favorited_tickers: Vec<(Exchange, Ticker)>) -> (Self, Task<Message>) {
        (
            Self {
                ticker_rows: Vec::new(),
                display_cache: HashMap::new(),
                favorited_tickers: favorited_tickers.into_iter().collect(),
                selected_tab: TickerTab::All,
                search_query: String::new(),
                show_sort_options: false,
                selected_sort_option: SortOptions::VolumeDesc,
                expand_ticker_card: None,
                scroll_offset: AbsoluteOffset::default(),
                selected_market: None,
                is_shown: false,
                tickers_info: HashMap::new(),
            },
            fetch_tickers_info(),
        )
    }

    pub fn update_table(&mut self, exchange: Exchange, ticker_rows: HashMap<Ticker, TickerStats>) {
        self.display_cache.retain(|(ex, _), _| ex != &exchange);

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
                    is_favorited: self.favorited_tickers.contains(&(exchange, ticker)),
                };
                self.ticker_rows.push(new_row);
                (None, new_row)
            };

            self.display_cache.insert(
                (exchange, ticker),
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

    fn favorite_ticker(&mut self, exchange: Exchange, ticker: Ticker) {
        if let Some(row) = self
            .ticker_rows
            .iter_mut()
            .find(|row| row.exchange == exchange && row.ticker == ticker)
        {
            row.is_favorited = !row.is_favorited;

            if row.is_favorited {
                self.favorited_tickers.insert((exchange, ticker));
            } else {
                self.favorited_tickers.remove(&(exchange, ticker));
            }
        }
    }

    fn matches_exchange(ex: Exchange, tab: &TickerTab) -> bool {
        match tab {
            TickerTab::Bybit => matches!(
                ex,
                Exchange::BybitLinear | Exchange::BybitSpot | Exchange::BybitInverse
            ),
            TickerTab::Binance => matches!(
                ex,
                Exchange::BinanceLinear | Exchange::BinanceInverse | Exchange::BinanceSpot
            ),
            TickerTab::Hyperliquid => {
                matches!(ex, Exchange::HyperliquidLinear | Exchange::HyperliquidSpot)
            }
            _ => false,
        }
    }

    pub fn update_ticker_info(
        &mut self,
        exchange: Exchange,
        info: HashMap<Ticker, Option<TickerInfo>>,
    ) -> Action {
        if let Some(tickers) = self.tickers_info.get_mut(&exchange) {
            for (ticker, ticker_info) in info {
                if let Some(existing_ticker_info) = tickers.get_mut(&ticker) {
                    *existing_ticker_info = ticker_info;
                } else {
                    tickers.insert(ticker, ticker_info);
                }
            }
        } else {
            self.tickers_info.insert(exchange, info);
        }

        let task = Task::perform(fetch_ticker_prices(exchange), move |result| match result {
            Ok(ticker_rows) => Message::UpdateTickerStats(exchange, ticker_rows),

            Err(err) => Message::ErrorOccurred(InternalError::Fetch(err.to_string())),
        });

        Action::Fetch(task)
    }

    pub fn update_ticker_rows(&mut self, exchange: Exchange, stats: HashMap<Ticker, TickerStats>) {
        let tickers_set: HashSet<_> = self
            .tickers_info
            .get(&exchange)
            .map(|info| info.keys().copied().collect())
            .unwrap_or_default();

        let filtered_tickers_stats = stats
            .into_iter()
            .filter(|(ticker, _)| tickers_set.contains(ticker))
            .collect();

        self.update_table(exchange, filtered_tickers_stats);
    }

    pub fn update(&mut self, message: Message) -> Option<Action> {
        match message {
            Message::ChangeTickersTableTab(tab) => {
                self.selected_tab = tab;
            }
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
            Message::FavoriteTicker(exchange, ticker) => {
                self.favorite_ticker(exchange, ticker);
            }
            Message::Scrolled(viewport) => {
                self.scroll_offset = viewport.absolute_offset();
            }
            Message::SetMarketFilter(market) => {
                if self.selected_market == market {
                    self.selected_market = None;
                } else {
                    self.selected_market = market;
                }
            }
            Message::TickerSelected(ticker, exchange, content) => {
                let ticker_info = self
                    .tickers_info
                    .get(&exchange)
                    .and_then(|info| info.get(&ticker))
                    .copied()
                    .flatten();

                if let Some(ticker_info) = ticker_info {
                    return Some(Action::TickerSelected(ticker_info, content));
                } else {
                    log::warn!("Ticker info not found for {ticker:?} on {exchange:?}");
                }
            }
            Message::ToggleTable => {
                self.is_shown = !self.is_shown;

                if self.is_shown {
                    self.display_cache.clear();
                    for row in self.ticker_rows.iter_mut() {
                        row.previous_stats = None;
                        self.display_cache.insert(
                            (row.exchange, row.ticker),
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
                    let fetch_tasks = {
                        self.tickers_info
                            .keys()
                            .map(|exchange| {
                                let exchange = *exchange;
                                Task::perform(fetch_ticker_prices(exchange), move |result| {
                                    match result {
                                        Ok(ticker_rows) => {
                                            Message::UpdateTickerStats(exchange, ticker_rows)
                                        }
                                        Err(err) => Message::ErrorOccurred(InternalError::Fetch(
                                            err.to_string(),
                                        )),
                                    }
                                })
                            })
                            .collect::<Vec<Task<Message>>>()
                    };

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
        let show_sorting_button = button(icon_text(Icon::Sort, 14).align_x(Horizontal::Center))
            .on_press(Message::ShowSortingOptions);

        let expanded_card = self.expand_ticker_card;

        let search_bar_row = row![
            text_input("Search for a ticker...", &self.search_query)
                .style(|theme, status| style::validated_text_input(theme, status, true))
                .on_input(Message::UpdateSearchQuery)
                .align_x(Horizontal::Left)
                .padding(6),
            if self.show_sort_options {
                show_sorting_button
                    .style(move |theme, status| style::button::transparent(theme, status, true))
            } else {
                show_sorting_button
                    .style(move |theme, status| style::button::transparent(theme, status, false))
            }
        ]
        .align_y(Vertical::Center)
        .spacing(4);

        let sort_options_column = {
            let spot_market_button = button(text("Spot"))
                .on_press(Message::SetMarketFilter(Some(MarketKind::Spot)))
                .style(move |theme, status| style::button::transparent(theme, status, false));

            let linear_markets_btn = button(text("Linear"))
                .on_press(Message::SetMarketFilter(Some(MarketKind::LinearPerps)))
                .style(move |theme, status| style::button::transparent(theme, status, false));

            let inverse_markets_btn = button(text("Inverse"))
                .on_press(Message::SetMarketFilter(Some(MarketKind::InversePerps)))
                .style(move |theme, status| style::button::transparent(theme, status, false));

            let volume_sort_button =
                sort_button("Volume", SortOptions::VolumeAsc, self.selected_sort_option);
            let change_sort_button =
                sort_button("Change", SortOptions::ChangeAsc, self.selected_sort_option);

            column![
                row![
                    Space::new(Length::FillPortion(2), Length::Shrink),
                    volume_sort_button.style(move |theme, status| {
                        style::button::transparent(
                            theme,
                            status,
                            matches!(
                                self.selected_sort_option,
                                SortOptions::VolumeAsc | SortOptions::VolumeDesc
                            ),
                        )
                    }),
                    Space::new(Length::FillPortion(1), Length::Shrink),
                    change_sort_button.style(move |theme, status| {
                        style::button::transparent(
                            theme,
                            status,
                            matches!(
                                self.selected_sort_option,
                                SortOptions::ChangeAsc | SortOptions::ChangeDesc
                            ),
                        )
                    }),
                    Space::new(Length::FillPortion(2), Length::Shrink),
                ],
                row![
                    Space::new(Length::FillPortion(1), Length::Shrink),
                    spot_market_button.style(move |theme, status| {
                        style::button::transparent(
                            theme,
                            status,
                            matches!(self.selected_market, Some(MarketKind::Spot)),
                        )
                    }),
                    Space::new(Length::FillPortion(1), Length::Shrink),
                    linear_markets_btn.style(move |theme, status| {
                        style::button::transparent(
                            theme,
                            status,
                            matches!(self.selected_market, Some(MarketKind::LinearPerps)),
                        )
                    }),
                    Space::new(Length::FillPortion(1), Length::Shrink),
                    inverse_markets_btn.style(move |theme, status| {
                        style::button::transparent(
                            theme,
                            status,
                            matches!(self.selected_market, Some(MarketKind::InversePerps)),
                        )
                    }),
                    Space::new(Length::FillPortion(1), Length::Shrink),
                ],
                horizontal_rule(1.0).style(style::split_ruler),
            ]
            .spacing(4)
        };

        let exchange_filters_row = {
            let favorites_button = tab_button(
                text(char::from(Icon::StarFilled).to_string()).font(ICONS_FONT),
                &self.selected_tab,
                TickerTab::Favorites,
                Some("Show favorites"),
            );
            let all_button = tab_button(text("ALL"), &self.selected_tab, TickerTab::All, None);

            let bybit_button = tab_button(
                icon_text(style::exchange_icon(Exchange::BybitLinear), 12),
                &self.selected_tab,
                TickerTab::Bybit,
                Some("Bybit"),
            );
            let binance_button = tab_button(
                icon_text(style::exchange_icon(Exchange::BinanceLinear), 12),
                &self.selected_tab,
                TickerTab::Binance,
                Some("Binance"),
            );
            let hyperliquid_button = tab_button(
                icon_text(style::exchange_icon(Exchange::HyperliquidLinear), 12),
                &self.selected_tab,
                TickerTab::Hyperliquid,
                Some("Hyperliquid"),
            );

            row![
                favorites_button,
                horizontal_space(),
                all_button,
                horizontal_space(),
                bybit_button,
                horizontal_space(),
                binance_button,
                horizontal_space(),
                hyperliquid_button,
            ]
        };

        let mut content = column![search_bar_row,]
            .spacing(8)
            .padding(padding::right(8))
            .width(Length::Fill);

        if self.show_sort_options {
            content = content.push(sort_options_column);
        };

        content = content.push(exchange_filters_row);

        let filter_predicate = |row: &TickerRowData| -> bool {
            let search_match = if self.search_query.is_empty() {
                true
            } else {
                let (display_str, _market) = row.ticker.display_symbol_and_type();
                let (raw_str, _) = row.ticker.to_full_symbol_and_type();
                // Search both display symbol and raw symbol for maximum compatibility
                display_str.contains(&self.search_query) || raw_str.contains(&self.search_query)
            };

            let market_match = match self.selected_market {
                Some(market_type) => row.ticker.market_type() == market_type,
                None => true,
            };

            let tab_match = match self.selected_tab {
                TickerTab::All => true,
                TickerTab::Favorites => row.is_favorited,
                _ => Self::matches_exchange(row.exchange, &self.selected_tab),
            };

            search_match && market_match && tab_match
        };

        let scroll_y = self.scroll_offset.y.max(0.0);
        let filtered_count = self
            .ticker_rows
            .iter()
            .filter(|row| filter_predicate(row))
            .count();

        let first_visible_index = ((scroll_y / TICKER_CARD_HEIGHT).floor().max(0.0) as isize
            - OVERSCAN_BUFFER)
            .max(0) as usize;
        let last_visible_index = (((scroll_y + bounds.height) / TICKER_CARD_HEIGHT).ceil() as isize
            + OVERSCAN_BUFFER)
            .min(filtered_count as isize) as usize;

        let top_space = Space::new(
            Length::Shrink,
            Length::Fixed((first_visible_index as f32) * TICKER_CARD_HEIGHT),
        );
        let bottom_space = Space::new(
            Length::Shrink,
            Length::Fixed(((filtered_count - last_visible_index) as f32) * TICKER_CARD_HEIGHT),
        );

        let mut ticker_cards = column![top_space].spacing(4);

        for (filtered_index, row) in self
            .ticker_rows
            .iter()
            .filter(|row| filter_predicate(row))
            .enumerate()
        {
            if filtered_index >= first_visible_index
                && filtered_index < last_visible_index
                && let Some(display_data) = self.display_cache.get(&(row.exchange, row.ticker))
            {
                ticker_cards = ticker_cards.push(ticker_card_container(
                    row.exchange,
                    &row.ticker,
                    display_data,
                    expanded_card,
                    row.is_favorited,
                ));
            }
        }

        ticker_cards = ticker_cards.push(bottom_space);
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

fn ticker_card_container<'a>(
    exchange: Exchange,
    ticker: &'a Ticker,
    display_data: &'a TickerDisplayData,
    expanded_card: Option<(Ticker, Exchange)>,
    is_fav: bool,
) -> Element<'a, Message> {
    if let Some((selected_ticker, selected_exchange)) = &expanded_card {
        if ticker == selected_ticker && exchange == *selected_exchange {
            container(create_expanded_ticker_card(
                exchange,
                ticker,
                display_data,
                is_fav,
            ))
            .style(style::ticker_card)
            .into()
        } else {
            create_ticker_card(exchange, ticker, display_data)
        }
    } else {
        create_ticker_card(exchange, ticker, display_data)
    }
}

fn create_ticker_card<'a>(
    exchange: Exchange,
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

    let icon = icon_text(style::exchange_icon(exchange), 12);

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
        .on_press(Message::ExpandTickerCard(Some((*ticker, exchange)))),
    )
    .height(Length::Fixed(56.0))
    .into()
}

fn create_expanded_ticker_card<'a>(
    exchange: Exchange,
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
            .on_press(Message::FavoriteTicker(exchange, *ticker))
            .style(move |theme, status| { style::button::transparent(theme, status, false) }),
            horizontal_space(),
            button_with_tooltip(
                icon_text(Icon::Link, 11),
                Message::TickerSelected(*ticker, exchange, None),
                Some("Use this ticker on selected pane/group"),
                iced::widget::tooltip::Position::Top,
                move |theme, status| style::button::transparent(theme, status, false)
            ),
        ]
        .spacing(2),
        row![
            match exchange {
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
            init_content_button("Heatmap Chart", "heatmap", *ticker, exchange, 180.0),
            init_content_button("Footprint Chart", "footprint", *ticker, exchange, 180.0),
            init_content_button("Candlestick Chart", "candlestick", *ticker, exchange, 180.0),
            init_content_button("Time&Sales", "time&sales", *ticker, exchange, 160.0),
        ]
        .width(Length::Fill)
        .spacing(2)
    ]
    .padding(padding::top(8).right(16).left(16).bottom(16))
    .spacing(12)
    .into()
}

fn tab_button<'a>(
    text: Text<'a, Theme, Renderer>,
    current_tab: &TickerTab,
    target_tab: TickerTab,
    tooltip_text: Option<&'a str>,
) -> Element<'a, Message, Theme, Renderer> {
    let mut btn =
        button(text).style(move |theme, status| style::button::transparent(theme, status, false));
    if *current_tab != target_tab {
        btn = btn.on_press(Message::ChangeTickersTableTab(target_tab));
    }

    tooltip(
        btn,
        if let Some(text) = tooltip_text {
            Some(text)
        } else {
            None
        },
        iced::widget::tooltip::Position::Top,
    )
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
    exchange: Exchange,
    width: f32,
) -> Button<'a, Message, Theme, Renderer> {
    button(text(label).align_x(Horizontal::Center))
        .on_press(Message::TickerSelected(
            ticker,
            exchange,
            Some(content.to_string()),
        ))
        .width(Length::Fixed(width))
}

#[derive(Clone, Debug, PartialEq)]
enum PriceChangeDirection {
    Increased,
    Decreased,
    Unchanged,
}

fn compute_display_data(
    ticker: &Ticker,
    stats: &TickerStats,
    previous_price: Option<f32>,
) -> TickerDisplayData {
    let (ticker_str, market) = ticker.display_symbol_and_type();
    let display_ticker = if ticker_str.len() >= 11 {
        ticker_str[..9].to_string() + "..."
    } else {
        ticker_str + {
            match market {
                MarketKind::Spot => "",
                MarketKind::LinearPerps | MarketKind::InversePerps => "P",
            }
        }
    };

    let current_price = stats.mark_price;
    let (price_unchanged_part, price_changed_part, price_change_direction) =
        if let Some(prev_price) = previous_price {
            split_price_changes(prev_price, current_price)
        } else {
            (
                current_price.to_string(),
                String::new(),
                PriceChangeDirection::Unchanged,
            )
        };

    TickerDisplayData {
        display_ticker,
        daily_change_pct: data::util::pct_change(stats.daily_price_chg),
        volume_display: data::util::currency_abbr(stats.daily_volume),
        mark_price_display: stats.mark_price.to_string(),
        price_unchanged_part,
        price_changed_part,
        price_change_direction,
        card_color_alpha: { (stats.daily_price_chg / 8.0).clamp(-1.0, 1.0) },
    }
}

fn split_price_changes(
    previous_price: f32,
    current_price: f32,
) -> (String, String, PriceChangeDirection) {
    if previous_price == current_price {
        return (
            current_price.to_string(),
            String::new(),
            PriceChangeDirection::Unchanged,
        );
    }

    let prev_str = previous_price.to_string();
    let curr_str = current_price.to_string();

    let direction = if current_price > previous_price {
        PriceChangeDirection::Increased
    } else {
        PriceChangeDirection::Decreased
    };

    let mut split_index = 0;
    let prev_chars: Vec<char> = prev_str.chars().collect();
    let curr_chars: Vec<char> = curr_str.chars().collect();

    for (i, &curr_char) in curr_chars.iter().enumerate() {
        if i >= prev_chars.len() || prev_chars[i] != curr_char {
            split_index = i;
            break;
        }
    }

    if split_index == 0 && curr_chars.len() != prev_chars.len() {
        split_index = prev_chars.len().min(curr_chars.len());
    }

    let unchanged_part = curr_str[..split_index].to_string();
    let changed_part = curr_str[split_index..].to_string();

    (unchanged_part, changed_part, direction)
}
