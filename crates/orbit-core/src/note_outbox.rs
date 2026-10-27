//! The notes outbox's decisions. What the person writes about a meeting (an
//! edit of its note, a task ticked, its title, a speaker's name, its
//! language, a request to write or transcribe it again) and their orders
//! about templates and series wait on this Mac until the hub took them: one
//! lane per meeting and one for the library, so a meeting the hub refuses or
//! does not answer never holds another one back.
//!
//! Pure: which lane an order goes in and what it changes, how a newer order
//! on the same target replaces an older one, which orders may go again when
//! no answer came back, how long a lane waits after a failure, and how the
//! orders still on the Mac lay over the hub's document. The daemon passes
//! the time and the hub's answers.
use crate::notes::{LibraryRequest, MeetingDocument, Section, SpeakerRenameScope};
use crate::protocol::Command;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Orders waiting on this Mac at most, every lane together. A newer order on
/// the same target replaces the older one, so a meeting owes a handful (its
/// note's edits, title, language, one per speaker renamed); 256 cover weeks
/// of writing with the hub away. It protects the daemon's copy of the outbox
/// and the store table; past it a new order is refused with both numbers,
/// never dropped. It does not limit what the hub keeps.
pub const MAX_WAITING_ORDERS: usize = 256;

/// The first wait of a lane whose request failed (the hub did not answer, or
/// refused private notes it must keep); each failure in a row doubles it.
/// Five seconds lets a hub that restarts take it at the next try without
/// asking a failing one every second while the person types.
pub const FIRST_RETRY_MS: i64 = 5_000;

/// The longest wait of a failing lane. Five minutes keep an outage of hours
/// to a few requests a lane; the hub's return (`notes_hub_ready`) sends what
/// is owed at once, so this is never the wait after an outage.
pub const MAX_RETRY_MS: i64 = 300_000;

/// A note order the user gave, as it goes to the hub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "order", rename_all = "snake_case", deny_unknown_fields)]
pub enum NoteOrder {
    Edit {
        sections: Option<Vec<Section>>,
        done: BTreeMap<String, bool>,
        base_revision: u64,
    },
    Write {
        template_id: String,
    },
    RestoreSuperseded,
    Rename {
        key: String,
        name: Option<String>,
        scope: SpeakerRenameScope,
    },
    RetryTranscript,
    Retranscribe {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<crate::transcription_config::TranscriptionModel>,
    },
    /// The meeting's language.
    Language {
        language: String,
    },
    /// The meeting's name; empty gives it back its own.
    Title {
        title: String,
    },
    /// «Borrar reunión», or «Terminar sin guardar» (`discarded`: its audio
    /// already left this Mac): the hub deletes the recording and all made
    /// from it. It goes before anything else of its meeting, which it makes
    /// moot, and waits while the meeting is still closing.
    Delete {
        #[serde(default)]
        discarded: bool,
    },
}

impl NoteOrder {
    /// The note order a command gives, with its meeting; `None` for a
    /// command that gives none.
    pub fn of_command(command: Command) -> Option<(String, Self)> {
        Some(match command {
            Command::TranscriptRetranscribe {
                recording_id,
                model,
            } => (recording_id, Self::Retranscribe { model }),
            Command::TranscriptRetry { recording_id } => (recording_id, Self::RetryTranscript),
            Command::NoteEdit {
                recording_id,
                sections,
                done,
                base_revision,
            } => (
                recording_id,
                Self::Edit {
                    sections,
                    done,
                    base_revision,
                },
            ),
            Command::NoteRegenerate {
                recording_id,
                template_id,
            } => (recording_id, Self::Write { template_id }),
            Command::NoteRestoreSuperseded { recording_id } => {
                (recording_id, Self::RestoreSuperseded)
            }
            Command::SpeakerRename {
                recording_id,
                key,
                name,
                scope,
            } => (recording_id, Self::Rename { key, name, scope }),
            Command::MeetingLanguage {
                recording_id,
                language,
            } => (recording_id, Self::Language { language }),
            Command::MeetingRename {
                recording_id,
                title,
            } => (recording_id, Self::Title { title }),
            Command::MeetingDelete { recording_id } => {
                (recording_id, Self::Delete { discarded: false })
            }
            _ => return None,
        })
    }

