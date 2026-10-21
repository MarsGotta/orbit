//! REF-11: Deimos sees and acts with the macOS permission, without switches.
//!
//! Actor, encrypted journal and a synthetic MCP peer. The test executable is
//! outside a bundle, so automatic engine startup is refused before launching
//! a process on the machine running the test. The permissions arrive the way
//! the helper sends them today: `screen` and `accessibility` as booleans.
use super::*;
use orbit_core::PermissionState;
use std::sync::atomic::{AtomicUsize, Ordering};

fn grant(runtime: &mut Runtime, accessibility: PermissionState, screen: PermissionState) {
    {
        let fixture_value = serde_json::json!(accessibility);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.accessibility_state",
            fixture_value,
        );
    }
    {
        let fixture_value = serde_json::json!(screen);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.screen_state",
            fixture_value,
        );
    }
}

/// A status answer from the helper. A boolean Accessibility value models the
/// current Swift preflight; a string permits an explicit synthetic denial.
fn helper_permissions(id: &str, accessibility: Value, screen: bool) -> Value {
    let mut permissions = serde_json::Map::new();
    permissions.insert(crate::permissions::MICROPHONE.into(), json!("authorized"));
    permissions.insert(crate::permissions::SCREEN.into(), json!(screen));
    permissions.insert(crate::permissions::ACCESSIBILITY.into(), accessibility);
    permissions.insert(crate::permissions::CALENDARS.into(), json!("full_access"));
    json!({"event": "response", "id": id, "ok": true,
        "result": {"permissions": Value::Object(permissions)}})
}

/// A2: with both permissions granted, the actor's own maintenance starts the
/// bundled engine: no order, no switch, no pairing and no hub.
#[tokio::test]
async fn ref11_a2_with_both_permissions_the_engine_starts_by_itself_without_the_hub() -> Result<()>
{
    let directory = tempfile::tempdir()?;
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
    let (mut runtime, _native) = fixture(directory.path())?;
    let bundle = directory.path().join(orbit_core::orbit_bundle::NAME);
    let daemon =
        orbit_core::orbit_bundle::member_path(&bundle, orbit_core::orbit_bundle::Member::App)
            .context("synthetic daemon route")?;
    std::fs::create_dir_all(daemon.parent().context("daemon directory")?)?;
    std::fs::write(&daemon, b"synthetic daemon")?;
    runtime.features.computer.daemon_executable = Some(daemon.clone());
    let missing_driver = crate::cua::resolve_driver_path("", &daemon)?;
    assert!(
        !missing_driver.exists(),
        "fixture cannot launch a host driver"
    );
    runtime.settings.device_id = None;
    {
        let fixture_value = serde_json::json!(Connection::Offline);
        crate::daemon::authority_fixture::set(&mut runtime.state, "connection", fixture_value);
    }
    grant(
        &mut runtime,
        PermissionState::Granted,
        PermissionState::Granted,
    );
    runtime.state.set_notice(NoticeCode::SessionOpened);
    runtime.state.durable.voice_active = true;
    let calendar = runtime.features.calendar.view.clone();
    let turn = runtime.state.durable.turn.clone();
    let notice = runtime.state.durable.notice.clone();
    let (work, mut events) = mpsc::channel(4);
    runtime.work.work_events = work;
    runtime.maintain_computer().await?;
    let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .context("Orbit no intentó levantar el motor por su cuenta")?
        .context("worker absent")?;
    runtime.work_event(event).await?;
    let view = runtime
        .features
        .computer
        .view
        .clone()
        .context("computer state absent")?;
    assert!(
        view.detail.contains("No se encontró el driver CUA"),
        "{}",
        view.detail
    );
    assert!(view.detail.contains("MacOS/cua-driver"), "{}", view.detail);
    assert_eq!(view.stage, ComputerStage::Failed);
    assert_eq!(runtime.features.calendar.view, calendar);
    assert_eq!(runtime.state.durable.turn, turn);
    assert_eq!(runtime.state.durable.notice, notice);
    assert!(runtime.state.durable.voice_active);
    Ok(())
}

