use crate::daemon_thread::EmbeddedDaemon;
use orbit_app_core::{
    AppBootstrap, AppCommand, AppError, AppSnapshot, CommandError, CommandReceipt, DaemonClient,
    LoginItemState, SettingsUpdateState, UpdateService, UpdateSnapshot,
};
use orbit_app_core::{ChatMessage, ChatSessionSummary};
use orbit_core::{
    refusal::{Refusal, UpdateCause},
    topics::{Cursor, Knowledge, Page},
};
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_opener::OpenerExt;

#[tauri::command]
pub fn orbit_snapshot(client: State<'_, DaemonClient>) -> AppBootstrap {
    client.bootstrap()
}

#[tauri::command]
pub async fn orbit_read_sessions_page(
    cursor: Option<Cursor>,
    client: State<'_, DaemonClient>,
) -> Result<Knowledge<Page<ChatSessionSummary>>, AppError> {
    client.read_sessions_page(cursor).await
}

/// One page of a conversation, read from the hub by the daemon.
#[tauri::command]
pub async fn orbit_read_session_messages(
    session_id: String,
    cursor: Option<Cursor>,
    client: State<'_, DaemonClient>,
) -> Result<Knowledge<Page<ChatMessage>>, AppError> {
    client.read_session_messages_page(session_id, cursor).await
}

/// Ajustes › Agentes: one read or order of an agent's configuration on the
/// hub, through the daemon, and what came of it.
#[tauri::command]
pub async fn orbit_agent(
    call: orbit_core::agent_config::AgentCall,
    client: State<'_, DaemonClient>,
) -> Result<orbit_app_core::AgentAnswer, AppError> {
    client.agent(call).await
}

pub use orbit_app_core::PanelOrder;

/// The floating panel's orders: answering a call, asking Deimos, and
/// answering what Deimos asks back.
#[tauri::command]
pub async fn orbit_panel_send(
    command: PanelOrder,
    op: Option<String>,
    app: AppHandle,
    client: State<'_, DaemonClient>,
) -> Result<CommandReceipt, CommandError> {
    let op = order_op(op);
    let command = match command {
        PanelOrder::PanelFocus { focus } => {
            crate::panel::focus(&app, focus);
            return Ok(client.local_receipt());
        }
        PanelOrder::OpenUrl { url } => {
            return open_url(&app, &client, &url)
                .map(|_| client.local_receipt())
                .map_err(|error| {
                    CommandError::from_app_error(
                        AppCommand::Status(
                            orbit_app_core::features::status::StatusCommand::OpenUrl { url },
                        ),
                        error,
                    )
                });
        }
        PanelOrder::CallRecord { call_id } => {
            AppCommand::Daemon(Box::new(orbit_core::protocol::Command::CallRecord {
                call_id,
            }))
        }
        PanelOrder::CallDismiss { call_id } => {
            AppCommand::Daemon(Box::new(orbit_core::protocol::Command::CallDismiss {
                call_id,
            }))
        }
        PanelOrder::RecordingKeep { recording_id } => {
            AppCommand::Daemon(Box::new(orbit_core::protocol::Command::RecordingKeep {
                recording_id,
            }))
        }
        PanelOrder::MeetingFinish => {
            AppCommand::Daemon(Box::new(orbit_core::protocol::Command::MeetingFinish))
        }
        PanelOrder::MeetingAsk { question, minutes } => {
            AppCommand::Daemon(Box::new(orbit_core::protocol::Command::MeetingAsk {
                question,
                minutes,
            }))
        }
        PanelOrder::MeetingQuestionClose { id } => AppCommand::Daemon(Box::new(
            orbit_core::protocol::Command::MeetingQuestionClose { id },
        )),
        PanelOrder::Approval { approved, choice } => {
            AppCommand::Daemon(Box::new(orbit_core::protocol::Command::Approval {
                approved,
                choice,
            }))
        }
        PanelOrder::Clarify {
            answer,
            question_id,
        } => AppCommand::Daemon(Box::new(orbit_core::protocol::Command::Clarify {
            answer,
            question_id,
        })),
    };
    let sent = command.clone();
    client
        .send_as(command, op)
        .await
        .map_err(|error| CommandError::from_app_error(sent, error))
}

