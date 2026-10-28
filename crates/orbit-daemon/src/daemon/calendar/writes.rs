//! Writes the user makes from Orbit, through the helper's single EventKit slot.
//!
//! A write is validated in orbit-core, queued, and sent when the slot is free,
//! before any read: a person is waiting on it, and it changes what the reads
//! would return. Its outcome is kept under the client's `request_id`: saved
//! (with the occurrence as it is now), deleted, refused by name with its
//! numbers, or unknown because its answer was lost. A write is never sent a
//! second time (AGENTS.md: never retry an input action merely because its
//! receipt was lost); when its outcome is unknown, or it changed the store,
//! the agenda on screen is read again.
use anyhow::Result;
use orbit_core::{
    calendar::{
        CalendarDelete, CalendarDeleteResult, CalendarSave, CalendarSaveResult, CalendarWriteFacts,
        CalendarWriteRefusal,
    },
    calendar_control::{CalendarWriteOperation, CalendarWriteStage, CalendarWriteView},
    wire::native::decode_result_body,
};

use super::*;

/// How many writes the daemon keeps track of at once: waiting for the slot,
/// with EventKit, or recently finished.
///
/// Protects: the daemon's memory and the IPC view that carries every record
/// to the app, against a client that sends writes faster than EventKit takes
/// them. Why 16: a person makes a few changes a second at most (a drag, then
/// an edit), and each write takes milliseconds; sixteen waiting means
/// something upstream is wrong, not that the user is busy. What it is not: a
/// limit on how many events they can change. A finished record makes room for a new
/// write; a write that finds sixteen still unfinished is refused by name with
/// that count and this limit (`calendar_write_queue_full`), and nothing is sent.
pub(super) const WRITE_RECORDS: usize = 16;

/// The largest occurrence a write record carries, in JSON bytes.
///
/// Protects the daemon's retained write records in memory. Why 128 KiB: the existing
/// allocation allows an event whose title is at its own limit (96 KiB) with its
/// guests. Only the newest saved write keeps its occurrence — the agenda is
/// read again after every write and the read brings the rest — so all the
/// records together never carry more than this. What it is not: a limit on
/// the event, which is saved whole; past it the record says the occurrence's
/// bytes and this limit, and the read shows the event.
pub(super) const MAX_WRITE_OCCURRENCE_BYTES: usize = 128 * 1024;

/// One write waiting for, or holding, the EventKit slot.
#[derive(Clone, Debug)]
pub(super) enum WriteRequest {
    Save(CalendarSave),
    Delete(CalendarDelete),
}

impl WriteRequest {
    fn operation(&self) -> CalendarWriteOperation {
        match self {
            Self::Save(CalendarSave::Create(_)) => CalendarWriteOperation::Create,
            Self::Save(CalendarSave::Change(_)) => CalendarWriteOperation::Update,
            Self::Delete(_) => CalendarWriteOperation::Delete,
        }
    }

    pub(super) fn command(&self) -> (&'static str, Value) {
        match self {
            Self::Save(save) => ("calendar_save", json!({ "calendar_save": save })),
            Self::Delete(delete) => ("calendar_delete", json!({ "calendar_delete": delete })),
        }
    }
}

/// What a write's waiting ended in, for a cause the daemon itself observed.
/// A write still in the queue was never sent; one with the helper may have
/// been applied.
fn wait_cause(code: &str) -> &str {
    match code {
        // The same restart a read reports as «read restarted»; for a write
        // it is the helper that went away under it.
        "calendar_read_restarted" => "calendar_helper_restarted",
        other => other,
    }
}

