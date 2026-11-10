//! The client of the hub: the device's WebSocket and the HTTP calls made with
//! its credential. It knows where the hub is and who the device is to it,
//! nothing else of the Mac.

// The crate is also compiled with cfg(test) by --all-targets; fixtures may
// inspect foreign enums with catch-all arms. Production always gets the lint.
#![cfg_attr(not(test), deny(clippy::wildcard_enum_match_arm))]

use anyhow::{Context, Result, bail};

pub mod hub;
pub mod hub_agents;
pub mod hub_notes;
pub mod hub_plugin;
pub mod hub_vault;
pub mod speech;

/// Where the hub is and who this device is to it.
#[derive(Clone, Default)]
pub struct HubEndpoint {
    pub hub_url: Option<String>,
    pub device_id: Option<String>,
    pub device_name: String,
    /// This Orbit installation's own id, said only when pairing; every other
    /// call leaves it out.
    pub installation_id: Option<String>,
}

pub fn validate_hub_url(raw: &str, development: bool) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(raw).context("Invalid hub URL")?;
    let local = matches!(
        url.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    );
    let tailnet = url
        .host_str()
        .is_some_and(orbit_core::orbit_bundle::plain_http_hub);
    if url.scheme() != "https" && !(url.scheme() == "http" && (development && local || tailnet)) {
        bail!("The hub requires HTTPS; HTTP is allowed only on localhost with --dev");
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
    {
        bail!("Use only the hub HTTPS origin, without credentials, paths or parameters");
    }
    Ok(url)
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
