use super::*;
use crate::notes::{DictionaryRequest, MeetingDocument};
use serde_json::json;

const RECORDING: &str = "6f2b7d5a-9c41-4b7e-8d3f-1a2b3c4d5e6f";

fn meeting(order: NoteOrder) -> QueuedOrder {
    QueuedOrder::Meeting {
        recording_id: RECORDING.into(),
        order,
    }
}

fn tick(id: &str, done: bool, sections: Option<&str>, base_revision: u64) -> QueuedOrder {
    meeting(NoteOrder::Edit {
        sections: sections.map(|markdown| {
            vec![Section {
                name: "Resumen".into(),
                markdown: markdown.into(),
            }]
        }),
        done: BTreeMap::from([(id.to_owned(), done)]),
        base_revision,
    })
}

#[test]
fn two_edits_of_one_note_merge_as_the_hub_merges_them() {
    let merged = tick("a", true, Some("- antes"), 3).superseded_by(tick("a", false, None, 4));
    assert_eq!(
        merged,
        meeting(NoteOrder::Edit {
            sections: Some(vec![Section {
                name: "Resumen".into(),
                markdown: "- antes".into(),
            }]),
            done: BTreeMap::from([("a".to_owned(), false)]),
            base_revision: 4,
        })
    );
    let both = tick("a", true, None, 3).superseded_by(tick("b", true, Some("- después"), 3));
    let QueuedOrder::Meeting {
        order: NoteOrder::Edit { sections, done, .. },
        ..
    } = both
    else {
        panic!("two edits did not stay an edit");
    };
    assert_eq!(done.len(), 2);
    assert_eq!(
        sections.map(|s| s[0].markdown.clone()),
        Some("- después".into())
    );
}

#[test]
fn the_newest_order_on_a_target_replaces_the_older_one() {
    let older = meeting(NoteOrder::Title {
        title: "Primera".into(),
    });
    let newer = meeting(NoteOrder::Title {
        title: "Segunda".into(),
    });
    assert_eq!(older.target(), newer.target());
    assert_eq!(older.superseded_by(newer.clone()), newer);
    let voice = meeting(NoteOrder::Rename {
        key: "mic_local".into(),
        name: Some("Ada".into()),
        scope: SpeakerRenameScope::Voice,
    });
    let turn = meeting(NoteOrder::Rename {
        key: "mic_local".into(),
        name: Some("Ada".into()),
        scope: SpeakerRenameScope::Turn { at_ms: 4_000 },
    });
    assert_ne!(voice.target(), turn.target());
    assert_eq!(voice.lane(), Lane::Meeting(RECORDING.into()));
    assert_eq!(voice.lane().key(), format!("meeting:{RECORDING}"));
}

#[test]
fn a_dictionary_order_never_replaces_another_and_is_not_sent_twice() {
    let correction = QueuedOrder::Library {
        request: LibraryRequest::Dictionary {
            request: DictionaryRequest::Save {
                dictionary_id: "dic-uno".into(),
                name: "Proyecto".into(),
            },
        },
    };
    assert_eq!(correction.target(), None);
    assert_eq!(correction.lane(), Lane::Library);
    assert!(!correction.repeatable());
    assert!(!meeting(NoteOrder::Retranscribe).repeatable());
    assert!(
        !meeting(NoteOrder::Write {
            template_id: "general".into()
        })
        .repeatable()
    );
    assert!(meeting(NoteOrder::RetryTranscript).repeatable());
    assert!(tick("a", true, None, 1).repeatable());
}

#[test]
fn a_failing_lane_waits_longer_each_time_up_to_five_minutes() {
    assert_eq!(retry_after_ms(0), FIRST_RETRY_MS);
    assert_eq!(retry_after_ms(1), 5_000);
    assert_eq!(retry_after_ms(2), 10_000);
    assert_eq!(retry_after_ms(6), 160_000);
    assert_eq!(retry_after_ms(7), MAX_RETRY_MS);
    assert_eq!(retry_after_ms(u32::MAX), MAX_RETRY_MS);
}

