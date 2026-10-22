//! A `MeetingStart` title over the manifest maximum is refused
//! naming the title, its length and the maximum, so the app or the CLI that
//! sent it can correct itself in one step. Synthetic fixtures only.
use super::*;
use crate::daemon::audio::tests::fixture;
use orbit_core::protocol::MeetingAudio;

/// The refusal for a title longer than the manifest maximum
/// says it is the title, how many characters it has and what the maximum
/// is, and reaches the asker as a typed IPC refusal. Before, the same case
/// answered `InvalidMeeting` — "un título y PID de aplicación válidos" —
/// with no numbers, indistinguishable from an empty title or a bad PID.
#[tokio::test]
async fn a_long_title_refusal_names_the_title_its_length_and_the_maximum() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    // "á" doubles the UTF-8 bytes of the title: the refusal must count
    // characters, which is what the maximum is defined in.
    let characters = orbit_core::MAX_MANIFEST_TITLE_CHARACTERS + 1;
    let error = runtime
        .command(Command::MeetingStart {
            title: "á".repeat(characters),
            audio: MeetingAudio::MicrophoneOnly,
            duration_minutes: None,
            event: None,
        })
        .await
        .err()
        .context("the over-long title must be refused")?;
    let refusal = refusal_from_error(&error);
    assert_eq!(
        refusal,
        orbit_core::refusal::Refusal::InvalidParameter {
            parameter: orbit_core::refusal::Parameter::Title,
            expected: orbit_core::refusal::Expected::TitleLength {
                characters: characters as u64,
                max_characters: orbit_core::MAX_MANIFEST_TITLE_CHARACTERS as u64,
            },
        },
        "the refusal must stay typed through the IPC path"
    );
    let text = refusal.to_string();
    assert!(
        serde_json::from_str::<serde_json::Value>(&text).is_ok_and(|v| v["parameter"] == "title"),
        "the refusal must name the title: {text}"
    );
    assert!(
        text.contains(&characters.to_string()),
        "the refusal must say how many characters it has: {text}"
    );
    assert!(
        text.contains(&orbit_core::MAX_MANIFEST_TITLE_CHARACTERS.to_string()),
        "the refusal must say the maximum: {text}"
    );
    // Nothing started: no helper order and no recording.
    crate::daemon::audio::tests::assert_no_helper_command(&mut receiver)?;
    assert!(runtime.state.durable.recording.is_none());
    // The neighbouring gates keep their own causes, not the title's numbers.
    let empty = runtime
        .command(Command::MeetingStart {
            title: "   ".into(),
            audio: MeetingAudio::MicrophoneOnly,
            duration_minutes: None,
            event: None,
        })
        .await
        .err()
        .context("an empty title must still be refused")?;
    let empty = refusal_from_error(&empty).to_string();
    assert!(
        !empty.contains(&characters.to_string()),
        "the empty title is another cause: {empty}"
    );
    runtime
        .command(Command::MeetingStart {
            title: "á".repeat(orbit_core::MAX_MANIFEST_TITLE_CHARACTERS),
            audio: MeetingAudio::MicrophoneOnly,
            duration_minutes: None,
            event: None,
        })
        .await?;
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording absent")?
            .title
            .chars()
            .count(),
        orbit_core::MAX_MANIFEST_TITLE_CHARACTERS
    );
    Ok(())
}

/// Every recording runs to the one safety ceiling, whatever its sender
/// asked: a duration from an older sender is accepted and has no effect.
#[tokio::test]
async fn every_recording_runs_to_the_one_safety_ceiling() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    runtime
        .command(Command::MeetingStart {
            title: "Revisión trimestral".into(),
            audio: MeetingAudio::MicrophoneOnly,
            duration_minutes: Some(30),
            event: None,
        })
        .await?;
    let recording = runtime
        .state
        .durable
        .recording
        .as_ref()
        .context("recording absent")?;
    assert_eq!(
        recording.expires_at_ms - recording.started_at_ms,
        240 * 60_000,
        "a recording stops only at the safety ceiling"
    );
    Ok(())
}
