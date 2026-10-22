use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::PathBuf,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u8,
    pub device_name: String,
    /// Last name learned from the hub, retained across offline starts in
    /// `hub-agents.toml` ([`HubAgents`]). `config.toml` still *accepts*
    /// the key, which 0.9.0 wrote there, and is read from it only until that
    /// file exists; it never writes it, so older versions keep loading it.
    #[serde(default, skip_serializing)]
    pub assistant_name: Option<String>,
    pub hub_url: Option<String>,
    pub device_id: Option<String>,
    pub helper_path: Option<PathBuf>,
    pub speak_responses: bool,
    /// Accepted for config.toml files written before source persistence retired.
    #[serde(default, rename = "max_source_intents", skip_serializing)]
    pub retired_max_source_intents: u64,
    #[serde(default, rename = "max_source_bytes", skip_serializing)]
    pub retired_max_source_bytes: u64,
    /// «Enviar informes de errores anónimos»: off by default, and a
    /// `config.toml` written before the switch existed loads as off.
    #[serde(default)]
    pub error_reports: bool,
    /// «Ver mi pantalla» is gone. Deimos sees the screen because macOS
    /// granted Screen Recording, not because a key in a file says so, and
    /// nothing reads this. Like `calendar_sharing` below, the key is still
    /// *accepted* — `deny_unknown_fields` would otherwise refuse every
    /// `config.toml` the previous version wrote, the user's included — and it is
    /// dropped the next time the settings are saved.
    #[serde(default, rename = "remote_observation", skip_serializing)]
    pub retired_remote_observation: bool,
    /// «Permitir control» is gone; macOS Accessibility and Screen
    /// Recording decide. Accepted, never read, dropped on the next save.
    #[serde(default, rename = "remote_action", skip_serializing)]
    pub retired_remote_action: bool,
    /// There is no «compartir agenda» switch any more. The copy
    /// leaves the Mac because macOS granted the calendar permission, not
    /// because a key in a file says so, and nothing reads this. The key is
    /// still *accepted* — `deny_unknown_fields` would otherwise refuse every
    /// `config.toml` written before this release, the user's included — and it is
    /// dropped the next time the settings are saved.
    #[serde(default, rename = "calendar_sharing", skip_serializing)]
    pub retired_calendar_sharing: bool,
    /// «Reuniones» is gone; Deimos sees the meetings always.
    /// Accepted, never read, dropped on the next save.
    #[serde(default, rename = "meeting_sharing", skip_serializing)]
    pub retired_meeting_sharing: bool,
    /// The separate live-voice engine and its early synthesis are
    /// gone; the call always uses the local detector. Both keys are accepted,
    /// never read, and dropped on the next save.
    #[serde(default, rename = "voice_live", skip_serializing)]
    pub retired_voice_live: bool,
    #[serde(default, rename = "voice_live_prefetch", skip_serializing)]
    pub retired_voice_live_prefetch: bool,
    /// The last list of agents the hub gave in `ready`, kept for starting
    /// without it, in `hub-agents.toml` and never in `config.toml`.
    /// `None`: the hub names no agents (0.9.0), and has one, called by
    /// `assistant_name`.
    #[serde(skip)]
    pub hub_agents: Option<orbit_core::agents::AgentRoster>,
}

/// What the Mac learned from the hub, kept beside `config.toml` in
/// `hub-agents.toml`. Its own file because `config.toml` must stay loadable
/// by older versions, whose settings refuse keys they do not know. A cache:
/// one that cannot be read is what was not learned yet, and the next
/// `ready` writes it again. Unknown keys are accepted for the same reason.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HubAgents {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_name: Option<String>,
    /// Last, because TOML writes tables after plain keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agents: Option<orbit_core::agents::AgentRoster>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            device_name: "Mi Mac".into(),
            assistant_name: None,
            hub_url: None,
            device_id: None,
            helper_path: None,
            speak_responses: false,
            retired_max_source_intents: 0,
            retired_max_source_bytes: 0,
            error_reports: false,
            retired_remote_observation: false,
            retired_remote_action: false,
            retired_calendar_sharing: false,
            retired_meeting_sharing: false,
            retired_voice_live: false,
            retired_voice_live_prefetch: false,
            hub_agents: None,
        }
    }
}

