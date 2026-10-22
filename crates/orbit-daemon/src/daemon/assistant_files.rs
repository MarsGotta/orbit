//! The assistant's folders: chosen in the macOS panel, kept encrypted with
//! their bookmarks, checked by the helper, and read for Deimos's `files_*`
//! requests.
//!
//! The panel choice is the user's whole decision: nothing here asks again,
//! expires or switches the folders off. Every request takes two steps with the
//! helper. The first opens the bookmarks and resolves the path asked for —
//! links and `..` followed on the Mac — and `orbit_core::assistant_files::admit`
//! decides on the answer, without the disk; only an admitted canonical path
//! reaches the second step, which lists, searches or reads it. Read-only.
//!
//! Paths and file contents never enter the daemon's log: only codes.
use super::{BridgeAsk, Runtime, hub};
use anyhow::{Context, Result};
use orbit_core::assistant_files::{
    self as files, Admitted, Asked, ContentKind, Entry, EntryKind, Folder, FolderAccess,
    FolderBookmark, FolderCause, FolderReport, FolderView, FoldersReply, ListReply, PickReply,
    ReadAs, ReadReply, SearchLimits, SearchReply, SearchRoot,
};
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_core::refusal::{Expected, Operation, Parameter, Refusal};
use orbit_core::topics::{Knowledge, Pending as TopicPending};
use orbit_core::wire::bridge::{
    BridgeRequest, BridgeResult, DeviceReplyParams, FileContent, FilesListResult, FilesReadResult,
    FilesSearchResult,
};
use orbit_core::wire::native::{NativeResponseFields, decode_result_body};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// What a `files_*` request asked, validated before the helper hears of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FilesAsk {
    List {
        path: Option<String>,
        after_index: u64,
    },
    Search {
        query: String,
        path: Option<String>,
        content: bool,
    },
    Read {
        path: String,
        after_index: u64,
        after_page: u32,
    },
}

impl FilesAsk {
    /// The ask of a `files_*` request, `None` for any other capability.
    pub(super) fn from_request(request: &BridgeRequest) -> Result<Option<Self>, Refusal> {
        let ask = match request {
            BridgeRequest::FilesList { parameters } => Self::List {
                path: parameters.path.clone(),
                after_index: parameters.after_index.unwrap_or(0),
            },
            BridgeRequest::FilesSearch { parameters } => {
                let bytes = parameters.query.len();
                if parameters.query.trim().is_empty() || bytes > files::MAX_QUERY_BYTES {
                    return Err(Refusal::InvalidParameter {
                        parameter: Parameter::Request,
                        expected: Expected::SearchQuery {
                            bytes: bytes as u64,
                            max_bytes: files::MAX_QUERY_BYTES as u64,
                        },
                    });
                }
                Self::Search {
                    query: parameters.query.clone(),
                    path: parameters.path.clone(),
                    content: parameters.content.unwrap_or(false),
                }
            }
            BridgeRequest::FilesRead { parameters } => Self::Read {
                path: parameters.path.clone(),
                after_index: parameters.after_index.unwrap_or(0),
                after_page: parameters.after_page.unwrap_or(0),
            },
            BridgeRequest::Screenshot { .. }
            | BridgeRequest::ScreenSources { .. }
            | BridgeRequest::SelectedText { .. }
            | BridgeRequest::ControlTools { .. }
            | BridgeRequest::PerformAction { .. }
            | BridgeRequest::CalendarRead { .. } => return Ok(None),
        };
        if let Some(path) = ask.path() {
            files::check_requested_path(path)?;
        }
        Ok(Some(ask))
    }

    fn path(&self) -> Option<&str> {
        match self {
            Self::List { path, .. } | Self::Search { path, .. } => path.as_deref(),
            Self::Read { path, .. } => Some(path),
        }
    }

    fn asked(&self) -> Asked {
        match self {
            Self::List { .. } => Asked::List,
            Self::Search { .. } => Asked::Search,
            Self::Read { .. } => Asked::Read,
        }
    }
}

