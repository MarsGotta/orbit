//! The reminder through a synthetic daemon: what it read is kept with the
//! meeting it was read for, a failure as its cause, and a save goes as the
//! title's digest and is read again once taken.
use super::*;
use orbit_core::{
    PROTOCOL_VERSION,
    protocol::{Command, Response, TopicResponse},
    topics::{Knowledge, Page},
};
use std::collections::BTreeMap;
use tokio::sync::mpsc;

const SERIES: &str = "revisión semanal";

/// A daemon that answers the reminder read with `reminder` and records the
/// orders it is given.
fn daemon(
    reminder: serde_json::Value,
) -> (
    crate::client_fixture::Endpoint,
    mpsc::Receiver<Command>,
    tokio::task::JoinHandle<()>,
) {
    let (calls, mut actor) = mpsc::channel::<orbit_ipc::Call>(8);
    let endpoint = crate::client_fixture::channel(calls, orbit_ipc::TopicChannels::new());
    let (orders, received) = mpsc::channel(8);
    let task = tokio::spawn(async move {
        while let Some(call) = actor.recv().await {
            let body = call.request.body.clone();
            let sent = if let Command::Read { read } = &body {
                let key = orbit_core::series_title::title_digest(SERIES);
                let item = serde_json::json!({"key": key, "reminder": reminder});
                call.reply.send(TopicResponse {
                    v: PROTOCOL_VERSION,
                    id: call.request.id,
                    ok: true,
                    revisions: BTreeMap::new(),
                    read: Some(read.id()),
                    answer: Some(Knowledge::Present {
                        value: Page {
                            items: vec![item],
                            next: None,
                            total: 1,
                            revision: 0,
                            store_version: None,
                        },
                        as_of_ms: None,
                    }),
                    refusal: None,
                })
            } else {
                call.reply.send(Response {
                    v: 1,
                    id: call.request.id,
                    ok: true,
                    status: None,
                    error: None,
                    refusal: None,
                    epoch: Some("epoch-1".into()),
                    revisions: Some(BTreeMap::new()),
                })
            };
            if sent.is_err() || orders.send(body).await.is_err() {
                break;
            }
        }
    });
    (endpoint, received, task)
}

fn kept(client: &DaemonClient) -> Option<crate::ReminderRead> {
    client
        .local_snapshot()
        .notes
        .and_then(|notes| notes.reminder)
        .map(|read| *read)
}

#[tokio::test]
async fn a_saved_reminder_goes_by_its_digest_and_is_read_again()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let paths = orbit_ipc::Paths::resolve(Some(directory.path().to_path_buf()))?;
    let (endpoint, mut orders, task) =
        daemon(serde_json::json!({"text": "Reservar la sala", "edited_at_ms": 7}));
    let client = DaemonClient::connect(paths, endpoint.client.clone());
    client
        .send(AppCommand::Notes(crate::NotesCommand::ReminderSave {
            series_key: SERIES.into(),
            text: "Reservar la sala".into(),
            edited_at_ms: 7,
        }))
        .await?;
    let order = orders.recv().await.ok_or("no order")?;
    assert_eq!(
        order,
        Command::ReminderWrite {
            key: orbit_core::series_title::title_digest(SERIES).ok_or("no digest")?,
            text: "Reservar la sala".into(),
            edited_at_ms: 7,
        }
    );
    let read = kept(&client).ok_or("not read again")?;
    assert_eq!(read.series_key, SERIES);
    assert_eq!(
        read.answer,
        Ok(Some(orbit_core::reminders::Reminder {
            text: "Reservar la sala".into(),
            edited_at_ms: 7
        }))
    );
    endpoint.abort();
    task.abort();
    Ok(())
}
