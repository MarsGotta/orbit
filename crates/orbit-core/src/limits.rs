//! Shared transport and device bounds. Topic and read page ceilings remain in
//! `topics` and `reads`: their registries name the resource and measured page.

use serde::Serialize;

// The daemon already admitted 121-byte session IDs both from the hub list and
// from local SessionOpen. Keeping that published size protects compatibility
// with older session listings; its original choice has no recorded measurement.
// This bounds a conversation identifier, not an IPC frame, a path, or an audio
// chunk ID (which happens to have a separate 121-byte bound).
pub const MAX_SESSION_ID_BYTES: usize = 121;

// The largest real IPC line is not measured. The existing 1 MiB socket reader
// and byte-exact synthetic boundary tests fix today's allocation ceiling.
// This protects one app IPC line, not a helper command or a hub frame.
pub const MAX_IPC_LINE_BYTES: usize = 1024 * 1024;
// The existing 1 MiB budget bounds a command response, matching the client
// reader. Its largest real payload is not measured; this preserves the
// allocation ceiling, not a command-body or agenda-topic limit.
pub const MAX_COMMAND_RESPONSE_BYTES: usize = 1024 * 1024;
// The largest real default command is not measured. The app IPC reader admits
// up to 1 MiB, so that existing admission bounds a default command, not the
// State or commands with their own tighter limits.
pub const MAX_DEFAULT_COMMAND_BYTES: usize = 1024 * 1024;
// A 4 MiB playback clip becomes 5,592,408 base64 bytes; the measured JSON
// command with a UUID id and 128-byte playback/provider fields is 5,592,815
// bytes including newline, as measured.
// Six MiB protects helper stdin and is not a bound on helper events.
pub const MAX_HELPER_COMMAND_LINE_BYTES: usize = 6 * 1024 * 1024;
// The largest helper event is a screenshot response: a 700,000-byte PNG cap
// expands to 933,336 base64 bytes. Serializing its response envelope with
// maximum-width binding fields measured 934,136 bytes including newline in
// five fixture runs (89.1% of 1 MiB). This existing stdout reader ceiling is
// not stdin's playback-clip budget.
pub const MAX_HELPER_EVENT_LINE_BYTES: usize = 1024 * 1024;
// The largest real inbound WebSocket frame is not measured. Its existing 1 MiB
// receiver gate bounds one incoming hub message, not the outbound 128 KiB frame.
pub const MAX_HUB_INCOMING_FRAME_BYTES: usize = 1024 * 1024;
// The largest real hub HTTP body is not measured. Its existing 1 MiB streaming
// gate bounds one response allocation, not the inbound WebSocket or IPC State.
pub const MAX_HUB_HTTP_BODY_BYTES: usize = 1024 * 1024;
// The hub config admits a 131,072-byte frame. This protects a frame sent to
// the server, not its larger inbound HTTP or WebSocket messages.
pub const MAX_HUB_OUTGOING_FRAME_BYTES: usize = 128 * 1024;
// Existing screen observations and action receipts were both cut at 32 KiB.
// This separate receipt bound protects an engine result in the republished
// State; it is not a screen observation or a limit on the engine catalogue.
pub const MAX_ACTION_RECEIPT_BYTES: usize = 32 * 1024;

