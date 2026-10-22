//! The composer's voice note at the daemon boundary: Esc cancels a note
//! without sending it, a note shorter than the speech minimum is not sent and
//! keeps its numbers for the composer, and «Reintentar transcripción» sends
//! the stored WAV again or says that the audio is gone. Audio, HTTP and helper
//! traffic are synthetic fixtures.
#![allow(clippy::unwrap_used)]

use super::capability_notice_tests::{TOKEN, frame, json_response, one_shot_http};
use super::*;
use crate::daemon::audio::tests::fixture;
use crate::daemon::turn::said;
use orbit_core::{
    Event,
    notice::NoticeCode,
    refusal::{Refusal, Resource},
    voice::ShortVoiceNote,
};

/// The helper's side of a voice note: it starts, delivers `frames` samples
/// at 8 kHz, and confirms the stop that `end` asks for.
async fn record(
    runtime: &mut Runtime,
    helper: &mut mpsc::Receiver<Value>,
    frames: usize,
    end: Command,
) -> Result<()> {
    runtime.command(Command::VoiceStart).await?;
    helper.recv().await.context("playback_stop absent")?;
    let start = helper.recv().await.context("audio_start absent")?;
    let stream = start["stream_id"]
        .as_str()
        .context("stream absent")?
        .to_owned();
    runtime
        .native_event(json!({"event":"response","id":start["id"],"ok":true,
            "result":{"stream_id":stream}}))
        .await?;
    runtime.native_event(frame(&stream, frames)).await?;
    runtime.command(end).await?;
    let stop = helper.recv().await.context("audio_stop absent")?;
    runtime
        .native_event(json!({"event":"response","id":stop["id"],"ok":true,
            "result":{"stream_id":stream,"drain_complete":true}}))
        .await
}

fn failure_code(runtime: &Runtime) -> Option<String> {
    runtime
        .state
        .durable
        .turn
        .as_ref()
        .and_then(|turn| turn.user.voice.as_ref())
        .and_then(|voice| voice.failure.as_ref())
        .map(|failure| failure.code.clone())
}

#[tokio::test]
async fn esc_cancels_the_note_without_transcribing_or_sending_it() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    runtime.token = Some(TOKEN.into());
    record(&mut runtime, &mut helper, 8_000, Command::VoiceCancel).await?;
    assert!(!runtime.state.durable.voice_active);
    assert!(runtime.state.durable.turn.is_none(), "no message was made");
    assert!(runtime.conversation.voice_task.active().is_none());
    assert!(said::has(&runtime.state, NoticeCode::VoiceTurnCancelled));
    assert!(!said::has(&runtime.state, NoticeCode::VoiceTranscribing));
    Ok(())
}

#[tokio::test]
async fn a_note_under_the_minimum_is_not_sent_and_keeps_its_numbers() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    runtime.token = Some(TOKEN.into());
    // 720 samples at 8 kHz: 90 ms, a click.
    record(&mut runtime, &mut helper, 720, Command::VoiceEnd).await?;
    assert_eq!(
        runtime.state.views.short_voice_note,
        Some(ShortVoiceNote {
            duration_ms: 90,
            minimum_ms: 160
        })
    );
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::VoiceNoteTooShort)
    );
    assert!(runtime.state.durable.turn.is_none(), "no message was made");
    assert!(runtime.conversation.voice_task.active().is_none());
    assert!(!said::has(&runtime.state, NoticeCode::VoiceTranscribing));
    assert!(
        runtime
            .fixture_store
            .list_jobs(10)?
            .iter()
            .all(|job| job.kind != "recording_manifest"),
        "a note not sent is not stored either"
    );
    Ok(())
}

#[tokio::test]
async fn retry_sends_the_stored_audio_again_and_completes_the_note() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    let (hub, _hub_rx) = mpsc::channel(4);
    runtime.hub.install(hub);
    let (work, mut work_events) = mpsc::channel(16);
    runtime.work.work_events = work;
    // Unpaired when the note is recorded: its transcription fails, its WAV stays.
    let unpaired = record(&mut runtime, &mut helper, 8_000, Command::VoiceEnd).await;
    assert!(unpaired.is_err());
    assert_eq!(
        failure_code(&runtime).as_deref(),
        Some("voice_hub_pairing_required")
    );
    let message_id = runtime
        .state
        .durable
        .turn
        .as_ref()
        .context("note absent")?
        .user
        .id
        .clone();

    runtime.token = Some(TOKEN.into());
    runtime.settings.hub_url = Some(
        one_shot_http(json_response("200 OK", "{\"text\":\"nota repetida\"}"))
            .await?
            .to_string(),
    );
    runtime
        .command(Command::VoiceRetry {
            message_id: message_id.clone(),
        })
        .await?;
    assert_eq!(
        failure_code(&runtime),
        None,
        "it reads as transcribing again"
    );
    assert!(said::has(&runtime.state, NoticeCode::VoiceTranscribing));
    let transcribed = tokio::time::timeout(Duration::from_secs(5), work_events.recv())
        .await
        .map_err(|_| anyhow::anyhow!("the synthetic transcription never arrived"))?
        .context("work channel closed")?;
    runtime.work_event(transcribed).await?;
    let user = &runtime
        .state
        .durable
        .turn
        .as_ref()
        .context("note absent")?
        .user;
    assert_eq!(user.id, message_id);
    assert_eq!(user.text, "nota repetida");
    assert_eq!(
        user.voice
            .as_ref()
            .and_then(|voice| voice.transcript.as_deref()),
        Some("nota repetida")
    );
    Ok(())
}

#[tokio::test]
async fn retry_without_the_audio_says_the_audio_is_gone() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    runtime.token = Some(TOKEN.into());
    runtime
        .apply(Event::VoiceStored {
            id: "voice-without-audio".into(),
            duration_ms: 2_000,
        })
        .await?;
    runtime
        .apply(Event::VoiceFailed {
            id: "voice-without-audio".into(),
            failure: orbit_core::VoiceTranscriptionFailure::local("transcription_upstream_failed"),
        })
        .await?;
    let refused = runtime
        .command(Command::VoiceRetry {
            message_id: "voice-without-audio".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(
        refused.downcast_ref::<Refusal>(),
        Some(&Refusal::NotFound {
            resource: Resource::Audio
        })
    );
    assert_eq!(
        failure_code(&runtime).as_deref(),
        Some("transcription_upstream_failed"),
        "the note keeps saying why it failed"
    );
    assert!(runtime.conversation.voice_task.active().is_none());
    Ok(())
}