impl Runtime {
    /// `Save` or `Delete` from a client: refused here by name when Orbit can
    /// already tell, otherwise queued for the slot.
    pub(super) fn calendar_write_command(
        &mut self,
        request_id: String,
        request: WriteRequest,
        refusal: Option<CalendarWriteRefusal>,
    ) -> Result<()> {
        let operation = request.operation();
        let refusal = refusal.or_else(|| self.local_write_refusal(&request_id));
        let unfinished = self
            .calendar
            .view
            .writes
            .iter()
            .filter(|write| {
                matches!(
                    write.stage,
                    CalendarWriteStage::Queued | CalendarWriteStage::Writing
                )
            })
            .count();
        if let Some(refusal) = refusal {
            return Err(self
                .calendar
                .refuse_write_locally(request_id, operation, refusal));
        }
        if unfinished >= WRITE_RECORDS {
            // Nothing was sent; the count and the limit go to whoever asked.
            return Err(CalendarWriteRefusal::QueueFull {
                unfinished: u64::try_from(unfinished).unwrap_or(u64::MAX),
                limit: WRITE_RECORDS as u64,
            }
            .into());
        }
        self.calendar.record_write(CalendarWriteView {
            request_id: request_id.clone(),
            operation,
            stage: CalendarWriteStage::Queued,
            occurrence: None,
            refusal: None,
            detail: "Waiting for EventKit to finish the previous work.".into(),
        });
        self.calendar.write_queue.push_back((request_id, request));
        self.pump_calendar();
        Ok(())
    }

    /// Why Orbit refuses a write by itself: a reused identifier, the
    /// user's stop, or no calendar reader on the Mac.
    fn local_write_refusal(&self, request_id: &str) -> Option<CalendarWriteRefusal> {
        let unavailable =
            !self.state.capabilities().native_helper() || !self.helper.calendar_backend_available();
        let reused = self
            .calendar
            .view
            .writes
            .iter()
            .any(|write| write.request_id == request_id);
        let cause = if reused {
            "calendar_write_request_reused"
        } else if self.state.privacy_stopped() {
            "privacy_stopped"
        } else if unavailable {
            "calendar_backend_unavailable"
        } else {
            return None;
        };
        Some(CalendarWriteRefusal::NotWritten {
            cause: cause.into(),
        })
    }

    /// Only a write still with the helper may end; one already ended (by a
    /// stop, a restart or a lost answer) keeps its outcome.
    fn finish_write(
        &mut self,
        request_id: &str,
        request: &WriteRequest,
        stage: CalendarWriteStage,
        occurrence: Option<orbit_core::calendar::CalendarOccurrence>,
        refusal: Option<CalendarWriteRefusal>,
    ) {
        if self.calendar.write_stage(request_id) != Some(CalendarWriteStage::Writing) {
            return;
        }
        let mut detail = match (&stage, &refusal) {
            (_, Some(refusal)) => refusal.notice(),
            (CalendarWriteStage::Deleted, None) => "Event deleted from the Mac calendar.".into(),
            _ => "Change saved to the Mac calendar.".into(),
        };
        let bytes = occurrence.as_ref().map_or(0, |value| {
            serde_json::to_vec(value).map_or(usize::MAX, |json| json.len())
        });
        let occurrence = if bytes > MAX_WRITE_OCCURRENCE_BYTES {
            detail.push_str(&format!(
                " The saved event is {bytes} bytes, exceeding the {MAX_WRITE_OCCURRENCE_BYTES} that fit this notice; it appears when the calendar is reread."
            ));
            None
        } else {
            occurrence
        };
        if occurrence.is_some() {
            // Only the newest saved write keeps its occurrence (see
            // `MAX_WRITE_OCCURRENCE_BYTES`); the read brings the others.
            for write in &mut self.calendar.view.writes {
                write.occurrence = None;
            }
        }
        let reread = matches!(
            stage,
            CalendarWriteStage::Saved
                | CalendarWriteStage::Deleted
                | CalendarWriteStage::OutcomeUnknown
        ) || matches!(refusal, Some(CalendarWriteRefusal::EventNotFound { .. }));
        self.calendar.record_write(CalendarWriteView {
            request_id: request_id.into(),
            operation: request.operation(),
            stage,
            occurrence: occurrence.map(Box::new),
            refusal,
            detail,
        });
        if reread {
            // The store changed, or may have: what is on screen is read again.
            // EventKit also says so (`calendar_invalidated`); this does not
            // wait for it.
            self.restart_screen_read();
        }
    }

