//! Explicit filesystem and Keychain boundary. Config files never contain tokens.
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
pub use orbit_ipc::{Paths, Settings, atomic_private_write};
use std::fs;

pub fn validate_hub_url(raw: &str, development: bool) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(raw).context("URL de hub no válida")?;
    let local = matches!(
        url.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    );
    let tailnet = url
        .host_str()
        .is_some_and(orbit_core::orbit_bundle::plain_http_hub);
    if url.scheme() != "https" && !(url.scheme() == "http" && (development && local || tailnet)) {
        bail!("El hub requiere HTTPS; HTTP solo se permite en localhost con --dev");
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
    {
        bail!("Usa solo el origen HTTPS del hub, sin claves, rutas ni parámetros");
    }
    Ok(url)
}

pub fn storage_key(paths: &Paths, development: bool) -> Result<[u8; 32]> {
    let secret = match get_secret(paths, "storage", development)? {
        Some(secret) => secret,
        None => {
            if paths.store.exists() {
                bail!("Falta la clave de un almacén existente; recupera Keychain antes de abrirlo");
            }
            let key: [u8; 32] = rand::random();
            let secret = STANDARD.encode(key);
            set_secret(paths, "storage", &secret, development)?;
            secret
        }
    };
    STANDARD
        .decode(secret)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Clave de almacenamiento no válida"))
}

pub fn get_secret(paths: &Paths, account: &str, development: bool) -> Result<Option<String>> {
    if development {
        let path = paths.root.join(format!("development-{account}.key"));
        return match fs::read_to_string(path) {
            Ok(value) => Ok(Some(value)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        };
    }
    #[cfg(target_os = "macos")]
    {
        let entry = keyring::Entry::new(orbit_core::orbit_bundle::KEYCHAIN_SERVICE, account)?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
    #[cfg(not(target_os = "macos"))]
    bail!(
        "Keychain requiere macOS. Para fixtures aislados utiliza --dev --data-dir /ruta/de/prueba"
    )
}

pub fn set_secret(paths: &Paths, account: &str, value: &str, development: bool) -> Result<()> {
    if development {
        paths.initialize()?;
        return atomic_private_write(
            &paths.root.join(format!("development-{account}.key")),
            value.as_bytes(),
        );
    }
    #[cfg(target_os = "macos")]
    {
        keyring::Entry::new(orbit_core::orbit_bundle::KEYCHAIN_SERVICE, account)?
            .set_password(value)?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    bail!("No hay Keychain en esta plataforma; no se guardó la credencial")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn insecure_or_credential_bearing_urls_fail_closed() {
        assert!(validate_hub_url("http://example.com", true).is_err());
        assert!(validate_hub_url("http://127.0.0.1:8765", false).is_err());
        assert!(validate_hub_url("http://127.0.0.1:8765", true).is_ok());
        assert!(validate_hub_url("https://token@example.com", false).is_err());
        assert!(validate_hub_url("https://example.com/?token=x", false).is_err());
    }

    /// Plain HTTP to a tailnet address is the variant's decision
    /// (`orbit_bundle::plain_http_hub`, which keeps the published build
    /// refusing it), in development or not; anything else still needs HTTPS.
    #[test]
    fn http_on_the_tailnet_follows_the_build_variant() {
        let variant = orbit_core::orbit_bundle::plain_http_hub("100.64.0.1");
        for development in [false, true] {
            assert_eq!(
                validate_hub_url("http://100.64.0.1:8710", development).is_ok(),
                variant
            );
            assert!(validate_hub_url("http://10.0.0.1:8710", development).is_err());
            assert!(validate_hub_url("http://100.64.0.1:8710/v1", development).is_err());
        }
    }
}
