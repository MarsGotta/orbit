//! Speaking while the previous turn finishes never cuts the call and
//! never loses what the user said, and what a stop cancels on purpose is not a
//! failure. Synthetic frames of 100 ms (800 samples at 8 kHz); every fact is
//! a frame or a helper reply given by hand, nothing waits on a clock.
use super::*;
use orbit_core::notice::NoticeClass;

/// One frame's PCM, in bytes.
const FRAME_BYTES: usize = 800 * 2;

struct Call {
    runtime: Runtime,
    helper: mpsc::Receiver<Value>,
    stream: String,
    sequence: u64,
    _directory: tempfile::TempDir,
}

impl Call {
    async fn open() -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut helper) = fixture(directory.path())?;
        runtime.link.token = Some("synthetic-token".into());
        let stream = start_call_for(&mut runtime, &mut helper, 600_000).await?;
        Ok(Self {
            runtime,
            helper,
            stream,
            sequence: 0,
            _directory: directory,
        })
    }

    /// `count` frames of voice or silence, as the helper sends them.
    async fn frames(&mut self, count: u64, voiced: bool) -> Result<()> {
        for _ in 0..count {
            let mut frame = call_frame(&self.stream, self.sequence, voiced);
            mark_frame(&mut frame, self.sequence, voiced);
            self.sequence += 1;
            self.runtime.native_event(frame).await?;
        }
        Ok(())
    }

    /// `count` frames of voice or silence, 100 ms apart from `at_ms` on, as
    /// they arrive: the call's own clock, not the wall's.
    async fn frames_at(&mut self, at_ms: i64, count: u64, voiced: bool) -> Result<()> {
        for index in 0..count {
            let mut frame = call_frame(&self.stream, self.sequence, voiced);
            self.sequence += 1;
            self.runtime
                .on_native_audio_at(&mut frame, at_ms + index as i64 * 100)
                .await?;
        }
        Ok(())
    }

    fn started_at_ms(&self) -> Result<i64> {
        Ok(self
            .runtime
            .state
            .durable
            .voice
            .call
            .as_ref()
            .context("call absent")?
            .started_at_ms)
    }

    /// `count` frames while Deimos's answer plays, as the helper sends them.
    async fn frames_over_answer(&mut self, count: u64, voiced: bool) -> Result<()> {
        for _ in 0..count {
            let mut frame = call_frame(&self.stream, self.sequence, voiced);
            mark_frame(&mut frame, self.sequence, voiced);
            frame["playback_active"] = json!(true);
            self.sequence += 1;
            self.runtime.native_event(frame).await?;
        }
        Ok(())
    }

    /// The helper orders sent since the last look: their command names.
    fn helper_orders(&mut self) -> Vec<String> {
        let mut orders = Vec::new();
        while let Ok(order) = self.helper.try_recv() {
            orders.push(order["command"].as_str().unwrap_or_default().to_owned());
        }
        orders
    }

    /// The hub's answer to the transcription in course: `text`.
    async fn transcribed(&mut self, text: &str) -> Result<String> {
        let operation = self.operation().context("no transcription in course")?;
        let ticket = self
            .runtime
            .assistant
            .voice_task
            .ticket()
            .cloned()
            .context("worker ticket")?;
        let result = crate::daemon::CaptureWork::Transcribed {
            ticket,
            source: crate::daemon::audio::VoiceSource {
                recording_id: operation.clone(),
                chunk_id: format!("{operation}-mic_local-0"),
                session_id: None,
                message_id: operation.clone(),
                duration_ms: 1_000,
            },
            text: text.into(),
        };
        self.runtime.work_event(WorkResult::Capture(result)).await?;
        Ok(operation)
    }

    /// A turn: 400 ms of the user's voice, then the 800 ms of silence that end it.
    async fn turn(&mut self) -> Result<()> {
        self.frames(4, true).await?;
        self.frames(8, false).await?;
        Ok(())
    }

    fn operation(&self) -> Option<String> {
        self.runtime.assistant.voice_operation.clone()
    }

    fn capturing(&self) -> bool {
        self.runtime
            .audio
            .call
            .as_ref()
            .is_some_and(|call| call.detector.is_capturing())
    }

    fn alive(&self) -> bool {
        self.runtime.state.durable.voice.call.is_some()
            && self
                .runtime
                .audio
                .call
                .as_ref()
                .is_some_and(|call| call.closing_at_ms.is_none())
    }

    fn voice_pcm(&self, operation: &str) -> Result<Vec<u8>> {
        let wav = self
            .runtime
            .fixture_store
            .read_chunk(&format!("{operation}-mic_local-0"))?;
        Ok(wav[44..].to_vec())
    }
}

