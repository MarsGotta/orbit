//! Explicit filesystem boundary and the store key. Config files never contain
//! tokens: secrets go through the given [`Secrets`].
use anyhow::{Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_hub_client::HubEndpoint;
pub use orbit_ipc::{Paths, Settings, atomic_private_write};

use crate::platform::Secrets;
pub use crate::platform::system::{get_secret, set_secret};

pub use orbit_hub_client::validate_hub_url;

/// Where the hub is and who this device is to it, out of the settings.
pub fn hub_endpoint(settings: &Settings) -> HubEndpoint {
    HubEndpoint {
        hub_url: settings.hub_url.clone(),
        device_id: settings.device_id.clone(),
        device_name: settings.device_name.clone(),
    }
}

pub fn storage_key(paths: &Paths, secrets: &dyn Secrets) -> Result<[u8; 32]> {
    let secret = match secrets.get("storage")? {
        Some(secret) => secret,
        None => {
            if paths.store.exists() {
                bail!("An existing store key is missing; recover Keychain before opening it");
            }
            let key: [u8; 32] = rand::random();
            let secret = STANDARD.encode(key);
            secrets.set("storage", &secret)?;
            secret
        }
    };
    STANDARD
        .decode(secret)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Invalid storage key"))
}
