use exchange::proxy::{Proxy, ProxyAuth};

const KEYCHAIN_SERVICE: &str = "flowsurface.proxy";

fn ensure_default_store() -> Result<(), keyring_core::Error> {
    if keyring_core::get_default_store().is_some() {
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    let store = apple_native_keyring_store::keychain::Store::new()?;

    #[cfg(target_os = "windows")]
    let store = windows_native_keyring_store::Store::new()?;

    #[cfg(target_os = "linux")]
    let store = dbus_secret_service_keyring_store::Store::new()?;

    keyring_core::set_default_store(store);
    Ok(())
}

fn entry_for(proxy: &Proxy) -> Result<keyring_core::Entry, keyring_core::Error> {
    ensure_default_store()?;
    let key = proxy.to_url_string_no_auth();
    keyring_core::Entry::new(KEYCHAIN_SERVICE, &key)
}

pub fn load_proxy_auth(proxy: &Proxy) -> Option<ProxyAuth> {
    let key = proxy.to_url_string_no_auth();

    let entry = match entry_for(proxy) {
        Ok(e) => e,
        Err(err) => {
            log::warn!(
                "Keychain entry init failed for service={KEYCHAIN_SERVICE} key={key}: {err}"
            );
            return None;
        }
    };

    let secret = match entry.get_password() {
        Ok(s) => s,
        Err(err) => {
            log::info!("No proxy auth in keychain for service={KEYCHAIN_SERVICE} key={key}: {err}");
            return None;
        }
    };

    match serde_json::from_str::<ProxyAuth>(&secret) {
        Ok(auth) => Some(auth),
        Err(err) => {
            log::warn!(
                "Proxy auth in keychain is invalid JSON for service={KEYCHAIN_SERVICE} key={key}: {err}"
            );
            None
        }
    }
}

pub fn save_proxy_auth(proxy: &Proxy) {
    let key = proxy.to_url_string_no_auth();

    let Some(auth) = proxy.auth() else {
        log::info!("Not saving proxy auth: auth is None (service={KEYCHAIN_SERVICE} key={key})");
        return;
    };

    let entry = match entry_for(proxy) {
        Ok(e) => e,
        Err(err) => {
            log::warn!(
                "Keychain entry init failed for service={KEYCHAIN_SERVICE} key={key}: {err}"
            );
            return;
        }
    };

    let secret = match serde_json::to_string(auth) {
        Ok(s) => s,
        Err(err) => {
            log::warn!(
                "Failed to serialize proxy auth for service={KEYCHAIN_SERVICE} key={key}: {err}"
            );
            return;
        }
    };

    match entry.set_password(&secret) {
        Ok(()) => {
            log::info!("Stored proxy auth in keychain (service={KEYCHAIN_SERVICE} key={key})")
        }
        Err(err) => {
            log::warn!(
                "Failed to store proxy auth in keychain (service={KEYCHAIN_SERVICE} key={key}): {err}"
            );
            return;
        }
    }

    match entry.get_password() {
        Ok(roundtrip) => {
            if roundtrip == secret {
                log::info!("Keychain roundtrip OK (service={KEYCHAIN_SERVICE} key={key})");
            } else {
                log::warn!("Keychain roundtrip MISMATCH (service={KEYCHAIN_SERVICE} key={key})");
            }
        }
        Err(err) => log::warn!(
            "Keychain roundtrip read FAILED (service={KEYCHAIN_SERVICE} key={key}): {err}"
        ),
    }
}
