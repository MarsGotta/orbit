//! «Recordatorio personal» at the daemon: read from the hub and saved to it
//! off the loop, against a synthetic hub on loopback (`fake_hub`). An older
//! hub is unknown, never «no reminder».
use super::super::fake_hub::{self, Hub, lock};
use crate::daemon::{Runtime, audio::tests::fixture};
use anyhow::{Context, Result, bail};
use orbit_core::{
    protocol::Command,
    reads::ReadRequest,
    refusal::Refusal,
    reminders::{Reminder, ReminderAnswer},
    topics::Knowledge,
};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

const KEY: &str = "ee009505531381ac0ce81bbac953dcf1f849ab8e3b8f94d1292854901fc22cff";

async fn paired(runtime: &mut Runtime) -> Result<fake_hub::Shared> {
    let hub: fake_hub::Shared = Arc::new(Mutex::new(Hub::default()));
    runtime.settings.hub_url = Some(fake_hub::start(hub.clone()).await?);
    runtime.settings.device_id = Some("11111111-1111-4111-8111-111111111111".into());
    runtime.token = Some("synthetic-not-a-credential".into());
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "connection",
        serde_json::json!("online"),
    );
    Ok(hub)
}

async fn read(runtime: &Runtime) -> Result<Result<ReminderAnswer, Refusal>> {
    let request = ReadRequest::PersonalReminder {
        key: KEY.into(),
        cursor: None,
    };
    let future = runtime.notes_read(&request).context("not a notes read")?;
    Ok(match future.await {
        Knowledge::Present { value, .. } => Ok(serde_json::from_value(
            value.items.first().cloned().context("no item")?,
        )?),
        Knowledge::Absent { refusal } => Err(refusal),
        Knowledge::Unknown { why } => bail!("unknown: {why:?}"),
    })
}

/// The save as the app sends it, answered on the app's real reply channel.
async fn save(runtime: &mut Runtime, text: &str, edited_at_ms: i64) -> Result<Option<Refusal>> {
    let (calls, mut received) = mpsc::channel(4);
    let (_ended, ended) = tokio::sync::watch::channel(None);
    let client = orbit_ipc::LocalClient::new(calls, &std::collections::BTreeMap::new(), ended);
    let command = Command::ReminderWrite {
        key: KEY.into(),
        text: text.into(),
        edited_at_ms,
    };
    let sent = command.clone();
    let asked = tokio::spawn(async move { client.request_command(sent).await });
    let call = received.recv().await.context("no order")?;
    runtime.serve_reminder_write(
        (KEY.into(), text.into(), edited_at_ms),
        call.request.id,
        call.reply,
        (None, std::collections::BTreeMap::new()),
    );
    Ok(match asked.await? {
        Ok(_) => None,
        Err(error) => Some(
            error
                .downcast_ref::<Refusal>()
                .cloned()
                .context("a refusal without its cause")?,
        ),
    })
}

#[tokio::test]
async fn a_saved_reminder_is_read_back_and_the_latest_edit_wins() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let hub = paired(&mut runtime).await?;
    assert_eq!(read(&runtime).await??.reminder, None, "none is a fact");
    assert_eq!(save(&mut runtime, "Reservar la sala", 2_000).await?, None);
    assert_eq!(save(&mut runtime, "Escrito antes", 1_000).await?, None);
    assert_eq!(
        read(&runtime).await??.reminder,
        Some(Reminder {
            text: "Reservar la sala".into(),
            edited_at_ms: 2_000
        })
    );
    let put = lock(&hub)
        .received
        .iter()
        .find(|request| request.method == "PUT")
        .map(|request| (request.path.clone(), request.body.clone()))
        .context("no save")?;
    assert_eq!(put.0, format!("/v1/reminders/{KEY}"));
    assert_eq!(put.1["text"], "Reservar la sala");
    Ok(())
}

#[tokio::test]
async fn an_older_hub_is_unknown_and_its_refusal_reaches_the_app() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let hub = paired(&mut runtime).await?;
    lock(&hub).without_reminder_routes = true;
    let older = Refusal::UpstreamFailed {
        service: orbit_core::refusal::Service::Hub,
        status: Some(404),
    };
    assert_eq!(read(&runtime).await?, Err(older.clone()));
    assert_eq!(
        save(&mut runtime, "Reservar la sala", 1).await?,
        Some(older)
    );
    Ok(())
}

#[tokio::test]
async fn without_a_pairing_the_save_says_so() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let refusal = save(&mut runtime, "Reservar la sala", 1)
        .await?
        .context("an unpaired Mac saved")?;
    assert!(
        !matches!(refusal, Refusal::UpstreamFailed { .. }),
        "{refusal:?}"
    );
    Ok(())
}
