#![allow(clippy::expect_used, clippy::unwrap_used)]
use super::*;

#[test]
fn session_deadline_is_guarded_against_the_ipc_read_window() {
    assert!(SESSION_READ_DEADLINE < orbit_ipc::KEYED_READ_TIMEOUT);
}

/// The hub's whole page of conversations, 128 rows with
/// canonical ids, the longest agent id the hub admits (32 characters)
/// and the longest numbers, fits the `Sessions` read.
#[test]
fn a_full_sessions_page_fits_its_read() {
    let rows = (0..128)
        .map(|index| orbit_core::SessionSummary {
            id: format!("00000000-0000-4000-8000-{index:012}"),
            agent_id: Some("a".repeat(32)),
            created_at_ms: i64::MAX,
            last_at_ms: i64::MAX,
            message_count: u64::MAX,
        })
        .collect::<Vec<_>>();
    let next = Some(Cursor::Id {
        id: rows[127].id.clone(),
    });
    let answer = bounded_page(&rows, next, u64::MAX, u64::MAX, ReadId::Sessions);
    let bytes = serde_json::to_vec(&answer).unwrap().len();
    eprintln!("a full sessions page is {bytes} bytes");
    assert!(matches!(answer, Knowledge::Present { .. }), "{answer:?}");
    assert!(bytes <= ReadId::Sessions.max_bytes());
}

/// A hub page at its 512 KiB ceiling of JSON, in characters that
/// escape the same on both sides, still fits the `SessionMessages` read
/// with the ids and `audio_available` the daemon adds.
#[test]
fn a_full_messages_page_fits_its_read() {
    let text = "\"".repeat(10 * 1024);
    let message = |index: usize| orbit_core::SessionMessage {
        attachments: Vec::new(),
        id: format!("00000000-0000-4000-8000-{index:012}"),
        role: "user".into(),
        text: text.clone(),
        at_ms: Some(i64::MAX),
        voice: Some(orbit_core::SessionVoice {
            duration_ms: u64::MAX,
            audio_available: true,
        }),
        // An answer's summary passes through as the hub wrote it: the
        // same bytes on both sides, so it changes nothing here.
        work: None,
    };
    // As many as the hub's page holds: 512 KiB less its 80-byte envelope.
    let mut messages = Vec::new();
    while serde_json::to_vec(&[&messages[..], &[message(messages.len())]].concat())
        .unwrap()
        .len()
        <= 512 * 1024 - 80
    {
        messages.push(message(messages.len()));
    }
    let hub_bytes = serde_json::to_vec(&messages).unwrap().len();
    assert!(hub_bytes >= 500 * 1024, "{hub_bytes}");
    let next = Some(Cursor::Id {
        id: i64::MAX.to_string(),
    });
    let answer = bounded_page(&messages, next, u64::MAX, u64::MAX, ReadId::SessionMessages);
    let bytes = serde_json::to_vec(&answer).unwrap().len();
    eprintln!("a full messages page is {bytes} bytes ({hub_bytes} from the hub)");
    assert!(matches!(answer, Knowledge::Present { .. }));
    assert!(bytes <= ReadId::SessionMessages.max_bytes());
}
