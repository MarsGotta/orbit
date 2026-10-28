//! Device language decisions. Callers supply the operating system's ordered
//! preferred languages; this module never reads a process or global locale.
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub enum LanguageChoice {
    #[default]
    System,
    #[serde(rename = "es")]
    Spanish,
    #[serde(rename = "en")]
    English,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedLanguage {
    Spanish,
    English,
}

impl LanguageChoice {
    pub fn resolve(self, preferred: &[impl AsRef<str>]) -> ResolvedLanguage {
        match self {
            Self::Spanish => ResolvedLanguage::Spanish,
            Self::English => ResolvedLanguage::English,
            Self::System => preferred
                .iter()
                .find_map(|tag| match tag.as_ref().split(['-', '_']).next() {
                    Some(code) if code.eq_ignore_ascii_case("es") => {
                        Some(ResolvedLanguage::Spanish)
                    }
                    Some(code) if code.eq_ignore_ascii_case("en") => {
                        Some(ResolvedLanguage::English)
                    }
                    _ => None,
                })
                .unwrap_or(ResolvedLanguage::English),
        }
    }
}
