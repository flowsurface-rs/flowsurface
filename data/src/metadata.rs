use crate::write_json_to_file;
use exchange::adapter::Venue;
use exchange::unit::{ContractSize, MinQtySize, MinTicksize};
use exchange::{Ticker, TickerInfo};

use chrono::{DateTime, Utc};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::time::Duration;

const MARKET_METADATA_CACHE_PATH: &str = "metadata-cache.json";

/// Cache is background-refreshed when older than this.
const MARKET_METADATA_CACHE_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// Metadata as persisted on disk. The ticker is stored once, as the map key,
/// instead of being duplicated inside every value.
#[derive(Serialize, Deserialize)]
struct MetadataCacheInfo {
    min_ticksize: MinTicksize,
    min_qty: MinQtySize,
    contract_size: Option<ContractSize>,
}

impl From<TickerInfo> for MetadataCacheInfo {
    fn from(info: TickerInfo) -> Self {
        Self {
            min_ticksize: info.min_ticksize,
            min_qty: info.min_qty,
            contract_size: info.contract_size,
        }
    }
}

impl MetadataCacheInfo {
    fn into_ticker_info(self, ticker: Ticker) -> TickerInfo {
        TickerInfo {
            ticker,
            min_ticksize: self.min_ticksize,
            min_qty: self.min_qty,
            contract_size: self.contract_size,
        }
    }
}

/// Freshness is tracked per venue, so one failed venue can never be masked by
/// successful fetches of another.
#[derive(Serialize, Deserialize)]
struct VenueCacheFile {
    #[serde(default)]
    last_updated: DateTime<Utc>,
    tickers: HashMap<Ticker, Option<MetadataCacheInfo>>,
}

/// Origin of a metadata result. Network results are authoritative and always
/// applied; cache seeds carry the on-disk freshness stamp they were persisted
/// with, so a seed can never overwrite a newer result that already landed.
#[derive(Clone, Copy, Debug)]
pub enum MetadataSource {
    /// A fresh network fetch result.
    Fresh,
    /// A cache seed served from disk, with the venue's persisted freshness.
    /// `refresh_pending` marks a background network refresh queued behind the
    /// seed; the venue's fetch stays in-flight until that refresh lands.
    Cached {
        fetched_at: DateTime<Utc>,
        refresh_pending: bool,
    },
}

/// The app's market metadata: the fetched ticker info, the cache toggle, the
/// freshness stamps, and the per-venue in-flight set. Freshness is tracked per
/// venue, so one failed venue can never be masked by successful fetches of
/// another, and a missing stamp counts as stale (which drives a fetch).
#[derive(Default, Clone)]
pub struct MarketMetadata {
    cache_enabled: bool,
    tickers: FxHashMap<Ticker, Option<TickerInfo>>,
    last_updated: FxHashMap<Venue, DateTime<Utc>>,
    in_flight: FxHashSet<Venue>,
}

impl MarketMetadata {
    /// Create with the cache toggle from saved settings, loading the disk
    /// cache when it is enabled. Stale entries are still served and refreshed
    /// in the background instead of blocking startup.
    pub fn with_cache_enabled(cache_enabled: bool) -> Self {
        let mut this = cache_enabled.then(Self::load).flatten().unwrap_or_default();
        this.cache_enabled = cache_enabled;
        this
    }

    pub fn set_cache_enabled(&mut self, enabled: bool) {
        if enabled
            && self.tickers.is_empty()
            && self.last_updated.is_empty()
            && let Some(loaded) = Self::load()
        {
            *self = loaded;
        }
        self.cache_enabled = enabled;
    }

    pub fn cache_enabled(&self) -> bool {
        self.cache_enabled
    }

    pub fn tickers(&self) -> &FxHashMap<Ticker, Option<TickerInfo>> {
        &self.tickers
    }

    /// Entries available for a venue — session-fetched or cache-seeded —
    /// regardless of the cache toggle. Used for startup cache hits and for
    /// deciding whether a failed refresh can degrade gracefully.
    pub fn usable_entries_for(&self, venue: Venue) -> HashMap<Ticker, Option<TickerInfo>> {
        self.tickers
            .iter()
            .filter(|(ticker, info)| ticker.exchange.venue() == venue && info.is_some())
            .map(|(ticker, info)| (*ticker, *info))
            .collect()
    }

    /// The freshness stamp persisted for a venue, if any.
    pub fn last_updated_for(&self, venue: Venue) -> Option<DateTime<Utc>> {
        self.last_updated.get(&venue).copied()
    }

    /// The most recent freshness stamp among the given venues, if any.
    pub fn latest_update_for(&self, venues: &FxHashSet<Venue>) -> Option<DateTime<Utc>> {
        self.last_updated
            .iter()
            .filter(|(venue, _)| venues.contains(venue))
            .map(|(_, updated)| *updated)
            .max()
    }

    /// Whether a venue's metadata is missing or too old to trust. This is the
    /// single rule that decides when a (re)fetch is needed; a missing stamp
    /// counts as stale so the venue gets fetched.
    pub fn needs_fetch(&self, venue: Venue) -> bool {
        !self.is_venue_fresh(venue)
    }

    /// Mark a metadata fetch as started for a venue. Returns `false` if one is
    /// already in flight, so concurrent fetches for the same venue are
    /// impossible by construction.
    pub fn begin_fetch(&mut self, venue: Venue) -> bool {
        self.in_flight.insert(venue)
    }

    /// Mark a metadata fetch as finished for a venue, whether it succeeded or
    /// failed. The freshness stamp is untouched by failure: a failed refresh
    /// leaves the previous stamp in place, so `needs_fetch` keeps driving
    /// retries.
    pub fn complete_fetch(&mut self, venue: Venue) {
        self.in_flight.remove(&venue);
    }

