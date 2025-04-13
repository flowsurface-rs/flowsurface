use std::collections::HashMap;

use crate::style::{self, ICONS_FONT, Icon, get_icon_text};
use data::InternalError;
use exchange::{
    Ticker, TickerInfo, TickerStats,
    adapter::{Exchange, MarketType, fetch_ticker_prices},
};
use iced::{
    Alignment, Element, Length, Renderer, Size, Subscription, Task, Theme,
    alignment::{self, Horizontal, Vertical},
    padding,
    widget::{
        Button, Container, Space, Text, button, column, container, horizontal_rule,
        horizontal_space, row,
        scrollable::{self, AbsoluteOffset},
        text, text_input,
    },
};

const TICKER_CARD_HEIGHT: f32 = 64.0;
const SEARCH_BAR_HEIGHT: f32 = 120.0;

pub enum Action {
    TickerSelected(TickerInfo, Exchange, String),
    ErrorOccurred(data::InternalError),
    Fetch(Task<Message>),
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TickerTab {
    All,
    Bybit,
    Binance,
    Favorites,
}

#[derive(Clone)]
struct TickerDisplayData {
    display_ticker: String,
    price_change_display: String,
    volume_display: String,
    mark_price_display: String,
    card_color_alpha: f32,
}

#[derive(Debug, Clone, PartialEq)]
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
    TickerSelected(Ticker, Exchange, String),
    ExpandTickerCard(Option<(Ticker, Exchange)>),
    FavoriteTicker(Exchange, Ticker),
    Scrolled(scrollable::Viewport),
    SetMarketFilter(Option<MarketType>),
    ToggleTable,
    FetchForTickerStats(Option<Exchange>),
    UpdateTickersInfo(Exchange, HashMap<Ticker, Option<TickerInfo>>),
    UpdateTickerStats(Exchange, HashMap<Ticker, TickerStats>),
    ErrorOccurred(data::InternalError),
}

pub struct TickersTable {
    ticker_stats: HashMap<Exchange, Vec<(Ticker, TickerStats)>>,
    combined_tickers: Vec<(Exchange, Ticker, TickerStats, bool)>,
    favorited_tickers: Vec<(Exchange, Ticker)>,
    display_cache: HashMap<(Exchange, Ticker), TickerDisplayData>,
    selected_tab: TickerTab,
    search_query: String,
    show_sort_options: bool,
    selected_sort_option: SortOptions,
    selected_market: Option<MarketType>,
    expand_ticker_card: Option<(Ticker, Exchange)>,
    scroll_offset: AbsoluteOffset,
    is_show: bool,
    tickers_info: HashMap<Exchange, HashMap<Ticker, Option<TickerInfo>>>,
}

impl TickersTable {
    pub fn new(favorited_tickers: Vec<(Exchange, Ticker)>) -> Self {
        Self {
            ticker_stats: HashMap::new(),
            combined_tickers: Vec::new(),
            display_cache: HashMap::new(),
            favorited_tickers,
            selected_tab: TickerTab::All,
            search_query: String::new(),
            show_sort_options: false,
            selected_sort_option: SortOptions::VolumeDesc,
            expand_ticker_card: None,
            scroll_offset: AbsoluteOffset::default(),
            selected_market: None,
            is_show: false,
            tickers_info: HashMap::new(),
        }
    }

    pub fn update_table(&mut self, exchange: Exchange, ticker_stats: HashMap<Ticker, TickerStats>) {
        self.display_cache.retain(|(ex, _), _| ex != &exchange);

        let tickers_vec: Vec<_> = ticker_stats
            .into_iter()
            .map(|(ticker, stats)| {
                self.display_cache.insert(
                    (exchange, ticker),
                    Self::compute_display_data(&ticker, &stats),
                );
                (ticker, stats)
            })
            .collect();

        self.ticker_stats.insert(exchange, tickers_vec);
        self.update_combined_tickers();
    }