/// A second real call (2026-10-18, Orbit 0.6.0): the user's turn was being
/// transcribed when they spoke again a second later. That second turn must
/// end and carry the first one with it; when they speak a third time the
/// call is still there.
#[tokio::test]
async fn speaking_while_a_turn_is_transcribed_keeps_the_call_and_both_turns() -> Result<()> {
    let mut call = Call::open().await?;
    call.turn().await?;
    let first = call.operation().context("the first turn was not sent")?;
    assert!(call.runtime.assistant.voice_task.active().is_some());
    let first_pcm = call.voice_pcm(&first)?;

    // The user speaks again while it is transcribed: 200 ms of voice begin a turn.
    call.frames(2, true).await?;
    assert!(call.capturing(), "the new turn began and was cut at once");
    // The first turn goes on in this one: it leaves no note of its own
    // (Orbit 0.11.0 left one reading «Va junto con lo que dijiste a
    // continuación», shown in every conversation).
    let state = &call.runtime.state.durable;
    assert!(
        state
            .assistant
            .turn
            .as_ref()
            .is_none_or(|turn| turn.user.id != first)
            && state
                .assistant
                .pending_voice
                .iter()
                .all(|note| note.message.id != first),
        "the joined turn left a note: {:?}",
        state.assistant.turn
    );
    call.frames(2, true).await?;
    call.frames(8, false).await?;
    let second = call.operation().context("the second turn never ended")?;
    assert_ne!(second, first);
    let second_pcm = call.voice_pcm(&second)?;
    assert!(
        second_pcm.starts_with(&first_pcm),
        "the first turn's voice is not in the second"
    );
    // The frames after the first turn, each once: 1 of silence that closed
    // its preroll, 4 of voice and the 7 of silence that ended it.
    assert_eq!(second_pcm.len() - first_pcm.len(), 12 * FRAME_BYTES);

    // Later the user speaks a third time: the call goes on.
    call.frames(4, true).await?;
    assert!(call.alive(), "the call was cut");
    assert!(call.capturing());
    Ok(())
}

/// A barge-in over Deimos's answer (the turn already reached the hub) is a
/// turn of its own that ends and is sent, and the call keeps listening.
#[tokio::test]
async fn a_turn_that_interrupts_deimos_ends_and_is_sent() -> Result<()> {
    let mut call = Call::open().await?;
    call.runtime.state.durable.assistant.active_request = Some("answer-in-course".into());
    call.frames(2, true).await?;
    assert!(
        call.capturing(),
        "the barge-in turn began and was cut at once"
    );
    call.frames(2, true).await?;
    call.frames(8, false).await?;
    assert!(call.operation().is_some(), "the barge-in turn never ended");
    call.frames(4, true).await?;
    assert!(call.alive());
    Ok(())
}

