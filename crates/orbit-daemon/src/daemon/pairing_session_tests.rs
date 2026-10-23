//! The daemon actor's pairing, Deimos sessions and transcript retry. (The
//! remote switches that used to move through the daemon are gone; what a
//! remote request is decided on now is the macOS permission, covered in
//! `actor_tests.rs` and `computer_control/engine_permission_tests.rs`.) Every
//! hub peer is a one-shot loopback HTTP fixture; no production hub, provider,
//! credential or helper process is contacted, and every value on the wire is
//! synthetic.
use super::*;
use crate::daemon::audio::tests::fixture;
use orbit_core::protocol::Command;
use orbit_core::topics::{Cursor, Knowledge, Page};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const DEVICE_ID: &str = "20000000-0000-4000-8000-000000000002";
const RECORDING_ID: &str = "10000000-0000-4000-8000-000000000001";
const SESSION_ID: &str = "30000000-0000-4000-8000-000000000003";
const DEVICE_TOKEN: &str = "synthetic-not-a-credential";
const DEVICE_NAME: &str = "Fixture Mac";

/// A paired runtime online with the one-shot hub at `base`.
pub(super) fn online_with_hub(runtime: &mut Runtime, base: &reqwest::Url) {
    runtime.settings.hub_url = Some(base.to_string());
    runtime.link.token = Some(DEVICE_TOKEN.into());
    runtime.settings.device_id = Some(DEVICE_ID.into());
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "connection",
        serde_json::json!(Connection::Online),
    );
}

/// The conversation page the screen reads after opening a conversation.
async fn read_messages(
    runtime: &Runtime,
    cursor: Option<Cursor>,
) -> Knowledge<Page<serde_json::Value>> {
    super::reads::HubReadContext::capture(runtime, 0)
        .messages(
            SESSION_ID.into(),
            cursor,
            runtime.voice_audio_reader.clone(),
        )
        .await
}

#[derive(Debug, Clone)]
pub(super) struct HttpRequest {
    pub(super) start_line: String,
    pub(super) authorization: Option<String>,
    pub(super) body: String,
}

pub(super) fn recorded(seen: &Arc<StdMutex<Option<HttpRequest>>>) -> Result<HttpRequest> {
    seen.lock()
        .ok()
        .and_then(|mut slot| slot.take())
        .context("the fixture received no request")
}

/// One loopback HTTP conversation: a single request is read in full, recorded,
/// and answered with the prepared raw response. Modeled on the transcript
/// fixtures: hand-written HTTP, closed as soon as the answer is written.
pub(super) async fn one_shot_http(
    status_and_body: Vec<u8>,
) -> Result<(reqwest::Url, Arc<StdMutex<Option<HttpRequest>>>)> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = reqwest::Url::parse(&format!("http://{}", listener.local_addr()?))?;
    let seen = Arc::new(StdMutex::new(None));
    let observed = seen.clone();
    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 1024];
        let head_end = loop {
            if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break position + 4;
            }
            let count = match socket.read(&mut chunk).await {
                Ok(count) => count,
                Err(_) => return,
            };
            if count == 0 || bytes.len() + count > 16_384 {
                return;
            }
            bytes.extend_from_slice(&chunk[..count]);
        };
        let head = String::from_utf8_lossy(&bytes[..head_end]).to_string();
        let content_length = head
            .lines()
            .filter_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .next()
            .unwrap_or(0);
        while bytes.len() < head_end + content_length {
            let count = match socket.read(&mut chunk).await {
                Ok(count) => count,
                Err(_) => return,
            };
            if count == 0 {
                return;
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        let authorization = head.lines().find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("authorization:")
                .map(|value| value.trim().to_owned())
        });
        let start_line = head.lines().next().unwrap_or_default().to_owned();
        let body = String::from_utf8_lossy(&bytes[head_end..]).to_string();
        if let Ok(mut slot) = observed.lock() {
            *slot = Some(HttpRequest {
                start_line,
                authorization,
                body,
            });
        }
        let _ = socket.write_all(&status_and_body).await;
    });
    Ok((base, seen))
}