/// The app's local report log, in the daemon's data directory, and
/// the typed endpoint of the daemon in this process.
pub struct AppReportLog {
    pub log: orbit_ipc::report::ReportLog,
    pub client: orbit_ipc::LocalClient,
}

/// The app's one report function: what the interface met (a refusal, a code
/// it has no entry for, a view that failed to paint, an uncaught error) goes
/// to `logs/report-app.log`. The type admits no text: a report with a
/// sentence in it does not decode and is not written.
///
/// The same line then goes to the daemon, whose one error-report
/// client decides whether it leaves for Sentry; the app has no client of its
/// own. A daemon that does not answer loses only that copy: the line is in
/// the app's log already.
#[tauri::command]
pub fn orbit_report(report: orbit_core::notice::Report, log: State<'_, AppReportLog>) {
    log.log.report(&report);
    let client = log.client.clone();
    tauri::async_runtime::spawn(async move {
        let _ = forward_report(&client, report).await;
    });
}

/// Hands one report line to the daemon, under the line's own op.
async fn forward_report(
    client: &orbit_ipc::LocalClient,
    report: orbit_core::notice::Report,
) -> anyhow::Result<()> {
    let op = report.op.clone();
    client
        .request_command_as(orbit_core::protocol::Command::Report { report }, op)
        .await
        .map(|_| ())
}

/// The op of an order from the interface: the one it sent, or a new
/// one when it sent none or one that is not an op.
fn order_op(op: Option<String>) -> orbit_core::notice::Op {
    op.as_deref()
        .and_then(orbit_core::notice::Op::parse)
        .unwrap_or_else(orbit_ipc::new_op)
}

/// The daemon in this process, for the composer's files. Their
/// bytes never cross the interface except the ones it already holds (a
/// pasted image or text) and the ones the chat's viewer shows.
pub struct Attachments {
    pub client: orbit_ipc::LocalClient,
}

/// A refused attachment order, as the interface reads every refusal
/// (`refusalOf` in `src/bridge/refusal.ts`).
#[derive(Debug, serde::Serialize)]
pub struct AttachmentRefusal {
    refusal: Refusal,
}

impl From<anyhow::Error> for AttachmentRefusal {
    fn from(error: anyhow::Error) -> Self {
        let refusal = error
            .chain()
            .find_map(|cause| cause.downcast_ref::<Refusal>().cloned())
            .unwrap_or_else(|| Refusal::Internal {
                cause: orbit_core::refusal::InternalCause::AppOperation {
                    code: "attachment".into(),
                    error: error.to_string(),
                },
            });
        Self { refusal }
    }
}

/// «Adjuntar», a dropped file, a pasted image or text, or «Adjuntar
/// ventana»: the daemon reads or captures it and uploads it to the hub; the
/// answer is what the hub keeps and, for a window, what became of its text.
#[tauri::command]
pub async fn orbit_attachment_add(
    source: orbit_core::attachments::AttachmentSource,
    attachments: State<'_, Attachments>,
) -> Result<orbit_core::attachments::AddedAttachment, AttachmentRefusal> {
    let response = attachments
        .client
        .attachment(orbit_core::protocol::Command::AttachmentAdd { source })
        .await?;
    let attachment = response
        .attachment
        .ok_or_else(|| anyhow::anyhow!("El daemon no entregó el adjunto"))?;
    Ok(orbit_core::attachments::AddedAttachment {
        attachment,
        window_text: response.window_text,
    })
}

