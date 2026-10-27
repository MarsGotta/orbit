//! What the hub's answer to an order is said as: the notice of a note order,
//! the refusal a library read shows by its target, a dictionary's failure
//! with its numbers. The outbox decides first whether the order is done
//! with (taken, refused, or not to be sent again); only then is it said.
use super::{
    MAX_KEPT_CONFLICTS, Outcome, Runtime,
    library::{MAX_KEPT_REFUSALS, MAX_KEPT_RESTORES, refusal_target, refuse_order},
};
use crate::hub_notes::Unreached;
use orbit_core::{
    note_outbox::NoteOrder,
    notes::{self as core, LibraryRequest},
    notice::{Notice, NoticeCode, NoticeKey},
};
use std::collections::BTreeMap;

/// The hub's numbers a dictionary refusal is said with, by their notice keys.
const DICTIONARY_NUMBERS: [(&str, NoticeKey); 12] = [
    ("terms", NoticeKey::Count),
    ("max_terms", NoticeKey::Total),
    ("forms", NoticeKey::Count),
    ("max_forms", NoticeKey::Total),
    ("dictionaries", NoticeKey::Count),
    ("max_dictionaries", NoticeKey::Total),
    ("characters", NoticeKey::Count),
    ("max_characters", NoticeKey::Total),
    ("series", NoticeKey::Count),
    ("max_series", NoticeKey::Total),
    ("bytes", NoticeKey::Bytes),
    ("max_bytes", NoticeKey::MaxBytes),
];

/// A final transcription whose answer did not come back: the hub may have
/// started it, so it is said as not confirmed, with the cause and its numbers.
fn unconfirmed(cause: Unreached) -> Notice {
    let notice = Notice::new(NoticeCode::TranscriptRetranscribeUnconfirmed);
    match cause {
        Unreached::Transport(_) => notice.with_token(NoticeKey::Cause, "hub_unreachable"),
        Unreached::TooLarge { bytes, max_bytes } => notice
            .with_token(NoticeKey::Cause, "hub_response_too_large")
            .with(NoticeKey::Bytes, bytes)
            .with(NoticeKey::MaxBytes, max_bytes as u64),
        Unreached::Cut { received_bytes } => notice
            .with_token(NoticeKey::Cause, "hub_response_cut")
            .with(NoticeKey::Bytes, received_bytes as u64),
        Unreached::Shape => notice.with_token(NoticeKey::Cause, "hub_response_invalid"),
        Unreached::Status { status, code } => {
            let mut notice = notice
                .with_token(NoticeKey::Cause, "hub_status")
                .with(NoticeKey::Status, u64::from(status));
            if let Some(code) = code {
                notice = notice.with_token(NoticeKey::Code, &code);
            }
            notice
        }
    }
}

impl Runtime {
    pub(super) fn log_outcome(&self, event: &str, outcome: &Outcome) {
        match outcome {
            Outcome::Done { .. } => {}
            Outcome::Refused { status, code, .. } => self.log(
                event,
                &[
                    ("status", &status.to_string()),
                    ("code", code.as_deref().unwrap_or("none")),
                ],
            ),
            Outcome::Unreachable { cause } => self.log(event, &[("cause", &cause.to_string())]),
        }
    }

    /// A note order the outbox is done with, said as the hub answered it.
    pub(super) fn order_answer(&mut self, order: &NoteOrder, outcome: Outcome) {
        match outcome {
            Outcome::Done { requeued, .. } => {
                // The hub announces the change (`recording.changed`); the
                // app reads the note again then.
                if matches!(order, NoteOrder::RetryTranscript) {
                    self.say(
                        Notice::new(NoticeCode::TranscriptRetryRequested)
                            .with(NoticeKey::Count, requeued.unwrap_or(0)),
                    );
                }
            }
            Outcome::Refused {
                status,
                code,
                numbers,
            } => self.order_refused(order, status, code, &numbers),
            Outcome::Unreachable { cause } => {
                self.log(
                    "note_order_unsent",
                    &[("order", order.name()), ("cause", &cause.to_string())],
                );
                let notice = if matches!(order, NoteOrder::Retranscribe { .. }) {
                    unconfirmed(cause)
                } else {
                    Notice::new(NoticeCode::NoteChangeUnsent)
                };
                self.say(notice.with_token(NoticeKey::Step, order.name()));
            }
        }
    }