#[tokio::test]
async fn ref501_unbundled_daemon_maintenance_refuses_before_starting_a_worker() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    runtime.settings.device_id = None;
    {
        let fixture_value = serde_json::json!(Connection::Offline);
        crate::daemon::authority_fixture::set(&mut runtime.state, "connection", fixture_value);
    }
    grant(
        &mut runtime,
        PermissionState::Granted,
        PermissionState::Granted,
    );
    runtime.state.set_notice(NoticeCode::SessionOpened);
    runtime.state.durable.voice_active = true;
    let calendar = runtime.features.calendar.view.clone();
    let turn = runtime.state.durable.turn.clone();
    let notice = runtime.state.durable.notice.clone();
    let (work, mut events) = mpsc::channel(4);
    runtime.work.work_events = work;
    runtime.maintain_computer().await?;
    let orbit_core::computer_control::engine::Status::Failed { cause, .. } =
        runtime.engine_status()
    else {
        anyhow::bail!("el mantenimiento no registró la negativa del arranque");
    };
    assert!(
        cause.contains("no es la app de un bundle de Orbit"),
        "{cause}"
    );
    assert!(cause.contains(&std::env::current_exe()?.display().to_string()));
    assert!(runtime.features.computer.work.active().is_none());
    assert!(events.try_recv().is_err(), "se lanzó algo");
    assert_eq!(runtime.features.calendar.view, calendar);
    assert_eq!(runtime.state.durable.turn, turn);
    assert_eq!(runtime.state.durable.notice, notice);
    assert!(runtime.state.durable.voice_active);
    Ok(())
}

/// A3: a permission that macOS does not grant keeps the engine from being
/// launched, and the cause names it; an unknown one does not launch it either,
/// and says it is not known rather than that it is refused.
#[tokio::test]
async fn ref11_a3_without_a_permission_the_engine_is_not_launched_and_the_cause_is_named()
-> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    runtime.settings.device_id = Some("mac-fixture".into());
    grant(
        &mut runtime,
        PermissionState::Denied,
        PermissionState::Granted,
    );
    runtime.maintain_computer().await?;
    assert!(
        runtime.features.computer.work.active().is_none(),
        "sin Accesibilidad se lanzó el motor"
    );
    let reason = permission_reason(&runtime);
    assert!(
        serde_json::from_str::<Value>(&reason)?["permission"] == "accessibility",
        "no dice qué falta: {reason}"
    );
    assert!(
        serde_json::from_str::<Value>(&reason)?["permission"] != "screen_recording",
        "nombra un permiso concedido: {reason}"
    );
    assert!(
        !reason.to_lowercase().contains("permitir control"),
        "manda a un interruptor que no existe: {reason}"
    );

    grant(
        &mut runtime,
        PermissionState::Unknown,
        PermissionState::Granted,
    );
    runtime.maintain_computer().await?;
    assert!(
        runtime.features.computer.work.active().is_none(),
        "un permiso desconocido lanzó el motor"
    );
    let reason = permission_reason(&runtime);
    assert!(
        serde_json::from_str::<Value>(&reason)?["code"] == "permission_unknown"
            && serde_json::from_str::<Value>(&reason)?["permission"] == "accessibility",
        "un permiso desconocido se contó como un «no»: {reason}"
    );

    // Deimos is told the same, with the code that names the permission.
    grant(
        &mut runtime,
        PermissionState::Granted,
        PermissionState::Denied,
    );
    let (hub, mut replies) = mpsc::channel(4);
    runtime.hub.install(hub);
    runtime
        .device_request_raw(&json!({"request_id": "ref11-a3", "capability": "perform_action",
            "parameters": {"action": {"tool": "launch_app", "arguments": {"bundle_id": "com.apple.calculator"}}}}))
        .await?;
    let reply = replies
        .recv()
        .await
        .context("la petición quedó sin respuesta")?;
    assert_eq!(
        reply["params"]["refusal"]["code"], "permission_denied",
        "{reply}"
    );
    assert_eq!(
        reply["params"]["refusal"]["permission"], "screen_recording",
        "{reply}"
    );
    Ok(())
}

/// A3: an engine that is running is not stopped because a permission is not
/// known for a moment.
#[tokio::test]
async fn ref11_a3_an_unknown_permission_does_not_stop_a_running_engine() -> Result<()> {
    let mut h = Mac::new(false).await?;
    grant(
        &mut h.runtime,
        PermissionState::Unknown,
        PermissionState::Unknown,
    );
    h.runtime.maintain_computer().await?;
    assert!(h.runtime.engine_is_ready(), "lo desconocido paró el motor");
    h.runtime
        .revoke_computer(&orbit_core::refusal::Refusal::ReadInterrupted {
            cause: orbit_core::refusal::ReadInterruption::ComputerRevoked,
        })
        .await;
    Ok(())
}