/// One attachment's bytes for the chat's viewer, as raw bytes (an
/// `ArrayBuffer` in the interface), not JSON.
#[tauri::command]
pub async fn orbit_attachment_content(
    attachment_id: String,
    attachments: State<'_, Attachments>,
) -> Result<tauri::ipc::Response, AttachmentRefusal> {
    let content = attachments.client.attachment_content(attachment_id).await?;
    Ok(tauri::ipc::Response::new(content.bytes))
}

/// «Abrir» (with the Mac's app for its type) or «Mostrar en Finder»: the
/// daemon writes the attachment to its folder and macOS opens or reveals it.
#[tauri::command]
pub async fn orbit_attachment_open(
    attachment_id: String,
    reveal: bool,
    app: AppHandle,
    attachments: State<'_, Attachments>,
) -> Result<(), AttachmentRefusal> {
    let response = attachments
        .client
        .attachment(orbit_core::protocol::Command::AttachmentSave { attachment_id })
        .await?;
    let path = response
        .path
        .ok_or_else(|| anyhow::anyhow!("El daemon no escribió el adjunto"))?;
    let opened = if reveal {
        app.opener().reveal_item_in_dir(&path)
    } else {
        app.opener().open_path(&path, None::<&str>)
    };
    opened.map_err(|error| anyhow::anyhow!("macOS no pudo abrir el adjunto: {error}").into())
}

#[tauri::command(rename = "orbit_send")]
pub async fn orbit_send_desktop(
    command: AppCommand,
    op: Option<String>,
    app: AppHandle,
    client: State<'_, DaemonClient>,
    daemon: State<'_, EmbeddedDaemon>,
    updater: State<'_, UpdateService>,
) -> Result<CommandReceipt, CommandError> {
    let op = order_op(op);
    let sent = command.clone();
    let result = match command {
        AppCommand::Permissions(
            orbit_app_core::features::permissions::PermissionsCommand::OpenSystemPrivacy { pane },
        ) => open_system_privacy(&app, &client, &pane).map(|_| client.local_receipt()),
        AppCommand::Status(orbit_app_core::features::status::StatusCommand::OpenUrl { url }) => {
            open_url(&app, &client, &url).map(|_| client.local_receipt())
        }
        AppCommand::Local(orbit_app_core::features::local::LocalCommand::LoginItem { on }) => {
            set_login_item(&app, &client, on).map(|_| client.local_receipt())
        }
        AppCommand::Local(orbit_app_core::features::local::LocalCommand::UpdateCheck) => {
            check_update(&client, &updater)
                .await
                .map(|_| client.local_receipt())
        }
        AppCommand::Local(orbit_app_core::features::local::LocalCommand::UpdateInstall) => {
            install_update(&app, &client, &daemon, &updater)
                .await
                .map(|_| client.local_receipt())
        }
        AppCommand::Status(orbit_app_core::features::status::StatusCommand::Quit) => {
            quit(app, daemon.inner().clone());
            Ok(client.local_receipt())
        }
        command => client.send_as(command, op).await,
    };
    result.map_err(|error| CommandError::from_app_error(sent, error))
}

/// Why a link is not opened. Each says its cause: the link may come from
/// Deimos's answer, which what was said in a meeting can steer, and a
/// Markdown label can hide where it leads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebUrlRefusal {
    /// Not a URL at all.
    Unparsable { reason: String },
    /// Only http and https leave Orbit for the browser.
    Scheme { scheme: String },
    /// A user or password before the host (`https://acme.com@malo.example`
    /// leads to `malo.example`).
    Credentials,
    /// No host to go to.
    NoHost,
}

impl std::fmt::Display for WebUrlRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unparsable { reason } => write!(f, "no es una dirección web válida ({reason})"),
            Self::Scheme { scheme } => {
                write!(f, "sólo se abren enlaces http y https; este es «{scheme}:»")
            }
            Self::Credentials => f.write_str(
                "lleva usuario o contraseña antes del sitio, que puede ocultar adónde lleva",
            ),
            Self::NoHost => f.write_str("no dice a qué sitio lleva"),
        }
    }
}

