use super::*;
use crate::daemon::audio::tests::fixture;
use crate::daemon::turn::said;
use crate::hub::HubEvent;
use orbit_core::notice::{DataValue, NoticeCode, NoticeKey, Token};
use orbit_core::protocol::Command;

/// SIM-21: a cause is no longer a sentence in the notice; it is logged in
/// `daemon.log` under the turn's op. The fixture logs there.
fn logging(runtime: &mut Runtime, directory: &std::path::Path) {
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory));
}

/// The last `daemon.log` line of `event`, as written.
fn last_logged(directory: &std::path::Path, event: &str) -> Result<String> {
    let log = std::fs::read_to_string(directory.join("logs/daemon.log"))?;
    log.lines()
        .rev()
        .find(|line| line.contains(&format!("\"event\":\"{event}\"")))
        .map(str::to_owned)
        .context("event absent from daemon.log")
}

/// One value of the current notice's data.
fn datum(runtime: &Runtime, key: NoticeKey) -> Option<DataValue> {
    runtime
        .state
        .durable
        .notice
        .as_ref()
        .and_then(|notice| notice.data.get(&key).cloned())
}

impl Runtime {
    async fn reconcile_snapshot(&mut self, value: &Value) -> Result<()> {
        let snapshot = serde_json::from_value(value.clone())?;
        self.reconcile_typed_snapshot(&snapshot).await
    }

    async fn interaction_result(&mut self, event: &str, frame: &Value) -> Result<()> {
        #[derive(Deserialize)]
        struct Receipt {
            id: Option<String>,
            data: InteractionData,
        }
        let receipt: Receipt = serde_json::from_value(frame.clone())?;
        self.interaction_result_typed(
            event == "interaction.resolved",
            receipt.id.as_deref(),
            &receipt.data,
        )
        .await
    }
}

fn snapshot(revision: u64, session: &str, pending: Value) -> Value {
    turn_snapshot(
        revision,
        session,
        "10000000-0000-0000-0000-000000000001",
        "streaming",
        pending,
    )
}

fn turn_snapshot(revision: u64, session: &str, turn: &str, status: &str, pending: Value) -> Value {
    json!({"revision":revision,"sessions":[{"session_id":session,"turn_id":turn,
        "status":status,"pending":pending}],"omitted":false})
}
fn question() -> Value {
    json!({"request_id":"question-one","kind":"approval","state":"pending","expires_at":null,"description":"Synthetic approval"})
}

#[tokio::test]
async fn reconnect_restores_only_the_selected_sessions_unanswered_questions() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    {
        let fixture_source: Option<String> = Some("selected".into());
        let fixture_value = serde_json::json!(fixture_source);
        crate::daemon::authority_fixture::set(&mut runtime.state, "session_id", fixture_value);
    }
    runtime
        .reconcile_snapshot(&snapshot(2, "selected", json!([question()])))
        .await?;
    assert_eq!(
        runtime
            .state
            .durable
            .pending_approval
            .as_ref()
            .context("missing pending")?
            .request_id,
        "question-one"
    );
    assert_eq!(
        runtime.state.durable.active_request.as_deref(),
        Some("10000000-0000-0000-0000-000000000001")
    );
    runtime
        .reconcile_snapshot(&snapshot(1, "selected", json!([])))
        .await?;
    assert!(runtime.state.durable.pending_approval.is_some());
    runtime
        .reconcile_snapshot(&snapshot(3, "another-session", json!([])))
        .await?;
    assert!(runtime.state.durable.pending_approval.is_some());
    runtime
        .reconcile_snapshot(&snapshot(4, "selected", json!([])))
        .await?;
    assert!(runtime.state.durable.pending_approval.is_none());
    runtime.state.durable.active_request = Some("20000000-0000-0000-0000-000000000002".into());
    runtime
        .reconcile_snapshot(&snapshot(5, "selected", json!([question()])))
        .await?;
    assert_eq!(
        runtime.state.durable.active_request.as_deref(),
        Some("20000000-0000-0000-0000-000000000002")
    );
    assert!(runtime.state.durable.pending_approval.is_none());
    assert!(
        runtime.hub.is_none(),
        "reconciliation does not invoke or resume Hermes"
    );
    Ok(())
}

#[tokio::test]
async fn response_is_not_applied_optimistically_and_expired_is_not_success() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    {
        let fixture_source: Option<String> = Some("selected".into());
        let fixture_value = serde_json::json!(fixture_source);
        crate::daemon::authority_fixture::set(&mut runtime.state, "session_id", fixture_value);
    }
    runtime
        .reconcile_snapshot(&snapshot(1, "selected", json!([question()])))
        .await?;
    let (sender, mut commands) = tokio::sync::mpsc::channel(4);
    runtime.hub.install(sender);
    runtime
        .command(orbit_core::protocol::Command::Approval { approved: true })
        .await?;
    assert!(runtime.state.durable.pending_approval.is_some());
    assert_eq!(runtime.state.notice_code(), Some(NoticeCode::ApprovalSent));
    assert!(
        runtime
            .command(orbit_core::protocol::Command::Approval { approved: true })
            .await
            .is_err()
    );
    let command = commands.recv().await.context("missing response command")?;
    assert_eq!(
        command["params"]["turn_id"],
        "10000000-0000-0000-0000-000000000001"
    );
    runtime.interaction_result("interaction.expired", &json!({"id":command["id"],"data":{"status":"expired","applied":false,"request_id":"question-one","session_id":"selected"}})).await?;
    assert!(runtime.state.durable.pending_approval.is_none());
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::ApprovalExpired)
    );
    Ok(())
}

