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
            primary: false,
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
        json!([
            {"id":"deimos","name":"Deimos","voice":true},
            {"id":"atlas","name":"Atlas","voice":false}
        ]),
        "each agent says whether it has a voice of its own, from its capabilities"
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
    assert_eq!(roster.reading_voice("atlas"), "deimos");
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

/// A question that names nobody goes to the primary, whoever the open
/// conversation is with: a helper that knows one name heard the primary's.
#[tokio::test]
async fn a_question_without_a_name_goes_to_the_primary() -> Result<()> {
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
    assert_eq!(params["agent_id"], "deimos");
    assert!(params.get("session_id").is_none(), "{params}");
    assert_eq!(runtime.state.agent_id(), Some("deimos"));
    Ok(())
}

/// A name the hub no longer lists (renamed or removed while the phrase was
/// heard) asks nobody: neither the open conversation nor the primary, which
/// was not called. Only its cause is recorded.
#[tokio::test]
async fn a_name_no_longer_listed_asks_nobody() -> Result<()> {
    let (_directory, mut runtime, mut hub_frames, _helper) = with_hub()?;
    ready(&mut runtime, two_agents()).await?;
    meeting(&mut runtime);
    runtime.native_event(heard(Some("Vega"))).await?;
    assert!(hub_frames.try_recv().is_err(), "nothing reached the hub");
    assert!(runtime.state.durable.active_request.is_none());
    Ok(())
}

/// The floating panel's «Preguntar a {principal}» names nobody: it goes to
/// the primary even while the open conversation is another agent's.
#[tokio::test]
async fn the_panels_question_goes_to_the_primary() -> Result<()> {
    let (_directory, mut runtime, mut hub_frames, _helper) = with_hub()?;
    ready(&mut runtime, two_agents()).await?;
    meeting(&mut runtime);
    runtime
        .command(Command::SessionOpen {
            id: SESSION.into(),
            agent_id: Some("atlas".into()),
        })
        .await?;
    runtime
        .command(Command::MeetingAsk {
            question: "¿Qué se decidió?".into(),
            minutes: 5,
        })
        .await?;
    let params = chat_send(&mut hub_frames).await?;
    assert_eq!(params["agent_id"], "deimos");
    assert_eq!(params["meeting"]["recording_id"], RECORDING);
    Ok(())
}

/// «Preguntar» in a note names nobody: the turn goes to the primary, in a
/// conversation of its own when the open one is another agent's, and in the
/// open one when it is the primary's.
#[tokio::test]
async fn a_turn_for_the_primary_opens_its_conversation_only_when_needed() -> Result<()> {
    let (_directory, mut runtime, mut hub_frames, _helper) = with_hub()?;
    ready(&mut runtime, two_agents()).await?;
    runtime
        .command(Command::SessionOpen {
            id: SESSION.into(),
            agent_id: Some("atlas".into()),
        })
        .await?;
    runtime
        .command(Command::Chat {
            attachments: Vec::new(),
            text: "Quiero preguntar sobre la nota «Revisión sintética».".into(),
            primary: true,
        })
        .await?;
    let params = chat_send(&mut hub_frames).await?;
    assert_eq!(params["agent_id"], "deimos");
    assert!(params.get("session_id").is_none(), "{params}");
    assert_eq!(runtime.state.agent_id(), Some("deimos"));
    Ok(())
}

/// What a hub says when its default agent has no voice and the other has one.
fn a_voice_beside_the_default() -> Value {
    json!({"event":"ready", "data": {
        "device_id": DEVICE, "hub_protocol": 1, "assistant_name": "Vega",
        "agents": [
            {"id":"vega","name":"Vega","kind":"acp","capabilities":["resume"]},
            {"id":"atlas","name":"Atlas","kind":"hermes","capabilities":["voice","resume"]}
        ],
        "default_agent_id": "vega"
    }})
}