/// A4: Swift's false preflight is a known non-grant and stops the engine.
#[tokio::test]
async fn ref11_a4_withdrawing_the_permission_stops_the_engine_and_says_why() -> Result<()> {
    let mut h = Mac::new(false).await?;
    grant(
        &mut h.runtime,
        PermissionState::Granted,
        PermissionState::Granted,
    );
    // The daemon asks the helper again on its own, so a change made in System
    // Settings reaches it without anybody restarting anything.
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(
            &mut h.runtime.state,
            "capabilities.native_helper",
            fixture_value,
        );
    }
    h.runtime.maintain_computer().await?;
    let mut status_id = None;
    while let Ok(command) = h._native.try_recv() {
        if command["command"] == "status" {
            status_id = command["id"].as_str().map(str::to_owned);
        }
    }
    // The answer uses the id of the status the daemon just sent. A different
    // id is unpaired and is not a permission observation (REF-404c).
    let status_id = status_id.context("el daemon no volvió a preguntar los permisos al helper")?;
    assert!(h.runtime.engine_is_ready());

    h.runtime
        .native_event(helper_permissions(&status_id, json!(false), true))
        .await?;
    h.runtime.maintain_computer().await?;
    assert!(
        !h.runtime.engine_is_ready(),
        "sin Accesibilidad el motor siguió tras el false real del helper"
    );
    assert_eq!(
        h.runtime.state.capabilities().accessibility_state(),
        PermissionState::NotGranted,
    );
    let remote = orbit_core::device_bridge::admits(
        "perform_action",
        orbit_core::device_bridge::Conditions {
            privacy_stopped: false,
            helper_connected: true,
            accessibility: h.runtime.state.capabilities().accessibility_state(),
            screen_recording: h.runtime.state.capabilities().screen_state(),
            calendar: orbit_core::device_bridge::CalendarPermission::Granted,
        },
    );
    assert_eq!(
        remote,
        Err(orbit_core::refusal::Refusal::PermissionNotGranted {
            permission: orbit_core::refusal::Permission::Accessibility,
        }),
        "Deimos debe recibir no concedido, no permission_unknown"
    );
    let orbit_core::topics::TopicData::Control(orbit_core::topics::Knowledge::Present {
        value,
        ..
    }) = crate::daemon::topic(&h.runtime, orbit_core::topics::TopicId::Control)
    else {
        panic!("control topic absent")
    };
    assert_eq!(
        value.engine,
        orbit_core::topics::Knowledge::Absent {
            refusal: orbit_core::refusal::Refusal::PermissionNotGranted {
                permission: orbit_core::refusal::Permission::Accessibility
            }
        }
    );
    let detail = permission_reason(&h.runtime);
    assert!(
        serde_json::from_str::<Value>(&detail)?["permission"] == "accessibility",
        "la parada no dice por qué: {detail}"
    );
    assert!(
        serde_json::from_str::<Value>(&permission_reason(&h.runtime))?["permission"]
            == "accessibility",
        "{}",
        permission_reason(&h.runtime)
    );
    Ok(())
}

/// A5: opening or starting another conversation, from the app or from Deimos,
/// leaves the engine connected, and its catalogue is not asked for again.
#[tokio::test]
async fn ref11_a5_another_conversation_leaves_the_engine_and_its_catalogue_alone() -> Result<()> {
    let lists = Arc::new(AtomicUsize::new(0));
    let mut h = Mac::with_engine(FakeEngine {
        tools: Value::Array(tools_named(&[
            "list_apps",
            "list_windows",
            "get_browser_state",
        ])),
        record: EngineRecord {
            catalogue_reads: lists.clone(),
            ..EngineRecord::default()
        },
        timeout: Duration::from_secs(5),
        ..FakeEngine::default()
    })
    .await?;
    grant(
        &mut h.runtime,
        PermissionState::Granted,
        PermissionState::Granted,
    );
    assert_eq!(lists.load(Ordering::SeqCst), 1);
    let epoch = h
        .runtime
        .computer_connection_epoch()
        .map(str::to_owned)
        .context("motor conectado")?;

    h.runtime.open_session("ref11-otra".into()).await?;
    h.runtime.command(Command::NewSession).await?;
    // Deimos opening one: the hub announces the session it started.
    h.runtime
        .apply(Event::Session("ref11-deimos".into()))
        .await?;
    h.runtime.maintain_computer().await?;

    assert!(
        h.runtime.engine_is_ready(),
        "cambiar de conversación paró el motor"
    );
    assert_eq!(
        h.runtime.computer_connection_epoch(),
        Some(epoch.as_str()),
        "cambiar de conversación levantó otro motor"
    );
    assert_eq!(
        lists.load(Ordering::SeqCst),
        1,
        "el catálogo se volvió a pedir"
    );
    h.runtime
        .revoke_computer(&orbit_core::refusal::Refusal::ReadInterrupted {
            cause: orbit_core::refusal::ReadInterruption::ComputerRevoked,
        })
        .await;
    Ok(())
}

