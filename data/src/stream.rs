use exchange::{PushFrequency, Ticker, TickerInfo, Timeframe, adapter::StreamKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub enum PersistStreamKind {
    Kline {
        ticker: Ticker,
        timeframe: Timeframe,
    },
    Depth(PersistDepth),
    Trades {
        ticker: Ticker,
    },
    /// Deprecated combined stream, kept for backward compatibility.
    /// Will be converted to separate Depth and Trades on load.
    DepthAndTrades(PersistDepth),
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct PersistDepth {
    pub ticker: Ticker,
    #[serde(default = "default_depth_aggr")]
    pub depth_aggr: exchange::adapter::StreamTicksize,
    #[serde(default = "default_push_freq")]
    pub push_freq: PushFrequency,
}

impl From<StreamKind> for PersistStreamKind {
    fn from(stream: StreamKind) -> Self {
        match stream {
            StreamKind::Kline {
                ticker_info,
                timeframe,
            } => PersistStreamKind::Kline {
                ticker: ticker_info.ticker,
                timeframe,
            },
            StreamKind::Depth {
                ticker_info,
                depth_aggr,
                push_freq,
            } => PersistStreamKind::Depth(PersistDepth {
                ticker: ticker_info.ticker,
                depth_aggr,
                push_freq,
            }),
            StreamKind::Trades { ticker_info } => PersistStreamKind::Trades {
                ticker: ticker_info.ticker,
            },
        }
    }
}

impl PersistStreamKind {
    /// Try to convert into runtime StreamKind list. `resolver` should return Some(TickerInfo) for a ticker,
    /// otherwise the conversion fails (so caller can trigger a refresh / fetch).
    pub fn into_stream_kinds<F>(self, mut resolver: F) -> Result<Vec<StreamKind>, String>
    where
        F: FnMut(&Ticker) -> Option<TickerInfo>,
    {
        match self {
            PersistStreamKind::Kline { ticker, timeframe } => resolver(&ticker)
                .map(|ti| {
                    vec![StreamKind::Kline {
                        ticker_info: ti,
                        timeframe,
                    }]
                })
                .ok_or_else(|| format!("TickerInfo not found for {}", ticker)),
            PersistStreamKind::Depth(d) => resolver(&d.ticker)
                .map(|ti| {
                    vec![StreamKind::Depth {
                        ticker_info: ti,
                        depth_aggr: d.depth_aggr,
                        push_freq: d.push_freq,
                    }]
                })
                .ok_or_else(|| format!("TickerInfo not found for {}", d.ticker)),
            PersistStreamKind::Trades { ticker } => resolver(&ticker)
                .map(|ti| vec![StreamKind::Trades { ticker_info: ti }])
                .ok_or_else(|| format!("TickerInfo not found for {}", ticker)),
            PersistStreamKind::DepthAndTrades(d) => resolver(&d.ticker)
                .map(|ti| {
                    vec![
                        StreamKind::Depth {
                            ticker_info: ti,
                            depth_aggr: d.depth_aggr,
                            push_freq: d.push_freq,
                        },
                        StreamKind::Trades { ticker_info: ti },
                    ]
                })
                .ok_or_else(|| format!("TickerInfo not found for {}", d.ticker)),
        }
    }
}

fn default_depth_aggr() -> exchange::adapter::StreamTicksize {
    exchange::adapter::StreamTicksize::Client
}

fn default_push_freq() -> PushFrequency {
    PushFrequency::ServerDefault
}
