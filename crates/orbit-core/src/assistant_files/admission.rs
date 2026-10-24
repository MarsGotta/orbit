//! Whether the path a request named, as the helper resolved it, lies in one of
//! the folders, and what the next step reads.
use super::{
    ContentKind, FileCause, Folder, FolderCause, content_kind, is_within, lexical, max_bytes,
};
use crate::refusal::Refusal;
use serde::{Deserialize, Serialize};

/// One folder as the helper found it while answering a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum FolderReport {
    /// Open, at this canonical path. `renewed_bookmark_base64` is the
    /// bookmark macOS asked to have renewed (it had gone stale).
    Accessible {
        id: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        renewed_bookmark_base64: Option<String>,
    },
    Unavailable {
        id: String,
        cause: FolderCause,
    },
}

impl FolderReport {
    pub fn id(&self) -> &str {
        match self {
            Self::Accessible { id, .. } | Self::Unavailable { id, .. } => id,
        }
    }
}

/// What a file system entry is, as `lstat`/`stat` says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Folder,
    /// A symbolic link, in a listing: it is not followed there. Reading it
    /// resolves it and decides on where it leads.
    Link,
    Other,
}

/// The requested path as the helper resolved it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum RequestedPath {
    /// It exists; `path` is where it leads, links and `..` followed.
    Found {
        path: String,
        kind: EntryKind,
        size_bytes: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        modified_ms: Option<i64>,
    },
    NotFound,
    Unavailable {
        cause: FileCause,
    },
}

/// The helper's answer to `assistant_folders`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoldersReply {
    pub folders: Vec<FolderReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested: Option<RequestedPath>,
}

/// What a request asks of the path it names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asked {
    List,
    Search,
    Read,
}

/// A request the folders admit, with what the next step needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Admitted {
    /// The folder that holds it, by id.
    pub folder: String,
    /// The canonical path the helper resolved: the one the next step reads.
    pub path: String,
    pub size_bytes: u64,
    pub modified_ms: Option<i64>,
    /// For a read, what the file is read as.
    pub content: Option<ContentKind>,
}

/// The paths a refusal names, one per folder: where it was found now, or
/// where it was last seen when its bookmark does not open.
pub fn folder_paths(saved: &[Folder], reports: &[FolderReport]) -> Vec<String> {
    saved
        .iter()
        .map(|folder| {
            reports
                .iter()
                .find_map(|report| match report {
                    FolderReport::Accessible { id, path, .. } if *id == folder.id => {
                        Some(path.clone())
                    }
                    FolderReport::Accessible { .. } | FolderReport::Unavailable { .. } => None,
                })
                .unwrap_or_else(|| folder.path.clone())
        })
        .collect()
}

fn accessible(reports: &[FolderReport]) -> impl Iterator<Item = (&str, &str)> {
    reports.iter().filter_map(|report| match report {
        FolderReport::Accessible { id, path, .. } => Some((id.as_str(), path.as_str())),
        FolderReport::Unavailable { .. } => None,
    })
}

/// The innermost accessible folder holding `path`: with nested folders the
/// closer one is the one it belongs to.
fn holder<'a>(path: &str, reports: &'a [FolderReport]) -> Option<(&'a str, &'a str)> {
    accessible(reports)
        .filter(|(_, root)| is_within(path, root))
        .max_by_key(|(_, root)| root.len())
}

/// [`is_within`] in any letter case. The folders live on the Mac's own disk,
/// whose file system (APFS as macOS formats it) ignores case: a path written
/// in another case names the same place. Only used to say what a path that
/// did not resolve would have been; admission decides on the exact path the
/// helper resolved.
fn is_within_any_case(path: &str, root: &str) -> bool {
    is_within(&path.to_lowercase(), &root.to_lowercase())
}

/// An unavailable folder whose last known path holds `path`, so a request
/// into a folder whose bookmark broke says that, not «outside the folders».
fn broken_holder<'a>(
    path: &str,
    saved: &'a [Folder],
    reports: &'a [FolderReport],
) -> Option<(&'a Folder, &'a FolderCause)> {
    saved.iter().find_map(|folder| {
        reports.iter().find_map(|report| match report {
            FolderReport::Unavailable { id, cause }
                if *id == folder.id && is_within_any_case(path, &folder.path) =>
            {
                Some((folder, cause))
            }
            FolderReport::Unavailable { .. } | FolderReport::Accessible { .. } => None,
        })
    })
}

