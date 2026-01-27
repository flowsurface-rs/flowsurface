use crate::adapter::AdapterError;

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use url::Url;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use std::time::Duration;

pub enum Proxy {
    Http {
        host: String,
        port: u16,
        username: Option<String>,
        password: Option<String>,
    },
    Socks5 {
        host: String,
        port: u16,
        username: Option<String>,
        password: Option<String>,
    },
}

impl Proxy {
    fn from_url(url: &Url) -> Result<Self, AdapterError> {
        let scheme = url.scheme().to_ascii_lowercase();
        let host = url
            .host_str()
            .ok_or_else(|| AdapterError::ParseError("Proxy host missing".to_string()))?
            .to_string();
        let port = url
            .port_or_known_default()
            .ok_or_else(|| AdapterError::ParseError("Proxy port missing".to_string()))?;

        let username = (!url.username().is_empty()).then(|| url.username().to_string());
        let password = url.password().map(|s| s.to_string());

        match scheme.as_str() {
            "http" | "https" => Ok(Proxy::Http {
                host,
                port,
                username,
                password,
            }),
            // Treat socks5h as socks5 here; tokio-socks will send the domain name to the proxy
            "socks5" | "socks5h" => Ok(Proxy::Socks5 {
                host,
                port,
                username,
                password,
            }),
            _ => Err(AdapterError::ParseError(format!(
                "Unsupported proxy scheme: {scheme} (use http://, https://, socks5://, socks5h://)"
            ))),
        }
    }

    pub async fn connect_tcp(
        &self,
        target_host: &str,
        target_port: u16,
    ) -> Result<TcpStream, AdapterError> {
        match self {
            Proxy::Http {
                host,
                port,
                username,
                password,
            } => {
                let proxy_addr = format!("{host}:{port}");

                let mut stream = TcpStream::connect(&proxy_addr)
                    .await
                    .map_err(|e| AdapterError::WebsocketError(e.to_string()))?;

                let proxy_auth = http_basic_proxy_auth(username.as_deref(), password.as_deref())?;
                http_connect_tunnel(&mut stream, target_host, target_port, proxy_auth.as_deref())
                    .await?;

                Ok(stream)
            }
            Proxy::Socks5 {
                host,
                port,
                username,
                password,
            } => {
                let proxy_addr = (host.as_str(), *port);
                let target_addr = (target_host, target_port);

                let stream = match (username.as_deref(), password.as_deref()) {
                    (Some(u), Some(p)) => tokio_socks::tcp::Socks5Stream::connect_with_password(
                        proxy_addr,
                        target_addr,
                        u,
                        p,
                    )
                    .await
                    .map_err(|e| AdapterError::WebsocketError(e.to_string()))?
                    .into_inner(),

                    (None, None) => {
                        tokio_socks::tcp::Socks5Stream::connect(proxy_addr, target_addr)
                            .await
                            .map_err(|e| AdapterError::WebsocketError(e.to_string()))?
                            .into_inner()
                    }

                    _ => {
                        return Err(AdapterError::ParseError(
                            "SOCKS5 proxy auth requires both username and password".to_string(),
                        ));
                    }
                };

                Ok(stream)
            }
        }
    }
}

impl std::fmt::Display for Proxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Proxy::Http { host, port, .. } => write!(f, "http://{host}:{port}"),
            Proxy::Socks5 { host, port, .. } => write!(f, "socks5://{host}:{port}"),
        }
    }
}

pub fn proxy_from_env() -> Option<Proxy> {
    let s = std::env::var("FLOWSURFACE_PROXY")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| std::env::var("HTTPS_PROXY").ok())
        .or_else(|| std::env::var("HTTP_PROXY").ok())
        .filter(|s| !s.trim().is_empty())?;

    let url = Url::parse(&s).ok()?;
    Proxy::from_url(&url).ok()
}

fn http_basic_proxy_auth(
    username: Option<&str>,
    password: Option<&str>,
) -> Result<Option<String>, AdapterError> {
    match (username, password) {
        (None, None) => Ok(None),
        (Some(u), Some(p)) => {
            let token = BASE64.encode(format!("{u}:{p}"));
            Ok(Some(format!("Basic {token}")))
        }
        _ => Err(AdapterError::ParseError(
            "HTTP proxy auth requires both username and password".to_string(),
        )),
    }
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

pub async fn proxy_smoke_test() -> Result<(), AdapterError> {
    // Don't log credentials
    let proxy = std::env::var("FLOWSURFACE_PROXY").ok();
    log::info!(
        "Proxy smoke test starting. FLOWSURFACE_PROXY set={}",
        proxy.is_some()
    );

    // 1) REST (HTTPS)
    let rest_url = "https://api.binance.com/api/v3/ping";
    let rest_res = tokio::time::timeout(
        Duration::from_secs(10),
        crate::limiter::http_request(rest_url, None, None),
    )
    .await;

    match rest_res {
        Ok(Ok(_body)) => log::info!("Proxy smoke test REST OK: {}", rest_url),
        Ok(Err(e)) => return Err(e),
        Err(_) => {
            return Err(AdapterError::InvalidRequest(format!(
                "Proxy smoke test REST TIMEOUT (10s): {rest_url}"
            )));
        }
    }

    // 2) WebSocket (Binance uses :9443)
    let ws_domain = "stream.binance.com";
    let ws_url = "wss://stream.binance.com:9443/ws/btcusdt@trade";
    let ws_res = tokio::time::timeout(
        Duration::from_secs(10),
        crate::connect::connect_ws(ws_domain, ws_url),
    )
    .await;

    match ws_res {
        Ok(Ok(_ws)) => log::info!("Proxy smoke test WS OK: {}", ws_url),
        Ok(Err(e)) => return Err(e),
        Err(_) => {
            return Err(AdapterError::WebsocketError(format!(
                "Proxy smoke test WS TIMEOUT (10s): {ws_url}"
            )));
        }
    }

    Ok(())
}
