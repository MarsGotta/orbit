//! The meetings and their notes live on the hub; the daemon passes them to
//! the app (`reads`) and carries the user's orders to the hub. It keeps only
//! their private notes until the hub confirms them (a durable outbox in the
//! store), the account name that names their microphone, and the head that
//! tells the app what to read again.
//!
//! Every request runs off the daemon loop and its answer comes back as a
//! [`NoteEvent`]. Nothing here logs note content.
use super::{
    Runtime, now,
    ticketed::{DeadlineKind, Settlement, Ticketed, WorkTicket},
};
use crate::hub_notes::{Answer, NoteHub, Unreached};
use anyhow::{Context, Result, bail};
use orbit_core::{
    CapturePhase,
    notes::{self as core, LibraryRefusal, LibraryRequest, Section},
    notice::{Notice, NoticeCode, NoticeKey},
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Duration;
use tokio::sync::mpsc;

#[cfg(test)]
mod fake_hub;
pub(super) mod reads;
#[cfg(test)]
mod tests;

/// Answers waiting for the daemon loop: at most one profile, one private
/// notes send, one library order and [`MAX_ORDERS_IN_FLIGHT`] note orders
/// are in flight (seven), so eight never makes a task wait. Not a queue of
/// notes.
pub(super) const NOTE_EVENTS: usize = 8;
/// Note orders (an edit, a rewrite, a rename, a retry) in flight at once.
/// The user gives them one by one; four let a slow answer not hold the next one
/// back while one Mac never keeps more than four requests open on the hub.
/// Past it the order is refused with both numbers, never dropped.
const MAX_ORDERS_IN_FLIGHT: usize = 4;
/// Library refusals kept to show, newest last. Each names its target, so a
/// template or a series shows its own; eight cover one session of edits. It
/// protects daemon memory and the library read; not a limit on orders.
const MAX_KEPT_REFUSALS: usize = 8;
/// Undos («Deshacer») remembered as answered, newest last: the app reads its
/// own a moment after pressing it. Four cover several in a row.
const MAX_KEPT_RESTORES: usize = 4;
/// After private notes or the profile could not reach the hub they go again
/// no sooner than this: the hub is down or the network is, and once every
/// thirty seconds keeps a failing hub from being asked every second while
/// the user types. Not a deadline: what is owed stays in the store.
const RETRY_AFTER_MS: i64 = 30_000;

pub(super) enum NoteEvent {
    /// The account's full name, read once off the loop.
    Owner { name: String },
    Profile {
        ticket: WorkTicket,
        outcome: Outcome,
    },
    Private {
        ticket: WorkTicket,
        outcome: Outcome,
    },
    Order {
        ticket: WorkTicket,
        outcome: Outcome,
    },
    Library {
        ticket: WorkTicket,
        outcome: Outcome,
    },
}

impl NoteEvent {
    fn kind(&self) -> &'static str {
        match self {
            Self::Owner { .. } => "note_owner",
            Self::Profile { .. } => "note_profile",
            Self::Private { .. } => "note_private",
            Self::Order { .. } => "note_order",
            Self::Library { .. } => "note_library",
        }
    }
}

/// How one request to the hub ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Outcome {
    Done {
        requeued: Option<u64>,
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
            Answer::Value(_) => Self::Done { requeued: None },
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

/// A note order the user gave, as it goes to the hub.
#[derive(Debug, Clone)]
pub(super) enum NoteOrder {
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
        scope: core::SpeakerRenameScope,
    },
    RetryTranscript,
    Retranscribe,
    /// The meeting's language.
    Language {
        language: String,
    },
    /// The meeting's name; empty gives it back its own.
    Title {
        title: String,
    },
}

