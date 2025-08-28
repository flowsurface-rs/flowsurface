use exchange::{
    Ticker, TickerStats,
    adapter::{Exchange, ExchangeInclusive, MarketKind},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    pub favorited_tickers: Vec<Ticker>,
    pub show_favorites: bool,
    pub selected_sort_option: SortOptions,
    pub selected_exchanges: Vec<ExchangeInclusive>,
    pub selected_markets: Vec<MarketKind>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            favorited_tickers: vec![],
            show_favorites: false,
            selected_sort_option: SortOptions::VolumeDesc,
            selected_exchanges: ExchangeInclusive::ALL.to_vec(),
            selected_markets: MarketKind::ALL.into_iter().collect(),
        }
    }
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub enum SortOptions {
    #[default]
    VolumeAsc,
    VolumeDesc,
    ChangeAsc,
    ChangeDesc,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PriceChangeDirection {
    Increased,
    Decreased,
    Unchanged,
}

#[derive(Clone, Copy)]
pub struct TickerRowData {
    pub exchange: Exchange,
    pub ticker: Ticker,
    pub stats: TickerStats,
    pub previous_stats: Option<TickerStats>,
    pub is_favorited: bool,
}

#[derive(Clone)]
pub struct TickerDisplayData {
    pub display_ticker: String,
    pub daily_change_pct: String,
    pub volume_display: String,
    pub mark_price_display: String,
    pub price_unchanged_part: String,
    pub price_changed_part: String,
    pub price_change_direction: PriceChangeDirection,
    pub card_color_alpha: f32,
}

pub fn compute_display_data(
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
        daily_change_pct: super::util::pct_change(stats.daily_price_chg),
        volume_display: super::util::currency_abbr(stats.daily_volume),
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
