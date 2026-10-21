//! Topic publication and unpaired reads at the real daemon socket.
#![allow(clippy::expect_used, clippy::unwrap_used)]
use orbit_core::{
    protocol::Command as OrbitCommand,
    topics::{Knowledge, TopicData, TopicId},
};
use orbit_ipc::{Paths, read_frame};
use serde_json::{Value, json};
use std::{
    error::Error,
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    net::UnixStream,
};

// Preserve the app-core fixtures' measured cold-start guard (3,969 ms worst).
// It bounds a hung process, not startup latency.
const START_BUDGET: Duration = Duration::from_secs(30);
// The worst recovery measured on a loaded disk was 36.4 s; 120 s catches a jam.
const RECOVERY_JAM_GUARD: Duration = Duration::from_secs(120);
const READ_BUDGET: Duration = START_BUDGET;

struct TestDaemon {
    child: Child,
    paths: Paths,
}
impl TestDaemon {
    async fn start(root: &Path, paths: &Paths) -> Result<Self, Box<dyn Error>> {
        let child = Command::new(env!("CARGO_BIN_EXE_orbit"))
            .args(["--dev", "--data-dir"])
            .arg(root)
            .arg("daemon")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let daemon = Self {
            child,
            paths: paths.clone(),
        };
        tokio::time::timeout(START_BUDGET, async {
            loop {
                if UnixStream::connect(&paths.socket).await.is_ok() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await?;
        Ok(daemon)
    }
    async fn shutdown(mut self, paths: &Paths) -> Result<(), Box<dyn Error>> {
        let response = orbit_ipc::request(paths, OrbitCommand::Shutdown).await?;
        assert!(response.ok, "shutdown refused: {:?}", response.refusal);
        tokio::time::timeout(START_BUDGET, async {
            loop {
                if let Some(status) = self.child.try_wait()? {
                    assert!(status.success());
                    break;
                }
                tokio::task::yield_now().await;
            }
            Ok::<_, std::io::Error>(())
        })
        .await??;
        Ok(())
    }
}
impl Drop for TestDaemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
async fn wait_until_recovered(daemon: &TestDaemon) -> Result<(), Box<dyn Error>> {
    tokio::time::timeout(RECOVERY_JAM_GUARD, async {
        loop {
            let response = orbit_ipc::request(&daemon.paths, OrbitCommand::Status).await?;
            if response.status.ok_or("startup status absent")?.recovery
                == orbit_core::RecoveryStatus::Ready
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        Ok::<_, Box<dyn Error>>(())
    })
    .await??;
    Ok(())
}

use orbit_core::reads::ReadRequest;
fn assert_initial_data(data: &TopicData, revision: u64) -> Result<(), Box<dyn Error>> {
    if revision == 0 {
        assert_eq!(*data, TopicData::not_observed(data.id()));
        return Ok(());
    }
    match data {
        TopicData::Recording(Knowledge::Present { value, .. }) => {
            assert!(value.recording.is_none());
            assert!(value.offline);
            assert!(matches!(
                value.applications,
                Knowledge::Absent {
                    refusal: orbit_core::refusal::Refusal::HelperNotRunning
                }
            ));
        }
        TopicData::Recording(other) => {
            panic!("recording must publish its initial facts: {other:?}")
        }
        TopicData::Status(Knowledge::Present { value, .. }) => {
            assert_eq!(serde_json::to_value(value)?["privacy_stopped"], false);
        }
        TopicData::Permissions(Knowledge::Present { value, .. }) => {
            assert_eq!(serde_json::to_value(value)?["microphone"], "unknown");
        }
        TopicData::Control(Knowledge::Present { value, .. }) => {
            assert!(matches!(&value.engine, Knowledge::Absent { .. }));
        }
        TopicData::Notes(Knowledge::Present { value, .. }) => {
            // The head of what changed; nothing recorded yet.
            assert!(value.recording.is_none());
            assert!(value.recordings.is_empty());
        }
        TopicData::Notes(other) => {
            panic!("notes must publish their observed head: {other:?}")
        }
        TopicData::Assistant(Knowledge::Present { value, .. }) => {
            assert!(value.session_id.is_none());
            assert!(value.turn.is_none());
            assert!(matches!(
                value.voice,
                orbit_core::topics::assistant::VoiceAvailability::Unknown { .. }
            ));
        }
        TopicData::Assistant(other) => {
            panic!("the observed assistant head must be present: {other:?}")
        }
        // Notices publish the current notice text from startup.
        TopicData::Notices(Knowledge::Present { .. }) => {}
        TopicData::Notices(other) => {
            panic!("notices publish their text from startup: {other:?}")
        }
        other => assert_eq!(*other, TopicData::not_observed(other.id())),
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn real_daemon_accepts_a_topic_subscription_and_sends_all_heads() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let daemon = TestDaemon::start(directory.path(), &paths).await?;
    wait_until_recovered(&daemon).await?;
    let stream = UnixStream::connect(&paths.socket).await?;
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let names = [
        "status",
        "permissions",
        "recording",
        "agenda",
        "notes",
        "assistant",
        "control",
        "notices",
    ];
    let mut request = serde_json::to_vec(&json!({
        "v": 1, "id": "topics-wire", "body": {"command": "subscribe_topics", "topics": names}
    }))?;
    request.push(b'\n');
    writer.write_all(&request).await?;
    let handshake: Value = serde_json::from_slice(
        &tokio::time::timeout(START_BUDGET, read_frame(&mut reader)).await??,
    )?;
    assert_eq!(handshake["ok"], true);
    assert_eq!(
        handshake["revisions"].as_object().map(|items| items.len()),
        Some(names.len())
    );
    let mut seen = std::collections::BTreeSet::new();
    let mut epoch = None;
    for _ in names {
        let event: Value = serde_json::from_slice(
            &tokio::time::timeout(START_BUDGET, read_frame(&mut reader)).await??,
        )?;
        assert_eq!(event["event"], "topic.changed");
        let frame = &event["frame"];
        let name = frame["data"]["topic"]
            .as_str()
            .ok_or("missing topic name")?;
        assert!(seen.insert(name.to_owned()));
        assert!(frame["revision"].as_u64().is_some());
        let observed = frame["epoch"].as_str().ok_or("missing epoch")?;
        assert!(!observed.is_empty());
        if let Some(previous) = epoch {
            assert_eq!(observed, previous);
        }
        epoch = Some(observed.to_owned());
    }
    assert_eq!(seen, names.into_iter().map(str::to_owned).collect());
    daemon.shutdown(&paths).await?;
    Ok(())
}
#[tokio::test(flavor = "multi_thread")]
async fn real_daemon_sends_initial_topics_and_keeps_equal_values_at_same_revision()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let daemon = TestDaemon::start(directory.path(), &paths).await?;
    wait_until_recovered(&daemon).await?;
    let (_reply, mut subscription) =
        orbit_ipc::subscribe_topics(&paths, TopicId::ALL.to_vec()).await?;
    let mut initial = std::collections::BTreeMap::new();
    for _ in TopicId::ALL {
        let frame = tokio::time::timeout(START_BUDGET, subscription.next_topic()).await??;
        assert_initial_data(&frame.data, frame.revision)?;
        assert!(initial.insert(frame.id(), frame).is_none());
    }
    assert_eq!(initial.len(), TopicId::ALL.len());
    let epoch = initial[&TopicId::Status].epoch.clone();
    assert!(initial.values().all(|frame| frame.epoch == epoch));
    while initial.values().any(|frame| frame.revision == 0) {
        let frame = tokio::time::timeout(START_BUDGET, subscription.next_topic()).await??;
        assert_eq!(frame.epoch, epoch);
        assert_initial_data(&frame.data, frame.revision)?;
        let previous = initial.get_mut(&frame.id()).ok_or("unexpected topic")?;
        assert!(frame.revision > previous.revision);
        *previous = frame;
    }

    let assistant = &initial[&TopicId::Assistant];
    assert!(assistant.revision > 0);
    assert_initial_data(&assistant.data, assistant.revision)?;

    let read = orbit_ipc::read_topic(
        &paths,
        orbit_core::reads::ReadRequest::NotesPage { cursor: None },
    )
    .await?;
    assert_eq!(read.read, Some(orbit_core::reads::ReadId::NotesPage));
    // Unpaired, the list says why the hub's part is missing and
    // lists no meeting of its own.
    assert!(matches!(
        read.answer,
        Some(Knowledge::Present { value, .. }) if value.items.len() == 1
            && value.items[0]["hub_refusal"]["code"] == "not_paired"
    ));

    let (_, mut control_stream) =
        orbit_ipc::subscribe_topics(&paths, vec![TopicId::Control]).await?;
    let control_json = tokio::time::timeout(START_BUDGET, async {
        loop {
            let frame = control_stream.next_topic().await?;
            if frame.revision > 0 {
                return Ok::<_, Box<dyn Error>>(serde_json::to_value(frame.data)?);
            }
        }
    })
    .await??;
    assert_eq!(
        control_json["knowledge"]["state"], "present",
        "{control_json}"
    );
    let engine = &control_json["knowledge"]["value"]["engine"];
    assert_eq!(engine["state"], "absent", "{engine}");
    let refusal = &engine["refusal"];
    assert!(
        refusal["code"] == "helper_not_running"
            || (refusal["code"] == "permission_unknown"
                && refusal["permission"] == "screen_and_accessibility"),
        "unexpected engine cause {refusal}"
    );

    let response = orbit_ipc::request(&paths, OrbitCommand::PrivacyStop).await?;
    assert!(response.ok, "privacy stop refused: {:?}", response.refusal);
    assert!(response.status.is_some_and(|status| status.privacy_stopped));
    // PrivacyStop changes the status fact, the recording fact, the control
    // engine and agenda's access decision; the notes index is not touched.
    // Wait for all four before comparing read revisions: the command reply
    // and topic publication use separate streams.
    let mut changed_status = None;
    let mut changed_recording = None;
    let mut changed_agenda = None;
    let mut changed_control = None;
    tokio::time::timeout(START_BUDGET, async {
        while changed_status.is_none()
            || changed_recording.is_none()
            || changed_agenda.is_none()
            || changed_control.is_none()
        {
            let frame = subscription.next_topic().await?;
            match &frame.data {
                TopicData::Status(Knowledge::Present { value, .. })
                    if serde_json::to_value(value)
                        .ok()
                        .is_some_and(|data| data["privacy_stopped"] == true) =>
                {
                    changed_status = Some(frame.revision);
                }
                TopicData::Recording(_) if frame.revision > read.revisions[&TopicId::Recording] => {
                    changed_recording = Some(frame.revision);
                }
                TopicData::Control(_) if frame.revision > read.revisions[&TopicId::Control] => {
                    changed_control = Some(frame.revision);
                }
                TopicData::Agenda(Knowledge::Absent {
                    refusal: orbit_core::refusal::Refusal::PrivacyStopped { .. },
                }) => changed_agenda = Some(frame.revision),
                _ => {}
            }
        }
        Ok::<_, Box<dyn Error>>(())
    })
    .await
    .map_err(|_| {
        std::io::Error::other(
            "status, recording, control and agenda topics never published the privacy stop",
        )
    })??;
    let status_revision = changed_status.ok_or("status topic never changed")?;
    assert!(status_revision > initial[&TopicId::Status].revision);
    let recording_revision = changed_recording.ok_or("recording topic never changed")?;
    let agenda_revision = changed_agenda.ok_or("agenda topic never changed")?;
    let control_revision = changed_control.ok_or("control topic never changed")?;
    assert_eq!(agenda_revision, read.revisions[&TopicId::Agenda] + 1);
    let after = orbit_ipc::read_topic(
        &paths,
        orbit_core::reads::ReadRequest::NotesPage { cursor: None },
    )
    .await?;
    assert!(after.revisions[&TopicId::Status] > read.revisions[&TopicId::Status]);
    assert!(after.revisions[&TopicId::Recording] >= recording_revision);
    assert_eq!(after.revisions[&TopicId::Agenda], agenda_revision);
    assert!(after.revisions[&TopicId::Control] >= control_revision);
    // The privacy stop writes a notice, so the notices topic
    // publishes new text and a new revision in the same turn.
    assert!(after.revisions[&TopicId::Notices] > read.revisions[&TopicId::Notices]);
    let changed = [
        TopicId::Status,
        TopicId::Recording,
        TopicId::Agenda,
        TopicId::Control,
        TopicId::Notices,
    ];
    for (id, revision) in &read.revisions {
        if !changed.contains(id) {
            assert_eq!(
                after.revisions[id], *revision,
                "{id:?} changed without data"
            );
        }
    }
    drop(subscription);
    daemon.shutdown(&paths).await?;

    let restarted = TestDaemon::start(directory.path(), &paths).await?;
    wait_until_recovered(&restarted).await?;
    let (_reply, mut subscription) =
        orbit_ipc::subscribe_topics(&paths, TopicId::ALL.to_vec()).await?;
    let first = tokio::time::timeout(START_BUDGET, subscription.next_topic()).await??;
    assert_ne!(first.epoch, epoch);
    drop(subscription);
    restarted.shutdown(&paths).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn unpaired_sessions_read_names_the_missing_pairing() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let daemon = TestDaemon::start(directory.path(), &paths).await?;
    wait_until_recovered(&daemon).await?;
    let response = tokio::time::timeout(
        READ_BUDGET,
        orbit_ipc::read_topic(&paths, ReadRequest::Sessions { cursor: None }),
    )
    .await??;
    let answer = response.answer.expect("the read must carry an answer");
    match answer {
        Knowledge::Absent { refusal } => {
            let value = serde_json::to_value(&refusal).unwrap();
            assert_eq!(
                value["code"], "not_paired",
                "an unpaired daemon names the missing pairing before blaming the hub"
            );
        }
        Knowledge::Unknown { .. } => {
            panic!(
                "the sessions read stays NotObservedYet: the daemon owns it and must either \
                 page it or refuse it with a cause"
            );
        }
        Knowledge::Present { .. } => {
            panic!("an offline daemon with no hub listing must not invent conversations")
        }
    }
    daemon.shutdown(&paths).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn session_messages_read_names_why_it_cannot_read() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let daemon = TestDaemon::start(directory.path(), &paths).await?;
    wait_until_recovered(&daemon).await?;
    let response = tokio::time::timeout(
        READ_BUDGET,
        orbit_ipc::read_topic(
            &paths,
            ReadRequest::SessionMessages {
                session_id: "30000000-0000-4000-8000-000000000003".into(),
                cursor: None,
            },
        ),
    )
    .await??;
    let answer = response.answer.expect("the read must carry an answer");
    // The conversation is the hub's; an unpaired daemon says so.
    match answer {
        Knowledge::Absent { refusal } => {
            assert_eq!(serde_json::to_value(&refusal)?["code"], "not_paired");
        }
        Knowledge::Unknown { .. } => {
            panic!("the messages read stays NotObservedYet: the daemon owns it")
        }
        Knowledge::Present { .. } => {
            panic!("an unpaired daemon must not invent a conversation")
        }
    }
    daemon.shutdown(&paths).await?;
    Ok(())
}
