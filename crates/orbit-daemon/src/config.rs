//! Explicit filesystem and Keychain boundary. Config files never contain tokens.
use anyhow::{Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_hub_client::HubEndpoint;
pub use orbit_ipc::{Paths, Settings, atomic_private_write};
use std::fs;

pub use orbit_hub_client::validate_hub_url;

/// Where the hub is and who this device is to it, out of the settings.
pub fn hub_endpoint(settings: &Settings) -> HubEndpoint {
    HubEndpoint {
        hub_url: settings.hub_url.clone(),
        device_id: settings.device_id.clone(),
        device_name: settings.device_name.clone(),
    }
}

pub fn storage_key(paths: &Paths, development: bool) -> Result<[u8; 32]> {
    let secret = match get_secret(paths, "storage", development)? {
        Some(secret) => secret,
        None => {
            if paths.store.exists() {
                bail!("An existing store key is missing; recover Keychain before opening it");
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
        .map_err(|_| anyhow::anyhow!("Invalid storage key"))
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
    bail!("Keychain is unavailable on this platform; the credential was not saved")
}
