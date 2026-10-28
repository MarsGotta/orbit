//! Presentation facts and an injected renderer. The shell owns human copy;
//! app-core can run without a shell, exposing diagnostic tokens in that case.
use orbit_core::{
    refusal::Refusal,
    topics::{Pending, TopicId},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateBlocker {
    NotObserved {
        topic: TopicId,
    },
    Unknown {
        topic: TopicId,
    },
    Stale {
        topic: TopicId,
    },
    Pending {
        topic: TopicId,
        why: Pending,
    },
    Refused {
        topic: TopicId,
        refusal: Box<Refusal>,
    },
    Recording,
    Call,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresentationText {
    UpdateBlocked(UpdateBlocker),
    AudioPending(Pending),
    ReadFailure {
        code: String,
        facts: serde_json::Value,
    },
    MeetingTitle {
        time: Option<String>,
    },
    AskAbout {
        title: String,
        recording_id: String,
    },
}

#[derive(Clone, Copy)]
pub struct PresentationCopy {
    pub render: fn(&PresentationText) -> String,
}

impl Default for PresentationCopy {
    fn default() -> Self {
        Self {
            render: |text| format!("{text:?}"),
        }
    }
}

impl PresentationCopy {
    pub fn text(self, text: PresentationText) -> String {
        (self.render)(&text)
    }

    pub(crate) fn recording_title(
        self,
        title: String,
        now_ms: i64,
        offset_ms: Option<i64>,
    ) -> String {
        if !title.trim().is_empty() {
            return title;
        }
        let time = offset_ms.map(|offset| {
            let minute = now_ms.saturating_add(offset).rem_euclid(86_400_000) / 60_000;
            format!("{:02}:{:02}", minute / 60, minute % 60)
        });
        self.text(PresentationText::MeetingTitle { time })
    }
}

/// Read failures travel in the existing structured-refusal encoding. Facts
/// remain available to callers; projections render a copy without changing it.
pub(crate) fn read_issue(code: &str, facts: serde_json::Value) -> String {
    Refusal::Internal {
        cause: orbit_core::refusal::InternalCause::AppOperation {
            code: code.into(),
            error: facts.to_string(),
        },
    }
    .to_string()
}

impl PresentationCopy {
    pub(crate) fn detail(self, detail: &str) -> String {
        if let Ok(Refusal::Internal {
            cause: orbit_core::refusal::InternalCause::AppOperation { code, error },
        }) = serde_json::from_str(detail)
            && code.starts_with("read_")
        {
            return self.text(PresentationText::ReadFailure {
                code,
                facts: serde_json::from_str(&error).unwrap_or_default(),
            });
        }
        detail.into()
    }

    pub(crate) fn optional_detail(self, detail: &mut Option<String>) {
        if let Some(value) = detail {
            *value = self.detail(value);
        }
    }
}