#[tokio::test]
async fn malformed_or_misbound_receipts_cannot_claim_an_approval_was_applied() -> Result<()> {
    for alteration in 0..3 {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _) = fixture(directory.path())?;
        {
            let fixture_source: Option<String> = Some("selected".into());
            let fixture_value = serde_json::json!(fixture_source);
            crate::daemon::authority_fixture::set(&mut runtime.state, "session_id", fixture_value);
        }
        runtime
            .reconcile_snapshot(&snapshot(1, "selected", json!([question()])))
            .await?;
        runtime.conversation.interaction_response = Some(PendingResponse {
            command_id: "response".into(),
            request_id: "question-one".into(),
            session_id: "selected".into(),
            clarification: false,
        });
        let mut data = json!({"status":"ok","applied":true,"resolved":1,"request_id":"question-one","session_id":"selected"});
        match alteration {
            0 => data["resolved"] = json!(0),
            1 => data["request_id"] = json!("another-question"),
            _ => data["session_id"] = json!("another-session"),
        }
        runtime
            .interaction_result(
                "interaction.resolved",
                &json!({"id":"response","data":data}),
            )
            .await?;
        assert!(runtime.state.durable.pending_approval.is_some());
        assert_eq!(
            runtime.state.notice_code(),
            Some(NoticeCode::ApprovalOutcomeUnknown)
        );
    }
    Ok(())
}

fn pending_response(clarification: bool) -> PendingResponse {
    PendingResponse {
        command_id: "response".into(),
        request_id: "question-one".into(),
        session_id: "selected".into(),
        clarification,
    }
}

fn historical_receipt(applied: bool) -> Value {
    json!({"id":"response","data":{
        "status":if applied { "ok" } else { "expired" },
        "applied":applied,"resolved":usize::from(applied),
        "request_id":"question-one","session_id":"selected",
        "turn_id":"10000000-0000-0000-0000-000000000001",
        "stale_interaction":true
    }})
}

#[tokio::test]
async fn historical_receipt_cannot_clear_a_new_snapshot_question_with_the_same_id() -> Result<()> {
    for clarification in [false, true] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut native) = fixture(directory.path())?;
        {
            let fixture_source: Option<String> = Some("selected".into());
            let fixture_value = serde_json::json!(fixture_source);
            crate::daemon::authority_fixture::set(&mut runtime.state, "session_id", fixture_value);
        }
        runtime
            .reconcile_snapshot(&snapshot(1, "selected", json!([question()])))
            .await?;
        runtime.conversation.interaction_response = Some(pending_response(clarification));

        let mut replacement = question();
        replacement["description"] = json!("New decision, reused request ID");
        replacement["kind"] = json!(if clarification { "clarify" } else { "approval" });
        runtime
            .reconcile_snapshot(&snapshot(2, "selected", json!([replacement])))
            .await?;
        runtime
            .interaction_result("interaction.resolved", &historical_receipt(true))
            .await?;

        assert!(runtime.conversation.interaction_response.is_none());
        let pending = runtime
            .state
            .durable
            .pending_approval
            .as_ref()
            .context("replacement lost")?;
        assert_eq!(pending.request_id, "question-one");
        assert_eq!(pending.description, "New decision, reused request ID");
        assert_eq!(pending.clarification, clarification);
        assert_eq!(
            runtime.state.notice_code(),
            Some(NoticeCode::ApprovalReceiptStale)
        );
        let durable: orbit_core::State = runtime
            .fixture_store
            .load_snapshot()?
            .context("snapshot absent")?;
        assert_eq!(
            durable.durable.pending_approval,
            runtime.state.durable.pending_approval
        );
        assert_eq!(runtime.snapshot_revision, Some(2));
        assert!(runtime.hub.is_none());
        assert!(native.try_recv().is_err());
    }
    Ok(())
}

#[tokio::test]
async fn historical_expiration_does_not_expire_a_replacement_question() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    {
        let fixture_source: Option<String> = Some("selected".into());
        let fixture_value = serde_json::json!(fixture_source);
        crate::daemon::authority_fixture::set(&mut runtime.state, "session_id", fixture_value);
    }
    runtime.conversation.interaction_response = Some(pending_response(false));
    runtime
        .reconcile_snapshot(&snapshot(2, "selected", json!([question()])))
        .await?;
    runtime
        .interaction_result("interaction.expired", &historical_receipt(false))
        .await?;
    assert!(runtime.conversation.interaction_response.is_none());
    assert!(runtime.state.durable.pending_approval.is_some());
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::ApprovalReceiptStale)
    );
    Ok(())
}

