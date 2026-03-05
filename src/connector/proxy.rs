use exchange::proxy::Proxy;
use std::sync::OnceLock;

static RUNTIME_PROXY_CFG: OnceLock<Option<Proxy>> = OnceLock::new();

pub fn set_runtime_proxy_cfg(cfg: Option<Proxy>) {
    if let Some(existing) = RUNTIME_PROXY_CFG.get() {
        if existing != &cfg {
            log::debug!(
                "Attempted to re-set runtime proxy config (ignored). existing={}, requested={}",
                existing
                    .as_ref()
                    .map(|p| p.to_log_string())
                    .unwrap_or_else(|| "direct (no proxy)".to_string()),
                cfg.as_ref()
                    .map(|p| p.to_log_string())
                    .unwrap_or_else(|| "direct (no proxy)".to_string()),
            );
        }
        return;
    }

    RUNTIME_PROXY_CFG
        .set(cfg.clone())
        .expect("Proxy runtime already initialized (set_runtime_proxy_cfg called twice)");

    match cfg {
        Some(c) => log::debug!("Runtime proxy config set: {}", c.to_log_string()),
        None => log::debug!("Runtime proxy config set: direct (no proxy)"),
    }
}

pub fn runtime_proxy_cfg() -> Option<Proxy> {
    match RUNTIME_PROXY_CFG.get() {
        Some(cfg) => cfg.clone(),
        None => {
            log::warn!("Proxy runtime not initialized yet; defaulting to direct (no proxy).");
            None
        }
    }
}
