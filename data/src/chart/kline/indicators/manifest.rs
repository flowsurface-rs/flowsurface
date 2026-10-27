use super::{IndicatorId, catalog, language::PackageOrigin};
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

/// Indicator sources published by the catalog service, as the host may install them.
#[derive(Debug, Clone, Deserialize)]
pub struct Catalog {
    schema_version: u32,
    language_version: u32,
    source_revision: String,
    source_base_path: String,
    pub indicators: Vec<Entry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Entry {
    pub id: IndicatorId,
    pub name: String,
    pub description: Option<String>,
    pub author: Option<Author>,
    pub bases: Vec<String>,
    pub path: String,
    pub sha256: String,
    pub size_bytes: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Author {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct FetchError {
    pub message: String,
    retry_at: Option<Instant>,
}

impl FetchError {
    pub fn retry_after(message: impl Into<String>, delay: Duration) -> Self {
        Self {
            message: message.into(),
            retry_at: Some(Instant::now() + delay),
        }
    }
}

impl From<String> for FetchError {
    fn from(message: String) -> Self {
        Self {
            message,
            retry_at: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceChange {
    Installed,
    Removed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installation {
    Bundled,
    Installed,
    NotInstalled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// Loads only while the catalog has never been requested.
    Open,
    /// Reloads on request, subject to the retry cooldown.
    Refresh,
}

/// Effect the host performs once a toggle is accepted.
#[derive(Debug, Clone)]
pub enum SourceAction {
    Install { entry: Entry, base: String },
    Remove(IndicatorId),
}

#[derive(Debug, Clone)]
pub enum Message {
    Open,
    Refresh,
    ToggleSource(IndicatorId),
    Fetched(Result<Catalog, FetchError>),
    SourceChanged {
        id: IndicatorId,
        change: SourceChange,
        result: Result<(), FetchError>,
    },
}

#[derive(Default)]
pub struct State {
    catalog: Option<Catalog>,
    status: Status,
    pending: BTreeSet<IndicatorId>,
    operation: SourceOperation,
}

#[derive(Default)]
enum Status {
    #[default]
    Unopened,
    Loading,
    Ready,
    Failed(FetchError),
}

#[derive(Default)]
enum SourceOperation {
    #[default]
    Idle,
    Running,
    Failed {
        id: IndicatorId,
        error: FetchError,
    },
}

impl Catalog {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let mut catalog: Self =
            serde_json::from_slice(bytes).map_err(|error| format!("Invalid catalog: {error}"))?;
        catalog.validate()?;
        catalog
            .indicators
            .sort_by_cached_key(|entry| entry.name.to_lowercase());
        Ok(catalog)
    }

    pub fn source_base_path(&self) -> &str {
        &self.source_base_path
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 || self.language_version != 1 {
            return Err(
                "This catalog requires an unsupported format or language version".to_owned(),
            );
        }

        if self.source_revision.len() != 40
            || !self
                .source_revision
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || self.source_base_path != format!("/v1/sources/{}/", self.source_revision)
        {
            return Err("Invalid catalog source revision".to_owned());
        }

        let mut ids = BTreeSet::new();
        for entry in &self.indicators {
            if !ids.insert(&entry.id) {
                return Err("Catalog contains duplicate IDs".to_owned());
            }

            if !entry.path.ends_with(".ind")
                || entry.path.split('/').any(|part| {
                    part.is_empty()
                        || part == "."
                        || part == ".."
                        || part.contains(['\\', ':', '?', '#', '%'])
                })
            {
                return Err(format!("Invalid source path for {}", entry.id.as_str()));
            }

            if entry.size_bytes == 0
                || entry.size_bytes > indicator_engine::CompileLimits::default().max_source_bytes
                || entry.sha256.len() != 64
                || !entry.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(format!(
                    "Invalid source size or checksum for {}",
                    entry.id.as_str()
                ));
            }
        }
        Ok(())
    }
}

impl State {
    pub fn catalog(&self) -> Option<&Catalog> {
        self.catalog.as_ref()
    }

    pub fn is_loading(&self) -> bool {
        matches!(self.status, Status::Loading)
    }

    pub fn error(&self) -> Option<&FetchError> {
        match &self.status {
            Status::Failed(error) => Some(error),
            _ => None,
        }
    }

    pub fn error_for(&self, id: &IndicatorId) -> Option<&FetchError> {
        match &self.operation {
            SourceOperation::Failed { id: failed, error } if failed == id => Some(error),
            _ => None,
        }
    }

    pub fn installation(&self, entry: &Entry) -> Installation {
        let origin = catalog()
            .get(&entry.id)
            .map(|registration| registration.origin);
        if origin == Some(PackageOrigin::Bundled) {
            return Installation::Bundled;
        }

        let pending = self.pending.contains(&entry.id);
        let present = match origin {
            Some(_) => !pending,
            None => pending,
        };
        if present {
            Installation::Installed
        } else {
            Installation::NotInstalled
        }
    }

    pub fn restart_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn restart_pending_for(&self, id: &IndicatorId) -> bool {
        self.pending.contains(id)
    }

    pub fn can_refresh(&self) -> bool {
        !self.is_loading() && self.can_request()
    }

    pub fn can_toggle(&self, installation: Installation) -> bool {
        !matches!(installation, Installation::Bundled) && !self.is_running() && self.can_request()
    }

    pub fn begin_load(&mut self, trigger: Trigger) -> bool {
        let allowed = match trigger {
            Trigger::Open => self.is_unopened() && self.can_request(),
            Trigger::Refresh => self.can_refresh(),
        };
        if allowed {
            self.status = Status::Loading;
        }
        allowed
    }

    pub fn loaded(&mut self, catalog: Catalog) {
        self.catalog = Some(catalog);
        self.status = Status::Ready;
    }

    pub fn failed(&mut self, error: FetchError) {
        self.status = Status::Failed(error);
    }

    pub fn begin_toggle(&mut self, id: &IndicatorId) -> Option<SourceAction> {
        let manifest = self.catalog.as_ref()?;
        let entry = manifest
            .indicators
            .iter()
            .find(|entry| entry.id == *id)?
            .clone();
        let base = manifest.source_base_path().to_owned();

        let installation = self.installation(&entry);
        if !self.can_toggle(installation) {
            return None;
        }
        self.operation = SourceOperation::Running;

        Some(if matches!(installation, Installation::Installed) {
            SourceAction::Remove(entry.id)
        } else {
            SourceAction::Install { entry, base }
        })
    }

    pub fn operation_succeeded(&mut self, id: IndicatorId, change: SourceChange) {
        self.record_change(id, change);
        self.operation = SourceOperation::Idle;
    }

    pub fn operation_failed(&mut self, id: IndicatorId, error: FetchError) {
        self.operation = SourceOperation::Failed { id, error };
    }

    fn is_unopened(&self) -> bool {
        matches!(self.status, Status::Unopened)
    }

    fn is_running(&self) -> bool {
        matches!(self.operation, SourceOperation::Running)
    }

    fn can_request(&self) -> bool {
        let catalog_retry = self.error().and_then(|error| error.retry_at);
        let operation_retry = match &self.operation {
            SourceOperation::Failed { error, .. } => error.retry_at,
            _ => None,
        };
        catalog_retry
            .into_iter()
            .chain(operation_retry)
            .max()
            .is_none_or(|at| Instant::now() >= at)
    }

    fn record_change(&mut self, id: IndicatorId, change: SourceChange) {
        let loaded = catalog().get(&id).is_some();
        let pending = match change {
            SourceChange::Installed => !loaded,
            SourceChange::Removed => loaded,
        };
        if pending {
            self.pending.insert(id);
        } else {
            self.pending.remove(&id);
        }
    }
}
