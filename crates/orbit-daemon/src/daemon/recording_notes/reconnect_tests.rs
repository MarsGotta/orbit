//! What the Mac reads again when the hub comes back. While the link was down
//! any meeting may have changed and this Mac heard of none of it, so every
//! `ready` counts them all as changed: the list is read again, and so is the
//! note open on screen, whether the head names its meeting or not.
//!
//! The app reads the list again when the head's `revision` moves, and the
//! open note when `changed_at` of its meeting passes what it read; these
//! tests read that head, as the `ready` leaves it. A synthetic hub on
//! loopback answers; nothing here is evidence about the real hub.
use super::{
    NoteEvent, NotesHandle,
    fake_hub::{self, Hub, lock},
};
use crate::daemon::{HubEvent, Runtime, audio::tests::fixture};
use anyhow::{Context, Result};
use orbit_core::topics::notes::Value as Head;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tempfile::TempDir;
use tokio::sync::mpsc;

const DEVICE: &str = "4a6c8e0b-2d4f-4a1c-9e3b-5d7f9a1c3e5b";
const HUB: &str = "c1e3a5b7-9d1f-4b3d-8f5a-7c9e1b3d5f7a";
/// The meeting whose note is open on screen: named in the head, because it
/// changed while the hub was there.
const OPEN: &str = "1b3d5f7a-9c1e-4a3c-8e5f-7a9c1e3b5d7f";
/// Another meeting that changed while the hub was there.
const SEEN: &str = "2c4e6a8b-0d2f-4b4d-9f6a-8b0d2f4c6e8a";
/// A meeting the head has never named: only `floor` speaks for it.
const NEVER_NAMED: &str = "3d5f7b9c-1e3a-4c5e-8a7b-9c1e3a5d7f9b";
const NOW: i64 = 1_800_000_000_000;
/// Loopback answers in milliseconds; this only ends a run that would hang.
const LIMIT: Duration = Duration::from_secs(20);
/// What is still on its way arrives in this, so a request that should not
/// have gone shows in the hub's log. It can only miss a wrong request, never
/// fail a right run.
const QUIET: Duration = Duration::from_millis(300);

async fn bounded(test: impl Future<Output = Result<()>>) -> Result<()> {
    tokio::time::timeout(LIMIT, test)
        .await
        .context("the run did not finish within its limit")?
}

struct Mac {
    runtime: Runtime,
    events: mpsc::Receiver<NoteEvent>,
    hub: fake_hub::Shared,
    directory: TempDir,
}

impl Mac {
    /// A daemon paired with the synthetic hub, its note answers handed to the
    /// test.
    async fn paired() -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _) = fixture(directory.path())?;
        runtime.notes = NotesHandle::for_tests(runtime.notes.events.clone());
        let events = runtime.notes.test_events();
        runtime.notes.pinned_now = Some(NOW);
        let hub: fake_hub::Shared = Arc::new(Mutex::new(Hub::default()));
        runtime.settings.hub_url = Some(fake_hub::start(hub.clone()).await?);
        runtime.settings.device_id = Some(DEVICE.into());
        runtime.link.token = Some("synthetic-not-a-credential".into());
        Ok(Self {
            runtime,
            events,
            hub,
            directory,
        })
    }

    /// A Mac that was connected, heard `OPEN` and `SEEN` change, and lost the
    /// link: the head names both.
    async fn was_away() -> Result<Self> {
        let mut mac = Self::paired().await?;
        mac.hears_changes_and_loses_the_hub().await?;
        Ok(mac)
    }

    async fn hears_changes_and_loses_the_hub(&mut self) -> Result<()> {
        self.hub_returns().await?;
        self.settle().await?;
        self.changed(OPEN).await?;
        self.changed(SEEN).await?;
        let head = self.head();
        anyhow::ensure!(
            head.recordings.contains_key(OPEN) && head.recordings.contains_key(SEEN),
            "the notices did not name their meetings: {head:?}"
        );
        self.runtime.hub_event(HubEvent::Disconnected).await
    }

    fn head(&self) -> &Head {
        &self.runtime.notes.head
    }

    async fn frame(&mut self, frame: Value) -> Result<()> {
        self.runtime.hub_event(HubEvent::Frame(frame)).await
    }

    /// The link comes up and the hub says `ready`, with its id and its
    /// change number: all it would take for a Mac to ask what changed.
    async fn hub_returns(&mut self) -> Result<()> {
        self.runtime.hub_event(HubEvent::Connected).await?;
        self.frame(json!({"v": 1, "event": "ready", "data": {
            "device_id": DEVICE, "hub_protocol": 1, "hub_id": HUB, "change": 140}}))
            .await
    }

    async fn changed(&mut self, recording: &str) -> Result<()> {
        self.frame(json!({"v": 1, "event": "recording.changed", "data": {
            "recording_id": recording, "change": 141}}))
            .await
    }

    /// Runs the daemon's side as its loop would (what is due goes out, each
    /// answer comes back as a note event) until nothing more arrives.
    async fn settle(&mut self) -> Result<()> {
        loop {
            self.runtime.maintain_recording_notes_at(NOW);
            match tokio::time::timeout(QUIET, self.events.recv()).await {
                Ok(Some(event)) => self.runtime.note_answer_at(event, NOW).await?,
                Ok(None) | Err(_) => return Ok(()),
            }
        }
    }
}