    pub fn is_in_flight(&self, venue: Venue) -> bool {
        self.in_flight.contains(&venue)
    }

    pub fn any_in_flight(&self) -> bool {
        !self.in_flight.is_empty()
    }

    /// Apply a metadata result for a venue, stamping the venue's freshness.
    ///
    /// Single ingest rule: fresh network results always apply; cache seeds
    /// apply only when at least as new as the current stamp. A seed landing
    /// after a newer result (e.g. a network fetch that already completed) is
    /// skipped instead of clobbering it. Empty results never apply, so a
    /// degraded fetch can't renew the freshness window with nothing.
    pub fn ingest(
        &mut self,
        venue: Venue,
        entries: &HashMap<Ticker, Option<TickerInfo>>,
        source: MetadataSource,
    ) {
        if entries.is_empty() {
            return;
        }

        let fetched_at = match source {
            MetadataSource::Fresh => Utc::now(),
            MetadataSource::Cached { fetched_at, .. } => fetched_at,
        };

        let applies = match source {
            MetadataSource::Fresh => true,
            MetadataSource::Cached { fetched_at, .. } => self
                .last_updated
                .get(&venue)
                .is_none_or(|stamp| fetched_at >= *stamp),
        };

        if !applies {
            return;
        }

        self.tickers
            .retain(|ticker, _| ticker.exchange.venue() != venue);
        self.tickers
            .extend(entries.iter().map(|(ticker, info)| (*ticker, *info)));
        self.last_updated.insert(venue, fetched_at);
    }

    /// Merge this snapshot into the on-disk cache and persist it, newest
    /// entries winning per venue. Returns whether the data was persisted.
    pub fn save_to_file(&self) -> bool {
        let snapshot = match Self::disk_cache() {
            Some(disk) => self.merged_with(disk),
            None => self.clone(),
        };

        match serde_json::to_string(&snapshot) {
            Ok(json) => match write_json_to_file(&json, MARKET_METADATA_CACHE_PATH) {
                Ok(()) => true,
                Err(e) => {
                    log::warn!("Failed to write market metadata cache: {e}");
                    false
                }
            },
            Err(e) => {
                log::warn!("Failed to serialize market metadata cache: {e}");
                false
            }
        }
    }

    fn disk_cache() -> Option<Self> {
        let path = crate::data_path(Some(MARKET_METADATA_CACHE_PATH));
        let contents = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&contents).ok()
    }

    /// Per-venue merge where the side with the newer `last_updated` stamp
    /// wins. Writes are serialized by the app, so this reconciles against the
    /// file left by an earlier session instead of clobbering it.
    fn merged_with(&self, disk: MarketMetadata) -> Self {
        let mut merged = self.clone();

        for (venue, disk_updated) in &disk.last_updated {
            let newer_on_disk = merged
                .last_updated
                .get(venue)
                .is_none_or(|snapshot_updated| *disk_updated > *snapshot_updated);

            if newer_on_disk {
                merged.last_updated.insert(*venue, *disk_updated);
                merged
                    .tickers
                    .retain(|ticker, _| ticker.exchange.venue() != *venue);
                for (ticker, info) in disk
                    .tickers
                    .iter()
                    .filter(|(ticker, _)| ticker.exchange.venue() == *venue)
                {
                    merged.tickers.insert(*ticker, *info);
                }
            }
        }

        merged
    }

    /// Load the cache from disk. Returns `None` if the file is missing,
    /// unreadable, or fails to parse.
    fn load() -> Option<Self> {
        let path = crate::data_path(Some(MARKET_METADATA_CACHE_PATH));

        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => {
                log::warn!("Failed to read market metadata cache: {e}");
                return None;
            }
        };

        let cache: MarketMetadata = match serde_json::from_str(&contents) {
            Ok(cache) => cache,
            Err(e) => {
                log::warn!("Failed to parse market metadata cache: {e}");
                let backup = path.with_extension("old.json");
                if let Err(rename_err) = std::fs::rename(&path, &backup) {
                    log::warn!("Failed to quarantine corrupt market metadata cache: {rename_err}");
                }
                return None;
            }
        };
        Some(cache)
    }

    fn is_venue_fresh(&self, venue: Venue) -> bool {
        self.last_updated
            .get(&venue)
            .and_then(|updated| Utc::now().signed_duration_since(*updated).to_std().ok())
            .is_some_and(|age| age < MARKET_METADATA_CACHE_MAX_AGE)
    }
}

#[derive(Serialize, Deserialize)]
struct MetadataCacheFile {
    venues: HashMap<Venue, VenueCacheFile>,
}

impl Serialize for MarketMetadata {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut venues: HashMap<Venue, VenueCacheFile> = HashMap::default();
        for (ticker, info) in &self.tickers {
            let venue = ticker.exchange.venue();
            venues
                .entry(venue)
                .or_insert_with(|| VenueCacheFile {
                    last_updated: self.last_updated.get(&venue).copied().unwrap_or_default(),
                    tickers: HashMap::default(),
                })
                .tickers
                .insert(*ticker, info.map(MetadataCacheInfo::from));
        }

        MetadataCacheFile { venues }.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for MarketMetadata {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let file = MetadataCacheFile::deserialize(deserializer)?;

        let mut tickers = FxHashMap::default();
        let mut last_updated = FxHashMap::default();
        for (venue, entry) in file.venues {
            last_updated.insert(venue, entry.last_updated);
            for (ticker, info) in entry.tickers {
                tickers.insert(ticker, info.map(|info| info.into_ticker_info(ticker)));
            }
        }

        Ok(Self {
            tickers,
            last_updated,
            ..Default::default()
        })
    }
}