#[tokio::test]
async fn historical_receipt_does_not_resurrect_a_terminal_snapshot() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut native) = fixture(directory.path())?;
    {
        let fixture_source: Option<String> = Some("selected".into());
        let fixture_value = serde_json::json!(fixture_source);
        crate::daemon::authority_fixture::set(&mut runtime.state, "session_id", fixture_value);
    }
    runtime
        .reconcile_snapshot(&snapshot(1, "selected", json!([question()])))
        .await?;
    runtime.conversation.interaction_response = Some(pending_response(true));
    let mut terminal = snapshot(2, "selected", json!([]));
    terminal["sessions"][0]["status"] = json!("completed");
    runtime.reconcile_snapshot(&terminal).await?;
    runtime
        .interaction_result("interaction.resolved", &historical_receipt(true))
        .await?;
    assert!(runtime.conversation.interaction_response.is_none());
    assert!(runtime.state.durable.pending_approval.is_none());
    assert!(runtime.state.durable.active_request.is_none());
    assert_eq!(runtime.snapshot_revision, Some(2));
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::ApprovalReceiptStale)
    );
    assert!(runtime.hub.is_none());
    assert!(native.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn current_receipt_still_resolves_approval_and_clarification() -> Result<()> {
    for clarification in [false, true] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _) = fixture(directory.path())?;
        {
            let fixture_source: Option<String> = Some("selected".into());
            let fixture_value = serde_json::json!(fixture_source);
            crate::daemon::authority_fixture::set(&mut runtime.state, "session_id", fixture_value);
        }
        runtime
            .reconcile_snapshot(&snapshot(1, "selected", json!([question()])))
            .await?;
        runtime.conversation.interaction_response = Some(pending_response(clarification));
        let mut receipt = historical_receipt(true);
        receipt["data"]
            .as_object_mut()
            .context("receipt data absent")?
            .remove("stale_interaction");
        runtime
            .interaction_result("interaction.resolved", &receipt)
            .await?;
        assert!(runtime.conversation.interaction_response.is_none());
        assert!(runtime.state.durable.pending_approval.is_none());
        assert_eq!(
            runtime.state.notice_code(),
            Some(NoticeCode::ApprovalApplied)
        );
    }
    Ok(())
}

/// REF-25 A1/A3. This is the same ordering as E2E_SLOW/E2E_NEW: interrupt the
/// old turn, open a conversation, admit its replacement, reconcile while the
/// hub still says `submitting`, then receive both the obsolete and new output.
#[tokio::test]
async fn submitting_snapshot_keeps_new_turn_and_discards_interrupted_reply() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    let (hub, mut hub_frames) = tokio::sync::mpsc::channel(16);
    runtime.hub.install(hub);

    runtime
        .command(Command::Chat {
            text: "E2E_SLOW".into(),
        })
        .await?;
    let old_send = hub_frames.recv().await.context("old chat.send absent")?;
    let old_turn = old_send["id"]
        .as_str()
        .context("old turn absent")?
        .to_owned();
    runtime
        .hub_event(HubEvent::Frame(json!({
            "event":"chat.session","id":old_turn,
            "data":{"session_id":"old-session"}
        })))
        .await?;

    runtime.command(Command::Interrupt).await?;
    let interruption = hub_frames.recv().await.context("chat.interrupt absent")?;
    assert_eq!(interruption["method"], "chat.interrupt");
    assert_eq!(interruption["params"]["turn_id"], old_turn);
    runtime.command(Command::NewSession).await?;
    runtime
        .command(Command::Chat {
            text: "E2E_NEW".into(),
        })
        .await?;
    let new_send = hub_frames.recv().await.context("new chat.send absent")?;
    let new_turn = new_send["id"]
        .as_str()
        .context("new turn absent")?
        .to_owned();
    runtime
        .hub_event(HubEvent::Frame(json!({
            "event":"chat.session","id":new_turn,
            "data":{"session_id":"new-session"}
        })))
        .await?;

    runtime
        .hub_event(HubEvent::Frame(json!({
            "event":"state.snapshot","data":turn_snapshot(
                1,"new-session",&new_turn,"submitting",json!([])
            )
        })))
        .await?;
    assert_eq!(
        runtime.state.durable.active_request.as_deref(),
        Some(new_turn.as_str())
    );

    for (event, text) in [
        ("chat.delta", "obsolete partial"),
        ("chat.completed", "OBSOLETE_E2E_RESPONSE"),
    ] {
        runtime
            .hub_event(HubEvent::Frame(json!({
                "event":event,"id":old_turn,
                "data":{"session_id":"old-session","text":text,"status":"success"}
            })))
            .await?;
    }
    runtime
        .hub_event(HubEvent::Frame(json!({
            "event":"chat.delta","id":new_turn,
            "data":{"session_id":"new-session","text":"new partial"}
        })))
        .await?;
    runtime
        .hub_event(HubEvent::Frame(json!({
            "event":"chat.completed","id":new_turn,
            "data":{"session_id":"new-session","text":"Respuesta final sintética E2E_NEW",
                "status":"success"}
        })))
        .await?;

    assert!(runtime.state.durable.active_request.is_none());
    let turn = runtime
        .state
        .durable
        .turn
        .as_ref()
        .context("turno ausente")?;
    assert!(
        std::iter::once(&turn.user)
            .chain(turn.answer.as_ref())
            .all(|message| {
                !message.text.contains("obsolete")
                    && !message.text.contains("OBSOLETE_E2E_RESPONSE")
            })
    );
    assert_eq!(
        turn.answer.as_ref().map(|message| message.text.as_str()),
        Some("Respuesta final sintética E2E_NEW")
    );
    Ok(())
}

const TURN: &str = "10000000-0000-0000-0000-000000000001";
const NEW_TURN: &str = "20000000-0000-0000-0000-000000000002";

