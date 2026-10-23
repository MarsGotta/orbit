//! The daemon's one state owner and the results its workers send back.
use super::*;

/// How many in-flight helper command IDs the daemon and bridge retain. This
/// protects the daemon and bridge's memory from a helper that stops replying
/// or leaks correlation IDs. In the bridge it only decides whether a reply is
/// labelled with its command name (`_bridge_command`); every reply is still
/// forwarded. In the daemon it also decides whether a late reply that is not
/// a capture reply (those are named by the capture registry) still has its
/// effect or is logged as unpaired. The helper runs 1 command at a time and
/// queues up to 32, answering anything beyond that at once, so 64 slots cover
/// its whole queue plus a burst. `playback`, `playback_clip` and `audio_stop`
/// skip that queue (`NativeCommandRouter.swift`, ~:84-87), so they can be in
/// flight beyond it; `audio_stop` is still named by the capture registry.
pub(crate) const MAX_PENDING_NATIVE: usize = 64;

/// Wake-up notices the store actor and the upload workers may have waiting for
/// the daemon's select loop while it is busy with another event. What it
/// protects: the loop's own memory and its freedom to come back — a notice
/// only has to exist for the loop to be woken, so 16 slots are slack for the
/// work results of one busy turn (seal receipts, upload settlements, vertical
/// completions), not a throughput size. What it is not: it is not an audio
/// queue (PCM travels in the store actor's own ordered lanes) and not a
/// durability bound — a `Sealed` wake-up that does not fit is dropped while
/// its receipt stays recorded in the actor, whose collector applies it within
/// the 1 s maintenance tick, or within `SEAL_RETRY_DELAY` after a failed seal;
/// a `StoreProblem` notice never takes that drop path (it waits for room on
/// the writer). The writer never blocks on this channel,
/// so nothing may treat it as backpressure either.
pub(super) const WORK_EVENTS_CAPACITY: usize = 16;

pub(super) enum WorkResult {
    Control(ControlWork),
    Capture(CaptureWork),
    Store(StoreWork),
    Problem {
        ticket: ticketed::WorkTicket,
        message: String,
    },
    SpeechAvailability {
        ticket: ticketed::WorkTicket,
        /// The agent whose voice was reading.
        voice: String,
        available: bool,
    },
    BridgeCaptureUpload {
        ticket: ticketed::WorkTicket,
        request_id: String,
        capture: bridge::PendingCapture,
        outcome: bridge::CaptureUploadOutcome,
    },
    /// An attachment order read a HEIF photo: only the loop sends the
    /// helper its `photo_jpeg`. The wait for its answer, or why it was not
    /// sent, goes back on `asked`.
    PhotoJpeg {
        source_path: std::path::PathBuf,
        destination_path: std::path::PathBuf,
        asked: tokio::sync::oneshot::Sender<
            std::result::Result<
                tokio::sync::oneshot::Receiver<helper_waits::HelperAnswer>,
                orbit_core::refusal::Refusal,
            >,
        >,
    },
}

impl WorkResult {
    /// The op of the turn that started this work; a result with no
    /// ticket gets a new one.
    pub(super) fn op(&self) -> orbit_core::notice::Op {
        let ticket = match self {
            Self::Control(ControlWork::Computer { ticket, .. })
            | Self::Capture(
                CaptureWork::VoiceProblem { ticket, .. } | CaptureWork::Transcribed { ticket, .. },
            )
            | Self::Store(
                StoreWork::Uploaded { ticket, .. }
                | StoreWork::UploadSettled { ticket, .. }
                | StoreWork::ChunkPruned { ticket, .. },
            )
            | Self::Problem { ticket, .. }
            | Self::SpeechAvailability { ticket, .. }
            | Self::BridgeCaptureUpload { ticket, .. } => Some(ticket),
            Self::Store(StoreWork::Sealed { .. } | StoreWork::StoreProblem { .. })
            | Self::PhotoJpeg { .. } => None,
        };
        ticket.map_or_else(orbit_ipc::new_op, |ticket| ticket.op().clone())
    }
}