pub(super) fn json_response(status: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// Pairing belongs to the daemon: the app never touches credentials. A
/// rejected code explains itself in the notice and saves nothing; a valid code
/// stores the credential where the CLI stored it today and never shows it.
#[tokio::test]
async fn pair_through_the_daemon_saves_the_credential_and_never_shows_it() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;

    // The hub refuses this code; nothing may be kept from the attempt.
    let (rejected, rejected_seen) = one_shot_http(json_response(
        "403 Forbidden",
        "{\"code\":\"invalid_code\",\"message\":\"Código no válido\"}",
    ))
    .await?;
    let outcome = runtime
        .command(Command::Pair {
            hub_url: rejected.to_string(),
            code: "codigo-rechazado".into(),
            device_name: DEVICE_NAME.into(),
        })
        .await;
    drop(outcome);
    let request = recorded(&rejected_seen)?;
    assert_eq!(request.start_line, "POST /v1/pair HTTP/1.1");
    assert!(
        request.body.contains("codigo-rechazado"),
        "the code did not travel to the hub: {request:?}"
    );
    let settings = runtime.paths.load_settings()?;
    assert!(settings.device_id.is_none(), "a rejected code paired");
    assert!(settings.hub_url.is_none(), "a rejected code saved the hub");
    assert!(
        crate::config::get_secret(&runtime.paths, "device", true)?.is_none(),
        "a rejected code saved a credential"
    );
    assert!(
        runtime.state.notice_code() == Some(orbit_core::notice::NoticeCode::PairingFailed),
        "the notice did not explain the rejection: {:?}",
        runtime.state.durable.notice
    );

    // A valid code pairs durably; the credential is stored, never published.
    let (accepted, accepted_seen) = one_shot_http(json_response(
        "200 OK",
        &format!("{{\"device_id\":\"{DEVICE_ID}\",\"token\":\"{DEVICE_TOKEN}\"}}"),
    ))
    .await?;
    runtime
        .command(Command::Pair {
            hub_url: accepted.to_string(),
            code: "codigo-valido".into(),
            device_name: DEVICE_NAME.into(),
        })
        .await?;
    let request = recorded(&accepted_seen)?;
    assert!(
        request.body.contains(DEVICE_NAME),
        "the device name did not travel to the hub: {request:?}"
    );
    let settings = runtime.paths.load_settings()?;
    assert_eq!(settings.device_id.as_deref(), Some(DEVICE_ID));
    let stored_hub = settings
        .hub_url
        .as_deref()
        .context("pairing did not save the hub")?;
    assert_eq!(reqwest::Url::parse(stored_hub)?, accepted);
    assert_eq!(
        crate::config::get_secret(&runtime.paths, "device", true)?.as_deref(),
        Some(DEVICE_TOKEN)
    );
    assert!(
        !serde_json::to_string(&runtime.state.durable.notice_log)?.contains(DEVICE_TOKEN),
        "the notice published the credential: {:?}",
        runtime.state.durable.notice
    );
    Ok(())
}

/// The daemon reads the hub's session list as the hub gives it, and can
/// open one of them.
#[tokio::test]
async fn sessions_are_the_hubs_rows_and_session_open_makes_one_active() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let (base, seen) = one_shot_http(json_response(
        "200 OK",
        "{\"sessions\":[\
          {\"id\":\"session-one\",\"created_at_ms\":1700000000125,\"last_at_ms\":1700000090000,\"message_count\":4},\
          {\"id\":\"session-two\",\"created_at_ms\":1700000001750,\"last_at_ms\":1700000001750,\"message_count\":0}],\
          \"total\":2,\"next\":null}",
    ))
    .await?;
    online_with_hub(&mut runtime, &base);
    let Knowledge::Present { value, .. } = super::reads::HubReadContext::capture(&runtime, 0)
        .sessions(None)
        .await
    else {
        bail!("the sessions read did not arrive");
    };
    let request = recorded(&seen)?;
    assert_eq!(request.start_line, "GET /v1/sessions?before= HTTP/1.1");
    assert!(
        request
            .authorization
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case(&format!("Bearer {DEVICE_TOKEN}"))),
        "the session list did not travel with the device credential"
    );
    let sessions = value
        .items
        .into_iter()
        .map(serde_json::from_value::<orbit_core::SessionSummary>)
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(sessions.len(), 2, "the hub's list was not projected whole");
    assert_eq!(sessions[0].id, "session-one");
    assert_eq!(sessions[0].created_at_ms, 1_700_000_000_125);
    assert_eq!(sessions[0].last_at_ms, 1_700_000_090_000);
    assert_eq!(sessions[0].message_count, 4);
    assert_eq!(value.total, 2);
    assert!(value.next.is_none());

    runtime
        .command(Command::SessionOpen {
            id: "session-two".into(),
            agent_id: None,
        })
        .await?;
    assert_eq!(
        runtime.state.session_id().as_deref(),
        Some("session-two"),
        "opening the session did not make it active"
    );
    Ok(())
}

