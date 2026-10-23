//! Voice notes, transcription and speech notices at the daemon boundary. Audio, HTTP and helper traffic
//! are synthetic fixtures; no provider, device or real profile is contacted.
#![allow(clippy::unwrap_used)]

use super::*;
use crate::daemon::audio::tests::fixture;
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::{CallPhase, CallState, Event};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(super) const TOKEN: &str = "synthetic-not-a-credential-voice";
const SPEECH_NOTICE: &str = "La voz de Deimos no está disponible; contesta por texto";

pub(super) fn frame(stream: &str, frames: usize) -> Value {
    json!({"event":"audio", "source":"mic_local", "stream_id":stream,
        "sample_rate":8000, "channels":1, "frames":frames, "sequence":0,
        "monotonic_ns":0, "pcm_s16le_base64":STANDARD.encode(vec![1; frames * 2])})
}

pub(super) fn json_response(status: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

pub(super) async fn one_shot_http(response: Vec<u8>) -> Result<reqwest::Url> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = reqwest::Url::parse(&format!("http://{}", listener.local_addr()?))?;
    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 4096];
        let head_end = loop {
            if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break position + 4;
            }
            let Ok(count) = socket.read(&mut chunk).await else {
                return;
            };
            if count == 0 || bytes.len() + count > 131_072 {
                return;
            }
            bytes.extend_from_slice(&chunk[..count]);
        };
        let head = String::from_utf8_lossy(&bytes[..head_end]);
        let content_length = head
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        while bytes.len() < head_end + content_length {
            let Ok(count) = socket.read(&mut chunk).await else {
                return;
            };
            if count == 0 {
                return;
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        let _ = socket.write_all(&response).await;
    });
    Ok(base)
}

