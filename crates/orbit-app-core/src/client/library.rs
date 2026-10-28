//! The note library read, page after page, into `notes.library`.
use super::*;
use crate::presentation::read_issue;
use orbit_core::{notes::LibraryItem, reads::ReadRequest, topics::Cursor};
use serde_json::json;

/// Why the library could not be read, or its items.
async fn library_items(daemon: &LocalClient) -> Result<Vec<LibraryItem>, String> {
    let mut cursor: Option<Cursor> = None;
    let mut visited = Vec::new();
    let mut items = Vec::new();
    loop {
        let page = note_read::read_page(
            daemon,
            ReadRequest::NoteLibrary {
                cursor: cursor.clone(),
            },
        )
        .await?;
        for raw in page.items {
            items.push(serde_json::from_value::<LibraryItem>(raw).map_err(|error| {
                read_issue(
                    "read_library_invalid",
                    json!({"diagnostic": error.to_string()}),
                )
            })?);
        }
        match page.next {
            Some(next) if !visited.contains(&next) => {
                visited.push(next.clone());
                cursor = Some(next);
            }
            Some(_) => {
                return Err(read_issue(
                    "read_library_cursor",
                    json!({"count": items.len(), "total": page.total}),
                ));
            }
            None => return Ok(items),
        }
    }
}

impl DaemonClient {
    /// Reads the library and publishes it. A failed read keeps what was
    /// shown and says why; nothing is shown as empty for lack of a read.
    pub(super) async fn read_library(&self) {
        let _reading = self.inner.library_io.lock().await;
        let answer = library_items(&self.inner.daemon).await;
        self.update_local(|snapshot| {
            let Some(notes) = snapshot.notes.as_mut() else {
                return;
            };
            match answer {
                Ok(items) => {
                    notes.library =
                        Some(Box::new(crate::projection::library::project_library(items)));
                }
                Err(cause) => {
                    let mut library = notes.library.take().unwrap_or_else(|| {
                        // Never read: unknown, not «no templates».
                        let mut empty = crate::projection::library::project_library(Vec::new());
                        empty.dictionaries = Some(crate::NoteLibraryListSnapshot {
                            state: crate::NoteTemplatesState::Unknown,
                            detail: None,
                        });
                        empty.templates.state = crate::NoteTemplatesState::Unknown;
                        empty.links.state = crate::NoteTemplatesState::Unknown;
                        Box::new(empty)
                    });
                    library.read = crate::NoteLibraryReadState::Failed;
                    library.read_detail = Some(cause);
                    notes.library = Some(library);
                }
            }
        });
    }
}