    fn order_refused(
        &mut self,
        order: &NoteOrder,
        status: u16,
        code: Option<String>,
        numbers: &BTreeMap<String, u64>,
    ) {
        let order_name = order.name();
        self.log(
            "note_order_refused",
            &[
                ("order", order_name),
                ("status", &status.to_string()),
                ("code", code.as_deref().unwrap_or("none")),
            ],
        );
        // Audio the hub cannot read is said with how many groups of how
        // many, never as a generic refusal.
        if let (Some("meeting_audio_unreadable"), Some(groups), Some(total)) =
            (code.as_deref(), numbers.get("groups"), numbers.get("total"))
        {
            let mut notice = Notice::new(NoticeCode::MeetingAudioUnreadable)
                .with(NoticeKey::Count, *groups)
                .with(NoticeKey::Total, *total);
            if matches!(order, NoteOrder::Retranscribe { .. }) {
                notice = notice
                    .with_token(NoticeKey::Code, "meeting_audio_unreadable")
                    .with(NoticeKey::Status, u64::from(status));
            }
            self.say(notice);
            return;
        }
        let missing_turn = code.as_deref() == Some("turn_no_longer_exists");
        let mut notice = Notice::new(if matches!(order, NoteOrder::Retranscribe { .. }) {
            NoticeCode::TranscriptRetranscribeRefused
        } else if missing_turn {
            NoticeCode::TurnNoLongerExists
        } else {
            NoticeCode::NoteChangeRefused
        });
        if !matches!(order, NoteOrder::Retranscribe { .. }) {
            notice = notice.with_token(NoticeKey::Step, order_name);
        }
        notice = notice.with(NoticeKey::Status, u64::from(status));
        if let Some(code) = &code {
            notice = notice.with_token(NoticeKey::Code, code);
        }
        if missing_turn && !matches!(order, NoteOrder::Retranscribe { .. }) {
            for (name, key) in [("at_ms", NoticeKey::AtMs), ("turns", NoticeKey::Turns)] {
                if let Some(value) = numbers.get(name) {
                    notice = notice.with(key, *value);
                }
            }
        }
        self.say(notice);
    }

    /// Private notes the hub refused: they stay on this Mac (only the person
    /// can give them up), and the refusal is said with its status and code.
    pub(super) fn private_notes_refused(&mut self, status: u16, code: Option<&str>) {
        let mut notice = Notice::new(NoticeCode::NoteChangeRefused)
            .with_token(NoticeKey::Step, "private_notes")
            .with(NoticeKey::Status, u64::from(status));
        if let Some(code) = code {
            notice = notice.with_token(NoticeKey::Code, code);
        }
        self.say(notice);
    }

    /// A library order the outbox is done with: a refusal is kept by its
    /// target for the library read.
    pub(super) fn library_answer(&mut self, request: &LibraryRequest, outcome: Outcome) {
        if let LibraryRequest::Dictionary { request } = request {
            self.dictionary_answer(request, outcome);
            return;
        }
        let series = matches!(request, LibraryRequest::UseForSeries { .. });
        let changed = matches!(outcome, Outcome::Done { .. });
        let library = &mut self.notes.library;
        match outcome {
            Outcome::Done { .. } => {
                let target = request.target();
                library
                    .refusals
                    .retain(|refusal| refusal_target(refusal) != target);
                if let LibraryRequest::RestoreTemplate {
                    request_id,
                    template_id,
                    ..
                } = request
                {
                    library
                        .restores
                        .push_back((request_id.clone(), template_id.clone()));
                    while library.restores.len() > MAX_KEPT_RESTORES {
                        library.restores.pop_front();
                    }
                }
            }
            Outcome::Refused {
                status,
                code,
                numbers,
            } => {
                // Every refusal of the hub's own carries its code; a 404
                // without one is a route this hub does not have (an older hub).
                let code = code.unwrap_or_else(|| {
                    if status == 404 {
                        "hub_route_missing".into()
                    } else {
                        "refused".into()
                    }
                });
                refuse_order(library, request, status, &code, numbers);
            }
            Outcome::Unreachable { cause } => {
                self.log("note_library_unsent", &[("cause", &cause.to_string())]);
                let library = &mut self.notes.library;
                refuse_order(library, request, 0, "hub_unreachable", BTreeMap::new());
            }
        }
        if series && changed {
            // The series' dictionary can change the transcripts of its
            // meetings: every read is read again once, as for a dictionary.
            self.notes.invalidate_note_reads();
        }
        self.notes.head.touch_library();
    }

