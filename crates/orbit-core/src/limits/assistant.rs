//! The assistant and its chat: a conversation's identifier, a turn's files,
//! the agents a hub offers and the questions asked during a meeting.
use super::Limit;

// The daemon already admitted 121-byte session IDs both from the hub list and
// from local SessionOpen. Keeping that published size protects compatibility
// with older session listings; its original choice has no recorded measurement.
// This bounds a conversation identifier, not an IPC frame, a path, or an audio
// chunk ID (which happens to have a separate 121-byte bound).
pub const MAX_SESSION_ID_BYTES: usize = 121;
// One attached file, the hub's `MAX_ATTACHMENT_BYTES`. What it
// protects on the Mac: the daemon's memory, since it reads a file whole by its
// path before uploading it and holds one whole when the chat's viewer asks
// for it. The hub refuses past it too; this copy lets the Mac refuse before
// reading. Why 20 MiB, and what it does not limit, is written in
// server/orbit_hub/attachments.py.
pub const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;

pub(super) fn limits() -> Vec<Limit> {
    vec![
        // The agents one hub offers, which every Mac's status topic carries;
        // the hub refuses to start or to add one past it
        // (server/orbit_hub/assistant/roster.py).
        Limit {
            name: "MAX_AGENTS",
            value: crate::agents::MAX_AGENTS as u64,
            unit: "agents",
            resource: "agents one hub offers and the Mac takes",
        },
        // Deimos during a meeting. The pause and the longest spoken
        // question are decided in the helper; Swift mirrors them
        // (`NativeLimits`, compared by the constants guard).
        Limit {
            name: "MAX_MEETING_QUESTION_BYTES",
            value: crate::meeting_question::MAX_QUESTION_BYTES as u64,
            unit: "bytes",
            resource: "one question to Deimos during a meeting",
        },
        Limit {
            name: "MEETING_DEFAULT_WINDOW_MINUTES",
            value: crate::meeting_question::DEFAULT_WINDOW_MINUTES as u64,
            unit: "minutes",
            resource: "transcript sent with a meeting question by default",
        },
        Limit {
            name: "MEETING_ANSWER_SHOWN_MS",
            value: crate::meeting_question::ANSWER_SHOWN_MS as u64,
            unit: "milliseconds",
            resource: "an answer in the floating panel",
        },
        Limit {
            name: "MEETING_FAILURE_SHOWN_MS",
            value: crate::meeting_question::FAILURE_SHOWN_MS as u64,
            unit: "milliseconds",
            resource: "a failed question in the floating panel",
        },
        Limit {
            name: "MEETING_QUESTION_PAUSE_MS",
            value: crate::meeting_question::QUESTION_PAUSE_MS,
            unit: "milliseconds",
            resource: "silence that ends a spoken question",
        },
        Limit {
            name: "MAX_SPOKEN_QUESTION_MS",
            value: crate::meeting_question::MAX_SPOKEN_QUESTION_MS,
            unit: "milliseconds",
            resource: "one spoken question from the phrase to its end",
        },
        Limit {
            name: "MAX_VOICE_TRIGGER_DETAIL_BYTES",
            value: crate::meeting_question::MAX_VOICE_TRIGGER_DETAIL_BYTES as u64,
            unit: "bytes",
            resource: "the helper's detail about the «Oye Deimos» detector",
        },
    ]
}
