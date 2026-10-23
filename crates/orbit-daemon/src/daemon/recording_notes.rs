//! The meetings and their notes live on the hub; the daemon passes them to
//! the app (`reads`) and carries the user's orders to the hub. It keeps what
//! the user wrote until the hub took it (`outbox`: the note orders, the
//! library's orders and the private notes, in lanes that never hold each
//! other back), the account name that names their microphone (`owner`), and
//! the head that tells the app what to read again.
//!
//! Every request runs off the daemon loop and its answer comes back as a
//! [`NoteEvent`]. Nothing here logs note content.
use super::{
    Runtime, now,
    ticketed::{Ticketed, WorkTicket},
};
use crate::hub_notes::{Answer, NoteHub, Unreached};
use anyhow::{Result, bail};
use orbit_core::{CapturePhase, notes as core};
use std::collections::BTreeMap;
use tokio::sync::mpsc;

mod answers;
mod commands;
mod deletion;
#[cfg(test)]
mod deletion_tests;
mod delivery;
#[cfg(test)]
mod fake_hub;
mod library;
mod lists;
mod outbox;
#[cfg(test)]
mod outbox_tests;
mod owner;
pub(super) mod reads;
mod reminders;
mod send;
mod supervised;
#[cfg(test)]
mod tests;

pub(super) use library::LibraryRuntime;
pub(super) use outbox::{Lane, Outbox};
#[cfg(test)]
use owner::read_account_name;

/// Answers waiting for the daemon loop: at most one owner name, one profile,
/// one deletion on this Mac and [`outbox::MAX_LANES_IN_FLIGHT`] lanes are in
/// flight (nine), so nine never makes a task wait. Not a queue of notes.
pub(super) const NOTE_EVENTS: usize = 3 + outbox::MAX_LANES_IN_FLIGHT;
/// Conflicts kept from one correction's answer, and corrections whose
/// conflicts are kept, newest last. The hub lists one per pair of a
/// dictionary written and another of the meeting that hears the form
/// otherwise; the band under the transcript names them, and past eight it
/// would be a list nobody reads. It protects the library read, not the order.
const MAX_KEPT_CONFLICTS: usize = 8;

pub(super) enum NoteEvent {
    /// The account's full name, read once off the loop.
    Owner { name: String },
    Profile {
        ticket: WorkTicket,
        outcome: Outcome,
    },
    /// The answer to one lane of the outbox.
    Lane {
        lane: Lane,
        ticket: WorkTicket,
        outcome: Outcome,
    },
    /// A deleted meeting's remains left this Mac, or why not.
    Purged {
        recording: String,
        scope: orbit_store::DeletionScope,
        outcome: std::result::Result<orbit_store::Deleted, String>,
    },
}

impl NoteEvent {
    fn kind(&self) -> &'static str {
        match self {
            Self::Owner { .. } => "note_owner",
            Self::Profile { .. } => "note_profile",
            Self::Lane { .. } => "note_lane",
            Self::Purged { .. } => "note_purged",
        }
    }
}

/// How one request to the hub ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Outcome {
    Done {
        requeued: Option<u64>,
        /// What the hub warned of when it took a correction: another
        /// dictionary of the meeting hears the form otherwise.
        conflicts: Vec<core::DictionaryConflict>,
    },
    Refused {
        status: u16,
        code: Option<String>,
        /// The hub's numbers for its refusal (`hub_notes::Answer::Refused`).
        numbers: BTreeMap<String, u64>,
    },
    Unreachable {
        cause: Unreached,
    },
}

impl<T> From<Answer<T>> for Outcome {
    fn from(answer: Answer<T>) -> Self {
        match answer {
            Answer::Value(_) => Self::Done {
                requeued: None,
                conflicts: Vec::new(),
            },
            Answer::Refused {
                status,
                code,
                numbers,
                ..
            } => Self::Refused {
                status,
                code,
                numbers,
            },
            Answer::Unreachable(cause) => Self::Unreachable { cause },
        }
    }
}

/// Hands one answer to the daemon loop. The receiver is gone only when the
/// runtime has ended; what the outbox owes is in the store.
async fn deliver(events: &mpsc::Sender<NoteEvent>, event: NoteEvent) {
    let kind = event.kind();
    if events.send(event).await.is_err() {
        eprintln!("note_result_receiver_closed kind={kind}");
    }
}

pub(super) struct NotesRuntime {
    events: mpsc::Sender<NoteEvent>,
    /// What the Notes topic says changed.
    pub(super) head: orbit_core::topics::notes::Value,
    owner_name: Option<String>,
    owner_lookup_started: bool,
    profile: Ticketed<()>,
    /// The hub connected and has not taken this Mac's owner name yet.
    profile_owed: bool,
    profile_due_ms: i64,
    pub(super) outbox: Outbox,
    pub(super) deletions: deletion::Deletions,
    pub(super) library: LibraryRuntime,
    /// Tests pin the clock the commands read; production reads the wall.
    pub(super) pinned_now: Option<i64>,
}

