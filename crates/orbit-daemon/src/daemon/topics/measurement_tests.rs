//! Repeatable release measurements; all data and files are synthetic.
use super::*;
use crate::daemon::calendar::tests::WRITE_RECORDS;
use orbit_core::{
    Approval, AudioSource, CapturePhase, Message, Recording, Turn,
    calendar::CalendarAuthorization,
    calendar_control::{CalendarWriteOperation, CalendarWriteStage, CalendarWriteView},
    computer_control::{ControlHistoryEntry, ControlOutcome, MAX_CONTROL_HISTORY},
    notice::{Notice, NoticeCode, NoticeKey, Op},
    topics::{Knowledge, notes::MAX_NAMED_RECORDINGS},
};
use std::{hint::black_box, time::Instant};

const SAMPLES: usize = 2_048;

fn worst_case(runtime: &mut Runtime) -> anyhow::Result<()> {
    let message = |role: &str, text: String| Message {
        attachments: Vec::new(),
        id: "request-measured".into(),
        role: role.into(),
        text,
        complete: false,
        at_ms: u64::MAX,
        voice: None,
    };
    runtime.state.durable.turn = Some(Turn {
        user: message("user", "\"".repeat(orbit_core::MAX_TEXT_BYTES)),
        answer: Some(message(
            "assistant",
            "\n".repeat(3 * orbit_core::MAX_TEXT_BYTES),
        )),
    });
    runtime.state.durable.pending_approval = Some(Approval {
        request_id: "request-measured".into(),
        description: "\"".repeat(16_000),
        clarification: false,
        choices: Vec::new(),
        questions: Vec::new(),
        deletes: None,
    });
    runtime.state.durable.recording = Some(Recording {
        id: "00000000-0000-4000-8000-000000000000".into(),
        title: "😀".repeat(orbit_core::MAX_MANIFEST_TITLE_CHARACTERS),
        started_at_ms: 0,
        ended_at_ms: None,
        expires_at_ms: i64::MAX,
        application_pid: Some(i32::MAX),
        audio: orbit_core::protocol::MeetingAudio::System,
        phase: CapturePhase::Starting,
        sealed_chunks: u64::MAX,
        gaps: u64::MAX,
        tracks: vec![
            orbit_core::TrackProgress {
                name: "mic_local".into(),
                sealed_chunks: u64::MAX,
                gaps: u64::MAX,
                last_chunk_at_ms: i64::MAX
            };
            2
        ],
        finalizing_since_ms: None,
        ceiling_notice_given: false,
    });
    runtime.features.meeting.audio_source_observed = true;
    runtime.state.views.audio_sources = (0..200)
        .map(|i| AudioSource {
            application_pid: i + 1,
            application_name: format!("Aplicación de prueba {i:03}"),
            bundle_id: format!("test.app.{i}"),
        })
        .collect();
    runtime.state.views.account = Some(orbit_core::AccountSummary {
        hub_url: Some("https://hub.example.invalid".into()),
        device_name: Some("m".repeat(256).into()),
        device_id_short: Some("d".repeat(128).into()),
    });
    runtime.features.calendar.view.authorization = CalendarAuthorization::FullAccess;
    runtime.features.calendar.inventory_valid = true;
    // calendar::writes::WRITE_RECORDS: the existing queue holds sixteen writes.
    runtime.features.calendar.view.writes = (0..WRITE_RECORDS)
        .map(|i| CalendarWriteView {
            request_id: format!("{i:03}{}", "x".repeat(61)),
            operation: CalendarWriteOperation::Create,
            stage: CalendarWriteStage::Saved,
            occurrence: None,
            refusal: None,
            detail: String::new(),
        })
        .collect();
    for i in 0..MAX_NAMED_RECORDINGS {
        runtime
            .features
            .notes
            .head
            .touch_recording(&format!("00000000-0000-4000-8000-{i:012x}"));
    }
    runtime.features.notes.head.revision = u64::MAX;
    runtime.features.notes.head.library_revision = u64::MAX;
    runtime.features.notes.head.floor = u64::MAX;
    for at in runtime.features.notes.head.recordings.values_mut() {
        *at = u64::MAX;
    }
    for _ in 0..orbit_core::notice::CAPACITY {
        let mut notice = Notice::new(NoticeCode::NoteChangeUnsent);
        for key in NoticeKey::ALL {
            notice = notice.with_token(*key, &"x".repeat(orbit_core::notice::Token::MAX_LEN));
        }
        notice.op = Op::parse(&"a".repeat(Op::MAX_LEN));
        runtime.state.set_notice(notice);
    }
    let history: Vec<_> = (0..MAX_CONTROL_HISTORY)
        .map(|i| ControlHistoryEntry {
            operation_id: format!("{i:03}{}", "o".repeat(125)),
            requested: "r".repeat(128),
            application: "\"".repeat(256),
            channel: "c".repeat(64),
            at_ms: i64::MAX,
            outcome: ControlOutcome::Performed,
        })
        .collect();
    assert!(history.iter().all(ControlHistoryEntry::is_valid));
    let mut bytes = serde_json::to_vec(&serde_json::json!({"schema":1,"history":history}))?;
    assert!(bytes.len() <= 256 * 1024, "{}", bytes.len());
    bytes.resize(256 * 1024, b' ');
    std::fs::write(runtime.paths.root.join("computer-control.json"), bytes)?;
    assert_eq!(runtime.control_history()?.len(), MAX_CONTROL_HISTORY);
    Ok(())
}