/// Each conversation says the agent it is with, as the hub lists it; a row
/// without one (a hub without agents) is the default agent's.
#[tokio::test]
async fn sessions_say_the_agent_each_one_is_with() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let (base, _seen) = one_shot_http(json_response(
        "200 OK",
        "{\"sessions\":[\
          {\"id\":\"session-atlas\",\"agent_id\":\"atlas\",\"created_at_ms\":1700000000125,\"last_at_ms\":1700000090000,\"message_count\":4},\
          {\"id\":\"session-old\",\"created_at_ms\":1700000001750,\"last_at_ms\":1700000001750,\"message_count\":0}],\
          \"total\":2,\"next\":null}",
    ))
    .await?;
    online_with_hub(&mut runtime, &base);
    let Knowledge::Present { value, .. } = super::reads::HubReadContext::capture(&runtime, 0)
        .sessions(None)
        .await
    else {
        bail!("the sessions read did not arrive");
    };
    let sessions = value
        .items
        .into_iter()
        .map(serde_json::from_value::<orbit_core::SessionSummary>)
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(sessions[0].agent_id.as_deref(), Some("atlas"));
    assert_eq!(sessions[1].agent_id, None);
    Ok(())
}

/// Opening a previous conversation and reading it as the screen does gives
/// the messages the hub keeps for it, in order, with the user's own by the id
/// the Mac gave them.
#[tokio::test]
async fn opening_a_previous_conversation_shows_its_messages() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let (base, seen) = one_shot_http(json_response(
        "200 OK",
        "{\"messages\":[\
          {\"id\":1,\"role\":\"user\",\"text\":\"Hola\",\"at_ms\":1700000000000,\"kind\":\"text\",\"client_id\":\"40000000-0000-4000-8000-000000000004\"},\
          {\"id\":2,\"role\":\"assistant\",\"text\":\"Respuesta\",\"at_ms\":1700000001000,\"kind\":\"text\",\
           \"work\":{\"started_at_ms\":1700000000000,\"ended_at_ms\":1700000001000,\"tools\":2,\"subjects\":[\"el calendario\"]}}],\
          \"next\":null,\"total\":2}",
    ))
    .await?;
    online_with_hub(&mut runtime, &base);
    runtime
        .command(Command::SessionOpen {
            id: SESSION_ID.into(),
            agent_id: None,
        })
        .await?;
    let Knowledge::Present { value, .. } = read_messages(&runtime, None).await else {
        bail!("the open conversation was not read");
    };
    let request = recorded(&seen)?;
    assert_eq!(
        request.start_line,
        format!("GET /v1/sessions/{SESSION_ID}/messages HTTP/1.1")
    );
    let messages = value
        .items
        .into_iter()
        .map(serde_json::from_value::<orbit_core::SessionMessage>)
        .collect::<Result<Vec<_>, _>>()?;
    let texts = messages
        .iter()
        .map(|message| message.text.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        texts,
        ["Hola", "Respuesta"],
        "opening a previous conversation does not show its messages"
    );
    assert_eq!(messages[0].id, "40000000-0000-4000-8000-000000000004");
    assert_eq!(messages[1].id, "2");
    assert_eq!(messages[0].at_ms, Some(1_700_000_000_000));
    // Each answer keeps what its turn did; the user's message has none.
    assert_eq!(messages[0].work, None);
    assert_eq!(
        messages[1].work,
        Some(orbit_core::agent_work::WorkSummary {
            started_at_ms: 1_700_000_000_000,
            ended_at_ms: 1_700_000_001_000,
            tools: 2,
            subjects: vec!["el calendario".into()],
        })
    );
    assert_eq!(value.total, 2);
    Ok(())
}