/// Arms one read-aloud request against a one-shot hub that refuses it as a
/// voice that did not arrive, and returns where its request is recorded.
async fn speech_hub(
    runtime: &mut Runtime,
) -> Result<(
    std::sync::Arc<std::sync::Mutex<Option<super::pairing_session_tests::HttpRequest>>>,
    mpsc::Receiver<WorkResult>,
)> {
    let (base, seen) =
        super::pairing_session_tests::one_shot_http(super::pairing_session_tests::json_response(
            "400 Bad Request",
            r#"{"code":"speech_unavailable"}"#,
        ))
        .await?;
    runtime.settings.hub_url = Some(base.to_string());
    runtime.token = Some("synthetic-not-a-credential-agents".into());
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "capabilities.native_helper",
        json!(true),
    );
    let (work, work_events) = mpsc::channel(16);
    runtime.work.work_events = work;
    Ok((seen, work_events))
}

/// The body of the read-aloud request, once its work has come back and the
/// runtime has taken it. A stall guard, not a budget.
async fn speech_request(
    runtime: &mut Runtime,
    seen: &std::sync::Arc<std::sync::Mutex<Option<super::pairing_session_tests::HttpRequest>>>,
    work_events: &mut mpsc::Receiver<WorkResult>,
) -> Result<Value> {
    let completed = tokio::time::timeout(std::time::Duration::from_secs(10), work_events.recv())
        .await?
        .context("the speech work did not arrive")?;
    runtime.work_event(completed).await?;
    let request = super::pairing_session_tests::recorded(seen)?;
    assert!(
        request.start_line.starts_with("POST /v1/audio/speech "),
        "{}",
        request.start_line
    );
    Ok(serde_json::from_str(&request.body)?)
}

/// «Escuchar» on an answer of a conversation with Atlas is read in Atlas's
/// voice, though the default agent has none.
#[tokio::test]
async fn read_aloud_names_the_agent_of_the_open_conversation() -> Result<()> {
    let (_directory, mut runtime, _hub, _helper) = with_hub()?;
    ready(&mut runtime, a_voice_beside_the_default()).await?;
    runtime
        .command(Command::SessionOpen {
            id: SESSION.into(),
            agent_id: Some("atlas".into()),
        })
        .await?;
    let (seen, mut work_events) = speech_hub(&mut runtime).await?;
    runtime
        .command(Command::Speak {
            message_id: "answer-atlas".into(),
            text: "Respuesta sintética".into(),
        })
        .await?;
    let body = speech_request(&mut runtime, &seen, &mut work_events).await?;
    assert_eq!(
        body,
        json!({"text": "Respuesta sintética", "agent_id": "atlas"})
    );
    Ok(())
}

/// A spoken reply (a call's, or a turn asked by voice) is read in the voice
/// of the agent the turn was for.
#[tokio::test]
async fn a_spoken_reply_names_the_agent_of_its_turn() -> Result<()> {
    let (_directory, mut runtime, mut hub_frames, _helper) = with_hub()?;
    ready(&mut runtime, a_voice_beside_the_default()).await?;
    runtime
        .command(Command::NewSession {
            agent_id: Some("atlas".into()),
        })
        .await?;
    let (seen, mut work_events) = speech_hub(&mut runtime).await?;
    runtime.conversation.speak_next = true;
    let params = ask(&mut runtime, &mut hub_frames).await?;
    assert_eq!(params["agent_id"], "atlas");
    let body = speech_request(&mut runtime, &seen, &mut work_events).await?;
    assert_eq!(body, json!({"text": "hecho", "agent_id": "atlas"}));
    Ok(())
}

/// A hub that lists no agents (0.9.0) has one voice, and the request names none.
#[tokio::test]
async fn read_aloud_to_a_hub_without_agents_names_none() -> Result<()> {
    let (_directory, mut runtime, _hub, _helper) = with_hub()?;
    ready(
        &mut runtime,
        json!({"event":"ready", "data": {"device_id": DEVICE, "hub_protocol": 1}}),
    )
    .await?;
    let (seen, mut work_events) = speech_hub(&mut runtime).await?;
    runtime
        .command(Command::Speak {
            message_id: "answer-default".into(),
            text: "Respuesta sintética".into(),
        })
        .await?;
    let body = speech_request(&mut runtime, &seen, &mut work_events).await?;
    assert_eq!(body, json!({"text": "Respuesta sintética"}));
    Ok(())
}