/// Durability and the message are observable before STT returns; the
/// completion enriches that same message instead of replacing its old text.
#[tokio::test]
async fn voice_wav_is_readable_by_message_id_and_transcript_is_optional() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    let (hub, _hub_rx) = mpsc::channel(4);
    runtime.link.outbound.install(hub);
    let (work, mut work_events) = mpsc::channel(16);
    runtime.work_events = work;
    runtime.settings.hub_url = Some(
        one_shot_http(json_response("200 OK", "{\"text\":\"nota sintética\"}"))
            .await?
            .to_string(),
    );
    runtime.link.token = Some(TOKEN.into());

    runtime.apply(Event::VoiceStart).await?;
    assert_eq!(
        helper.recv().await.context("playback_stop ausente")?["command"],
        "playback_stop"
    );
    let start = helper.recv().await.context("audio_start ausente")?;
    let stream = start["stream_id"]
        .as_str()
        .context("stream ausente")?
        .to_owned();
    runtime
        .native_event(json!({"event":"response","id":start["id"],"ok":true,
            "result":{"stream_id":stream}}))
        .await?;
    runtime.native_event(frame(&stream, 8000)).await?;
    runtime.apply(Event::VoiceEnd).await?;
    let stop = helper.recv().await.context("audio_stop ausente")?;
    runtime
        .native_event(json!({"event":"response","id":stop["id"],"ok":true,
            "result":{"stream_id":stream,"drain_complete":true}}))
        .await?;

    let pending = &runtime
        .state
        .durable
        .assistant
        .turn
        .as_ref()
        .context("nota ausente")?
        .user;
    let message_id = pending.id.clone();
    let pending_voice = pending
        .voice
        .as_ref()
        .context("metadatos de voz ausentes")?;
    assert_eq!(pending.role, "user");
    assert!(pending.at_ms > 0);
    assert!(pending_voice.duration_ms > 0);
    assert_eq!(pending_voice.transcript, None);
    assert!(pending_voice.audio_available);
    assert_eq!(pending.text, "");

    let audio = runtime.voice_note_audio(&message_id)?;
    assert_eq!(audio.message_id, message_id);
    assert_eq!(audio.format, "wav");
    let wav = STANDARD.decode(audio.bytes_base64)?;
    assert_eq!(&wav[..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
    assert_eq!((wav.len() - 44) / 2, 8000);
    let missing = runtime.voice_note_audio("missing-message").unwrap_err();
    assert_eq!(missing.to_string(), "voice_note_missing");

    let transcribed = tokio::time::timeout(Duration::from_secs(5), work_events.recv())
        .await
        .map_err(|_| anyhow::anyhow!("the synthetic transcription never arrived"))?
        .context("canal de trabajos cerrado")?;
    runtime.work_event(transcribed).await?;
    let completed = &runtime
        .state
        .durable
        .assistant
        .turn
        .as_ref()
        .context("nota ausente")?
        .user;
    assert_eq!(completed.id, message_id);
    assert_eq!(completed.role, "user");
    assert!(completed.at_ms > 0);
    assert!(
        completed
            .voice
            .as_ref()
            .context("metadatos de voz perdidos")?
            .duration_ms
            > 0
    );
    assert_eq!(completed.text, "nota sintética");
    assert_eq!(
        completed
            .voice
            .as_ref()
            .and_then(|voice| voice.transcript.as_deref()),
        Some("nota sintética")
    );
    Ok(())
}

#[tokio::test]
async fn failed_hub_transcription_replaces_pending_note_with_cause() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    let (hub, _hub_rx) = mpsc::channel(4);
    runtime.link.outbound.install(hub);
    let (work, mut work_events) = mpsc::channel(16);
    runtime.work_events = work;
    runtime.settings.hub_url = Some(
        one_shot_http(json_response(
            "502 Bad Gateway",
            r#"{"code":"transcription_http_error","message":"El servicio de transcripción devolvió HTTP 503","refusal":{"code":"transcription_http_error","provider_status":503,"retry":{"kind":"never"}}}"#,
        ))
        .await?
        .to_string(),
    );
    runtime.link.token = Some(TOKEN.into());

    runtime.apply(Event::VoiceStart).await?;
    helper.recv().await.context("playback_stop ausente")?;
    let start = helper.recv().await.context("audio_start ausente")?;
    let stream = start["stream_id"].as_str().context("stream ausente")?;
    runtime
        .native_event(json!({"event":"response","id":start["id"],"ok":true,
            "result":{"stream_id":stream}}))
        .await?;
    runtime.native_event(frame(stream, 8000)).await?;
    runtime.apply(Event::VoiceEnd).await?;
    let stop = helper.recv().await.context("audio_stop ausente")?;
    runtime
        .native_event(json!({"event":"response","id":stop["id"],"ok":true,
            "result":{"stream_id":stream,"drain_complete":true}}))
        .await?;
    let message_id = runtime
        .state
        .durable
        .assistant
        .turn
        .as_ref()
        .context("nota ausente")?
        .user
        .id
        .clone();
    let failed = work_events.recv().await.context("synthetic reply absent")?;
    let mut board = super::topics::TopicBoard::new("synthetic".into());
    let channels = board.channels();
    let _assistant = channels[&orbit_core::topics::TopicId::Assistant].subscribe();
    let _notices = channels[&orbit_core::topics::TopicId::Notices].subscribe();
    board.publish(&super::topics::TopicSource {
        state: &runtime.state,
        runtime: Some(&runtime),
        now_ms: 1,
    });
    board.state_changed();
    runtime.work_event(failed).await?;
    let note = Some(
        &runtime
            .state
            .durable
            .assistant
            .turn
            .as_ref()
            .context("nota perdida")?
            .user,
    )
    .filter(|message| message.id == message_id)
    .context("nota perdida")?;
    let wire = serde_json::to_value(note)?;
    assert_eq!(wire["voice"]["failure"]["code"], "transcription_http_error");
    assert_eq!(wire["voice"]["failure"]["provider_status"], 503);
    assert!(wire["voice"]["transcript"].is_null());
    assert!(wire["voice"]["audio_available"].as_bool().unwrap_or(false));
    assert_ne!(
        runtime.state.notice_code(),
        Some(orbit_core::notice::NoticeCode::VoiceTranscribing)
    );
    let saved: orbit_core::State = runtime
        .fixture_store
        .load_snapshot()?
        .context("snapshot ausente")?;
    let saved_note = Some(
        &saved
            .durable
            .assistant
            .turn
            .as_ref()
            .context("note not persisted")?
            .user,
    )
    .filter(|message| message.id == message_id)
    .context("note not persisted")?;
    assert_eq!(
        saved_note
            .voice
            .as_ref()
            .and_then(|voice| voice.failure.as_ref())
            .and_then(|failure| failure.provider_status),
        Some(503)
    );
    let published = board.publish(&super::topics::TopicSource {
        state: &runtime.state,
        runtime: Some(&runtime),
        now_ms: 1 + orbit_core::topics::TopicId::Notices
            .min_interval_ms()
            .max(orbit_core::topics::TopicId::Assistant.min_interval_ms()),
    });
    assert!(published.contains(&orbit_core::topics::TopicId::Assistant));
    assert!(published.contains(&orbit_core::topics::TopicId::Notices));
    let topic = serde_json::to_value(
        channels[&orbit_core::topics::TopicId::Assistant]
            .borrow()
            .data
            .clone(),
    )?;
    assert_eq!(
        topic["knowledge"]["value"]["turn"]["user"]["voice"]["failure"]["provider_status"],
        503
    );
    assert!(topic["knowledge"]["value"]["activity"].is_null());
    Ok(())
}