/// Older pages are asked for with the hub's cursor, and a voice note
/// says whether this Mac still holds its audio, found by the Mac's id.
#[tokio::test]
async fn an_older_page_and_the_voice_notes_audio() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let held = "50000000-0000-4000-8000-000000000005";
    let gone = "60000000-0000-4000-8000-000000000006";
    let metadata = orbit_store::ChunkMetadata {
        id: format!("{held}-mic_local-0"),
        recording_id: held.into(),
        track: "mic_local".into(),
        capture_epoch_id: None,
        sequence: 0,
        sample_rate: 16_000,
        channels: 1,
        frames: 160,
        monotonic_ns: 1,
        first_sample: 0,
        started_at_ms: 1,
    };
    let wav = crate::native::wav(&[0_u8; 320], 16_000, 1)?;
    runtime
        .store_actor
        .run(move |store| Ok(store.seal_chunk(&metadata, &wav)?))
        .await?;
    let (base, seen) = one_shot_http(json_response(
        "200 OK",
        &format!(
            "{{\"messages\":[\
              {{\"id\":7,\"role\":\"user\",\"text\":\"Dicho\",\"at_ms\":5,\"kind\":\"voice\",\"duration_ms\":4200,\"client_id\":\"{held}\"}},\
              {{\"id\":8,\"role\":\"user\",\"text\":\"Antes\",\"at_ms\":null,\"kind\":\"voice\",\"duration_ms\":900,\"client_id\":\"{gone}\"}}],\
              \"next\":6,\"total\":30}}"
        ),
    ))
    .await?;
    online_with_hub(&mut runtime, &base);
    let Knowledge::Present { value, .. } =
        read_messages(&runtime, Some(Cursor::Id { id: "9".into() })).await
    else {
        bail!("the older page was not read");
    };
    assert_eq!(
        recorded(&seen)?.start_line,
        format!("GET /v1/sessions/{SESSION_ID}/messages?before=9 HTTP/1.1")
    );
    assert_eq!(value.next, Some(Cursor::Id { id: "6".into() }));
    let messages = value
        .items
        .into_iter()
        .map(serde_json::from_value::<orbit_core::SessionMessage>)
        .collect::<Result<Vec<_>, _>>()?;
    let voices = messages
        .iter()
        .map(|message| message.voice.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        voices,
        [
            Some(orbit_core::SessionVoice {
                duration_ms: 4_200,
                audio_available: true
            }),
            Some(orbit_core::SessionVoice {
                duration_ms: 900,
                audio_available: false
            }),
        ]
    );
    assert_eq!(
        messages[1].at_ms, None,
        "a message stored before messages carried their time has none"
    );
    Ok(())
}

/// Without the hub there is no chat. Reading a conversation offline asks
/// nothing, refuses with the missing link and tells the user what is missing
/// (a notice they can fix).
#[tokio::test]
async fn reading_a_conversation_offline_says_the_hub_is_missing() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    runtime.settings.hub_url = Some("http://127.0.0.1:9".into());
    runtime.link.token = Some(DEVICE_TOKEN.into());
    runtime.settings.device_id = Some(DEVICE_ID.into());
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "connection",
        serde_json::json!(Connection::Offline),
    );
    let answer = read_messages(&runtime, None).await;
    assert!(
        matches!(
            answer,
            Knowledge::Absent {
                refusal: Refusal::HubUnreachable { .. }
            }
        ),
        "{answer:?}"
    );
    runtime.say_when_conversations_need_the_hub();
    assert_eq!(
        runtime.state.notice_code(),
        Some(orbit_core::notice::NoticeCode::ConversationsNeedHub)
    );
    assert_eq!(
        orbit_core::notice::NoticeCode::ConversationsNeedHub.class(),
        orbit_core::notice::NoticeClass::Fixable
    );
    Ok(())
}

#[tokio::test]
async fn transcript_retry_calls_the_hub_route_with_the_device_bearer_and_publishes_requeued()
-> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let (base, seen) = one_shot_http(json_response("200 OK", "{\"requeued\":2}")).await?;
    runtime.settings.hub_url = Some(base.to_string());
    runtime.link.token = Some(DEVICE_TOKEN.into());
    runtime.settings.device_id = Some(DEVICE_ID.into());
    let mut events = runtime.notes.test_events();

    runtime
        .command(Command::TranscriptRetry {
            recording_id: RECORDING_ID.into(),
        })
        .await?;
    // The order goes off the loop; its answer comes back as a note
    // event, the path production takes.
    let answer = tokio::time::timeout(std::time::Duration::from_secs(10), events.recv())
        .await
        .context("the fixture did not answer the retry")?
        .context("the notes channel closed")?;
    runtime.note_answer(answer).await?;
    let request = recorded(&seen)?;
    assert_eq!(
        request.start_line,
        format!("POST /v1/recordings/{RECORDING_ID}/transcript/retry HTTP/1.1")
    );
    assert!(
        request
            .authorization
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case(&format!("Bearer {DEVICE_TOKEN}"))),
        "the retry did not travel with the device credential"
    );
    assert!(
        runtime.state.durable.notice
            == Some(
                orbit_core::notice::Notice {
                    sequence: runtime
                        .state
                        .durable
                        .notice
                        .as_ref()
                        .and_then(|notice| notice.sequence),
                    op: runtime
                        .state
                        .durable
                        .notice
                        .as_ref()
                        .and_then(|notice| notice.op.clone()),
                    ..orbit_core::notice::Notice::new(
                        orbit_core::notice::NoticeCode::TranscriptRetryRequested,
                    )
                }
                .with(orbit_core::notice::NoticeKey::Count, 2_u64)
            ),
        "the notice did not say how many groups were queued: {:?}",
        runtime.state.durable.notice
    );
    Ok(())
}