/// Deimos is speaking and the microphone hears a second of noise: his answer
/// goes on, no turn is created and nothing is left in the conversation
/// (2026-10-23: such clips cut him off and stayed as failed notes).
#[tokio::test]
async fn noise_over_deimos_answer_stops_nothing_and_leaves_nothing() -> Result<()> {
    let mut call = Call::open().await?;
    call.helper_orders();
    let before = call.runtime.state.durable.notice_log.records().len();
    let turns = call
        .runtime
        .state
        .durable
        .voice
        .call
        .as_ref()
        .map(|call| call.turns);
    // A second of sound loud enough to open a turn, then the silence that ends it.
    call.frames_over_answer(10, true).await?;
    call.frames_over_answer(8, false).await?;
    assert!(
        !call
            .helper_orders()
            .iter()
            .any(|order| order == "playback_stop"),
        "Deimos was cut off before anything was heard"
    );
    assert!(
        call.runtime.state.durable.assistant.turn.is_none(),
        "a turn was created"
    );
    // The hub hears no words in it.
    call.transcribed("").await?;
    assert!(
        !call
            .helper_orders()
            .iter()
            .any(|order| order == "playback_stop")
    );
    let state = &call.runtime.state.durable;
    assert!(
        state.assistant.turn.is_none(),
        "a turn was created: {:?}",
        state.assistant.turn
    );
    assert!(state.assistant.pending_voice.is_empty(), "a note was left");
    assert!(failed_since(&call.runtime, before).is_empty());
    assert_eq!(
        call.runtime
            .state
            .durable
            .voice
            .call
            .as_ref()
            .map(|call| call.turns),
        turns,
        "the noise counted as a turn of the call"
    );
    assert!(call.alive());
    Ok(())
}

/// Words said over Deimos's answer stop him once they are known, and are
/// the turn that is sent.
#[tokio::test]
async fn words_over_deimos_answer_stop_him_and_are_sent() -> Result<()> {
    let mut call = Call::open().await?;
    let _hub_commands = call.runtime.link.outbound.open(16);
    call.helper_orders();
    call.frames_over_answer(4, true).await?;
    call.frames_over_answer(8, false).await?;
    assert!(
        !call
            .helper_orders()
            .iter()
            .any(|order| order == "playback_stop")
    );
    let operation = call.transcribed("Espera un momento").await?;
    assert!(
        call.helper_orders()
            .iter()
            .any(|order| order == "playback_stop"),
        "the words did not stop Deimos"
    );
    let turn = call
        .runtime
        .state
        .durable
        .assistant
        .turn
        .as_ref()
        .context("no turn")?;
    assert_eq!(turn.user.id, operation);
    assert_eq!(turn.user.text, "Espera un momento");
    assert!(
        call.runtime.voice_note_audio(&operation).is_ok(),
        "its audio is kept"
    );
    assert!(
        call.runtime
            .state
            .durable
            .assistant
            .active_request
            .is_some(),
        "not sent"
    );
    Ok(())
}

/// A turn in which the hub heard no words leaves no note, and its audio goes
/// first in what the user says next: they had not finished.
#[tokio::test]
async fn a_turn_without_words_waits_for_the_next_one() -> Result<()> {
    let mut call = Call::open().await?;
    call.turn().await?;
    let first = call.operation().context("the first turn was not sent")?;
    let first_pcm = call.voice_pcm(&first)?;
    let before = call.runtime.state.durable.notice_log.records().len();
    call.transcribed("").await?;
    let state = &call.runtime.state.durable;
    assert!(
        state.assistant.turn.is_none(),
        "the silent turn stayed: {:?}",
        state.assistant.turn
    );
    assert!(state.assistant.pending_voice.is_empty());
    assert!(failed_since(&call.runtime, before).is_empty());
    assert!(
        call.voice_pcm(&first).is_err(),
        "its audio stayed on the Mac"
    );
    call.turn().await?;
    let second = call.operation().context("the second turn was not sent")?;
    assert!(
        call.voice_pcm(&second)?.starts_with(&first_pcm),
        "the silent turn's audio is not in the next one"
    );
    Ok(())
}

fn failed_since(runtime: &Runtime, before: usize) -> Vec<orbit_core::notice::Notice> {
    runtime.state.durable.notice_log.records()[before..]
        .iter()
        .filter(|notice| notice.class() == NoticeClass::Failed)
        .cloned()
        .collect()
}

