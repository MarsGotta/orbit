//! P9-12: Deimos during a meeting. Mars asks with her voice («Oye Deimos,
//! …»), with «Preguntar a Deimos» in the pill or the floating panel, or with
//! the dictation shortcut while recording; the question goes to Deimos as a
//! turn about the meeting (SIM-30: the hub adds the meeting's last minutes
//! from its own transcript) and the floating panel follows that turn to its
//! answer or to its cause.
//!
//! Pure: what may be asked now and following the turn. Time and facts are
//! passed in; the daemon carries out the effects.
use crate::Connection;
use serde::{Deserialize, Serialize};

/// The windows of transcript Mars can choose from the button, in minutes.
/// Mars fixed 2 as the default for REF-602 (2026-09-29); P9-12 keeps it.
/// The app receives them from app-core, never copies them.
pub const WINDOW_MINUTES: [u8; 3] = [2, 5, 10];
pub const DEFAULT_WINDOW_MINUTES: u8 = 2;

/// The longest question, in bytes.
///
/// Protects: the recording topic's head (16 KiB, `topics::Recording`), where
/// the question travels beside the recording, its signal and up to 200 audio
/// applications. Why 1 KiB: a question spoken in the 30 s the voice allows
/// measures well under it (about 75 words, ~500 bytes in Spanish), and a
/// typed one is a sentence or two. It is not a limit on what Deimos can be
/// asked: a longer request belongs in the Asistente.
pub const MAX_QUESTION_BYTES: usize = 1024;

/// How long an answer stays in the floating panel after it arrives, unless
/// Mars closes it first. Forty-five seconds is long enough to
/// read a few lines during a call. It is not how long the answer is kept: it
/// stays in the conversation.
pub const ANSWER_SHOWN_MS: i64 = 45_000;

/// How long a failure stays in the panel: enough to read its cause, short
/// enough not to cover the call. Not a retry interval.
pub const FAILURE_SHOWN_MS: i64 = 8_000;

/// The pause that ends a spoken question («Oye Deimos, …» and then silence).
/// Decided in the helper (`NativeLimits.assistantQuestionPauseMilliseconds`,
/// compared by the constants guard); listed here so the registry names it.
/// It is not a recognition timeout.
pub const QUESTION_PAUSE_MS: u64 = 1_500;

/// The longest spoken question, from the phrase to its end: a question that
/// never pauses (a call heard through the speakers keeps the microphone busy)
/// is sent at this point with what was heard. Mirrored in the helper
/// (`NativeLimits.maximumAssistantQuestionMilliseconds`).
pub const MAX_SPOKEN_QUESTION_MS: u64 = 30_000;

/// How Mars asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionOrigin {
    /// «Oye Deimos, …» on her own microphone.
    Voice,
    /// «Preguntar a Deimos» in the pill or the panel.
    Typed,
    /// The dictation shortcut (⌃⌥Espacio) during a recording.
    Hotkey,
}

/// Where the turn is. «Pensando» and the answer are separate states, and a
/// failure always carries its cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionStage {
    Thinking,
    Answered,
    Failed,
}

/// What is already in progress when Mars asks: one word each, never one for
/// all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BusyTurn {
    /// Deimos is still answering the previous turn.
    AssistantTurn,
    /// The previous turn waits for Mars to allow or answer something.
    PendingApproval,
    /// A voice note is being recorded (⌃⌥Espacio held).
    VoiceNote,
    /// A voice note is being transcribed.
    VoiceTranscription,
    /// A voice call with Deimos is open.
    Call,
}