    /// A write that ended without an answer to use. Before it was sent
    /// (`sent == false`) the store is known to be unchanged; after, nobody
    /// can know, and it is never sent again.
    pub(super) fn calendar_write_failed(
        &mut self,
        request_id: &str,
        request: &WriteRequest,
        code: &str,
        sent: bool,
    ) {
        let cause = wait_cause(code).to_owned();
        if sent {
            self.finish_write(
                request_id,
                request,
                CalendarWriteStage::OutcomeUnknown,
                None,
                Some(CalendarWriteRefusal::OutcomeUnknown { cause }),
            );
        } else {
            self.calendar.record_write(CalendarWriteView {
                request_id: request_id.into(),
                operation: request.operation(),
                stage: CalendarWriteStage::Refused,
                occurrence: None,
                refusal: Some(CalendarWriteRefusal::NotWritten {
                    cause: cause.clone(),
                }),
                detail: CalendarWriteRefusal::NotWritten { cause }.notice(),
            });
        }
    }

    /// The helper refused the write, or said its outcome is unknown.
    pub(super) fn calendar_write_refused(
        &mut self,
        request_id: &str,
        request: &WriteRequest,
        code: &str,
        detail: Option<&Value>,
    ) {
        match CalendarWriteRefusal::from_native(code, detail) {
            Ok(refusal) => {
                let stage = if refusal.known_not_written() {
                    CalendarWriteStage::Refused
                } else {
                    CalendarWriteStage::OutcomeUnknown
                };
                self.finish_write(request_id, request, stage, None, Some(refusal));
            }
            // A refusal that breaks the contract is not a fact about the store.
            Err(_) => {
                self.calendar_write_failed(request_id, request, "calendar_reply_invalid", true)
            }
        }
    }

    /// The helper's answer to a write, admitted for the exact write sent.
    pub(super) fn admit_calendar_write(
        &mut self,
        pending: &Pending,
        request_id: &str,
        request: &WriteRequest,
        value: &Value,
    ) -> Result<()> {
        let facts = CalendarWriteFacts {
            native_instance_id: &pending.host.id,
            privacy_generation: pending.host.generation,
        };
        match request {
            WriteRequest::Save(_) => {
                let result: CalendarSaveResult = decode_result_body("calendar_save", value)?;
                result.validate_response(&facts)?;
                self.finish_write(
                    request_id,
                    request,
                    CalendarWriteStage::Saved,
                    Some(result.occurrence),
                    None,
                );
            }
            WriteRequest::Delete(delete) => {
                let result: CalendarDeleteResult = decode_result_body("calendar_delete", value)?;
                result.validate_response(delete, &facts)?;
                self.finish_write(request_id, request, CalendarWriteStage::Deleted, None, None);
            }
        }
        Ok(())
    }

    /// A stop, a withdrawn permission or a new helper epoch: writes still in
    /// the queue were never sent and are refused by that cause; the one with
    /// the helper, if any, can no longer be answered and its outcome is
    /// unknown. Nothing is sent again.
    pub(super) fn revoke_calendar_writes(&mut self, code: &str) {
        let queued: Vec<(String, WriteRequest)> = self.calendar.write_queue.drain(..).collect();
        for (request_id, request) in queued {
            self.calendar_write_failed(&request_id, &request, code, false);
        }
        if let Some(Job::Write {
            request_id,
            request,
        }) = self.calendar.parked.take()
        {
            self.calendar_write_failed(&request_id, &request, code, false);
        }
        let in_flight = self
            .calendar
            .pending
            .as_ref()
            .and_then(|pending| match &pending.job {
                Job::Write {
                    request_id,
                    request,
                } => Some((request_id.clone(), request.clone())),
                Job::Permission | Job::Inventory { .. } | Job::Window { .. } => None,
            });
        if let Some((request_id, request)) = in_flight {
            self.calendar_write_failed(&request_id, &request, code, true);
        }
    }
}

