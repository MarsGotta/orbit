use super::FeatureInput;
use crate::{AppSnapshot, OpenNoteSnapshot};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub type View = Option<OpenNoteSnapshot>;

pub fn project(input: &FeatureInput<'_>) -> View {
    input
        .local
        .notes
        .as_ref()
        .and_then(|notes| notes.open.clone())
        .map(|mut note| {
            super::notes::localize_note(&mut note, input.copy);
            note
        })
}

pub fn apply(view: &View, snapshot: &mut AppSnapshot) {
    if let Some(notes) = snapshot.notes.as_mut() {
        notes.open = view.clone();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "command", rename_all = "snake_case")]
#[ts(export, export_to = "../../apps/desktop/src/bridge/generated/")]
pub enum NoteCommand {
    NotesOpen {
        recording_id: String,
    },
    /// The name of one speaker of the open note, by the hub's key; `null`
    /// gives it back its default name.
    SpeakerRename {
        speaker_key: String,
        name: Option<String>,
        #[serde(default)]
        scope: orbit_core::notes::SpeakerRenameScope,
    },
    /// The user's private notes (markdown), kept by the daemon until the hub
    /// confirms them.
    NotePrivateSave {
        recording_id: String,
        markdown: String,
    },
    /// The note's sections; `sections` replaces the whole list.
    NoteEdit {
        recording_id: String,
        sections: Vec<crate::Section>,
    },
    /// Mark one task done or not done.
    NoteActionToggle {
        recording_id: String,
        action_id: String,
        done: bool,
    },
    AskOrbitAbout {
        recording_id: String,
    },
}

fn open_note<'a>(snapshot: &'a AppSnapshot, recording_id: &str) -> Option<&'a OpenNoteSnapshot> {
    snapshot
        .notes
        .as_ref()
        .and_then(|notes| notes.open.as_ref())
        .filter(|note| note.recording_id == recording_id)
}

/// The hub's revision of the note an edit is made on: the open note's, or
/// the series' earlier note Today shows the tasks of.
fn note_revision(snapshot: &AppSnapshot, recording_id: &str) -> Result<u64, crate::AppError> {
    open_note(snapshot, recording_id)
        .and_then(|note| note.note_revision)
        .or_else(|| series_note_revision(snapshot, recording_id))
        .ok_or_else(crate::AppError::note_unavailable)
}

/// The revision of the series' earlier note, when that is `recording_id` and
/// its note is written (`client::series`).
fn series_note_revision(snapshot: &AppSnapshot, recording_id: &str) -> Option<u64> {
    let read = snapshot.notes.as_ref()?.series_previous.as_deref()?;
    let meeting = read.answer.as_ref().ok()?.previous.as_ref()?;
    let note = meeting.note.as_ref()?;
    (meeting.recording_id == recording_id && note.action_items.is_some()).then_some(note.revision)
}

pub(crate) fn map_command_with_copy(
    command: NoteCommand,
    snapshot: &AppSnapshot,
    copy: crate::PresentationCopy,
) -> Result<orbit_core::protocol::Command, crate::AppError> {
    use orbit_core::protocol::Command as DaemonCommand;
    match command {
        NoteCommand::NotesOpen { recording_id } => Ok(DaemonCommand::Read {
            read: orbit_core::reads::ReadRequest::NoteDetail {
                id: recording_id,
                cursor: None,
            },
        }),
        NoteCommand::NotePrivateSave {
            recording_id,
            markdown,
        } => Ok(DaemonCommand::NotePrivateSave {
            base_revision: open_note(snapshot, &recording_id)
                .map_or(0, |note| note.private_notes_revision),
            recording_id,
            markdown,
        }),
        NoteCommand::NoteEdit {
            recording_id,
            sections,
        } => Ok(DaemonCommand::NoteEdit {
            base_revision: note_revision(snapshot, &recording_id)?,
            recording_id,
            sections: Some(sections),
            done: std::collections::BTreeMap::new(),
        }),
        NoteCommand::NoteActionToggle {
            recording_id,
            action_id,
            done,
        } => Ok(DaemonCommand::NoteEdit {
            base_revision: note_revision(snapshot, &recording_id)?,
            recording_id,
            sections: None,
            done: [(action_id, done)].into(),
        }),
        NoteCommand::AskOrbitAbout { recording_id } => Ok(ask_about(snapshot, &recording_id, copy)),
        NoteCommand::SpeakerRename {
            speaker_key,
            name,
            scope,
        } => speaker_rename(snapshot, speaker_key, name, scope),
    }
}

