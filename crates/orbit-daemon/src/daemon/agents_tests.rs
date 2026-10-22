//! Several agents (hub-v1 «Agents»), through the daemon's own paths: a
//! synthetic hub channel and a synthetic helper. The daemon learns the list
//! from `ready`, keeps it for starting without the hub, publishes it, binds
//! each conversation to its agent and names that agent in `chat.send`; the
//! wake phrase hears every name and the question goes to the one named.
use super::*;
use crate::{daemon::audio::tests::fixture, hub::HubEvent};
use orbit_core::{protocol::Command, topics::TopicId};
use tokio::sync::mpsc;

const DEVICE: &str = "device-agents";
const SESSION: &str = "30000000-0000-4000-8000-000000000005";
const RECORDING: &str = "6f2b7d5a-9c41-4b7e-8d3f-1a2b3c4d5e70";
const MIC: &str = "mic-stream-agents";
const START: i64 = 1_800_000_000_000;

type Fixture = (
    tempfile::TempDir,
    Runtime,
    mpsc::Receiver<Value>,
    mpsc::Receiver<Value>,
);

fn with_hub() -> Result<Fixture> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, helper) = fixture(directory.path())?;
    let (hub, hub_frames) = mpsc::channel(16);
    runtime.hub.install(hub);
    runtime.settings.device_id = Some(DEVICE.into());
    Ok((directory, runtime, hub_frames, helper))
}

/// What a hub with two agents says on connecting.
fn two_agents() -> Value {
    json!({"event":"ready", "data": {
        "device_id": DEVICE, "hub_protocol": 1, "assistant_name": "Deimos",
        "agents": [
            {"id":"deimos","name":"Deimos","kind":"hermes","capabilities":["images","voice","resume"]},
            {"id":"atlas","name":"Atlas","kind":"hermes","capabilities":["resume"]}
        ],
        "default_agent_id": "deimos"
    }})
}

async fn ready(runtime: &mut Runtime, frame: Value) -> Result<()> {
    runtime.hub_event(HubEvent::Frame(frame)).await
}

fn status(runtime: &Runtime) -> Result<Value> {
    let source = crate::daemon::topics::TopicSource {
        state: &runtime.state,
        runtime: Some(runtime),
        now_ms: 0,
    };
    let mut board = crate::daemon::topics::TopicBoard::new("agents".into());
    let channels = board.channels();
    let _status = channels[&TopicId::Status].subscribe();
    let _assistant = channels[&TopicId::Assistant].subscribe();
    board.publish(&source);
    let status = serde_json::to_value(&channels[&TopicId::Status].borrow().data)?;
    let assistant = serde_json::to_value(&channels[&TopicId::Assistant].borrow().data)?;
    Ok(json!({
        "status": status["knowledge"]["value"],
        "assistant": assistant["knowledge"]["value"],
    }))
}

/// The next chat.send to the hub. A stall guard, not a budget: the frame is
/// sent before the command returns.
async fn chat_send(frames: &mut mpsc::Receiver<Value>) -> Result<Value> {
    let frame = tokio::time::timeout(std::time::Duration::from_secs(10), frames.recv())
        .await
        .ok()
        .flatten()
        .context("no chat.send")?;
    if frame["method"] != "chat.send" {
        bail!("not a chat.send: {frame}");
    }
    Ok(frame["params"].clone())
}

/// One turn, answered as the hub answers it (the conversation's id, then
/// its completion), so the next one is admitted.
async fn ask(runtime: &mut Runtime, frames: &mut mpsc::Receiver<Value>) -> Result<Value> {
    runtime
        .command(Command::Chat {
            attachments: Vec::new(),
            text: "pregunta sintética".into(),
        })
        .await?;
    let params = chat_send(frames).await?;
    let turn = runtime
        .state
        .durable
        .active_request
        .clone()
        .context("no turn")?;
    let session = runtime
        .state
        .session_id()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    for frame in [
        json!({"event": "chat.session", "id": turn, "data": {"session_id": session}}),
        json!({"event": "chat.completed", "id": turn,
            "data": {"session_id": session, "text": "hecho", "status": "completed"}}),
    ] {
        runtime.hub_event(HubEvent::Frame(frame)).await?;
    }
    if runtime.state.durable.active_request.is_some() {
        bail!("the turn did not end");
    }
    Ok(params)
}

fn helper_commands(helper: &mut mpsc::Receiver<Value>) -> Vec<Value> {
    let mut commands = Vec::new();
    while let Ok(command) = helper.try_recv() {
        if command["command"] != "status" {
            commands.push(command);
        }
    }
    commands
}