impl Settings {
    pub fn assistant_name(&self) -> &str {
        self.assistant_name.as_deref().unwrap_or("Deimos")
    }

    /// The agents a conversation can be with: the hub's last list, or the
    /// one agent of a hub that names none, called by `assistant_name`.
    pub fn roster(&self) -> orbit_core::agents::AgentRoster {
        self.hub_agents
            .clone()
            .and_then(orbit_core::agents::AgentRoster::checked)
            .unwrap_or_else(|| orbit_core::agents::AgentRoster::single(self.assistant_name()))
    }
}

#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub socket: PathBuf,
    pub settings: PathBuf,
    pub store: PathBuf,
}

impl Paths {
    pub fn resolve(override_path: Option<PathBuf>) -> Result<Self> {
        let root = match override_path {
            Some(path) => path,
            None => {
                let [qualifier, organization, application] =
                    orbit_core::orbit_bundle::DATA_DIRECTORY;
                directories::ProjectDirs::from(qualifier, organization, application)
                    .context("No se pudo localizar el directorio de usuario")?
                    .data_local_dir()
                    .to_owned()
            }
        };
        if !root.is_absolute() {
            bail!("--data-dir debe ser una ruta absoluta");
        }
        let socket = root.join("orbit.sock");
        if socket.as_os_str().len() > 100 {
            bail!("La ruta de datos es demasiado larga para el socket local (máximo 100 bytes)");
        }
        Ok(Self {
            settings: root.join("config.toml"),
            store: root.join("store"),
            socket,
            root,
        })
    }

    pub fn initialize(&self) -> Result<()> {
        fs::create_dir_all(&self.root)?;
        let metadata = fs::symlink_metadata(&self.root)?;
        if metadata.file_type().is_symlink() {
            bail!("El directorio de datos no puede ser un enlace simbólico");
        }
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != rustix::process::geteuid().as_raw() {
            bail!("El directorio de datos pertenece a otro usuario");
        }
        fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))?;
        Ok(())
    }

    /// `hub-agents.toml`, beside `config.toml`.
    pub fn hub_agents(&self) -> PathBuf {
        self.settings.with_file_name("hub-agents.toml")
    }

    pub fn load_settings(&self) -> Result<Settings> {
        let mut settings = if self.settings.exists() {
            let settings: Settings = toml::from_str(&fs::read_to_string(&self.settings)?)
                .context("config.toml no es válido; los datos almacenados no se han alterado")?;
            if settings.version != 1 {
                bail!("Versión de configuración no compatible");
            }
            settings
        } else {
            Settings::default()
        };
        // Once the file exists it is the only source; before, a name
        // config.toml holds is 0.9.0's, and moves there on the next save.
        if self.hub_agents().exists() {
            let learned = fs::read_to_string(self.hub_agents())
                .ok()
                .and_then(|text| toml::from_str::<HubAgents>(&text).ok())
                .unwrap_or_default();
            settings.assistant_name = learned.assistant_name;
            settings.hub_agents = learned.agents;
        }
        Ok(settings)
    }

    /// Writes what the hub taught first, so a save cut short between the
    /// two files never loses a name that was only in `config.toml`.
    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        self.initialize()?;
        if settings.assistant_name.is_some()
            || settings.hub_agents.is_some()
            || self.hub_agents().exists()
        {
            let learned = HubAgents {
                assistant_name: settings.assistant_name.clone(),
                agents: settings.hub_agents.clone(),
            };
            atomic_private_write(
                &self.hub_agents(),
                toml::to_string_pretty(&learned)?.as_bytes(),
            )?;
        }
        atomic_private_write(&self.settings, toml::to_string_pretty(settings)?.as_bytes())
    }
}