impl crate::daemon::calendar::CalendarHandle {
    /// Keeps one record per `request_id`, oldest first; a finished one makes
    /// room for a new write (`WRITE_RECORDS`).
    fn record_write(&mut self, record: CalendarWriteView) {
        let writes = &mut self.view.writes;
        if let Some(existing) = writes
            .iter_mut()
            .find(|write| write.request_id == record.request_id)
        {
            *existing = record;
            return;
        }
        while writes.len() >= WRITE_RECORDS {
            let Some(oldest) = writes.iter().position(|write| {
                !matches!(
                    write.stage,
                    CalendarWriteStage::Queued | CalendarWriteStage::Writing
                )
            }) else {
                break;
            };
            writes.remove(oldest);
        }
        writes.push(record);
    }

    fn write_stage(&self, request_id: &str) -> Option<CalendarWriteStage> {
        self.view
            .writes
            .iter()
            .find(|write| write.request_id == request_id)
            .map(|write| write.stage)
    }

    /// The write is now with the helper.
    pub(super) fn write_sent(&mut self, request_id: &str) {
        if let Some(write) = self
            .view
            .writes
            .iter_mut()
            .find(|write| write.request_id == request_id)
        {
            write.stage = CalendarWriteStage::Writing;
            write.detail = "EventKit is applying the change.".into();
        }
    }
}

impl crate::daemon::calendar::CalendarHandle {
    /// A write refused before anything is sent: its record says why, unless
    /// the identifier is another write's, and the caller hears the cause.
    fn refuse_write_locally(
        &mut self,
        request_id: String,
        operation: CalendarWriteOperation,
        refusal: CalendarWriteRefusal,
    ) -> anyhow::Error {
        // The causes Orbit refuses by itself, before anything is sent.
        let cause = match &refusal {
            CalendarWriteRefusal::NotWritten { cause } => Some(cause.as_str()),
            CalendarWriteRefusal::EventNotFound { .. }
            | CalendarWriteRefusal::NotEditable { .. }
            | CalendarWriteRefusal::InvalidChange(_)
            | CalendarWriteRefusal::PermissionDenied { .. }
            | CalendarWriteRefusal::SaveFailed { .. }
            | CalendarWriteRefusal::OutcomeUnknown { .. }
            | CalendarWriteRefusal::QueueFull { .. } => None,
        };
        let notice = match cause {
            Some("privacy_stopped") => {
                "Everything is stopped: Orbit will not change the calendar until rearmed. Nothing changed.".to_owned()
            }
            Some("calendar_backend_unavailable") => {
                "The Mac calendar reader is currently unavailable. Nothing changed.".to_owned()
            }
            Some("calendar_write_request_reused") => format!(
                "A write with identifier {request_id} already exists; each change requires its own. Nothing changed."
            ),
            Some(_) | None => refusal.notice(),
        };
        // Whoever asked hears the same cause, typed where a common
        // refusal exists and by its notice otherwise.
        let error: anyhow::Error = match cause {
            Some("privacy_stopped") => {
                orbit_core::refusal::Refusal::PrivacyStopped { since_ms: None }.into()
            }
            Some("calendar_backend_unavailable") => {
                orbit_core::refusal::Refusal::CalendarBackendUnavailable.into()
            }
            Some(_) | None => refusal.clone().into(),
        };
        // A reused identifier names another write's record: that record
        // stays as it is, and only the caller hears the refusal.
        if cause != Some("calendar_write_request_reused") {
            self.record_write(CalendarWriteView {
                request_id,
                operation,
                stage: CalendarWriteStage::Refused,
                occurrence: None,
                refusal: Some(refusal),
                detail: notice,
            });
        }
        error
    }
}