#[tokio::test]
async fn ref23_stop_writes_interrupt_to_the_hub_socket_before_retiring_it() -> Result<()> {
    use futures_util::StreamExt;
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    runtime.settings.hub_url = Some(format!("http://{}", listener.local_addr()?));
    let (written, observed) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        let mut socket = tokio_tungstenite::accept_async(stream).await?;
        while let Some(frame) = socket.next().await {
            match frame? {
                tokio_tungstenite::tungstenite::Message::Text(text) => {
                    let _ = written.send(serde_json::from_str::<Value>(&text)?);
                    return anyhow::Ok(());
                }
                tokio_tungstenite::tungstenite::Message::Ping(_) => {}
                tokio_tungstenite::tungstenite::Message::Close(_) => break,
                _ => {}
            }
        }
        anyhow::bail!("socket closed before interrupt")
    });
    let (hub, commands) = tokio::sync::mpsc::channel(4);
    let (events, mut received_events) = tokio::sync::mpsc::channel(4);
    runtime.hub.install(hub);
    runtime.hub_task = Some(tokio::spawn(crate::hub::run(
        runtime.settings.clone(),
        "synthetic-token".into(),
        true,
        commands,
        crate::hub::Events {
            generation: runtime.state.generation(),
            sender: events,
        },
    )));
    let (_, connected) = received_events
        .recv()
        .await
        .context("hub did not connect")?;
    assert!(matches!(connected, HubEvent::Connected));
    {
        let fixture_source: Option<String> = Some("selected".into());
        let fixture_value = serde_json::json!(fixture_source);
        crate::daemon::authority_fixture::set(&mut runtime.state, "session_id", fixture_value);
    }
    runtime.state.durable.active_request = Some(TURN.into());
    runtime
        .privacy_stop(orbit_core::privacy_stop::MarsStop::Order)
        .await;
    let frame = tokio::time::timeout(std::time::Duration::from_secs(2), observed).await??;
    assert_eq!(frame["method"], "chat.interrupt");
    assert_eq!(frame["params"]["turn_id"], TURN);
    server.await??;
    Ok(())
}

#[tokio::test]
async fn an_old_privacy_latch_cannot_send_a_new_interrupt() -> Result<()> {
    use futures_util::StreamExt;
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    runtime.settings.hub_url = Some(format!("http://{}", listener.local_addr()?));
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        let mut socket = tokio_tungstenite::accept_async(stream).await?;
        while let Some(frame) = socket.next().await {
            match frame? {
                tokio_tungstenite::tungstenite::Message::Ping(_) => {}
                next => return anyhow::Ok(next),
            }
        }
        anyhow::bail!("socket vanished without close frame")
    });
    let (hub, commands) = tokio::sync::mpsc::channel(4);
    let (events, mut received_events) = tokio::sync::mpsc::channel(4);
    runtime.hub.install(hub);
    runtime.hub_task = Some(tokio::spawn(crate::hub::run(
        runtime.settings.clone(),
        "synthetic-token".into(),
        true,
        commands,
        crate::hub::Events {
            generation: runtime.state.generation(),
            sender: events,
        },
    )));
    let (_, connected) = received_events
        .recv()
        .await
        .context("hub did not connect")?;
    assert!(matches!(connected, HubEvent::Connected));
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(&mut runtime.state, "privacy_stopped", fixture_value);
    }
    let result = runtime
        .send(crate::hub::command(
            "old-latch",
            "chat.interrupt",
            json!({"session_id":"selected","turn_id":TURN}),
        )?)
        .await;
    assert!(
        result.is_err(),
        "an old latch authorized a new network order"
    );
    runtime
        .hub
        .retire(crate::daemon::hub_link::Retirement::NetworkCancelled);
    if let Some(task) = runtime.hub_task.take() {
        task.await??;
    }
    assert!(
        matches!(
            server.await??,
            tokio_tungstenite::tungstenite::Message::Close(_)
        ),
        "a new interrupt reached the hub socket after an old latch"
    );
    Ok(())
}

#[tokio::test]
async fn rearmed_maintenance_keeps_the_new_hub_socket() -> Result<()> {
    use futures_util::StreamExt;
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(&mut runtime.state, "privacy_stopped", fixture_value);
    }
    runtime.privacy_retirement_owed = true;
    runtime.apply(orbit_core::Event::LocalRearm).await?;
    assert!(!runtime.state.privacy_stopped());
    assert!(!runtime.privacy_retirement_owed);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    runtime.settings.hub_url = Some(format!("http://{}", listener.local_addr()?));
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        let mut socket = tokio_tungstenite::accept_async(stream).await?;
        while let Some(frame) = socket.next().await {
            match frame? {
                tokio_tungstenite::tungstenite::Message::Text(text) => {
                    return anyhow::Ok(serde_json::from_str::<Value>(&text)?);
                }
                tokio_tungstenite::tungstenite::Message::Close(_) => {
                    anyhow::bail!("rearmed hub socket closed during maintenance")
                }
                _ => {}
            }
        }
        anyhow::bail!("rearmed hub socket disappeared")
    });
    let (hub, commands) = tokio::sync::mpsc::channel(4);
    let (events, mut received_events) = tokio::sync::mpsc::channel(4);
    runtime.hub.install(hub);
    runtime.hub_task = Some(tokio::spawn(crate::hub::run(
        runtime.settings.clone(),
        "synthetic-token".into(),
        true,
        commands,
        crate::hub::Events {
            generation: runtime.state.generation(),
            sender: events,
        },
    )));
    let (_, connected) = received_events
        .recv()
        .await
        .context("hub did not connect")?;
    assert!(matches!(connected, HubEvent::Connected));
    runtime.run_maintenance().await;
    runtime
        .send(crate::hub::command("after-rearm", "ping", json!({}))?)
        .await?;
    let frame = tokio::time::timeout(std::time::Duration::from_secs(2), server).await???;
    assert_eq!(frame["method"], "ping");
    Ok(())
}