/// What a hub says with three agents: Deimos, the default, and Atlas each
/// with a voice of its own, and Nova without one, read in Deimos's.
fn three_voices() -> Value {
    json!({"event":"ready", "data": {
        "device_id": DEVICE, "hub_protocol": 1, "assistant_name": "Deimos",
        "agents": [
            {"id":"deimos","name":"Deimos","kind":"hermes","capabilities":["voice","resume"]},
            {"id":"atlas","name":"Atlas","kind":"hermes","capabilities":["voice","resume"]},
            {"id":"nova","name":"Nova","kind":"acp","capabilities":["resume"]}
        ],
        "default_agent_id": "deimos"
    }})
}

/// Reads one answer aloud in a conversation with `agent_id` against a hub
/// whose voice does not arrive, and returns the assistant topic's voice.
async fn a_voice_fails(runtime: &mut Runtime, agent_id: Option<&str>) -> Result<Value> {
    open_with(runtime, agent_id).await?;
    let (seen, mut work_events) = speech_hub(runtime).await?;
    runtime
        .command(Command::Speak {
            message_id: "answer-voice".into(),
            text: "Respuesta sintética".into(),
        })
        .await?;
    speech_request(runtime, &seen, &mut work_events).await?;
    Ok(status(runtime)?["assistant"]["voice"].clone())
}

async fn open_with(runtime: &mut Runtime, agent_id: Option<&str>) -> Result<()> {
    runtime
        .command(Command::NewSession {
            agent_id: agent_id.map(str::to_owned),
        })
        .await
}

/// The assistant topic's voice in a new conversation with `agent_id`.
async fn voice_with(runtime: &mut Runtime, agent_id: Option<&str>) -> Result<Value> {
    open_with(runtime, agent_id).await?;
    Ok(status(runtime)?["assistant"]["voice"].clone())
}

/// Atlas's own voice failing says nothing of Deimos's: each voice keeps
/// what its last reading said, and one not read yet is unknown, not «no».
#[tokio::test]
async fn a_failed_voice_is_not_another_agents() -> Result<()> {
    let (_directory, mut runtime, _hub, _helper) = with_hub()?;
    ready(&mut runtime, three_voices()).await?;
    let failed = a_voice_fails(&mut runtime, Some("atlas")).await?;
    assert_eq!(failed["state"], "absent", "{failed}");
    assert_eq!(failed["code"], "speech_unavailable");
    assert_eq!(voice_with(&mut runtime, None).await?["state"], "unknown");
    assert_eq!(
        voice_with(&mut runtime, Some("nova")).await?["state"],
        "unknown",
        "Nova is read in Deimos's voice, which has not failed"
    );
    assert_eq!(
        voice_with(&mut runtime, Some("atlas")).await?["state"],
        "absent",
        "Atlas's failure stays known in its next conversation"
    );
    Ok(())
}

/// `two_agents`, with the hub naming `default_agent_id` its default.
fn two_agents_with_default(default_agent_id: &str) -> Value {
    let mut frame = two_agents();
    frame["data"]["default_agent_id"] = json!(default_agent_id);
    frame
}

/// One turn answered in `session`, with one tool step when `tool`; returns
/// the turn's id and its `chat.send`.
async fn turn_in(
    runtime: &mut Runtime,
    frames: &mut mpsc::Receiver<Value>,
    session: &str,
    tool: bool,
) -> Result<(String, Value)> {
    runtime
        .command(Command::Chat {
            attachments: Vec::new(),
            text: "pregunta sintética".into(),
            primary: false,
        })
        .await?;
    let params = chat_send(frames).await?;
    let turn = runtime
        .state
        .durable
        .active_request
        .clone()
        .context("no turn")?;
    let mut frames_in =
        vec![json!({"event": "chat.session", "id": turn, "data": {"session_id": session}})];
    if tool {
        frames_in.push(json!({"event": "chat.item", "id": turn, "data": {
            "session_id": session, "item_id": "herramienta-1", "kind": "tool",
            "phase": "completed", "label": "Mirando la pantalla", "subject": "la pantalla"}}));
    }
    frames_in.push(json!({"event": "chat.completed", "id": turn,
        "data": {"session_id": session, "text": "hecho", "status": "completed"}}));
    for frame in frames_in {
        runtime.hub_event(HubEvent::Frame(frame)).await?;
    }
    Ok((turn, params))
}

