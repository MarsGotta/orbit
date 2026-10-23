//! «Silenciar» during a call with the assistant: it stops hearing the user,
//! who still hears it. While muted the helper sends silence (synthetic here,
//! with a loud frame standing for what a helper must never send), no turn
//! opens, and the muted time is not the silence that hangs up.
use super::*;

/// Mutes or unmutes the call's microphone, answered as the helper applied it.
async fn set_muted(
    runtime: &mut Runtime,
    helper: &mut mpsc::Receiver<Value>,
    stream: &str,
    muted: bool,
) -> Result<()> {
    runtime.command(Command::MicrophoneMute { muted }).await?;
    let order = loop {
        let command = helper.try_recv().context("mute order absent")?;
        if command["command"] != "status" {
            break command;
        }
    };
    assert_eq!(order["command"], "audio_mute");
    assert_eq!(order["stream_id"], stream);
    assert_eq!(
        runtime.call_microphone_muted(),
        !muted,
        "not before the answer"
    );
    runtime
        .native_event(json!({"event":"response","id":order["id"],"ok":true,
            "result":{"stream_id":stream,"muted":muted}}))
        .await?;
    assert_eq!(runtime.call_microphone_muted(), muted);
    Ok(())
}

#[tokio::test]
async fn a_muted_call_opens_no_turn_never_hangs_up_for_silence_and_listens_again() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    let stream = start_call_for(&mut runtime, &mut helper, 600_000).await?;
    let started = runtime
        .state
        .durable
        .voice
        .call
        .as_ref()
        .context("call absent")?
        .started_at_ms;
    set_muted(&mut runtime, &mut helper, &stream, true).await?;
    for sequence in 0..20 {
        let action = runtime
            .prepare_call_frame(&call_frame(&stream, sequence, true), started + 1_000)
            .await?;
        assert_eq!(
            action,
            orbit_core::voice::TurnAction::None,
            "no turn while muted"
        );
    }
    let action = runtime
        .prepare_call_frame(&call_frame(&stream, 20, false), started + 200_000)
        .await?;
    assert_eq!(action, orbit_core::voice::TurnAction::None);
    assert!(
        runtime.state.durable.voice.call.is_some(),
        "three minutes muted are not silence that hangs up"
    );
    set_muted(&mut runtime, &mut helper, &stream, false).await?;
    let mut began = false;
    for sequence in 21..40 {
        let action = runtime
            .prepare_call_frame(&call_frame(&stream, sequence, true), started + 200_100)
            .await?;
        began |= matches!(action, orbit_core::voice::TurnAction::Begin { .. });
    }
    assert!(began, "unmuted, the call hears the user again");
    Ok(())
}