pub(super) async fn deliver_work(sender: &mpsc::Sender<WorkResult>, result: WorkResult) {
    let kind = result.kind();
    if sender.send(result).await.is_err() {
        eprintln!("work_result_receiver_closed kind={kind}");
    }
}

impl WorkResult {
    /// What kind of result this is, as a code for the logs.
    pub(super) fn kind(&self) -> &'static str {
        match self {
            WorkResult::Control(ControlWork::Computer { .. }) => "computer",
            WorkResult::Capture(CaptureWork::VoiceProblem { .. }) => "voice_problem",
            WorkResult::Capture(CaptureWork::Transcribed { .. }) => "transcription",
            WorkResult::Store(StoreWork::Uploaded { .. }) => "upload",
            WorkResult::Store(StoreWork::UploadSettled { .. }) => "upload_settlement",
            WorkResult::Store(StoreWork::ChunkPruned { .. }) => "chunk_cleanup",
            WorkResult::Store(StoreWork::Sealed { .. }) => "seal",
            WorkResult::Store(StoreWork::StoreProblem { .. }) => "store_problem",
            WorkResult::Problem { .. } => "problem",
            WorkResult::SpeechAvailability { .. } => "speech_availability",
            WorkResult::BridgeCaptureUpload { .. } => "bridge_capture_upload",
            WorkResult::PhotoJpeg { .. } => "photo_jpeg",
        }
    }
}

pub(super) enum ControlWork {
    Computer {
        ticket: ticketed::WorkTicket,
        event: Box<Result<computer_control::ComputerCompletion>>,
    },
}

pub(super) enum CaptureWork {
    VoiceProblem {
        ticket: ticketed::WorkTicket,
        operation_id: String,
        failure: orbit_core::VoiceTranscriptionFailure,
    },
    Transcribed {
        ticket: ticketed::WorkTicket,
        source: audio::VoiceSource,
        text: String,
    },
}

pub(super) enum StoreWork {
    ChunkPruned {
        ticket: ticketed::WorkTicket,
        result: Result<()>,
    },
    Uploaded {
        ticket: ticketed::WorkTicket,
        job_id: String,
        result: Result<()>,
    },
    UploadSettled {
        ticket: ticketed::WorkTicket,
        succeeded: bool,
        error: Option<String>,
        /// What to tell the person about a failed upload (`work.rs`).
        notice: Option<orbit_core::notice::Notice>,
        result: Result<()>,
    },
    /// A seal's receipt is there. The loop collects every receipt the lane
    /// holds, whoever took it from the writer, so the one named here is
    /// only what woke it.
    #[allow(
        dead_code,
        reason = "the wake-up names its receipt for whoever reads it"
    )]
    Sealed {
        recording: String,
        sequence: u64,
    },
    StoreProblem {
        message: String,
    },
}