/// Only web links leave Orbit for the browser: from the main window and
/// from the floating panel's answers. Parsed, never matched by
/// prefix; the parsed form is what opens.
pub(crate) fn web_url(url: &str) -> Result<tauri::Url, WebUrlRefusal> {
    let parsed = tauri::Url::parse(url).map_err(|error| WebUrlRefusal::Unparsable {
        reason: error.to_string(),
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(WebUrlRefusal::Scheme {
            scheme: parsed.scheme().to_owned(),
        });
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(WebUrlRefusal::Credentials);
    }
    if parsed.host_str().is_none_or(str::is_empty) {
        return Err(WebUrlRefusal::NoHost);
    }
    Ok(parsed)
}

fn open_url(app: &AppHandle, _client: &DaemonClient, url: &str) -> Result<(), AppError> {
    let url = web_url(url).map_err(|refusal| AppError {
        code: "link_refused".into(),
        message: format!("No se abre el enlace: {refusal}."),
        refusal: None,
    })?;
    app.opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|error| AppError::daemon(error.to_string()))?;
    Ok(())
}

fn open_system_privacy(
    app: &AppHandle,
    _client: &DaemonClient,
    pane: &str,
) -> Result<(), AppError> {
    let pane = match pane {
        "Microphone" | "ScreenCapture" | "Accessibility" | "Calendars" => pane,
        _ => return Err(AppError::desktop_only()),
    };
    let url = format!("x-apple.systempreferences:com.apple.preference.security?Privacy_{pane}");
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|error| AppError::daemon(error.to_string()))?;
    Ok(())
}

fn set_login_item(
    app: &AppHandle,
    client: &DaemonClient,
    on: bool,
) -> Result<AppSnapshot, AppError> {
    let manager = app.autolaunch();
    let result = if on {
        manager.enable()
    } else {
        manager.disable()
    };
    result.map_err(|error| AppError::daemon(error.to_string()))?;
    Ok(client.set_login_item(if on {
        LoginItemState::On
    } else {
        LoginItemState::Off
    }))
}

async fn check_update(
    client: &DaemonClient,
    updater: &UpdateService,
) -> Result<AppSnapshot, AppError> {
    if client.composed_snapshot().settings.update.state == SettingsUpdateState::Installing {
        return Ok(client.composed_snapshot());
    }
    client.set_update(UpdateSnapshot {
        state: SettingsUpdateState::Checking,
        version: None,
        installed_version: None,
        message: None,
        refusal: None,
        last_installation: None,
        install_blocked: None,
    });
    Ok(client.set_update_result(updater.check_configured().await))
}

pub(crate) async fn install_update(
    app: &AppHandle,
    client: &DaemonClient,
    daemon: &EmbeddedDaemon,
    updater: &UpdateService,
) -> Result<AppSnapshot, AppError> {
    let version = client.update_version();
    match updater.install_when_idle(client).await {
        Ok(Some(_)) => {
            quit(app.clone(), daemon.clone());
            Ok(client.composed_snapshot())
        }
        Ok(None) => Ok(client.composed_snapshot()),
        Err(refusal) => {
            let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
            Ok(client.set_update(failed_install_snapshot(version, refusal, home.as_deref())))
        }
    }
}

fn failed_install_snapshot(
    version: Option<String>,
    refusal: Refusal,
    home: Option<&std::path::Path>,
) -> UpdateSnapshot {
    let mut snapshot = UpdateSnapshot {
        state: SettingsUpdateState::Failed,
        version,
        installed_version: None,
        message: None,
        refusal: Some(refusal),
        last_installation: None,
        install_blocked: None,
    };
    if matches!(
        snapshot.refusal.as_ref(),
        Some(Refusal::Update {
            cause: UpdateCause::InstallerFailed { .. }
        })
    ) && let Some(home) = home
    {
        match UpdateService::last_installation_at(home) {
            Ok(Some(last)) => snapshot.last_installation = Some(last),
            Err(log_refusal) => snapshot.refusal = Some(log_refusal),
            Ok(None) => {}
        }
    }
    snapshot
}