/// The step of a request the helper is answering.
#[derive(Debug, Clone)]
enum Stage {
    /// The bookmarks are being opened and the path resolved.
    Resolving,
    /// The admitted path (none for a search of every folder) is being
    /// listed, searched or read.
    Working { admitted: Option<Admitted> },
}

#[derive(Debug, Clone)]
struct InFlight {
    native_id: String,
    ask_id: String,
    ask: FilesAsk,
    stage: Stage,
    started: Instant,
    /// The folders when the request started: the ones its answer names.
    saved: Vec<Folder>,
}

/// The daemon's part of the folders: the list as stored, what the helper last
/// said of each, and the one order of each kind it is waiting on.
#[derive(Debug, Default)]
pub(super) struct FilesRuntime {
    /// `None` until read from the store; an unreadable row is its cause.
    folders: Option<Result<Vec<Folder>, String>>,
    /// The last word on each folder, by id; absent is unknown, never «no».
    access: BTreeMap<String, FolderAccess>,
    /// The panel the helper has open.
    pick: Option<String>,
    /// A check of every folder that answers no request (after an addition,
    /// or a helper that came back).
    check: Option<String>,
    request: Option<InFlight>,
}

const STEP_TIMEOUT: Duration = Duration::from_millis(files::HELPER_STEP_TIMEOUT_MS);

impl Runtime {
    /// Reads the folders at startup. A row that cannot be read is said and
    /// published with its cause, never as an empty list.
    pub(super) fn recover_assistant_folders(&mut self) {
        let loaded = self.reader.load_assistant_folders();
        if loaded.is_err() {
            self.log("assistant_folders_unreadable", &[]);
        }
        self.features.files.folders =
            Some(loaded.map_err(|error| {
                format!("No se pudieron leer las carpetas del asistente ({error})")
            }));
    }

    fn saved_folders(&mut self) -> Result<Vec<Folder>, Refusal> {
        if self.features.files.folders.is_none() {
            self.recover_assistant_folders();
        }
        match &self.features.files.folders {
            Some(Ok(folders)) => Ok(folders.clone()),
            Some(Err(cause)) => Err(Refusal::StorageFailed {
                operation: Operation::Files,
                cause: Some(cause.clone().into()),
            }),
            None => Ok(Vec::new()),
        }
    }

    /// The folders as the control topic publishes them.
    pub(super) fn folders_knowledge(&self) -> Knowledge<Vec<FolderView>> {
        match &self.features.files.folders {
            None => Knowledge::Unknown {
                why: TopicPending::NotObservedYet,
            },
            Some(Err(cause)) => Knowledge::Absent {
                refusal: Refusal::StorageFailed {
                    operation: Operation::Files,
                    cause: Some(cause.clone().into()),
                },
            },
            Some(Ok(folders)) => Knowledge::Present {
                value: folders
                    .iter()
                    .map(|folder| FolderView {
                        id: folder.id.clone(),
                        path: folder.path.clone(),
                        access: self
                            .features
                            .files
                            .access
                            .get(&folder.id)
                            .cloned()
                            .unwrap_or(FolderAccess::Unknown),
                    })
                    .collect(),
                as_of_ms: None,
            },
        }
    }

    fn send_files_command(&mut self, name: &str, mut fields: Value) -> Result<String, Refusal> {
        let id = format!("assistant-files-{}", super::id());
        if let Some(object) = fields.as_object_mut() {
            object.insert("id".into(), Value::from(id.clone()));
        }
        let command = crate::native::command(name, fields).map_err(|_| Refusal::Internal {
            cause: orbit_core::refusal::InternalCause::NativeCommand,
        })?;
        self.native
            .try_send(command.clone())
            .map_err(|_| Refusal::Busy {
                operation: Operation::Files,
                retry_after_ms: None,
            })?;
        self.record_pending_native(&command);
        Ok(id)
    }

