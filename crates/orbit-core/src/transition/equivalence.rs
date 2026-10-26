//! The new `decide` decides exactly as main's did before the state was split
//! by domain. main's `decide` is kept here, test-only (`legacy/`), on main's
//! flat state; a seeded corpus of event sequences runs through both, and
//! every step must give the same refusal, or the same effects, authority
//! change and resulting state, in memory and as written.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::{Event, decide};
use crate::attachments::{Attachment, AttachmentKind};
use crate::calendar::CalendarAuthorization;
use crate::failure::DegradedKind;
use crate::model::*;
use crate::notice::NoticeKey;
use crate::protocol::MeetingAudio;
use crate::transition::{AssistantState, CaptureState, HubLinkState, PrivacyState, VoiceState};
use crate::voice::{CallOrigin, CallPhase, ShortVoiceNote};
use std::collections::BTreeMap;

mod legacy {
    pub(super) mod decide;
    pub(super) mod model;
}

/// Sequences in the corpus, and events in each.
const SEQUENCES: u64 = 1_000;
const STEPS: usize = 120;
/// Each kind of event is accepted at least this often in the corpus.
const MIN_ACCEPTED: u64 = 25;

/// SplitMix64: a fixed seed gives the same corpus on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
    fn pick<T: Clone>(&mut self, items: &[T]) -> T {
        items[self.below(items.len() as u64) as usize].clone()
    }
}

fn to_legacy(state: &State) -> legacy::model::State {
    let d = &state.durable;
    legacy::model::State {
        durable: legacy::model::DurableState {
            schema: d.schema,
            generation: d.privacy.generation,
            privacy_stopped: d.privacy.stopped,
            privacy_stop_by: d.privacy.stop_by,
            connection: d.link.connection.clone(),
            session_id: d.assistant.session_id.clone(),
            agent_id: d.assistant.agent_id.clone(),
            active_request: d.assistant.active_request.clone(),
            turn: d.assistant.turn.clone(),
            pending_voice: d.assistant.pending_voice.clone(),
            recording: d.capture.recording.clone(),
            discarding: d.capture.discarding.clone(),
            voice_active: d.voice.voice_active,
            call: d.voice.call.clone(),
            pending_approval: d.assistant.pending_approval.clone(),
            capabilities: d.capabilities.clone(),
            notice: d.notice.clone(),
            notice_log: d.notice_log.clone(),
            pending_jobs: d.pending_jobs,
        },
        views: state.views.clone(),
    }
}

fn from_legacy(state: legacy::model::State) -> State {
    let d = state.durable;
    State {
        durable: DurableState {
            schema: d.schema,
            privacy: PrivacyState {
                generation: d.generation,
                stopped: d.privacy_stopped,
                stop_by: d.privacy_stop_by,
            },
            link: HubLinkState {
                connection: d.connection,
            },
            assistant: AssistantState {
                session_id: d.session_id,
                agent_id: d.agent_id,
                active_request: d.active_request,
                turn: d.turn,
                pending_voice: d.pending_voice,
                pending_approval: d.pending_approval,
            },
            capture: CaptureState {
                recording: d.recording,
                discarding: d.discarding,
            },
            voice: VoiceState {
                voice_active: d.voice_active,
                call: d.call,
            },
            capabilities: d.capabilities,
            notice: d.notice,
            notice_log: d.notice_log,
            pending_jobs: d.pending_jobs,
        },
        views: state.views,
    }
}

/// Where a sequence starts: nothing yet, a Mac ready for everything, or a
/// state main saved (`tests/fixtures/durable`), as written and as a restart
/// reads it.
fn starts() -> Vec<State> {
    let saved = [
        include_str!("../../tests/fixtures/durable/live_recording.json"),
        include_str!("../../tests/fixtures/durable/conversation_pending_voice.json"),
        include_str!("../../tests/fixtures/durable/pending_voice_offline_outbox.json"),
        include_str!("../../tests/fixtures/durable/call.json"),
        include_str!("../../tests/fixtures/durable/discard.json"),
        include_str!("../../tests/fixtures/durable/privacy_stop.json"),
        include_str!("../../tests/fixtures/durable/revoked.json"),
        include_str!("../../tests/fixtures/durable/cut_closed.json"),
    ];
    let mut starts = vec![State::default(), ready()];
    for json in saved {
        let durable: DurableState = serde_json::from_str(json).unwrap();
        starts.push(State::from(durable.clone()));
        starts.push(State::from(durable.recovered()));
    }
    starts
}

fn ready() -> State {
    let mut state = State::default();
    state.durable.capabilities.native_helper = true;
    state.durable.capabilities.microphone_state = PermissionState::Granted;
    state.durable.capabilities.screen_state = PermissionState::Granted;
    state.durable.capabilities.voice_processing_supported = true;
    state.durable.link.connection = Connection::Online;
    state
}

