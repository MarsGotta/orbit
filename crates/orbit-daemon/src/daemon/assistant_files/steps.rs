//! A files request's two steps: the folders resolved, then the order admitted
//! inside them and its answer, fitted to the page and sent to Deimos.
use super::*;

/// The bookmark of one of the request's folders.
pub(super) fn bookmark(request: &InFlight, id: &str) -> Option<FolderBookmark> {
    request
        .saved
        .iter()
        .find(|folder| folder.id == id)
        .map(FolderBookmark::from)
}

/// A search of every open folder. A folder inside another (or the same one
/// twice) is walked once, as part of the outer one: the helper walks each
/// root whole.
pub(super) fn search_every_folder(
    request: &InFlight,
    reply: &FoldersReply,
    open: &[(String, String)],
    query: &str,
    content: bool,
) -> Result<(&'static str, Value, Option<Admitted>), Refusal> {
    let covered = |index: usize, path: &str| {
        open.iter().enumerate().any(|(other_index, (_, other))| {
            other_index != index
                && files::is_within(path, other)
                && (path != other || other_index < index)
        })
    };
    let roots: Vec<SearchRoot> = open
        .iter()
        .enumerate()
        .filter(|(index, (_, path))| !covered(*index, path))
        .filter_map(|(_, (id, path))| {
            bookmark(request, id).map(|folder| SearchRoot {
                folder,
                path: path.clone(),
            })
        })
        .collect();
    if roots.is_empty() {
        return Err(first_unavailable(&request.saved, &reply.folders));
    }
    Ok((
        "assistant_files_search",
        json!({"roots":roots,"query":query,"content":content,
            "limits":SearchLimits::ORBIT}),
        None,
    ))
}

/// A list, a search or a read inside one folder, admitted against the
/// folders the helper resolved.
pub(super) fn order_in_folder(
    ask: &FilesAsk,
    request: &InFlight,
    reply: &FoldersReply,
) -> Result<(&'static str, Value, Option<Admitted>), Refusal> {
    let path = ask.path().unwrap_or_default();
    let admitted = files::admit(ask.asked(), path, &request.saved, reply)?;
    let folder = bookmark(request, &admitted.folder).ok_or(Refusal::HelperReplyInvalid {
        field: "assistant_folders.folders.id".into(),
        expected: "a folder of the request".into(),
        received_type: "unknown_id".into(),
    })?;
    let (name, fields) = match ask {
        FilesAsk::List { after_index, .. } => (
            "assistant_files_list",
            json!({"folder":folder,"path":admitted.path,"after_index":after_index,
                "max_entries":files::MAX_LIST_ENTRIES}),
        ),
        FilesAsk::Search { query, content, .. } => (
            "assistant_files_search",
            json!({"roots":[SearchRoot{folder, path: admitted.path.clone()}],
                "query":query,"content":content,"limits":SearchLimits::ORBIT}),
        ),
        FilesAsk::Read {
            after_index,
            after_page,
            ..
        } => (
            "assistant_files_read",
            json!({"folder":folder,"path":admitted.path,
                "read": read_as(admitted.content, *after_index, *after_page)}),
        ),
    };
    Ok((name, fields, Some(admitted)))
}

/// A folder's listing, fitted to the page's budget.
pub(super) fn list_result(
    admitted: Option<&Admitted>,
    reply: ListReply,
    after_index: u64,
) -> BridgeResult {
    let path = admitted.map(|admitted| admitted.path.clone());
    let budget = files::page_budget(path.as_deref().unwrap_or_default());
    let (entries, next_after_index) =
        files::fit_entries(reply.entries, after_index, reply.next_after_index, budget);
    BridgeResult::FilesList(FilesListResult {
        path,
        entries,
        total: reply.total,
        after_index,
        next_after_index,
    })
}

/// A search's hits, fitted to the page's budget.
pub(super) fn search_result(
    query: &str,
    shown: Option<String>,
    content: bool,
    reply: SearchReply,
) -> BridgeResult {
    let budget = files::page_budget(query)
        .saturating_sub(files::encoded_len(shown.as_deref().unwrap_or_default()));
    let (results, stopped) = files::fit_hits(reply.results, reply.stopped, budget);
    BridgeResult::FilesSearch(FilesSearchResult {
        query: query.to_owned(),
        path: shown,
        content,
        results,
        visited: reply.visited,
        files_read: reply.files_read,
        bytes_read: reply.bytes_read,
        partial_files: reply.partial_files,
        stopped,
    })
}