#[tokio::test]
async fn a_hub_socket_that_cannot_open_is_reported_as_an_unconfirmed_interrupt() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    logging(&mut runtime, directory.path());
    let unused = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    runtime.settings.hub_url = Some(format!("http://{}", unused.local_addr()?));
    drop(unused);
    let (hub, commands) = tokio::sync::mpsc::channel(4);
    let (events, _received_events) = tokio::sync::mpsc::channel(4);
    runtime.hub.install(hub);
    runtime.hub_task = Some(tokio::spawn(crate::hub::run(
        runtime.settings.clone(),
        "synthetic-token".into(),
        true,
        commands,
        crate::hub::Events {
            generation: runtime.state.generation(),
            sender: events,
        },
    )));
    {
        let fixture_source: Option<String> = Some("selected".into());
        let fixture_value = serde_json::json!(fixture_source);
        crate::daemon::authority_fixture::set(&mut runtime.state, "session_id", fixture_value);
    }
    runtime.state.durable.active_request = Some(TURN.into());
    runtime
        .privacy_stop(orbit_core::privacy_stop::MarsStop::Order)
        .await;
    // Unconfirmed, not refused: the queued interrupt is not taken for a
    // socket write. The count goes to daemon.log under the stop's op.
    let cause = last_logged(directory.path(), "notice_cause")?;
    assert!(
        cause.contains("assistant_interrupt_unconfirmed") && cause.contains("1 trama"),
        "a queued interrupt was mistaken for a socket write: {cause}"
    );
    assert!(
        !runtime
            .conversation
            .interrupted_turns
            .contains("selected", TURN),
        "an unconfirmed socket write hid the turn on rearm"
    );
    Ok(())
}

#[tokio::test]
async fn failed_hub_drain_reports_both_unsent_frames() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    logging(&mut runtime, directory.path());
    let unused = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    runtime.settings.hub_url = Some(format!("http://{}", unused.local_addr()?));
    drop(unused);
    let (hub, commands) = tokio::sync::mpsc::channel(4);
    let (events, _received_events) = tokio::sync::mpsc::channel(4);
    runtime.hub.install(hub.clone());
    runtime.hub_task = Some(tokio::spawn(crate::hub::run(
        runtime.settings.clone(),
        "synthetic-token".into(),
        true,
        commands,
        crate::hub::Events {
            generation: runtime.state.generation(),
            sender: events,
        },
    )));
    hub.send(crate::hub::command("queued", "ping", json!({}))?)
        .await?;
    drop(hub);
    {
        let fixture_source: Option<String> = Some("selected".into());
        let fixture_value = serde_json::json!(fixture_source);
        crate::daemon::authority_fixture::set(&mut runtime.state, "session_id", fixture_value);
    }
    runtime.state.durable.active_request = Some(TURN.into());
    runtime
        .privacy_stop(orbit_core::privacy_stop::MarsStop::Order)
        .await;
    let cause = last_logged(directory.path(), "notice_cause")?;
    assert!(cause.contains("2 tramas"), "wrong unsent count: {cause}");
    Ok(())
}

/// REF-23 A4: her stop must retain the turn identity before the core clears
/// `active_request`, and REF-30 must keep a stale live snapshot from reviving it.
#[tokio::test]
async fn ref23_privacy_stop_interrupts_the_live_turn_and_keeps_it_hidden() -> Result<()> {
    use futures_util::StreamExt;
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    runtime.settings.hub_url = Some(format!("http://{}", listener.local_addr()?));
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        let mut socket = tokio_tungstenite::accept_async(stream).await?;
        while let Some(frame) = socket.next().await {
            match frame? {
                tokio_tungstenite::tungstenite::Message::Text(text) => {
                    return anyhow::Ok(serde_json::from_str::<Value>(&text)?);
                }
                tokio_tungstenite::tungstenite::Message::Ping(_) => {}
                tokio_tungstenite::tungstenite::Message::Close(_) => break,
                _ => {}
            }
        }
        anyhow::bail!("socket closed before the live turn was interrupted")
    });
    let (hub, commands) = tokio::sync::mpsc::channel(4);
    let (events, mut received_events) = tokio::sync::mpsc::channel(4);
    runtime.hub.install(hub);
    runtime.hub_task = Some(tokio::spawn(crate::hub::run(
        runtime.settings.clone(),
        "synthetic-token".into(),
        true,
        commands,
        crate::hub::Events {
            generation: runtime.state.generation(),
            sender: events,
        },
    )));
    let (_, connected) = received_events
        .recv()
        .await
        .context("hub did not connect")?;
    assert!(matches!(connected, HubEvent::Connected));
    {
        let fixture_source: Option<String> = Some("selected".into());
        let fixture_value = serde_json::json!(fixture_source);
        crate::daemon::authority_fixture::set(&mut runtime.state, "session_id", fixture_value);
    }
    runtime.state.durable.active_request = Some(TURN.into());

    runtime
        .privacy_stop(orbit_core::privacy_stop::MarsStop::Order)
        .await;
    let interruption = server.await??;
    assert_eq!(interruption["method"], "chat.interrupt");
    assert_eq!(interruption["params"]["turn_id"], TURN);
    assert!(
        runtime
            .conversation
            .interrupted_turns
            .contains("selected", TURN)
    );
    runtime.command(Command::Rearm).await?;
    runtime
        .reconcile_snapshot(&turn_snapshot(1, "selected", TURN, "streaming", json!([])))
        .await?;
    assert!(
        runtime.state.durable.active_request.is_none(),
        "a stale hub turn revived on rearm"
    );
    Ok(())
}

