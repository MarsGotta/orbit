//! The hub's lists the notes reads pass through: the earlier meeting of a
//! series, and the library (dictionaries, templates and their links) with
//! what this Mac adds to it.
use super::super::reads::bounded_page;
use crate::hub_notes::NoteHub;
use orbit_core::{
    reads::ReadId,
    refusal::Refusal,
    topics::{Cursor, Knowledge, Page},
};
use serde_json::Value;

fn absent(refusal: Refusal) -> Knowledge<Page<Value>> {
    Knowledge::Absent { refusal }
}

/// The earlier meeting of a series (`(calendar_item_id, before_ms)`) and its
/// note's tasks, as the hub gives them; failing that, with `title_digest`, the
/// earlier meeting of the same title. `previous: null` is the hub's fact that
/// there is none.
pub(super) async fn series_previous(
    hub: Result<NoteHub, Refusal>,
    (calendar_item_id, before_ms): (String, i64),
    title_digest: Option<String>,
    revision: u64,
) -> Knowledge<Page<Value>> {
    let read = match hub {
        Ok(hub) => match hub
            .previous_meeting(&calendar_item_id, before_ms, title_digest.as_deref())
            .await
        {
            // The route names no recording, so its 404 is a hub that does
            // not have it (0.10 and earlier): unknown, never «none».
            crate::hub_notes::Answer::Refused { status: 404, .. } => Err(Refusal::UpstreamFailed {
                service: orbit_core::refusal::Service::Hub,
                status: Some(404),
            }),
            answer @ (crate::hub_notes::Answer::Value(_)
            | crate::hub_notes::Answer::Refused { .. }
            | crate::hub_notes::Answer::Unreachable(_)) => answer.into_result(),
        },
        Err(refusal) => Err(refusal),
    };
    match read {
        Ok(answer)
            if answer.calendar_item_id == calendar_item_id && answer.before_ms == before_ms =>
        {
            bounded_page(&[answer], None, 1, revision, ReadId::SeriesPrevious)
        }
        Ok(_) => absent(Refusal::HubReplyInvalid {
            field: "calendar_item_id".into(),
            received_bytes: None,
        }),
        Err(refusal) => absent(refusal),
    }
}

pub(super) async fn library(
    hub: Result<NoteHub, Refusal>,
    skip: usize,
    state: Vec<orbit_core::notes::LibraryItem>,
    revision: u64,
) -> Knowledge<Page<Value>> {
    use orbit_core::notes::{LibraryItem, LibraryList};
    let mut items = Vec::new();
    match &hub {
        Ok(hub) => {
            let dictionaries = match hub.dictionaries().await {
                crate::hub_notes::Answer::Refused { status, .. } => Err(Refusal::UpstreamFailed {
                    service: orbit_core::refusal::Service::Hub,
                    status: Some(status),
                }),
                answer @ (crate::hub_notes::Answer::Value(_)
                | crate::hub_notes::Answer::Unreachable(_)) => answer.into_result(),
            };
            match dictionaries {
                Ok(list) => {
                    items.extend(list.dictionaries.into_iter().map(LibraryItem::Dictionary))
                }
                Err(refusal) => items.push(LibraryItem::Unavailable {
                    list: LibraryList::Dictionaries,
                    refusal,
                }),
            }
            match hub.templates().await.into_result() {
                Ok(list) => items.extend(list.templates.into_iter().map(LibraryItem::Template)),
                Err(refusal) => items.push(LibraryItem::Unavailable {
                    list: LibraryList::Templates,
                    refusal,
                }),
            }
            match hub.links().await.into_result() {
                Ok(links) => items.extend(links.links.into_iter().map(LibraryItem::Link)),
                Err(refusal) => items.push(LibraryItem::Unavailable {
                    list: LibraryList::Links,
                    refusal,
                }),
            }
        }
        Err(refusal) => {
            for list in [
                LibraryList::Templates,
                LibraryList::Links,
                LibraryList::Dictionaries,
            ] {
                items.push(LibraryItem::Unavailable {
                    list,
                    refusal: refusal.clone(),
                });
            }
        }
    }
    items.extend(state);
    super::super::reads::page(
        &items,
        Some(Cursor::Index { index: skip as u64 }),
        ReadId::NoteLibrary,
        revision,
    )
}