/// Why a question did not get its answer. Each one says its cause and its
/// numbers; none is «algo salió mal».
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cause", rename_all = "snake_case")]
pub enum QuestionCause {
    /// Mars's privacy stop is on.
    PrivacyStopped,
    /// There is no recording in progress to ask about.
    NoMeeting,
    /// The Mac has no link to the hub: `connection` says which state.
    HubUnreachable { connection: Connection },
    /// The hub answered that Deimos (Hermes) is not reachable from it.
    DeimosUnavailable { code: String },
    /// Something else is in progress.
    TurnInFlight { turn: BusyTurn },
    /// Nothing to ask.
    QuestionEmpty,
    /// The question is longer than [`MAX_QUESTION_BYTES`].
    QuestionTooLong { bytes: u64, max_bytes: u64 },
    /// A window the button does not offer.
    WindowNotOffered { minutes: u16, offered: Vec<u8> },
    /// Deimos ended the turn with an error.
    TurnFailed,
    /// Mars interrupted the turn.
    Interrupted,
    /// The hub refused the turn: its code and its message.
    HubRefused { code: String, message: String },
    /// The turn ended and no answer text arrived.
    NoAnswer,
    /// This device was revoked by the hub.
    Revoked,
}

/// What the admission looks at.
#[derive(Debug, Clone)]
pub struct AskFacts {
    pub privacy_stopped: bool,
    pub connection: Connection,
    /// The first thing in progress, if any.
    pub busy: Option<BusyTurn>,
    /// A recording is in progress (recording or paused).
    pub in_meeting: bool,
}

/// Whether Mars can ask this now. The order is the order in which she can
/// fix things: her stop first, then what she wrote, then the meeting, the
/// link and what is in progress.
pub fn admit(facts: &AskFacts, question: &str, minutes: u16) -> Result<(), QuestionCause> {
    if facts.privacy_stopped {
        return Err(QuestionCause::PrivacyStopped);
    }
    if clean_question(question).is_empty() {
        return Err(QuestionCause::QuestionEmpty);
    }
    if question.len() > MAX_QUESTION_BYTES {
        return Err(QuestionCause::QuestionTooLong {
            bytes: question.len() as u64,
            max_bytes: MAX_QUESTION_BYTES as u64,
        });
    }
    if !WINDOW_MINUTES
        .iter()
        .any(|offered| u16::from(*offered) == minutes)
    {
        return Err(QuestionCause::WindowNotOffered {
            minutes,
            offered: WINDOW_MINUTES.to_vec(),
        });
    }
    if !facts.in_meeting {
        return Err(QuestionCause::NoMeeting);
    }
    match facts.connection {
        Connection::Online => {}
        Connection::Revoked => return Err(QuestionCause::Revoked),
        Connection::Offline | Connection::Connecting => {
            return Err(QuestionCause::HubUnreachable {
                connection: facts.connection.clone(),
            });
        }
    }
    if let Some(turn) = facts.busy {
        return Err(QuestionCause::TurnInFlight { turn });
    }
    Ok(())
}

/// A question and where its turn is: what the floating panel shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeetingQuestion {
    /// The turn's request id; the answer is the message `{id}:assistant`.
    pub id: String,
    /// The recording asked about; absent when there was none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recording_id: Option<String>,
    pub question: String,
    pub minutes: u8,
    pub origin: QuestionOrigin,
    pub stage: QuestionStage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<QuestionCause>,
    pub asked_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<i64>,
    /// The question reached Deimos as a turn. A refusal before sending never
    /// did, and the panel must not say «Deimos no respondió» about it.
    #[serde(default)]
    pub sent: bool,
}

/// What the conversation says about a question's turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnFacts<'a> {
    /// The turn is still the one in flight.
    pub in_flight: bool,
    /// The complete answer, when it arrived.
    pub answer: Option<&'a str>,
    /// The cause the daemon heard before the turn ended (a hub error, an
    /// interruption, a lost link), if any.
    pub failure: Option<&'a QuestionCause>,
}