/// REF-30 A1/A2. The local interrupt is already applied before a hub snapshot
/// can arrive. Neither live hub state may reclaim that discarded identity,
/// while an independently identified replacement must remain visible. The two
/// live snapshots follow the real order (`submitting`, then `streaming`): a
/// memory that retired on any snapshot of that turn — not only on a non-live
/// one — would let the second of them revive it.
async fn assert_interrupted_snapshot_stays_out(status: &str) -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    let (hub, mut hub_frames) = tokio::sync::mpsc::channel(4);
    runtime.hub.install(hub);
    {
        let fixture_source: Option<String> = Some("selected".into());
        let fixture_value = serde_json::json!(fixture_source);
        crate::daemon::authority_fixture::set(&mut runtime.state, "session_id", fixture_value);
    }
    runtime.state.durable.active_request = Some(TURN.into());

    runtime.command(Command::Interrupt).await?;
    let interruption = hub_frames.recv().await.context("chat.interrupt absent")?;
    assert_eq!(interruption["method"], "chat.interrupt");
    assert_eq!(interruption["params"]["turn_id"], TURN);
    assert!(runtime.state.durable.active_request.is_none());

    for (revision, live) in [(1, "submitting"), (2, "streaming")] {
        runtime
            .reconcile_snapshot(&turn_snapshot(revision, "selected", TURN, live, json!([])))
            .await?;
        assert!(
            runtime.state.durable.active_request.is_none(),
            "interrupted turn revived from {live}"
        );
        assert!(
            runtime
                .conversation
                .interrupted_turns
                .contains("selected", TURN),
            "a live snapshot ({live}) retired the interrupted identity"
        );
    }

    runtime
        .reconcile_snapshot(&turn_snapshot(
            3,
            "selected",
            TURN,
            "interrupting",
            json!([]),
        ))
        .await?;
    assert!(
        !runtime
            .conversation
            .interrupted_turns
            .contains("selected", TURN),
        "a non-live snapshot did not retire the interrupted identity"
    );

    runtime
        .reconcile_snapshot(&turn_snapshot(4, "selected", NEW_TURN, status, json!([])))
        .await?;
    assert_eq!(
        runtime.state.durable.active_request.as_deref(),
        Some(NEW_TURN),
        "new turn was hidden with hub status {status}"
    );
    Ok(())
}

#[tokio::test]
async fn interrupted_submitting_snapshot_stays_out_and_a_new_turn_is_visible() -> Result<()> {
    assert_interrupted_snapshot_stays_out("submitting").await
}

#[tokio::test]
async fn interrupted_streaming_snapshot_stays_out_and_a_new_turn_is_visible() -> Result<()> {
    assert_interrupted_snapshot_stays_out("streaming").await
}

#[test]
fn interrupted_turn_memory_is_bounded_and_non_live_facts_retire_entries() {
    let mut interrupted = InterruptedTurns::default();
    for index in 0..=MAX_INTERRUPTED_TURNS {
        interrupted.remember(format!("session-{index}"), format!("turn-{index}"));
    }

    assert_eq!(interrupted.entries.len(), MAX_INTERRUPTED_TURNS);
    assert!(!interrupted.contains("session-0", "turn-0"));
    assert!(interrupted.contains("session-128", "turn-128"));

    interrupted.forget("session-128", "turn-128");
    assert!(!interrupted.contains("session-128", "turn-128"));
    assert_eq!(interrupted.entries.len(), MAX_INTERRUPTED_TURNS - 1);
}

/// REF-30 A4. An interrupt request that never left Orbit must not keep the
/// turn hidden: the hub still owns it live, so the memory goes, the notice
/// names the delivery cause, and the next snapshot shows a turn Mars can
/// interrupt again. The closed channel is synthetic: nothing here talks to a
/// real hub.
#[tokio::test]
async fn undelivered_interrupt_keeps_the_turn_visible_to_interrupt_again() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    logging(&mut runtime, directory.path());
    let (hub, hub_frames) = tokio::sync::mpsc::channel(4);
    runtime.hub.install(hub);
    // Dropping the receiver makes `try_send` fail without the request ever
    // leaving Orbit.
    drop(hub_frames);
    {
        let fixture_source: Option<String> = Some("selected".into());
        let fixture_value = serde_json::json!(fixture_source);
        crate::daemon::authority_fixture::set(&mut runtime.state, "session_id", fixture_value);
    }
    runtime.state.durable.active_request = Some(TURN.into());

    runtime.command(Command::Interrupt).await?;
    assert!(runtime.state.durable.active_request.is_none());
    assert!(
        !runtime
            .conversation
            .interrupted_turns
            .contains("selected", TURN),
        "a turn whose interrupt never left Orbit stayed hidden"
    );
    // The class of this code is «puedes arreglarlo»: interrupt again. The
    // delivery cause is logged under the order's op.
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::AssistantInterruptNotDelivered),
        "{:?}",
        runtime.state.durable.notice
    );

    runtime
        .reconcile_snapshot(&turn_snapshot(1, "selected", TURN, "streaming", json!([])))
        .await?;
    assert_eq!(
        runtime.state.durable.active_request.as_deref(),
        Some(TURN),
        "the hub still streams the turn but Orbit kept it hidden"
    );
    Ok(())
}

