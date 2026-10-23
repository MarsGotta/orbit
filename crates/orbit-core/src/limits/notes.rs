//! Notes and meetings: copies of the hub's note bounds.
use super::Limit;

pub(super) fn limits() -> Vec<Limit> {
    [note_limits(), template_limits()]
        .into_iter()
        .flatten()
        .collect()
}

fn note_limits() -> Vec<Limit> {
    vec![
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
    ]
}

fn template_limits() -> Vec<Limit> {
    vec![
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
    ]
}
