use std::time::{Duration, Instant};

use data::stream::PersistStreamKind;
use exchange::adapter::StreamKind;

/// Persisted stream resolution to avoid loop retries
const RESOLVE_RETRY_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedStream {
    /// Streams that are persisted but needs to be resolved for use
    Waiting {
        streams: Vec<PersistStreamKind>,
        last_attempt: Option<Instant>,
    },
    /// Streams that are active and ready to use, but can't persist
    Ready(Vec<StreamKind>),
    Blocked {
        streams: Vec<PersistStreamKind>,
        reason: String,
        last_attempt: Option<Instant>,
    },
}

impl ResolvedStream {
    pub fn waiting(streams: Vec<PersistStreamKind>) -> Self {
        ResolvedStream::Waiting {
            streams,
            last_attempt: None,
        }
    }

    /// Returns streams to resolve only if the retry interval has elapsed.
    pub fn due_streams_to_resolve(&mut self, now: Instant) -> Option<Vec<PersistStreamKind>> {
        let (streams, last_attempt) = match self {
            ResolvedStream::Waiting {
                streams,
                last_attempt,
            } => (streams, last_attempt),
            ResolvedStream::Blocked {
                streams,
                last_attempt,
                ..
            } => (streams, last_attempt),
            _ => return None,
        };

        if streams.is_empty() {
            return None;
        }

        let should_retry = last_attempt
            .map(|t| now.duration_since(t) >= RESOLVE_RETRY_INTERVAL)
            .unwrap_or(true);

        if !should_retry {
            return None;
        }

        *last_attempt = Some(now);
        Some(streams.clone())
    }

    pub fn matches_stream(&self, stream: &StreamKind) -> bool {
        match self {
            ResolvedStream::Ready(existing) => existing.iter().any(|s| s == stream),
            _ => false,
        }
    }

    pub fn ready_iter_mut(&mut self) -> Option<impl Iterator<Item = &mut StreamKind>> {
        match self {
            ResolvedStream::Ready(streams) => Some(streams.iter_mut()),
            _ => None,
        }
    }

    pub fn ready_iter(&self) -> Option<impl Iterator<Item = &StreamKind>> {
        match self {
            ResolvedStream::Ready(streams) => Some(streams.iter()),
            _ => None,
        }
    }

    pub fn set_trade_stream(&mut self, ticker_info: exchange::TickerInfo, enabled: bool) -> bool {
        let ResolvedStream::Ready(streams) = self else {
            return false;
        };

        let mut found = false;
        let original_len = streams.len();
        streams.retain(|stream| {
            let StreamKind::Trades {
                ticker_info: existing,
            } = stream
            else {
                return true;
            };
            if !enabled || *existing != ticker_info || found {
                false
            } else {
                found = true;
                true
            }
        });
        let added = enabled && !found;
        if added {
            streams.push(StreamKind::Trades { ticker_info });
        }
        streams.len() != original_len
    }

    pub fn canonicalize_kline_trade_streams(&mut self) -> bool {
        let ResolvedStream::Ready(streams) = self else {
            return false;
        };
        let Some(base_ticker) = streams.iter().find_map(|stream| match stream {
            StreamKind::Kline { ticker_info, .. } => Some(*ticker_info),
            _ => None,
        }) else {
            return false;
        };
        let original_len = streams.len();
        streams.retain(|stream| {
            !matches!(
                stream,
                StreamKind::Trades { ticker_info } if *ticker_info != base_ticker
            )
        });
        streams.len() != original_len
    }

    /// Reconcile the canonical depth stream for a ticker. Heatmap and ladder
    /// panes share this exchange subscription when they need the same book.
    pub fn set_depth_stream(&mut self, ticker_info: exchange::TickerInfo, enabled: bool) -> bool {
        let ResolvedStream::Ready(streams) = self else {
            return false;
        };

        let canonical = StreamKind::Depth {
            ticker_info,
            depth_aggr: exchange::adapter::StreamTicksize::Client,
            push_freq: exchange::PushFrequency::ServerDefault,
        };
        let mut changed = false;
        if enabled {
            let mut found = false;
            streams.retain_mut(|stream| {
                let StreamKind::Depth {
                    ticker_info: existing,
                    depth_aggr,
                    push_freq,
                } = stream
                else {
                    return true;
                };
                if *existing != ticker_info {
                    return true;
                }
                if found {
                    changed = true;
                    return false;
                }
                found = true;
                if *depth_aggr != exchange::adapter::StreamTicksize::Client
                    || *push_freq != exchange::PushFrequency::ServerDefault
                {
                    *depth_aggr = exchange::adapter::StreamTicksize::Client;
                    *push_freq = exchange::PushFrequency::ServerDefault;
                    changed = true;
                }
                true
            });
            if !found {
                streams.push(canonical);
                changed = true;
            }
        } else {
            let original_len = streams.len();
            streams.retain(|stream| {
                !matches!(stream, StreamKind::Depth { ticker_info: existing, .. } if *existing == ticker_info)
            });
            changed = streams.len() != original_len;
        }
        changed
    }

    pub fn find_ready_map<F, T>(&self, f: F) -> Option<T>
    where
        F: FnMut(&StreamKind) -> Option<T>,
    {
        match self {
            ResolvedStream::Ready(streams) => streams.iter().find_map(f),
            _ => None,
        }
    }

    pub fn into_waiting(self) -> Vec<PersistStreamKind> {
        match self {
            ResolvedStream::Waiting { streams, .. } => streams,
            ResolvedStream::Ready(streams) => {
                streams.into_iter().map(PersistStreamKind::from).collect()
            }
            ResolvedStream::Blocked { streams, .. } => streams,
        }
    }
}