impl MeetingQuestion {
    /// A question that went to Deimos and waits for its answer.
    pub fn thinking(
        id: String,
        recording_id: String,
        question: String,
        minutes: u8,
        origin: QuestionOrigin,
        now_ms: i64,
    ) -> Self {
        Self {
            id,
            recording_id: Some(recording_id),
            question,
            minutes,
            origin,
            stage: QuestionStage::Thinking,
            cause: None,
            asked_at_ms: now_ms,
            finished_at_ms: None,
            sent: true,
        }
    }

    /// A question that never reached Deimos, with its cause. The text is
    /// kept at most at [`MAX_QUESTION_BYTES`] (the recording topic's head
    /// carries it); its original size is in the cause when that was the
    /// problem.
    pub fn refused(
        id: String,
        recording_id: Option<String>,
        question: String,
        minutes: u8,
        origin: QuestionOrigin,
        cause: QuestionCause,
        now_ms: i64,
    ) -> Self {
        Self {
            id,
            recording_id,
            question: at_most(question, MAX_QUESTION_BYTES),
            minutes,
            origin,
            stage: QuestionStage::Failed,
            cause: Some(cause),
            asked_at_ms: now_ms,
            finished_at_ms: Some(now_ms),
            sent: false,
        }
    }

    /// Follows the turn. Returns whether the question changed.
    pub fn follow(&mut self, turn: TurnFacts<'_>, now_ms: i64) -> bool {
        if self.stage != QuestionStage::Thinking || turn.in_flight {
            return false;
        }
        match turn.answer.filter(|answer| !answer.trim().is_empty()) {
            Some(_) => {
                self.stage = QuestionStage::Answered;
                self.cause = None;
            }
            None => {
                self.stage = QuestionStage::Failed;
                self.cause = Some(turn.failure.cloned().unwrap_or(QuestionCause::NoAnswer));
            }
        }
        self.finished_at_ms = Some(now_ms);
        true
    }

    /// Until when the panel shows it; `None` while Deimos is thinking (the
    /// panel stays until the answer or Mars closes it).
    pub fn shown_until_ms(&self) -> Option<i64> {
        let finished = self.finished_at_ms?;
        match self.stage {
            QuestionStage::Thinking => None,
            QuestionStage::Answered => Some(finished.saturating_add(ANSWER_SHOWN_MS)),
            QuestionStage::Failed => Some(finished.saturating_add(FAILURE_SHOWN_MS)),
        }
    }

    /// The panel has shown it long enough.
    pub fn expired(&self, now_ms: i64) -> bool {
        self.shown_until_ms().is_some_and(|until| now_ms >= until)
    }
}