#[tokio::test]
async fn pairing_another_device_forgets_the_previous_conversation_durably() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    runtime
        .apply(Event::Session {
            id: "other-macs-session".into(),
            agent_id: None,
        })
        .await?;
    let (accepted, seen) = one_shot_http(json_response(
        "200 OK",
        &format!("{{\"device_id\":\"{DEVICE_ID}\",\"token\":\"{DEVICE_TOKEN}\"}}"),
    ))
    .await?;
    runtime
        .command(Command::Pair {
            hub_url: accepted.to_string(),
            code: "synthetic-code".into(),
            device_name: DEVICE_NAME.into(),
        })
        .await?;
    recorded(&seen)?;
    assert!(
        runtime.state.session_id().is_none(),
        "pair kept another device's session"
    );
    let saved: orbit_core::DurableState =
        runtime.fixture_store.load_snapshot()?.context("snapshot")?;
    assert!(
        saved.session_id().is_none(),
        "restart would recover another device's session"
    );
    Ok(())
}

#[tokio::test]
async fn pairing_does_not_install_an_identity_if_forgetting_the_session_cannot_be_saved()
-> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    runtime
        .apply(Event::Session {
            id: "previous-session".into(),
            agent_id: None,
        })
        .await?;
    let database = rusqlite::Connection::open(runtime.paths.store.join("orbit.sqlite3"))?;
    database.execute_batch(
        "CREATE TRIGGER fail_pair_snapshot BEFORE UPDATE ON settings
         WHEN NEW.key='snapshot' BEGIN
           SELECT RAISE(ABORT, 'synthetic snapshot failure');
         END;",
    )?;
    let (accepted, seen) = one_shot_http(json_response(
        "200 OK",
        &format!("{{\"device_id\":\"{DEVICE_ID}\",\"token\":\"{DEVICE_TOKEN}\"}}"),
    ))
    .await?;
    assert!(
        runtime
            .command(Command::Pair {
                hub_url: accepted.to_string(),
                code: "synthetic-code".into(),
                device_name: DEVICE_NAME.into(),
            })
            .await
            .is_err()
    );
    recorded(&seen)?;
    assert!(
        runtime.paths.load_settings()?.device_id.is_none(),
        "new identity was installed before the durable retirement"
    );
    assert!(crate::config::get_secret(&runtime.paths, "device", true)?.is_none());
    let saved: orbit_core::DurableState =
        runtime.fixture_store.load_snapshot()?.context("snapshot")?;
    assert_eq!(saved.session_id().as_deref(), Some("previous-session"));
    Ok(())
}

