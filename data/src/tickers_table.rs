use exchange::{
    Ticker, TickerStats,
    adapter::{Exchange, MarketKind, Venue},
    unit::{MinTicksize, price::Price},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    pub favorited_tickers: Vec<Ticker>,
    pub show_favorites: bool,
    pub selected_sort_option: SortOptions,
    pub selected_exchanges: Vec<Venue>,
    pub selected_markets: Vec<MarketKind>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            favorited_tickers: vec![],
            show_favorites: false,
            selected_sort_option: SortOptions::VolumeDesc,
            selected_exchanges: Venue::ALL.to_vec(),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PriceChange {
    Increased,
    Decreased,
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
    pub mark_price_display: Option<String>,
    pub price_unchanged_part: Option<String>,
    pub price_changed_part: Option<String>,
    pub price_change: Option<PriceChange>,
    pub card_color_alpha: f32,
}

pub fn compute_display_data(
    ticker: &Ticker,
    stats: &TickerStats,
    previous_price: Option<Price>,
    precision: Option<MinTicksize>,
) -> TickerDisplayData {
    let (display_ticker, _market) = ticker.display_symbol_and_type();

    let current_price = stats.mark_price;
    let current_price_display = price_to_display_string(current_price, precision);
    let price_parts = previous_price
        .and_then(|prev_price| split_price_changes(prev_price, current_price, precision))
        .or_else(|| {
            current_price_display
                .clone()
                .map(|price| (price, String::new(), None))
        });

    let (price_unchanged_part, price_changed_part, price_change) = price_parts
        .map(|(unchanged, changed, change)| (Some(unchanged), Some(changed), change))
        .unwrap_or((None, None, None));

    TickerDisplayData {
        display_ticker,
        daily_change_pct: super::util::pct_change(stats.daily_price_chg),
        volume_display: super::util::currency_abbr(stats.daily_volume.to_f32_lossy()),
        mark_price_display: current_price_display,
        price_unchanged_part,
        price_changed_part,
        price_change,
        card_color_alpha: { (stats.daily_price_chg / 8.0).clamp(-1.0, 1.0) },
    }
}

fn split_price_changes(
    previous_price: Price,
    current_price: Price,
    precision: Option<MinTicksize>,
) -> Option<(String, String, Option<PriceChange>)> {
    let curr_str = price_to_display_string(current_price, precision)?;

    if previous_price == current_price {
        return Some((curr_str, String::new(), None));
    }

    let prev_str = price_to_display_string(previous_price, precision)?;

    if prev_str == curr_str {
        return Some((curr_str, String::new(), None));
    }

    let direction = Some(if current_price > previous_price {
        PriceChange::Increased
    } else {
        PriceChange::Decreased
    });

    let split_index = prev_str
        .bytes()
        .zip(curr_str.bytes())
        .position(|(prev, curr)| prev != curr)
        .unwrap_or_else(|| prev_str.len().min(curr_str.len()));

    let unchanged_part = curr_str[..split_index].to_string();
    let changed_part = curr_str[split_index..].to_string();

    Some((unchanged_part, changed_part, direction))
}

fn price_to_display_string(price: Price, precision: Option<MinTicksize>) -> Option<String> {
    precision.map(|precision| price.to_string(precision))
}
