//! DAE-02 acceptance at the daemon actor: message timestamps and voice-note
//! duration, the account the state publishes after pairing, and the tri-state
//! permissions the helper reports. The helper is the fixture's fake channel,
//! the hub peers are one-shot loopback HTTP fixtures, and every value on the
//! wire is synthetic.
use super::*;
use crate::daemon::audio::tests::fixture;
use orbit_core::protocol::Command;
use orbit_core::voice::{CallPhase, CallState};
use orbit_core::{CapturePhase, Event, PermissionState};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const DEVICE_ID: &str = "20000000-0000-4000-8000-000000000002";
const DEVICE_TOKEN: &str = "synthetic-not-a-credential-dae02";
const DEVICE_NAME: &str = "Mac de la fixture DAE-02";

fn json_response(status: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// One loopback HTTP conversation, modeled on the DAE-01 fixtures: a single
/// request is read in full and answered with the prepared raw response. No
/// production hub, provider or credential is contacted.
async fn one_shot_http(response: Vec<u8>) -> Result<reqwest::Url> {
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
            let count = match socket.read(&mut chunk).await {
                Ok(count) => count,
                Err(_) => return,
            };
            if count == 0 || bytes.len() + count > 131_072 {
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
        let _ = socket.write_all(&response).await;
    });
    Ok(base)
}

/// A helper status answer: TCC gives the microphone a status; the screen and
/// accessibility preflights are booleans and cannot prove a denial.
fn helper_status(microphone: &str, screen: bool, accessibility: bool) -> Value {
    json!({"event":"response","id":"status","ok":true,
        "result":{"permissions":{"microphone":microphone,"screen":screen,
            "accessibility":accessibility,"calendars":"full_access"}}})
}

/// DEI-06 A1. The daemon, which owns call state, declares it on every chat turn.
#[tokio::test]
async fn dei06_a1_chat_send_declares_an_active_call() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let (hub, mut hub_frames) = mpsc::channel(16);
    runtime.hub.install(hub);
    runtime.state.durable.call = Some(CallState {
        id: "20000000-0000-4000-8000-000000000006".into(),
        phase: CallPhase::Listening,
        started_at_ms: 1_000,
        expires_at_ms: 61_000,
        turns: 0,
    });

    runtime
        .command(Command::Chat {
            text: "pregunta durante la llamada".into(),
        })
        .await?;

    let sent = hub_frames.recv().await.context("chat.send ausente")?;
    assert_eq!(sent["method"], "chat.send");
    assert_eq!(sent["params"]["call_active"], true);
    Ok(())
}

/// A2. A chat turn and its reply publish two messages whose moments allow
/// ordering them; a pushed-to-talk turn publishes the transcribed note as a
/// user message that carries its duration.
#[tokio::test]
async fn a2_chat_turn_messages_carry_increasing_at_ms() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let (hub, mut hub_frames) = mpsc::channel(16);
    runtime.hub.install(hub);

    runtime
        .command(Command::Chat {
            text: "pregunta sintética".into(),
        })
        .await?;
    let sent = hub_frames.recv().await.context("chat.send ausente")?;
    assert_eq!(sent["method"], "chat.send");
    let turn = runtime
        .state
        .durable
        .active_request
        .clone()
        .context("turno activo ausente")?;
    runtime
        .hub_event(HubEvent::Frame(json!({
            "event": "chat.delta", "id": turn,
            "data": {"text": "respuesta parcial sintética"}
        })))
        .await?;
    runtime
        .hub_event(HubEvent::Frame(json!({
            "event": "chat.completed", "id": turn,
            "data": {"text": "respuesta final sintética"}
        })))
        .await?;

    let turn = runtime
        .state
        .durable
        .turn
        .as_ref()
        .context("turno ausente")?;
    let answer = turn.answer.as_ref().context("respuesta ausente")?;
    let messages = [&turn.user, answer];
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[0].text, "pregunta sintética");
    assert_eq!(messages[1].role, "assistant");
    assert_eq!(messages[1].text, "respuesta final sintética");
    assert!(messages[1].complete);
    assert!(
        messages[0].at_ms > 0,
        "el mensaje del usuario no lleva su hora: {:?}",
        messages[0]
    );
    assert!(
        messages[1].at_ms > messages[0].at_ms,
        "las horas no permiten ordenar la conversación: {:?}",
        messages
            .iter()
            .map(|message| message.at_ms)
            .collect::<Vec<_>>()
    );
    Ok(())
}

