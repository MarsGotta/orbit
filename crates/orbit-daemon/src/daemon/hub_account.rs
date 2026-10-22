//! App-facing daemon operations that own credentials, configuration and hub reads.

use super::{Runtime, account_summary};
use anyhow::{Result, bail};
use orbit_core::Connection;
use orbit_core::limits::MAX_SESSION_ID_BYTES;
use orbit_core::notice::NoticeCode;

const MAX_DEVICE_NAME_CHARS: usize = 80;
const MAX_PAIR_CODE_BYTES: usize = 256;

impl Runtime {
    pub(super) fn hub_access(&self) -> Result<&str, orbit_core::refusal::Refusal> {
        crate::hub::access(
            &self.settings,
            self.token.as_deref(),
            self.state.privacy_stopped(),
            self.state.connection(),
        )
    }

    pub(super) async fn pair_device(
        &mut self,
        hub_url: String,
        code: String,
        device_name: String,
    ) -> Result<()> {
        let result = self.pair_device_inner(hub_url, code, device_name).await;
        if let Err(error) = &result {
            self.say_failure(NoticeCode::PairingFailed, error);
        }
        result
    }

    async fn pair_device_inner(
        &mut self,
        hub_url: String,
        code: String,
        device_name: String,
    ) -> Result<()> {
        if code.is_empty() || code.len() > MAX_PAIR_CODE_BYTES {
            bail!("Código no válido");
        }
        let device_name = normalized_device_name(&device_name)?;
        let hub_url = crate::config::validate_hub_url(&hub_url, self.development)?.to_string();
        let mut settings = self.paths.load_settings()?;
        settings.hub_url = Some(hub_url);
        settings.device_name = device_name;
        let (device_id, token) =
            crate::hub::pair(&settings, &code, self.development, self.op()).await?;
        // Retire the old conversation durably before installing another identity.
        self.apply(orbit_core::Event::DeviceChanged).await?;
        crate::config::set_secret(&self.paths, "device", &token, self.development)?;
        settings.device_id = Some(device_id);
        self.paths.save_settings(&settings)?;
        self.settings = settings;
        self.state.views.account = Some(account_summary(&self.settings));
        self.token = Some(token);
        self.restart_hub_connection().await?;
        self.say(NoticeCode::Paired);
        Ok(())
    }

    async fn restart_hub_connection(&mut self) -> Result<()> {
        self.snapshot_revision = None;
        if self.state.connection() != Connection::Offline {
            self.apply(orbit_core::Event::Connect(Connection::Offline))
                .await?;
        }
        self.connect();
        Ok(())
    }

    pub(super) async fn open_session(
        &mut self,
        session_id: String,
        agent_id: Option<String>,
    ) -> Result<()> {
        if session_id.is_empty() || session_id.len() > MAX_SESSION_ID_BYTES {
            bail!(
                "Identificador de sesión de {} bytes; se admiten entre 1 y {MAX_SESSION_ID_BYTES} bytes",
                session_id.len()
            );
        }
        if session_id.chars().any(char::is_control) {
            bail!(
                "Identificador de sesión de {} bytes con carácter de control; máximo {MAX_SESSION_ID_BYTES} bytes",
                session_id.len()
            );
        }
        self.ensure_known_agent(agent_id.as_deref())?;
        self.ensure_conversation_can_change()?;
        // The engine is not the conversation's. It stays connected,
        // with its catalogue, whichever conversation is open.
        self.apply(orbit_core::Event::OpenSession {
            id: session_id,
            agent_id,
        })
        .await?;
        self.say(NoticeCode::SessionOpened);
        Ok(())
    }
}

fn normalized_device_name(raw: &str) -> Result<String> {
    let name = raw.trim();
    if name.is_empty()
        || name.chars().count() > MAX_DEVICE_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        bail!("Nombre de dispositivo no válido");
    }
    Ok(name.to_owned())
}

#[cfg(test)]
mod pairing_tests {
    use super::*;
    use crate::daemon::audio::tests::fixture;

    #[tokio::test]
    async fn pairing_after_revocation_lets_the_user_rearm() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _native) = fixture(directory.path())?;
        {
            let fixture_value = serde_json::json!(Connection::Revoked);
            crate::daemon::authority_fixture::set(&mut runtime.state, "connection", fixture_value);
        }
        {
            let fixture_value = serde_json::json!(true);
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "privacy_stopped",
                fixture_value,
            );
        }

        // Pair's successful local tail uses this path after a synthetic hub
        // has supplied the new device credential; no external hub is needed.
        runtime.restart_hub_connection().await?;
        assert_eq!(runtime.state.connection(), Connection::Offline);
        runtime.apply(orbit_core::Event::LocalRearm).await?;
        assert!(!runtime.state.privacy_stopped());
        Ok(())
    }
}
