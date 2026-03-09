//! Minimal Telegram Bot API client for flowsurface telemetry alerts.
//!
//! Reads `FLOWSURFACE_TG_BOT_TOKEN` and `FLOWSURFACE_TG_CHAT_ID` from env.
//! If either is unset, all sends silently no-op (guard-by-default).

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use reqwest::Client;

static BOT_TOKEN: LazyLock<Option<String>> =
    LazyLock::new(|| std::env::var("FLOWSURFACE_TG_BOT_TOKEN").ok());

static CHAT_ID: LazyLock<Option<String>> =
    LazyLock::new(|| std::env::var("FLOWSURFACE_TG_CHAT_ID").ok());

static HTTP: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("telegram http client")
});

/// Returns true if Telegram alerting is configured.
pub fn is_configured() -> bool {
    BOT_TOKEN.is_some() && CHAT_ID.is_some()
}

/// Send a plain-text alert. No-ops if not configured.
pub async fn send_alert(message: &str) {
    let (Some(token), Some(chat_id)) = (BOT_TOKEN.as_deref(), CHAT_ID.as_deref()) else {
        return;
    };

    let url = format!("https://api.telegram.org/bot{token}/sendMessage");

    match HTTP
        .post(&url)
        .form(&[
            ("chat_id", chat_id),
            ("text", message),
            ("parse_mode", "HTML"),
        ])
        .send()
        .await
    {
        Ok(resp) if !resp.status().is_success() => {
            log::warn!(
                "[telegram] send failed: HTTP {} — {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            );
        }
        Err(e) => {
            log::warn!("[telegram] send error: {e}");
        }
        _ => {}
    }
}

/// Send a formatted alert with a severity prefix.
pub async fn alert(severity: Severity, component: &str, detail: &str) {
    let icon = match severity {
        Severity::Critical => "🔴",
        Severity::Warning => "⚠️",
        Severity::Info => "ℹ️",
        Severity::Recovery => "🟢",
    };

    let msg = format!(
        "{icon} <b>flowsurface — {component}</b>\n{detail}",
    );
    send_alert(&msg).await;
}

/// Blocking send for use in panic hooks and other sync contexts.
/// Creates a one-shot tokio runtime — do NOT call from within an async runtime.
pub fn send_alert_blocking(message: &str) {
    let (Some(token), Some(chat_id)) = (BOT_TOKEN.as_deref(), CHAT_ID.as_deref()) else {
        return;
    };

    let url = format!("https://api.telegram.org/bot{token}/sendMessage");
    let msg = message.to_string();
    let chat = chat_id.to_string();

    // Best-effort: spawn a thread with a blocking reqwest client to avoid
    // interfering with any existing tokio runtime (panic hooks are tricky).
    let _ = std::thread::Builder::new()
        .name("tg-panic-alert".into())
        .spawn(move || {
            let client = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build();
            if let Ok(client) = client {
                let _ = client
                    .post(&url)
                    .form(&[
                        ("chat_id", chat.as_str()),
                        ("text", msg.as_str()),
                        ("parse_mode", "HTML"),
                    ])
                    .send();
            }
        })
        .and_then(|h| h.join().map_err(|_| std::io::Error::other("join failed")));
}

/// Level 2 startup guard: probe ClickHouse and SSE sidecar, then send
/// a startup alert with connectivity status. Call once during app init.
pub async fn startup_health_check() {
    if !is_configured() {
        return;
    }

    let ch_host = std::env::var("FLOWSURFACE_CH_HOST").unwrap_or_else(|_| "localhost".into());
    let ch_port = std::env::var("FLOWSURFACE_CH_PORT").unwrap_or_else(|_| "18123".into());
    let sse_host = std::env::var("FLOWSURFACE_SSE_HOST").unwrap_or_else(|_| "localhost".into());
    let sse_port = std::env::var("FLOWSURFACE_SSE_PORT").unwrap_or_else(|_| "18081".into());

    let probe = Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_else(|_| HTTP.clone());

    let ch_ok = probe
        .get(format!("http://{ch_host}:{ch_port}/"))
        .send()
        .await
        .is_ok();
    let sse_ok = probe
        .get(format!("http://{sse_host}:{sse_port}/health"))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);

    let mut lines = vec!["App launched".to_string()];
    if ch_ok {
        lines.push("CH: ✓".into());
    } else {
        lines.push(format!("CH: ✗ UNREACHABLE ({ch_host}:{ch_port})"));
    }
    if sse_ok {
        lines.push("SSE: ✓".into());
    } else {
        lines.push("SSE: ✗ (may be disabled)".into());
    }

    let severity = if !ch_ok {
        Severity::Critical
    } else {
        Severity::Info
    };
    alert(severity, "startup", &lines.join("\n")).await;
}

#[derive(Debug, Clone, Copy)]
pub enum Severity {
    Critical,
    Warning,
    Info,
    Recovery,
}

static COOLDOWNS: LazyLock<Mutex<HashMap<String, std::time::Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Returns true if enough time has passed since last alert for this key.
/// Use for high-frequency paths (WS timeouts, rate limits) to prevent spam.
pub fn should_alert(key: &str, cooldown_secs: u64) -> bool {
    let mut map = COOLDOWNS.lock().unwrap_or_else(|e| e.into_inner());
    let now = std::time::Instant::now();
    if let Some(last) = map.get(key)
        && now.duration_since(*last) < std::time::Duration::from_secs(cooldown_secs)
    {
        return false;
    }
    map.insert(key.to_string(), now);
    true
}

/// Fire-and-forget Telegram alert macro. Use from any async or sync context.
/// Expands to tokio::spawn + alert(). No-ops if Telegram not configured.
///
/// Usage: `tg_alert!(Severity::Warning, "clickhouse", "HTTP {}: {}", status, body)`
#[macro_export]
macro_rules! tg_alert {
    ($sev:expr, $comp:expr, $($arg:tt)*) => {
        if $crate::telegram::is_configured() {
            let msg = format!($($arg)*);
            let sev = $sev;
            let comp: &'static str = $comp;
            tokio::spawn(async move {
                $crate::telegram::alert(sev, comp, &msg).await;
            });
        }
    };
}
