use crate::adapter::AdapterError;

use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum ProxyScheme {
    Http,
    Https,
    Socks5,
    Socks5h,
}

impl ProxyScheme {
    pub fn as_str(self) -> &'static str {
        match self {
            ProxyScheme::Http => "http",
            ProxyScheme::Https => "https",
            ProxyScheme::Socks5 => "socks5",
            ProxyScheme::Socks5h => "socks5h",
        }
    }

    pub const ALL: [ProxyScheme; 4] = [
        ProxyScheme::Http,
        ProxyScheme::Https,
        ProxyScheme::Socks5,
        ProxyScheme::Socks5h,
    ];
}

impl std::fmt::Display for ProxyScheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ProxyAuth {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Proxy {
    pub scheme: ProxyScheme,
    pub host: String,
    pub port: u16,
    pub auth: Option<ProxyAuth>,
}

impl Proxy {
    pub async fn connect_tcp(
        &self,
        target_host: &str,
        target_port: u16,
    ) -> Result<TcpStream, AdapterError> {
        match self.scheme {
            ProxyScheme::Http => {
                let proxy_addr = format!("{}:{}", self.host, self.port);

                let mut stream = TcpStream::connect(&proxy_addr)
                    .await
                    .map_err(|e| AdapterError::WebsocketError(e.to_string()))?;

                let proxy_auth = match &self.auth {
                    None => None,
                    Some(ProxyAuth { username, password }) => {
                        let token = BASE64.encode(format!("{username}:{password}"));
                        Some(format!("Basic {token}"))
                    }
                };

                http_connect_tunnel(&mut stream, target_host, target_port, proxy_auth.as_deref())
                    .await?;

                Ok(stream)
            }
            ProxyScheme::Https => Err(AdapterError::WebsocketError(
                "HTTPS proxy scheme is not supported for websocket connections (would require TLS-to-proxy + CONNECT). Use http://, socks5://, or implement HTTPS-proxy tunneling."
                    .to_string(),
            )),
            // Treat socks5h as socks5; tokio-socks will send the domain name to the proxy
            ProxyScheme::Socks5 | ProxyScheme::Socks5h => {
                let proxy_addr = (self.host.as_str(), self.port);
                let target_addr = (target_host, target_port);

                let stream = match self.auth.as_ref() {
                    Some(ProxyAuth { username, password }) => {
                        tokio_socks::tcp::Socks5Stream::connect_with_password(
                            proxy_addr,
                            target_addr,
                            username,
                            password,
                        )
                        .await
                        .map_err(|e| AdapterError::WebsocketError(e.to_string()))?
                        .into_inner()
                    }
                    None => tokio_socks::tcp::Socks5Stream::connect(proxy_addr, target_addr)
                        .await
                        .map_err(|e| AdapterError::WebsocketError(e.to_string()))?
                        .into_inner(),
                };

                Ok(stream)
            }
        }
    }

    pub fn try_from_str_strict(s: &str) -> Result<Self, String> {
        let s = s.trim();

        if s.is_empty() {
            return Err("Proxy URL is empty".to_string());
        }
        if !s.contains("://") {
            return Err(format!(
                "Invalid proxy value (missing scheme): {s:?}. Expected e.g. http://127.0.0.1:8080 or socks5h://127.0.0.1:1080."
            ));
        }

        let url = url::Url::parse(s).map_err(|e| format!("Invalid proxy URL: {e}"))?;
        Self::try_from_url(&url)
    }

    pub fn try_from_url(url: &url::Url) -> Result<Self, String> {
        let scheme_str = url.scheme().to_ascii_lowercase();
        let scheme = match scheme_str.as_str() {
            "http" => ProxyScheme::Http,
            "https" => ProxyScheme::Https,
            "socks5" => ProxyScheme::Socks5,
            "socks5h" => ProxyScheme::Socks5h,
            _ => {
                return Err(format!(
                    "Unsupported proxy scheme: {scheme_str} (use http://, https://, socks5://, socks5h://)"
                ));
            }
        };

        let host = url
            .host_str()
            .ok_or_else(|| "Proxy host missing".to_string())?
            .to_string();

        let port = url
            .port_or_known_default()
            .ok_or_else(|| "Proxy port missing".to_string())?;

        let username = (!url.username().is_empty()).then(|| url.username().to_string());
        let password = url.password().map(|s| s.to_string());

        let auth = match (username, password) {
            (None, None) => None,
            (Some(username), Some(password)) => Some(ProxyAuth { username, password }),
            _ => return Err("Proxy auth requires both username and password".to_string()),
        };

        Ok(Self {
            scheme,
            host,
            port,
            auth,
        })
    }