pub fn atomic_private_write(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Ruta sin directorio padre")?;
    let temp = parent.join(format!(".orbit-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&temp, path)?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every key a `config.toml` written by this build may hold. Orbit
    /// 0.8.0's and 0.9.0's `Settings` refuse unknown keys
    /// (`deny_unknown_fields`), so going back one or two versions works
    /// only while this list stays inside what both accept. What the Mac
    /// learns from the hub goes to `hub-agents.toml` instead.
    const WRITTEN_BY_THIS_BUILD: &[&str] = &[
        "version",
        "device_name",
        "hub_url",
        "device_id",
        "helper_path",
        "speak_responses",
        "error_reports",
    ];
    /// The keys Orbit 0.8.0's `Settings` accepts (read from its source,
    /// `crates/orbit-ipc/src/paths.rs`); 0.9.0 accepts these and
    /// `assistant_name`.
    const ACCEPTED_BY_0_8_0: &[&str] = &[
        "version",
        "device_name",
        "hub_url",
        "device_id",
        "helper_path",
        "speak_responses",
        "max_source_intents",
        "max_source_bytes",
        "error_reports",
        "remote_observation",
        "remote_action",
        "calendar_sharing",
        "meeting_sharing",
        "voice_live",
        "voice_live_prefetch",
    ];

    fn agents() -> orbit_core::agents::AgentRoster {
        orbit_core::agents::AgentRoster::admit(
            vec![
                orbit_core::agents::Agent::named("deimos", "Deimos"),
                orbit_core::agents::Agent::named("atlas", "Atlas"),
            ],
            Some("atlas".into()),
        )
        .unwrap_or_else(|refusal| panic!("{refusal:?}"))
    }

    #[test]
    fn a_config_written_by_this_build_loads_in_0_8_0_and_0_9_0() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let paths = Paths::resolve(Some(directory.path().to_owned()))?;
        let settings = Settings {
            assistant_name: Some("Atlas".into()),
            hub_agents: Some(agents()),
            hub_url: Some("https://hub.example".into()),
            device_id: Some("device-1".into()),
            helper_path: Some("/Applications/Orbit.app/Contents/MacOS/OrbitNative".into()),
            error_reports: true,
            ..Settings::default()
        };
        paths.save_settings(&settings)?;
        let written: toml::Table = toml::from_str(&fs::read_to_string(&paths.settings)?)?;
        let keys = written.keys().map(String::as_str).collect::<Vec<_>>();
        let mut expected = WRITTEN_BY_THIS_BUILD.to_vec();
        expected.sort_unstable();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, expected, "{written}");
        for key in keys {
            assert!(ACCEPTED_BY_0_8_0.contains(&key), "0.8.0 refuses `{key}`");
        }
        Ok(())
    }

    #[test]
    fn what_the_hub_taught_is_kept_in_its_own_file() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let paths = Paths::resolve(Some(directory.path().to_owned()))?;
        let settings = Settings {
            assistant_name: Some("Atlas".into()),
            hub_agents: Some(agents()),
            ..Settings::default()
        };
        paths.save_settings(&settings)?;
        let learned = fs::read_to_string(directory.path().join("hub-agents.toml"))?;
        assert!(learned.contains("Atlas"), "{learned}");
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(directory.path().join("hub-agents.toml"))?
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let loaded = paths.load_settings()?;
        assert_eq!(loaded.assistant_name.as_deref(), Some("Atlas"));
        assert_eq!(loaded.hub_agents, Some(agents()));
        Ok(())
    }

    /// A `config.toml` 0.9.0 wrote holds the name it learned: it is read
    /// until the new file exists, and moves there on the next save.
    #[test]
    fn the_name_a_0_9_0_config_holds_moves_to_its_own_file() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let paths = Paths::resolve(Some(directory.path().to_owned()))?;
        paths.initialize()?;
        fs::write(
            &paths.settings,
            "version = 1\ndevice_name = \"Mi Mac\"\nspeak_responses = false\n\
             assistant_name = \"Atlas\"\n",
        )?;
        let settings = paths.load_settings()?;
        assert_eq!(settings.assistant_name(), "Atlas");
        paths.save_settings(&settings)?;
        assert!(!fs::read_to_string(&paths.settings)?.contains("assistant_name"));
        assert_eq!(paths.load_settings()?.assistant_name(), "Atlas");
        // Once the new file exists, a name left in config.toml is not read.
        fs::write(
            &paths.settings,
            "version = 1\ndevice_name = \"Mi Mac\"\nspeak_responses = false\n\
             assistant_name = \"Otro\"\n",
        )?;
        assert_eq!(paths.load_settings()?.assistant_name(), "Atlas");
        Ok(())
    }

    /// A `config.toml` written by the previous version, with the
    /// three retired switches on, still loads; nothing keeps them, and the next
    /// save forgets them.
    #[test]
    fn a_config_with_the_three_old_switches_loads_and_forgets_them() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let paths = Paths::resolve(Some(directory.path().to_owned()))?;
        paths.initialize()?;
        fs::write(
            &paths.settings,
            "version = 1\ndevice_name = \"Mi Mac\"\nspeak_responses = false\n\
             remote_observation = true\nremote_action = true\nmeeting_sharing = true\n",
        )?;
        let settings = paths.load_settings()?;
        assert_eq!(settings.device_name, "Mi Mac");
        paths.save_settings(&settings)?;
        let written = fs::read_to_string(&paths.settings)?;
        for key in ["remote_observation", "remote_action", "meeting_sharing"] {
            assert!(
                !written.contains(key),
                "{key} is still in config.toml after saving:\n{written}"
            );
        }
        Ok(())
    }

    /// A `config.toml` written before «Enviar informes de errores
    /// anónimos» existed loads with it off, and the choice survives a save.
    #[test]
    fn a_config_without_the_error_report_switch_loads_with_it_off() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let paths = Paths::resolve(Some(directory.path().to_owned()))?;
        paths.initialize()?;
        fs::write(
            &paths.settings,
            "version = 1\ndevice_name = \"Mi Mac\"\nspeak_responses = false\n",
        )?;
        let mut settings = paths.load_settings()?;
        assert!(!settings.error_reports);
        settings.error_reports = true;
        paths.save_settings(&settings)?;
        assert!(paths.load_settings()?.error_reports);
        Ok(())
    }

    /// A `config.toml` written with the live-voice engine switched on
    /// still loads after the engine was removed, and the next save forgets it.
    #[test]
    fn a_config_with_the_retired_live_voice_keys_loads_and_forgets_them() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let paths = Paths::resolve(Some(directory.path().to_owned()))?;
        paths.initialize()?;
        fs::write(
            &paths.settings,
            "version = 1\ndevice_name = \"Mi Mac\"\nspeak_responses = false\n\
             voice_live = true\nvoice_live_prefetch = true\n",
        )?;
        let settings = paths.load_settings()?;
        assert_eq!(settings.device_name, "Mi Mac");
        paths.save_settings(&settings)?;
        let written = fs::read_to_string(&paths.settings)?;
        assert!(
            !written.contains("voice_live"),
            "voice_live sigue en config.toml tras guardar:\n{written}"
        );
        Ok(())
    }
    #[test]
    fn retired_source_limits_load_and_are_forgotten_on_save() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let paths = Paths::resolve(Some(directory.path().to_owned()))?;
        paths.initialize()?;
        fs::write(
            &paths.settings,
            r#"version = 1
device_name = "Mi Mac"
speak_responses = false
max_source_intents = 200
max_source_bytes = 140000000
"#,
        )?;
        let settings = paths.load_settings()?;
        paths.save_settings(&settings)?;
        let saved = fs::read_to_string(&paths.settings)?;
        assert!(!saved.contains("max_source_intents"));
        assert!(!saved.contains("max_source_bytes"));
        Ok(())
    }
}
