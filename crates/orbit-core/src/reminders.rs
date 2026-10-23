//! «Recordatorio personal» in Hoy: what the owner wants to remember for a
//! meeting, kept by the hub so every Mac paired with it shows it (hub-v1,
//! `/v1/reminders/{key}`), and never put into a note. A meeting's reminder is
//! named by its title's digest (`series_title::title_digest`), never by its
//! title. The latest edit wins, by when it was written.
use serde::{Deserialize, Serialize};

/// A reminder as the hub keeps it: its text (empty when it was emptied, which
/// is an edit too) and when the Mac that wrote it wrote it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reminder {
    pub text: String,
    pub edited_at_ms: i64,
}

/// The hub's answer for one meeting: `reminder: None` is the hub saying
/// nobody wrote one, a fact and not an unknown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReminderAnswer {
    pub key: String,
    pub reminder: Option<Reminder>,
}