#[cfg(test)]
fn map_command(
    command: NoteCommand,
    snapshot: &AppSnapshot,
) -> Result<orbit_core::protocol::Command, crate::AppError> {
    map_command_with_copy(
        command,
        snapshot,
        crate::PresentationCopy {
            render: |text| match text {
                crate::PresentationText::AskAbout {
                    title,
                    recording_id,
                } => format!("Quiero preguntar sobre la nota «{title}» ({recording_id})."),
                crate::PresentationText::UpdateBlocked(_)
                | crate::PresentationText::AudioPending(_)
                | crate::PresentationText::ReadFailure { .. }
                | crate::PresentationText::MeetingTitle { .. } => format!("{text:?}"),
            },
        },
    )
}

/// «Preguntar a Orbit» about a note: a chat message that names it by its
/// title, the open note's or the list's, or by its id when neither has it.
fn ask_about(
    snapshot: &AppSnapshot,
    recording_id: &str,
    copy: crate::PresentationCopy,
) -> orbit_core::protocol::Command {
    let notes = snapshot.notes.as_ref();
    let title = open_note(snapshot, recording_id)
        .map(|note| note.title.as_str())
        .or_else(|| {
            notes?
                .list
                .iter()
                .find(|note| note.recording_id == recording_id)
                .map(|note| note.title.as_str())
        })
        .unwrap_or(recording_id);
    orbit_core::protocol::Command::Chat {
        attachments: Vec::new(),
        text: copy.text(crate::PresentationText::AskAbout {
            title: title.into(),
            recording_id: recording_id.into(),
        }),
        // Nobody named: the primary answers.
        primary: true,
    }
}

/// A speaker's name, in the open note that has that speaker.
fn speaker_rename(
    snapshot: &AppSnapshot,
    speaker_key: String,
    name: Option<String>,
    scope: orbit_core::notes::SpeakerRenameScope,
) -> Result<orbit_core::protocol::Command, crate::AppError> {
    let recording_id = snapshot
        .notes
        .as_ref()
        .and_then(|notes| notes.open.as_ref())
        .filter(|note| {
            note.speakers
                .iter()
                .any(|speaker| speaker.key == speaker_key)
        })
        .map(|note| note.recording_id.clone())
        .ok_or_else(crate::AppError::note_unavailable)?;
    Ok(orbit_core::protocol::Command::SpeakerRename {
        recording_id,
        key: speaker_key,
        name,
        scope,
    })
}

#[cfg(test)]
mod mapping_tests {
    use super::*;
    use orbit_core::protocol::Command;

    const RECORDING: &str = "6f2b7d5a-9c41-4b7e-8d3f-1a2b3c4d5e6f";

    fn snapshot() -> Result<AppSnapshot, serde_json::Error> {
        Ok(AppSnapshot {
            notes: Some(serde_json::from_value(serde_json::json!({
            "list": [],
            "open": {
                "recording_id": RECORDING, "title": "Comité", "owner_name": "Alina",
                "started_at_ms": 1, "ended_at_ms": 2, "event": null, "summary": null,
                "note_revision": 7, "actions": [], "decisions": [], "open_topics": [],
                "private_notes": "", "private_notes_revision": 3,
                "progress": {
                    "upload": {"state": "done", "cause": null},
                    "transcript": {"state": "done", "cause": null},
                    "summary": {"state": "done", "cause": null}
                },
                "transcript": {"language": "es", "turns": [], "pending_chunks": 0, "failed_groups": 0},
                "speakers": [{"key": "mic_local", "track": "mic_local", "name": "Alina",
                    "color": "var(--o-c2)", "share_pct": 100}]
            }
            }))?),
            ..AppSnapshot::default()
        })
    }

    /// «Preguntar» in a note names nobody: the primary answers it.
    #[test]
    fn asking_about_a_note_goes_to_the_primary() -> Result<(), serde_json::Error> {
        let mapped = map_command(
            NoteCommand::AskOrbitAbout {
                recording_id: RECORDING.into(),
            },
            &snapshot()?,
        );
        assert!(matches!(mapped,
            Ok(Command::Chat { text, primary: true, .. }) if text.contains("«Comité»")));
        Ok(())
    }