/// Every quit Orbit orders itself: the window hands over what it still
/// holds (`interface_flush.rs`), then the daemon closes in order, then the
/// process ends.
pub(crate) fn quit(app: AppHandle, daemon: EmbeddedDaemon) {
    tauri::async_runtime::spawn(async move {
        let exit = app.clone();
        crate::interface_flush::flush_then(
            &app,
            crate::interface_flush::INTERFACE_FLUSH_BOUND,
            async move {
                daemon.shutdown().await;
                exit.exit(0);
            },
        )
        .await;
    });
}

#[cfg(test)]
mod update_failure_tests {
    use super::*;
    use std::fs;

    #[test]
    fn failed_installer_shows_logged_cause_and_path_in_settings() -> anyhow::Result<()> {
        let home = tempfile::tempdir()?;
        let log = home.path().join("Library/Logs/Orbit/instalador.log");
        fs::create_dir_all(
            log.parent()
                .ok_or_else(|| anyhow::anyhow!("log has no parent"))?,
        )?;
        fs::write(
            &log,
            "Inicio 2026-10-01T10:00:00Z\nresultado|0.3.14|2026-10-01T10:00:01Z|fallo|curl: estado 23\n",
        )?;
        let refusal = Refusal::Update {
            cause: UpdateCause::InstallerFailed { status: Some(1) },
        };
        let snapshot =
            failed_install_snapshot(Some("0.3.14".into()), refusal.clone(), Some(home.path()));
        let last = snapshot
            .last_installation
            .ok_or_else(|| anyhow::anyhow!("missing last result"))?;
        assert_eq!(last.failure.as_deref(), Some("curl: estado 23"));
        assert_eq!(
            last.log_path.as_deref(),
            Some(
                log.to_str()
                    .ok_or_else(|| anyhow::anyhow!("non-UTF8 log"))?
            )
        );
        assert_eq!(snapshot.refusal, Some(refusal));

        fs::write(&log, "Inicio 2026-10-01T11:00:00Z\n")?;
        let missing = failed_install_snapshot(
            None,
            Refusal::Update {
                cause: UpdateCause::InstallerFailed { status: Some(7) },
            },
            Some(home.path()),
        );
        assert!(
            matches!(missing.refusal, Some(Refusal::Update { cause: UpdateCause::InstallLog { reason: orbit_core::refusal::InstallLogIssue::MissingResult { started_at: Some(at) }, .. } }) if at == "2026-10-01T11:00:00Z")
        );
        Ok(())
    }
}

#[cfg(test)]
mod report_forwarding_tests {
    use super::forward_report;
    use orbit_core::protocol::Command;

    /// What the interface reports reaches the daemon as
    /// `Command::Report`, under the report's op, so the daemon's one client
    /// decides whether it leaves.
    #[tokio::test]
    async fn an_interface_report_reaches_the_daemon_under_its_op() -> anyhow::Result<()> {
        let (sender, mut daemon) = tokio::sync::mpsc::channel(1);
        let (ended, end) = tokio::sync::watch::channel(None);
        let client = orbit_ipc::LocalClient::new(sender, &orbit_ipc::TopicChannels::new(), end);
        let report: orbit_core::notice::Report = serde_json::from_value(serde_json::json!({
            "op": "op-interface-0002", "code": "ui_render_failed",
            "class": "failed", "source": "ui"
        }))?;
        let forwarding = tokio::spawn({
            let client = client.clone();
            let report = report.clone();
            async move { forward_report(&client, report).await }
        });
        let call = daemon
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("the daemon received nothing"))?;
        assert_eq!(
            call.request.op.as_ref().map(|op| op.as_str()),
            Some("op-interface-0002")
        );
        match &call.request.body {
            Command::Report { report: received } => assert_eq!(received, &report),
            other => panic!("{other:?}"),
        }
        drop(ended);
        drop(call);
        let _ = forwarding.await?;
        Ok(())
    }
}