    fn update_combined_tickers(&mut self) {
        self.combined_tickers.clear();

        self.ticker_stats.iter().for_each(|(exchange, tickers)| {
            for (ticker, stats) in tickers {
                let is_fav = self
                    .favorited_tickers
                    .iter()
                    .any(|(ex, tick)| ex == exchange && tick == ticker);
                self.combined_tickers
                    .push((*exchange, *ticker, *stats, is_fav));
            }
        });

        match self.selected_sort_option {
            SortOptions::VolumeDesc => {
                self.combined_tickers
                    .sort_by(|a: &(Exchange, Ticker, TickerStats, bool), b| {
                        b.2.daily_volume
                            .partial_cmp(&a.2.daily_volume)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
            }
            SortOptions::VolumeAsc => {
                self.combined_tickers
                    .sort_by(|a: &(Exchange, Ticker, TickerStats, bool), b| {
                        a.2.daily_volume
                            .partial_cmp(&b.2.daily_volume)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
            }
            SortOptions::ChangeDesc => {
                self.combined_tickers
                    .sort_by(|a: &(Exchange, Ticker, TickerStats, bool), b| {
                        b.2.daily_price_chg
                            .partial_cmp(&a.2.daily_price_chg)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
            }
            SortOptions::ChangeAsc => {
                self.combined_tickers
                    .sort_by(|a: &(Exchange, Ticker, TickerStats, bool), b| {
                        a.2.daily_price_chg
                            .partial_cmp(&b.2.daily_price_chg)
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

        self.update_combined_tickers();
    }

    fn favorite_ticker(&mut self, exchange: Exchange, ticker: Ticker) {
        for (ex, tick, _, is_fav) in &mut self.combined_tickers {
            if ex == &exchange && tick == &ticker {
                *is_fav = !*is_fav;
            }
        }

        self.favorited_tickers = self
            .combined_tickers
            .iter()
            .filter(|(_, _, _, is_fav)| *is_fav)
            .map(|(exchange, ticker, _, _)| (*exchange, *ticker))
            .collect();
    }

    pub fn get_favorited_tickers(&self) -> Vec<(Exchange, Ticker)> {
        self.combined_tickers
            .iter()
            .filter(|(_, _, _, is_fav)| *is_fav)
            .map(|(exchange, ticker, _, _)| (*exchange, *ticker))
            .collect()
    }

    fn compute_display_data(ticker: &Ticker, stats: &TickerStats) -> TickerDisplayData {
        let (ticker_str, market) = ticker.display_symbol_and_type();
        let display_ticker = if ticker_str.len() >= 11 {
            ticker_str[..9].to_string() + "..."
        } else {
            ticker_str + {
                match market {
                    MarketType::Spot => "",
                    MarketType::LinearPerps | MarketType::InversePerps => "P",
                }
            }
        };

        TickerDisplayData {
            display_ticker,
            price_change_display: convert_to_pct_change(stats.daily_price_chg).to_string(),
            volume_display: convert_to_currency_abbr(stats.daily_volume).to_string(),
            mark_price_display: stats.mark_price.to_string(),
            card_color_alpha: { (stats.daily_price_chg / 8.0).clamp(-1.0, 1.0) },
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
            _ => false,
        }
    }

    fn create_ticker_container<'a>(
        &'a self,
        is_visible: bool,
        exchange: Exchange,
        ticker: &'a Ticker,
        is_fav: bool,
    ) -> Container<'a, Message> {
        if !is_visible {
            return container(column![].width(Length::Fill).height(Length::Fixed(60.0)));
        }

        let display_data = &self.display_cache[&(exchange, *ticker)];

        container(
            if let Some((selected_ticker, selected_exchange)) = &self.expand_ticker_card {
                if ticker == selected_ticker && exchange == *selected_exchange {
                    create_expanded_ticker_card(exchange, ticker, display_data, is_fav)
                } else {
                    create_ticker_card(exchange, ticker, display_data)
                }
            } else {
                create_ticker_card(exchange, ticker, display_data)
            },
        )
        .style(style::ticker_card)
    }

    fn is_container_visible(&self, index: usize, bounds: Size) -> bool {
        let item_top = SEARCH_BAR_HEIGHT + (index as f32 * TICKER_CARD_HEIGHT);
        let item_bottom = item_top + TICKER_CARD_HEIGHT;

        (item_bottom >= (self.scroll_offset.y - (2.0 * TICKER_CARD_HEIGHT)))
            && (item_top <= (self.scroll_offset.y + bounds.height + (2.0 * TICKER_CARD_HEIGHT)))
    }

    pub fn is_open(&self) -> bool {
        self.is_show
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
            Ok(ticker_stats) => Message::UpdateTickerStats(exchange, ticker_stats),

            Err(err) => Message::ErrorOccurred(InternalError::Fetch(err.to_string())),
        });

        Action::Fetch(task)
    }

    pub fn update_ticker_stats(&mut self, exchange: Exchange, stats: HashMap<Ticker, TickerStats>) {
        let tickers = self
            .tickers_info
            .get(&exchange)
            .map(|info| info.keys().copied().collect::<Vec<_>>())
            .unwrap_or_default();

        let filtered_tickers_stats = stats
            .into_iter()
            .filter(|(ticker, _)| tickers.iter().any(|t| t == ticker))
            .collect::<HashMap<Ticker, TickerStats>>();

        self.update_table(exchange, filtered_tickers_stats);
    }

    pub fn update(&mut self, message: Message) -> Action {
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
            Message::TickerSelected(ticker, exchange, chart_type) => {
                let ticker_info = self
                    .tickers_info
                    .get(&exchange)
                    .and_then(|info| info.get(&ticker))
                    .copied()
                    .flatten();

                if let Some(ticker_info) = ticker_info {
                    return Action::TickerSelected(ticker_info, exchange, chart_type);
                } else {
                    log::warn!("Ticker info not found for {ticker:?} on {exchange:?}");
                }
            }
            Message::ToggleTable => {
                self.is_show = !self.is_show;
            }
            Message::FetchForTickerStats(exchange) => {
                let task = if let Some(exchange) = exchange {
                    Task::perform(fetch_ticker_prices(exchange), move |result| match result {
                        Ok(ticker_stats) => Message::UpdateTickerStats(exchange, ticker_stats),
                        Err(err) => Message::ErrorOccurred(InternalError::Fetch(err.to_string())),
                    })
                } else {
                    let fetch_tasks = {
                        Exchange::ALL
                            .iter()
                            .map(|exchange| {
                                Task::perform(fetch_ticker_prices(*exchange), move |result| {
                                    match result {
                                        Ok(ticker_stats) => {
                                            Message::UpdateTickerStats(*exchange, ticker_stats)
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

                return Action::Fetch(task);
            }
            Message::UpdateTickerStats(exchange, stats) => {
                self.update_ticker_stats(exchange, stats);
            }
            Message::UpdateTickersInfo(exchange, info) => {
                self.update_ticker_info(exchange, info);

                let task =
                    Task::perform(fetch_ticker_prices(exchange), move |result| match result {
                        Ok(ticker_stats) => Message::UpdateTickerStats(exchange, ticker_stats),

                        Err(err) => Message::ErrorOccurred(InternalError::Fetch(err.to_string())),
                    });

                return Action::Fetch(task);
            }
            Message::ErrorOccurred(err) => {
                log::error!("Error occurred: {err}");
                return Action::ErrorOccurred(err);
            }
        }

        Action::None
    }

    pub fn view(&self, bounds: Size) -> Element<'_, Message> {
        let show_sorting_button = button(get_icon_text(Icon::Sort, 14).align_x(Horizontal::Center))
            .on_press(Message::ShowSortingOptions);

        let search_bar_row = row![
            text_input("Search for a ticker...", &self.search_query)
                .style(style::search_input)
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
                .on_press(Message::SetMarketFilter(Some(MarketType::Spot)))
                .style(move |theme, status| style::button::transparent(theme, status, false));

            let linear_markets_btn = button(text("Linear"))
                .on_press(Message::SetMarketFilter(Some(MarketType::LinearPerps)))
                .style(move |theme, status| style::button::transparent(theme, status, false));

            let inverse_markets_btn = button(text("Inverse"))
                .on_press(Message::SetMarketFilter(Some(MarketType::InversePerps)))
                .style(move |theme, status| style::button::transparent(theme, status, false));

            let volume_sort_button = button(
                row![
                    text("Volume"),
                    get_icon_text(
                        if self.selected_sort_option == SortOptions::VolumeDesc {
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
            .on_press(Message::ChangeSortOption(SortOptions::VolumeAsc));

            let change_sort_button = button(
                row![
                    text("Change"),
                    get_icon_text(
                        if self.selected_sort_option == SortOptions::ChangeDesc {
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
            .on_press(Message::ChangeSortOption(SortOptions::ChangeAsc));

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
                            matches!(self.selected_market, Some(MarketType::Spot)),
                        )
                    }),
                    Space::new(Length::FillPortion(1), Length::Shrink),
                    linear_markets_btn.style(move |theme, status| {
                        style::button::transparent(
                            theme,
                            status,
                            matches!(self.selected_market, Some(MarketType::LinearPerps)),
                        )
                    }),
                    Space::new(Length::FillPortion(1), Length::Shrink),
                    inverse_markets_btn.style(move |theme, status| {
                        style::button::transparent(
                            theme,
                            status,
                            matches!(self.selected_market, Some(MarketType::InversePerps)),
                        )
                    }),
                    Space::new(Length::FillPortion(1), Length::Shrink),
                ],
                horizontal_rule(1.0).style(style::split_ruler),
            ]
            .spacing(4)
        };

        let exchange_filters_row = {
            let all_button = create_tab_button(text("ALL"), &self.selected_tab, TickerTab::All);
            let bybit_button =
                create_tab_button(text("Bybit"), &self.selected_tab, TickerTab::Bybit);
            let binance_button =
                create_tab_button(text("Binance"), &self.selected_tab, TickerTab::Binance);
            let favorites_button = create_tab_button(
                text(char::from(Icon::StarFilled).to_string()).font(ICONS_FONT),
                &self.selected_tab,
                TickerTab::Favorites,
            );

            row![
                favorites_button,
                horizontal_space(),
                all_button,
                horizontal_space(),
                bybit_button,
                horizontal_space(),
                binance_button,
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

        let mut ticker_cards = column![].spacing(4);

        match self.selected_tab {
            TickerTab::All => {
                ticker_cards =
                    self.combined_tickers
                        .iter()
                        .filter(|(_, ticker, _, _)| {
                            let (ticker, market) = ticker.to_full_symbol_and_type();
                            ticker.contains(&self.search_query)
                                && match self.selected_market {
                                    Some(market_type) => market == market_type,
                                    None => true,
                                }
                        })
                        .enumerate()
                        .fold(
                            ticker_cards,
                            |ticker_cards, (index, (exchange, ticker, _, is_fav))| {
                                let is_visible = self.is_container_visible(index, bounds);
                                ticker_cards.push(self.create_ticker_container(
                                    is_visible, *exchange, ticker, *is_fav,
                                ))
                            },
                        );
            }
            TickerTab::Favorites => {
                ticker_cards =
                    self.combined_tickers
                        .iter()
                        .filter(|(_, ticker, _, is_fav)| {
                            let (ticker, market) = ticker.to_full_symbol_and_type();
                            *is_fav
                                && ticker.contains(&self.search_query)
                                && match self.selected_market {
                                    Some(market_type) => market == market_type,
                                    None => true,
                                }
                        })
                        .enumerate()
                        .fold(
                            ticker_cards,
                            |ticker_cards, (index, (exchange, ticker, _, is_fav))| {
                                let is_visible = self.is_container_visible(index, bounds);
                                ticker_cards.push(self.create_ticker_container(
                                    is_visible, *exchange, ticker, *is_fav,
                                ))
                            },
                        );
            }
            _ => {
                ticker_cards = self
                    .combined_tickers
                    .iter()
                    .filter(|(ex, ticker, _, _)| {
                        let (ticker, market) = ticker.to_full_symbol_and_type();
                        Self::matches_exchange(*ex, &self.selected_tab)
                            && ticker.contains(&self.search_query)
                            && match self.selected_market {
                                Some(market_type) => market == market_type,
                                None => true,
                            }
                    })
                    .enumerate()
                    .fold(
                        ticker_cards,
                        |ticker_cards, (index, (ex, ticker, _, is_fav))| {
                            let is_visible = self.is_container_visible(index, bounds);
                            ticker_cards.push(
                                self.create_ticker_container(is_visible, *ex, ticker, *is_fav),
                            )
                        },
                    );
            }
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
        iced::time::every(std::time::Duration::from_secs(if self.is_open() {
            25
        } else {
            300
        }))
        .map(|_| Message::FetchForTickerStats(None))
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

    container(
        button(
            row![
                color_column,
                column![
                    row![
                        row![
                            match exchange {
                                Exchange::BybitInverse
                                | Exchange::BybitLinear
                                | Exchange::BybitSpot => get_icon_text(Icon::BybitLogo, 12),
                                Exchange::BinanceInverse
                                | Exchange::BinanceLinear
                                | Exchange::BinanceSpot => get_icon_text(Icon::BinanceLogo, 12),
                            },
                            text(&display_data.display_ticker),
                        ]
                        .spacing(2)
                        .align_y(alignment::Vertical::Center),
                        Space::new(Length::Fill, Length::Shrink),
                        text(&display_data.price_change_display),
                    ]
                    .spacing(4)
                    .align_y(alignment::Vertical::Center),
                    row![
                        text(&display_data.mark_price_display),
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
            button(get_icon_text(Icon::Return, 11))
                .on_press(Message::ExpandTickerCard(None))
                .style(move |theme, status| style::button::transparent(theme, status, false)),
            button(if is_fav {
                get_icon_text(Icon::StarFilled, 11)
            } else {
                get_icon_text(Icon::Star, 11)
            })
            .on_press(Message::FavoriteTicker(exchange, *ticker))
            .style(move |theme, status| style::button::transparent(theme, status, false)),
        ]
        .spacing(2),
        row![
            match exchange {
                Exchange::BybitInverse | Exchange::BybitLinear | Exchange::BybitSpot =>
                    get_icon_text(Icon::BybitLogo, 12),
                Exchange::BinanceInverse | Exchange::BinanceLinear | Exchange::BinanceSpot =>
                    get_icon_text(Icon::BinanceLogo, 12),
            },
            text(
                ticker_str
                    + " "
                    + &market.to_string()
                    + match market {
                        MarketType::Spot => "",
                        MarketType::LinearPerps | MarketType::InversePerps => " Perp",
                    }
            ),
        ]
        .spacing(2),
        column![
            row![
                text("Last Updated Price: ").size(11),
                Space::new(Length::Fill, Length::Shrink),
                text(&display_data.mark_price_display)
            ],
            row![
                text("Daily Change: ").size(11),
                Space::new(Length::Fill, Length::Shrink),
                text(&display_data.price_change_display),
            ],
            row![
                text("Daily Volume: ").size(11),
                Space::new(Length::Fill, Length::Shrink),
                text(&display_data.volume_display),
            ],
        ]
        .spacing(4),
        column![
            button(text("Heatmap Chart").align_x(Horizontal::Center))
                .on_press(Message::TickerSelected(
                    *ticker,
                    exchange,
                    "heatmap".to_string()
                ))
                .width(Length::Fixed(180.0)),
            button(text("Footprint Chart").align_x(Horizontal::Center))
                .on_press(Message::TickerSelected(
                    *ticker,
                    exchange,
                    "footprint".to_string()
                ))
                .width(Length::Fixed(180.0)),
            button(text("Candlestick Chart").align_x(Horizontal::Center))
                .on_press(Message::TickerSelected(
                    *ticker,
                    exchange,
                    "candlestick".to_string()
                ))
                .width(Length::Fixed(180.0)),
            button(text("Time&Sales").align_x(Horizontal::Center))
                .on_press(Message::TickerSelected(
                    *ticker,
                    exchange,
                    "time&sales".to_string()
                ))
                .width(Length::Fixed(160.0)),
        ]
        .width(Length::Fill)
        .spacing(2),
    ]
    .padding(padding::top(8).right(16).left(16).bottom(16))
    .spacing(12)
    .into()
}

fn create_tab_button<'a>(
    text: Text<'a, Theme, Renderer>,
    current_tab: &TickerTab,
    target_tab: TickerTab,
) -> Button<'a, Message, Theme, Renderer> {
    let mut btn =
        button(text).style(move |theme, status| style::button::transparent(theme, status, false));
    if *current_tab != target_tab {
        btn = btn.on_press(Message::ChangeTickersTableTab(target_tab));
    }
    btn
}

fn convert_to_currency_abbr(price: f32) -> String {
    if price > 1_000_000_000.0 {
        format!("${:.2}b", price / 1_000_000_000.0)
    } else if price > 1_000_000.0 {
        format!("${:.1}m", price / 1_000_000.0)
    } else if price > 1000.0 {
        format!("${:.2}k", price / 1000.0)
    } else {
        format!("${price:.2}")
    }
}

fn convert_to_pct_change(change: f32) -> String {
    if change > 0.0 {
        format!("+{change:.2}%")
    } else {
        format!("{change:.2}%")
    }
}