impl NotesRuntime {
    pub(super) fn new(events: mpsc::Sender<NoteEvent>) -> Self {
        Self {
            events,
            head: Default::default(),
            owner_name: None,
            owner_lookup_started: false,
            profile: Ticketed::logical(),
            profile_owed: false,
            profile_due_ms: i64::MIN,
            outbox: Outbox::default(),
            deletions: deletion::Deletions::default(),
            library: LibraryRuntime::default(),
            pinned_now: None,
        }
    }

    /// Test runtimes name the user without asking the host's account.
    #[cfg(test)]
    pub(super) fn for_tests(events: mpsc::Sender<NoteEvent>) -> Self {
        Self {
            owner_name: Some("Alina".into()),
            ..Self::new(events)
        }
    }

    #[cfg(test)]
    pub(super) fn test_events(&mut self) -> mpsc::Receiver<NoteEvent> {
        let (events, receiver) = mpsc::channel(NOTE_EVENTS);
        self.events = events;
        receiver
    }
}

fn check_recording(recording_id: &str) -> Result<()> {
    if !orbit_core::wire::canonical_uuid(recording_id) {
        bail!(core::NoteCommandError::InvalidRecording);
    }
    Ok(())
}

impl Runtime {
    pub(super) fn notes_now(&self) -> i64 {
        self.features.notes.pinned_now.unwrap_or_else(now)
    }

    pub(super) fn active_recording_id(&self) -> Option<&str> {
        self.state
            .durable
            .recording
            .as_ref()
            .filter(|recording| {
                matches!(
                    recording.phase,
                    CapturePhase::Starting
                        | CapturePhase::Recording
                        | CapturePhase::Paused
                        | CapturePhase::Finalizing
                )
            })
            .map(|recording| recording.id.as_str())
    }

    /// The hub, when Orbit may talk to it now: paired, online and armed.
    pub(super) fn note_hub(&self) -> Option<NoteHub> {
        let token = self.hub_access().ok()?;
        let base =
            crate::config::validate_hub_url(self.settings.hub_url.as_deref()?, self.development)
                .ok()?;
        Some(NoteHub {
            base,
            token: token.to_owned(),
            op: self.turn.op.clone(),
        })
    }

    /// What an older Mac or an interrupted run left owed.
    pub(super) fn recover_recording_notes(&mut self) -> Result<()> {
        self.recover_outbox()
    }

    /// The hub connected: the owner name goes to it, what is owed goes now,
    /// and the list may have changed while it was away.
    pub(super) fn notes_hub_ready(&mut self) {
        let notes = &mut self.features.notes;
        notes.profile_owed = true;
        notes.profile_due_ms = i64::MIN;
        notes.head.touch();
        self.outbox_hub_ready();
    }

    /// `recording.changed`: the app reads that recording again.
    pub(super) fn on_recording_changed(&mut self, recording_id: &str) {
        if orbit_core::wire::canonical_uuid(recording_id) {
            self.features.notes.head.touch_recording(recording_id);
        } else {
            self.log("recording_changed_invalid", &[]);
        }
    }

    /// A recording's closure reached the hub: the list reads it from there.
    pub(super) fn recording_uploaded(&mut self, recording_id: &str) {
        self.features.notes.head.touch_recording(recording_id);
    }

    /// A privacy stop, a revocation, the close or a new device: every request
    /// to the hub is aborted now. What the user wrote stays in the store and
    /// goes after the rearm (`outbox::retire_outbox`).
    pub(super) fn retire_recording_notes(&mut self) {
        let notes = &mut self.features.notes;
        notes.profile.cancel();
        notes.profile_owed = true;
        self.retire_outbox();
    }

    pub(super) fn maintain_recording_notes(&mut self) -> usize {
        let now_ms = self.notes_now();
        self.maintain_recording_notes_at(now_ms)
    }

    /// Starts whatever is due. Returns how many requests went out.
    pub(super) fn maintain_recording_notes_at(&mut self, now_ms: i64) -> usize {
        self.start_owner_lookup();
        self.start_due_purge(now_ms);
        let Some(hub) = self.note_hub() else {
            return 0;
        };
        let started = usize::from(self.start_profile(&hub, now_ms));
        started + self.start_due_lanes(&hub, now_ms)
    }

    pub(super) async fn note_answer(&mut self, event: NoteEvent) -> Result<()> {
        let now_ms = self.notes_now();
        self.note_answer_at(event, now_ms).await
    }

    /// One answer from the hub. An answer started before a privacy stop or a
    /// change of pairing settles as expired and is dropped.
    pub(super) async fn note_answer_at(&mut self, event: NoteEvent, now_ms: i64) -> Result<()> {
        let current = self.state.generation();
        match event {
            NoteEvent::Owner { name } => {
                self.features.notes.owner_name = Some(name);
            }
            NoteEvent::Profile { ticket, outcome } => {
                let settlement = self.features.notes.profile.settle(&ticket, current);
                if self.settled_value("note_profile", settlement).is_some() {
                    self.profile_answer(&outcome, now_ms);
                }
            }
            NoteEvent::Lane {
                lane,
                ticket,
                outcome,
            } => self.lane_answer(lane, ticket, outcome, now_ms).await?,
            NoteEvent::Purged {
                recording,
                scope,
                outcome,
            } => self.purge_answer(recording, scope, outcome, now_ms),
        }
        Ok(())
    }
}