/// A call is closed while a chunk of it is still being written
/// (the real cut: `capture_stop` answered with «El audio anterior todavía está
/// drenándose»). The closure waits for the chunk; it is not a failure, and
/// the next maintenance closes the call once the chunk is in.
#[tokio::test]
async fn closing_a_call_whose_audio_is_still_sealing_waits_instead_of_failing() -> Result<()> {
    let mut call = Call::open().await?;
    call.turn().await?;
    call.runtime.apply(Event::CallStop).await?;
    // The helper's drain is confirmed and every real chunk is in.
    call.runtime.audio.lifecycle.stops.clear();
    call.runtime.audio.mic_stream = None;
    let segments = call.runtime.capture.buffer.pause_source("mic_local")?;
    call.runtime.seal(segments)?;
    crate::daemon::audio::tests::await_seal_receipts(&mut call.runtime)?;
    // One chunk of this call that the store has not confirmed yet.
    call.runtime
        .capture
        .pending_seals
        .push_back(crate::capture::Segment {
            metadata: orbit_store::ChunkMetadata {
                capture_epoch_id: None,
                id: "synthetic-call-mic_local-unsealed".into(),
                recording_id: "synthetic-call".into(),
                track: "mic_local".into(),
                sequence: 99,
                sample_rate: 8000,
                channels: 1,
                frames: 800,
                monotonic_ns: 0,
                first_sample: 0,
                started_at_ms: 0,
            },
            pcm: vec![0; FRAME_BYTES],
        });
    let before = call.runtime.state.durable.notice_log.records().len();
    call.runtime.finish_call_drain().await?;
    assert!(
        call.runtime.audio.call.is_some(),
        "closed with a chunk unwritten"
    );
    assert!(failed_since(&call.runtime, before).is_empty());
    // The chunk lands; the next maintenance closes the call.
    call.runtime.capture.pending_seals.remove(0);
    call.runtime.finish_call_drain().await?;
    assert!(call.runtime.audio.call.is_none());
    assert!(
        call.runtime
            .fixture_store
            .get_job("finalize-synthetic-call")?
            .is_some()
    );
    Ok(())
}

/// A cut stops every source; the helper then cancels the call's
/// own `audio_stop` because the whole stop overtook it. That cancellation is
/// the stop doing its job, not a refusal of the Mac.
#[tokio::test]
async fn an_audio_stop_cancelled_by_the_whole_stop_is_not_a_failure() -> Result<()> {
    let mut call = Call::open().await?;
    call.frames(3, false).await?;
    call.runtime.apply(Event::CallStop).await?;
    let audio_stop = loop {
        let command = call.helper.recv().await.context("audio stop absent")?;
        if command["command"] == "audio_stop" {
            break command;
        }
    };
    call.runtime
        .request_full_audio_stop(crate::daemon::audio::FullStop::Cut)
        .await?;
    let before = call.runtime.state.durable.notice_log.records().len();
    call.runtime
        .native_event(json!({"event":"response","id":audio_stop["id"],"ok":false,
            "error":{"code":"operation_cancelled_by_stop","message":"synthetic"}}))
        .await?;
    assert!(
        failed_since(&call.runtime, before).is_empty(),
        "{:?}",
        failed_since(&call.runtime, before)
    );
    Ok(())
}

fn published_notices(runtime: &Runtime) -> Result<Value> {
    use orbit_core::topics::TopicId;
    let mut board = crate::daemon::topics::TopicBoard::new("call-notices".into());
    let receiver = board.channels()[&TopicId::Notices].subscribe();
    board.publish(&crate::daemon::topics::TopicSource {
        state: &runtime.state,
        runtime: Some(runtime),
        now_ms: 0,
    });
    Ok(serde_json::to_value(&receiver.borrow().data)?["knowledge"]["value"].clone())
}

