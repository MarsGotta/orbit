//! Navigation detaches a hub turn without interrupting it.
use super::*;
use crate::daemon::audio::tests::fixture;
use crate::hub::HubEvent;
use orbit_core::protocol::Command;

const SESSION: &str = "30000000-0000-4000-8000-000000000005";

#[tokio::test]
async fn navigation_detaches_turn_and_reopening_observes_it() -> Result<()> {
    for new in [false, true] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _) = fixture(directory.path())?;
        let (sender, mut frames) = tokio::sync::mpsc::channel(16);
        runtime.link.outbound.install(sender);
        runtime.settings.device_id = Some("test-device".into());
        runtime
            .hub_event(HubEvent::Frame(
                json!({"event":"ready","data":{"device_id":"test-device","chat_observe":true}}),
            ))
            .await?;
        runtime
            .command(Command::SessionOpen {
                id: SESSION.into(),
                agent_id: None,
            })
            .await?;
        runtime
            .command(Command::Chat {
                text: "Question".into(),
                attachments: Vec::new(),
                primary: false,
            })
            .await?;
        let turn = runtime
            .state
            .durable
            .assistant
            .active_request
            .clone()
            .context("no submitted turn")?;
        let command = if new {
            Command::NewSession { agent_id: None }
        } else {
            Command::SessionOpen {
                id: "other".into(),
                agent_id: None,
            }
        };
        runtime.command(command).await?;
        assert!(runtime.state.durable.assistant.active_request.is_none());
        assert!(runtime.state.durable.assistant.turn.is_none());
        while let Ok(frame) = frames.try_recv() {
            assert_ne!(frame["method"], "chat.interrupt");
        }
        for (event, data) in [
            (
                "chat.delta",
                json!({"session_id":SESSION,"text":"Late private answer"}),
            ),
            (
                "chat.completed",
                json!({"session_id":SESSION,"text":"Late private answer"}),
            ),
            (
                "approval.required",
                json!({"session_id":SESSION,"request_id":"permission","description":"Permit?"}),
            ),
            (
                "chat.status",
                json!({"session_id":SESSION,"kind":"retrying_openai","text":"Retry"}),
            ),
        ] {
            runtime
                .hub_event(HubEvent::Frame(
                    json!({"event":event,"id":turn,"data":data}),
                ))
                .await?;
        }
        let notice_before = runtime.state.notice_code();
        runtime
            .hub_event(HubEvent::Frame(
                json!({"event":"error","id":turn,"data":{"message":"Late failure"}}),
            ))
            .await?;
        assert_eq!(runtime.state.notice_code(), notice_before);
        assert!(runtime.state.durable.assistant.turn.is_none());
        assert!(runtime.state.durable.assistant.pending_approval.is_none());
        runtime.hub_event(HubEvent::Frame(json!({"event":"state.snapshot","data":{
            "revision":1,"sessions":[{"session_id":SESSION,"turn_id":turn,"status":"streaming",
            "pending":[{"kind":"approval","state":"pending","request_id":"permission","description":"Permit?"}]}],"omitted":false}
        }))).await?;
        assert_eq!(
            runtime
                .assistant
                .waiting_sessions
                .get(SESSION)
                .map(String::as_str),
            Some("approval")
        );
        assert!(runtime.state.durable.assistant.pending_approval.is_none());
        runtime
            .command(Command::SessionOpen {
                id: SESSION.into(),
                agent_id: None,
            })
            .await?;
        let observed = frames.try_recv()?;
        assert_eq!(observed["method"], "chat.observe");
        assert_eq!(observed["params"]["session_id"], SESSION);
        runtime.hub_event(HubEvent::Frame(json!({"event":"state.snapshot","data":{
            "revision":2,"sessions":[{"session_id":SESSION,"turn_id":turn,"status":"streaming",
            "pending":[{"kind":"approval","state":"pending","request_id":"permission","description":"Permit?"}],
            "turn":{"user":{"id":turn,"role":"user","text":"Question","complete":true,"at_ms":1},
                    "answer":{"id":format!("{turn}:assistant"),"role":"assistant","text":"Partial","complete":false,"at_ms":2}}}]}
        }))).await?;
        assert_eq!(
            runtime.state.durable.assistant.active_request.as_deref(),
            Some(turn.as_str())
        );
        assert!(runtime.state.durable.assistant.pending_approval.is_some());
        assert_eq!(
            runtime
                .state
                .durable
                .assistant
                .turn
                .as_ref()
                .and_then(|t| t.answer.as_ref())
                .map(|a| a.text.as_str()),
            Some("Partial")
        );
    }
    Ok(())
}
