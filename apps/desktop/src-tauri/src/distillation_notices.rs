//! Ready proposals use the same native channel as recording and chat notices.
use orbit_app_core::{DaemonClient, NotesSnapshot};
use orbit_core::vault::{
    Distillation, DistillationAction, DistillationStatus, VaultCall, VaultOutcome,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, PoisonError},
};
use tauri::Manager;
use tokio::sync::Notify;

pub struct Watcher {
    // One hub page of ids/statuses only; proposal content is never retained.
    previous: Option<BTreeMap<String, DistillationStatus>>,
    requested: bool,
    preparing: bool,
    note_sequence: u32,
}
impl Default for Watcher {
    fn default() -> Self {
        Self::new(0)
    }
}
impl Watcher {
    fn new(note_sequence: u32) -> Self {
        Self {
            previous: None,
            requested: true,
            preparing: false,
            note_sequence,
        }
    }
    pub fn should_poll(&self, privacy_stopped: bool) -> bool {
        !privacy_stopped && (self.requested || self.preparing)
    }
    fn take_poll(&mut self, privacy_stopped: bool) -> bool {
        if privacy_stopped {
            self.requested = false;
            self.preparing = false;
            return false;
        }
        let poll = self.should_poll(false);
        self.requested = false;
        poll
    }
    fn note_ready(&mut self, sequence: u32) -> bool {
        if sequence <= self.note_sequence {
            return false;
        }
        self.note_sequence = sequence;
        self.requested = true;
        true
    }
    fn preparation_requested(&mut self, call: &VaultCall) -> bool {
        let prepares = prepares(call);
        self.requested |= prepares;
        prepares
    }
    pub fn observe(&mut self, entries: &[Distillation]) -> usize {
        let ready = self.previous.as_ref().map_or(0, |previous| {
            entries
                .iter()
                .filter(|entry| {
                    entry.status == DistillationStatus::Proposed
                        && previous.get(&entry.id) != Some(&DistillationStatus::Proposed)
                })
                .count()
        });
        self.preparing = entries.iter().any(|entry| {
            matches!(
                entry.status,
                DistillationStatus::Queued | DistillationStatus::Preparing
            )
        });
        self.previous = Some(
            entries
                .iter()
                .map(|entry| (entry.id.clone(), entry.status.clone()))
                .collect(),
        );
        ready
    }
}

pub fn prepares(call: &VaultCall) -> bool {
    matches!(
        call,
        VaultCall::PrepareDistillation { .. }
            | VaultCall::DistillationAction {
                action: DistillationAction::Prepare,
                ..
            }
    )
}

#[derive(Clone)]
struct Handle(Arc<Inner>);
struct Inner {
    watcher: Mutex<Watcher>,
    wake: Notify,
}
impl Handle {
    fn new(note_sequence: u32) -> Self {
        Self(Arc::new(Inner {
            watcher: Mutex::new(Watcher::new(note_sequence)),
            wake: Notify::new(),
        }))
    }
    fn change<T>(&self, change: impl FnOnce(&mut Watcher) -> T) -> T {
        change(
            &mut self
                .0
                .watcher
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }
    fn trigger(&self, reason: impl FnOnce(&mut Watcher) -> bool) {
        if self.change(reason) {
            self.0.wake.notify_one();
        }
    }
}

/// This is the existing background Notes frame used by «Tu nota está lista».
/// Its sequence changes only after app-core saw a note being written finish.
pub fn follow_notes<R: tauri::Runtime>(app: &tauri::AppHandle<R>, notes: &Option<NotesSnapshot>) {
    if let Some(ready) = notes.as_ref().and_then(|notes| notes.ready.as_ref())
        && let Some(handle) = app.try_state::<Handle>()
    {
        handle.trigger(|watcher| watcher.note_ready(ready.sequence));
    }
}

/// Called after the request settles, including a lost receipt: a read can
/// reconcile it, but the watcher never repeats the preparation command.
pub fn preparation_requested<R: tauri::Runtime>(app: &tauri::AppHandle<R>, call: &VaultCall) {
    if let Some(handle) = app.try_state::<Handle>() {
        handle.trigger(|watcher| watcher.preparation_requested(call));
    }
}

pub fn watch<R: tauri::Runtime>(app: tauri::AppHandle<R>, client: DaemonClient) {
    let notes = client.composed_snapshot().notes;
    let sequence = notes
        .and_then(|notes| notes.ready)
        .map_or(0, |ready| ready.sequence);
    let handle = Handle::new(sequence);
    app.manage(handle.clone());
    tauri::async_runtime::spawn(async move {
        loop {
            let privacy_stopped = client.composed_snapshot().settings.privacy_stopped;
            if handle.change(|watcher| watcher.take_poll(privacy_stopped)) {
                read(&app, &client, &handle).await;
            }
            if handle.change(|watcher| watcher.requested) {
                continue;
            }
            if handle.change(|watcher| watcher.preparing) {
                // Only an observed queued/preparing proposal owns a timer.
                // Ten seconds bounds readiness latency, not proposal lifetime.
                tokio::select! {
                    _ = handle.0.wake.notified() => {},
                    _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => {},
                }
            } else {
                // No timer or hub work between the three explicit reasons.
                handle.0.wake.notified().await;
            }
        }
    });
}

async fn read<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    client: &DaemonClient,
    handle: &Handle,
) {
    let count = match client
        .vault(VaultCall::ListDistillations {
            meeting_id: None,
            page: 1,
        })
        .await
    {
        Ok(VaultOutcome::Distillations { page }) => {
            handle.change(|watcher| watcher.observe(&page.entries))
        }
        Ok(VaultOutcome::Refused { .. } | VaultOutcome::DistillationDisabled { .. }) => {
            handle.change(|watcher| watcher.preparing = false);
            0
        }
        // A transport failure neither invents queued work at startup nor
        // erases an already observed preparation. The next reason/tick reads it.
        _ => 0,
    };
    if count > 0 {
        let copy = crate::copy::for_language(crate::language::current(app));
        crate::notify::post(
            app,
            crate::notify::TurnNotice {
                title: copy.distillation_title.into(),
                body: copy.distillation_body.into(),
            },
            true,
        );
    }
}

#[cfg(test)]
#[path = "distillation_notices/tests.rs"]
mod tests;
