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

mod folders;

mod steps;

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
    /// listed, searched or read. `open` is every folder the first step
    /// found open, by id and canonical path: what a search of every folder
    /// may answer from.
    Working {
        admitted: Option<Admitted>,
        open: Vec<(String, String)>,
    },
}

#[derive(Debug, Clone)]
struct InFlight {
    native_id: String,
    ask_id: String,
    ask: FilesAsk,
    stage: Stage,
    started: Instant,
    /// The folders when the request started, less any removed since: the
    /// ones its answer names and the only ones it reads.
    saved: Vec<Folder>,
}

/// The daemon's part of the folders: the list as stored, what the helper last
/// said of each, and the one order of each kind it is waiting on.
#[derive(Debug, Default)]
pub(super) struct FilesHandle {
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

const REQUEST_TIMEOUT: Duration = Duration::from_millis(files::HELPER_REQUEST_TIMEOUT_MS);

impl Runtime {
    /// A helper that went away answers nothing it was asked: its folders are
    /// unknown again and a request it held is refused with `cause` — it
    /// restarted, or it is gone and nothing has started again yet.
    pub(super) fn files_helper_lost(&mut self, cause: Refusal) {
        self.files.access.clear();
        self.files.pick = None;
        self.files.check = None;
        if let Some(request) = self.files.request.take() {
            self.refuse_files(&request.ask_id, cause);
        }
    }

    /// [`Self::files_helper_lost`] for a helper that started again.
    pub(super) fn files_helper_restarted(&mut self) {
        self.files_helper_lost(Refusal::ReadInterrupted {
            cause: orbit_core::refusal::ReadInterruption::HelperRestarted {
                previous_generation: None,
                current_generation: None,
                previous_instance: None,
                current_instance: None,
            },
        });
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
        let native_id = self
            .helper
            .send_files_command("assistant_folders", fields)?;
        self.files.request = Some(InFlight {
            native_id,
            ask_id: ask.id.clone(),
            ask: request,
            stage: Stage::Resolving,
            started: Instant::now(),
            saved,
        });
        Ok(())
    }

    /// A request the helper has not finished within its bound, both steps
    /// counted from its start, is refused with both numbers, so the bridge's
    /// single slot is free before the hub gives up.
    pub(super) fn check_files_timeout(&mut self) {
        let expired = self
            .files
            .request
            .as_ref()
            .is_some_and(|request| request.started.elapsed() >= REQUEST_TIMEOUT);
        if !expired {
            return;
        }
        if let Some(request) = self.files.request.take() {
            self.refuse_files(
                &request.ask_id,
                Refusal::Timeout {
                    operation: Operation::Files,
                    waited_ms: u64::try_from(request.started.elapsed().as_millis())
                        .unwrap_or(u64::MAX),
                    deadline_ms: files::HELPER_REQUEST_TIMEOUT_MS,
                },
            );
        }
    }

    fn refuse_files(&mut self, request_id: &str, refusal: Refusal) {
        if !self.assistant.still_asked(request_id) {
            return;
        }
        self.assistant.bridge_request = None;
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
        if !self.assistant.still_asked(request_id) {
            return;
        }
        self.assistant.bridge_request = None;
        let frame = json!(DeviceReplyParams::answered(request_id.to_owned(), result));
        let sent = self.send_frame(hub::command(request_id, "device.reply", frame));
        self.note_bridge_send(&sent, "the folder answer");
    }

    /// Takes the helper's answer to any folder order; `false` when the
    /// response is not one of them.
    pub(super) async fn files_response(&mut self, event: &Value) -> Result<bool> {
        let response = NativeResponseFields::from_value(event)?;
        if self.files.pick.as_deref() == Some(response.id.as_str()) {
            self.files.pick = None;
            self.on_folder_picked(&response).await?;
            return Ok(true);
        }
        if self.files.check.as_deref() == Some(response.id.as_str()) {
            self.files.check = None;
            match Self::folders_reply(&response) {
                Ok(reply) => self.take_reports(&reply.folders).await,
                Err(_) => self.log("assistant_folders_check_failed", &[]),
            }
            return Ok(true);
        }
        let Some(request) = self
            .files
            .request
            .take_if(|request| request.native_id == response.id)
        else {
            return Ok(false);
        };
        // Released meanwhile (privacy stop, a lost hub, a timeout): whoever
        // released it answered; nothing more is read for it.
        if !self.assistant.still_asked(&request.ask_id) {
            return Ok(true);
        }
        let outcome = match &request.stage {
            Stage::Resolving => self.on_resolved(&request, &response).await,
            Stage::Working { admitted, open } => {
                let admitted = admitted.clone();
                let open = open.clone();
                self.on_worked(&request, admitted.as_ref(), &open, &response)
                    .await
            }
        };
        if let Err(refusal) = outcome {
            self.refuse_files(&request.ask_id, refusal);
        }
        Ok(true)
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
}

fn invalid_detail(code: &str) -> Refusal {
    Refusal::HelperReplyInvalid {
        field: "error.detail.cause".into(),
        expected: format!("native_v1/{code}").into(),
        received_type: "invalid".into(),
    }
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

impl crate::daemon::assistant::AssistantHandle {
    /// Whether `request_id` is still the bridge request being served: one the
    /// privacy stop, a disconnection or a timeout released is answered by
    /// whoever released it, never twice.
    fn still_asked(&self, request_id: &str) -> bool {
        self.bridge_request
            .as_ref()
            .is_some_and(|ask| ask.id == request_id)
    }
}

impl FilesHandle {
    /// The folders as the control topic publishes them.
    pub(super) fn folders_knowledge(&self) -> Knowledge<Vec<FolderView>> {
        match &self.folders {
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

    /// A folder the second step could not open is said in Ajustes as the
    /// first step's report would have said it.
    fn note_step_folder(&mut self, request: &InFlight, response: &NativeResponseFields) {
        let Some(error) = response
            .error
            .as_ref()
            .filter(|error| !response.ok && error.code == "assistant_folder_unavailable")
        else {
            return;
        };
        let detail = error.detail.as_ref();
        let id = detail
            .and_then(|detail| detail.get("id"))
            .and_then(Value::as_str);
        let cause = detail
            .and_then(|detail| detail.get("cause"))
            .cloned()
            .and_then(|cause| serde_json::from_value::<FolderCause>(cause).ok());
        if let (Some(id), Some(cause)) = (id, cause)
            && request.saved.iter().any(|folder| folder.id == id)
        {
            self.access.insert(
                id.to_owned(),
                FolderAccess::Unavailable {
                    cause: cause.bounded(),
                },
            );
        }
    }
}

impl crate::daemon::helper::HelperHandle {
    fn send_files_command(&mut self, name: &str, mut fields: Value) -> Result<String, Refusal> {
        let id = format!("assistant-files-{}", super::id());
        if let Some(object) = fields.as_object_mut() {
            object.insert("id".into(), Value::from(id.clone()));
        }
        let command = crate::native::command(name, fields).map_err(|_| Refusal::Internal {
            cause: orbit_core::refusal::InternalCause::NativeCommand,
        })?;
        self.sender
            .try_send(command.clone())
            .map_err(|_| Refusal::Busy {
                operation: Operation::Files,
                retry_after_ms: None,
            })?;
        self.record_pending(&command);
        Ok(id)
    }
}

#[cfg(test)]
mod tests;
