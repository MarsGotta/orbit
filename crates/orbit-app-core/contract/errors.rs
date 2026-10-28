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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(skip)]
        pub refusal: Option<Box<Refusal>>,
    }
}

impl AppError {
    pub fn operation(code: &str, diagnostic: impl Into<String>) -> Self {
        let message = diagnostic.into();
        Self {
            code: code.into(),
            message: message.clone(),
            refusal: Some(Box::new(Refusal::Internal {
                cause: orbit_core::refusal::InternalCause::AppOperation {
                    code: code.into(),
                    error: message,
                },
            })),
        }
    }

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
        if refusal.is_some() {
            result.refusal = refusal;
        }
        result
    }

    pub fn daemon(message: impl Into<String>) -> Self {
        Self::operation("daemon_error", format!("daemon_error: {}", message.into()))
    }

    pub fn unavailable() -> Self {
        Self::operation("daemon_unavailable", "daemon_unavailable")
    }

    pub fn calendar(message: impl Into<String>) -> Self {
        Self::operation(
            "calendar_error",
            format!("calendar_error: {}", message.into()),
        )
    }

    pub fn update(message: impl Into<String>) -> Self {
        Self::operation("update_error", format!("update_error: {}", message.into()))
    }

    pub fn settings(message: impl Into<String>) -> Self {
        Self::operation(
            "settings_error",
            format!("settings_error: {}", message.into()),
        )
    }

    pub fn desktop_only() -> Self {
        Self::operation("desktop_command", "desktop_command")
    }

    pub fn client_command() -> Self {
        Self::operation("client_command", "client_command")
    }

    pub fn note_unavailable() -> Self {
        Self::operation("note_unavailable", "note_unavailable")
    }

    pub fn speaker_undo_unavailable() -> Self {
        Self::operation("speaker_undo_unavailable", "speaker_undo_unavailable")
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for AppError {}