// Repeatable measurement on the Mac: cargo test -p orbit-daemon --lib
// measure_worst_case_projections -- --ignored --nocapture
#[tokio::test]
#[ignore = "manual measurement on the Mac; not a gate criterion"]
async fn measure_worst_case_projections() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = crate::daemon::audio::tests::fixture(directory.path())?;
    crate::daemon::calendar::tests::seed(&mut runtime, &mut helper).await?;
    worst_case(&mut runtime)?;
    let source = TopicSource {
        state: &runtime.state,
        runtime: Some(&runtime),
        now_ms: 0,
    };
    for id in TopicId::ALL {
        // A missing runtime would measure a cheap unknown instead of the real projection.
        let data = project(*id, &source);
        match &data {
            TopicData::Agenda(Knowledge::Present { value, .. }) => {
                assert_eq!(runtime.features.calendar.view.writes.len(), WRITE_RECORDS);
                assert!(!value.writes.is_empty());
            }
            TopicData::Control(Knowledge::Present { value, .. }) => {
                assert!(
                    matches!(&value.history, Knowledge::Present { value, .. } if value.total == 256 && value.recent.len() == 20)
                );
            }
            TopicData::Assistant(Knowledge::Present { .. }) => {}
            TopicData::Assistant(other) => panic!("worst valid assistant must fit: {other:?}"),
            _ => {}
        }
        measure_projection(*id, &source, &format!("{id:?}"))?;
    }
    for round in 1..=3 {
        let mut ns = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let mut board = TopicBoard::new("measurement".into());
            let _receivers: Vec<_> = board
                .channels()
                .values()
                .map(watch::Sender::subscribe)
                .collect();
            let start = Instant::now();
            black_box(board.publish_command(black_box(&source)));
            ns.push(start.elapsed().as_nanos());
        }
        ns.sort_unstable();
        eprintln!(
            "publication round={round} n={SAMPLES} p50_ns={} p99_ns={} max_ns={}",
            ns[SAMPLES / 2],
            ns[(SAMPLES * 99).div_ceil(100) - 1],
            ns[SAMPLES - 1]
        );
    }
    // Six-byte escaping is the expensive, valid-input refusal path of a turn.
    let turn = runtime
        .state
        .durable
        .turn
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("missing turn"))?;
    turn.user.text = "\u{0001}".repeat(orbit_core::MAX_TEXT_BYTES);
    turn.answer
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("missing answer"))?
        .text = "\u{0001}".repeat(3 * orbit_core::MAX_TEXT_BYTES);
    let source = TopicSource {
        state: &runtime.state,
        runtime: Some(&runtime),
        now_ms: 0,
    };
    assert!(matches!(
        project(TopicId::Assistant, &source),
        TopicData::Assistant(Knowledge::Absent { .. })
    ));
    measure_projection(TopicId::Assistant, &source, "AssistantEscaped")?;

    let saved: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../contracts/fixtures/native/results/calendar_save.json"
    ))?;
    let mut occurrence: orbit_core::calendar::CalendarOccurrence =
        serde_json::from_value(saved["occurrence"].clone())?;
    occurrence.calendar_item_id = "x".repeat(orbit_core::calendar::MAX_LABEL_BYTES);
    for write in &mut runtime.features.calendar.view.writes {
        write.occurrence = Some(Box::new(occurrence.clone()));
    }
    let source = TopicSource {
        state: &runtime.state,
        runtime: Some(&runtime),
        now_ms: 0,
    };
    measure_projection(TopicId::Agenda, &source, "AgendaSaved")?;

    // Model the proposed invalidation at 94 state inputs/s, preserving the
    // existing per-topic cadence. No sleep, host input or audio effects.
    for round in 1..=3 {
        let mut board = TopicBoard::new("cadence".into());
        let _receivers: Vec<_> = board
            .channels()
            .values()
            .map(watch::Sender::subscribe)
            .collect();
        let mut ns = Vec::with_capacity(SAMPLES);
        let mut projections = BTreeMap::<TopicId, usize>::new();
        for frame in 0..SAMPLES {
            let source = TopicSource {
                state: &runtime.state,
                runtime: Some(&runtime),
                now_ms: frame as u64 * 1_000 / 94,
            };
            let before: BTreeMap<_, _> = board
                .cells
                .iter()
                .map(|(id, cell)| (*id, cell.last_projected_ms))
                .collect();
            let start = Instant::now();
            for cell in board.cells.values_mut() {
                cell.dirty = true;
            }
            black_box(board.publish(black_box(&source)));
            ns.push(start.elapsed().as_nanos());
            for (id, cell) in &board.cells {
                if cell.last_projected_ms != before[id] {
                    *projections.entry(*id).or_default() += 1;
                }
            }
        }
        let total: u128 = ns.iter().sum();
        ns.sort_unstable();
        eprintln!(
            "cadence round={round} n={SAMPLES} input_hz=94 total_ns={total} p50_ns={} p99_ns={} max_ns={} projections={projections:?}",
            ns[SAMPLES / 2],
            ns[(SAMPLES * 99).div_ceil(100) - 1],
            ns[SAMPLES - 1]
        );
    }
    Ok(())
}

fn measure_projection(id: TopicId, source: &TopicSource<'_>, label: &str) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec(&project(id, source))?.len();
    for round in 1..=3 {
        for _ in 0..64 {
            black_box(project(id, black_box(source)));
        }
        let mut ns = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let start = Instant::now();
            let data = black_box(project(id, black_box(source)));
            ns.push(start.elapsed().as_nanos());
            drop(data);
        }
        ns.sort_unstable();
        eprintln!(
            "projection topic={label} round={round} n={SAMPLES} bytes={bytes} p50_ns={} p99_ns={} max_ns={}",
            ns[SAMPLES / 2],
            ns[(SAMPLES * 99).div_ceil(100) - 1],
            ns[SAMPLES - 1]
        );
    }
    Ok(())
}
