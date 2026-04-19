use std::error::Error;

#[derive(thiserror::Error, Debug)]
pub enum AdapterError {
    #[error("{0}")]
    FetchError(FetchError),
    #[error("Parsing: {0}")]
    ParseError(String),
    #[error("Stream: {0}")]
    WebsocketError(String),
    #[error("Invalid request: {0}")]
    InvalidRequest(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AdapterErrorKind {
    Fetch,
    Parse,
    Websocket,
    InvalidRequest,
}

#[derive(Debug)]
pub struct FetchError {
    detail: String,
    human_message: &'static str,
}

impl FetchError {
    fn from_reqwest_detail(error: &reqwest::Error, detail: String) -> Self {
        let human_message = ReqwestErrorKind::from_error(error).ui_message();

        Self {
            detail,
            human_message,
        }
    }

    fn from_status_detail(status: reqwest::StatusCode, detail: String) -> Self {
        let human_message = if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            "Rate limited. Check logs for details."
        } else if status.is_server_error() {
            "Server error. Check logs for details."
        } else if status.is_client_error() {
            "Request was rejected. Check logs for details."
        } else {
            "Request failed. Check logs for details."
        };

        Self {
            detail,
            human_message,
        }
    }

    pub fn human_message(&self) -> &'static str {
        self.human_message
    }

    pub fn ui_message(&self) -> &'static str {
        self.human_message()
    }
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.detail)
    }
}

impl From<reqwest::Error> for AdapterError {
    fn from(error: reqwest::Error) -> Self {
        let detail = ReqwestErrorContext::from_error(&error, None).summary();
        Self::FetchError(FetchError::from_reqwest_detail(&error, detail))
    }
}

impl AdapterError {
    pub fn kind(&self) -> AdapterErrorKind {
        match self {
            Self::FetchError(_) => AdapterErrorKind::Fetch,
            Self::ParseError(_) => AdapterErrorKind::Parse,
            Self::WebsocketError(_) => AdapterErrorKind::Websocket,
            Self::InvalidRequest(_) => AdapterErrorKind::InvalidRequest,
        }
    }

    pub(crate) fn request_failed(
        method: &reqwest::Method,
        url: &str,
        error: reqwest::Error,
    ) -> Self {
        let detail = format!(
            "{}: request failed | {}",
            method,
            ReqwestErrorContext::from_error(&error, Some(url)).summary()
        );
        Self::FetchError(FetchError::from_reqwest_detail(&error, detail))
    }

    pub(crate) fn response_body_failed(
        method: &reqwest::Method,
        url: &str,
        status: reqwest::StatusCode,
        content_type: &str,
        error: reqwest::Error,
    ) -> Self {
        let detail = format!(
            "{}: failed reading response body | status={} | content-type={} | {}",
            method,
            status,
            content_type,
            ReqwestErrorContext::from_error(&error, Some(url)).summary()
        );
        Self::FetchError(FetchError::from_reqwest_detail(&error, detail))
    }

    pub(crate) fn http_status_failed(status: reqwest::StatusCode, detail: String) -> Self {
        Self::FetchError(FetchError::from_status_detail(status, detail))
    }

    pub fn human_message(&self) -> String {
        match self {
            Self::FetchError(error) => error.human_message().to_string(),
            Self::ParseError(_) => "Invalid server response. Check logs for details.".to_string(),
            Self::WebsocketError(_) => "Stream error. Check logs for details.".to_string(),
            Self::InvalidRequest(message) => message.clone(),
        }
    }

    pub fn ui_message(&self) -> String {
        self.human_message()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReqwestErrorKind {
    Timeout,
    Connect,
    Decode,
    Body,
    Request,
    Other,
}

impl ReqwestErrorKind {
    fn from_error(error: &reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::Timeout
        } else if error.is_connect() {
            Self::Connect
        } else if error.is_decode() {
            Self::Decode
        } else if error.is_body() {
            Self::Body
        } else if error.is_request() {
            Self::Request
        } else {
            Self::Other
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Connect => "connect",
            Self::Decode => "decode",
            Self::Body => "body",
            Self::Request => "request",
            Self::Other => "other",
        }
    }

    fn ui_message(self) -> &'static str {
        match self {
            Self::Timeout => "Request timed out. Check logs for details.",
            Self::Connect => "Connection failed. Check logs for details.",
            Self::Decode | Self::Body => "Invalid server response. Check logs for details.",
            Self::Request | Self::Other => "Request failed. Check logs for details.",
        }
    }
}

#[derive(Debug)]
struct ReqwestErrorContext {
    message: String,
    kind: ReqwestErrorKind,
    status: Option<reqwest::StatusCode>,
    url: Option<String>,
    io_source: Option<String>,
    source_chain: Option<String>,
}

impl ReqwestErrorContext {
    fn from_error(error: &reqwest::Error, request_url: Option<&str>) -> Self {
        let target = Self::request_target_url(error, request_url);

        Self {
            message: Self::sanitize_error_message(error),
            kind: ReqwestErrorKind::from_error(error),
            status: error.status(),
            url: target.map(|url| url.to_string()),
            io_source: Self::io_source(error),
            source_chain: Self::source_chain(error),
        }
    }

    fn summary(&self) -> String {
        let mut details = vec![
            format!("error={}", self.message),
            format!("kind={}", self.kind.as_str()),
        ];

        if let Some(status) = self.status {
            details.push(format!("status={status}"));
        }

        if let Some(url) = &self.url {
            details.push(format!("url={url}"));
        }

        if let Some(io_source) = &self.io_source {
            details.push(io_source.clone());
        }

        if let Some(source_chain) = &self.source_chain {
            details.push(format!("source_chain={source_chain}"));
        }

        details.join(" | ")
    }

    fn sanitize_error_message(error: &reqwest::Error) -> String {
        let mut message = error.to_string().replace('\n', "\\n");

        if let Some(idx) = message.find(" for url (") {
            message.truncate(idx);
        }

        if message.len() > 180 {
            message.truncate(177);
            message.push_str("...");
        }

        message
    }

    fn request_target_url(
        error: &reqwest::Error,
        request_url: Option<&str>,
    ) -> Option<reqwest::Url> {
        error
            .url()
            .cloned()
            .or_else(|| request_url.and_then(|url| reqwest::Url::parse(url).ok()))
    }

    fn source_chain(error: &reqwest::Error) -> Option<String> {
        let mut current = error.source();
        let mut chain = Vec::new();

        while let Some(source) = current {
            if chain.len() >= 8 {
                chain.push("...".to_string());
                break;
            }

            let mut msg = source.to_string().replace('\n', "\\n");

            if msg.len() > 180 {
                msg.truncate(177);
                msg.push_str("...");
            }

            chain.push(msg);
            current = source.source();
        }

        if chain.is_empty() {
            None
        } else {
            Some(chain.join(" <- "))
        }
    }

    fn io_source(error: &reqwest::Error) -> Option<String> {
        let mut current = error.source();

        while let Some(source) = current {
            if let Some(io_error) = source.downcast_ref::<std::io::Error>() {
                let mut details = vec![format!("io_kind={:?}", io_error.kind())];

                if let Some(code) = io_error.raw_os_error() {
                    details.push(format!("os_error={code}"));
                }

                return Some(details.join(" | "));
            }

            current = source.source();
        }

        None
    }
}