#[tokio::test]
async fn call_notices_have_their_op_before_publication_and_stay_immutable() -> Result<()> {
    use orbit_core::notice::{Op, ReportSource};
    let mut call = Call::open().await?;
    call.runtime.report_log = Some(orbit_ipc::report::ReportLog::daemon(call._directory.path()));
    let op = Op::parse("call-transcription-one").context("valid op")?;
    call.runtime.begin_turn(op.clone(), ReportSource::Daemon);
    call.turn().await?;
    let before = published_notices(&call.runtime)?;
    call.runtime.begin_turn(
        Op::parse("call-following-frame").context("valid op")?,
        ReportSource::Daemon,
    );
    let after = published_notices(&call.runtime)?;
    let saved = before["notices"]
        .as_array()
        .context("notices")?
        .iter()
        .find(|notice| notice["code"] == "voice_note_saved")
        .context("saved notice")?;
    assert_eq!(
        saved["op"],
        op.as_str(),
        "the core's notice was published without its op"
    );
    assert_eq!(
        before, after,
        "an already published notice changed in the following turn"
    );
    let lines = std::fs::read_to_string(call._directory.path().join("logs/report-daemon.log"))?;
    let records: Vec<Value> = lines
        .lines()
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    for code in ["voice_note_saved", "voice_transcribing"] {
        let matching: Vec<_> = records
            .iter()
            .filter(|notice| notice["code"] == code)
            .collect();
        assert_eq!(matching.len(), 1, "each notice is reported once");
        assert_eq!(matching[0]["op"], op.as_str());
    }
    Ok(())
}

#[tokio::test]
async fn joining_a_call_turn_closes_the_transcription_it_replaces() -> Result<()> {
    use orbit_core::notice::{Op, ReportSource};
    let mut call = Call::open().await?;
    let mut frames = Vec::new();
    let mut unfinished = Vec::new();
    for index in 0..3 {
        let op = Op::parse(&format!("call-transcription-{index}")).context("valid op")?;
        call.runtime.begin_turn(op.clone(), ReportSource::Daemon);
        call.turn().await?;
        call.runtime.report_notices();
        frames.push(published_notices(&call.runtime)?);
        call.runtime.begin_turn(
            Op::parse(&format!("call-barge-in-{index}")).context("valid op")?,
            ReportSource::Daemon,
        );
        call.frames(2, true).await?;
        call.runtime.report_notices();
        frames.push(published_notices(&call.runtime)?);
        let closed = call
            .runtime
            .state
            .durable
            .notice_log
            .records()
            .iter()
            .any(|notice| {
                notice.code == orbit_core::notice::NoticeCode::VoiceTurnCancelled
                    && notice.op.as_ref() == Some(&op)
            });
        if !closed {
            unfinished.push(op);
        }
        call.frames(2, true).await?;
    }
    assert!(
        unfinished.is_empty(),
        "transcriptions without a terminal notice: {unfinished:?}"
    );
    let expected: Value = serde_json::from_str(include_str!(
        "../../../../contracts/fixtures/call-notice-frames.json"
    ))?;
    assert_eq!(
        json!(frames),
        expected,
        "the UI consumes these actual call frames"
    );
    Ok(())
}

