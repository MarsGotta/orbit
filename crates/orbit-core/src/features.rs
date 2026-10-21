//! The product functions shared by commands and presentation frames.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export, export_to = "../../apps/desktop/src/bridge/generated/")
)]
#[serde(rename_all = "snake_case")]
pub enum FeatureId {
    Status,
    Permissions,
    Agenda,
    Notes,
    Note,
    Recording,
    Assistant,
    Control,
    Briefing,
    Local,
}

impl FeatureId {
    pub const ALL: &'static [Self] = &[
        Self::Status,
        Self::Permissions,
        Self::Agenda,
        Self::Notes,
        Self::Note,
        Self::Recording,
        Self::Assistant,
        Self::Control,
        Self::Briefing,
        Self::Local,
    ];
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Permissions => "permissions",
            Self::Agenda => "agenda",
            Self::Notes => "notes",
            Self::Note => "note",
            Self::Recording => "recording",
            Self::Assistant => "assistant",
            Self::Control => "control",
            Self::Briefing => "briefing",
            Self::Local => "local",
        }
    }
}