    /// «Añadir carpeta»: the panel opens on the Mac; the choice arrives in
    /// `files_response`. Refused at once, by name, when no folder can be added.
    pub(super) fn add_assistant_folder(&mut self) -> Result<()> {
        if self.state.privacy_stopped() {
            return Err(Refusal::PrivacyStopped { since_ms: None }.into());
        }
        if !self.state.capabilities().native_helper() {
            return Err(Refusal::HelperNotRunning.into());
        }
        if self.features.files.pick.is_some() {
            return Err(Refusal::Busy {
                operation: Operation::Files,
                retry_after_ms: None,
            }
            .into());
        }
        let saved = self.saved_folders()?;
        files::room_for_another(&saved)?;
        let id = self.send_files_command("assistant_folder_pick", json!({}))?;
        self.features.files.pick = Some(id);
        Ok(())
    }

    /// «Quitar»: forgotten at once, bookmark included.
    pub(super) async fn remove_assistant_folder(&mut self, id: &str) -> Result<()> {
        let saved = self.saved_folders()?;
        if let Some(folders) = files::without_folder(&saved, id) {
            self.store_folders(folders).await?;
        }
        self.features.files.access.remove(id);
        Ok(())
    }

    async fn store_folders(&mut self, folders: Vec<Folder>) -> Result<()> {
        let kept = folders.clone();
        self.store_actor
            .run(move |store| Ok(store.save_assistant_folders(&kept)?))
            .await
            .context("No se pudieron guardar las carpetas del asistente")?;
        self.features.files.folders = Some(Ok(folders));
        Ok(())
    }

    /// Asks the helper whether every folder still opens, for Ajustes.
    pub(super) fn check_assistant_folders(&mut self) {
        let Ok(saved) = self.saved_folders() else {
            return;
        };
        if saved.is_empty()
            || self.features.files.check.is_some()
            || !self.state.capabilities().native_helper()
        {
            return;
        }
        let bookmarks: Vec<FolderBookmark> = saved.iter().map(FolderBookmark::from).collect();
        if let Ok(id) = self.send_files_command("assistant_folders", json!({"folders":bookmarks})) {
            self.features.files.check = Some(id);
        }
    }

    /// A helper that went away answers nothing it was asked: its folders are
    /// unknown again and a request it held is refused with that cause.
    pub(super) fn files_helper_restarted(&mut self) {
        self.features.files.access.clear();
        self.features.files.pick = None;
        self.features.files.check = None;
        if let Some(request) = self.features.files.request.take() {
            self.refuse_files(
                &request.ask_id,
                Refusal::ReadInterrupted {
                    cause: orbit_core::refusal::ReadInterruption::HelperRestarted {
                        previous_generation: None,
                        current_generation: None,
                        previous_instance: None,
                        current_instance: None,
                    },
                },
            );
        }
    }

    /// The first step of a `files_*` request.
    pub(super) fn files_bridge_start(&mut self, ask: &BridgeAsk) -> Result<(), Refusal> {
        let request = ask.files.clone().ok_or(Refusal::InvalidParameter {
            parameter: Parameter::Request,
            expected: Expected::ValidSelection,
        })?;
        let saved = self.saved_folders()?;
        if saved.is_empty() {
            return Err(Refusal::NoFolders);
        }
        let bookmarks: Vec<FolderBookmark> = saved.iter().map(FolderBookmark::from).collect();
        let mut fields = json!({ "folders": bookmarks });
        if let Some(path) = request.path() {
            fields["path"] = Value::from(path);
        }
        let native_id = self.send_files_command("assistant_folders", fields)?;
        self.features.files.request = Some(InFlight {
            native_id,
            ask_id: ask.id.clone(),
            ask: request,
            stage: Stage::Resolving,
            started: Instant::now(),
            saved,
        });
        Ok(())
    }

    /// A request whose helper step outlived its bound is refused with both
    /// numbers, so the bridge's single slot is free before the hub gives up.
    pub(super) fn check_files_timeout(&mut self) {
        let expired = self
            .features
            .files
            .request
            .as_ref()
            .is_some_and(|request| request.started.elapsed() >= STEP_TIMEOUT);
        if !expired {
            return;
        }
        if let Some(request) = self.features.files.request.take() {
            self.refuse_files(
                &request.ask_id,
                Refusal::Timeout {
                    operation: Operation::Files,
                    waited_ms: u64::try_from(request.started.elapsed().as_millis())
                        .unwrap_or(u64::MAX),
                    deadline_ms: files::HELPER_STEP_TIMEOUT_MS,
                },
            );
        }
    }