/// How the helper reads an admitted file.
pub(super) fn read_as(content: Option<ContentKind>, after_index: u64, after_page: u32) -> ReadAs {
    match content.unwrap_or(ContentKind::Text) {
        ContentKind::Text => ReadAs::Text {
            after_index,
            max_bytes: files::MAX_TEXT_PAGE_BYTES,
        },
        ContentKind::Pdf => ReadAs::Pdf {
            after_page,
            after_index,
            max_bytes: files::MAX_TEXT_PAGE_BYTES,
            max_pages: files::MAX_PDF_PAGES_PER_ANSWER,
        },
        ContentKind::Image => ReadAs::Image {
            max_png_bytes: u32::try_from(orbit_core::sources::MAX_SOURCE_BYTES).unwrap_or(u32::MAX),
            max_width: orbit_core::sources::MAX_WINDOW_WIDTH,
            max_height: orbit_core::sources::MAX_WINDOW_HEIGHT,
        },
    }
}

/// The folders themselves as a listing: each by its whole path, with why it
/// does not open when it does not.
pub(super) fn roots_page(
    request: &InFlight,
    reply: &FoldersReply,
    after_index: u64,
) -> BridgeResult {
    let paths = files::folder_paths(&request.saved, &reply.folders);
    let entries: Vec<Entry> = request
        .saved
        .iter()
        .zip(paths)
        .map(|(folder, path)| Entry {
            name: path,
            kind: EntryKind::Folder,
            size_bytes: None,
            modified_ms: None,
            unavailable: reply.folders.iter().find_map(|report| match report {
                FolderReport::Unavailable { id, cause } if *id == folder.id => Some(cause.clone()),
                FolderReport::Unavailable { .. } | FolderReport::Accessible { .. } => None,
            }),
        })
        .collect();
    let total = entries.len() as u64;
    let skipped: Vec<Entry> = entries
        .into_iter()
        .skip(usize::try_from(after_index).unwrap_or(usize::MAX))
        .collect();
    let (entries, next_after_index) =
        files::fit_entries(skipped, after_index, None, files::MAX_PAGE_BYTES);
    BridgeResult::FilesList(FilesListResult {
        path: None,
        entries,
        total,
        after_index,
        next_after_index,
    })
}

/// The answer to a request whose folder was removed while the helper read
/// it: the path is no longer inside the folders that remain.
pub(super) fn removed_meanwhile(request: &InFlight) -> Refusal {
    if request.saved.is_empty() {
        return Refusal::NoFolders;
    }
    Refusal::OutsideFolders {
        path: request.ask.path().unwrap_or_default().to_owned(),
        resolved: None,
        folders: request
            .saved
            .iter()
            .map(|folder| folder.path.clone())
            .collect(),
    }
}

impl Runtime {
    pub(super) async fn on_resolved(
        &mut self,
        request: &InFlight,
        response: &NativeResponseFields,
    ) -> Result<(), Refusal> {
        if request.saved.is_empty() {
            return Err(Refusal::NoFolders);
        }
        if let Some(refusal) = Self::helper_refusal(request, response) {
            self.files.note_step_folder(request, response);
            return Err(refusal);
        }
        let mut reply = Self::folders_reply(response)?;
        // A folder removed while this step ran is not one of the request's.
        reply
            .folders
            .retain(|report| request.saved.iter().any(|folder| folder.id == report.id()));
        self.take_reports(&reply.folders).await;
        let open: Vec<(String, String)> = reply
            .folders
            .iter()
            .filter_map(|report| match report {
                FolderReport::Accessible { id, path, .. } => Some((id.clone(), path.clone())),
                FolderReport::Unavailable { .. } => None,
            })
            .collect();
        let (name, fields, admitted) = match &request.ask {
            FilesAsk::List {
                path: None,
                after_index,
            } => {
                self.answer_files(&request.ask_id, roots_page(request, &reply, *after_index));
                return Ok(());
            }
            FilesAsk::Search {
                query,
                path: None,
                content,
            } => search_every_folder(request, &reply, &open, query, *content)?,
            ask @ (FilesAsk::List { .. } | FilesAsk::Search { .. } | FilesAsk::Read { .. }) => {
                order_in_folder(ask, request, &reply)?
            }
        };
        let native_id = self.helper.send_files_command(name, fields)?;
        // `started` stays the request's: its bound counts both steps.
        self.files.request = Some(InFlight {
            native_id,
            stage: Stage::Working { admitted, open },
            ..request.clone()
        });
        Ok(())
    }