#[tokio::test]
async fn a_call_transcription_always_closes_even_after_another_operations_notice() -> Result<()> {
    use orbit_core::notice::{NoticeCode, Op, ReportSource};
    for succeeded in [true, false] {
        let mut call = Call::open().await?;
        let _hub_commands = call.runtime.link.outbound.open(16);
        let op = Op::parse("call-transcription-result").context("valid op")?;
        call.runtime.begin_turn(op.clone(), ReportSource::Daemon);
        call.turn().await?;
        let operation = call.operation().context("transcription operation")?;
        let ticket = call
            .runtime
            .assistant
            .voice_task
            .ticket()
            .cloned()
            .context("worker ticket")?;
        call.runtime.begin_turn(
            Op::parse("unrelated-audio-gap").context("valid op")?,
            ReportSource::Daemon,
        );
        call.runtime
            .apply(Event::Degraded(orbit_core::failure::DegradedKind::AudioGap))
            .await?;
        call.runtime.begin_turn(op.clone(), ReportSource::Daemon);
        let result = if succeeded {
            crate::daemon::CaptureWork::Transcribed {
                ticket,
                source: crate::daemon::audio::VoiceSource {
                    recording_id: operation.clone(),
                    chunk_id: format!("{operation}-mic_local-0"),
                    session_id: None,
                    message_id: operation,
                    duration_ms: 1_200,
                },
                text: "Voz sintética".into(),
            }
        } else {
            crate::daemon::CaptureWork::VoiceProblem {
                ticket,
                operation_id: operation,
                failure: orbit_core::VoiceTranscriptionFailure::local("synthetic_stt_failed"),
            }
        };
        call.runtime.work_event(WorkResult::Capture(result)).await?;
        let code = if succeeded {
            NoticeCode::AssistantThinking
        } else {
            NoticeCode::VoiceTranscriptionFailed
        };
        assert!(
            call.runtime
                .state
                .durable
                .notice_log
                .records()
                .iter()
                .any(|notice| notice.code == code && notice.op.as_ref() == Some(&op)),
            "the result must close its own working notice even after another op spoke"
        );
    }
    Ok(())
}

/// A forgotten call in a room with a fan that starts now and then: every
/// burst of noise opens a turn, the hub finds no words in it, and the call
/// still closes after an unanswered question. Noise cannot answer it.
#[tokio::test]
async fn a_forgotten_noisy_call_asks_and_then_hangs_up_without_words() -> Result<()> {
    let mut call = Call::open().await?;
    let started = call.started_at_ms()?;
    for burst in 0..5 {
        let at = started + burst * 10_000;
        call.frames_at(at, 4, true).await?;
        call.frames_at(at + 400, 8, false).await?;
        call.transcribed("").await?;
        assert!(call.alive(), "hung up {} s in", burst * 10);
    }
    call.runtime.link.token = None;
    call.runtime.check_silence_at(started + 50_000).await?;
    assert!(call.alive());
    assert!(call.runtime.state.views.silence_questions.call.pending());
    call.frames_at(started + 65_000, 1, false).await?;
    call.runtime.check_silence_at(started + 65_000).await?;
    assert!(!call.alive(), "noise without words never answers");
    Ok(())
}

/// Words heard in a turn are what keeps a call going: the user's silence
/// counts again from the frame the call had when they were known.
#[tokio::test]
async fn words_heard_in_a_call_turn_start_the_silence_again() -> Result<()> {
    let mut call = Call::open().await?;
    let _hub_commands = call.runtime.link.outbound.open(16);
    let started = call.started_at_ms()?;
    call.frames_at(started + 50_000, 4, true).await?;
    call.frames_at(started + 50_400, 8, false).await?;
    // Quiet while the turn is transcribed, past the minute.
    call.frames_at(started + 51_200, 100, false).await?;
    assert!(call.alive(), "hung up while a turn was being heard");
    call.transcribed("¿Cómo va el informe trimestral?").await?;
    let detector = &call
        .runtime
        .audio
        .call
        .as_ref()
        .context("call absent")?
        .detector;
    assert_eq!(detector.last_activity_ms(), started + 61_100);
    Ok(())
}

#[tokio::test]
async fn repeated_joins_keep_the_first_voiced_frame_and_preroll() -> Result<()> {
    let mut call = Call::open().await?;
    call.frames(3, false).await?;
    call.turn().await?;
    let first = call.operation().context("first turn")?;
    let first_pcm = call.voice_pcm(&first)?;
    call.transcribed("").await?;
    for _ in 0..3 {
        call.turn().await?;
        let operation = call.operation().context("joined turn")?;
        let pcm = call.voice_pcm(&operation)?;
        assert!(
            pcm.starts_with(&first_pcm),
            "the earliest carried voice was lost"
        );
    }
    Ok(())
}

