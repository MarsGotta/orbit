//! A deletion on this Mac whose worker panics is a deletion that failed: it
//! is owed again and retried, and the next one is not held behind it.
use super::*;
use crate::daemon::audio::tests::fixture;
use crate::supervision::Actor;
use std::time::Duration;

const THROWN_AWAY: &str = "8e2d6b1f-3c4a-4d5e-9f6a-7b8c9d0e1f2a";
const NOW: i64 = 1_800_000_000_000;

#[tokio::test]
async fn a_deletion_whose_worker_panics_is_owed_again_and_retried() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let mut answers = runtime.features.notes.test_events();
    runtime
        .features
        .notes
        .deletions
        .owed
        .insert(THROWN_AWAY.into(), DeletionScope::Audio);
    runtime.supervision.inject(Actor::NoteDeletion, 1);
    runtime.start_due_purge(NOW);
    assert_eq!(
        runtime.features.notes.deletions.purging.as_deref(),
        Some(THROWN_AWAY)
    );
    let answer = tokio::time::timeout(Duration::from_secs(30), answers.recv())
        .await?
        .ok_or_else(|| anyhow::anyhow!("the panicked deletion never answered"))?;
    runtime.note_answer_at(answer, NOW).await?;
    let deletions = &runtime.features.notes.deletions;
    assert_eq!(deletions.purging, None, "the deletion slot stayed taken");
    assert_eq!(deletions.owed.get(THROWN_AWAY), Some(&DeletionScope::Audio));
    assert!(deletions.retry_at_ms.contains_key(THROWN_AWAY));
    assert_eq!(runtime.supervision.restarts(Actor::NoteDeletion), 1);

    // Its retry runs, and this time it goes through.
    runtime.start_due_purge(NOW + orbit_core::note_outbox::FIRST_RETRY_MS);
    let answer = tokio::time::timeout(Duration::from_secs(30), answers.recv())
        .await?
        .ok_or_else(|| anyhow::anyhow!("the retried deletion never answered"))?;
    runtime
        .note_answer_at(answer, NOW + orbit_core::note_outbox::FIRST_RETRY_MS)
        .await?;
    let deletions = &runtime.features.notes.deletions;
    assert!(
        deletions.owed.is_empty(),
        "the retried deletion is still owed"
    );
    assert!(deletions.purged.contains_key(THROWN_AWAY));
    Ok(())
}
