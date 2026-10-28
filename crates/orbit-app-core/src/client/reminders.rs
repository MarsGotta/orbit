//! «Recordatorio personal» of Hoy's next meeting on the hub: read when Hoy
//! asks, and saved through the daemon, which answers once the hub kept it.
//! The interface keeps its own copy whatever this says.
use super::*;
use crate::presentation::read_issue;
use orbit_core::{reads::ReadRequest, reminders::ReminderAnswer};
use serde_json::json;

impl DaemonClient {
    /// Reads the hub's reminder of the meeting of `series_key` and keeps it,
    /// or why it could not be read.
    pub(super) async fn read_reminder(&self, series_key: String) {
        let answer = match orbit_core::series_title::title_digest(&series_key) {
            None => Err(read_issue("read_reminder_title", json!({}))),
            Some(key) => note_read::read_page(
                &self.inner.daemon,
                ReadRequest::PersonalReminder { key, cursor: None },
            )
            .await
            .and_then(|page| {
                page.items
                    .into_iter()
                    .next()
                    .ok_or_else(|| read_issue("read_reminder_empty", json!({})))
            })
            .and_then(|item| {
                serde_json::from_value::<ReminderAnswer>(item)
                    .map(|answer| answer.reminder)
                    .map_err(|error| {
                        read_issue(
                            "read_reminder_invalid",
                            json!({"diagnostic": error.to_string()}),
                        )
                    })
            }),
        };
        let read = crate::ReminderRead { series_key, answer };
        self.update_local(|snapshot| {
            if let Some(notes) = snapshot.notes.as_mut() {
                notes.reminder = Some(Box::new(read));
            }
        });
    }

    /// The daemon order for an edit of the reminder of `series_key`.
    pub(super) fn reminder_write(
        series_key: &str,
        text: String,
        edited_at_ms: i64,
    ) -> Result<Command, AppError> {
        let key = orbit_core::series_title::title_digest(series_key).ok_or_else(|| AppError {
            code: "reminder_without_title".into(),
            message: "The meeting has no title for saving a reminder".into(),
            refusal: None,
        })?;
        Ok(Command::ReminderWrite {
            key,
            text,
            edited_at_ms,
        })
    }
}

#[cfg(test)]
#[path = "reminders_tests.rs"]
mod tests;
