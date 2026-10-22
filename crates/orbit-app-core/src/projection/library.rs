//! The note library read (the hub's templates and assignments, and
//! the orders of this session), as the app shows it.
use crate::*;
use orbit_core::notes::{LibraryItem, LibraryList};
use std::collections::{BTreeMap, BTreeSet};

fn known() -> NoteLibraryListSnapshot {
    NoteLibraryListSnapshot {
        state: NoteTemplatesState::Available,
        detail: None,
    }
}

fn action(name: &str) -> NoteLibraryAction {
    match name {
        "delete" => NoteLibraryAction::Delete,
        "assign" => NoteLibraryAction::Assign,
        "unassign" => NoteLibraryAction::Unassign,
        "use_for_series" => NoteLibraryAction::UseForSeries,
        _ => NoteLibraryAction::Save,
    }
}

/// The library from the items of its read, in their order.
pub(crate) fn project_library(items: Vec<LibraryItem>) -> NoteLibrarySnapshot {
    let mut library = NoteLibrarySnapshot {
        dictionaries: Some(known()),
        dictionary_items: Vec::new(),
        dictionary_failures: Vec::new(),
        dictionary_pending: Vec::new(),
        read: NoteLibraryReadState::Available,
        read_detail: None,
        templates: known(),
        items: Vec::new(),
        links: known(),
        assignments: Vec::new(),
        refusals: Vec::new(),
        restores: Vec::new(),
        bounds: NoteTemplateBoundsSnapshot {
            max_sections: orbit_core::notes::MAX_SECTIONS,
            max_name_chars: orbit_core::notes::MAX_NAME_CHARS,
            max_context_chars: orbit_core::notes::MAX_CONTEXT_CHARS,
            max_instructions_chars: orbit_core::notes::MAX_INSTRUCTIONS_CHARS,
            max_custom_templates: orbit_core::notes::MAX_CUSTOM_TEMPLATES,
        },
    };
    let mut pending = BTreeSet::new();
    for item in items {
        match item {
            LibraryItem::Dictionary(dictionary) => {
                library.dictionary_items.push(DictionarySnapshot {
                    dictionary,
                    pending: false,
                })
            }
            LibraryItem::DictionaryRefused { failure } => library.dictionary_failures.push(failure),
            LibraryItem::Template(template) => library.items.push(NoteTemplateDetailSnapshot {
                id: template.id,
                name: template.name,
                context: template.context,
                sections: template.sections,
                builtin: template.builtin,
                forked_from: template
                    .forked_from
                    .map(|origin| NoteTemplateOriginSnapshot {
                        id: origin.id,
                        name: origin.name,
                    }),
                pending: false,
            }),
            LibraryItem::Link(link) => library.assignments.push(NoteTemplateLinkSnapshot {
                calendar_item_id: link.calendar_item_id,
                template_id: link.template_id,
                template_name: None,
                pending: false,
            }),
            LibraryItem::Unavailable { list, refusal } => {
                let failed = NoteLibraryListSnapshot {
                    state: if refusal.needs_the_hub() {
                        NoteTemplatesState::NeedsHub
                    } else {
                        NoteTemplatesState::Failed
                    },
                    detail: Some(refusal.to_string()),
                };
                match list {
                    LibraryList::Dictionaries => library.dictionaries = Some(failed),
                    LibraryList::Templates => library.templates = failed,
                    LibraryList::Links => library.links = failed,
                }
            }
            LibraryItem::Pending { target } => {
                pending.insert(target);
            }
            LibraryItem::Refused(refused) => library.refusals.push(NoteLibraryRefusalSnapshot {
                target_kind: refused.target_kind,
                target_id: refused.target_id,
                action: action(&refused.action),
                status: refused.status,
                code: refused.code,
                numbers: (!refused.numbers.is_empty()).then_some(refused.numbers),
            }),
            LibraryItem::Restored {
                request_id,
                template_id,
            } => library.restores.push(NoteTemplateRestoreSnapshot {
                request_id,
                template_id,
            }),
        }
    }
    library.dictionary_pending = pending
        .iter()
        .filter(|target| {
            target.starts_with("dictionary:") || target.starts_with("dictionary_meeting:")
        })
        .cloned()
        .collect();
    for dictionary in &mut library.dictionary_items {
        dictionary.pending = pending.contains(&format!("dictionary:{}", dictionary.dictionary.id));
    }
    let names = library
        .items
        .iter()
        .map(|template| (template.id.clone(), template.name.clone()))
        .collect::<BTreeMap<_, _>>();
    for template in &mut library.items {
        template.pending = pending.contains(&format!("template:{}", template.id));
    }
    for link in &mut library.assignments {
        link.template_name = names.get(&link.template_id).cloned();
        link.pending = pending.contains(&format!("series:{}", link.calendar_item_id));
    }
    library
}

#[cfg(test)]
mod tests;
