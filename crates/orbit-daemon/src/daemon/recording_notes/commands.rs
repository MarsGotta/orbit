//! The note orders the app gives (`commands::command` sends them here). What
//! the hub would refuse is refused here at once, with its numbers; anything
//! else is kept in the outbox and taken: the app's promise resolves once the
//! order is on this Mac's disk.
use super::{Outcome, Runtime, check_recording};
use crate::hub_notes::Unreached;
use anyhow::{Result, bail};
use orbit_core::{
    note_outbox::{NoteOrder, QueuedOrder},
    notes::{self as core, LibraryRequest},
    protocol::Command,
};
use std::collections::BTreeMap;

impl Runtime {
    /// A command that gives a note order (`NoteOrder::of_command`).
    pub(in crate::daemon) async fn note_command(&mut self, command: Command) -> Result<()> {
        match NoteOrder::of_command(command) {
            Some((recording_id, order)) => self.note_order(recording_id, order).await,
            None => bail!("Esta orden no es de una nota"),
        }
    }

    /// A note order, checked as the hub checks it and kept until the hub
    /// takes it; its answer comes back as a notice when it fails.
    async fn note_order(&mut self, recording_id: String, order: NoteOrder) -> Result<()> {
        check_recording(&recording_id)?;
        let order = match order {
            NoteOrder::Rename { key, name, scope } => NoteOrder::Rename {
                key,
                scope,
                name: core::speaker_name(name.as_deref())?,
            },
            NoteOrder::Language { language } => {
                core::meeting_language(&language)?;
                NoteOrder::Language { language }
            }
            NoteOrder::Title { title } => NoteOrder::Title {
                title: core::meeting_title(&title)?,
            },
            other @ (NoteOrder::Edit { .. }
            | NoteOrder::Write { .. }
            | NoteOrder::RestoreSuperseded
            | NoteOrder::RetryTranscript
            | NoteOrder::Retranscribe) => other,
        };
        self.queue_order(QueuedOrder::Meeting {
            recording_id,
            order,
        })
        .await
    }

    /// A template, series or dictionary order. A dictionary order is said
    /// as not sent at once when the hub cannot be asked now, as before: it
    /// adds a correction, so it is never kept to be sent later.
    pub(in crate::daemon) async fn note_library(&mut self, request: LibraryRequest) -> Result<()> {
        if let LibraryRequest::Dictionary {
            request: dictionary,
        } = &request
        {
            if let Err(refusal) = self.hub_access() {
                self.dictionary_answer(
                    dictionary,
                    Outcome::Refused {
                        status: 0,
                        code: Some(refusal.code().name().into()),
                        numbers: BTreeMap::new(),
                    },
                );
                return Ok(());
            }
            if self.note_hub().is_none() {
                self.dictionary_answer(
                    dictionary,
                    Outcome::Unreachable {
                        cause: Unreached::Transport(if self.settings.hub_url.is_none() {
                            "falta la URL del hub"
                        } else {
                            "la URL del hub no es válida"
                        }),
                    },
                );
                return Ok(());
            }
        }
        self.queue_order(QueuedOrder::Library { request }).await
    }
}