#[tokio::test]
async fn manual_final_pass_reaches_hub_and_refusals_reach_notice_and_report() -> Result<()> {
    use orbit_core::notice::{NoticeCode, NoticeKey};
    for (response, expected, expected_status, count, total) in [
        (json_response("200 OK", "{}"), "ok", 200, 0, 0),
        (
            json_response("400 Bad Request", r#"{"code":"recording_still_open"}"#),
            "recording_still_open",
            400,
            0,
            0,
        ),
        (
            json_response("409 Conflict", r#"{"code":"diarization_not_configured"}"#),
            "diarization_not_configured",
            409,
            0,
            0,
        ),
        (
            json_response(
                "409 Conflict",
                r#"{"code":"meeting_audio_unreadable","refusal":{"groups":1,"total":4}}"#,
            ),
            "meeting_audio_unreadable",
            409,
            1,
            4,
        ),
    ] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _helper) = fixture(directory.path())?;
        runtime.report_log = Some(orbit_ipc::report::ReportLog::daemon(directory.path()));
        let (base, seen) = one_shot_http(response).await?;
        online_with_hub(&mut runtime, &base);
        let mut events = runtime.notes.test_events();
        let command: Command = serde_json::from_value(serde_json::json!({
            "command":"transcript_retranscribe", "recording_id":RECORDING_ID
        }))?;
        runtime.command(command).await?;
        let answer = events.recv().await.context("the fake hub must answer")?;
        runtime.note_answer(answer).await?;
        let request = recorded(&seen)?;
        assert_eq!(
            request.start_line,
            format!("POST /v1/recordings/{RECORDING_ID}/final-pass HTTP/1.1")
        );
        assert!(
            request
                .authorization
                .as_deref()
                .is_some_and(|v| v.eq_ignore_ascii_case(&format!("Bearer {DEVICE_TOKEN}")))
        );
        if expected != "ok" {
            let notice = runtime
                .state
                .durable
                .notice
                .clone()
                .context("refusal notice")?;
            let data = serde_json::to_value(&notice.data)?;
            assert_eq!(data["status"], expected_status);
            assert!(data.get("closed").is_none());
            assert!(data.get("configured").is_none());
            if expected == "meeting_audio_unreadable" {
                assert_eq!(notice.code, NoticeCode::MeetingAudioUnreadable);
                assert_eq!(data["count"], count);
                assert_eq!(data["total"], total);
            } else {
                assert_eq!(data["code"], expected);
            }
            runtime.report_notices();
            let report = std::fs::read_to_string(directory.path().join("logs/report-daemon.log"))?;
            assert!(report.contains(expected), "{report}");
            let row = report
                .lines()
                .map(serde_json::from_str::<serde_json::Value>)
                .collect::<std::result::Result<Vec<_>, _>>()?
                .into_iter()
                .find(|row| row["data"]["code"] == expected)
                .context("the refusal must reach the report with its code")?;
            assert_eq!(row["data"]["status"], expected_status);
            if count > 0 {
                assert!(
                    report.contains(r#""count":1"#) && report.contains(r#""total":4"#),
                    "{report}"
                );
            }
            assert!(notice.data.contains_key(&NoticeKey::Code));
        }
    }
    Ok(())
}

#[tokio::test]
async fn a_lost_hub_response_is_unconfirmed_with_its_own_notice_and_report() -> Result<()> {
    for (response, cause, status) in [
        (Vec::new(), "hub_unreachable", None),
        (
            json_response(
                "503 Service Unavailable",
                r#"{"code":"worker_unavailable"}"#,
            ),
            "hub_status",
            Some(503),
        ),
    ] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _helper) = fixture(directory.path())?;
        runtime.report_log = Some(orbit_ipc::report::ReportLog::daemon(directory.path()));
        // The peer closes without an HTTP answer; it is a local fixture, never a service.
        let (base, seen) = one_shot_http(response).await?;
        online_with_hub(&mut runtime, &base);
        let mut events = runtime.notes.test_events();
        let command: Command = serde_json::from_value(serde_json::json!({
            "command":"transcript_retranscribe", "recording_id":RECORDING_ID
        }))?;
        runtime.command(command).await?;
        let answer = events
            .recv()
            .await
            .context("answer from the closed fake hub")?;
        runtime.note_answer(answer).await?;
        recorded(&seen)?;
        assert_eq!(
            runtime.state.notice_code(),
            Some(orbit_core::notice::NoticeCode::TranscriptRetranscribeUnconfirmed)
        );
        let report = std::fs::read_to_string(directory.path().join("logs/report-daemon.log"))?;
        assert!(
            report.contains("transcript_retranscribe_unconfirmed"),
            "{report}"
        );
        assert!(report.contains(cause), "{report}");
        if let Some(status) = status {
            let data = serde_json::to_value(
                &runtime
                    .state
                    .durable
                    .notice
                    .as_ref()
                    .context("notice")?
                    .data,
            )?;
            assert_eq!(data["status"], status);
            assert_eq!(data["code"], "worker_unavailable");
        }
    }
    Ok(())
}