    pub(super) fn dictionary_answer(
        &mut self,
        request: &core::DictionaryRequest,
        outcome: Outcome,
    ) {
        let changed = matches!(&outcome, Outcome::Done { .. });
        let library = &mut self.notes.library;
        let target = request.target();
        library
            .dictionary_failures
            .retain(|failure| failure.target != target);
        library
            .dictionary_conflicts
            .retain(|(conflicted, _)| *conflicted != target);
        let failure = match outcome {
            Outcome::Done { conflicts, .. } => {
                if !conflicts.is_empty() {
                    library
                        .dictionary_conflicts
                        .push_back((target.clone(), conflicts));
                    while library.dictionary_conflicts.len() > MAX_KEPT_CONFLICTS {
                        library.dictionary_conflicts.pop_front();
                    }
                }
                None
            }
            refused @ (Outcome::Refused { .. } | Outcome::Unreachable { .. }) => {
                dictionary_failure(target, refused)
            }
        };
        let report = failure.as_ref().map(|failure| {
            let mut notice = Notice::new(NoticeCode::LibraryRefused)
                .with_token(NoticeKey::Code, &failure.code)
                .with_token(NoticeKey::Step, "dictionary");
            for (name, key) in DICTIONARY_NUMBERS {
                if let Some(value) = failure.numbers.get(name) {
                    notice = notice.with(key, *value);
                }
            }
            notice
        });
        if let Some(failure) = failure {
            library.dictionary_failures.push_back(failure);
            while library.dictionary_failures.len() > MAX_KEPT_REFUSALS {
                library.dictionary_failures.pop_front();
            }
        }
        if changed {
            // A shared dictionary can change any open transcript. Its confirmed
            // write invalidates all reads once, rather than flooding the hub's
            // bounded WebSocket queue with one event per historical recording.
            self.notes.invalidate_note_reads();
        }
        self.notes.head.touch_library();
        if let Some(report) = report {
            self.record(report);
        }
    }
}

/// Why a dictionary order did not change the dictionary, with the hub's code
/// and numbers, or the Mac's own cause (status 0) when it never left.
fn dictionary_failure(target: String, outcome: Outcome) -> Option<core::DictionaryFailure> {
    match outcome {
        Outcome::Done { .. } => None,
        Outcome::Refused {
            status,
            code,
            numbers,
        } => {
            let code = code.unwrap_or_else(|| "dictionary_refused".into());
            let detail = if status == 0 {
                match code.as_str() {
                    "not_paired" => "Orbit no está emparejado con el hub".into(),
                    "privacy_stopped" => "La parada de privacidad está activada".into(),
                    "hub_unreachable" => "El hub no está conectado".into(),
                    _ => "El cambio del diccionario no se envió al hub".into(),
                }
            } else {
                format!("El hub rechazó el cambio del diccionario (HTTP {status})")
            };
            Some(core::DictionaryFailure {
                target,
                code,
                detail,
                numbers,
            })
        }
        Outcome::Unreachable { cause } => Some(core::DictionaryFailure {
            target,
            code: "hub_unreachable".into(),
            detail: format!("No se confirmó el cambio del diccionario: {cause}"),
            numbers: BTreeMap::new(),
        }),
    }
}

impl crate::daemon::recording_notes::NotesHandle {
    /// Every note read is read again once (a dictionary changed), rather than
    /// one WebSocket event per historical recording.
    fn invalidate_note_reads(&mut self) {
        let head = &mut self.head;
        head.touch();
        head.recordings.clear();
        head.floor = head.revision;
    }
}