/// A7: the privacy stop still cuts an action in flight, exactly as before.
#[tokio::test]
async fn ref11_a7_the_privacy_stop_still_cuts_an_action_in_flight() -> Result<()> {
    let mut h = Mac::new(true).await?;
    grant(
        &mut h.runtime,
        PermissionState::Granted,
        PermissionState::Granted,
    );
    h.runtime.call_engine_tool(
        orbit_core::computer_control::ToolCall {
            tool: "launch_app".into(),
            arguments: json!({"bundle_id": "com.apple.calculator"}),
        },
        "local",
    )?;
    tokio::time::timeout(Duration::from_secs(1), async {
        while h.calls("launch_app") == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await?;

    h.runtime
        .native_event(json!({"event": "privacy_stop", "privacy_stopped": true}))
        .await?;
    assert!(
        h.runtime.features.computer.work.active().is_none(),
        "la acción siguió en marcha"
    );
    assert!(!h.runtime.engine_is_ready(), "el motor siguió conectado");
    assert!(
        h.runtime
            .control_history()?
            .iter()
            .all(|entry| entry.outcome != ControlOutcome::Performed),
        "una acción cortada quedó como hecha"
    );
    let refused = h.runtime.call_engine_tool(
        orbit_core::computer_control::ToolCall {
            tool: "launch_app".into(),
            arguments: json!({"bundle_id": "com.apple.calculator"}),
        },
        "local",
    );
    assert!(
        refused.is_err(),
        "con la parada echada se aceptó otra acción"
    );
    assert_eq!(h.calls("launch_app"), 1, "se repitió la acción");
    Ok(())
}

/// Review (b): with no helper connected there is nobody to report the
/// permissions, and none will arrive by waiting. That is its own cause, with
/// its own code, and it does not tell Deimos to ask again in a few seconds.
/// Reached for real: a rearm without a helper rearms locally.
#[tokio::test]
async fn ref11_review_b_without_a_helper_the_refusal_says_so_and_promises_nothing() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    runtime.settings.device_id = Some("mac-fixture".into());
    {
        let fixture_value = serde_json::json!(orbit_core::Capabilities::default());
        crate::daemon::authority_fixture::set(&mut runtime.state, "capabilities", fixture_value);
    }
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(&mut runtime.state, "privacy_stopped", fixture_value);
    }
    runtime.command(Command::Rearm).await?;
    assert!(
        !runtime.state.privacy_stopped(),
        "el rearme sin helper no rearmó"
    );
    let (hub, mut replies) = mpsc::channel(4);
    runtime.hub.install(hub);
    runtime
        .device_request_raw(&json!({"request_id": "ref11-b", "capability": "perform_action",
            "parameters": {"action": {"tool": "launch_app", "arguments": {"bundle_id": "com.apple.calculator"}}}}))
        .await?;
    let reply = replies.recv().await.context("sin respuesta")?;
    assert_eq!(
        reply["params"]["refusal"]["code"], "helper_not_running",
        "{reply}"
    );
    assert_eq!(
        reply["params"]["refusal"]["retry"]["kind"], "after_fix",
        "a missing helper cannot be fixed by waiting: {reply}"
    );

    // With the helper connected and its answer not in yet, the cause is the
    // other one, and asking again in a moment is the right advice.
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.native_helper",
            fixture_value,
        );
    }
    runtime
        .device_request_raw(&json!({"request_id": "ref11-b2", "capability": "perform_action",
            "parameters": {"action": {"tool": "launch_app", "arguments": {"bundle_id": "com.apple.calculator"}}}}))
        .await?;
    let reply = replies.recv().await.context("sin respuesta")?;
    assert_eq!(
        reply["params"]["refusal"]["code"], "permission_unknown",
        "{reply}"
    );
    assert_eq!(
        reply["params"]["refusal"]["permission"], "screen_and_accessibility",
        "{reply}"
    );
    Ok(())
}