#[tokio::test]
async fn oversized_hub_refusal_keeps_body_size_and_ceiling() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let maximum = orbit_core::limits::MAX_HUB_HTTP_BODY_BYTES;
    let declared = maximum + 1;
    let response = format!(
        "HTTP/1.1 502 Bad Gateway\r\nContent-Type: application/json\r\nContent-Length: {declared}\r\nConnection: close\r\n\r\n"
    );
    runtime.settings.hub_url = Some(one_shot_http(response.into_bytes()).await?.to_string());
    let failure = crate::hub::transcribe(
        &runtime.settings,
        TOKEN,
        true,
        vec![0],
        &orbit_ipc::new_op(),
    )
    .await
    .unwrap_err();
    let wire = serde_json::to_value(failure)?;
    assert_eq!(wire["code"], "transcription_hub_response_too_large");
    assert_eq!(wire["bytes"], declared);
    assert_eq!(wire["max_bytes"], maximum);
    Ok(())
}

/// The HTTP code is translated at the boundary and the actor records it
/// only once while the same call remains active.
#[tokio::test]
async fn speech_unavailable_is_specific_and_once_per_call() -> Result<()> {
    let base = one_shot_http(json_response(
        "400 Bad Request",
        "{\"code\":\"speech_unavailable\",\"detail\":\"synthetic\"}",
    ))
    .await?;
    let settings = crate::config::Settings {
        hub_url: Some(base.to_string()),
        ..crate::config::Settings::default()
    };
    let (native, _native_rx) = mpsc::channel(2);
    let error = crate::speech::play(
        settings,
        TOKEN.into(),
        true,
        "respuesta sintética".into(),
        None,
        "playback-synthetic".into(),
        native,
        orbit_ipc::new_op(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), SPEECH_NOTICE);

    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    runtime.state.durable.voice.call = Some(CallState {
        id: "call-speech".into(),
        phase: CallPhase::Responding,
        started_at_ms: 1,
        expires_at_ms: 60_001,
        turns: 1,
        origin: orbit_core::CallOrigin::App,
    });
    for _ in 0..2 {
        let generation = runtime.state.generation();
        let ticket = crate::daemon::audio::tests::pending_work(
            &mut runtime.assistant.speech_task,
            generation,
            (),
        )?;
        runtime
            .work_event(WorkResult::Problem {
                ticket,
                message: SPEECH_NOTICE.into(),
            })
            .await?;
        assert!(!runtime.state.privacy_stopped());
        assert!(!runtime.audio.audio_is_draining());
        assert_eq!(runtime.state.views.draining_since_ms, None);
    }
    let notices = runtime
        .state
        .durable
        .notice_log
        .records()
        .iter()
        .filter(|notice| notice.code == orbit_core::notice::NoticeCode::SpeechFailed)
        .count();
    assert_eq!(notices, 1);
    Ok(())
}

