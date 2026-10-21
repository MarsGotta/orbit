//! Isolated subprocesses speak MCP over stdio. No driver, system census or input.
use super::*;
use crate::cua::LocalCua;

async fn exiting_service(tail: &str, marker: &std::path::Path) -> Result<(ActionService, u32)> {
    let initialize = json!({"jsonrpc":"2.0","id":1,"result":{
        "protocolVersion":"2025-06-18", "serverInfo":{"version":crate::cua::VERIFIED_DRIVER_VERSION}}});
    let tools = json!({"jsonrpc":"2.0","id":2,"result":{"tools":tools_named(&["list_apps", "list_windows", "launch_app"])}});
    let script = format!(
        "read -r line\nprintf '%s\\n' '{initialize}'\nread -r line\nread -r line\nprintf '%s\\n' '{tools}'\n{tail}"
    );
    let child = tokio::process::Command::new("/bin/sh")
        .args(["-c", &script, "sim98-fixture"])
        .arg(marker)
        .env_clear()
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let pid = child.id().context("fixture PID")?;
    let mut client = LocalCua::from_process(child, Duration::from_secs(5))?;
    client.initialize().await?;
    Ok((ActionService::from_client(client.into_owned()), pid))
}

async fn exited(pid: u32) -> Result<()> {
    let pid = rustix::process::Pid::from_raw(i32::try_from(pid)?).context("fixture PID")?;
    tokio::time::timeout(
        Duration::from_secs(60),
        tokio::task::spawn_blocking(move || {
            loop {
                match rustix::process::waitid(
                    rustix::process::WaitId::Pid(pid),
                    rustix::process::WaitIdOptions::EXITED | rustix::process::WaitIdOptions::NOWAIT,
                ) {
                    Err(rustix::io::Errno::INTR) => continue,
                    result => return result.map(|_| ()),
                }
            }
        }),
    )
    .await???;
    Ok(())
}

fn assert_exit_notice(runtime: &Runtime, code: &str, status: i64) -> Result<()> {
    let notice = runtime
        .state
        .durable
        .notice
        .as_ref()
        .context("exit notice")?;
    assert_eq!(notice.code.name(), "engine_exited");
    let data = serde_json::to_value(&notice.data)?;
    assert_eq!(data["cause"], "proxy");
    assert_eq!(data["code"], code);
    assert_eq!(data["status"], status);
    Ok(())
}

#[tokio::test]
async fn sim98_idle_exit_is_said_once_with_status_and_releases_the_session() -> Result<()> {
    for (tail, code, status) in [
        ("exit 0", "exit", 0),
        ("exit 23", "exit", 23),
        ("kill -TERM $$", "signal", 15),
    ] {
        let mut h = Mac::new(false).await?;
        let (service, pid) = exiting_service(tail, h._directory.path()).await?;
        h.runtime.features.computer.service = Some(service);
        exited(pid).await?;
        h.runtime.maintain_engine().await;
        assert_exit_notice(&h.runtime, code, status)?;
        assert!(h.runtime.features.computer.service.is_none());
        assert_eq!(
            h.runtime.features.computer.engine_failure_kind,
            Some(orbit_core::refusal::EngineCause::Unhealthy)
        );
        let records = h.runtime.state.durable.notice_log.records().len();
        h.runtime.maintain_engine().await;
        assert_eq!(h.runtime.state.durable.notice_log.records().len(), records);
        // Recovery goes through the ordinary start, without a real driver.
        h.runtime.features.computer.engine_retry_at = Some(Instant::now());
        h.runtime.maintain_engine().await;
        assert!(
            h.runtime
                .features
                .computer
                .engine_failure
                .as_deref()
                .is_some_and(|cause| cause.contains("no es la app de un bundle de Orbit"))
        );
    }
    Ok(())
}

#[tokio::test]
async fn sim98_exit_during_input_keeps_one_uncertain_receipt_and_never_replays() -> Result<()> {
    let mut h = Mac::new(false).await?;
    let marker = h._directory.path().join("input-count");
    let (service, _) = exiting_service(
        "read -r line\nprintf 'input\\n' >> \"$1\"\nexit 29",
        &marker,
    )
    .await?;
    h.runtime.features.computer.service = Some(service);
    h.runtime.call_engine_tool(
        ToolCall {
            tool: "launch_app".into(),
            arguments: json!({"bundle_id":"fixture"}),
        },
        "fixture",
    )?;
    h.finish().await?;
    assert_exit_notice(&h.runtime, "exit", 29)?;
    let history = h.runtime.control_history()?;
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].outcome, ControlOutcome::Unknown);
    h.runtime.maintain_engine().await;
    assert_eq!(std::fs::read_to_string(marker)?, "input\n");
    assert!(h.runtime.features.computer.service.is_none());
    Ok(())
}

#[tokio::test]
async fn sim98_an_ordered_close_does_not_report_an_unexpected_exit() -> Result<()> {
    let mut h = Mac::new(false).await?;
    let (service, _) = exiting_service("read -r line", h._directory.path()).await?;
    h.runtime.features.computer.service = Some(service);
    h.runtime.closing = true;
    h.runtime.maintain_engine().await;
    assert!(h.runtime.features.computer.service.is_none());
    assert!(
        !h.runtime
            .state
            .durable
            .notice_log
            .records()
            .iter()
            .any(|record| record.code.name() == "engine_exited")
    );
    Ok(())
}