/// A3. Pairing is the moment Ajustes learns the account: the state publishes
/// the hub, the device name and a short device identifier, and the device
/// credential never appears in any serialization of the state.
#[tokio::test]
async fn a3_pairing_publishes_the_account_without_the_token() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let base = one_shot_http(json_response(
        "200 OK",
        &format!("{{\"device_id\":\"{DEVICE_ID}\",\"token\":\"{DEVICE_TOKEN}\"}}"),
    ))
    .await?;

    runtime
        .command(Command::Pair {
            hub_url: base.to_string(),
            code: "codigo-sintetico".into(),
            device_name: DEVICE_NAME.into(),
        })
        .await?;

    let account = runtime
        .state
        .views
        .account
        .as_ref()
        .context("el estado no publica la cuenta")?;
    let stored = account
        .hub_url
        .as_deref()
        .context("la cuenta no publica el hub")?;
    assert_eq!(reqwest::Url::parse(stored)?, base);
    assert_eq!(
        account.device_name.as_deref(),
        Some(DEVICE_NAME),
        "la cuenta no publica el nombre del dispositivo"
    );
    let short = account
        .device_id_short
        .as_deref()
        .context("la cuenta no publica el identificador corto")?;
    assert_eq!(short.len(), 8, "device_id_short: {short}");
    assert_eq!(short, &DEVICE_ID[..8]);

    let served = serde_json::to_string(&runtime.state)?;
    assert!(
        !served.contains(DEVICE_TOKEN),
        "la credencial viajó en el estado publicado"
    );
    let durable = serde_json::to_string(&runtime.state.durable)?;
    assert!(
        !durable.contains(DEVICE_TOKEN),
        "la credencial viajó en el estado durable"
    );
    Ok(())
}

/// A4. The microphone's TCC status and the two preflight booleans reach the
/// permissions view while the capture decision uses the observed status.
#[tokio::test]
async fn a4_helper_permissions_feed_the_state_and_capture() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let meeting = || Event::StartMeeting {
        id: "dae02-recording".into(),
        title: "Reunión sintética".into(),
        audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
        duration_ms: 60_000,
    };

    // Each answer is paired with a status the daemon sent. An id it did not
    // send is unpaired and is not read as permissions (REF-404c).
    runtime.record_pending_native(&json!({"id": "status", "command": "status"}));
    runtime
        .native_event(helper_status("denied", false, true))
        .await?;
    assert_eq!(
        runtime.state.capabilities().microphone_state(),
        PermissionState::Denied
    );
    assert_eq!(
        runtime.state.capabilities().screen_state(),
        PermissionState::NotGranted
    );
    assert_eq!(
        runtime.state.capabilities().accessibility_state(),
        PermissionState::Granted
    );
    let refused = runtime
        .apply(meeting())
        .await
        .err()
        .context("un micrófono denegado debe rechazar la captura")?;
    assert_eq!(
        refused.downcast_ref::<orbit_core::refusal::Refusal>(),
        Some(&orbit_core::refusal::Refusal::PermissionDenied {
            permission: orbit_core::refusal::Permission::Microphone,
        })
    );

    runtime.record_pending_native(&json!({"id": "status", "command": "status"}));
    runtime
        .native_event(helper_status("notDetermined", true, false))
        .await?;
    assert_eq!(
        runtime.state.capabilities().microphone_state(),
        PermissionState::NotDetermined
    );
    let still_refused = runtime
        .apply(meeting())
        .await
        .err()
        .context("un micrófono sin determinar debe seguir rechazando la captura")?;
    assert_eq!(
        still_refused.downcast_ref::<orbit_core::refusal::Refusal>(),
        Some(&orbit_core::refusal::Refusal::PermissionUnknown {
            permission: orbit_core::refusal::Permission::Microphone,
        })
    );

    runtime.record_pending_native(&json!({"id": "status", "command": "status"}));
    runtime
        .native_event(helper_status("authorized", true, false))
        .await?;
    assert_eq!(
        runtime.state.capabilities().microphone_state(),
        PermissionState::Granted
    );
    runtime.apply(meeting()).await?;
    assert!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .is_some_and(|recording| recording.phase == CapturePhase::Starting)
    );
    Ok(())
}
