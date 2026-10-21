use super::*;
use orbit_core::notes::{LibraryRefusal, Template, TemplateLink};

#[test]
fn the_library_shows_the_hubs_lists_with_the_orders_on_their_way() {
    let library = project_library(vec![
        LibraryItem::Template(Template {
            id: "mia".into(),
            name: "Comité".into(),
            context: String::new(),
            sections: Vec::new(),
            builtin: false,
            forked_from: None,
        }),
        LibraryItem::Link(TemplateLink {
            calendar_item_id: "serie-1".into(),
            template_id: "mia".into(),
        }),
        LibraryItem::Pending {
            target: "template:mia".into(),
        },
        LibraryItem::Refused(LibraryRefusal {
            target_kind: "series".into(),
            target_id: "serie-2".into(),
            action: "assign".into(),
            status: 0,
            code: "hub_unreachable".into(),
        }),
        LibraryItem::Restored {
            request_id: "deshacer-1".into(),
            template_id: "mia".into(),
        },
    ]);
    assert!(library.items[0].pending);
    assert_eq!(
        library.assignments[0].template_name.as_deref(),
        Some("Comité")
    );
    assert!(!library.assignments[0].pending);
    assert_eq!(library.refusals[0].code, "hub_unreachable");
    assert_eq!(library.refusals[0].action, NoteLibraryAction::Assign);
    assert_eq!(library.restores[0].request_id, "deshacer-1");
    assert_eq!(library.templates.state, NoteTemplatesState::Available);
}

#[test]
fn a_list_the_hub_could_not_give_is_failed_never_empty() {
    let library = project_library(vec![LibraryItem::Unavailable {
        list: LibraryList::Links,
        refusal: orbit_core::refusal::Refusal::Legacy {
            text: "respuesta del hub no válida".into(),
        },
    }]);
    assert_eq!(library.links.state, NoteTemplatesState::Failed);
    assert!(library.links.detail.is_some());
    assert_eq!(library.templates.state, NoteTemplatesState::Available);
}

/// SIM-25 run 20: before pairing, the library could not be asked and the app
/// logged `library_read_failed`. Out of reach is unknown, never failed.
#[test]
fn a_list_the_hub_cannot_be_asked_for_needs_the_hub_and_is_not_failed() {
    for refusal in [
        orbit_core::refusal::Refusal::NotPaired,
        orbit_core::refusal::Refusal::HubUnreachable { since_ms: None },
        orbit_core::refusal::Refusal::PrivacyStopped { since_ms: None },
    ] {
        let library = project_library(vec![LibraryItem::Unavailable {
            list: LibraryList::Templates,
            refusal,
        }]);
        assert_eq!(library.templates.state, NoteTemplatesState::NeedsHub);
    }
}

#[test]
fn dictionary_projection_keeps_terms_series_pending_and_unavailable_separate()
-> Result<(), Box<dyn std::error::Error>> {
    let item: LibraryItem = serde_json::from_value(serde_json::json!({
        "item": "dictionary", "id": "vocabulary", "name": "Proyecto",
        "terms": [{"id": "entry", "correct": "NAV", "wrong": ["nab"]}],
        "series": [{"calendar_item_id": "daily", "title": "Daily"}]
    }))?;
    let projected = serde_json::to_value(project_library(vec![
        item,
        LibraryItem::Pending {
            target: "dictionary:vocabulary".into(),
        },
    ]))?;
    assert_eq!(projected["dictionaries"]["state"], "available");
    assert_eq!(
        projected["dictionary_items"][0]["dictionary"]["terms"][0]["correct"],
        "NAV"
    );
    assert_eq!(
        projected["dictionary_items"][0]["dictionary"]["series"][0]["title"],
        "Daily"
    );
    assert_eq!(projected["dictionary_items"][0]["pending"], true);
    let failed: LibraryItem = serde_json::from_value(serde_json::json!({
        "item": "unavailable", "list": "dictionaries", "refusal": {"code": "hub_unreachable", "since_ms": null}
    }))?;
    let projected = serde_json::to_value(project_library(vec![failed]))?;
    assert_eq!(projected["dictionaries"]["state"], "needs_hub");
    assert_eq!(projected["templates"]["state"], "available");
    Ok(())
}