    pub fn to_url_string(&self) -> String {
        let mut url = url::Url::parse(&format!(
            "{}://{}:{}/",
            self.scheme.as_str(),
            self.host,
            self.port
        ))
        .expect("Proxy::to_url_string: invalid components");

        if let Some(auth) = &self.auth {
            let _ = url.set_username(&auth.username);
            let _ = url.set_password(Some(&auth.password));
        }

        let mut out = url.to_string();
        if out.ends_with('/') {
            out.pop();
        }
        out
    }

    pub fn to_url_string_no_auth(&self) -> String {
        format!("{}://{}:{}", self.scheme.as_str(), self.host, self.port)
    }

    pub fn to_url_string_redacted(&self) -> String {
        if self.auth.is_some() {
            format!(
                "{}://{}:***@{}:{}",
                self.scheme.as_str(),
                self.auth.as_ref().unwrap().username,
                self.host,
                self.port
            )
        } else {
            self.to_url_string_no_auth()
        }
    }
}

impl std::fmt::Display for Proxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_url_string_redacted())
    }
}

// Single runtime source of truth (set by UI/persisted config at startup)
static RUNTIME_PROXY_CFG: OnceLock<Option<Proxy>> = OnceLock::new();

/// Set the runtime proxy config (intended to be called once, early at startup)
pub fn set_runtime_proxy_cfg(cfg: &Option<Proxy>) {
    RUNTIME_PROXY_CFG
        .set(cfg.clone())
        .expect("Proxy runtime already initialized (set_runtime_proxy_cfg called twice)");

    match cfg {
        Some(c) => log::info!("Runtime proxy config set: {}", c.to_url_string_redacted()),
        None => log::info!("Runtime proxy config set: direct (no proxy)"),
    }
}

pub fn runtime_proxy_cfg() -> Option<Proxy> {
    RUNTIME_PROXY_CFG
        .get()
        .expect("Proxy runtime not initialized. Call set_runtime_proxy_cfg(Some(..)|None) before any network use.")
        .clone()
}

pub fn try_apply_proxy(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    let Some(cfg) = runtime_proxy_cfg() else {
        return builder;
    };

    let proxy = match reqwest::Proxy::all(cfg.to_url_string_no_auth()) {
        Ok(p) => p,
        Err(e) => {
            log::warn!(
                "Failed to configure proxy (scheme={}): {}",
                cfg.scheme.as_str(),
                e
            );
            return builder;
        }
    };

    let proxy = match (cfg.scheme, cfg.auth.as_ref()) {
        (ProxyScheme::Http | ProxyScheme::Https, Some(auth)) => {
            proxy.basic_auth(&auth.username, &auth.password)
        }
        _ => proxy,
    };

    log::info!("Using proxy for REST: {}", cfg.to_url_string_redacted());
    builder.proxy(proxy)
}

async fn http_connect_tunnel(
    stream: &mut TcpStream,
    target_host: &str,
    target_port: u16,
    proxy_authorization: Option<&str>,
) -> Result<(), AdapterError> {
    let mut req = format!(
        "CONNECT {target_host}:{target_port} HTTP/1.1\r\nHost: {target_host}:{target_port}\r\nProxy-Connection: keep-alive\r\n"
    );

    if let Some(auth) = proxy_authorization {
        req.push_str(&format!("Proxy-Authorization: {auth}\r\n"));
    }
    req.push_str("\r\n");

    stream
        .write_all(req.as_bytes())
        .await
        .map_err(|e| AdapterError::WebsocketError(e.to_string()))?;

    let mut buf = Vec::with_capacity(1024);
    let mut tmp = [0u8; 512];
    const MAX_HDR: usize = 16 * 1024;

    loop {
        let n = stream
            .read(&mut tmp)
            .await
            .map_err(|e| AdapterError::WebsocketError(e.to_string()))?;
        if n == 0 {
            return Err(AdapterError::WebsocketError(
                "Proxy closed connection during CONNECT".to_string(),
            ));
        }
        buf.extend_from_slice(&tmp[..n]);

        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buf.len() > MAX_HDR {
            return Err(AdapterError::WebsocketError(
                "Proxy CONNECT response headers too large".to_string(),
            ));
        }
    }

    let hdr = String::from_utf8_lossy(&buf);
    let status = hdr.lines().next().unwrap_or("<no status line>");
    let ok = status.contains(" 200 ");
    if !ok {
        return Err(AdapterError::WebsocketError(format!(
            "Proxy CONNECT failed: {status}"
        )));
    }

    Ok(())
}
