use data::chart::kline::indicators::{
    self, IndicatorId,
    manifest::{Catalog, Entry, FetchError},
};
use std::time::Duration;

const API_ORIGIN: &str = "https://api.flowsurface.com";
const MAX_CATALOG_BYTES: usize = 4 << 20;

pub async fn fetch(client: reqwest::Client) -> Result<Catalog, FetchError> {
    let bytes = download(
        format!("{API_ORIGIN}/v1/indicators"),
        MAX_CATALOG_BYTES,
        client,
    )
    .await?;

    Catalog::parse(&bytes).map_err(FetchError::from)
}

pub async fn install(
    entry: Entry,
    base: String,
    client: reqwest::Client,
) -> Result<(), FetchError> {
    let mut url = url::Url::parse(API_ORIGIN).map_err(|error| error.to_string())?;
    url.set_path(&base);
    url.path_segments_mut()
        .map_err(|()| "Invalid source URL".to_owned())?
        .pop_if_empty()
        .extend(entry.path.split('/'));

    let bytes = download(url.into(), entry.size_bytes, client).await?;

    let digest = ring::digest::digest(&ring::digest::SHA256, &bytes);
    let checksum = digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if bytes.len() != entry.size_bytes || !checksum.eq_ignore_ascii_case(&entry.sha256) {
        return Err(
            "Downloaded source does not match its catalog checksum or size"
                .to_owned()
                .into(),
        );
    }

    blocking(move || indicators::write_indicator_source(&entry.id, &bytes)).await
}

pub async fn remove(id: IndicatorId) -> Result<(), FetchError> {
    blocking(move || indicators::remove_indicator_source(&id)).await
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
        return Err(FetchError::retry_after(
            format!("Too many requests. Try again in {seconds} seconds."),
            Duration::from_secs(seconds),
        ));
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
