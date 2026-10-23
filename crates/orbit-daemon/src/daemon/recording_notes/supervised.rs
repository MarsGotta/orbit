//! The notes' workers under supervision. A worker that ended in a panic can
//! deliver nothing: the device profile is owed again and the outbox's lanes
//! send again what the store still has waiting, as after a failed request.
use super::NotesHandle;
use crate::supervision::Actor;

impl NotesHandle {
    /// The notes' workers that fell since the last look, one entry each.
    pub(in crate::daemon) fn reap_panicked(&mut self) -> Vec<Actor> {
        let mut fell = Vec::new();
        if self.profile.panicked().is_some() {
            self.profile.cancel();
            self.profile_owed = true;
            fell.push(Actor::NoteProfile);
        }
        let lanes = self.outbox.reap_panicked();
        fell.extend(std::iter::repeat_n(Actor::NoteLane, lanes));
        fell
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[allow(clippy::panic, reason = "the request panics on purpose")]
    async fn a_profile_whose_request_panics_is_owed_again() -> anyhow::Result<()> {
        let (events, _received) = tokio::sync::mpsc::channel(super::super::NOTE_EVENTS);
        let mut notes = NotesHandle::for_tests(events);
        notes.profile.attach_test(
            0,
            (),
            tokio::spawn(async { panic!("injected profile panic") }),
        );
        notes.profile_owed = false;
        while notes
            .profile
            .task_handle()
            .is_some_and(|task| !task.is_finished())
        {
            tokio::task::yield_now().await;
        }
        assert_eq!(notes.reap_panicked(), vec![Actor::NoteProfile]);
        assert!(notes.profile.active().is_none());
        assert!(notes.profile_owed, "the hub never took the profile");
        Ok(())
    }
}
