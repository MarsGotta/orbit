//! The hub's `chat.status` warnings reach the open conversation's chat with
//! their text; every other status stays as it was.
#![allow(clippy::expect_used)]
use super::*;
use crate::daemon::audio::tests::fixture;
use crate::hub::HubEvent;
use orbit_core::notice::NoticeCode;
use orbit_core::protocol::Command;
use orbit_core::topics::TopicId;

const OPEN: &str = "30000000-0000-4000-8000-000000000391";
const OTHER: &str = "30000000-0000-4000-8000-000000000392";
const WITHDRAWN: &str =
    "Hermes withdrew its approval question (timeout); it can no longer be answered";

/// The warnings the chat reads from the assistant topic, oldest first.
fn warnings(runtime: &Runtime) -> Vec<String> {
    let data = serde_json::to_value(super::topic(runtime, TopicId::Assistant))
        .expect("the assistant topic encodes");
    match &data["knowledge"]["value"]["warnings"] {
        serde_json::Value::Null => Vec::new(),
        value => serde_json::from_value(value.clone()).expect("the warnings are texts"),
    }
}

fn assistant_value(runtime: &Runtime) -> serde_json::Value {
    let data = serde_json::to_value(super::topic(runtime, TopicId::Assistant))
        .expect("the assistant topic encodes");
    data["knowledge"]["value"].clone()
}

type Sent = tokio::sync::mpsc::Receiver<serde_json::Value>;

/// A daemon with `OPEN` open and a turn of it in flight: the turn, and
/// what the daemon sends the hub (kept open, not read).
async fn open_with_turn(runtime: &mut Runtime) -> Result<(String, Sent)> {
    let (sender, frames) = tokio::sync::mpsc::channel(256);
    runtime.link.outbound.install(sender);
    runtime.settings.device_id = Some("test-device".into());
    runtime
        .hub_event(HubEvent::Frame(
            json!({"event":"ready","data":{"device_id":"test-device","chat_observe":true}}),
        ))
        .await?;
    open(runtime, OPEN).await?;
    Ok((ask(runtime).await?, frames))
}

async fn open(runtime: &mut Runtime, session: &str) -> Result<()> {
    runtime
        .command(Command::SessionOpen {
            id: session.into(),
            agent_id: None,
        })
        .await
}

async fn ask(runtime: &mut Runtime) -> Result<String> {
    runtime
        .command(Command::Chat {
            text: "Question".into(),
            attachments: Vec::new(),
            primary: false,
        })
        .await?;
    runtime
        .state
        .durable
        .assistant
        .active_request
        .clone()
        .context("no submitted turn")
}

async fn status(
    runtime: &mut Runtime,
    turn: Option<&str>,
    session: &str,
    kind: &str,
    text: &str,
) -> Result<()> {
    let data = json!({"session_id":session,"kind":kind,"text":text});
    let frame = match turn {
        Some(turn) => json!({"event":"chat.status","id":turn,"data":data}),
        None => json!({"event":"chat.status","data":data}),
    };
    runtime.hub_event(HubEvent::Frame(frame)).await
}

#[tokio::test]
async fn a_warning_of_the_open_conversation_reaches_the_chat_with_its_text() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let (turn, _sent) = open_with_turn(&mut runtime).await?;
    assert!(warnings(&runtime).is_empty());
    status(&mut runtime, Some(&turn), OPEN, "warning", WITHDRAWN).await?;
    assert_eq!(warnings(&runtime), [WITHDRAWN]);
    // One without a turn (the hub names none when it has none) is the
    // conversation's all the same; the text is the hub's, as it came.
    status(
        &mut runtime,
        None,
        OPEN,
        "warning",
        "Hermes sent a payload that is not a JSON object; ignored",
    )
    .await?;
    assert_eq!(
        warnings(&runtime),
        [
            WITHDRAWN,
            "Hermes sent a payload that is not a JSON object; ignored"
        ]
    );
    // The turn goes on: a warning is not a wait nor the turn's end.
    assert_eq!(assistant_value(&runtime)["turn_in_flight"], true);
    Ok(())
}

#[tokio::test]
async fn a_warning_of_another_conversation_is_not_shown_in_the_open_one() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let (turn, _sent) = open_with_turn(&mut runtime).await?;
    status(
        &mut runtime,
        None,
        OTHER,
        "warning",
        "Not this conversation",
    )
    .await?;
    status(
        &mut runtime,
        Some(&turn),
        OTHER,
        "warning",
        "Not this conversation either",
    )
    .await?;
    assert!(warnings(&runtime).is_empty(), "{:?}", warnings(&runtime));
    status(&mut runtime, Some(&turn), OPEN, "warning", WITHDRAWN).await?;
    assert_eq!(warnings(&runtime), [WITHDRAWN]);
    Ok(())
}