/// The decision: whether the path a request named, as the helper resolved
/// it, may be read, and as what.
///
/// Inside means the resolved path — links and `..` already followed on the
/// Mac — is a folder or lies within one. A path that leads outside is refused
/// with the path asked for and every folder. A missing path is only called
/// missing when, as written, it lies inside a folder (in any letter case).
///
/// Outside the folders, Orbit does not say what exists: a path that, as
/// written, lies outside every folder gets the same refusal whether it exists
/// or not, never where it leads (which would tell both that it exists and its
/// true name). Only a path written inside a folder that leads out — a link
/// in the folder — is told where it leads.
pub fn admit(
    asked: Asked,
    requested_path: &str,
    saved: &[Folder],
    reply: &FoldersReply,
) -> Result<Admitted, Refusal> {
    let said = lexical(requested_path);
    let said_inside = accessible(&reply.folders).any(|(_, root)| is_within_any_case(&said, root))
        || saved
            .iter()
            .any(|folder| is_within_any_case(&said, &folder.path));
    let outside = |resolved: Option<&str>| Refusal::OutsideFolders {
        path: requested_path.to_owned(),
        resolved: resolved
            .filter(|resolved| said_inside && *resolved != requested_path)
            .map(str::to_owned),
        folders: folder_paths(saved, &reply.folders),
    };
    let broken = |path: &str| {
        broken_holder(path, saved, &reply.folders).map(|(folder, cause)| {
            Refusal::FolderUnavailable {
                path: folder.path.clone(),
                cause: cause.clone(),
            }
        })
    };
    let Some(requested) = reply.requested.as_ref() else {
        return Err(Refusal::HelperReplyInvalid {
            field: "requested".into(),
            expected: "assistant_folders.requested".into(),
            received_type: "absent".into(),
        });
    };
    let (path, kind, size_bytes, modified_ms) = match requested {
        RequestedPath::NotFound => {
            if let Some(refusal) = broken(&said) {
                return Err(refusal);
            }
            return Err(if said_inside {
                Refusal::FileUnavailable {
                    path: requested_path.to_owned(),
                    cause: FileCause::NotFound,
                }
            } else {
                outside(None)
            });
        }
        RequestedPath::Unavailable { cause } => {
            return Err(if said_inside {
                Refusal::FileUnavailable {
                    path: requested_path.to_owned(),
                    cause: cause.clone(),
                }
            } else {
                outside(None)
            });
        }
        RequestedPath::Found {
            path,
            kind,
            size_bytes,
            modified_ms,
        } => (path, *kind, *size_bytes, *modified_ms),
    };
    let Some((folder, _)) = holder(path, &reply.folders) else {
        return Err(broken(path).unwrap_or_else(|| outside(Some(path))));
    };
    let content = content_for(asked, kind, path, size_bytes, requested_path)?;
    Ok(Admitted {
        folder: folder.to_owned(),
        path: path.clone(),
        size_bytes,
        modified_ms,
        content,
    })
}

/// What an entry inside the folders is read as, for what was asked of it: a
/// listing or a search needs a folder, a read a file within its kind's weight.
fn content_for(
    asked: Asked,
    kind: EntryKind,
    path: &str,
    size_bytes: u64,
    requested_path: &str,
) -> Result<Option<ContentKind>, Refusal> {
    let wrong_kind = |cause| Refusal::FileUnavailable {
        path: requested_path.to_owned(),
        cause,
    };
    match (asked, kind) {
        (Asked::List | Asked::Search, EntryKind::Folder) => Ok(None),
        (Asked::List | Asked::Search, EntryKind::File | EntryKind::Link | EntryKind::Other) => {
            Err(wrong_kind(FileCause::NotAFolder))
        }
        (Asked::Read, EntryKind::File) => {
            let content = content_kind(path);
            if let Some(max) = max_bytes(content)
                && size_bytes > max
            {
                return Err(Refusal::FileTooLarge {
                    path: requested_path.to_owned(),
                    content,
                    bytes: size_bytes,
                    max_bytes: max,
                });
            }
            Ok(Some(content))
        }
        (Asked::Read, EntryKind::Folder | EntryKind::Link | EntryKind::Other) => {
            Err(wrong_kind(FileCause::NotAFile))
        }
    }
}