/// The helper's «Oye Deimos» detector, as it last said (`voice_trigger`
/// native event). The daemon keeps the last one; before the helper says
/// anything it is unknown, never «not available».
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum VoiceTrigger {
    /// This Mac cannot listen for the phrase: the helper's cause code
    /// (`macos_version`, `locale_unsupported`, …) and, for the version, the
    /// macOS it runs. The button still works.
    Unavailable {
        cause: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        os_version: Option<String>,
    },
    /// Available and not listening: no recording in progress.
    Idle,
    /// Getting the Spanish speech model, once, before the first listen.
    Downloading {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        percent: Option<u8>,
    },
    /// Listening on Mars's microphone for this recording.
    Listening,
    /// It stopped listening: the helper's cause code and Apple's detail,
    /// which never carries what was heard.
    Failed {
        cause: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
}

/// The longest detail the daemon keeps from the helper's `voice_trigger`:
/// an error description from Apple's frameworks, a sentence. It protects the
/// recording topic's head; it is not a log line limit.
pub const MAX_VOICE_TRIGGER_DETAIL_BYTES: usize = 256;

/// `text` cut at a character boundary to at most `max` bytes.
fn at_most(mut text: String, max: usize) -> String {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

/// What decides whether «Oye Deimos» listens now.
#[derive(Debug, Clone)]
pub struct VoiceFacts<'a> {
    /// The phase of the open recording, if any.
    pub recording: Option<crate::CapturePhase>,
    pub privacy_stopped: bool,
    pub helper_connected: bool,
    /// The helper announced `audio_wake_phrase` in `ready`.
    pub helper_listens: bool,
    /// The daemon asked the helper to listen on the meeting's microphone.
    pub requested: bool,
    /// The helper's last `voice_trigger`.
    pub report: Option<&'a VoiceTrigger>,
}

/// Why «Oye Deimos» does not listen during a recording, with its numbers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cause", rename_all = "snake_case")]
pub enum NotListening {
    PrivacyStopped,
    Paused,
    /// Needs macOS 26; the Mac runs `os_version`.
    MacosVersion {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        os_version: Option<String>,
    },
    LocaleUnsupported,
    /// The helper said it cannot, with its code.
    Unavailable {
        code: String,
    },
    /// It stopped: the helper's code and Apple's detail.
    Failed {
        code: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    /// The Mac's helper is not connected.
    HelperDisconnected,
    /// The connected helper predates the phrase (no `audio_wake_phrase`).
    HelperWithoutPhrase,
    /// The recording has no microphone stream to listen on yet.
    NoMicrophone,
}

/// «Oye Deimos» during a recording, as the pill and the panel say it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum VoiceStatus {
    Listening,
    /// Asked to listen; the helper has not said it does yet.
    Starting,
    Downloading {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        percent: Option<u8>,
    },
    NotListening {
        why: NotListening,
    },
}

/// Whether «Oye Deimos» listens, and why not. `None` outside a recording:
/// there is nothing to say then (and an idle detector is not a cause).
pub fn voice_status(facts: &VoiceFacts<'_>) -> Option<VoiceStatus> {
    use crate::CapturePhase;
    let paused = match facts.recording.as_ref()? {
        CapturePhase::Starting | CapturePhase::Recording => false,
        CapturePhase::Paused => true,
        CapturePhase::Idle | CapturePhase::Finalizing | CapturePhase::Interrupted => return None,
    };
    let off = |why| Some(VoiceStatus::NotListening { why });
    if facts.privacy_stopped {
        return off(NotListening::PrivacyStopped);
    }
    // A Mac that can never listen says so even while paused.
    if let Some(VoiceTrigger::Unavailable { cause, os_version }) = facts.report {
        return off(match cause.as_str() {
            "macos_version" => NotListening::MacosVersion {
                os_version: os_version.clone(),
            },
            "locale_unsupported" => NotListening::LocaleUnsupported,
            other => NotListening::Unavailable { code: other.into() },
        });
    }
    if paused {
        return off(NotListening::Paused);
    }
    if !facts.helper_connected {
        return off(NotListening::HelperDisconnected);
    }
    if !facts.helper_listens {
        return off(NotListening::HelperWithoutPhrase);
    }
    if !facts.requested {
        return off(NotListening::NoMicrophone);
    }
    match facts.report {
        Some(VoiceTrigger::Listening) => Some(VoiceStatus::Listening),
        Some(VoiceTrigger::Downloading { percent }) => {
            Some(VoiceStatus::Downloading { percent: *percent })
        }
        Some(VoiceTrigger::Failed { cause, detail }) => off(NotListening::Failed {
            code: cause.clone(),
            detail: detail.clone(),
        }),
        Some(VoiceTrigger::Idle | VoiceTrigger::Unavailable { .. }) | None => {
            Some(VoiceStatus::Starting)
        }
    }
}

/// Mars's question, trimmed as it goes to Deimos.
pub fn clean_question(question: &str) -> &str {
    question.trim()
}

/// Whether two questions are the same words, whatever their capitals,
/// accents-free marks and punctuation: the transcription can write the same
/// phrase as «¿De qué hablamos?» and «de qué hablamos».
pub fn same_question(a: &str, b: &str) -> bool {
    let words = |text: &str| {
        text.chars()
            .filter(|character| character.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let a = words(a);
    !a.is_empty() && a == words(b)
}

#[cfg(test)]
mod tests;
