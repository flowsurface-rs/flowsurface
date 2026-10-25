use data::chart::kline::indicators::{self, IndicatorId, language::PackageOrigin};
use iced::Task;
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

const API_ORIGIN: &str = "https://api.flowsurface.com";
const MAX_CATALOG_BYTES: usize = 4 << 20;

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
    path: String,
    sha256: String,
    size_bytes: usize,
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

impl From<String> for FetchError {
    fn from(message: String) -> Self {
        Self {
            message,
            retry_at: None,
        }
    }
}

#[derive(Default)]
pub struct CatalogState {
    catalog: Option<Catalog>,
    status: Status,
}

#[derive(Default)]
enum Status {
    #[default]
    Unopened,
    Loading,
    Ready,
    Failed(FetchError),
}

impl CatalogState {
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

    fn is_unopened(&self) -> bool {
        matches!(self.status, Status::Unopened)
    }

    fn begin_loading(&mut self) {
        self.status = Status::Loading;
    }

    fn loaded(&mut self, catalog: Catalog) {
        self.catalog = Some(catalog);
        self.status = Status::Ready;
    }

    fn failed(&mut self, error: FetchError) {
        self.status = Status::Failed(error);
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

#[derive(Default)]
pub enum SourceOperation {
    #[default]
    Idle,
    Running,
    Failed {
        id: IndicatorId,
        error: FetchError,
    },
}

#[derive(Default)]
pub struct State {
    catalog: CatalogState,
    pending: BTreeSet<IndicatorId>,
    operation: SourceOperation,
}

impl State {
    pub fn catalog_state(&self) -> &CatalogState {
        &self.catalog
    }

    pub fn error_for(&self, id: &IndicatorId) -> Option<&FetchError> {
        match &self.operation {
            SourceOperation::Failed { id: failed, error } if failed == id => Some(error),
            _ => None,
        }
    }

    pub fn installation(&self, entry: &Entry) -> Installation {
        let origin = entry.registration().map(|registration| registration.origin);
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

    fn open(&mut self, client: &reqwest::Client) -> Task<Message> {
        if self.catalog.is_unopened() {
            self.refresh(client)
        } else {
            Task::none()
        }
    }

    fn can_request(&self) -> bool {
        let catalog_retry = self.catalog.error().and_then(|error| error.retry_at);
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

    pub fn can_refresh(&self) -> bool {
        !self.catalog.is_loading() && self.can_request()
    }

    pub fn can_toggle(&self, installation: Installation) -> bool {
        !matches!(installation, Installation::Bundled) && !self.is_running() && self.can_request()
    }

    fn is_running(&self) -> bool {
        matches!(self.operation, SourceOperation::Running)
    }

    fn record_change(&mut self, id: IndicatorId, change: SourceChange) {
        let loaded = indicators::catalog().get(&id).is_some();
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

    fn refresh(&mut self, client: &reqwest::Client) -> Task<Message> {
        if !self.can_refresh() {
            return Task::none();
        }
        self.catalog.begin_loading();
        Task::perform(Catalog::fetch(client.clone()), Message::Fetched)
    }

    pub fn update(&mut self, message: Message, client: &reqwest::Client) -> Task<Message> {
        match message {
            Message::Open => return self.open(client),
            Message::Refresh => return self.refresh(client),
            Message::ToggleSource(id) => {
                let Some(catalog) = self.catalog.catalog() else {
                    return Task::none();
                };
                let Some(entry) = catalog
                    .indicators
                    .iter()
                    .find(|entry| entry.id == id)
                    .cloned()
                else {
                    return Task::none();
                };

                let installation = self.installation(&entry);
                if !self.can_toggle(installation) {
                    return Task::none();
                }

                let base = catalog.source_base_path.clone();
                self.operation = SourceOperation::Running;

                if matches!(installation, Installation::Installed) {
                    let removed = id.clone();
                    return Task::perform(
                        blocking(move || indicators::remove_indicator_source(&removed)),
                        move |result| Message::SourceChanged {
                            id,
                            change: SourceChange::Removed,
                            result,
                        },
                    );
                }

                return Task::perform(entry.install(base, client.clone()), move |result| {
                    Message::SourceChanged {
                        id,
                        change: SourceChange::Installed,
                        result,
                    }
                });
            }
            Message::Fetched(result) => match result {
                Ok(catalog) => self.catalog.loaded(catalog),
                Err(error) => self.catalog.failed(error),
            },
            Message::SourceChanged { id, change, result } => match result {
                Ok(()) => {
                    self.record_change(id, change);
                    self.operation = SourceOperation::Idle;
                }
                Err(error) => self.operation = SourceOperation::Failed { id, error },
            },
        }
        Task::none()
    }
}

impl Catalog {
    async fn fetch(client: reqwest::Client) -> Result<Self, FetchError> {
        let bytes = download(
            format!("{API_ORIGIN}/v1/indicators"),
            MAX_CATALOG_BYTES,
            client,
        )
        .await?;

        let mut catalog: Self =
            serde_json::from_slice(&bytes).map_err(|error| format!("Invalid catalog: {error}"))?;
        catalog.validate()?;
        catalog
            .indicators
            .sort_by_cached_key(|entry| entry.name.to_lowercase());
        Ok(catalog)
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

impl Entry {
    pub fn registration(&self) -> Option<&'static indicators::IndicatorRegistration> {
        indicators::catalog().get(&self.id)
    }

    async fn install(self, base: String, client: reqwest::Client) -> Result<(), FetchError> {
        let mut url = url::Url::parse(API_ORIGIN).map_err(|error| error.to_string())?;
        url.set_path(&base);
        url.path_segments_mut()
            .map_err(|()| "Invalid source URL".to_owned())?
            .pop_if_empty()
            .extend(self.path.split('/'));

        let bytes = download(url.into(), self.size_bytes, client).await?;

        let digest = ring::digest::digest(&ring::digest::SHA256, &bytes);
        let checksum = digest
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if bytes.len() != self.size_bytes || !checksum.eq_ignore_ascii_case(&self.sha256) {
            return Err(
                "Downloaded source does not match its catalog checksum or size"
                    .to_owned()
                    .into(),
            );
        }

        blocking(move || indicators::write_indicator_source(&self.id, &bytes)).await
    }
}

async fn blocking(
    work: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> Result<(), FetchError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| format!("Indicator worker failed: {error}"))?
        .map_err(FetchError::from)
}

async fn download(
    url: String,
    limit: usize,
    client: reqwest::Client,
) -> Result<Vec<u8>, FetchError> {
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let seconds = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| {
                value.parse::<u32>().ok().or_else(|| {
                    chrono::DateTime::parse_from_rfc2822(value)
                        .ok()
                        .and_then(|at| {
                            (at.timestamp() - chrono::Utc::now().timestamp())
                                .max(0)
                                .try_into()
                                .ok()
                        })
                })
            })
            .unwrap_or(10);
        let seconds = u64::from(seconds);
        return Err(FetchError {
            message: format!("Too many requests. Try again in {seconds} seconds."),
            retry_at: Some(Instant::now() + Duration::from_secs(seconds)),
        });
    }

    if !response.status().is_success() {
        return Err(format!("Indicator server returned {}", response.status()).into());
    }

    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err("Server response exceeds the allowed size".to_owned().into());
    }

    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err("Server response exceeds the allowed size".to_owned().into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
