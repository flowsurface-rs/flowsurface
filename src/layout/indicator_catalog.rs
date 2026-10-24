use data::chart::kline::indicators::{self, IndicatorId, language::PackageOrigin};
use iced::Task;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

const API_ORIGIN: &str = "https://api.flowsurface.com";
const MAX_CATALOG_BYTES: usize = 4 << 20;

#[derive(Debug, Clone)]
pub enum Message {
    Open,
    Refresh,
    Install(IndicatorId),
    Remove(IndicatorId),
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
pub enum CatalogState {
    #[default]
    Unopened,
    Loading(Option<Catalog>),
    Ready(Catalog),
    Failed {
        previous: Option<Catalog>,
        error: FetchError,
    },
}

impl CatalogState {
    pub fn catalog(&self) -> Option<&Catalog> {
        match self {
            Self::Unopened => None,
            Self::Loading(previous) | Self::Failed { previous, .. } => previous.as_ref(),
            Self::Ready(catalog) => Some(catalog),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceChange {
    Installed,
    Removed,
}

#[derive(Default)]
pub enum SourceOperation {
    #[default]
    Idle,
    Running {
        id: IndicatorId,
        change: SourceChange,
    },
    Failed {
        id: IndicatorId,
        error: FetchError,
    },
}

#[derive(Default)]
pub struct State {
    catalog: CatalogState,
    source_changes: BTreeMap<IndicatorId, SourceChange>,
    operation: SourceOperation,
}

impl State {
    pub fn catalog_state(&self) -> &CatalogState {
        &self.catalog
    }

    pub fn operation(&self) -> &SourceOperation {
        &self.operation
    }

    pub fn source_change(&self, id: &IndicatorId) -> Option<SourceChange> {
        self.source_changes.get(id).copied()
    }

    fn open(&mut self, client: &reqwest::Client) -> Task<Message> {
        if matches!(self.catalog, CatalogState::Unopened) {
            self.refresh(client)
        } else {
            Task::none()
        }
    }

    fn can_request(&self) -> bool {
        let catalog_retry = match &self.catalog {
            CatalogState::Failed { error, .. } => error.retry_at,
            _ => None,
        };
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
        !matches!(self.catalog, CatalogState::Loading(_)) && self.can_request()
    }

    pub fn can_install(&self, entry: &Entry) -> bool {
        !matches!(self.operation, SourceOperation::Running { .. })
            && self.source_changes.get(&entry.id) != Some(&SourceChange::Installed)
            && (entry.registration().is_none()
                || self.source_changes.get(&entry.id) == Some(&SourceChange::Removed))
            && self.can_request()
    }

    pub fn is_installed(&self, entry: &Entry) -> bool {
        match self.source_changes.get(&entry.id) {
            Some(SourceChange::Installed) => true,
            Some(SourceChange::Removed) => false,
            None => entry
                .registration()
                .is_some_and(|registration| registration.origin == PackageOrigin::Installed),
        }
    }

    pub fn can_remove(&self, entry: &Entry) -> bool {
        self.is_installed(entry)
            && !matches!(self.operation, SourceOperation::Running { .. })
            && self.can_request()
    }

    fn refresh(&mut self, client: &reqwest::Client) -> Task<Message> {
        if !self.can_refresh() {
            return Task::none();
        }
        self.catalog = CatalogState::Loading(self.catalog.catalog().cloned());
        Task::perform(Catalog::fetch(client.clone()), Message::Fetched)
    }

    pub fn update(&mut self, message: Message, client: &reqwest::Client) -> Task<Message> {
        match message {
            Message::Open => return self.open(client),
            Message::Refresh => return self.refresh(client),
            Message::Install(id) => {
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
                if !self.can_install(&entry) {
                    return Task::none();
                }
                let base = catalog.source_base_path.clone();
                let restore = self.source_changes.get(&id) == Some(&SourceChange::Removed);
                self.operation = SourceOperation::Running {
                    id: id.clone(),
                    change: SourceChange::Installed,
                };
                return Task::perform(
                    entry.install(base, client.clone(), restore),
                    move |result| Message::SourceChanged {
                        id: id.clone(),
                        change: SourceChange::Installed,
                        result,
                    },
                );
            }
            Message::Remove(id) => {
                let Some(entry) = self
                    .catalog
                    .catalog()
                    .and_then(|catalog| catalog.indicators.iter().find(|entry| entry.id == id))
                else {
                    return Task::none();
                };
                if !self.can_remove(entry) {
                    return Task::none();
                }
                self.operation = SourceOperation::Running {
                    id: id.clone(),
                    change: SourceChange::Removed,
                };
                return Task::perform(
                    async move {
                        let result = tokio::task::spawn_blocking(move || {
                            indicators::remove_indicator_source(&id)
                        })
                        .await
                        .map_err(|error| error.to_string())?;
                        result.map_err(FetchError::from)
                    },
                    {
                        let id = entry.id.clone();
                        move |result| Message::SourceChanged {
                            id: id.clone(),
                            change: SourceChange::Removed,
                            result,
                        }
                    },
                );
            }
            Message::Fetched(result) => {
                self.catalog = match result {
                    Ok(catalog) => CatalogState::Ready(catalog),
                    Err(error) => CatalogState::Failed {
                        previous: self.catalog.catalog().cloned(),
                        error,
                    },
                };
            }
            Message::SourceChanged { id, change, result } => match result {
                Ok(()) => {
                    let restored = change == SourceChange::Installed
                        && self.source_changes.get(&id) == Some(&SourceChange::Removed);
                    if restored && indicators::catalog().get(&id).is_some() {
                        self.source_changes.remove(&id);
                    } else {
                        self.source_changes.insert(id, change);
                    }
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

    async fn install(
        self,
        base: String,
        client: reqwest::Client,
        restore: bool,
    ) -> Result<(), FetchError> {
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
        tokio::task::spawn_blocking(move || {
            if restore {
                indicators::restore_indicator_source(&self.id, &bytes)
            } else {
                indicators::install_indicator_source(&self.id, &bytes)
            }
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(Into::into)
    }
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