pub(super) struct Runtime {
    pub(super) closing: bool,
    /// A timed-out privacy retirement keeps its durable tail owed to the
    /// next maintenance turn after the writer recovers.
    pub(super) privacy_retirement_owed: bool,
    pub(super) paths: Paths,
    pub(super) settings: Settings,
    pub(super) development: bool,
    pub(super) state: State,
    /// The loop's synchronous reads, each timed into `turn_watch`.
    pub(super) reader: timed_reader::TimedReader,
    /// Times every loop turn, its steps and its waits (`daemon_turn_slow`).
    pub(super) turn_watch: turn_watch::TurnWatch,
    /// Reads voice-note audio off the actor; see `reads::VoiceAudioReader`.
    pub(super) voice_audio_reader: reads::VoiceAudioReader,
    pub(super) store_actor: store_actor::StoreActor,
    /// Every actor's panics, counted; the loop ends on a spent budget.
    pub(super) supervision: crate::supervision::Supervision,
    /// `None` only in fixtures that assert nothing about the trail.
    pub(super) diagnostics: Option<crate::diagnostics::DiagnosticLog>,
    /// The local report log; `None` in fixtures, like `diagnostics`.
    pub(super) report_log: Option<orbit_ipc::report::ReportLog>,
    /// The Mac's one error-report client: what reaches the local report log
    /// may also leave for Sentry, if the user allowed it.
    pub(super) error_reports: crate::error_reports::ErrorReports,
    /// The op of the turn being handled and who it answers.
    pub(super) turn: turn::Turn,
    pub(super) native: mpsc::Sender<Value>,
    pub(super) hub: hub_link::HubLink,
    pub(super) hub_protocol_version: Option<u64>,
    pub(super) unrecognized_hub_events: BTreeSet<String>,
    /// Whether the hub last said it was nearly full (`ready.storage`,
    /// `storage.status`); `None` before it said anything. Kept in memory
    /// only, so a restart of Orbit says a nearly full hub once more.
    pub(super) hub_storage_nearly_full: Option<bool>,
    pub(super) hub_task: Option<JoinHandle<Result<()>>>,
    pub(super) snapshot_revision: Option<u64>,
    pub(super) hub_events: mpsc::Sender<(u64, HubEvent)>,
    /// Helper command ids still waiting for a reply, with the name of the
    /// command each one carried. Bounded by [`MAX_PENDING_NATIVE`]; entries
    /// are removed when the matching response arrives, so a late refusal can
    /// be classified without parsing names out of the id.
    pub(super) pending_native: BTreeMap<String, String>,
    /// Helper orders a task off the loop waits for (`helper_waits`), by id.
    /// An entry leaves when its reply arrives, when its task gave up and
    /// the next order is sent, or when the helper restarts.
    pub(super) helper_waits:
        BTreeMap<String, tokio::sync::oneshot::Sender<helper_waits::HelperAnswer>>,
    pub(super) token: Option<String>,
    pub(super) work: WorkRuntime,
    pub(super) conversation: ConversationRuntime,
    pub(super) capture: CaptureRuntime,
    pub(super) features: FeatureRuntime,
    pub(super) reads: InFlightReads,
    #[cfg(test)]
    pub(super) fixture_store: Store,
}

pub(super) struct InFlightReads {
    pub(super) tasks: Vec<JoinHandle<()>>,
    pub(super) privacy_epoch_tx: watch::Sender<u64>,
    pub(super) privacy_epoch_rx: watch::Receiver<u64>,
    pub(super) was_privacy_stopped: bool,
}

impl Default for InFlightReads {
    fn default() -> Self {
        let (privacy_epoch_tx, privacy_epoch_rx) = watch::channel(0);
        Self {
            tasks: Vec::new(),
            privacy_epoch_tx,
            privacy_epoch_rx,
            was_privacy_stopped: false,
        }
    }
}

impl InFlightReads {
    pub(super) fn retire(&mut self, change: orbit_core::authority::AuthorityChange) {
        use orbit_core::authority::AuthorityChange;
        match change {
            AuthorityChange::UserStop | AuthorityChange::Revoked => {
                publish_privacy_epoch(true, &mut self.was_privacy_stopped, &self.privacy_epoch_tx);
            }
            AuthorityChange::Closing => {
                cancel_remote_reads(&mut self.tasks);
            }
            AuthorityChange::Device => {}
            AuthorityChange::Rearm | AuthorityChange::RearmWithoutStop => {
                publish_privacy_epoch(false, &mut self.was_privacy_stopped, &self.privacy_epoch_tx);
            }
            AuthorityChange::HubLink
            | AuthorityChange::HubTransportReady
            | AuthorityChange::HubLinkOffline
            | AuthorityChange::HubDisconnected
            | AuthorityChange::Conversation
            | AuthorityChange::Interrupt => {}
        }
    }
}

pub(super) struct WorkRuntime {
    /// The one durable transfer worker (upload or finalize). A second job
    /// waits for its turn; this is not a queue limit.
    pub(super) worker: ticketed::Ticketed<()>,
    pub(super) retry_gate: upload_outcome::SharedGate,
    pub(super) finalize_cursor: Option<String>,
    pub(super) work_events: mpsc::Sender<WorkResult>,
    pub(super) durable_snapshot: DurableSnapshot,
    pub(super) force_snapshot_once: bool,
    pub(super) finalize_ready: BTreeMap<String, bool>,
}

