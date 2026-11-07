//! «Recordatorio personal» on the hub (hub-v1, `/v1/reminders/{key}`): one
//! meeting's reminder, named by its title's digest.
use super::{Answer, NoteHub};
use orbit_core::reminders::ReminderAnswer;
use serde_json::json;

impl NoteHub {
    /// `GET /v1/reminders/{key}`.
    pub async fn reminder(&self, key: &str) -> Answer<ReminderAnswer> {
        self.send(
            reqwest::Method::GET,
            self.endpoint(&["v1", "reminders", key]),
            None,
        )
        .await
    }

    /// `PUT /v1/reminders/{key}` `{text, edited_at_ms}`: the answer is the
    /// one the hub kept, the latest.
    pub async fn save_reminder(
        &self,
        key: &str,
        text: &str,
        edited_at_ms: i64,
    ) -> Answer<ReminderAnswer> {
        self.send(
            reqwest::Method::PUT,
            self.endpoint(&["v1", "reminders", key]),
            Some(json!({"text": text, "edited_at_ms": edited_at_ms})),
        )
        .await
    }
}
