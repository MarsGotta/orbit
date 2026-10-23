//! What the library read adds to the hub's lists: the orders still on
//! this Mac, the refused ones and the undos answered. Kept in memory only:
//! the orders themselves wait in the outbox.
use super::Runtime;
use orbit_core::notes::{self as core, LibraryRefusal, LibraryRequest};
use std::collections::{BTreeMap, VecDeque};

/// Library refusals kept to show, newest last. Each names its target, so a
/// template or a series shows its own; eight cover one session of edits. It
/// protects daemon memory and the library read; not a limit on orders.
pub(super) const MAX_KEPT_REFUSALS: usize = 8;
/// Undos («Deshacer») remembered as answered, newest last: the app reads its
/// own a moment after pressing it. Four cover several in a row.
pub(super) const MAX_KEPT_RESTORES: usize = 4;

#[derive(Default)]
pub(in crate::daemon) struct LibraryRuntime {
    pub(super) dictionary_failures: VecDeque<core::DictionaryFailure>,
    /// The conflicts of the last corrections, by their orders' targets.
    pub(super) dictionary_conflicts: VecDeque<(String, Vec<core::DictionaryConflict>)>,
    pub(super) refusals: VecDeque<LibraryRefusal>,
    pub(super) restores: VecDeque<(String, String)>,
}

impl Runtime {
    /// What the library read adds to the hub's lists: the orders on their
    /// way, the refused ones and the undos answered.
    pub(in crate::daemon) fn library_state(&self) -> Vec<core::LibraryItem> {
        let library = &self.features.notes.library;
        let pending =
            self.features
                .notes
                .outbox
                .library_orders()
                .map(|request| core::LibraryItem::Pending {
                    target: request.target(),
                });
        let refused = library
            .refusals
            .iter()
            .cloned()
            .map(core::LibraryItem::Refused);
        let restored =
            library
                .restores
                .iter()
                .map(|(request_id, template_id)| core::LibraryItem::Restored {
                    request_id: request_id.clone(),
                    template_id: template_id.clone(),
                });
        let dictionary_failures = library
            .dictionary_failures
            .iter()
            .cloned()
            .map(|failure| core::LibraryItem::DictionaryRefused { failure });
        let dictionary_conflicts =
            library
                .dictionary_conflicts
                .iter()
                .map(
                    |(target, conflicts)| core::LibraryItem::DictionaryConflicts {
                        target: target.clone(),
                        conflicts: conflicts.clone(),
                    },
                );
        pending
            .chain(refused)
            .chain(restored)
            .chain(dictionary_failures)
            .chain(dictionary_conflicts)
            .collect()
    }
}

pub(super) fn refusal_target(refusal: &LibraryRefusal) -> String {
    format!("{}:{}", refusal.target_kind, refusal.target_id)
}

pub(super) fn refuse_order(
    library: &mut LibraryRuntime,
    request: &LibraryRequest,
    status: u16,
    code: &str,
    numbers: BTreeMap<String, u64>,
) {
    let (target_kind, target_id, action) = match request {
        LibraryRequest::Dictionary { request } => {
            library
                .dictionary_failures
                .push_back(core::DictionaryFailure {
                    target: request.target(),
                    code: code.into(),
                    detail: "No se confirmó el cambio: la sesión del hub se cerró".into(),
                    numbers: BTreeMap::new(),
                });
            while library.dictionary_failures.len() > MAX_KEPT_REFUSALS {
                library.dictionary_failures.pop_front();
            }
            return;
        }
        LibraryRequest::SaveTemplate { template_id, .. }
        | LibraryRequest::RestoreTemplate { template_id, .. } => ("template", template_id, "save"),
        LibraryRequest::DeleteTemplate { template_id } => ("template", template_id, "delete"),
        LibraryRequest::Assign {
            calendar_item_id,
            template_id: Some(_),
        } => ("series", calendar_item_id, "assign"),
        LibraryRequest::Assign {
            calendar_item_id,
            template_id: None,
        } => ("series", calendar_item_id, "unassign"),
        LibraryRequest::UseForSeries { recording_id, .. } => {
            ("meeting_series", recording_id, "use_for_series")
        }
    };
    library.refusals.push_back(LibraryRefusal {
        target_kind: target_kind.into(),
        target_id: target_id.clone(),
        action: action.into(),
        status,
        code: code.into(),
        numbers,
    });
    while library.refusals.len() > MAX_KEPT_REFUSALS {
        library.refusals.pop_front();
    }
}
