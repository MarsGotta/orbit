//! Explicit filesystem boundary and the store key. Config files never contain
//! tokens: secrets go through the given [`Secrets`].
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_hub_client::HubEndpoint;
pub use orbit_ipc::{Paths, Settings, atomic_private_write};
use std::fs;

use crate::platform::Secrets;
#[cfg(feature = "mac")]
pub use orbit_mac::{get_secret, set_secret};

pub use orbit_hub_client::validate_hub_url;

/// Where the hub is and who this device is to it, out of the settings.
pub fn hub_endpoint(settings: &Settings) -> HubEndpoint {
    HubEndpoint {
        hub_url: settings.hub_url.clone(),
        device_id: settings.device_id.clone(),
        device_name: settings.device_name.clone(),
        installation_id: None,
    }
}

/// The file in the data folder that keeps this installation's id. Not in
/// `config.toml`, which may not gain keys (Orbit 0.8 and 0.9 refuse them), and
/// not in the Keychain: it is no secret.
const INSTALLATION_ID_FILE: &str = "installation-id";

/// This installation's id: a version 4 UUID made the first time it is asked
/// for and kept in the data folder, so it stays the same across restarts,
/// refused pairings and other hubs, and only another data folder has another.
///
/// `Ok(Err(cause))`: the file is there but unreadable or holds no canonical
/// UUID. It is left as it is, and the device pairs without an id, as an Orbit
/// from before it did; the caller logs the cause. `Err`: a new id could not be
/// written.
pub fn installation_id(paths: &Paths) -> Result<Result<String>> {
    let path = paths.root.join(INSTALLATION_ID_FILE);
    match fs::read_to_string(&path) {
        Ok(text) => Ok(canonical_uuid(text.trim())
            .with_context(|| format!("{} does not hold a canonical UUID", path.display()))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let id = uuid::Uuid::new_v4().hyphenated().to_string();
            paths.initialize()?;
            atomic_private_write(&path, format!("{id}\n").as_bytes())
                .with_context(|| format!("{} could not be written", path.display()))?;
            Ok(Ok(id))
        }
        Err(error) => {
            Ok(Err(error).with_context(|| format!("{} could not be read", path.display())))
        }
    }
}

fn canonical_uuid(id: &str) -> Result<String> {
    let parsed = uuid::Uuid::parse_str(id)?;
    if parsed.hyphenated().to_string() != id {
        bail!("not written in canonical form");
    }
    Ok(id.to_owned())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Result<(tempfile::TempDir, Paths)> {
        let folder = tempfile::Builder::new()
            .prefix("orbit-installation-")
            .tempdir_in("/tmp")?;
        let paths = Paths::resolve(Some(folder.path().to_owned()))?;
        Ok((folder, paths))
    }

    #[test]
    fn an_unusable_installation_id_is_named_and_left_as_it_is() -> Result<()> {
        let (_folder, paths) = paths()?;
        let path = paths.root.join(INSTALLATION_ID_FILE);
        for kept in [
            "not-an-installation\n",
            "0A1B2C3D-4E5F-4A6B-8C7D-9E0F1A2B3C4D\n",
        ] {
            fs::write(&path, kept)?;
            let Err(cause) = installation_id(&paths)? else {
                bail!("{kept:?} was taken as an installation id");
            };
            assert!(format!("{cause:#}").contains(&path.display().to_string()));
            assert_eq!(fs::read_to_string(&path)?, kept, "the file was rewritten");
        }
        Ok(())
    }

    #[test]
    fn a_missing_installation_id_is_made_once() -> Result<()> {
        let (_folder, paths) = paths()?;
        let first = installation_id(&paths)??;
        assert_eq!(installation_id(&paths)??, first);
        assert_eq!(canonical_uuid(&first)?, first);
        Ok(())
    }
}