#[derive(Default)]
pub(super) struct LibraryRuntime {
    in_flight: Ticketed<LibraryRequest>,
    dictionary_failures: VecDeque<core::DictionaryFailure>,
    /// Orders not sent yet, one per target, in the order the user gave them: two
    /// orders about the same template never reach the hub out of order.
    waiting: VecDeque<LibraryRequest>,
    refusals: VecDeque<LibraryRefusal>,
    restores: VecDeque<(String, String)>,
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
    /// Recordings whose private notes wait in the store for the hub.
    private_pending: BTreeSet<String>,
    private: Ticketed<String>,
    private_due_ms: i64,
    orders: Vec<Ticketed<(String, NoteOrder)>>,
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
            private_pending: BTreeSet::new(),
            private: Ticketed::logical(),
            private_due_ms: i64::MIN,
            orders: (0..MAX_ORDERS_IN_FLIGHT)
                .map(|_| Ticketed::logical())
                .collect(),
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

/// How long the account's full name may take. macOS answers `id -F` from
/// Directory Services in milliseconds; if opendirectoryd hangs, the read hangs
/// with it. Two seconds is far above a healthy answer; meanwhile the hub names
/// the microphone «Micrófono local». It bounds this one read, which runs off
/// the daemon loop.
const ACCOUNT_NAME: DeadlineKind = DeadlineKind::new(
    "account_name",
    Duration::from_secs(2),
    "the account name is read off the loop and never holds a privacy stop",
);

/// The full name of the account (`pw_gecos`) from a program that prints it,
/// `id -F` on macOS: any failure or a late answer is «Yo», never an error.
// Used on macOS, and by its test everywhere.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) async fn read_account_name(program: &std::path::Path, deadline: Duration) -> String {
    let output = tokio::time::timeout(
        deadline,
        tokio::process::Command::new(program)
            .arg("-F")
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await;
    let gecos = match output {
        Ok(Ok(output)) if output.status.success() => String::from_utf8(output.stdout).ok(),
        Ok(Ok(_) | Err(_)) | Err(_) => None,
    };
    core::owner_name(gecos.as_deref())
}

/// macOS keeps accounts in Directory Services, so `/etc/passwd` does not hold
/// them there; elsewhere (development) `/etc/passwd` is read.
async fn account_full_name() -> String {
    #[cfg(target_os = "macos")]
    {
        read_account_name(std::path::Path::new("/usr/bin/id"), ACCOUNT_NAME.duration()).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let read = tokio::task::spawn_blocking(|| {
            let uid = rustix::process::getuid().as_raw().to_string();
            std::fs::read_to_string("/etc/passwd")
                .ok()
                .and_then(|passwd| {
                    passwd.lines().find_map(|line| {
                        let fields = line.split(':').collect::<Vec<_>>();
                        if fields.get(2) == Some(&uid.as_str()) {
                            fields.get(4).map(|gecos| (*gecos).to_owned())
                        } else {
                            None
                        }
                    })
                })
        });
        let gecos = match tokio::time::timeout(ACCOUNT_NAME.duration(), read).await {
            Ok(Ok(gecos)) => gecos,
            Ok(Err(_)) | Err(_) => None,
        };
        core::owner_name(gecos.as_deref())
    }
}

fn check_recording(recording_id: &str) -> Result<()> {
    if !orbit_core::wire::canonical_uuid(recording_id) {
        bail!(core::NoteCommandError::InvalidRecording);
    }
    Ok(())
}

impl Runtime {
    fn notes_now(&self) -> i64 {
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

    /// The private notes an older Mac or an interrupted run left owed.
    pub(super) fn recover_recording_notes(&mut self) -> Result<()> {
        self.features.notes.private_pending = self
            .reader
            .pending_private_notes()
            .context("No se pudieron leer las notas privadas pendientes")?
            .into_iter()
            .collect();
        Ok(())
    }

    /// The hub connected: the owner name goes to it, owed private notes go
    /// now, and the list may have changed while it was away.
    pub(super) fn notes_hub_ready(&mut self) {
        let notes = &mut self.features.notes;
        notes.profile_owed = true;
        notes.profile_due_ms = i64::MIN;
        notes.private_due_ms = i64::MIN;
        notes.head.touch();
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
    /// to the hub is aborted now. Private notes stay in the store and go out
    /// after the rearm; library orders not sent are said as not sent.
    pub(super) fn retire_recording_notes(&mut self) {
        let notes = &mut self.features.notes;
        notes.profile.cancel();
        notes.profile_owed = true;
        notes.private.cancel();
        notes.private_due_ms = i64::MIN;
        for slot in &mut notes.orders {
            slot.cancel();
        }
        let library = &mut notes.library;
        let dropped = library
            .in_flight
            .active()
            .cloned()
            .into_iter()
            .chain(library.waiting.drain(..))
            .collect::<Vec<_>>();
        library.in_flight.cancel();
        if !dropped.is_empty() {
            for request in &dropped {
                refuse_order(library, request, 0, "not_sent");
            }
            notes.head.touch_library();
        }
    }

    pub(super) fn maintain_recording_notes(&mut self) -> usize {
        let now_ms = self.notes_now();
        self.maintain_recording_notes_at(now_ms)
    }

    /// Starts whatever is due. Returns how many requests went out.
    pub(super) fn maintain_recording_notes_at(&mut self, now_ms: i64) -> usize {
        self.start_owner_lookup();
        let Some(hub) = self.note_hub() else {
            return 0;
        };
        let mut started = usize::from(self.start_profile(&hub, now_ms));
        started += usize::from(self.start_private_notes(&hub, now_ms));
        started += usize::from(self.start_library_order(&hub));
        started
    }

    fn start_owner_lookup(&mut self) {
        let notes = &mut self.features.notes;
        if notes.owner_name.is_some() || notes.owner_lookup_started {
            return;
        }
        notes.owner_lookup_started = true;
        let events = notes.events.clone();
        tokio::spawn(async move {
            let name = account_full_name().await;
            deliver(&events, NoteEvent::Owner { name }).await;
        });
    }

    fn start_profile(&mut self, hub: &NoteHub, now_ms: i64) -> bool {
        let generation = self.state.generation();
        let op = self.turn.op.clone();
        let notes = &mut self.features.notes;
        let Some(owner) = notes.owner_name.clone() else {
            return false;
        };
        if !notes.profile_owed || now_ms < notes.profile_due_ms {
            return false;
        }
        let hub = hub.clone();
        let events = notes.events.clone();
        let started = notes
            .profile
            .start_logical(generation, op, (), move |ticket| {
                tokio::spawn(async move {
                    let outcome = hub.profile(&owner).await.into();
                    deliver(&events, NoteEvent::Profile { ticket, outcome }).await;
                })
            })
            .is_some();
        if started {
            notes.profile_owed = false;
        }
        started
    }

    fn start_private_notes(&mut self, hub: &NoteHub, now_ms: i64) -> bool {
        let notes = &self.features.notes;
        if notes.private.active().is_some() || now_ms < notes.private_due_ms {
            return false;
        }
        let Some(recording) = notes.private_pending.first().cloned() else {
            return false;
        };
        let pending = match self.reader.private_notes(&recording) {
            Ok(Some(pending)) => pending,
            Ok(None) => {
                self.features.notes.private_pending.remove(&recording);
                return false;
            }
            Err(error) => {
                self.log("private_notes_unreadable", &[("error", &error.to_string())]);
                self.features.notes.private_due_ms = now_ms.saturating_add(RETRY_AFTER_MS);
                return false;
            }
        };
        let generation = self.state.generation();
        let op = self.turn.op.clone();
        let hub = hub.clone();
        let notes = &mut self.features.notes;
        let events = notes.events.clone();
        notes
            .private
            .start_logical(generation, op, recording.clone(), move |ticket| {
                tokio::spawn(async move {
                    let outcome = send_private_notes(&hub, &pending).await;
                    deliver(&events, NoteEvent::Private { ticket, outcome }).await;
                })
            })
            .is_some()
    }

    fn start_library_order(&mut self, hub: &NoteHub) -> bool {
        let generation = self.state.generation();
        let op = self.turn.op.clone();
        let notes = &mut self.features.notes;
        let library = &mut notes.library;
        if library.in_flight.active().is_some() {
            return false;
        }
        let Some(request) = library.waiting.pop_front() else {
            return false;
        };
        let hub = hub.clone();
        let events = notes.events.clone();
        let sent = request.clone();
        library
            .in_flight
            .start_logical(generation, op, request, move |ticket| {
                tokio::spawn(async move {
                    let outcome = send_library_order(&hub, &sent).await;
                    deliver(&events, NoteEvent::Library { ticket, outcome }).await;
                })
            })
            .is_some()
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
                if self.settled_value("note_profile", settlement).is_none() {
                    return Ok(());
                }
                if !matches!(outcome, Outcome::Done { .. }) {
                    self.log_outcome("device_profile_not_saved", &outcome);
                    let notes = &mut self.features.notes;
                    notes.profile_owed = true;
                    notes.profile_due_ms = now_ms.saturating_add(RETRY_AFTER_MS);
                }
            }
            NoteEvent::Private { ticket, outcome } => {
                let settlement = self.features.notes.private.settle(&ticket, current);
                let Some(recording) = self.settled_value("note_private", settlement) else {
                    return Ok(());
                };
                self.private_answer(recording, outcome, now_ms).await?;
            }
            NoteEvent::Order { ticket, outcome } => {
                let mut settled = Settlement::Foreign;
                for slot in &mut self.features.notes.orders {
                    settled = slot.settle(&ticket, current);
                    if !matches!(settled, Settlement::Foreign) {
                        break;
                    }
                }
                let Some((_recording, order)) = self.settled_value("note_order", settled) else {
                    return Ok(());
                };
                self.order_answer(&order, outcome);
            }
            NoteEvent::Library { ticket, outcome } => {
                let settlement = self
                    .features
                    .notes
                    .library
                    .in_flight
                    .settle(&ticket, current);
                let Some(request) = self.settled_value("note_library", settlement) else {
                    return Ok(());
                };
                self.library_answer(&request, outcome);
            }
        }
        Ok(())
    }

