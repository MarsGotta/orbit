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
    /// Mars's private notes (markdown), kept by the daemon until the hub
    /// confirms them (SIM-41).
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

/// The hub's revision of the open note an edit is made on.
fn note_revision(snapshot: &AppSnapshot, recording_id: &str) -> Result<u64, crate::AppError> {
    open_note(snapshot, recording_id)
        .and_then(|note| note.note_revision)
        .ok_or_else(crate::AppError::note_unavailable)
}

pub(crate) fn map_command(
    command: NoteCommand,
    snapshot: &AppSnapshot,
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
        NoteCommand::AskOrbitAbout { recording_id } => {
            let notes = snapshot.notes.as_ref();
            let title = open_note(snapshot, &recording_id)
                .map(|note| note.title.as_str())
                .or_else(|| {
                    notes?
                        .list
                        .iter()
                        .find(|note| note.recording_id == recording_id)
                        .map(|note| note.title.as_str())
                })
                .unwrap_or(recording_id.as_str());
            Ok(DaemonCommand::Chat {
                text: format!("Quiero preguntar sobre la nota «{title}» ({recording_id})."),
            })
        }
        NoteCommand::SpeakerRename {
            speaker_key,
            name,
            scope,
        } => {
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
            Ok(DaemonCommand::SpeakerRename {
                recording_id,
                key: speaker_key,
                name,
                scope,
            })
        }
    }
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
                    "color": "var(--o-blue)", "share_pct": 100}]
            }
            }))?),
            ..AppSnapshot::default()
        })
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
