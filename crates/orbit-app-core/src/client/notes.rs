//! The notes list, read from the daemon, which reads it from the hub
//! and adds this Mac's own recordings.
use super::*;
use crate::presentation::read_issue;
use orbit_core::{
    notes::{ListItem, MeetingRow},
    reads::ReadRequest,
    refusal::Refusal,
    topics::{Cursor, Page},
};
use serde_json::{Value, json};

/// Pages of the list one refresh may read: fifty meetings each, so a
/// thousand notes. It protects the app from a cursor that never ends, not
/// how many notes the user keeps: past it the list says there are more.
const MAX_LIST_PAGES: usize = 20;

/// How long the list waits before it is read again after a failure whose
/// cause passes (a hub that answered 502 while it restarted, a page slower
/// than the app's ten-second call to the daemon after the Mac woke): twice,
/// after one second and then three more. It keeps a blip from saying the
/// list failed and from reaching the report log as one, and it costs a
/// refresh at most four seconds more. Nothing measured sets these numbers;
/// a failure that outlasts them is said, with its cause, and the next change
/// of the list's head reads it again as before.
const LIST_RETRY_DELAYS: [std::time::Duration; 2] = [
    std::time::Duration::from_secs(1),
    std::time::Duration::from_secs(3),
];

/// Why the list was not read: the detail as before, its cause as a code,
/// and whether that cause passes, so reading again may succeed.
struct Unlisted {
    detail: String,
    cause: String,
    passing: bool,
}

impl Unlisted {
    fn of_refusal(refusal: &Refusal, detail: String) -> Self {
        use orbit_core::refusal::{InternalCause, Retry};
        let (cause, passing) = match refusal {
            // The app's own check of the answer (`read_list_invalid`…).
            Refusal::Internal {
                cause: InternalCause::AppOperation { code, .. },
            } => (code.clone(), false),
            other => (
                other.code().name().to_owned(),
                matches!(other.retry(), Retry::CheckOutcome | Retry::AfterMs(_)),
            ),
        };
        Self {
            detail,
            cause,
            passing,
        }
    }

    /// A detail this client wrote: a refusal in its wire form.
    fn of_detail(detail: String) -> Self {
        match serde_json::from_str::<Refusal>(&detail) {
            Ok(refusal) => Self::of_refusal(&refusal, detail),
            Err(_) => Self {
                detail,
                cause: "unrecognized".into(),
                passing: false,
            },
        }
    }
}

/// One page of the list, or why it was not read. The call to the daemon
/// says its own cause: the daemon's typed refusal, its orderly closure, or
/// the app's ten-second call ending first (the hub may take up to its 90 s).
async fn list_page(daemon: &LocalClient, request: ReadRequest) -> Result<Page<Value>, Unlisted> {
    let response = daemon.read_topic(request.clone()).await.map_err(|error| {
        let detail = error.to_string();
        if let Some(refusal) = error.downcast_ref::<Refusal>() {
            return Unlisted::of_refusal(refusal, detail);
        }
        let (cause, passing) = if detail == "orbit_closing" {
            ("orbit_closing", false)
        } else if error
            .downcast_ref::<tokio::time::error::Elapsed>()
            .is_some()
        {
            ("daemon_call_timeout", true)
        } else {
            ("daemon_call_failed", false)
        };
        Unlisted {
            detail,
            cause: cause.into(),
            passing,
        }
    })?;
    note_read::checked_page(response, &request).map_err(Unlisted::of_detail)
}

/// The list as one refresh read it.
#[derive(Default)]
struct ListRead {
    rows: Vec<MeetingRow>,
    failure: Option<Unlisted>,
    needs_hub: bool,
    more: bool,
}

impl DaemonClient {
    /// Reads the list again: as many pages as are shown, at least one. A
    /// failure whose cause passes is read again by itself
    /// ([`LIST_RETRY_DELAYS`]) before the list says it failed.
    pub(super) async fn refresh_notes(&self) -> Result<AppSnapshot, AppError> {
        let mut writing = self.inner.list_io.lock().await;
        let wanted = self
            .inner
            .notes_pages
            .load(Ordering::Acquire)
            .clamp(1, MAX_LIST_PAGES);
        let mut read = self.read_list(wanted).await;
        for delay in LIST_RETRY_DELAYS {
            if !read.failure.as_ref().is_some_and(|failure| failure.passing) {
                break;
            }
            tokio::time::sleep(delay).await;
            read = self.read_list(wanted).await;
        }
        let ListRead {
            rows,
            failure,
            needs_hub,
            more,
        } = read;
        let (now_ms, zone) = self.inner.calendar.lock().await.local_now();
        let list = rows
            .into_iter()
            .map(|row| crate::projection::notes::project_row(row, now_ms, &zone))
            .collect::<Vec<_>>();
        let ready = crate::projection::notes::follow_writing(&mut writing, &list)
            .map(|item| (item.recording_id.clone(), item.title.clone()));
        Ok(self.update_local(|snapshot| {
            if let Some(notes) = snapshot.notes.as_mut() {
                if let Some((recording_id, title)) = ready {
                    notes.ready = Some(crate::NoteReadySnapshot {
                        recording_id,
                        title,
                        sequence: notes
                            .ready
                            .as_ref()
                            .map_or(1, |last| last.sequence.saturating_add(1)),
                    });
                }
                notes.projection_clock = Some(Box::new((now_ms, zone.clone())));
                notes.list = list;
                notes.index = Some(Box::new(crate::NotesIndexSnapshot {
                    state: if failure.is_some() {
                        crate::NotesIndexState::Failed
                    } else if needs_hub {
                        crate::NotesIndexState::NeedsHub
                    } else {
                        crate::NotesIndexState::Available
                    },
                    more,
                    cause: failure.as_ref().map(|failure| failure.cause.clone()),
                    detail: failure.map(|failure| failure.detail),
                }));
            }
        }))
    }

    /// One read of the list's shown pages, which stops at the first page
    /// that fails.
    async fn read_list(&self, wanted: usize) -> ListRead {
        let mut read = ListRead::default();
        let mut cursor: Option<Cursor> = None;
        for page_number in 0..wanted {
            let page = match list_page(
                &self.inner.daemon,
                ReadRequest::NotesPage {
                    cursor: cursor.clone(),
                },
            )
            .await
            {
                Ok(page) => page,
                Err(cause) => {
                    read.failure = Some(cause);
                    break;
                }
            };
            read_rows(page.items, &mut read);
            read.more = page.next.is_some();
            match page.next {
                Some(next) if page_number + 1 < wanted => cursor = Some(next),
                Some(_) | None => break,
            }
        }
        read
    }

    /// «Cargar más»: one page more is shown from now on.
    pub(super) async fn read_more_notes(&self) -> Result<AppSnapshot, AppError> {
        self.inner.notes_pages.fetch_add(1, Ordering::AcqRel);
        self.refresh_notes().await
    }
}

fn read_rows(items: Vec<Value>, read: &mut ListRead) {
    for item in items {
        match serde_json::from_value::<ListItem>(item) {
            Ok(ListItem::Meeting(row)) => read.rows.push(row),
            Ok(ListItem::HubRefused { hub_refusal }) if hub_refusal.needs_the_hub() => {
                read.needs_hub = true;
            }
            Ok(ListItem::HubRefused { hub_refusal }) => {
                read.failure = Some(Unlisted::of_refusal(&hub_refusal, hub_refusal.to_string()));
            }
            Err(error) => {
                read.failure = Some(Unlisted::of_detail(read_issue(
                    "read_list_invalid",
                    json!({"diagnostic": error.to_string()}),
                )))
            }
        }
    }
}