#[tokio::test]
async fn voiced_onset_pauses_and_noise_resumes_without_cancelling() -> Result<()> {
    let mut call = Call::open().await?;
    call.helper_orders();
    call.frames_over_answer(1, true).await?;
    let mut frame = call_frame(&call.stream, call.sequence, true);
    frame["playback_active"] = json!(true);
    call.sequence += 1;
    let onset = std::time::Instant::now();
    let (handled, paused) = tokio::join!(call.runtime.native_event(frame), async {
        let order = tokio::time::timeout(Duration::from_secs(1), call.helper.recv())
            .await?
            .context("pause command")?;
        Ok::<_, anyhow::Error>((order, onset.elapsed()))
    });
    handled?;
    let (order, elapsed) = paused?;
    assert_eq!(order["command"], "playback_pause");
    assert!(
        elapsed < Duration::from_millis(100),
        "pause dispatch: {elapsed:?}"
    );
    eprintln!("detected voiced onset to helper pause dispatch: {elapsed:?}");
    call.frames(2, true).await?;
    call.frames(8, false).await?;
    call.transcribed("").await?;
    let orders = call.helper_orders();
    assert!(orders.iter().any(|order| order == "playback_resume"));
    assert!(!orders.iter().any(|order| order == "playback_stop"));
    assert!(call.runtime.state.durable.assistant.turn.is_none());
    Ok(())
}

#[tokio::test]
async fn accepted_barge_in_keeps_the_next_answer_audible() -> Result<()> {
    let mut call = Call::open().await?;
    let _hub = call.runtime.link.outbound.open(16);
    call.runtime.state.durable.assistant.active_request = Some("synthetic-answer".into());
    call.frames_over_answer(4, true).await?;
    call.frames(8, false).await?;
    call.transcribed("Espera, revisa el siguiente punto")
        .await?;
    assert!(
        call.runtime.assistant.speak_next,
        "interrupt consumed the next answer's voice"
    );
    Ok(())
}

fn mark_frame(frame: &mut Value, sequence: u64, voiced: bool) {
    let sample = if voiced { 1_000 } else { 0 } + sequence as i16;
    frame["pcm_s16le_base64"] = json!(STANDARD.encode(sample.to_le_bytes().repeat(800)));
}

#[tokio::test]
async fn a_short_candidate_between_joined_turns_keeps_the_carried_audio() -> Result<()> {
    let mut call = Call::open().await?;
    call.turn().await?;
    let first = call.operation().context("first turn")?;
    let first_pcm = call.voice_pcm(&first)?;
    call.transcribed("").await?;
    // 120 ms opens a candidate but does not meet the 160 ms submission floor.
    for _ in 0..6 {
        let mut frame = call_frame(&call.stream, call.sequence, true);
        frame["frames"] = json!(160);
        frame["pcm_s16le_base64"] = json!(STANDARD.encode(1_234_i16.to_le_bytes().repeat(160)));
        call.sequence += 1;
        call.runtime.native_event(frame).await?;
    }
    call.frames(8, false).await?;
    assert!(call.operation().is_none());
    call.turn().await?;
    let final_turn = call.operation().context("final turn")?;
    assert!(
        call.voice_pcm(&final_turn)?.starts_with(&first_pcm),
        "short candidate lost the carried prefix"
    );
    Ok(())
}

#[path = "call_reading_tests.rs"]
mod reading;