    /// Whether `request_id` is still the bridge request being served: one the
    /// privacy stop, a disconnection or a timeout released is answered by
    /// whoever released it, never twice.
    fn still_asked(&self, request_id: &str) -> bool {
        self.conversation
            .bridge_request
            .as_ref()
            .is_some_and(|ask| ask.id == request_id)
    }

    fn refuse_files(&mut self, request_id: &str, refusal: Refusal) {
        if !self.still_asked(request_id) {
            return;
        }
        self.conversation.bridge_request = None;
        let code = refusal.code().name();
        self.say(Notice::new(NoticeCode::BridgeRefused).with_token(NoticeKey::Code, code));
        let frame = json!(DeviceReplyParams::refused(
            request_id.to_owned(),
            refusal,
            None
        ));
        let sent = self.send_frame(hub::command(request_id, "device.reply", frame));
        self.note_bridge_send(&sent, &format!("the folder refusal ({code})"));
    }

    fn answer_files(&mut self, request_id: &str, result: BridgeResult) {
        if !self.still_asked(request_id) {
            return;
        }
        self.conversation.bridge_request = None;
        let frame = json!(DeviceReplyParams::answered(request_id.to_owned(), result));
        let sent = self.send_frame(hub::command(request_id, "device.reply", frame));
        self.note_bridge_send(&sent, "the folder answer");
    }

    /// Takes the helper's answer to any folder order; `false` when the
    /// response is not one of them.
    pub(super) async fn files_response(&mut self, event: &Value) -> Result<bool> {
        let response = NativeResponseFields::from_value(event)?;
        if self.features.files.pick.as_deref() == Some(response.id.as_str()) {
            self.features.files.pick = None;
            self.on_folder_picked(&response).await?;
            return Ok(true);
        }
        if self.features.files.check.as_deref() == Some(response.id.as_str()) {
            self.features.files.check = None;
            match Self::folders_reply(&response) {
                Ok(reply) => self.take_reports(&reply.folders).await,
                Err(_) => self.log("assistant_folders_check_failed", &[]),
            }
            return Ok(true);
        }
        let Some(request) = self
            .features
            .files
            .request
            .take_if(|request| request.native_id == response.id)
        else {
            return Ok(false);
        };
        // Released meanwhile (privacy stop, a lost hub, a timeout): whoever
        // released it answered; nothing more is read for it.
        if !self.still_asked(&request.ask_id) {
            return Ok(true);
        }
        let outcome = match &request.stage {
            Stage::Resolving => self.on_resolved(&request, &response).await,
            Stage::Working { admitted } => {
                let admitted = admitted.clone();
                self.on_worked(&request, admitted.as_ref(), &response).await
            }
        };
        if let Err(refusal) = outcome {
            self.refuse_files(&request.ask_id, refusal);
        }
        Ok(true)
    }

    async fn on_folder_picked(&mut self, response: &NativeResponseFields) -> Result<()> {
        if !response.ok {
            let code = response
                .error
                .as_ref()
                .map_or("assistant_folder_pick_failed", |error| error.code.as_str())
                .to_owned();
            self.say(
                Notice::new(NoticeCode::HelperCommandFailed).with_token(NoticeKey::Code, &code),
            );
            return Ok(());
        }
        let picked = response
            .result
            .as_ref()
            .map(|result| decode_result_body::<PickReply>("assistant_folder_pick", result));
        let Some(Ok(PickReply::Chosen {
            path,
            bookmark_base64,
        })) = picked
        else {
            if !matches!(picked, Some(Ok(PickReply::Cancelled))) {
                self.log("assistant_folder_pick_invalid", &[]);
            }
            return Ok(());
        };
        let saved = self.saved_folders()?;
        let chosen = Folder {
            id: uuid::Uuid::new_v4().to_string(),
            path,
            bookmark_base64,
        };
        match files::with_folder(&saved, chosen) {
            Ok(Some(folders)) => {
                self.store_folders(folders).await?;
                self.check_assistant_folders();
            }
            Ok(None) => {}
            Err(refusal) => {
                self.say(
                    Notice::new(NoticeCode::HelperCommandFailed)
                        .with_token(NoticeKey::Code, refusal.code().name()),
                );
            }
        }
        Ok(())
    }