    fn log_outcome(&self, event: &str, outcome: &Outcome) {
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

    async fn private_answer(
        &mut self,
        recording: String,
        outcome: Outcome,
        now_ms: i64,
    ) -> Result<()> {
        match outcome {
            Outcome::Done { .. } => {
                // The text the hub confirmed is the one sent; a newer one
                // typed meanwhile stays in the outbox and goes next.
                let sent = self
                    .reader
                    .private_notes(&recording)?
                    .map(|pending| pending.markdown);
                let Some(sent) = sent else {
                    self.features.notes.private_pending.remove(&recording);
                    return Ok(());
                };
                let id = recording.clone();
                let confirmed = self
                    .store_actor
                    .run(move |store| Ok(store.confirm_private_notes(&id, &sent)?))
                    .await
                    .context("No se pudo apuntar que el hub tiene las notas privadas")?;
                if confirmed {
                    self.features.notes.private_pending.remove(&recording);
                }
            }
            Outcome::Refused { .. } | Outcome::Unreachable { .. } => {
                // They stay in the store; the user keeps seeing what they typed.
                self.log_outcome("private_notes_not_saved", &outcome);
                self.features.notes.private_due_ms = now_ms.saturating_add(RETRY_AFTER_MS);
            }
        }
        Ok(())
    }

    fn order_answer(&mut self, order: &NoteOrder, outcome: Outcome) {
        let order_name = match order {
            NoteOrder::Edit { .. } => "edit",
            NoteOrder::Write { .. } => "write",
            NoteOrder::RestoreSuperseded => "restore_superseded",
            NoteOrder::Rename { .. } => "rename",
            NoteOrder::RetryTranscript => "retry_transcript",
            NoteOrder::Retranscribe => "retranscribe",
            NoteOrder::Language { .. } => "language",
            NoteOrder::Title { .. } => "title",
        };
        match outcome {
            Outcome::Done { requeued } => {
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
            } => {
                self.log(
                    "note_order_refused",
                    &[
                        ("order", order_name),
                        ("status", &status.to_string()),
                        ("code", code.as_deref().unwrap_or("none")),
                    ],
                );
                // Audio the hub cannot read is said with how many
                // groups of how many, never as a generic refusal.
                if let (Some("meeting_audio_unreadable"), Some(groups), Some(total)) =
                    (code.as_deref(), numbers.get("groups"), numbers.get("total"))
                {
                    let mut notice = Notice::new(NoticeCode::MeetingAudioUnreadable)
                        .with(NoticeKey::Count, *groups)
                        .with(NoticeKey::Total, *total);
                    if matches!(order, NoteOrder::Retranscribe) {
                        notice = notice
                            .with_token(NoticeKey::Code, "meeting_audio_unreadable")
                            .with(NoticeKey::Status, u64::from(status));
                    }
                    self.say(notice);
                    return;
                }
                if matches!(order, NoteOrder::Retranscribe) {
                    let mut notice = Notice::new(NoticeCode::TranscriptRetranscribeRefused)
                        .with(NoticeKey::Status, u64::from(status));
                    if let Some(code) = &code {
                        notice = notice.with_token(NoticeKey::Code, code);
                    }
                    self.say(notice);
                    return;
                }
                let mut notice = Notice::new(if code.as_deref() == Some("turn_no_longer_exists") {
                    NoticeCode::TurnNoLongerExists
                } else {
                    NoticeCode::NoteChangeRefused
                })
                .with_token(NoticeKey::Step, order_name)
                .with(NoticeKey::Status, u64::from(status));
                if let Some(code) = &code {
                    notice = notice.with_token(NoticeKey::Code, code);
                }
                if code.as_deref() == Some("turn_no_longer_exists") {
                    for (name, key) in [("at_ms", NoticeKey::AtMs), ("turns", NoticeKey::Turns)] {
                        if let Some(value) = numbers.get(name) {
                            notice = notice.with(key, *value);
                        }
                    }
                }
                self.say(notice);
            }
            Outcome::Unreachable { cause } => {
                self.log(
                    "note_order_unsent",
                    &[("order", order_name), ("cause", &cause.to_string())],
                );
                let notice = if matches!(order, NoteOrder::Retranscribe) {
                    let notice = Notice::new(NoticeCode::TranscriptRetranscribeUnconfirmed);
                    match cause {
                        Unreached::Transport(_) => {
                            notice.with_token(NoticeKey::Cause, "hub_unreachable")
                        }
                        Unreached::TooLarge { bytes, max_bytes } => notice
                            .with_token(NoticeKey::Cause, "hub_response_too_large")
                            .with(NoticeKey::Bytes, bytes)
                            .with(NoticeKey::MaxBytes, max_bytes as u64),
                        Unreached::Cut { received_bytes } => notice
                            .with_token(NoticeKey::Cause, "hub_response_cut")
                            .with(NoticeKey::Bytes, received_bytes as u64),
                        Unreached::Shape => {
                            notice.with_token(NoticeKey::Cause, "hub_response_invalid")
                        }
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
                } else {
                    Notice::new(NoticeCode::NoteChangeUnsent)
                };
                self.say(notice.with_token(NoticeKey::Step, order_name));
            }
        }
    }

    fn library_answer(&mut self, request: &LibraryRequest, outcome: Outcome) {
        if let LibraryRequest::Dictionary { request } = request {
            self.dictionary_answer(request, outcome);
            return;
        }
        let library = &mut self.features.notes.library;
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
            Outcome::Refused { status, code, .. } => {
                refuse_order(
                    library,
                    request,
                    status,
                    code.as_deref().unwrap_or("refused"),
                );
            }
            Outcome::Unreachable { cause } => {
                self.log("note_library_unsent", &[("cause", &cause.to_string())]);
                let library = &mut self.features.notes.library;
                refuse_order(library, request, 0, "hub_unreachable");
            }
        }
        self.features.notes.head.touch_library();
    }

    fn dictionary_answer(&mut self, request: &core::DictionaryRequest, outcome: Outcome) {
        let changed = matches!(&outcome, Outcome::Done { .. });
        let library = &mut self.features.notes.library;
        let target = request.target();
        library
            .dictionary_failures
            .retain(|failure| failure.target != target);
        let failure = match outcome {
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
        };
        let report = failure.as_ref().map(|failure| {
            let mut notice = Notice::new(NoticeCode::LibraryRefused)
                .with_token(NoticeKey::Code, &failure.code)
                .with_token(NoticeKey::Step, "dictionary");
            for (name, key) in [
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
            ] {
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
            let head = &mut self.features.notes.head;
            head.touch();
            head.recordings.clear();
            head.floor = head.revision;
        }
        self.features.notes.head.touch_library();
        if let Some(report) = report {
            self.record(report);
        }
    }

    pub(super) async fn note_private_save(
        &mut self,
        recording_id: String,
        markdown: String,
        base_revision: u64,
    ) -> Result<()> {
        check_recording(&recording_id)?;
        core::check_private_notes(&markdown)?;
        let now_ms = self.notes_now();
        let pending = orbit_store::PendingPrivateNotes {
            recording_id: recording_id.clone(),
            markdown,
            base_revision,
        };
        self.store_actor
            .run(move |store| Ok(store.save_private_notes(&pending, now_ms)?))
            .await
            .context("No se pudieron guardar las notas privadas")?;
        let notes = &mut self.features.notes;
        notes.private_pending.insert(recording_id);
        notes.private_due_ms = i64::MIN;
        Ok(())
    }

    /// Sends one note order to the hub, off the loop. Refused at once when
    /// the hub cannot be reached now or too many are in flight, with the
    /// numbers; its answer comes back as a notice when it fails.
    pub(super) fn note_order(&mut self, recording_id: String, order: NoteOrder) -> Result<()> {
        check_recording(&recording_id)?;
        let order = match order {
            NoteOrder::Rename { key, name, scope } => NoteOrder::Rename {
                key,
                scope,
                name: core::speaker_name(name.as_deref())?,
            },
            NoteOrder::Language { language } => {
                core::meeting_language(&language)?;
                NoteOrder::Language { language }
            }
            NoteOrder::Title { title } => NoteOrder::Title {
                title: core::meeting_title(&title)?,
            },
            other @ (NoteOrder::Edit { .. }
            | NoteOrder::Write { .. }
            | NoteOrder::RestoreSuperseded
            | NoteOrder::RetryTranscript
            | NoteOrder::Retranscribe) => other,
        };
        let hub = self
            .note_hub()
            .ok_or(orbit_core::refusal::Refusal::HubUnreachable { since_ms: None })?;
        let generation = self.state.generation();
        let op = self.turn.op.clone();
        let notes = &mut self.features.notes;
        let events = notes.events.clone();
        let in_flight = notes
            .orders
            .iter()
            .filter(|slot| slot.active().is_some())
            .count();
        let Some(slot) = notes.orders.iter_mut().find(|slot| slot.active().is_none()) else {
            return Err(orbit_core::refusal::Refusal::WorkInFlight {
                in_flight: in_flight as u64,
                max: MAX_ORDERS_IN_FLIGHT as u64,
            }
            .into());
        };
        let sent = (recording_id.clone(), order.clone());
        slot.start_logical(generation, op, (recording_id, order), move |ticket| {
            tokio::spawn(async move {
                let outcome = send_note_order(&hub, &sent.0, &sent.1).await;
                deliver(&events, NoteEvent::Order { ticket, outcome }).await;
            })
        });
        Ok(())
    }

    /// A template or assignment order. It waits behind the one in
    /// flight; an earlier order about the same thing not sent yet is
    /// replaced by this one.
    pub(super) fn note_library(&mut self, request: LibraryRequest) {
        if let LibraryRequest::Dictionary {
            request: dictionary,
        } = &request
        {
            if let Err(refusal) = self.hub_access() {
                self.dictionary_answer(
                    dictionary,
                    Outcome::Refused {
                        status: 0,
                        code: Some(refusal.code().name().into()),
                        numbers: BTreeMap::new(),
                    },
                );
                return;
            }
            if self.note_hub().is_none() {
                self.dictionary_answer(
                    dictionary,
                    Outcome::Unreachable {
                        cause: Unreached::Transport(if self.settings.hub_url.is_none() {
                            "falta la URL del hub"
                        } else {
                            "la URL del hub no es válida"
                        }),
                    },
                );
                return;
            }
            self.features.notes.library.waiting.push_back(request);
            self.features.notes.head.touch_library();
            if let Some(hub) = self.note_hub() {
                self.start_library_order(&hub);
            }
            return;
        }
        let library = &mut self.features.notes.library;
        let target = request.target();
        match library
            .waiting
            .iter_mut()
            .find(|waiting| waiting.target() == target)
        {
            Some(waiting) => *waiting = request,
            None => library.waiting.push_back(request),
        }
        self.features.notes.head.touch_library();
        if let Some(hub) = self.note_hub() {
            self.start_library_order(&hub);
        }
    }

    /// What the library read adds to the hub's lists: the orders on their
    /// way, the refused ones and the undos answered.
    pub(super) fn library_state(&self) -> Vec<core::LibraryItem> {
        let library = &self.features.notes.library;
        let pending = library
            .in_flight
            .active()
            .into_iter()
            .chain(library.waiting.iter())
            .map(|request| core::LibraryItem::Pending {
                target: request.target(),
            });
        let refused = library
            .refusals
            .iter()
            .cloned()
            .map(core::LibraryItem::Refused);
        let restored =
            library
                .restores
                .iter()
                .map(|(request_id, template_id)| core::LibraryItem::Restored {
                    request_id: request_id.clone(),
                    template_id: template_id.clone(),
                });
        let dictionary_failures = library
            .dictionary_failures
            .iter()
            .cloned()
            .map(|failure| core::LibraryItem::DictionaryRefused { failure });
        pending
            .chain(refused)
            .chain(restored)
            .chain(dictionary_failures)
            .collect()
    }
}

fn refusal_target(refusal: &LibraryRefusal) -> String {
    format!("{}:{}", refusal.target_kind, refusal.target_id)
}

fn refuse_order(library: &mut LibraryRuntime, request: &LibraryRequest, status: u16, code: &str) {
    let (target_kind, target_id, action) = match request {
        LibraryRequest::Dictionary { request } => {
            library
                .dictionary_failures
                .push_back(core::DictionaryFailure {
                    target: request.target(),
                    code: code.into(),
                    detail: "No se confirmó el cambio: la sesión del hub se cerró".into(),
                    numbers: BTreeMap::new(),
                });
            while library.dictionary_failures.len() > MAX_KEPT_REFUSALS {
                library.dictionary_failures.pop_front();
            }
            return;
        }
        LibraryRequest::SaveTemplate { template_id, .. }
        | LibraryRequest::RestoreTemplate { template_id, .. } => ("template", template_id, "save"),
        LibraryRequest::DeleteTemplate { template_id } => ("template", template_id, "delete"),
        LibraryRequest::Assign {
            calendar_item_id,
            template_id: Some(_),
        } => ("series", calendar_item_id, "assign"),
        LibraryRequest::Assign {
            calendar_item_id,
            template_id: None,
        } => ("series", calendar_item_id, "unassign"),
    };
    library.refusals.push_back(LibraryRefusal {
        target_kind: target_kind.into(),
        target_id: target_id.clone(),
        action: action.into(),
        status,
        code: code.into(),
    });
    while library.refusals.len() > MAX_KEPT_REFUSALS {
        library.refusals.pop_front();
    }
}

/// The hub's answer to a note edit on a revision that moved (409
/// `note_revision_changed`, which carries no revision): the document is read
/// again for the current one, and the order goes once more on it. The Mac is
/// the only writer of this device's private notes and edits, so the revision
/// moves under it only when the hub itself rewrote the note or an earlier
/// order of the user's had not been answered yet.
fn conflict<T>(answer: &Answer<T>) -> bool {
    matches!(answer, Answer::Refused { status: 409, .. })
}

async fn current_document(
    hub: &NoteHub,
    recording: &str,
) -> Result<core::MeetingDocument, Outcome> {
    match hub.document(recording).await {
        Answer::Value(document) => Ok(document),
        other @ (Answer::Refused { .. } | Answer::Unreachable(_)) => Err(other.into()),
    }
}

/// Private notes go on the revision the hub's 409 names
/// (`stale_revision {revision, base_revision}`); a 409 without it is
/// answered as the hub gave it.
async fn send_private_notes(hub: &NoteHub, pending: &orbit_store::PendingPrivateNotes) -> Outcome {
    let recording = &pending.recording_id;
    let first = hub
        .private_notes(recording, &pending.markdown, pending.base_revision)
        .await;
    match first {
        Answer::Refused {
            status: 409,
            revision: Some(current),
            ..
        } if current != pending.base_revision => hub
            .private_notes(recording, &pending.markdown, current)
            .await
            .into(),
        other @ (Answer::Value(_) | Answer::Refused { .. } | Answer::Unreachable(_)) => {
            other.into()
        }
    }
}

async fn send_note_order(hub: &NoteHub, recording: &str, order: &NoteOrder) -> Outcome {
    match order {
        NoteOrder::Edit {
            sections,
            done,
            base_revision,
        } => {
            let first = hub
                .edit_note(recording, sections.as_deref(), done, *base_revision)
                .await;
            if !conflict(&first) {
                return first.into();
            }
            match current_document(hub, recording).await {
                Ok(document) => {
                    let revision = document.note.map_or(0, |note| note.revision);
                    hub.edit_note(recording, sections.as_deref(), done, revision)
                        .await
                        .into()
                }
                Err(outcome) => outcome,
            }
        }
        NoteOrder::Write { template_id } => hub.write_note(recording, template_id).await.into(),
        NoteOrder::RestoreSuperseded => restore_superseded(hub, recording).await,
        NoteOrder::Rename { key, name, scope } => hub
            .rename(recording, key, name.as_deref(), *scope)
            .await
            .into(),
        NoteOrder::Language { language } => hub.language(recording, language).await.into(),
        NoteOrder::Title { title } => hub.title(recording, title).await.into(),
        NoteOrder::Retranscribe => hub.final_pass(recording).await.into(),
        NoteOrder::RetryTranscript => match hub.retry_transcript(recording).await {
            Answer::Value(value) => Outcome::Done {
                requeued: value.get("requeued").and_then(serde_json::Value::as_u64),
            },
            other @ (Answer::Refused { .. } | Answer::Unreachable(_)) => other.into(),
        },
    }
}

/// The edits a regeneration set aside come back as an edit of the note: the
/// meeting's document says they exist, the note's own route carries them.
async fn restore_superseded(hub: &NoteHub, recording: &str) -> Outcome {
    let note = match hub.note_with_superseded(recording).await {
        Answer::Value(note) => note,
        other @ (Answer::Refused { .. } | Answer::Unreachable(_)) => return other.into(),
    };
    let Some(sections) = note
        .get("superseded_edits")
        .and_then(|edits| edits.get("sections"))
        .and_then(|sections| serde_json::from_value::<Vec<Section>>(sections.clone()).ok())
    else {
        return Outcome::Refused {
            status: 404,
            code: Some("nothing_superseded".into()),
            numbers: BTreeMap::new(),
        };
    };
    let revision = note
        .get("revision")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    hub.edit_note(recording, Some(&sections), &BTreeMap::new(), revision)
        .await
        .into()
}

async fn send_library_order(hub: &NoteHub, request: &LibraryRequest) -> Outcome {
    match request {
        LibraryRequest::Dictionary { request } => hub.dictionary_order(request).await.into(),
        LibraryRequest::SaveTemplate { template_id, draft } => {
            hub.save_template(template_id, draft).await.into()
        }
        LibraryRequest::DeleteTemplate { template_id } => {
            hub.delete_template(template_id).await.into()
        }
        LibraryRequest::Assign {
            calendar_item_id,
            template_id: Some(template_id),
        } => hub.link(calendar_item_id, template_id).await.into(),
        LibraryRequest::Assign {
            calendar_item_id,
            template_id: None,
        } => hub.unlink(calendar_item_id).await.into(),
        LibraryRequest::RestoreTemplate {
            template_id,
            draft,
            series,
            ..
        } => {
            let saved: Outcome = hub.save_template(template_id, draft).await.into();
            if !matches!(saved, Outcome::Done { .. }) {
                return saved;
            }
            for calendar_item_id in series.iter().flatten() {
                let linked: Outcome = hub.link(calendar_item_id, template_id).await.into();
                if !matches!(linked, Outcome::Done { .. }) {
                    return linked;
                }
            }
            saved
        }
    }
}