#[tokio::test]
async fn speech_refusal_reaches_the_assistant_topic_as_a_code() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let (work, mut work_events) = mpsc::channel(16);
    runtime.work_events = work;
    runtime.settings.hub_url = Some(
        one_shot_http(json_response(
            "400 Bad Request",
            "{\"code\":\"speech_unavailable\"}",
        ))
        .await?
        .to_string(),
    );
    runtime.link.token = Some(TOKEN.into());
    runtime.assistant.speak_next = true;
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.native_helper",
            fixture_value,
        );
    }
    runtime.state.durable.assistant.turn = Some(orbit_core::Turn {
        user: orbit_core::Message {
            attachments: Vec::new(),
            id: "speech-refusal".into(),
            role: "user".into(),
            text: "Pregunta sintética".into(),
            complete: true,
            at_ms: 1,
            voice: None,
        },
        answer: Some(orbit_core::Message {
            attachments: Vec::new(),
            id: "speech-refusal:assistant".into(),
            role: "assistant".into(),
            text: "Respuesta sintética".into(),
            complete: true,
            at_ms: 2,
            voice: None,
        }),
    });
    runtime.start_speech();
    let completed = tokio::time::timeout(Duration::from_secs(5), work_events.recv())
        .await?
        .context("the speech work did not arrive")?;
    runtime.work_event(completed).await?;
    let mut board = super::topics::TopicBoard::new("synthetic".into());
    let channels = board.channels();
    let _receiver = channels[&orbit_core::topics::TopicId::Assistant].subscribe();
    board.publish(&super::topics::TopicSource {
        state: &runtime.state,
        runtime: Some(&runtime),
        now_ms: 1,
    });
    let data = serde_json::to_value(
        channels[&orbit_core::topics::TopicId::Assistant]
            .borrow()
            .data
            .clone(),
    )?;
    assert_eq!(data["knowledge"]["state"], "present");
    assert_eq!(data["knowledge"]["value"]["voice"]["state"], "absent");
    assert_eq!(
        data["knowledge"]["value"]["voice"]["code"],
        "speech_unavailable"
    );
    Ok(())
}

#[tokio::test]
async fn speech_unavailable_names_the_configured_assistant() -> Result<()> {
    let base = one_shot_http(json_response(
        "400 Bad Request",
        "{\"code\":\"speech_unavailable\"}",
    ))
    .await?;
    let settings: crate::config::Settings = serde_json::from_value(json!({
        "version": 1, "device_name": "Fixture Mac", "hub_url": base.to_string(),
        "speak_responses": false, "assistant_name": "Atlas"
    }))?;
    let (native, _receiver) = mpsc::channel(2);
    let error = crate::speech::play(
        settings,
        TOKEN.into(),
        true,
        "Respuesta".into(),
        None,
        "fixture-playback".into(),
        native,
        orbit_ipc::new_op(),
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "La voz de Atlas no está disponible; contesta por texto"
    );
    Ok(())
}

/// With several agents, the notice names the one whose voice was asked
/// for, not the default one.
#[tokio::test]
async fn speech_unavailable_names_the_agent_asked_for() -> Result<()> {
    let base = one_shot_http(json_response(
        "400 Bad Request",
        "{\"code\":\"speech_unavailable\"}",
    ))
    .await?;
    let mut settings = crate::config::Settings {
        hub_url: Some(base.to_string()),
        ..crate::config::Settings::default()
    };
    settings.hub_agents = orbit_core::agents::AgentRoster::admit(
        vec![
            orbit_core::agents::Agent::named("vega", "Vega"),
            orbit_core::agents::Agent::named("atlas", "Atlas"),
        ],
        Some("vega".into()),
    )
    .ok();
    let (native, _receiver) = mpsc::channel(2);
    let error = crate::speech::play(
        settings,
        TOKEN.into(),
        true,
        "Respuesta".into(),
        Some("atlas".into()),
        "fixture-playback".into(),
        native,
        orbit_ipc::new_op(),
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "La voz de Atlas no está disponible; contesta por texto"
    );
    Ok(())
}
