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
    /// Last name learned from the hub, retained across offline starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
        }
    }
}

impl Settings {
    pub fn assistant_name(&self) -> &str {
        self.assistant_name.as_deref().unwrap_or("Deimos")
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

    pub fn load_settings(&self) -> Result<Settings> {
        if !self.settings.exists() {
            return Ok(Settings::default());
        }
        let settings: Settings = toml::from_str(&fs::read_to_string(&self.settings)?)
            .context("config.toml no es válido; los datos almacenados no se han alterado")?;
        if settings.version != 1 {
            bail!("Versión de configuración no compatible");
        }
        Ok(settings)
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        self.initialize()?;
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