/// REF-30 A4. An `error` frame answering our `chat.interrupt` — matched by
/// command id — means the hub refused or could not interrupt, so the turn is
/// whatever the hub still publishes: the memory goes, the notice carries the
/// hub's cause, and the turn is visible to interrupt again. An error with an
/// unrelated id must not do any of that.
#[tokio::test]
async fn hub_rejected_interrupt_returns_the_turn_and_unrelated_errors_do_not() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    logging(&mut runtime, directory.path());
    let (hub, mut hub_frames) = tokio::sync::mpsc::channel(4);
    runtime.hub.install(hub);
    {
        let fixture_source: Option<String> = Some("selected".into());
        let fixture_value = serde_json::json!(fixture_source);
        crate::daemon::authority_fixture::set(&mut runtime.state, "session_id", fixture_value);
    }
    runtime.state.durable.active_request = Some(TURN.into());

    runtime.command(Command::Interrupt).await?;
    let interruption = hub_frames.recv().await.context("chat.interrupt absent")?;
    assert!(
        runtime
            .conversation
            .interrupted_turns
            .contains("selected", TURN)
    );

    runtime
        .hub_event(HubEvent::Frame(json!({
            "event":"error","id":"another-command",
            "data":{"code":"stale_turn","message":"El turno ya no está activo"}
        })))
        .await?;
    assert!(
        runtime
            .conversation
            .interrupted_turns
            .contains("selected", TURN),
        "an unrelated error retired the interrupted identity"
    );

    runtime
        .hub_event(HubEvent::Frame(json!({
            "event":"error","id":interruption["id"],
            "data":{"code":"upstream_error","message":"Hermes no aceptó interrumpir el turno"}
        })))
        .await?;
    assert!(
        !runtime
            .conversation
            .interrupted_turns
            .contains("selected", TURN),
        "the hub's refusal did not retire the interrupted identity"
    );
    // The notice must not claim Hermes did not interrupt: with codes like
    // `outcome_unknown` the order may have arrived. It names the hub's answer
    // and says the turn stays as the hub publishes it.
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::AssistantInterruptRefused)
    );
    assert_eq!(
        datum(&runtime, NoticeKey::Code),
        Token::new("upstream_error").map(DataValue::Token)
    );

    runtime
        .reconcile_snapshot(&turn_snapshot(1, "selected", TURN, "streaming", json!([])))
        .await?;
    assert_eq!(
        runtime.state.durable.active_request.as_deref(),
        Some(TURN),
        "the hub still streams the turn but Orbit kept it hidden"
    );
    Ok(())
}

/// REF-30 A4 follow-up. The answer to an interrupt command arrives on the
/// connection that carried it; once that connection is gone no frame can
/// ever match its command id again, so a reconnect must not keep the ack
/// slot claiming an answer is pending. The interrupted-turn memory itself is
/// a fact about the turn, not about the command, so it survives.
#[tokio::test]
async fn reconnect_forgets_the_interrupt_ack_that_can_no_longer_arrive() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    let (hub, mut hub_frames) = tokio::sync::mpsc::channel(4);
    runtime.hub.install(hub);
    {
        let fixture_source: Option<String> = Some("selected".into());
        let fixture_value = serde_json::json!(fixture_source);
        crate::daemon::authority_fixture::set(&mut runtime.state, "session_id", fixture_value);
    }
    runtime.state.durable.active_request = Some(TURN.into());

    runtime.command(Command::Interrupt).await?;
    let interruption = hub_frames.recv().await.context("chat.interrupt absent")?;
    assert_eq!(interruption["method"], "chat.interrupt");
    assert!(
        runtime.conversation.interrupted_turns.pending_ack.is_some(),
        "the fixture did not leave an interrupt answer pending"
    );

    runtime.hub_event(HubEvent::Connected).await?;
    assert!(
        runtime.conversation.interrupted_turns.pending_ack.is_none(),
        "a reconnect kept an interrupt ack that can no longer be answered"
    );
    assert!(
        runtime
            .conversation
            .interrupted_turns
            .contains("selected", TURN),
        "the reconnect also forgot the interrupted turn itself"
    );
    Ok(())
}

/// A live state must take up the hub's turn when Orbit holds none; a state
/// that is not live must clear the turn Orbit holds, so each case starts from
/// the state its decision has to change.
async fn assert_snapshot_turn_state(status: &str, expected_live: bool) -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    {
        let fixture_source: Option<String> = Some("selected".into());
        let fixture_value = serde_json::json!(fixture_source);
        crate::daemon::authority_fixture::set(&mut runtime.state, "session_id", fixture_value);
    }
    runtime.state.durable.active_request = (!expected_live).then(|| TURN.into());
    runtime
        .reconcile_snapshot(&turn_snapshot(1, "selected", TURN, status, json!([])))
        .await?;
    assert_eq!(
        runtime.state.durable.active_request.as_deref(),
        expected_live.then_some(TURN),
        "wrong live-turn decision for hub status {status}"
    );
    Ok(())
}

#[tokio::test]
async fn snapshot_status_decides_whether_the_response_is_live() -> Result<()> {
    for (status, live) in [
        ("idle", false),
        ("submitting", true),
        ("streaming", true),
        ("interrupting", false),
        ("completed", false),
        ("failed", false),
        ("outcome_unknown", false),
    ] {
        assert_snapshot_turn_state(status, live).await?;
    }
    Ok(())
}