/// A conversation begun without naming an agent is with the hub's default
/// at that time. When the hub later names another default (its `orbit.toml`
/// changed and it restarted), that conversation is still the first agent's:
/// the next turn names it, or the hub refuses it as `agent_mismatch`.
#[tokio::test]
async fn a_conversation_keeps_its_agent_when_the_hub_changes_its_default() -> Result<()> {
    let (_directory, mut runtime, mut frames, _helper) = with_hub()?;
    ready(&mut runtime, two_agents_with_default("deimos")).await?;
    runtime
        .command(Command::NewSession { agent_id: None })
        .await?;
    let session = "30000000-0000-4000-8000-0000000000b1";
    let (_, first) = turn_in(&mut runtime, &mut frames, session, false).await?;
    assert_eq!(first["agent_id"], "deimos");

    ready(&mut runtime, two_agents_with_default("atlas")).await?;
    let (_, second) = turn_in(&mut runtime, &mut frames, session, false).await?;
    assert_eq!(second["session_id"], session);
    assert_eq!(
        second["agent_id"], "deimos",
        "the conversation {session} is Deimos's; naming another agent is agent_mismatch"
    );
    assert_eq!(status(&runtime)?["assistant"]["agent_id"], "deimos");
    Ok(())
}

/// The work the chat shows is the turn's in flight, never the last one's:
/// once a new turn is sent, the previous turn's work does not stand for it.
#[tokio::test]
async fn a_new_turn_does_not_carry_the_previous_turns_work() -> Result<()> {
    let (_directory, mut runtime, mut frames, _helper) = with_hub()?;
    ready(&mut runtime, two_agents()).await?;
    let session = "30000000-0000-4000-8000-0000000000b2";
    let (first, _) = turn_in(&mut runtime, &mut frames, session, true).await?;
    runtime
        .command(Command::Chat {
            attachments: Vec::new(),
            text: "otra pregunta sintética".into(),
            primary: false,
        })
        .await?;
    chat_send(&mut frames).await?;
    let second = runtime
        .state
        .durable
        .active_request
        .clone()
        .context("no turn")?;
    let carried = runtime
        .conversation
        .work
        .as_ref()
        .map(|work| work.turn_id.clone());
    assert_ne!(
        carried.as_deref(),
        Some(first.as_str()),
        "turn {second} is in flight and the topic still carries turn {first}'s work"
    );
    Ok(())
}

/// An agent without a voice of its own is read in the default agent's, so
/// what fails is the default agent's voice: for Nova and Deimos alike, not
/// for Atlas.
#[tokio::test]
async fn an_agent_read_in_the_default_voice_shares_its_failure() -> Result<()> {
    let (_directory, mut runtime, _hub, _helper) = with_hub()?;
    ready(&mut runtime, three_voices()).await?;
    let failed = a_voice_fails(&mut runtime, Some("nova")).await?;
    assert_eq!(failed["state"], "absent", "{failed}");
    assert_eq!(voice_with(&mut runtime, None).await?["state"], "absent");
    assert_eq!(
        voice_with(&mut runtime, Some("atlas")).await?["state"],
        "unknown"
    );
    Ok(())
}