#[test]
fn unchanged_rules_match_the_flat_model_after_duration_retirement() {
    let mut rng = Rng(0x0A1B_2C3D_4E5F_6071);
    let starts = starts();
    let mut accepted: BTreeMap<String, u64> = BTreeMap::new();
    let (mut refused, mut restarts, mut steps) = (0u64, 0u64, 0u64);
    for sequence in 0..SEQUENCES {
        let mut state = starts[(sequence % starts.len() as u64) as usize].clone();
        let mut old = to_legacy(&state);
        let mut now_ms: i64 = 1_790_000_000_000;
        for step in 0..STEPS {
            now_ms = advance(&mut rng, now_ms);
            if rng.chance(1) {
                restarts += 1;
                state = state.recovered();
                old = old.recovered();
                assert_same(&state, &old, sequence, step, "recovered");
                continue;
            }
            if rng.chance(5) {
                let supported = rng.chance(85);
                state.set_voice_processing_supported(supported);
                old.durable.capabilities.voice_processing_supported = supported;
            }
            let event = event(&mut rng, &state);
            let name = variant(&event);
            let new = decide(&state, event.clone(), now_ms);
            let navigated = matches!(event, Event::NewSession { .. } | Event::OpenSession { .. });
            let mut main = legacy::decide::decide(&old, event, now_ms);
            // Navigation now detaches the selected turn. The frozen pre-split
            // reducer retained that identity; every other decision stays equivalent.
            if navigated && let Ok(main) = &mut main {
                main.state.durable.active_request = None;
            }
            steps += 1;
            match (new, main) {
                (Err(new), Err(main)) => {
                    assert_eq!(new, main, "sequence {sequence} step {step}: {name}");
                    refused += 1;
                }
                (Ok(new), Ok(main)) => {
                    let at = format!("sequence {sequence} step {step}: {name}");
                    assert_eq!(new.effects, main.effects, "{at}");
                    assert_eq!(new.authority_change, main.authority_change, "{at}");
                    assert_same(&new.state, &main.state, sequence, step, &name);
                    state = new.state;
                    old = main.state;
                    *accepted.entry(name).or_default() += 1;
                }
                (new, main) => panic!(
                    "sequence {sequence} step {step}: {name}: new {:?}, main {:?}",
                    new.map(|_| ()),
                    main.map(|_| ())
                ),
            }
        }
    }
    // The corpus reaches every event, accepted, and refusals too.
    let rare: Vec<&str> = EVENTS
        .iter()
        .copied()
        .filter(|name| accepted.get(*name).copied().unwrap_or_default() < MIN_ACCEPTED)
        .collect();
    assert!(
        rare.is_empty(),
        "accepted too rarely: {rare:?}\n{accepted:?}"
    );
    assert!(refused > steps / 10, "{refused} refusals in {steps} steps");
    assert!(restarts > 100, "{restarts} restarts");
}

fn assert_same(new: &State, main: &legacy::model::State, sequence: u64, step: usize, name: &str) {
    assert_eq!(
        serde_json::to_string(new).unwrap(),
        serde_json::to_string(main).unwrap(),
        "sequence {sequence} step {step}: {name}: written differently"
    );
    assert_eq!(
        *new,
        from_legacy(main.clone()),
        "sequence {sequence} step {step}: {name}"
    );
}

/// Mostly a moment, sometimes the drain's or a call's wait, now and then
/// the recording's ceiling, and a wall clock that goes back.
fn advance(rng: &mut Rng, now_ms: i64) -> i64 {
    let delta = match rng.below(100) {
        0..=59 => rng.below(5_000) as i64,
        60..=79 => rng.below(20_000) as i64,
        80..=91 => rng.below(600_000) as i64,
        92..=95 => 3_000_000 + rng.below(1_000_000) as i64,
        96..=97 => 14_000_000 + rng.below(1_000_000) as i64,
        _ => -(rng.below(5_000) as i64),
    };
    now_ms.saturating_add(delta)
}

const EVENTS: [&str; 53] = [
    "Connect",
    "Connecting",
    "Submit",
    "SubmitVoice",
    "VoiceStored",
    "VoiceFailed",
    "VoiceRetry",
    "VoiceDiscard",
    "VoiceUnheard",
    "VoiceTooShort",
    "Session",
    "HubTransportReady",
    "HubDisconnected",
    "NewSession",
    "OpenSession",
    "DeviceChanged",
    "Delta",
    "Completed",
    "Approval",
    "Interrupt",
    "StartMeeting",
    "CaptureStarted",
    "CaptureFinalized",
    "EmptyCaptureClosed",
    "CutRecordingClosed",
    "PauseMeeting",
    "ResumeMeeting",
    "FinishMeeting",
    "DiscardMeeting",
    "ChunkSealed",
    "CaptureGap",
    "VoiceStart",
    "VoiceEnd",
    "VoiceCancel",
    "CallStart",
    "CallStop",
    "CallReady",
    "CallPhase",
    "CallTurnStarted",
    "PrivacyStop",
    "HelperLatched",
    "LocalRearm",
    "HelperReady",
    "HelperLost",
    "PermissionObserved",
    "CalendarAuthorization",
    "HelperStopUndelivered",
    "Tick",
    "Failure",
    "Degraded",
    "CaptureRefused",
    "CaptureCut",
    "Close",
];

fn variant(event: &Event) -> String {
    format!("{event:?}")
        .split(|c: char| !c.is_ascii_alphanumeric())
        .next()
        .unwrap_or_default()
        .to_owned()
}

mod events;
use events::event;