    #[test]
    fn a_rename_names_the_open_note_and_the_hubs_key() -> Result<(), serde_json::Error> {
        let mapped = map_command(
            NoteCommand::SpeakerRename {
                speaker_key: "mic_local".into(),
                name: Some("Ada".into()),
                scope: orbit_core::notes::SpeakerRenameScope::Voice,
            },
            &snapshot()?,
        );
        assert!(matches!(mapped,
            Ok(Command::SpeakerRename { recording_id, key, name, scope: orbit_core::notes::SpeakerRenameScope::Voice })
                if recording_id == RECORDING && key == "mic_local" && name.as_deref() == Some("Ada")));
        // A key the open note does not have is refused, never invented.
        assert!(
            map_command(
                NoteCommand::SpeakerRename {
                    speaker_key: "remote_app".into(),
                    name: None,
                    scope: orbit_core::notes::SpeakerRenameScope::Voice,
                },
                &snapshot()?,
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn a_turn_rename_preserves_its_scope_across_the_app_boundary() -> Result<(), serde_json::Error>
    {
        let input = serde_json::json!({
            "command": "speaker_rename", "speaker_key": "mic_local", "name": "Ada",
            "scope": {"kind": "turn", "at_ms": 12_000}
        });
        let command: NoteCommand = serde_json::from_value(input)?;
        let mapped = map_command(command, &snapshot()?)
            .map_err(|error| serde::de::Error::custom(error.to_string()))?;
        assert_eq!(
            serde_json::to_value(mapped)?["scope"],
            serde_json::json!({"kind": "turn", "at_ms": 12_000})
        );
        Ok(())
    }

    /// Today marks a task of the series' earlier note done through the same
    /// order as its Actions tab, on the revision the series read gave.
    #[test]
    fn a_task_of_the_series_earlier_note_is_marked_on_its_revision() -> Result<(), serde_json::Error>
    {
        const EARLIER: &str = "11111111-1111-4111-8111-111111111111";
        let mut snapshot = snapshot()?;
        let answer = |note: serde_json::Value| -> Result<_, serde_json::Error> {
            serde_json::from_value(serde_json::json!({
                "calendar_item_id": "serie", "before_ms": 10,
                "previous": {"recording_id": EARLIER, "title": "Comité", "started_at_ms": 1,
                    "note": note}
            }))
        };
        let toggle = || NoteCommand::NoteActionToggle {
            recording_id: EARLIER.into(),
            action_id: "a0123456789ab".into(),
            done: true,
        };
        if let Some(notes) = snapshot.notes.as_mut() {
            notes.series_previous = Some(Box::new(crate::SeriesPreviousRead {
                calendar_item_id: "serie".into(),
                before_ms: 10,
                answer: Ok(answer(serde_json::json!({"state": "ready", "revision": 4,
                    "short_summary": null, "action_items": []}))?),
            }));
        }
        assert!(matches!(map_command(toggle(), &snapshot),
            Ok(Command::NoteEdit { base_revision: 4, recording_id, sections: None, done })
                if recording_id == EARLIER && done.get("a0123456789ab") == Some(&true)));
        // A note not written has no tasks to mark: refused, never revision 0.
        if let Some(notes) = snapshot.notes.as_mut() {
            notes.series_previous = Some(Box::new(crate::SeriesPreviousRead {
                calendar_item_id: "serie".into(),
                before_ms: 10,
                answer: Ok(answer(
                    serde_json::json!({"state": "generating", "revision": 4,
                    "short_summary": null, "action_items": null}),
                )?),
            }));
        }
        assert!(map_command(toggle(), &snapshot).is_err());
        Ok(())
    }

    #[test]
    fn an_edit_is_made_on_the_revision_the_note_was_read_at() -> Result<(), serde_json::Error> {
        let mapped = map_command(
            NoteCommand::NoteActionToggle {
                recording_id: RECORDING.into(),
                action_id: "a0123456789ab".into(),
                done: true,
            },
            &snapshot()?,
        );
        assert!(matches!(mapped,
            Ok(Command::NoteEdit { base_revision: 7, sections: None, done, .. })
                if done.get("a0123456789ab") == Some(&true)));
        let mapped = map_command(
            NoteCommand::NotePrivateSave {
                recording_id: RECORDING.into(),
                markdown: "- a".into(),
            },
            &snapshot()?,
        );
        assert!(matches!(
            mapped,
            Ok(Command::NotePrivateSave {
                base_revision: 3,
                ..
            })
        ));
        // Another note than the open one has no revision here: refused.
        assert!(
            map_command(
                NoteCommand::NoteEdit {
                    recording_id: "11111111-1111-4111-8111-111111111111".into(),
                    sections: Vec::new(),
                },
                &snapshot()?,
            )
            .is_err()
        );
        Ok(())
    }
}