fn meeting(runtime: &mut Runtime) {
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "recording",
        json!({"id": RECORDING, "title": "Revisión sintética", "started_at_ms": START,
            "ended_at_ms": null, "expires_at_ms": START + 3_600_000, "application_pid": null,
            "audio": "system", "phase": "recording", "sealed_chunks": 0, "gaps": 0}),
    );
    runtime.capture.mic_stream = Some(MIC.into());
    runtime.features.question.clock_ms = Some(START + 60_000);
}

fn heard(name: Option<&str>) -> Value {
    let mut event = json!({"v": 1, "event": "assistant_question", "stream_id": MIC,
        "text": "¿Qué se decidió?", "started_ms": START + 50_000, "ended_ms": START + 55_000});
    if let Some(name) = name {
        event["assistant_name"] = json!(name);
    }
    event
}

#[tokio::test]
async fn the_hubs_agents_reach_the_app_and_the_wake_phrase_and_survive_a_restart() -> Result<()> {
    let (directory, mut runtime, _hub, mut helper) = with_hub()?;
    meeting(&mut runtime);
    runtime.features.question.helper_listens = true;
    ready(&mut runtime, two_agents()).await?;
    runtime.sync_wake_phrase().await?;

    let published = status(&runtime)?;
    assert_eq!(
        published["status"]["agents"],
        json!([{"id":"deimos","name":"Deimos"},{"id":"atlas","name":"Atlas"}])
    );
    assert_eq!(published["status"]["default_agent_id"], "deimos");
    let commands = helper_commands(&mut helper);
    let wake = commands
        .iter()
        .find(|c| c["command"] == "audio_wake_phrase" && c["enabled"] == true)
        .context("wake on")?;
    assert_eq!(wake["assistant_name"], "Deimos", "today's field stays");
    assert_eq!(wake["assistant_names"], json!(["Deimos", "Atlas"]));

    // A new runtime reads the same list before any hub connection.
    drop(runtime);
    let restarted =
        crate::config::Paths::resolve(Some(directory.path().to_owned()))?.load_settings()?;
    let roster = restarted.roster();
    assert_eq!(roster.names(), ["Deimos", "Atlas"]);
    assert_eq!(roster.default_agent_id(), "deimos");
    Ok(())
}

/// A hub that sends no list (0.9.0) has one agent, named by `assistant_name`,
/// even after a hub that had several.
#[tokio::test]
async fn a_hub_without_agents_has_one_named_by_its_assistant_name() -> Result<()> {
    let (_directory, mut runtime, _hub, _helper) = with_hub()?;
    ready(&mut runtime, two_agents()).await?;
    ready(
        &mut runtime,
        json!({"event":"ready", "data": {
            "device_id": DEVICE, "hub_protocol": 1, "assistant_name": "Vega"
        }}),
    )
    .await?;
    let published = status(&runtime)?;
    assert_eq!(
        published["status"]["agents"],
        json!([{"id":"default","name":"Vega"}])
    );
    assert_eq!(published["status"]["default_agent_id"], "default");
    assert!(runtime.paths.load_settings()?.hub_agents.is_none());
    Ok(())
}

/// A list the Mac cannot rely on is not taken: the one it had stays.
#[tokio::test]
async fn a_list_whose_default_is_not_in_it_keeps_the_one_the_mac_had() -> Result<()> {
    let (_directory, mut runtime, _hub, _helper) = with_hub()?;
    ready(&mut runtime, two_agents()).await?;
    let mut broken = two_agents();
    broken["data"]["agents"] =
        json!([{"id":"vega","name":"Vega","kind":"hermes","capabilities":[]}]);
    broken["data"]["default_agent_id"] = json!("nadie");
    ready(&mut runtime, broken).await?;
    assert_eq!(runtime.settings.roster().names(), ["Deimos", "Atlas"]);
    assert_eq!(
        runtime.paths.load_settings()?.roster().names(),
        ["Deimos", "Atlas"]
    );
    assert_eq!(runtime.state.connection(), orbit_core::Connection::Online);
    Ok(())
}

/// A 0.9.0 hub refuses unknown `chat.send` fields: it never gets `agent_id`.
#[tokio::test]
async fn a_hub_without_agents_never_receives_an_agent_id() -> Result<()> {
    let (_directory, mut runtime, mut hub_frames, _helper) = with_hub()?;
    ready(
        &mut runtime,
        json!({"event":"ready", "data": {"device_id": DEVICE, "hub_protocol": 1}}),
    )
    .await?;
    let params = ask(&mut runtime, &mut hub_frames).await?;
    assert!(params.get("agent_id").is_none(), "{params}");
    Ok(())
}

