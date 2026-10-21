//! SIM-41: the notes list, read from the daemon, which reads it from the hub
//! and adds this Mac's own recordings.
use super::*;
use orbit_core::{notes::ListItem, reads::ReadRequest, topics::Cursor};

/// Pages of the list one refresh may read: fifty meetings each, so a
/// thousand notes. It protects the app from a cursor that never ends, not
/// how many notes Mars keeps: past it the list says there are more.
const MAX_LIST_PAGES: usize = 20;

impl DaemonClient {
    /// Reads the list again: as many pages as are shown, at least one.
    pub(super) async fn refresh_notes(&self) -> Result<AppSnapshot, AppError> {
        let _reading = self.inner.list_io.lock().await;
        let wanted = self
            .inner
            .notes_pages
            .load(Ordering::Acquire)
            .clamp(1, MAX_LIST_PAGES);
        let mut rows = Vec::new();
        let mut cursor: Option<Cursor> = None;
        let mut failure = None;
        let mut needs_hub = false;
        let mut more = false;
        for page_number in 0..wanted {
            let page = match note_read::read_page(
                &self.inner.daemon,
                ReadRequest::NotesPage {
                    cursor: cursor.clone(),
                },
            )
            .await
            {
                Ok(page) => page,
                Err(cause) => {
                    failure = Some(cause);
                    break;
                }
            };
            for item in page.items {
                match serde_json::from_value::<ListItem>(item) {
                    Ok(ListItem::Meeting(row)) => rows.push(row),
                    Ok(ListItem::HubRefused { hub_refusal }) if hub_refusal.needs_the_hub() => {
                        needs_hub = true;
                    }
                    Ok(ListItem::HubRefused { hub_refusal }) => {
                        failure = Some(hub_refusal.to_string());
                    }
                    Err(error) => failure = Some(format!("Nota no válida en la lista: {error}")),
                }
            }
            more = page.next.is_some();
            match page.next {
                Some(next) if page_number + 1 < wanted => cursor = Some(next),
                Some(_) | None => break,
            }
        }
        let (now_ms, zone) = self.inner.calendar.lock().await.local_now();
        let list = rows
            .into_iter()
            .map(|row| crate::projection::notes::project_row(row, now_ms, &zone))
            .collect::<Vec<_>>();
        Ok(self.update_local(|snapshot| {
            if let Some(notes) = snapshot.notes.as_mut() {
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
                    detail: failure,
                }));
            }
        }))
    }

    /// «Cargar más»: one page more is shown from now on.
    pub(super) async fn read_more_notes(&self) -> Result<AppSnapshot, AppError> {
        self.inner.notes_pages.fetch_add(1, Ordering::AcqRel);
        self.refresh_notes().await
    }
}
