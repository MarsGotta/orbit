//! Control of the Mac: the engine that acts on it and the windows the
//! assistant may select.
use super::fmt;
use serde::{Deserialize, Serialize};

parameter_enum!(EngineCause { NotInstalled => "not_installed", NotRunning => "not_running", Unhealthy => "unhealthy", Unsupported => "unsupported", VoiceProcessingUnsupported => "voice_processing_unsupported", InitializationFailed => "initialization_failed" });

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionFacts {
    pub reason: SelectionFailure,
    pub application_bytes: Option<u64>,
    pub title_bytes: Option<u64>,
    pub filters_truncated: bool,
    pub application: Option<String>,
    pub title: Option<String>,
    pub windows: u64,
    pub matching: u64,
    pub selectable: u64,
    pub omitted: u64,
    pub inventory: Vec<SelectionWindow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionFailure {
    WindowNotFound,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionWindow {
    pub application: String,
    pub title: String,
    pub unavailable_reason: Option<String>,
    pub has_binding: bool,
}

macro_rules! control_refusals {
    ([$next:ident $(, $rest:ident)*] ; $($entries:tt)*) => {
        $next! { [$($rest),*] ; $($entries)*
            EngineUnavailable = 25 => "engine_unavailable" {
                cause: EngineCause,
            }, retry Retry::AfterFix;
        }
    };
}
pub(super) use control_refusals;
