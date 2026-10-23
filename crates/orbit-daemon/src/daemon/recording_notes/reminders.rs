//! «Recordatorio personal»: one meeting's reminder, read from the hub and
//! saved to it off the loop. The Mac keeps its own copy (the interface's) and
//! sends it again when the hub can be reached; the hub keeps the latest edit.
use super::super::{Runtime, ipc_replies::command_response, reads::bounded_page};
use crate::hub_notes::{Answer, NoteHub};
use orbit_core::{
    reads::ReadId,
    refusal::{Refusal, Service},
    reminders::ReminderAnswer,
    topics::{Knowledge, Page, TopicId},
};
use serde_json::Value;
use std::collections::BTreeMap;

/// What the hub answered, with its 404 named as a hub without the route (up
/// to 0.11): unknown, never «no reminder», so the Mac keeps its own copy.
fn answered(answer: Answer<ReminderAnswer>, key: &str) -> Result<ReminderAnswer, Refusal> {
    let answer = match answer {
        Answer::Refused { status: 404, .. } => Err(Refusal::UpstreamFailed {
            service: Service::Hub,
            status: Some(404),
        }),
        other @ (Answer::Value(_) | Answer::Refused { .. } | Answer::Unreachable(_)) => {
            other.into_result()
        }
    }?;
    if answer.key == key {
        Ok(answer)
    } else {
        Err(Refusal::HubReplyInvalid {
            field: "key".into(),
            received_bytes: None,
        })
    }
}

/// The reminder of meeting `key`, as one item; `reminder: null` is the hub's
/// fact that nobody wrote one.
pub(super) async fn read(
    hub: Result<NoteHub, Refusal>,
    key: String,
    revision: u64,
) -> Knowledge<Page<Value>> {
    let read = match hub {
        Ok(hub) => answered(hub.reminder(&key).await, &key),
        Err(refusal) => Err(refusal),
    };
    match read {
        Ok(answer) => bounded_page(&[answer], None, 1, revision, ReadId::PersonalReminder),
        Err(refusal) => Knowledge::Absent { refusal },
    }
}

impl Runtime {
    /// `Command::ReminderWrite`, answered once the hub kept it (or a later
    /// edit), or with why it could not: the interface keeps its copy and
    /// sends it again. A privacy stop while it runs ends it.
    pub(in crate::daemon) fn serve_reminder_write(
        &mut self,
        (key, text, edited_at_ms): (String, String, i64),
        request_id: String,
        reply: orbit_ipc::ReplySender,
        (epoch, revisions): (Option<String>, BTreeMap<TopicId, u64>),
    ) {
        let hub = self
            .hub_access()
            .map(|_| ())
            .and_then(|()| self.note_hub().ok_or(Refusal::NotPaired));
        let mut privacy_epoch = self.reads.privacy_epoch_rx.clone();
        let started_at_epoch = *privacy_epoch.borrow_and_update();
        let log_root = self.paths.root.clone();
        self.reads.tasks.push(tokio::spawn(async move {
            let saved = async {
                let hub = hub?;
                answered(hub.save_reminder(&key, &text, edited_at_ms).await, &key)
            };
            let refusal = tokio::select! {
                _ = privacy_epoch.changed() => Some(Refusal::PrivacyStopped { since_ms: None }),
                saved = saved => saved.err(),
            }
            .or_else(|| {
                (started_at_epoch != *privacy_epoch.borrow())
                    .then_some(Refusal::PrivacyStopped { since_ms: None })
            });
            let response = command_response(request_id, None, refusal, epoch, revisions);
            if reply.send(response).is_err() {
                crate::diagnostics::DiagnosticLog::in_root(&log_root)
                    .record("ipc_reply_receiver_gone", &[("command", "reminder_write")]);
            }
        }));
    }
}

#[cfg(test)]
#[path = "reminder_tests.rs"]
mod tests;