/// Public helper frames and transcription results, no real audio or provider.
#[tokio::test]
async fn presence_words_stay_local_and_other_words_are_a_normal_turn() -> Result<()> {
    for reply in [
        "sí",
        "sigo",
        "aquí",
        "espera",
        "mantén",
        "no cuelgues",
        "¿Qué hora es?",
    ] {
        let mut call = Call::open().await?;
        let mut hub = call.runtime.link.outbound.open(16);
        call.runtime.link.token = None; // Notice-only fallback, no speech request.
        let started = call.started_at_ms()?;
        call.frames_at(started + 30_000, 1, false).await?;
        call.runtime.check_silence_at(started + 30_000).await?;
        assert!(call.runtime.state.views.silence_questions.call.pending());
        assert!(call.runtime.assistant.read_aloud.is_none());
        call.runtime.link.token = Some("synthetic-token".into());
        call.frames_at(started + 31_000, 4, true).await?;
        call.frames_at(started + 31_400, 8, false).await?;
        call.transcribed(reply).await?;
        assert!(call.alive(), "{reply}");
        assert!(!call.runtime.state.views.silence_questions.call.pending());
        if reply == "¿Qué hora es?" {
            assert_eq!(hub.try_recv()?["params"]["text"], reply);
        } else {
            assert!(
                hub.try_recv().is_err(),
                "presence sent to assistant: {reply}"
            );
            assert!(
                call.runtime
                    .state
                    .durable
                    .assistant
                    .active_request
                    .is_none()
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn presence_noise_never_answers_and_cuelga_uses_normal_hangup() -> Result<()> {
    for reply in ["", ".", "Cuelga"] {
        let mut call = Call::open().await?;
        let _hub = call.runtime.link.outbound.open(16);
        call.runtime.link.token = None;
        let started = call.started_at_ms()?;
        call.frames_at(started + 30_000, 1, false).await?;
        call.runtime.check_silence_at(started + 30_000).await?;
        call.runtime.link.token = Some("synthetic-token".into());
        call.frames_at(started + 31_000, 4, true).await?;
        call.frames_at(started + 31_400, 8, false).await?;
        call.transcribed(reply).await?;
        if reply != "Cuelga" {
            assert!(call.alive());
            assert!(call.runtime.state.views.silence_questions.call.pending());
            call.frames_at(started + 45_000, 1, false).await?;
            call.runtime.check_silence_at(started + 45_000).await?;
        }
        assert!(!call.alive(), "{reply}");
        assert!(call.helper_orders().contains(&"audio_stop".into()));
    }
    Ok(())
}

#[tokio::test]
async fn silence_question_waits_for_speech_thought_and_mac_action() -> Result<()> {
    for work in ["speech", "thought", "action"] {
        let mut call = Call::open().await?;
        let _hub = call.runtime.link.outbound.open(16);
        call.runtime.link.token = None;
        let started = call.started_at_ms()?;
        call.frames_at(started + 30_000, 1, false).await?;
        call.runtime.link.token = Some("synthetic-token".into());
        match work {
            "speech" => call.frames_over_answer(1, false).await?,
            "thought" => {
                call.runtime
                    .apply(Event::Submit {
                        id: "thinking-turn".into(),
                        text: "Resume el plan".into(),
                        attachments: vec![],
                    })
                    .await?
            }
            _ => {
                pending_work(
                    &mut call.runtime.computer.work,
                    call.runtime.state.generation(),
                    (),
                )?;
            }
        }
        call.runtime.check_silence_at(started + 60_000).await?;
        assert!(call.alive());
        assert!(
            !call.runtime.state.views.silence_questions.call.pending(),
            "{work}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn the_questions_own_playback_never_restarts_its_answer_clock() -> Result<()> {
    let mut call = Call::open().await?;
    call.runtime.link.token = None;
    let started = call.started_at_ms()?;
    call.frames_at(started + 30_000, 1, false).await?;
    call.runtime.check_silence_at(started + 30_000).await?;
    // A frame queued before the question finished still says playback_active.
    // Its later delivery must not turn the question itself into assistant work.
    let mut queued = call_frame(&call.stream, call.sequence, false);
    call.sequence += 1;
    queued["playback_active"] = json!(true);
    call.runtime
        .on_native_audio_at(&mut queued, started + 33_000)
        .await?;
    call.runtime
        .native_event(
            json!({"v":1, "event":"playback_progress", "playback_id":"presence-clip",
        "state":"played", "playback_active":false}),
        )
        .await?;
    call.runtime.check_silence_at(started + 33_000).await?;
    assert!(call.runtime.state.views.silence_questions.call.pending());
    call.frames_at(started + 45_000, 1, false).await?;
    call.runtime.check_silence_at(started + 45_000).await?;
    assert!(!call.alive());
    Ok(())
}
