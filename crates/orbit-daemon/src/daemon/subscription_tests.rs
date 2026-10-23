use super::*;
use anyhow::bail;
use orbit_core::{RecoveryStatus, topics::TopicId};
use std::time::Duration;

fn fixture() -> Result<(tempfile::TempDir, Paths)> {
    let directory = tempfile::tempdir()?;
    let paths = Paths::resolve(Some(directory.path().join("orbit")))?;
    paths.save_settings(&Settings {
        hub_url: Some("http://127.0.0.1:0".into()),
        ..Settings::default()
    })?;
    Ok((directory, paths))
}

async fn ready(paths: &Paths, daemon: &mut JoinHandle<Result<()>>) -> Result<()> {
    loop {
        if paths.socket.exists()
            && let Ok(response) = ipc::request(paths, Command::Status).await
            && response
                .status
                .is_some_and(|state| matches!(state.recovery, RecoveryStatus::Ready))
        {
            return Ok(());
        }
        tokio::select! {
            result = &mut *daemon => {
                result??;
                bail!("daemon stopped before it became ready")
            }
            () = tokio::time::sleep(Duration::from_millis(10)) => {}
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn shutdown_records_numeric_turn_counts_without_turn_content() -> Result<()> {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (_directory, paths) = fixture()?;
            let daemon_paths = paths.clone();
            let mut daemon = tokio::task::spawn_local(async move {
                run_with_helper_inner(daemon_paths, true, None, None).await
            });
            ready(&paths, &mut daemon).await?;
            ipc::request(&paths, Command::Shutdown).await?;
            daemon.await??;
            let log = std::fs::read_to_string(paths.root.join("logs/daemon.log"))?;
            let summary = log
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .find(|line| line["event"] == "loop_turns")
                .context("shutdown omitted loop branch counts")?;
            let fields = summary["fields"].as_object().context("counts missing")?;
            let names = [
                "signals",
                "commands",
                "native",
                "hub",
                "work",
                "notes",
                "topic_due",
                "tick",
                "capture_tick",
                "elapsed_ms",
            ];
            assert_eq!(fields.len(), names.len());
            for name in names {
                let count = fields
                    .get(name)
                    .and_then(Value::as_str)
                    .and_then(|value| value.parse::<u64>().ok());
                assert!(count.is_some(), "{name} must contain only a count");
            }
            assert!(
                fields["commands"]
                    .as_str()
                    .context("commands")?
                    .parse::<u64>()?
                    >= 2
            );
            assert_eq!(fields["signals"], "0");
            assert_eq!(fields["native"], "0");
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "current_thread")]
async fn command_receipt_revisions_are_already_published_by_the_actor() -> Result<()> {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (_directory, paths) = fixture()?;
            let daemon_paths = paths.clone();
            let mut daemon = tokio::task::spawn_local(async move {
                run_with_helper_inner(daemon_paths, true, None, None).await
            });
            ready(&paths, &mut daemon).await?;
            let (_initial, mut subscription) =
                ipc::subscribe_topics(&paths, vec![TopicId::Status, TopicId::Control]).await?;
            for _ in 0..2 {
                let _initial_frame = subscription.next_topic().await?;
            }
            let receipt = ipc::request(&paths, Command::PrivacyStop).await?;
            assert!(receipt.ok);
            let revisions = receipt
                .revisions
                .context("command reply omitted revisions")?;
            let mut seen = std::collections::BTreeSet::new();
            for _ in 0..2 {
                let frame = subscription.next_topic().await?;
                let id = frame.data.id();
                assert_eq!(Some(&frame.revision), revisions.get(&id));
                let published = serde_json::to_value(&frame.data)?;
                if id == TopicId::Status {
                    assert_eq!(published["knowledge"]["value"]["privacy_stopped"], true);
                } else if id == TopicId::Control {
                    assert_eq!(
                        published["knowledge"]["value"]["engine"]["refusal"]["code"],
                        "privacy_stopped"
                    );
                } else {
                    bail!("unexpected topic in command subscription: {id:?}");
                }
                seen.insert(id);
            }
            assert_eq!(
                seen,
                [TopicId::Status, TopicId::Control].into_iter().collect()
            );
            ipc::request(&paths, Command::Shutdown).await?;
            daemon.await??;
            Ok(())
        })
        .await
}