#[tokio::test]
async fn every_other_status_stays_as_it_was() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let (turn, _sent) = open_with_turn(&mut runtime).await?;
    for kind in ["status", "thinking", "lifecycle", "retrying_openai"] {
        runtime.state.set_notice(NoticeCode::AudioGap);
        status(&mut runtime, Some(&turn), OPEN, kind, "procesando").await?;
        // Recorded, never its text: as before.
        assert_eq!(
            runtime.state.notice_code(),
            Some(NoticeCode::AssistantProgress),
            "{kind}"
        );
        assert!(
            warnings(&runtime).is_empty(),
            "{kind}: {:?}",
            warnings(&runtime)
        );
    }
    assert_eq!(assistant_value(&runtime)["activity"], "retrying_openai");
    // An empty warning says nothing, as an empty status never did.
    status(&mut runtime, Some(&turn), OPEN, "warning", "  ").await?;
    assert!(warnings(&runtime).is_empty());
    Ok(())
}

#[tokio::test]
async fn the_warnings_keep_their_ceilings() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let (turn, _sent) = open_with_turn(&mut runtime).await?;
    // The same text twice is one warning.
    status(&mut runtime, Some(&turn), OPEN, "warning", WITHDRAWN).await?;
    status(&mut runtime, Some(&turn), OPEN, "warning", WITHDRAWN).await?;
    assert_eq!(warnings(&runtime), [WITHDRAWN]);
    // At most three: the newest stay, oldest first.
    for index in 1..=4 {
        let text = format!("Warning number {index}");
        status(&mut runtime, Some(&turn), OPEN, "warning", &text).await?;
    }
    assert_eq!(
        warnings(&runtime),
        ["Warning number 2", "Warning number 3", "Warning number 4"]
    );
    // At most 500 characters each, cut on a character, never split.
    let long: String = "ñ".repeat(2_000);
    status(&mut runtime, Some(&turn), OPEN, "warning", &long).await?;
    let shown = warnings(&runtime);
    let last = shown.last().expect("the long warning is shown");
    assert!(
        last.chars().count() <= 500,
        "{} characters",
        last.chars().count()
    );
    assert!(last.starts_with(&"ñ".repeat(400)));
    assert_eq!(shown.len(), 3);
    Ok(())
}

#[tokio::test]
async fn changing_conversation_empties_the_warnings() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let (turn, _sent) = open_with_turn(&mut runtime).await?;
    status(&mut runtime, Some(&turn), OPEN, "warning", WITHDRAWN).await?;
    assert_eq!(warnings(&runtime), [WITHDRAWN]);
    open(&mut runtime, OTHER).await?;
    assert!(warnings(&runtime).is_empty(), "{:?}", warnings(&runtime));
    // Back in the first one, what was said there is not said again: the
    // hub does not keep its warnings, and neither does the Mac.
    open(&mut runtime, OPEN).await?;
    assert!(warnings(&runtime).is_empty(), "{:?}", warnings(&runtime));
    // A conversation opened with nothing under way empties them too.
    status(&mut runtime, None, OPEN, "warning", WITHDRAWN).await?;
    assert_eq!(warnings(&runtime), [WITHDRAWN]);
    runtime
        .command(Command::NewSession { agent_id: None })
        .await?;
    assert!(warnings(&runtime).is_empty(), "{:?}", warnings(&runtime));
    Ok(())
}

#[tokio::test]
async fn a_new_turn_starts_without_the_last_turn_warnings() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let (turn, _sent) = open_with_turn(&mut runtime).await?;
    status(&mut runtime, Some(&turn), OPEN, "warning", WITHDRAWN).await?;
    runtime
        .hub_event(HubEvent::Frame(json!({
            "event":"chat.completed","id":turn,"data":{"session_id":OPEN,"text":"Answer"}
        })))
        .await?;
    // The answer is in; its warnings stay beside it.
    assert!(runtime.state.durable.assistant.active_request.is_none());
    assert_eq!(warnings(&runtime), [WITHDRAWN]);
    let next = ask(&mut runtime).await?;
    assert_ne!(next, turn);
    assert!(warnings(&runtime).is_empty(), "{:?}", warnings(&runtime));
    Ok(())
}

#[tokio::test]
async fn a_reconnection_neither_repeats_nor_drops_what_was_shown() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let (turn, _sent) = open_with_turn(&mut runtime).await?;
    status(&mut runtime, Some(&turn), OPEN, "warning", WITHDRAWN).await?;
    runtime.hub_event(HubEvent::Disconnected).await?;
    runtime.hub_event(HubEvent::Connected).await?;
    assert_eq!(warnings(&runtime), [WITHDRAWN]);
    Ok(())
}
