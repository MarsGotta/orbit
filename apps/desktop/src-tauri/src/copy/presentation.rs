//! Render app-core's facts with this shell's catalogue.
use super::{Copy, fill};
use orbit_app_core::{PresentationCopy, PresentationText, UpdateBlocker};
use orbit_core::{
    language::ResolvedLanguage,
    topics::{Pending, TopicId},
};

pub fn renderer(language: ResolvedLanguage) -> PresentationCopy {
    PresentationCopy {
        render: match language {
            ResolvedLanguage::Spanish => |text| render(super::spanish(), text),
            ResolvedLanguage::English => |text| render(super::english(), text),
        },
    }
}

fn render(copy: &Copy, text: &PresentationText) -> String {
    match text {
        PresentationText::UpdateBlocked(reason) => update_blocker(copy, reason),
        PresentationText::ReadFailure { code, facts } => {
            let template = copy
                .read_failures
                .iter()
                .find(|(key, _)| key == code)
                .map(|(_, text)| *text)
                .unwrap_or(copy.read_failed);
            let mut values: Vec<_> = ["count", "max", "total", "diagnostic", "index"]
                .into_iter()
                .filter_map(|key| {
                    facts.get(key).map(|value| {
                        (
                            key,
                            value
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| value.to_string()),
                        )
                    })
                })
                .collect();
            values.push(("code", code.clone()));
            fill(
                template,
                &values
                    .iter()
                    .map(|(key, value)| (*key, value.as_str()))
                    .collect::<Vec<_>>(),
            )
        }
        PresentationText::AudioPending(why) => pending(copy, why, true),
        PresentationText::MeetingTitle { time } => match time {
            Some(time) => format!("{} {time}", copy.meeting_title),
            None => copy.meeting_title.into(),
        },
        PresentationText::AskAbout {
            title,
            recording_id,
        } => fill(copy.ask_about, &[("title", title), ("id", recording_id)]),
    }
}

fn update_blocker(copy: &Copy, reason: &UpdateBlocker) -> String {
    let (template, detail) = match reason {
        UpdateBlocker::Recording => return copy.update_recording_live.into(),
        UpdateBlocker::Call => return copy.update_call_live.into(),
        UpdateBlocker::NotObserved { topic } => (
            if *topic == TopicId::Recording {
                copy.update_recording_unobserved
            } else {
                copy.update_call_unobserved
            },
            String::new(),
        ),
        UpdateBlocker::Unknown { topic } => (
            if *topic == TopicId::Recording {
                copy.update_recording_unknown
            } else {
                copy.update_call_unknown
            },
            String::new(),
        ),
        UpdateBlocker::Stale { topic } => (
            if *topic == TopicId::Recording {
                copy.update_recording_stale
            } else {
                copy.update_call_stale
            },
            String::new(),
        ),
        UpdateBlocker::Pending { topic, why } => (
            if *topic == TopicId::Recording {
                copy.update_recording_pending
            } else {
                copy.update_call_pending
            },
            pending(copy, why, false),
        ),
        UpdateBlocker::Refused { topic, refusal } => (
            if *topic == TopicId::Recording {
                copy.update_recording_refused
            } else {
                copy.update_call_refused
            },
            refusal.to_string(),
        ),
    };
    fill(template, &[("detail", &detail)])
}

fn pending(copy: &Copy, why: &Pending, audio: bool) -> String {
    match why {
        Pending::NotObservedYet => if audio {
            copy.audio_unobserved
        } else {
            copy.pending_unobserved
        }
        .into(),
        Pending::HelperStarting => if audio {
            copy.audio_helper
        } else {
            copy.pending_helper
        }
        .into(),
        Pending::DaemonStarting => if audio {
            copy.audio_daemon
        } else {
            copy.pending_daemon
        }
        .into(),
        Pending::Reading { since_ms } => fill(
            if audio {
                copy.audio_reading
            } else {
                copy.pending_reading
            },
            &[("since_ms", &since_ms.to_string())],
        ),
        Pending::Transcription {
            completed,
            pending,
            failed,
            queued,
        } => fill(
            if audio {
                copy.audio_transcription
            } else {
                copy.pending_transcription
            },
            &[
                ("completed", &completed.to_string()),
                ("pending", &pending.to_string()),
                ("failed", &failed.to_string()),
                (
                    "queued",
                    if audio {
                        if *queued { "true" } else { "false" }
                    } else if *queued {
                        copy.yes
                    } else {
                        copy.no
                    },
                ),
            ],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn update_and_audio_copy_preserve_pending_measurements_in_both_languages() {
        for (language, expected) in [
            (ResolvedLanguage::Spanish, "leyendo"),
            (ResolvedLanguage::English, "reading"),
        ] {
            let copy = renderer(language);
            let text = copy.text(PresentationText::UpdateBlocked(UpdateBlocker::Pending {
                topic: TopicId::Recording,
                why: Pending::Reading { since_ms: 123 },
            }));
            assert!(text.contains(expected), "{text}");
            assert!(text.contains("123"), "{text}");
            let text = copy.text(PresentationText::AudioPending(Pending::Transcription {
                completed: 13,
                pending: 21,
                failed: 34,
                queued: true,
            }));
            for number in ["13", "21", "34"] {
                assert!(text.contains(number), "{text}");
            }
        }
    }
}

#[cfg(test)]
mod content_tests {
    use super::*;

    #[test]
    fn generated_titles_and_prefilled_questions_follow_the_language_and_preserve_content() {
        for (language, title, question) in [
            (
                ResolvedLanguage::Spanish,
                "Reunión 12:20",
                "Quiero preguntar",
            ),
            (ResolvedLanguage::English, "Meeting 12:20", "I want to ask"),
        ] {
            let copy = renderer(language);
            assert_eq!(
                copy.text(PresentationText::MeetingTitle {
                    time: Some("12:20".into())
                }),
                title
            );
            let text = copy.text(PresentationText::AskAbout {
                title: "Comité {id}".into(),
                recording_id: "recording-1".into(),
            });
            assert!(text.starts_with(question));
            assert!(
                text.contains("Comité {id}"),
                "user content is preserved: {text}"
            );
            assert!(text.contains("(recording-1)"));
        }
    }

    #[test]
    fn retained_read_failures_keep_their_counts_in_both_languages() {
        for language in [ResolvedLanguage::Spanish, ResolvedLanguage::English] {
            let text = renderer(language).text(PresentationText::ReadFailure {
                code: "read_turn_limit".into(),
                facts: serde_json::json!({"max": 64, "count": 211}),
            });
            assert!(text.contains("64"));
            assert!(text.contains("211"));
            let unknown = renderer(language).text(PresentationText::ReadFailure {
                code: "read_future".into(),
                facts: serde_json::json!({}),
            });
            assert!(unknown.contains("read_future"));
        }
    }
}