    /// The order's name in notices and logs.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Edit { .. } => "edit",
            Self::Write { .. } => "write",
            Self::RestoreSuperseded => "restore_superseded",
            Self::Rename { .. } => "rename",
            Self::RetryTranscript => "retry_transcript",
            Self::Retranscribe { .. } => "retranscribe",
            Self::Language { .. } => "language",
            Self::Title { .. } => "title",
            Self::Delete { .. } => "delete",
        }
    }

    fn target(&self) -> String {
        match self {
            Self::Rename {
                key,
                scope: SpeakerRenameScope::Voice,
                ..
            } => format!("speaker:{key}"),
            Self::Rename {
                key,
                scope: SpeakerRenameScope::Turn { at_ms },
                ..
            } => format!("speaker:{key}:{at_ms}"),
            other @ (Self::Edit { .. }
            | Self::Write { .. }
            | Self::RestoreSuperseded
            | Self::RetryTranscript
            | Self::Retranscribe { .. }
            | Self::Language { .. }
            | Self::Title { .. }
            | Self::Delete { .. }) => other.name().to_owned(),
        }
    }

    /// What the order still changes in the meeting's document while it
    /// waits, as the app names it: `sections`, `action:{id}`, `title`…
    pub fn pending_changes(&self) -> Vec<String> {
        match self {
            Self::Edit { sections, done, .. } => sections
                .iter()
                .map(|_| "sections".to_owned())
                .chain(done.keys().map(|id| format!("action:{id}")))
                .collect(),
            Self::Rename { key, .. } => vec![format!("speaker:{key}")],
            other @ (Self::Write { .. }
            | Self::RestoreSuperseded
            | Self::RetryTranscript
            | Self::Retranscribe { .. }
            | Self::Language { .. }
            | Self::Title { .. }
            | Self::Delete { .. }) => vec![other.name().to_owned()],
        }
    }
}

/// The lane an order waits in: one per meeting, one for the library.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Lane {
    Library,
    Meeting(String),
}

impl Lane {
    /// How the store names the lane.
    pub fn key(&self) -> String {
        match self {
            Self::Library => "library".into(),
            Self::Meeting(recording) => format!("meeting:{recording}"),
        }
    }
}

/// One order waiting on this Mac.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "lane", rename_all = "snake_case", deny_unknown_fields)]
pub enum QueuedOrder {
    Meeting {
        recording_id: String,
        order: NoteOrder,
    },
    Library {
        request: LibraryRequest,
    },
}

impl QueuedOrder {
    pub fn lane(&self) -> Lane {
        match self {
            Self::Meeting { recording_id, .. } => Lane::Meeting(recording_id.clone()),
            Self::Library { .. } => Lane::Library,
        }
    }

    /// What the order changes: a newer order on the same target of the same
    /// lane replaces it. `None` for a dictionary order, which never replaces
    /// another: two corrections into one dictionary are two corrections.
    pub fn target(&self) -> Option<String> {
        match self {
            Self::Meeting { order, .. } => Some(order.target()),
            Self::Library {
                request: LibraryRequest::Dictionary { .. },
            } => None,
            Self::Library { request } => Some(request.target()),
        }
    }

