use super::*;

#[test]
fn oversized_page_is_absent_before_any_transport_receives_it() -> Result<()> {
    use orbit_core::{
        reads::ReadId,
        refusal::{Refusal, Resource},
        topics::{Knowledge, Page, TOPIC_ENVELOPE_BYTES},
    };
    let answer = Knowledge::Present {
        value: Page {
            items: vec![serde_json::json!({"text": "\\".repeat(650 * 1024)})],
            next: None,
            total: 1,
            revision: 3,
            store_version: None,
        },
        as_of_ms: None,
    };
    let original = TopicResponse {
        v: 1,
        id: "page".into(),
        ok: true,
        revisions: BTreeMap::new(),
        read: Some(ReadId::NotesPage),
        answer: Some(answer.clone()),
        refusal: None,
    };
    let bytes = serde_json::to_vec(&original)?.len() + 1;
    let response = topic_response(
        "page".into(),
        BTreeMap::new(),
        Some(ReadId::NotesPage),
        Some(answer),
    );
    assert!(
        response.answer
            == Some(Knowledge::Absent {
                refusal: Refusal::TooLarge {
                    resource: Resource::IpcResponse,
                    bytes: bytes as u64,
                    max_bytes: (ReadId::NotesPage.max_bytes() + TOPIC_ENVELOPE_BYTES) as u64,
                }
            })
    );
    Ok(())
}