/// Checks one engine receipt before it enters the repeatedly command response.
/// The caller supplies the fact to check; this makes no I/O or retry decision.
pub fn bounded_action_receipt(value: &impl Serialize) -> Result<serde_json::Value, String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| "No se pudo serializar el recibo de acción".to_owned())?;
    if bytes.len() > MAX_ACTION_RECEIPT_BYTES {
        return Err(format!(
            "Recibo de acción de {} bytes; máximo {MAX_ACTION_RECEIPT_BYTES}",
            bytes.len()
        ));
    }
    serde_json::from_slice(&bytes).map_err(|_| "No se pudo leer el recibo de acción".to_owned())
}
// A JSON action has its own 16,384-byte contract, independent of the hub's
// request envelope. The hub-only bounds live in contracts/constants.json.
pub const MAX_ACTION_BYTES: usize = 16 * 1024;
// The upload ends before the hub's 30-second waiter. This bounds one upload,
// not a retry or a deadline for user consent.
pub const CAPTURE_UPLOAD_TIMEOUT_SECONDS: u64 = 20;
// The playback decoder accepts at most 4 MiB of compressed audio; measured
// base64 expansion is 5,592,408 bytes. This bounds decoded audio, not a line.
pub const MAX_SPEECH_CLIP_BYTES: usize = 4 * 1024 * 1024;
// One attached file, the hub's `MAX_ATTACHMENT_BYTES`. What it
// protects on the Mac: the daemon's memory, since it reads a file whole by its
// path before uploading it and holds one whole when the chat's viewer asks
// for it. The hub refuses past it too; this copy lets the Mac refuse before
// reading. Why 20 MiB, and what it does not limit, is written in
// server/orbit_hub/attachments.py.
pub const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;
// A 160 KiB inventory leaves 2 KiB for the JSON envelope after bounded rows.
// It protects one window listing, not the image captured from a window.
pub const MAX_WINDOW_INVENTORY_BYTES: usize = 160 * 1024;
// The calendar values below are owned by `calendar`, mirrored in Swift and
// compared by the constants guard (`scripts/tests/test_constants_copies.py`).
// Registry entries cite them directly; they are not second definitions of
// those ceilings.

#[derive(Serialize)]
pub struct Limit {
    pub name: &'static str,
    pub value: u64,
    pub unit: &'static str,
    pub resource: &'static str,
}