#[tokio::test]
async fn outcome_unknown_says_the_answer_is_uncertain_instead_of_thinking() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    {
        let fixture_source: Option<String> = Some("selected".into());
        let fixture_value = serde_json::json!(fixture_source);
        crate::daemon::authority_fixture::set(&mut runtime.state, "session_id", fixture_value);
    }
    runtime.state.durable.active_request = Some(TURN.into());
    runtime.state.set_notice(NoticeCode::AssistantThinking);
    runtime
        .reconcile_snapshot(&turn_snapshot(
            1,
            "selected",
            TURN,
            "outcome_unknown",
            json!([]),
        ))
        .await?;
    assert!(runtime.state.durable.active_request.is_none());
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::AssistantOutcomeUnknown),
        "the notice still says: {:?}",
        runtime.state.durable.notice
    );
    Ok(())
}

#[tokio::test]
async fn unknown_snapshot_state_is_named_and_changes_nothing() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    {
        let fixture_source: Option<String> = Some("selected".into());
        let fixture_value = serde_json::json!(fixture_source);
        crate::daemon::authority_fixture::set(&mut runtime.state, "session_id", fixture_value);
    }
    runtime.state.durable.active_request = Some(TURN.into());
    runtime
        .reconcile_snapshot(&turn_snapshot(
            1,
            "selected",
            TURN,
            "future_state",
            json!([]),
        ))
        .await?;
    assert_eq!(runtime.state.durable.active_request.as_deref(), Some(TURN));
    assert_eq!(
        runtime.snapshot_revision, None,
        "an unknown state was applied"
    );
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::AssistantStateUnknown)
    );
    assert!(
        said::mentions(&runtime.state, "future_state"),
        "the notice does not name the value: {:?}",
        runtime.state.durable.notice
    );
    Ok(())
}

#[tokio::test]
async fn a_chat_rejection_keeps_its_code_and_op_without_logging_content() -> Result<()> {
    for code in ["session_not_found", "new_hub_rejection"] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _helper) = fixture(directory.path())?;
        logging(&mut runtime, directory.path());
        runtime
            .apply(orbit_core::Event::Session("foreign-session".into()))
            .await?;
        runtime.state.durable.active_request = Some("rejected-turn".into());
        runtime.begin_turn(
            orbit_core::notice::Op::parse("op-chat-rejected").context("op")?,
            orbit_core::notice::ReportSource::Daemon,
        );
        let mut frames = runtime.hub.open(4);
        runtime
            .hub_event(HubEvent::Frame(
                json!({"event":"error","id":"rejected-turn",
            "data":{"code":code,"message":"PRIVATE HUB MESSAGE"}}),
            ))
            .await?;
        runtime.report_notices();
        let notice =
            serde_json::to_value(runtime.state.durable.notice.as_ref().context("notice")?)?;
        assert_eq!(notice["data"]["code"], code);
        assert_eq!(notice["op"], "op-chat-rejected");
        if code == "session_not_found" {
            assert_eq!(notice["code"], "assistant_session_not_found");
            assert_eq!(
                runtime
                    .state
                    .durable
                    .notice
                    .as_ref()
                    .context("notice")?
                    .class(),
                orbit_core::notice::NoticeClass::Fixable
            );
            assert!(!runtime.state.notice_code().context("code")?.is_failure());
            assert!(runtime.state.session_id().is_none());
            let saved: orbit_core::DurableState =
                runtime.fixture_store.load_snapshot()?.context("snapshot")?;
            assert!(saved.session_id().is_none());
        } else {
            assert_eq!(notice["code"], "assistant_failed");
            assert_eq!(
                runtime.state.session_id().as_deref(),
                Some("foreign-session")
            );
        }
        assert!(runtime.state.durable.active_request.is_none());
        assert!(
            frames.try_recv().is_err(),
            "a rejected turn must never be replayed"
        );
        runtime
            .hub_event(HubEvent::Frame(
                json!({"event":"chat.session","id":"rejected-turn",
            "data":{"session_id":"late-session"}}),
            ))
            .await?;
        assert_ne!(runtime.state.session_id().as_deref(), Some("late-session"));
        let line = last_logged(directory.path(), "chat_rejected")?;
        assert!(line.contains(code));
        assert!(line.contains("op-chat-rejected"));
        assert!(!line.contains("PRIVATE HUB MESSAGE"));
    }
    Ok(())
}

#[tokio::test]
async fn hub_rejection_diagnostics_keep_numbers_without_text() -> Result<()> {
    for (request_id, event) in [
        (Some("rejected-turn"), "chat_rejected"),
        (None, "hub_error"),
    ] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _helper) = fixture(directory.path())?;
        logging(&mut runtime, directory.path());
        runtime.state.durable.active_request = request_id.map(str::to_owned);
        runtime.begin_turn(
            orbit_core::notice::Op::parse("op-rejection-numbers").context("op")?,
            orbit_core::notice::ReportSource::Daemon,
        );
        runtime.hub_event(HubEvent::Frame(json!({
            "event": "error", "id": request_id,
            "data": {"code": "too_large", "message": "PRIVATE HUB MESSAGE",
                "numbers": {"bytes": 128, "max_bytes": 64, "text": "PRIVATE DETAIL", "flag": true}}
        }))).await?;
        let line = last_logged(directory.path(), event)?;
        let entry: Value = serde_json::from_str(&line)?;
        assert_eq!(entry["fields"]["code"], "too_large");
        assert_eq!(entry["fields"]["op"], "op-rejection-numbers");
        let numbers: Value =
            serde_json::from_str(entry["fields"]["numbers"].as_str().context("numbers")?)?;
        assert_eq!(numbers, json!({"bytes": 128, "max_bytes": 64}));
        assert!(!line.contains("PRIVATE"));
    }
    Ok(())
}
