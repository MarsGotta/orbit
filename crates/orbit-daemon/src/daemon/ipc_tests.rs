use super::*;

#[test]
fn abandoned_reply_records_command_once_without_conversation() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let diagnostics = crate::diagnostics::DiagnosticLog::in_root(directory.path());
    let (sender, receiver) = tokio::sync::oneshot::channel::<()>();
    drop(receiver);
    record_ipc_reply_loss(
        sender.send(()),
        &Command::Chat {
            attachments: Vec::new(),
            text: "SECRET_FIXTURE_CONVERSATION".into(),
            primary: false,
        },
        Some(&diagnostics),
    );
    let log = std::fs::read_to_string(diagnostics.path())?;
    assert_eq!(log.matches("ipc_reply_receiver_gone").count(), 1, "{log}");
    assert!(log.contains("\"command\":\"chat\""), "{log}");
    assert!(!log.contains("SECRET_FIXTURE_CONVERSATION"), "{log}");
    Ok(())
}
