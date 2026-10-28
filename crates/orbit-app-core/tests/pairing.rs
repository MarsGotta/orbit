use orbit_app_core::{
    AppSnapshot, DaemonConnection,
    features::{FeatureInput, status},
    topics::TopicCache,
};
use orbit_core::{
    AccountSummary, Connection,
    topics::{Knowledge, TopicData, TopicFrame},
};

fn account() -> AccountSummary {
    AccountSummary {
        hub_url: Some("https://hub.example".into()),
        device_name: Some("Sample Mac".into()),
        device_id_short: Some("7f31c1a2".into()),
    }
}

fn status_snapshot(
    account: Option<AccountSummary>,
    hub: Connection,
) -> Result<serde_json::Value, serde_json::Error> {
    let mut topics = TopicCache::default();
    topics.observe_at(
        TopicFrame {
            epoch: "pairing".into(),
            revision: 1,
            data: TopicData::Status(Knowledge::Present {
                value: orbit_core::topics::status::Value {
                    attachments: None,
                    assistant_name: None,
                    agents: Vec::new(),
                    default_agent_id: None,
                    addable_agents: None,
                    hub,
                    account,
                    privacy_stopped: false,
                    helper_connected: true,
                    error_reports: None,
                    language: None,
                },
                as_of_ms: None,
            }),
        },
        Some(1),
    );
    let local = AppSnapshot::empty(1, DaemonConnection::Ready);
    let view = status::project(&FeatureInput::new(&local, &topics));
    serde_json::to_value(view)
}

#[test]
fn projects_saved_pairing_independently_from_link_connection()
-> Result<(), Box<dyn std::error::Error>> {
    let offline = status_snapshot(Some(account()), Connection::Offline)?;
    assert_eq!(offline["settings"]["hub"]["paired"], true);
    assert_eq!(offline["settings"]["hub"]["connection"], "offline");
    assert_eq!(offline["settings"]["hub"]["url"], "https://hub.example");
    assert_eq!(offline["settings"]["hub"]["device_name"], "Sample Mac");

    let without_account = status_snapshot(None, Connection::Online)?;
    assert_eq!(without_account["settings"]["hub"]["paired"], false);
    assert_eq!(without_account["settings"]["hub"]["connection"], "online");

    let revoked = status_snapshot(Some(account()), Connection::Revoked)?;
    assert_eq!(revoked["settings"]["hub"]["paired"], true);
    assert_eq!(revoked["settings"]["hub"]["connection"], "revoked");
    Ok(())
}