    /// This order, replaced by a newer one on the same target. Two edits of
    /// one note merge as the hub merges them: the newer sections replace the
    /// older ones, and each task keeps its newest mark. Anything else is the
    /// newer order alone; the older one is dropped, never sent.
    pub fn superseded_by(self, newer: Self) -> Self {
        match (self, newer) {
            (
                Self::Meeting {
                    order:
                        NoteOrder::Edit {
                            sections: older_sections,
                            done: mut older_done,
                            ..
                        },
                    ..
                },
                Self::Meeting {
                    recording_id,
                    order:
                        NoteOrder::Edit {
                            sections,
                            done,
                            base_revision,
                        },
                },
            ) => {
                older_done.extend(done);
                Self::Meeting {
                    recording_id,
                    order: NoteOrder::Edit {
                        sections: sections.or(older_sections),
                        done: older_done,
                        base_revision,
                    },
                }
            }
            // A meeting thrown away stays thrown away: its audio is gone.
            (
                Self::Meeting {
                    order: NoteOrder::Delete { discarded: older },
                    ..
                },
                Self::Meeting {
                    recording_id,
                    order: NoteOrder::Delete { discarded },
                },
            ) => Self::Meeting {
                recording_id,
                order: NoteOrder::Delete {
                    discarded: older || discarded,
                },
            },
            (_, newer) => newer,
        }
    }

    /// The meeting this order deletes, and whether its capture was thrown
    /// away; `None` for any other order.
    pub fn deletes(&self) -> Option<(&str, bool)> {
        match self {
            Self::Meeting {
                recording_id,
                order: NoteOrder::Delete { discarded },
            } => Some((recording_id, *discarded)),
            Self::Meeting { .. } | Self::Library { .. } => None,
        }
    }

    /// Whether the order may go again when no answer came back (the hub did
    /// not answer, cut the answer, or asked for time). Setting a value twice
    /// sets it once; writing the note again or a final transcription redo
    /// the hub's work, and a dictionary order adds a correction, so those
    /// are said as not confirmed instead.
    pub fn repeatable(&self) -> bool {
        !matches!(
            self,
            Self::Meeting {
                order: NoteOrder::Write { .. } | NoteOrder::Retranscribe { .. },
                ..
            } | Self::Library {
                request: LibraryRequest::Dictionary { .. }
            }
        )
    }
}

/// How long a lane waits after `failures` failed requests in a row.
pub fn retry_after_ms(failures: u32) -> i64 {
    let doublings = failures.saturating_sub(1).min(16);
    FIRST_RETRY_MS
        .saturating_mul(1_i64 << doublings)
        .min(MAX_RETRY_MS)
}

/// The meeting's orders still on this Mac, oldest first, laid over the
/// hub's document: what the person sees is what they wrote, and
/// `pending_changes` says which of it the hub does not have yet. An empty
/// title gives the meeting back its own, which only the hub knows; a
/// speaker given back their default name keeps the hub's.
pub fn lay_over<'a>(
    document: &mut MeetingDocument,
    orders: impl IntoIterator<Item = &'a NoteOrder>,
) {
    let mut pending = std::collections::BTreeSet::new();
    for order in orders {
        pending.extend(order.pending_changes());
        match order {
            NoteOrder::Title { title } if !title.is_empty() => document.title.clone_from(title),
            NoteOrder::Language { language } => document.language = Some(language.clone()),
            NoteOrder::Rename {
                key,
                name: Some(name),
                scope: SpeakerRenameScope::Voice,
            } => {
                for speaker in document.speakers.iter_mut().filter(|s| s.key == *key) {
                    speaker.name.clone_from(name);
                }
            }
            NoteOrder::Edit { sections, done, .. } => {
                let seen = document.note.as_mut().and_then(|note| note.note.as_mut());
                if let Some(seen) = seen {
                    if let Some(sections) = sections {
                        seen.sections.clone_from(sections);
                    }
                    for item in &mut seen.action_items {
                        if let Some(mark) = done.get(&item.id) {
                            item.done = *mark;
                        }
                    }
                }
            }
            NoteOrder::Title { .. }
            | NoteOrder::Rename { .. }
            | NoteOrder::Write { .. }
            | NoteOrder::RestoreSuperseded
            | NoteOrder::RetryTranscript
            | NoteOrder::Retranscribe { .. }
            | NoteOrder::Delete { .. } => {}
        }
    }
    document.pending_changes = pending.into_iter().collect();
}

#[cfg(test)]
mod tests;