pub(super) struct ConversationRuntime {
    pub(super) bridge_request: Option<BridgeAsk>,
    pub(super) interaction_response: Option<interactions::PendingResponse>,
    pub(super) interrupted_turns: interactions::InterruptedTurns,
    /// One voice turn worker; a second turn cannot start while one runs.
    pub(super) voice_task: ticketed::Ticketed<()>,
    pub(super) voice_operation: Option<String>,
    /// Voice turns left without a trace since the daemon started (nothing
    /// heard, or carried by the next turn): the count the log carries.
    pub(super) dropped_voice_turns: u64,
    /// One playback at a time; a replay replaces the current one.
    pub(super) speech_task: ticketed::Ticketed<()>,
    /// What the last reading in each voice said, by the agent whose voice
    /// it is (`AgentRoster::reading_voice`): `true` it played, `false` the
    /// hub's voice did not arrive. A voice not read yet is not here, which
    /// is unknown, not «no». One entry per voice that has read, so at most
    /// one per agent a hub has listed while the daemon runs.
    pub(super) speech_voices: BTreeMap<String, bool>,
    pub(super) speak_next: bool,
    pub(super) speech_notice_call_id: Option<String>,
    /// The answer the user asked to hear («Escuchar»), while it plays.
    pub(super) read_aloud: Option<speech::ReadAloud>,
    /// What the agent did in the turn in progress, or the last one
    /// (`chat.item`). Live only: a restart starts without it.
    pub(super) work: Option<orbit_core::agent_work::AgentWork>,
    /// The connected hub's attachment ceilings, from its last `ready`;
    /// `None` before one arrives and for a 0.9.0 hub, which has none.
    pub(super) hub_attachments: Option<orbit_core::attachments::AttachmentLimits>,
    /// The conversation the next new one takes over (`Command::Handover`),
    /// sent with its first `chat.send` and then forgotten.
    pub(super) handover_from: Option<String>,
    /// «Adjuntar ventana» while its capture runs (`window_attachment`).
    pub(super) window_attachment: Option<window_attachment::PendingWindow>,
}

pub(super) struct CaptureRuntime {
    pub(super) buffer: capture_lane::SplitBuffer,
    pub(super) pending_seals: capture_lane::SealQueue,
    /// The recording's lane, shared with the capture task.
    pub(super) lane: capture_lane::CaptureLane,
    /// Why the last seal the loop admitted failed, for the notice its caller
    /// says (`say_failure`).
    pub(super) seal_cause: Option<&'static str>,
    /// The bridge position of the last helper event the loop served: what
    /// the task handed over after it may be served.
    pub(super) served_control: u64,
    pub(super) reserve_exhausted: bool,
    pub(super) seal_retry_at: Option<std::time::Instant>,
    pub(super) audio: audio::AudioLifecycle,
    pub(super) call_runtime: Option<call::CallRuntime>,
    pub(super) mic_stream: Option<String>,
    pub(super) remote_stream: Option<String>,
    pub(super) pending_starts: Vec<String>,
    pub(super) retired_call_starts: BTreeMap<String, String>,
    pub(super) retired_stops: BTreeMap<String, audio::RetiredStreamStop>,
    pub(super) interruption: Option<orbit_core::RecordingInterruption>,
    /// The cut that left the recording interrupted, while it waits to
    /// resume or close (`cut_recovery`).
    pub(super) recovery: Option<cut_recovery::CutRecovery>,
}

pub(super) struct FeatureRuntime {
    pub(super) sources: sources::SourceRuntime,
    pub(super) calendar: calendar::CalendarRuntime,
    pub(super) computer: computer_control::ComputerRuntime,
    pub(super) meeting: meeting_runtime::MeetingRuntime,
    /// The call detection, its preference and the agenda around a call.
    pub(super) call: call_detection::CallRuntime,
    /// The question to Deimos during a meeting and the «Oye Deimos»
    /// detector's state.
    pub(super) question: meeting_question::QuestionRuntime,
    pub(super) native_host: native_host::NativeHost,
    pub(super) notes: recording_notes::NotesRuntime,
    /// The folders the user gave the assistant, and the helper's word on them.
    pub(super) files: assistant_files::FilesRuntime,
}