#[test]
fn the_orders_on_this_mac_lay_over_the_hubs_document() -> Result<(), serde_json::Error> {
    let mut document: MeetingDocument = serde_json::from_value(json!({
        "recording_id": RECORDING, "title": "Del hub", "started_at_ms": 1, "duration_ms": 2,
        "capture": "closed", "language": "es",
        "speakers": [{"key": "mic_local", "name": "Micrófono local"}],
        "transcript": {"state": "ready", "pending_chunks": 0, "failed_groups": []},
        "note": {"state": "ready", "revision": 7, "note": {
            "sections": [{"name": "Resumen", "markdown": "- del hub"}],
            "action_items": [{"id": "a", "text": "Enviar", "done": false}]}},
    }))?;
    let orders = [
        NoteOrder::Title {
            title: "Mía".into(),
        },
        NoteOrder::Rename {
            key: "mic_local".into(),
            name: Some("Ada".into()),
            scope: SpeakerRenameScope::Voice,
        },
        NoteOrder::Language {
            language: "en".into(),
        },
    ];
    let edit = tick("a", true, Some("- mío"), 7);
    let QueuedOrder::Meeting { order: edit, .. } = edit else {
        panic!("not a meeting order");
    };
    lay_over(&mut document, orders.iter().chain([&edit]));
    assert_eq!(document.title, "Mía");
    assert_eq!(document.speakers[0].name, "Ada");
    assert_eq!(document.language.as_deref(), Some("en"));
    let seen = document.note.as_ref().and_then(|note| note.note.as_ref());
    assert_eq!(
        seen.map(|seen| seen.sections[0].markdown.as_str()),
        Some("- mío")
    );
    assert_eq!(seen.map(|seen| seen.action_items[0].done), Some(true));
    assert_eq!(
        document.pending_changes,
        [
            "action:a",
            "language",
            "sections",
            "speaker:mic_local",
            "title"
        ]
    );
    // Nothing waiting: the hub's document as it came.
    lay_over(&mut document, []);
    assert!(document.pending_changes.is_empty());
    Ok(())
}

/// «Borrar reunión» is one order per meeting, set again as often as it is
/// asked, and the meeting's capture thrown away stays thrown away.
#[test]
fn a_delete_is_one_order_of_its_meeting_and_a_discard_stays_one() -> Result<(), serde_json::Error> {
    let asked = NoteOrder::of_command(Command::MeetingDelete {
        recording_id: RECORDING.into(),
    });
    assert_eq!(
        asked,
        Some((RECORDING.to_owned(), NoteOrder::Delete { discarded: false }))
    );
    let asked = meeting(NoteOrder::Delete { discarded: false });
    assert_eq!(asked.target().as_deref(), Some("delete"));
    assert!(asked.repeatable(), "deleting twice deletes once");
    assert_eq!(asked.deletes(), Some((RECORDING, false)));
    let thrown = meeting(NoteOrder::Delete { discarded: true });
    assert_eq!(
        thrown.clone().superseded_by(asked.clone()).deletes(),
        Some((RECORDING, true))
    );
    assert_eq!(
        asked.superseded_by(thrown).deletes(),
        Some((RECORDING, true))
    );
    assert_eq!(meeting(NoteOrder::Retranscribe).deletes(), None);
    // An order written without `discarded` reads as a plain delete.
    let older: QueuedOrder = serde_json::from_value(json!({
        "lane": "meeting", "recording_id": RECORDING, "order": {"order": "delete"}
    }))?;
    assert_eq!(older.deletes(), Some((RECORDING, false)));
    Ok(())
}

#[test]
fn a_meeting_waiting_to_be_deleted_says_so_in_its_document() -> Result<(), serde_json::Error> {
    let mut document: MeetingDocument = serde_json::from_value(json!({
        "recording_id": RECORDING, "title": "Revisión de Lumen", "started_at_ms": 1,
        "duration_ms": 2, "capture": "closed", "language": null, "speakers": [],
        "transcript": {"state": "ready", "pending_chunks": 0, "failed_groups": []},
        "note": null,
    }))?;
    lay_over(&mut document, [&NoteOrder::Delete { discarded: false }]);
    assert_eq!(document.pending_changes, vec!["delete".to_owned()]);
    Ok(())
}