    pub(super) async fn on_worked(
        &mut self,
        request: &InFlight,
        admitted: Option<&Admitted>,
        open: &[(String, String)],
        response: &NativeResponseFields,
    ) -> Result<(), Refusal> {
        let kept = |id: &str| request.saved.iter().any(|folder| folder.id == id);
        // Removed while the helper read it: what was read there is dropped.
        if let Some(admitted) = admitted
            && !kept(&admitted.folder)
        {
            return Err(removed_meanwhile(request));
        }
        if let Some(refusal) = Self::helper_refusal(request, response) {
            self.files.note_step_folder(request, response);
            return Err(refusal);
        }
        let result = response
            .result
            .as_ref()
            .ok_or(Refusal::HelperReplyInvalid {
                field: "result".into(),
                expected: "object".into(),
                received_type: "absent".into(),
            })?;
        match &request.ask {
            FilesAsk::List { after_index, .. } => {
                let reply: ListReply = decode_result_body("assistant_files_list", result)
                    .map_err(Refusal::helper_reply_invalid)?;
                self.answer_files(&request.ask_id, list_result(admitted, reply, *after_index));
            }
            FilesAsk::Search {
                query,
                path,
                content,
            } => {
                let mut reply: SearchReply = decode_result_body("assistant_files_search", result)
                    .map_err(Refusal::helper_reply_invalid)?;
                // A search of every folder, one of them removed meanwhile.
                if admitted.is_none() && open.iter().any(|(id, _)| !kept(id)) {
                    if request.saved.is_empty() {
                        return Err(Refusal::NoFolders);
                    }
                    reply.results.retain(|hit| {
                        open.iter()
                            .any(|(id, root)| kept(id) && files::is_within(&hit.path, root))
                    });
                }
                let shown = admitted
                    .map(|admitted| admitted.path.clone())
                    .or_else(|| path.clone());
                self.answer_files(
                    &request.ask_id,
                    search_result(query, shown, *content, reply),
                );
            }
            FilesAsk::Read { .. } => {
                let admitted = admitted.ok_or(Refusal::InvalidParameter {
                    parameter: Parameter::Request,
                    expected: Expected::ValidSelection,
                })?;
                let reply: ReadReply = decode_result_body("assistant_files_read", result)
                    .map_err(Refusal::helper_reply_invalid)?;
                self.answer_read(request, admitted, reply).await?;
            }
        }
        Ok(())
    }

    pub(super) async fn answer_read(
        &mut self,
        request: &InFlight,
        admitted: &Admitted,
        reply: ReadReply,
    ) -> Result<(), Refusal> {
        let budget = files::page_budget(&admitted.path);
        let content = match reply {
            ReadReply::Text {
                text,
                after_index,
                next_after_index,
                ..
            } => {
                let (text, next_after_index) =
                    files::fit_text(text, after_index, next_after_index, budget);
                FileContent::Text {
                    text,
                    after_index,
                    next_after_index,
                }
            }
            ReadReply::Pdf {
                pages,
                page_count,
                after_page,
                after_index,
                next_after_page,
                next_after_index,
            } => {
                let next = next_after_page.map(|page| (page, next_after_index.unwrap_or(0)));
                let (pages, next) = files::fit_pages(pages, after_page, after_index, next, budget);
                FileContent::Pdf {
                    pages,
                    page_count,
                    after_page,
                    after_index,
                    next_after_page: next.map(|(page, _)| page),
                    next_after_index: next.map(|(_, index)| index),
                }
            }
            ReadReply::Image {
                png_base64,
                width,
                height,
                source_width,
                source_height,
            } => {
                return self
                    .answer_image(
                        request,
                        admitted,
                        &png_base64,
                        (width, height),
                        (source_width, source_height),
                    )
                    .await;
            }
        };
        self.answer_files(
            &request.ask_id,
            BridgeResult::FilesRead(FilesReadResult {
                path: admitted.path.clone(),
                size_bytes: admitted.size_bytes,
                modified_ms: admitted.modified_ms,
                content,
            }),
        );
        Ok(())
    }

    /// An image read: its bytes go out of band, as a screenshot's do; the
    /// reply that names the capture is sent once the hub holds it.
    pub(super) async fn answer_image(
        &mut self,
        request: &InFlight,
        admitted: &Admitted,
        png_base64: &str,
        (width, height): (u32, u32),
        (source_width, source_height): (u32, u32),
    ) -> Result<(), Refusal> {
        use base64::Engine as _;
        let invalid = || Refusal::HelperReplyInvalid {
            field: "png_base64".into(),
            expected: "png_within_capture_bounds".into(),
            received_type: "string".into(),
        };
        let png = base64::engine::general_purpose::STANDARD
            .decode(png_base64.as_bytes())
            .map_err(|_| invalid())?;
        orbit_core::sources::validate_png(&png, width, height).map_err(|_| invalid())?;
        let Some(ask) = self
            .assistant
            .bridge_ask(crate::daemon::BridgeKind::Files)
            .filter(|ask| ask.id == request.ask_id)
            .cloned()
        else {
            return Ok(());
        };
        let partial = FilesReadResult {
            path: admitted.path.clone(),
            size_bytes: admitted.size_bytes,
            modified_ms: admitted.modified_ms,
            content: FileContent::Image {
                capture_id: String::new(),
                width,
                height,
                mime_type: "image/png".into(),
                sha256: String::new(),
                source_width,
                source_height,
            },
        };
        self.upload_file_capture(&ask, png, partial)
            .await
            .map_err(|_| Self::bridge_refusal_code("capture_upload_failed"))?;
        Ok(())
    }
}
