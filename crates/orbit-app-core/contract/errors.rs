use super::*;
use orbit_core::refusal::Refusal;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandError {
    pub command: AppCommand,
    pub refusal: Box<Refusal>,
}

impl CommandError {
    pub fn from_app_error(command: AppCommand, error: AppError) -> Self {
        Self {
            command,
            // Adapters without a domain refusal still retain their operation
            // code and diagnostic cause.
            refusal: error.refusal.unwrap_or_else(|| {
                Box::new(Refusal::Internal {
                    cause: orbit_core::refusal::InternalCause::AppOperation {
                        code: error.code,
                        error: error.message,
                    },
                })
            }),
        }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.refusal, formatter)
    }
}

impl std::error::Error for CommandError {}

app_type! {
    pub struct AppError {
        pub code: String,
        pub message: String,
        #[serde(skip)]
        #[ts(skip)]
        pub refusal: Option<Box<Refusal>>,
    }
}

impl AppError {
    pub fn from_daemon(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::with_cause(Self::daemon(error.to_string()), error)
    }

    pub fn from_calendar(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::with_cause(Self::calendar(error.to_string()), error)
    }

    fn with_cause(mut result: Self, error: &(dyn std::error::Error + 'static)) -> Self {
        let mut cause = Some(error);
        let mut refusal = None;
        while let Some(error) = cause {
            if let Some(typed) = error.downcast_ref::<Refusal>() {
                refusal = Some(Box::new(typed.clone()));
                break;
            }
            cause = error.source();
        }
        result.refusal = refusal;
        result
    }

    pub fn daemon(message: impl Into<String>) -> Self {
        Self {
            code: "daemon_error".into(),
            refusal: None,
            message: format!("daemon_error: {}", message.into()),
        }
    }

    pub fn unavailable() -> Self {
        Self {
            code: "daemon_unavailable".into(),
            refusal: None,
            message: "daemon_unavailable".into(),
        }
    }

    pub fn calendar(message: impl Into<String>) -> Self {
        Self {
            code: "calendar_error".into(),
            refusal: None,
            message: format!("calendar_error: {}", message.into()),
        }
    }

    pub fn update(message: impl Into<String>) -> Self {
        Self {
            code: "update_error".into(),
            refusal: None,
            message: format!("update_error: {}", message.into()),
        }
    }

    pub fn settings(message: impl Into<String>) -> Self {
        Self {
            code: "settings_error".into(),
            refusal: None,
            message: format!("settings_error: {}", message.into()),
        }
    }

    pub fn desktop_only() -> Self {
        Self {
            code: "desktop_command".into(),
            refusal: None,
            message: "desktop_command".into(),
        }
    }

    pub fn client_command() -> Self {
        Self {
            code: "client_command".into(),
            refusal: None,
            message: "client_command".into(),
        }
    }

    pub fn note_unavailable() -> Self {
        Self {
            code: "note_unavailable".into(),
            refusal: None,
            message: "note_unavailable".into(),
        }
    }

    pub fn speaker_undo_unavailable() -> Self {
        Self {
            code: "speaker_undo_unavailable".into(),
            refusal: None,
            message: "speaker_undo_unavailable".into(),
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for AppError {}
