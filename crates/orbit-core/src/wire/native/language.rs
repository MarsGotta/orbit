//! The app's resolved speech locale. It governs the phrase and its question;
//! meeting transcription and note languages are independent.
use crate::language::{LanguageChoice, ResolvedLanguage};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub enum SpeechLocale {
    #[serde(rename = "es_ES")]
    Spanish,
    #[serde(rename = "en_US")]
    EnglishUS,
    #[serde(rename = "en_GB")]
    EnglishGB,
}

impl SpeechLocale {
    pub fn resolve(choice: LanguageChoice, preferred: &[impl AsRef<str>]) -> Self {
        if choice.resolve(preferred) == ResolvedLanguage::Spanish {
            return Self::Spanish;
        }
        let english = preferred.iter().find(|tag| {
            tag.as_ref()
                .split(['-', '_'])
                .next()
                .is_some_and(|code| code.eq_ignore_ascii_case("en"))
        });
        if english.is_some_and(|tag| {
            tag.as_ref()
                .split(['-', '_'])
                .any(|part| part.eq_ignore_ascii_case("GB"))
        }) {
            Self::EnglishGB
        } else {
            Self::EnglishUS
        }
    }
}
