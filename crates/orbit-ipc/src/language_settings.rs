//! Per-Mac presentation settings, owned and saved by the daemon. Kept beside
//! config.toml because older releases reject unknown keys in that file.
use crate::{Paths, atomic_private_write};
use anyhow::{Context, Result};
use orbit_core::language::LanguageChoice;
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
struct PresentationSettings {
    #[serde(default)]
    language: LanguageChoice,
}

impl Paths {
    pub(crate) fn load_language(&self) -> Result<LanguageChoice> {
        let path = self.root.join("presentation.toml");
        let language = if path.exists() {
            toml::from_str::<PresentationSettings>(&std::fs::read_to_string(path)?)
                .context("Invalid presentation.toml")?
                .language
        } else {
            LanguageChoice::System
        };
        Ok(language)
    }

    pub(crate) fn save_language(&self, language: LanguageChoice) -> Result<()> {
        atomic_private_write(
            &self.root.join("presentation.toml"),
            toml::to_string_pretty(&PresentationSettings { language })?.as_bytes(),
        )
    }
}