pub fn registry() -> Vec<Limit> {
    macro_rules! item {
        ($name:ident, $unit:literal, $resource:literal) => {
            Limit {
                name: stringify!($name),
                value: $name as u64,
                unit: $unit,
                resource: $resource,
            }
        };
    }
    vec![
        // Page ceilings are owned by the topic/read registries and calculated
        // here, never copied as another literal. They protect one page, not
        // retained topic or read history; see the measurements beside each.
        Limit {
            name: "TOPIC_TRANSPORT_MAX_BYTES",
            value: crate::topics::transport_max_bytes() as u64,
            unit: "bytes",
            resource: "largest topic transport page",
        },
        Limit {
            name: "READ_MAX_PAGE_BYTES",
            value: crate::reads::ReadId::ALL
                .iter()
                .map(|id| id.max_bytes() as u64)
                .fold(0, u64::max),
            unit: "bytes",
            resource: "largest paged read",
        },
        // One capture's PNG, mirrored by the hub's upload route
        // (server/orbit_hub/captures.py) and the helper's encoder
        // (Limits.swift); scripts/tests/test_constants_copies.py compares them.
        Limit {
            name: "MAX_SOURCE_BYTES",
            value: crate::sources::MAX_SOURCE_BYTES as u64,
            unit: "bytes",
            resource: "capture PNG",
        },
        item!(MAX_IPC_LINE_BYTES, "bytes", "app IPC line"),
        item!(MAX_COMMAND_RESPONSE_BYTES, "bytes", "command response"),
        item!(MAX_DEFAULT_COMMAND_BYTES, "bytes", "default command"),
        item!(MAX_HELPER_COMMAND_LINE_BYTES, "bytes", "helper stdin line"),
        item!(MAX_HELPER_EVENT_LINE_BYTES, "bytes", "helper stdout line"),
        item!(
            MAX_HUB_INCOMING_FRAME_BYTES,
            "bytes",
            "hub inbound WebSocket frame"
        ),
        item!(MAX_HUB_HTTP_BODY_BYTES, "bytes", "hub HTTP response body"),
        item!(
            MAX_HUB_OUTGOING_FRAME_BYTES,
            "bytes",
            "hub outbound WebSocket frame"
        ),
        item!(MAX_ACTION_RECEIPT_BYTES, "bytes", "engine action receipt"),
        item!(MAX_ACTION_BYTES, "bytes", "serialized engine action"),
        item!(CAPTURE_UPLOAD_TIMEOUT_SECONDS, "seconds", "capture upload"),
        item!(MAX_SPEECH_CLIP_BYTES, "bytes", "speech playback clip"),
        item!(
            MAX_WINDOW_INVENTORY_BYTES,
            "bytes",
            "window selection inventory"
        ),
        // Copies of the hub's note bounds (`orbit_hub/notes`): the private
        // notes the Mac keeps until the hub confirms them, and the template
        // editor's bounds. The guard compares each with its Python
        // definition; see `notes`.
        Limit {
            name: "MAX_PRIVATE_NOTES_CHARS",
            value: crate::notes::MAX_PRIVATE_NOTES_CHARS as u64,
            unit: "characters",
            resource: "private notes of one recording",
        },
        Limit {
            name: "MAX_NOTE_SECTIONS",
            value: crate::notes::MAX_SECTIONS as u64,
            unit: "sections",
            resource: "note template sections",
        },
        Limit {
            name: "MAX_NOTE_NAME_CHARS",
            value: crate::notes::MAX_NAME_CHARS as u64,
            unit: "characters",
            resource: "note section or template name",
        },
        Limit {
            name: "MAX_MEETING_TITLE_CHARS",
            value: crate::MAX_MANIFEST_TITLE_CHARACTERS as u64,
            unit: "characters",
            resource: "meeting title, at its start and when renamed",
        },
        Limit {
            name: "MAX_PERSON_NAME_CHARS",
            value: crate::notes::MAX_PERSON_NAME_CHARS as u64,
            unit: "characters",
            resource: "owner or speaker name",
        },
        Limit {
            name: "NOTES_LIST_PAGE",
            value: crate::notes::LIST_PAGE as u64,
            unit: "meetings",
            resource: "meetings per page of the hub's list",
        },
        Limit {
            name: "NOTE_TURN_PAGE_BYTES",
            value: crate::notes::TURN_PAGE_BYTES as u64,
            unit: "bytes",
            resource: "turns per page of the hub's transcript",
        },
        Limit {
            name: "MAX_NOTE_EVENT_TITLE_CHARS",
            value: crate::notes::MAX_EVENT_TITLE_CHARS as u64,
            unit: "characters",
            resource: "linked event title in a closing manifest",
        },
        Limit {
            name: "MAX_NOTE_EVENT_ATTENDEES",
            value: crate::notes::MAX_EVENT_ATTENDEES as u64,
            unit: "guests",
            resource: "linked event guests in a closing manifest",
        },
        Limit {
            name: "MAX_NOTE_ATTENDEE_NAME_CHARS",
            value: crate::notes::MAX_ATTENDEE_NAME_CHARS as u64,
            unit: "characters",
            resource: "linked event guest name",
        },
        Limit {
            name: "MAX_NOTE_ATTENDEE_EMAIL_CHARS",
            value: crate::notes::MAX_ATTENDEE_EMAIL_CHARS as u64,
            unit: "characters",
            resource: "linked event guest address",
        },
        // The call detection's times. The app shows the countdown
        // (from the deadline the daemon publishes, never a copy of these).
        Limit {
            name: "CALL_CONFIRM_MS",
            value: crate::call_detection::CALL_CONFIRM_MS as u64,
            unit: "milliseconds",
            resource: "microphone hold before a call counts",
        },
        Limit {
            name: "CALL_RELEASE_MS",
            value: crate::call_detection::CALL_RELEASE_MS as u64,
            unit: "milliseconds",
            resource: "microphone release before a call is over",
        },
        Limit {
            name: "AUTO_STOP_COUNTDOWN_MS",
            value: crate::call_detection::AUTO_STOP_COUNTDOWN_MS as u64,
            unit: "milliseconds",
            resource: "countdown before a call's recording ends",
        },
        Limit {
            name: "CALL_EVENT_LOOKAHEAD_MS",
            value: crate::call_detection::EVENT_LOOKAHEAD_MS as u64,
            unit: "milliseconds",
            resource: "how soon an event starts to be a call's event",
        },
        // Copies of the hub's template bounds
        // (`orbit_hub/notes/{templates,custom_templates}.py`), shown by the
        // template editor; see `notes`.
        Limit {
            name: "MAX_NOTE_TEMPLATE_CONTEXT_CHARS",
            value: crate::notes::MAX_CONTEXT_CHARS as u64,
            unit: "characters",
            resource: "note template context",
        },
        Limit {
            name: "MAX_NOTE_TEMPLATE_INSTRUCTIONS_CHARS",
            value: crate::notes::MAX_INSTRUCTIONS_CHARS as u64,
            unit: "characters",
            resource: "note template section instructions",
        },
        Limit {
            name: "MAX_CUSTOM_NOTE_TEMPLATES",
            value: crate::notes::MAX_CUSTOM_TEMPLATES as u64,
            unit: "templates",
            resource: "custom note templates of one device",
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
        Limit {
            name: "MAX_NATIVE_PAGE_BYTES",
            value: crate::calendar::MAX_NATIVE_PAGE_BYTES as u64,
            unit: "bytes",
            resource: "calendar helper page",
        },
        Limit {
            name: "NATIVE_PAGE_EVENT_BYTES",
            value: crate::calendar::NATIVE_PAGE_EVENT_BYTES as u64,
            unit: "bytes",
            resource: "calendar page event budget",
        },
        Limit {
            name: "EVENTKIT_MAX_QUERY_MS",
            value: crate::calendar::EVENTKIT_MAX_QUERY_MS as u64,
            unit: "milliseconds",
            resource: "EventKit query window",
        },
        Limit {
            name: "MAX_TITLE_BYTES",
            value: crate::calendar::MAX_TITLE_BYTES as u64,
            unit: "bytes",
            resource: "calendar title",
        },
        Limit {
            name: "MAX_URL_BYTES",
            value: crate::calendar::MAX_URL_BYTES as u64,
            unit: "bytes",
            resource: "calendar URL",
        },
        Limit {
            name: "MAX_LABEL_BYTES",
            value: crate::calendar::MAX_LABEL_BYTES as u64,
            unit: "bytes",
            resource: "calendar label",
        },
        Limit {
            name: "MAX_TIMEZONE_BYTES",
            value: crate::calendar::MAX_TIMEZONE_BYTES as u64,
            unit: "bytes",
            resource: "calendar timezone",
        },
        Limit {
            name: "READ_LIFETIME_NS",
            value: crate::calendar::READ_LIFETIME_NS,
            unit: "nanoseconds",
            resource: "calendar read lifetime",
        },
        Limit {
            name: "PERMISSION_LIFETIME_NS",
            value: crate::calendar::PERMISSION_LIFETIME_NS,
            unit: "nanoseconds",
            resource: "calendar permission response lifetime",
        },
        Limit {
            name: "MAX_GRID_DAYS",
            value: crate::calendar::MAX_GRID_DAYS as u64,
            unit: "days",
            resource: "widest N-day calendar grid",
        },
        // The bounds of the calendar writes and of the notes a read carries.
        Limit {
            name: "MAX_NOTES_BYTES",
            value: crate::calendar::MAX_NOTES_BYTES as u64,
            unit: "bytes",
            resource: "calendar event notes",
        },
        Limit {
            name: "MAX_EVENT_ALARMS",
            value: crate::calendar::MAX_EVENT_ALARMS as u64,
            unit: "alarms",
            resource: "alarms of one written calendar event",
        },
        Limit {
            name: "MAX_ALARM_MINUTES",
            value: crate::calendar::MAX_ALARM_MINUTES,
            unit: "minutes",
            resource: "calendar alarm offset before its event",
        },
        Limit {
            name: "WRITE_LIFETIME_NS",
            value: crate::calendar::WRITE_LIFETIME_NS,
            unit: "nanoseconds",
            resource: "calendar write lifetime before its outcome is unknown",
        },
    ]
}