    fn folders_reply(response: &NativeResponseFields) -> Result<FoldersReply, Refusal> {
        let result = response
            .result
            .as_ref()
            .ok_or(Refusal::HelperReplyInvalid {
                field: "assistant_folders.result".into(),
                expected: "object".into(),
                received_type: "absent".into(),
            })?;
        let mut reply: FoldersReply = decode_result_body("assistant_folders", result)
            .map_err(Refusal::helper_reply_invalid)?;
        for report in &mut reply.folders {
            if let FolderReport::Unavailable { cause, .. } = report {
                *cause = cause.clone().bounded();
            }
        }
        Ok(reply)
    }

    /// What the helper just said of each folder: kept for Ajustes, and a
    /// renewed bookmark or a moved folder stored.
    async fn take_reports(&mut self, reports: &[FolderReport]) {
        for report in reports {
            let access = match report {
                FolderReport::Accessible { .. } => FolderAccess::Accessible,
                FolderReport::Unavailable { cause, .. } => FolderAccess::Unavailable {
                    cause: cause.clone(),
                },
            };
            self.features
                .files
                .access
                .insert(report.id().to_owned(), access);
        }
        if let Ok(saved) = self.saved_folders()
            && let Some(folders) = files::refreshed(&saved, reports)
            && self.store_folders(folders).await.is_err()
        {
            self.log("assistant_folders_refresh_unsaved", &[]);
        }
    }

    /// The helper's refusal of a step, in the request's own terms.
    fn helper_refusal(request: &InFlight, response: &NativeResponseFields) -> Option<Refusal> {
        if response.ok {
            return None;
        }
        let Some(error) = response.error.as_ref() else {
            return Some(Refusal::HelperReplyInvalid {
                field: "error".into(),
                expected: "native_v1/error".into(),
                received_type: "absent".into(),
            });
        };
        let detail = error.detail.as_ref();
        let cause = detail.and_then(|detail| detail.get("cause")).cloned();
        Some(match error.code.as_str() {
            "assistant_folder_unavailable" => {
                let id = detail
                    .and_then(|detail| detail.get("id"))
                    .and_then(Value::as_str);
                match cause.and_then(|cause| serde_json::from_value::<FolderCause>(cause).ok()) {
                    Some(cause) => Refusal::FolderUnavailable {
                        path: request
                            .saved
                            .iter()
                            .find(|folder| Some(folder.id.as_str()) == id)
                            .map_or_else(String::new, |folder| folder.path.clone()),
                        cause: cause.bounded(),
                    },
                    None => invalid_detail("assistant_folder_unavailable"),
                }
            }
            "assistant_file_unavailable" => {
                match cause.and_then(|cause| serde_json::from_value(cause).ok()) {
                    Some(cause) => Refusal::FileUnavailable {
                        path: request.ask.path().unwrap_or_default().to_owned(),
                        cause,
                    },
                    None => invalid_detail("assistant_file_unavailable"),
                }
            }
            code => Self::bridge_refusal_code(code),
        })
    }

    async fn on_resolved(
        &mut self,
        request: &InFlight,
        response: &NativeResponseFields,
    ) -> Result<(), Refusal> {
        if let Some(refusal) = Self::helper_refusal(request, response) {
            return Err(refusal);
        }
        let reply = Self::folders_reply(response)?;
        self.take_reports(&reply.folders).await;
        let bookmark = |id: &str| {
            request
                .saved
                .iter()
                .find(|folder| folder.id == id)
                .map(FolderBookmark::from)
        };
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
            } => {
                let roots: Vec<SearchRoot> = reply
                    .folders
                    .iter()
                    .filter_map(|report| match report {
                        FolderReport::Accessible { id, path, .. } => {
                            bookmark(id).map(|folder| SearchRoot {
                                folder,
                                path: path.clone(),
                            })
                        }
                        FolderReport::Unavailable { .. } => None,
                    })
                    .collect();
                if roots.is_empty() {
                    return Err(first_unavailable(&request.saved, &reply.folders));
                }
                (
                    "assistant_files_search",
                    json!({"roots":roots,"query":query,"content":content,
                        "limits":SearchLimits::ORBIT}),
                    None,
                )
            }
            ask @ (FilesAsk::List { .. } | FilesAsk::Search { .. } | FilesAsk::Read { .. }) => {
                let path = ask.path().unwrap_or_default();
                let admitted = files::admit(ask.asked(), path, &request.saved, &reply)?;
                let folder = bookmark(&admitted.folder).ok_or(Refusal::HelperReplyInvalid {
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
                (name, fields, Some(admitted))
            }
        };
        let native_id = self.send_files_command(name, fields)?;
        self.features.files.request = Some(InFlight {
            native_id,
            stage: Stage::Working { admitted },
            started: Instant::now(),
            ..request.clone()
        });
        Ok(())
    }

