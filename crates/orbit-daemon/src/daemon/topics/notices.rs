use super::{ProjectTopic, TopicSource};
use orbit_core::{
    refusal::{Refusal, Resource},
    topics::{Knowledge, TopicData, TopicId, notices::Value},
};

pub(super) struct Projector;

impl ProjectTopic for Projector {
    fn project(&self, source: &TopicSource<'_>) -> TopicData {
        let data = TopicData::Notices(Knowledge::Present {
            value: Value {
                notice: source.state.durable.notice.clone(),
                notices: source.state.durable.notice_log.records().to_vec(),
            },
            as_of_ms: None,
        });
        match serde_json::to_vec(&data) {
            Ok(bytes) if bytes.len() <= TopicId::Notices.max_bytes() => data,
            Ok(bytes) => TopicData::absent(
                TopicId::Notices,
                Refusal::TooLarge {
                    resource: Resource::IpcResponse,
                    bytes: bytes.len() as u64,
                    max_bytes: TopicId::Notices.max_bytes() as u64,
                },
            ),
            Err(_error) => TopicData::absent(
                TopicId::Notices,
                Refusal::EncodeFailed {
                    resource: orbit_core::refusal::Resource::IpcResponse,
                },
            ),
        }
    }
}