/// Review (c): a status the helper has not answered is not asked again, so an
/// unanswered poll never piles up in the pending list, where evicting entries
/// would strip a real order's answer of its name.
#[tokio::test]
async fn ref11_review_c_an_unanswered_status_is_not_asked_again() -> Result<()> {
    let mut h = Mac::new(false).await?;
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(
            &mut h.runtime.state,
            "capabilities.native_helper",
            fixture_value,
        );
    }
    grant(
        &mut h.runtime,
        PermissionState::Granted,
        PermissionState::Granted,
    );
    for _ in 0..5 {
        h.runtime.features.computer.next_permission_poll = None;
        h.runtime.poll_permissions();
    }
    let mut asked = 0;
    while let Ok(command) = h._native.try_recv() {
        asked += usize::from(command["command"] == "status");
    }
    assert_eq!(asked, 1, "se preguntó otra vez sin respuesta");
    let pending = h
        .runtime
        .pending_native
        .values()
        .filter(|name| name.as_str() == "status")
        .count();
    assert_eq!(pending, 1, "cada sondeo sin respuesta dejó su entrada");

    // An answer two polls late is not coming: that one question is given up —
    // its entry removed — and asked again, so a helper that was replaced and
    // never saw it does not stop the polling for good.
    let (id, _) = h
        .runtime
        .features
        .computer
        .permission_poll
        .clone()
        .context("sondeo pendiente")?;
    let long_ago = std::time::Instant::now()
        .checked_sub(Duration::from_secs(11))
        .context("reloj monótono demasiado joven")?;
    h.runtime.features.computer.permission_poll = Some((id.clone(), long_ago));
    h.runtime.features.computer.next_permission_poll = None;
    h.runtime.poll_permissions();
    assert!(
        !h.runtime.pending_native.contains_key(&id),
        "la pregunta perdida sigue pendiente"
    );
    let pending = h
        .runtime
        .pending_native
        .values()
        .filter(|name| name.as_str() == "status")
        .count();
    assert_eq!(pending, 1);
    assert_eq!(
        h._native
            .try_recv()
            .ok()
            .map(|command| command["command"].clone()),
        Some(json!("status")),
        "no se volvió a preguntar"
    );
    h.runtime
        .revoke_computer(&orbit_core::refusal::Refusal::ReadInterrupted {
            cause: orbit_core::refusal::ReadInterruption::ComputerRevoked,
        })
        .await;
    Ok(())
}

/// Review (e), kept on purpose: nothing on the engine belongs to the app's
/// conversation. A remote action comes from Deimos with no session at all
/// (`bridge_route.py`, `device.request`: request_id, capability, parameters),
/// possibly from another channel. So another conversation retires none of it: an
/// action in flight finishes, its receipt is recorded as performed, and the
/// engine stays up. Retiring it used to abort the worker, which killed the
/// engine and filed an action that may have run as «cancelled».
#[tokio::test]
async fn ref11_review_e_another_conversation_does_not_cut_an_action_in_flight() -> Result<()> {
    let mut h = Mac::new(false).await?;
    h.runtime.call_engine_tool(
        orbit_core::computer_control::ToolCall {
            tool: "launch_app".into(),
            arguments: json!({"bundle_id": "com.apple.calculator"}),
        },
        "local",
    )?;
    h.runtime.open_session("ref11-otra".into()).await?;
    h.runtime.command(Command::NewSession).await?;
    h.finish().await?;
    assert_eq!(h.calls("launch_app"), 1);
    assert!(
        h.runtime.engine_is_ready(),
        "el motor cayó con la conversación"
    );
    assert_eq!(
        h.runtime
            .control_history()?
            .first()
            .map(|entry| entry.outcome),
        Some(ControlOutcome::Performed),
        "una acción hecha quedó archivada como otra cosa"
    );
    h.runtime
        .revoke_computer(&orbit_core::refusal::Refusal::ReadInterrupted {
            cause: orbit_core::refusal::ReadInterruption::ComputerRevoked,
        })
        .await;
    Ok(())
}