/// One agent, several, and the agent of a conversation opened from the list.
#[tokio::test]
async fn each_turn_names_the_agent_its_conversation_is_with() -> Result<()> {
    let (_directory, mut runtime, mut hub_frames, _helper) = with_hub()?;
    ready(&mut runtime, two_agents()).await?;
    // A conversation begun without a choice is with the default agent.
    let params = ask(&mut runtime, &mut hub_frames).await?;
    assert_eq!(params["agent_id"], "deimos");

    runtime
        .command(Command::NewSession {
            agent_id: Some("atlas".into()),
        })
        .await?;
    assert_eq!(status(&runtime)?["assistant"]["agent_id"], "atlas");
    let params = ask(&mut runtime, &mut hub_frames).await?;
    assert_eq!(params["agent_id"], "atlas");
    assert!(params.get("session_id").is_none(), "a new conversation");

    // Back to the default agent with «Nueva conversación».
    runtime
        .command(Command::NewSession { agent_id: None })
        .await?;
    let params = ask(&mut runtime, &mut hub_frames).await?;
    assert_eq!(params["agent_id"], "deimos");

    // Another agent's conversation, opened from the list, stays its own.
    runtime
        .command(Command::SessionOpen {
            id: SESSION.into(),
            agent_id: Some("atlas".into()),
        })
        .await?;
    let params = ask(&mut runtime, &mut hub_frames).await?;
    assert_eq!(params["agent_id"], "atlas");
    assert_eq!(params["session_id"], SESSION);
    Ok(())
}

/// An agent the hub did not list is refused before anything changes, with
/// the id asked for and the ones there are.
#[tokio::test]
async fn an_agent_the_hub_did_not_list_is_refused_with_the_ones_there_are() -> Result<()> {
    let (_directory, mut runtime, _hub, _helper) = with_hub()?;
    ready(&mut runtime, two_agents()).await?;
    runtime
        .command(Command::SessionOpen {
            id: SESSION.into(),
            agent_id: Some("atlas".into()),
        })
        .await?;
    for command in [
        Command::NewSession {
            agent_id: Some("orion".into()),
        },
        Command::SessionOpen {
            id: "30000000-0000-4000-8000-000000000006".into(),
            agent_id: Some("orion".into()),
        },
    ] {
        let Err(error) = runtime.command(command).await else {
            bail!("an unknown agent was admitted");
        };
        let text = error.to_string();
        assert!(text.contains("«orion»"), "{text}");
        assert!(text.contains("deimos, atlas"), "{text}");
    }
    assert_eq!(runtime.state.session_id().as_deref(), Some(SESSION));
    assert_eq!(runtime.state.agent_id(), Some("atlas"));
    Ok(())
}

/// «Oye Atlas» during a meeting while the open conversation is Deimos's:
/// the question goes to Atlas, in a conversation of its own; «Oye Deimos»
/// stays in Deimos's.
#[tokio::test]
async fn the_spoken_question_goes_to_the_agent_named() -> Result<()> {
    let (_directory, mut runtime, mut hub_frames, _helper) = with_hub()?;
    ready(&mut runtime, two_agents()).await?;
    meeting(&mut runtime);
    runtime
        .command(Command::SessionOpen {
            id: SESSION.into(),
            agent_id: Some("deimos".into()),
        })
        .await?;
    runtime.native_event(heard(Some("Deimos"))).await?;
    let params = chat_send(&mut hub_frames).await?;
    assert_eq!(params["agent_id"], "deimos");
    assert_eq!(params["session_id"], SESSION);
    assert_eq!(params["meeting"]["recording_id"], RECORDING);
    let turn = runtime
        .state
        .durable
        .active_request
        .clone()
        .context("no turn")?;
    runtime
        .hub_event(HubEvent::Frame(json!({
            "event": "chat.completed", "id": turn,
            "data": {"session_id": SESSION, "text": "hecho", "status": "completed"}
        })))
        .await?;

    runtime.native_event(heard(Some("Atlas"))).await?;
    let params = chat_send(&mut hub_frames).await?;
    assert_eq!(params["agent_id"], "atlas");
    assert!(params.get("session_id").is_none(), "{params}");
    assert_eq!(params["meeting"]["recording_id"], RECORDING);
    assert_eq!(runtime.state.agent_id(), Some("atlas"));
    Ok(())
}

/// A question from a helper that knows one name goes to the open conversation.
#[tokio::test]
async fn a_question_without_a_name_stays_in_the_open_conversation() -> Result<()> {
    let (_directory, mut runtime, mut hub_frames, _helper) = with_hub()?;
    ready(&mut runtime, two_agents()).await?;
    meeting(&mut runtime);
    runtime
        .command(Command::SessionOpen {
            id: SESSION.into(),
            agent_id: Some("atlas".into()),
        })
        .await?;
    runtime.native_event(heard(None)).await?;
    let params = chat_send(&mut hub_frames).await?;
    assert_eq!(params["agent_id"], "atlas");
    assert_eq!(params["session_id"], SESSION);
    Ok(())
}