    async fn on_worked(
        &mut self,
        request: &InFlight,
        admitted: Option<&Admitted>,
        response: &NativeResponseFields,
    ) -> Result<(), Refusal> {
        if let Some(refusal) = Self::helper_refusal(request, response) {
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
                let path = admitted.map(|admitted| admitted.path.clone());
                let budget = files::page_budget(path.as_deref().unwrap_or_default());
                let (entries, next_after_index) =
                    files::fit_entries(reply.entries, *after_index, reply.next_after_index, budget);
                self.answer_files(
                    &request.ask_id,
                    BridgeResult::FilesList(FilesListResult {
                        path,
                        entries,
                        total: reply.total,
                        after_index: *after_index,
                        next_after_index,
                    }),
                );
            }
            FilesAsk::Search {
                query,
                path,
                content,
            } => {
                let reply: SearchReply = decode_result_body("assistant_files_search", result)
                    .map_err(Refusal::helper_reply_invalid)?;
                let shown = admitted
                    .map(|admitted| admitted.path.clone())
                    .or_else(|| path.clone());
                let budget = files::page_budget(query)
                    .saturating_sub(files::encoded_len(shown.as_deref().unwrap_or_default()));
                let (results, stopped) = files::fit_hits(reply.results, reply.stopped, budget);
                self.answer_files(
                    &request.ask_id,
                    BridgeResult::FilesSearch(FilesSearchResult {
                        query: query.clone(),
                        path: shown,
                        content: *content,
                        results,
                        visited: reply.visited,
                        files_read: reply.files_read,
                        bytes_read: reply.bytes_read,
                        partial_files: reply.partial_files,
                        stopped,
                    }),
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

    async fn answer_read(
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
                    .conversation
                    .bridge_request
                    .clone()
                    .filter(|ask| ask.id == request.ask_id)
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
                // The bytes go out of band, as a screenshot's do; the reply
                // that names the capture is sent once the hub holds it.
                self.upload_file_capture(&ask, png, partial)
                    .await
                    .map_err(|_| Self::bridge_refusal_code("capture_upload_failed"))?;
                return Ok(());
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
}

fn invalid_detail(code: &str) -> Refusal {
    Refusal::HelperReplyInvalid {
        field: "error.detail.cause".into(),
        expected: format!("native_v1/{code}").into(),
        received_type: "invalid".into(),
    }
}

/// How the helper reads an admitted file.
fn read_as(content: Option<ContentKind>, after_index: u64, after_page: u32) -> ReadAs {
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
fn roots_page(request: &InFlight, reply: &FoldersReply, after_index: u64) -> BridgeResult {
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

/// Why a search over every folder has none to walk.
fn first_unavailable(saved: &[Folder], reports: &[FolderReport]) -> Refusal {
    saved
        .iter()
        .find_map(|folder| {
            reports.iter().find_map(|report| match report {
                FolderReport::Unavailable { id, cause } if *id == folder.id => {
                    Some(Refusal::FolderUnavailable {
                        path: folder.path.clone(),
                        cause: cause.clone(),
                    })
                }
                FolderReport::Unavailable { .. } | FolderReport::Accessible { .. } => None,
            })
        })
        .unwrap_or(Refusal::NoFolders)
}

#[cfg(test)]
mod tests;