#[tokio::test]
async fn dictionary_orders_preserve_numbers_without_logging_terms() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    runtime.report_log = Some(orbit_ipc::report::ReportLog::daemon(directory.path()));
    let (base, seen) = one_shot_http(json_response("409 Conflict", r#"{"code":"dictionary_full","message":"Hay 128 términos; el máximo es 128","refusal":{"terms":128,"max_terms":128}}"#)).await?;
    online_with_hub(&mut runtime, &base);
    let mut events = runtime.notes.test_events();
    let command: Command = serde_json::from_value(serde_json::json!({
        "command": "note_library", "request": {"action": "dictionary", "request": {
            "action": "correct", "recording_id": RECORDING_ID, "dictionary_id": SESSION_ID,
            "wrong": "private-wrong-canary", "correct": "private-right-canary", "name": null
        }}
    }))?;
    runtime.command(command).await?;
    runtime
        .note_answer(events.recv().await.context("dictionary answer")?)
        .await?;
    let wire = recorded(&seen)?;
    assert_eq!(
        wire.start_line,
        "POST /v1/meeting-dictionaries/orders HTTP/1.1"
    );
    assert_eq!(
        wire.authorization.as_deref(),
        Some("bearer synthetic-not-a-credential")
    );
    let payload: serde_json::Value = serde_json::from_str(&wire.body)?;
    assert_eq!(payload["correct"], "private-right-canary");
    let items = serde_json::to_value(runtime.notes.library_state())?;
    let refusal = items
        .as_array()
        .context("library array")?
        .iter()
        .find(|i| i["item"] == "dictionary_refused")
        .context("dictionary refusal")?;
    assert_eq!(refusal["failure"]["code"], "dictionary_full");
    assert_eq!(refusal["failure"]["numbers"]["terms"], 128);
    assert_eq!(refusal["failure"]["numbers"]["max_terms"], 128);
    runtime.report_notices();
    let report = std::fs::read_to_string(directory.path().join("logs/report-daemon.log"))?;
    assert!(
        report.contains("dictionary_full")
            && report.contains(r#""count":128"#)
            && report.contains(r#""total":128"#)
    );
    assert!(!report.contains("private-wrong-canary") && !report.contains("private-right-canary"));
    Ok(())
}

/// The hub's answer to a correction into several dictionaries when another
/// dictionary of the meeting hears the form otherwise: the app is told, by
/// the order's target, which one the meeting shows (the first wins). The
/// shape is the one `server/tests/test_meeting_dictionary_lists.py` asserts.
#[tokio::test]
async fn a_correction_s_conflicts_reach_the_library_by_its_target() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let answer = r#"{"dictionaries":[],"conflicts":[{"wrong":"Fondos","correct":"Fondant",
        "dictionary_id":"dic-dos","other_dictionary_id":"dic-uno","other_name":"Ferretería Albar",
        "other_correct":"LedgerBox","applied_dictionary_id":"dic-uno"}]}"#;
    let (base, seen) = one_shot_http(json_response("200 OK", answer)).await?;
    online_with_hub(&mut runtime, &base);
    let mut events = runtime.notes.test_events();
    let command: Command = serde_json::from_value(serde_json::json!({
        "command": "note_library", "request": {"action": "dictionary", "request": {
            "action": "correct", "recording_id": RECORDING_ID, "dictionary_ids": ["dic-dos"],
            "wrong": "Fondos", "correct": "Fondant", "name": null
        }}
    }))?;
    runtime.command(command).await?;
    runtime
        .note_answer(events.recv().await.context("dictionary answer")?)
        .await?;
    recorded(&seen)?;
    let items = serde_json::to_value(runtime.notes.library_state())?;
    let warned = items
        .as_array()
        .context("library array")?
        .iter()
        .find(|item| item["item"] == "dictionary_conflicts")
        .context("the conflicts of the correction")?;
    assert_eq!(warned["target"], "dictionaries:dic-dos");
    assert_eq!(
        warned["conflicts"],
        serde_json::json!([{"wrong":"Fondos","correct":"Fondant","dictionary_id":"dic-dos",
            "other_dictionary_id":"dic-uno","other_name":"Ferretería Albar",
            "other_correct":"LedgerBox","applied_dictionary_id":"dic-uno"}])
    );
    Ok(())
}

