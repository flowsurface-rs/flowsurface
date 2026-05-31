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
}

impl ResolvedStream {
    pub fn waiting(streams: Vec<PersistStreamKind>) -> Self {
        ResolvedStream::Waiting {
            streams,
            last_attempt: None,
        }
    }

    /// Returns streams to resolve only if the retry interval has elapsed
    pub fn due_streams_to_resolve(&mut self, now: Instant) -> Option<Vec<PersistStreamKind>> {
        let ResolvedStream::Waiting {
            streams,
            last_attempt,
        } = self
        else {
            return None;
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
        }
    }
}
