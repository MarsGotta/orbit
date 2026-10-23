use super::{ProjectTopic, TopicSource};
use orbit_core::{
    topics::TopicData,
    topics::{Knowledge, status::Value},
};

pub(super) struct Projector;

impl ProjectTopic for Projector {
    fn project(&self, source: &TopicSource<'_>) -> TopicData {
        let state = source.state;
        let roster = source.runtime.map(|runtime| runtime.settings.roster());
        TopicData::Status(Knowledge::Present {
            value: Value {
                assistant_name: source
                    .runtime
                    .map(|runtime| runtime.settings.assistant_name().to_owned()),
                agents: roster
                    .as_ref()
                    .map(|roster| roster.agents().to_vec())
                    .unwrap_or_default(),
                default_agent_id: roster
                    .as_ref()
                    .map(|roster| roster.default_agent_id().to_owned()),
                addable_agents: roster
                    .as_ref()
                    .and_then(|roster| roster.addable().map(<[String]>::to_vec)),
                attachments: source
                    .runtime
                    .and_then(|runtime| runtime.assistant.hub_attachments),
                hub: state.connection().clone(),
                account: state.views.account.clone(),
                privacy_stopped: state.privacy_stopped(),
                helper_connected: state.helper_connected(),
                error_reports: source
                    .runtime
                    .and_then(|runtime| runtime.error_reports.choice()),
            },
            as_of_ms: None,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use orbit_core::topics::Knowledge;

    /// The switch is offered only by a build with a Sentry address, and
    /// says the user's choice; turning it changes the topic.
    #[tokio::test]
    async fn the_status_topic_offers_the_error_report_switch_only_with_an_address() {
        let directory = tempfile::tempdir().unwrap();
        let (mut runtime, _helper) =
            crate::daemon::audio::tests::fixture(directory.path()).unwrap();
        let said = |runtime: &crate::daemon::Runtime| {
            let source = TopicSource {
                state: &runtime.state,
                runtime: Some(runtime),
                now_ms: 0,
            };
            match Projector.project(&source) {
                TopicData::Status(Knowledge::Present { value, .. }) => value.error_reports,
                other => panic!("{other:?}"),
            }
        };
        assert_eq!(said(&runtime), None);
        runtime.error_reports =
            crate::error_reports::ErrorReports::start(Some("http://public@127.0.0.1:9/1"), false);
        assert_eq!(said(&runtime), Some(false));
        runtime.error_reports.set_enabled(true);
        assert_eq!(said(&runtime), Some(true));
    }
}