/// An interrupted turn ended: its work says when, as a completed one does.
#[tokio::test]
async fn an_interrupted_turns_work_ends() -> Result<()> {
    let (_directory, mut runtime, mut frames, _helper) = with_hub()?;
    ready(&mut runtime, two_agents()).await?;
    let session = "30000000-0000-4000-8000-0000000000b3";
    runtime
        .command(Command::Chat {
            attachments: Vec::new(),
            text: "pregunta sintética".into(),
            primary: false,
        })
        .await?;
    chat_send(&mut frames).await?;
    let turn = runtime
        .state
        .durable
        .active_request
        .clone()
        .context("no turn")?;
    for frame in [
        json!({"event": "chat.session", "id": turn, "data": {"session_id": session}}),
        json!({"event": "chat.item", "id": turn, "data": {
            "session_id": session, "item_id": "herramienta-1", "kind": "tool",
            "phase": "started", "label": "Mirando la pantalla", "subject": "la pantalla"}}),
    ] {
        runtime.hub_event(HubEvent::Frame(frame)).await?;
    }
    runtime.command(Command::Interrupt).await?;
    assert!(runtime.state.durable.active_request.is_none());
    let work = runtime.conversation.work.as_ref().context("no work")?;
    assert!(
        work.ended_at_ms.is_some(),
        "the interrupted turn's work never ends: {work:?}"
    );
    Ok(())
}

/// `count` agents at the hub's bounds: a 32-byte id and a 48-character
/// name in four-byte characters.
fn many_agents(count: usize) -> Value {
    // Each at the hub's bounds: a 32-byte id, a 48-character name, a
    // 90-character description, four bytes a character, and every field of
    // its profile and account the hub publishes.
    let agents: Vec<Value> = (0..count)
        .map(|index| {
            json!({"id": format!("agente-{index:025}"),
                "name": format!("{index:02}{}", "𝔸".repeat(46)),
                "kind": "acp", "capabilities": ["images", "voice", "resume"],
                "service": "claude-code", "description": "𝔸".repeat(90), "color": "c4",
                "removable": true, "account": "signed_out",
                "account_email": "a".repeat(128), "plan": "𝔸".repeat(16),
                "limit": {"until_ms": 9_999_999_999_999_i64},
                "usage": {"used_percent": 100, "window": "seven_day_overage_included",
                          "resets_at_ms": 9_999_999_999_999_i64}})
        })
        .collect();
    json!({"event":"ready", "data": {
        "device_id": DEVICE, "hub_protocol": 1, "assistant_name": "𝔸".repeat(48),
        "agents": agents, "default_agent_id": format!("agente-{:025}", 0)}})
}

fn status_knowledge(runtime: &Runtime) -> Result<Value> {
    let source = crate::daemon::topics::TopicSource {
        state: &runtime.state,
        runtime: Some(runtime),
        now_ms: 0,
    };
    let mut board = crate::daemon::topics::TopicBoard::new("agents".into());
    let channels = board.channels();
    let _status = channels[&TopicId::Status].subscribe();
    board.publish(&source);
    let data = serde_json::to_value(&channels[&TopicId::Status].borrow().data)?;
    Ok(data["knowledge"].clone())
}

/// The status topic carries the hub's agents: as many as the Mac takes, at
/// the hub's bounds, keep the whole topic (connection, privacy stop, helper
/// and account included).
#[tokio::test]
async fn agents_at_their_ceiling_keep_the_status_topic() -> Result<()> {
    let (_directory, mut runtime, _frames, _helper) = with_hub()?;
    ready(&mut runtime, many_agents(orbit_core::agents::MAX_AGENTS)).await?;
    assert_eq!(
        runtime.settings.roster().agents().len(),
        orbit_core::agents::MAX_AGENTS
    );
    let knowledge = status_knowledge(&runtime)?;
    assert_eq!(
        knowledge["state"], "present",
        "refused whole: {}",
        knowledge["refusal"]
    );
    Ok(())
}

/// Forty agents are more than the Mac takes: the list is refused with its
/// count, the one the Mac had stays and the status topic with it.
#[tokio::test]
async fn forty_agents_keep_the_status_topic_and_the_list_the_mac_had() -> Result<()> {
    let (_directory, mut runtime, _frames, _helper) = with_hub()?;
    ready(&mut runtime, two_agents()).await?;
    ready(&mut runtime, many_agents(40)).await?;
    assert_eq!(runtime.settings.roster().names(), ["Deimos", "Atlas"]);
    let knowledge = status_knowledge(&runtime)?;
    assert_eq!(
        knowledge["state"], "present",
        "refused whole: {}",
        knowledge["refusal"]
    );
    Ok(())
}

