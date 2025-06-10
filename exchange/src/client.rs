use crate::adapter::StreamError;
use crate::limiter::{self, SourceLimit};
use once_cell::sync::Lazy;
use reqwest::Response;

static HTTP_CLIENT: Lazy<RateLimitedClient> = Lazy::new(|| RateLimitedClient::new());

pub async fn http_request(
    url: &str,
    source: SourceLimit,
    weight: Option<usize>,
) -> Result<String, StreamError> {
    HTTP_CLIENT.get_text(url, source, weight.unwrap_or(1)).await
}

pub struct RateLimitedClient {
    client: reqwest::Client,
}

impl RateLimitedClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    pub async fn get(
        &self,
        url: &str,
        source: SourceLimit,
        weight: usize,
    ) -> Result<Response, StreamError> {
        limiter::acquire_permit(source, weight).await;

        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(StreamError::FetchError)?;

        // These errors caused mostly by IP exceeding rate limits
        // They may be serious as in repeating can lead to IP ban, so we just kill the app
        // TODO: should probably handle this gracefully on higher level
        match source {
            SourceLimit::BinanceSpot | SourceLimit::BinancePerp => {
                if response.status() == 429 {
                    eprintln!("Binance API request returned 429 for: {}", url);
                    std::process::exit(1);
                }
            }
            SourceLimit::Bybit => {
                if response.status() == 403 {
                    eprintln!("Bybit API request returned 403 for: {}", url);
                    std::process::exit(1);
                }
            }
        }

        Ok(response)
    }

    pub async fn get_text(
        &self,
        url: &str,
        source: SourceLimit,
        weight: usize,
    ) -> Result<String, StreamError> {
        let response = self.get(url, source, weight).await?;
        response.text().await.map_err(StreamError::FetchError)
    }
}
