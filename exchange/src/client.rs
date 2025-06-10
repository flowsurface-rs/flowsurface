use crate::adapter::StreamError;
use crate::limiter::{self, SourceLimit};
use once_cell::sync::Lazy;
use reqwest::Response;

static HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(reqwest::Client::new);

pub async fn http_request(
    url: &str,
    source: SourceLimit,
    weight: Option<usize>,
) -> Result<String, StreamError> {
    let response = rate_limited_get(url, source, weight.unwrap_or(1)).await?;
    response.text().await.map_err(StreamError::FetchError)
}

async fn rate_limited_get(
    url: &str,
    source: SourceLimit,
    weight: usize,
) -> Result<Response, StreamError> {
    limiter::acquire_permit(source, weight).await;

    let response = HTTP_CLIENT
        .get(url)
        .send()
        .await
        .map_err(StreamError::FetchError)?;

    let status = response.status();
    // These errors mostly related to IP/rate limiting/location restrictions
    // They may be serious as in they can act as a warning before IP ban;
    // we shouldn't ever end up here, so currently we just terminate the whole app
    // TODO: should probably handle this gracefully on higher level
    match source {
        SourceLimit::BinanceSpot | SourceLimit::BinancePerp => {
            if status == 429 || status == 418 {
                eprintln!("Binance API request returned {} for: {}", status, url);
                std::process::exit(1);
            }
        }
        SourceLimit::Bybit => {
            if status == 403 {
                eprintln!("Bybit API request returned {} for: {}", status, url);
                std::process::exit(1);
            }
        }
    }

    Ok(response)
}
