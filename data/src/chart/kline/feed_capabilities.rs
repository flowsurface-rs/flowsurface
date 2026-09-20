use exchange::adapter::Exchange;
use exchange::{TickerInfo, Timeframe};

/// Provider capability predicates shared by fetch planning and indicators.
pub fn supports_open_interest(ticker_info: TickerInfo, timeframe: Timeframe) -> bool {
    supports_open_interest_exchange(ticker_info.exchange())
        && supports_open_interest_timeframe(timeframe)
}

pub fn supports_open_interest_exchange(exchange: Exchange) -> bool {
    matches!(
        exchange,
        Exchange::BinanceLinear
            | Exchange::BinanceInverse
            | Exchange::BybitLinear
            | Exchange::BybitInverse
            | Exchange::OkexLinear
            | Exchange::OkexInverse
    )
}

pub fn supports_open_interest_timeframe(timeframe: Timeframe) -> bool {
    timeframe >= Timeframe::M5 && timeframe <= Timeframe::H4 && timeframe != Timeframe::H2
}
