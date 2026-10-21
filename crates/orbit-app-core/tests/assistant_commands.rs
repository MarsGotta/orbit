//! Internal daemon orders are refused by name before reaching the channel.
use orbit_app_core::{AppCommand, DaemonClient};
use serde_json::json;
#[path = "support/fake_daemon.rs"]
mod fake_daemon;

#[tokio::test]
async fn internal_orders_are_refused_by_name_before_ipc() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let paths = orbit_ipc::Paths::resolve(Some(directory.path().to_path_buf()))?;
    let client = DaemonClient::connect(paths, fake_daemon::idle());
    for value in [
        json!({"command":"status"}),
        json!({"command":"subscribe_topics","topics":[]}),
        json!({"command":"read","read":{"kind":"notes_page","cursor":null}}),
        json!({"command":"report","report":{"op":"op-interface-0002","code":"ui_render_failed","class":"failed","source":"ui"}}),
        json!({"command":"shutdown"}),
    ] {
        let command = serde_json::from_value::<AppCommand>(value.clone())?;
        let error = client
            .send(command)
            .await
            .err()
            .ok_or("internal order was accepted")?;
        assert_eq!(error.code, "internal_command");
        assert!(
            error
                .message
                .contains(value["command"].as_str().ok_or("command name")?)
        );
    }
    Ok(())
}