/// The hub's agents changed (a profile, one added, an account): the Mac
/// keeps the new list, publishes each agent's profile and account, and the
/// wake phrase hears the new names.
#[tokio::test]
async fn a_changed_list_replaces_the_one_the_mac_had() -> Result<()> {
    let (directory, mut runtime, _hub, mut helper) = with_hub()?;
    meeting(&mut runtime);
    runtime.features.question.helper_listens = true;
    ready(&mut runtime, two_agents()).await?;
    runtime.sync_wake_phrase().await?;
    helper_commands(&mut helper);
    runtime
        .hub_event(HubEvent::Frame(json!({"event": "agents.changed", "data": {
            "agents": [
                {"id":"deimos","name":"Eris","kind":"hermes","capabilities":["voice"],
                 "service":"hermes","description":"Tu asistente","color":"c3","removable":false},
                {"id":"codex-1a2b3c4d","name":"Nova","kind":"acp","capabilities":["images"],
                 "service":"codex","description":"","color":"c2","removable":true,
                 "account":"expired","limit":{"until_ms":1_790_000_000_000_i64}}
            ],
            "default_agent_id": "codex-1a2b3c4d",
            "addable_agents": ["codex"]}})))
        .await?;
    let published = status(&runtime)?;
    assert_eq!(
        published["status"]["agents"],
        json!([
            {"id":"deimos","name":"Eris","voice":true,"service":"hermes",
             "description":"Tu asistente","color":"c3"},
            {"id":"codex-1a2b3c4d","name":"Nova","voice":false,"service":"codex",
             "description":"","color":"c2","removable":true,"account":"expired",
             "limit":{"until_ms":1_790_000_000_000_i64}}
        ])
    );
    assert_eq!(published["status"]["default_agent_id"], "codex-1a2b3c4d");
    assert_eq!(runtime.settings.assistant_name(), "Nova");
    let wake = helper_commands(&mut helper)
        .into_iter()
        .find(|c| c["command"] == "audio_wake_phrase" && c["enabled"] == true)
        .context("wake again")?;
    assert_eq!(wake["assistant_names"], json!(["Eris", "Nova"]));
    // Kept for a start without the hub, with what it can add.
    drop(runtime);
    let restarted =
        crate::config::Paths::resolve(Some(directory.path().to_owned()))?.load_settings()?;
    assert_eq!(restarted.roster().names(), ["Eris", "Nova"]);
    assert_eq!(
        restarted.roster().addable(),
        Some(&["codex".to_owned()][..])
    );
    Ok(())
}

/// «Seguir con Deimos»: a new conversation with the other agent whose first
/// turn names the one it takes over; a hub that cannot hand over is not asked.
#[tokio::test]
async fn a_handover_opens_a_conversation_that_names_the_one_it_takes_over() -> Result<()> {
    let (_directory, mut runtime, mut frames, _helper) = with_hub()?;
    ready(&mut runtime, two_agents()).await?;
    let refused = runtime
        .command(Command::Handover {
            session_id: SESSION.into(),
            agent_id: "deimos".into(),
            text: "¿Y mañana?".into(),
        })
        .await;
    assert!(
        refused
            .as_ref()
            .is_err_and(|error| error.to_string().contains("no pasa conversaciones")),
        "{refused:?}"
    );
    let mut able = two_agents();
    able["data"]["addable_agents"] = json!([]);
    ready(&mut runtime, able).await?;
    runtime
        .command(Command::Handover {
            session_id: SESSION.into(),
            agent_id: "atlas".into(),
            text: "¿Y mañana?".into(),
        })
        .await?;
    let params = chat_send(&mut frames).await?;
    assert_eq!(params["handover_from"], SESSION);
    assert_eq!(params["agent_id"], "atlas");
    assert_eq!(params["text"], "¿Y mañana?");
    assert!(params.get("session_id").is_none(), "{params}");
    assert_eq!(runtime.state.agent_id(), Some("atlas"));
    // Only its first turn names it.
    assert_eq!(runtime.conversation.handover_from, None);
    Ok(())
}
