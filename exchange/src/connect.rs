use crate::adapter::AdapterError;

use bytes::Bytes;
use fastwebsockets::FragmentCollector;
use http_body_util::Empty;
use hyper::{
    Request,
    header::{CONNECTION, UPGRADE},
    upgrade::Upgraded,
};
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio_rustls::{
    TlsConnector,
    rustls::{ClientConfig, OwnedTrustAnchor},
};
use url::Url;

use std::sync::LazyLock;

pub static TLS_CONNECTOR: LazyLock<TlsConnector> =
    LazyLock::new(|| tls_connector().expect("failed to create TLS connector"));

fn tls_connector() -> Result<TlsConnector, AdapterError> {
    let mut root_store = tokio_rustls::rustls::RootCertStore::empty();

    root_store.add_trust_anchors(webpki_roots::TLS_SERVER_ROOTS.0.iter().map(|ta| {
        OwnedTrustAnchor::from_subject_spki_name_constraints(
            ta.subject,
            ta.spki,
            ta.name_constraints,
        )
    }));

    let config = ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    Ok(TlsConnector::from(std::sync::Arc::new(config)))
}

pub enum State {
    Disconnected,
    Connected(FragmentCollector<TokioIo<Upgraded>>),
}

pub async fn connect_ws(
    domain: &str,
    url: &str,
) -> Result<FragmentCollector<TokioIo<Upgraded>>, AdapterError> {
    let parsed = Url::parse(url).map_err(|e| AdapterError::InvalidRequest(e.to_string()))?;

    let target_port = parsed.port_or_known_default().ok_or_else(|| {
        AdapterError::InvalidRequest("Missing port for websocket URL".to_string())
    })?;

    let stream = setup_tcp(domain, target_port).await?;

    match parsed.scheme() {
        "wss" => {
            let tls_stream = upgrade_to_tls(domain, stream).await?;
            upgrade_to_websocket(domain, tls_stream, &parsed).await
        }
        "ws" => upgrade_to_websocket(domain, stream, &parsed).await,
        _ => Err(AdapterError::InvalidRequest(
            "Invalid scheme for websocket URL".to_string(),
        )),
    }
}

async fn setup_tcp(
    domain: &str,
    target_port: u16,
) -> Result<super::proxy::ProxyStream, AdapterError> {
    if let Some(proxy) = super::proxy::runtime_proxy_cfg() {
        log::info!("Using proxy for WS: {}", proxy);
        return proxy.connect_tcp(domain, target_port).await;
    }

    let addr = format!("{domain}:{target_port}");
    let tcp = tokio::net::TcpStream::connect(&addr)
        .await
        .map_err(|e| AdapterError::WebsocketError(e.to_string()))?;

    Ok(super::proxy::ProxyStream::Plain(tcp))
}

async fn upgrade_to_tls<S>(
    domain: &str,
    stream: S,
) -> Result<tokio_rustls::client::TlsStream<S>, AdapterError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let domain: tokio_rustls::rustls::ServerName =
        tokio_rustls::rustls::ServerName::try_from(domain)
            .map_err(|_| AdapterError::ParseError("invalid dnsname".to_string()))?;

    TLS_CONNECTOR
        .connect(domain, stream)
        .await
        .map_err(|e| AdapterError::WebsocketError(e.to_string()))
}

async fn upgrade_to_websocket<S>(
    domain: &str,
    stream: S,
    parsed: &Url,
) -> Result<FragmentCollector<TokioIo<Upgraded>>, AdapterError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let mut path_and_query = parsed.path().to_string();
    if let Some(q) = parsed.query() {
        path_and_query.push('?');
        path_and_query.push_str(q);
    }
    if path_and_query.is_empty() {
        path_and_query.push('/');
    }

    let req: Request<Empty<Bytes>> = Request::builder()
        .method("GET")
        .uri(path_and_query)
        .header("Host", domain)
        .header(UPGRADE, "websocket")
        .header(CONNECTION, "upgrade")
        .header(
            "Sec-WebSocket-Key",
            fastwebsockets::handshake::generate_key(),
        )
        .header("Sec-WebSocket-Version", "13")
        .body(Empty::<Bytes>::new())
        .map_err(|e| AdapterError::WebsocketError(e.to_string()))?;

    let exec = TokioExecutor::new();
    let (ws, _) = fastwebsockets::handshake::client(&exec, req, stream)
        .await
        .map_err(|e| AdapterError::WebsocketError(e.to_string()))?;

    Ok(FragmentCollector::new(ws))
}