/// A hub up to 0.10.1 answers a correction with the dictionary alone: no
/// conflicts is what it says, not an unreadable answer.
#[tokio::test]
async fn an_older_hub_s_correction_has_no_conflicts_to_show() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let answer = r#"{"id":"dic-uno","name":"Proyecto","terms":[],"series":[]}"#;
    let (base, seen) = one_shot_http(json_response("200 OK", answer)).await?;
    online_with_hub(&mut runtime, &base);
    let mut events = runtime.notes.test_events();
    let command: Command = serde_json::from_value(serde_json::json!({
        "command": "note_library", "request": {"action": "dictionary", "request": {
            "action": "correct", "recording_id": RECORDING_ID, "dictionary_id": "dic-uno",
            "wrong": "fondos", "correct": "LedgerBox", "name": null
        }}
    }))?;
    runtime.command(command).await?;
    runtime
        .note_answer(events.recv().await.context("dictionary answer")?)
        .await?;
    recorded(&seen)?;
    let items = serde_json::to_value(runtime.notes.library_state())?;
    let items = items.as_array().context("library array")?;
    assert!(
        !items
            .iter()
            .any(|item| item["item"] == "dictionary_conflicts")
    );
    assert!(
        !items
            .iter()
            .any(|item| item["item"] == "dictionary_refused")
    );
    Ok(())
}

#[tokio::test]
async fn an_offline_dictionary_order_is_refused_without_retaining_terms_to_retry() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let base = reqwest::Url::parse("http://127.0.0.1:0")?;
    online_with_hub(&mut runtime, &base);
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "connection",
        serde_json::json!(Connection::Offline),
    );
    let command: Command = serde_json::from_value(serde_json::json!({
        "command":"note_library", "request":{"action":"dictionary","request":{
            "action":"correct", "recording_id":RECORDING_ID, "dictionary_id":SESSION_ID,
            "wrong":"offline-private-wrong", "correct":"offline-private-correct", "name":null
        }}
    }))?;
    runtime.command(command).await?;
    let state = serde_json::to_value(runtime.notes.library_state())?;
    let items = state.as_array().context("library")?;
    assert!(!items.iter().any(|item| item["item"] == "pending"));
    let refusal = items
        .iter()
        .find(|item| item["item"] == "dictionary_refused")
        .context("offline refusal")?;
    assert_eq!(refusal["failure"]["code"], "hub_unreachable");
    assert_eq!(refusal["failure"]["detail"], "El hub no está conectado");
    assert!(!state.to_string().contains("offline-private"));
    Ok(())
}

#[tokio::test]
async fn a_confirmed_dictionary_change_invalidates_every_recording_read() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let (base, seen) = one_shot_http(json_response("200 OK", "{}")).await?;
    online_with_hub(&mut runtime, &base);
    runtime.notes.head.touch_recording(RECORDING_ID);
    let before = runtime.notes.head.changed_at(RECORDING_ID);
    let mut events = runtime.notes.test_events();
    let command: Command = serde_json::from_value(serde_json::json!({
        "command": "note_library", "request": {"action": "dictionary", "request": {
            "action": "remove", "dictionary_id": SESSION_ID, "entry_id": "entry"
        }}
    }))?;
    runtime.command(command).await?;
    runtime
        .note_answer(events.recv().await.context("dictionary confirmation")?)
        .await?;
    recorded(&seen)?;
    let head = &runtime.notes.head;
    assert!(head.changed_at(RECORDING_ID) > before);
    assert_eq!(
        head.changed_at("40000000-0000-4000-8000-000000000004"),
        head.revision
    );
    assert!(head.library_revision > 0);
    Ok(())
}

#[tokio::test]
async fn local_access_refusals_keep_their_actual_cause() -> Result<()> {
    for (stopped, expected) in [(false, "not_paired"), (true, "privacy_stopped")] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _helper) = fixture(directory.path())?;
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "privacy_stopped",
            serde_json::json!(stopped),
        );
        let command: Command = serde_json::from_value(serde_json::json!({
            "command": "note_library", "request": {"action": "dictionary", "request": {
                "action": "remove", "dictionary_id": SESSION_ID, "entry_id": "entry"
            }}
        }))?;
        runtime.command(command).await?;
        let state = serde_json::to_value(runtime.notes.library_state())?;
        let failure = state
            .as_array()
            .context("library")?
            .iter()
            .find(|item| item["item"] == "dictionary_refused")
            .context("local refusal")?;
        assert_eq!(failure["failure"]["code"], expected);
        assert!(
            !failure["failure"]["detail"]
                .as_str()
                .context("detail")?
                .contains("HTTP 0")
        );
    }
    Ok(())
}
