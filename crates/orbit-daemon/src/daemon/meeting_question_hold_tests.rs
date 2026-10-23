//! The pointer over the floating panel holds the question there: the
//! daemon, which lets it go at `shown_until_ms`, waits while it is held and
//! gives it a short grace once the pointer leaves. A synthetic helper and
//! clock; nothing native.
use crate::daemon::{Runtime, audio::tests::fixture};
use anyhow::{Context, Result};
use orbit_core::{
    meeting_question::{FAILURE_SHOWN_MS, HOLD_GRACE_MS, QuestionCause},
    protocol::Command,
};

const NOW: i64 = 1_800_000_000_000;

/// A question refused for having no meeting: shown for eight seconds.
async fn refused_question(runtime: &mut Runtime) -> Result<String> {
    runtime.question.clock_ms = Some(NOW);
    runtime
        .command(Command::MeetingAsk {
            question: "¿Qué se decidió?".into(),
            minutes: 2,
        })
        .await?;
    let question = runtime
        .question
        .current
        .clone()
        .context("the refused question")?;
    assert_eq!(question.cause, Some(QuestionCause::NoMeeting));
    Ok(question.id)
}

async fn at(runtime: &mut Runtime, now_ms: i64) -> Result<bool> {
    runtime.question.clock_ms = Some(now_ms);
    runtime.maintain_meeting_question().await?;
    Ok(runtime.question.current.is_some())
}

#[tokio::test]
async fn a_held_question_stays_past_its_time_and_goes_after_the_grace() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let id = refused_question(&mut runtime).await?;
    runtime.question.clock_ms = Some(NOW + 1_000);
    runtime
        .command(Command::MeetingQuestionHold {
            id: id.clone(),
            held: true,
        })
        .await?;
    assert!(at(&mut runtime, NOW + FAILURE_SHOWN_MS + 60_000).await?);
    runtime
        .command(Command::MeetingQuestionHold { id, held: false })
        .await?;
    let released = NOW + FAILURE_SHOWN_MS + 60_000;
    assert!(at(&mut runtime, released + HOLD_GRACE_MS - 1).await?);
    assert!(!at(&mut runtime, released + HOLD_GRACE_MS).await?);
    Ok(())
}

#[tokio::test]
async fn holding_another_question_changes_nothing() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    refused_question(&mut runtime).await?;
    runtime
        .command(Command::MeetingQuestionHold {
            id: "another".into(),
            held: true,
        })
        .await?;
    assert!(!at(&mut runtime, NOW + FAILURE_SHOWN_MS).await?);
    // A hold for a question already gone is not an error either.
    runtime
        .command(Command::MeetingQuestionHold {
            id: "another".into(),
            held: false,
        })
        .await?;
    Ok(())
}