#[tokio::test]
async fn a_note_the_head_names_is_read_again_when_the_hub_returns() -> Result<()> {
    bounded(async {
        let mut mac = Mac::was_away().await?;
        let read = mac.head().changed_at(OPEN);
        mac.hub_returns().await?;
        let now = mac.head().changed_at(OPEN);
        assert!(now > read, "read at {read}, and now it says {now}");
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_note_the_head_never_named_is_read_again_when_the_hub_returns() -> Result<()> {
    bounded(async {
        let mut mac = Mac::was_away().await?;
        let read = mac.head().changed_at(NEVER_NAMED);
        mac.hub_returns().await?;
        let now = mac.head().changed_at(NEVER_NAMED);
        assert!(now > read, "read at {read}, and now it says {now}");
        Ok(())
    })
    .await
}

/// Each return costs one read of the open note: the second finds it read at
/// what the first left.
#[tokio::test]
async fn every_return_of_the_hub_reads_the_open_note_again() -> Result<()> {
    bounded(async {
        let mut mac = Mac::was_away().await?;
        mac.hub_returns().await?;
        let read = mac.head().changed_at(OPEN);
        mac.runtime.hub_event(HubEvent::Disconnected).await?;
        mac.hub_returns().await?;
        let now = mac.head().changed_at(OPEN);
        assert!(now > read, "read at {read}, and now it says {now}");
        Ok(())
    })
    .await
}

#[tokio::test]
async fn the_list_is_still_read_again_when_the_hub_returns() -> Result<()> {
    bounded(async {
        let mut mac = Mac::was_away().await?;
        let read = mac.head().revision;
        mac.hub_returns().await?;
        let now = mac.head().revision;
        assert!(now > read, "the list moved from {read} to {now}");
        Ok(())
    })
    .await
}

/// The owner's name is the one request a `ready` makes today. The hub sent
/// its id and number twice, so a Mac that asked what changed would have
/// asked here.
#[tokio::test]
async fn the_hub_is_asked_nothing_for_this_when_it_returns() -> Result<()> {
    bounded(async {
        let mut mac = Mac::was_away().await?;
        lock(&mac.hub).received.clear();
        mac.hub_returns().await?;
        assert!(
            mac.runtime.note_hub().is_some(),
            "this Mac cannot reach the hub, so its silence proves nothing"
        );
        mac.settle().await?;
        let asked = lock(&mac.hub)
            .received
            .iter()
            .filter(|request| !(request.method == "PUT" && request.path == "/v1/device/profile"))
            .map(|request| format!("{} {}?{}", request.method, request.path, request.query))
            .collect::<Vec<_>>();
        assert!(asked.is_empty(), "the hub was asked: {asked:?}");
        Ok(())
    })
    .await
}

/// Every file under the daemon's folder, with its bytes outside the store.
/// The store's own files are named but not compared: today's daemon already
/// writes to it on `ready`, and SQLite's `-shm` index comes and goes with
/// readers.
fn files(root: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut found = BTreeMap::new();
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else if !path.to_string_lossy().ends_with("-shm") {
                let name = path.strip_prefix(root)?;
                let bytes = if name.starts_with("store") {
                    Vec::new()
                } else {
                    std::fs::read(&path)?
                };
                found.insert(name.display().to_string(), bytes);
            }
        }
    }
    Ok(found)
}

#[tokio::test]
async fn nothing_reaches_the_daemons_folder_when_the_hub_returns() -> Result<()> {
    bounded(async {
        let mut mac = Mac::paired().await?;
        let written = files(mac.directory.path())?;
        mac.hears_changes_and_loses_the_hub().await?;
        mac.hub_returns().await?;
        mac.settle().await?;
        let now = files(mac.directory.path())?;
        assert_eq!(
            written.keys().collect::<Vec<_>>(),
            now.keys().collect::<Vec<_>>(),
            "a file appeared or left"
        );
        for (name, bytes) in &written {
            assert!(now.get(name) == Some(bytes), "{name} was written");
        }
        Ok(())
    })
    .await
}

/// Only `ready` counts them all: once back, a notice marks its own meeting
/// and leaves the others as they were read.
#[tokio::test]
async fn a_notice_after_the_return_reads_only_its_own_note() -> Result<()> {
    bounded(async {
        let mut mac = Mac::was_away().await?;
        mac.hub_returns().await?;
        let read = mac.head().clone();
        mac.changed(SEEN).await?;
        let now = mac.head();
        assert!(
            now.changed_at(SEEN) > read.changed_at(SEEN),
            "the notice's own note is not read again"
        );
        for other in [OPEN, NEVER_NAMED] {
            assert_eq!(
                now.changed_at(other),
                read.changed_at(other),
                "a notice about another meeting moved {other}"
            );
        }
        Ok(())
    })
    .await
}
