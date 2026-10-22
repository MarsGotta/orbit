//! The folders the user gave the assistant on this Mac, and what it may read
//! in them.
//!
//! Choosing a folder in the macOS panel is the whole decision: nothing here
//! asks again, expires or switches it off, and removing the folder in Ajustes
//! is the only way back. Everything is read-only.
//!
//! What is decided here, without a disk: whether a path the helper resolved
//! (symbolic links and `..` followed, by `realpath` on the Mac) lies inside one
//! of the folders, what each kind of file may weigh, and how much of a file one
//! answer carries. The helper reports facts; the daemon passes them in.
use crate::refusal::{Expected, Parameter, Refusal};
use serde::{Deserialize, Serialize};

/// How many folders the user may give the assistant.
///
/// Protects: every request to the helper carries the bookmark of each folder,
/// and a refusal names every folder so the assistant can correct its path.
/// Why eight: a handful of project folders is the use; eight paths at their
/// longest keep that list under 8 KiB in a refusal and the bookmarks under
/// 200 KiB in one helper command. Not a limit on the files inside them.
pub const MAX_FOLDERS: usize = 8;

/// The longest path accepted in a request or kept for a folder, in bytes.
///
/// Protects: the hub's 8 KiB request body for a non-action bridge request and
/// the frames that echo the path back. Why 1024: it is macOS's `PATH_MAX`, the
/// longest path its file system calls accept, so a longer one could not be
/// opened anyway. Copied in `server/orbit_mcp/bridge.py` and compared by the
/// guard of `contracts/constants.json`.
pub const MAX_PATH_BYTES: usize = 1024;

/// The largest bookmark kept for a folder, in bytes before base64.
///
/// Protects: the encrypted settings row and each helper command, which carry
/// every bookmark. Why 16 KiB: a bookmark of a folder is a few hundred bytes to
/// a few KiB (it records the path, the volume and a few properties); sixteen
/// is several times that and still bounds the row. A bookmark over it is
/// refused with its size, never cut.
pub const MAX_BOOKMARK_BYTES: usize = 16 * 1024;

/// The most entries of a folder one listing asks the helper for.
///
/// Protects: the helper's reply line (1 MiB) and the time it spends reading
/// attributes. Why 200: the same count as one window inventory; the page is
/// then fitted to `MAX_PAGE_BYTES` and continues with `after_index`. Not a
/// limit on how many files a folder may hold.
pub const MAX_LIST_ENTRIES: u32 = 200;

/// How many bytes of results one bridge answer may carry once encoded.
///
/// Protects: the link to the hub, which refuses any outgoing frame over
/// 128 KiB. Why 96 KiB: the reasoning and the number of
/// `device_bridge::MAX_CALENDAR_PAGE_BYTES`, leaving 32 KiB for the envelope.
/// What does not fit continues on the next page; nothing is dropped.
pub const MAX_PAGE_BYTES: usize = 96 * 1024;

/// How many bytes of text one read of a file answers with.
///
/// Protects: the assistant's context, through Hermes's 50 000-character limit
/// on one tool result. Why 48 000: the translator's
/// `meetings_bridge.page.MAX_ANSWER_CHARS`, which leaves 2 000 characters for
/// the header; a byte is never fewer than one character, so the page fits. The
/// rest of the file is one `after_index` away; this is not a file size limit.
pub const MAX_TEXT_PAGE_BYTES: u32 = 48_000;

/// The most pages of a PDF one answer extracts.
///
/// Protects: the helper's time on one request, for a scanned PDF whose pages
/// carry no text (each still costs PDFKit a page load). Why 50: a few seconds
/// of extraction at most; the answer says which pages it read and where to
/// go on. Not a limit on a document's pages.
pub const MAX_PDF_PAGES_PER_ANSWER: u32 = 50;

/// The heaviest PDF read, in bytes.
///
/// Protects: the helper's memory, because PDFKit maps the whole document to
/// read any page. Why 64 MiB: a text PDF of several hundred pages is a few MiB
/// and a scan with a text layer tens of MiB; above it the read is refused with
/// both sizes, never started.
pub const MAX_PDF_BYTES: u64 = 64 * 1024 * 1024;

/// The heaviest image read, in bytes.
///
/// Protects: the helper's memory and time while ImageIO decodes the image to
/// fit it. Why 32 MiB: above any compressed photograph from a camera or phone
/// (a 48-megapixel HEIC or JPEG is under 25 MiB); raw files above it are
/// refused with both sizes, never opened.
pub const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;

/// The longest search text, in bytes.
///
/// Protects: the request body and the comparison per file name. Why 256: a
/// file name is at most 255 bytes on APFS, so a longer text cannot match one.
/// Copied in `server/orbit_mcp/bridge.py` and compared by the guard.
pub const MAX_QUERY_BYTES: usize = 256;

/// The most matches one search answers with.
///
/// Protects: the answer's size and the assistant's context. Why 50: a page of
/// results a person could read; a search that finds more says so and asks for
/// a narrower one.
pub const MAX_SEARCH_RESULTS: u32 = 50;

/// The most folder entries one search visits.
///
/// Protects: the helper's time on a very large tree (a home folder, a
/// repository with dependencies). Why 20 000: a few seconds of directory reads;
/// a search that stops here says how many it visited.
pub const MAX_SEARCH_VISITED: u32 = 20_000;

/// The most files one content search opens.
///
/// Protects: the helper's time and the disk. Why 1 000: with the per-file
/// ceiling below, about the bytes ceiling below; the answer names which one
/// stopped it.
pub const MAX_SEARCH_FILES_READ: u32 = 1_000;

/// The most bytes one content search reads, all files together.
///
/// Protects: the helper's time and memory. Why 32 MiB: a few seconds of
/// sequential reading on an SSD; the answer says it stopped here.
pub const MAX_SEARCH_BYTES_READ: u64 = 32 * 1024 * 1024;

/// How much of each file a content search reads, from its start.
///
/// Protects: one large log from taking the whole search. Why 1 MiB: notes,
/// documents and source files are well under it; a match further in is not
/// found, and the answer says how many files were read only in part.
pub const MAX_SEARCH_FILE_BYTES: u64 = 1024 * 1024;

/// The characters of the line shown around a content match.
///
/// Protects: the size of each result. Why 160: two lines of prose, enough to
/// recognise the match; the file itself is one read away.
pub const MAX_SNIPPET_CHARS: u32 = 160;

/// How long one search may walk, in milliseconds.
///
/// Protects: the bridge request, which the hub abandons at 30 s. Why 10 s: the
/// helper still has to answer and the daemon to reply within that wait; a
/// search that stops on time says so with what it found.
pub const SEARCH_DEADLINE_MS: u64 = 10_000;

/// How long the daemon waits for the helper to answer a whole request, both
/// of its steps together, from when the request reached the Mac.
///
/// Protects: the bridge's single slot, which a lost answer would hold until the
/// hub gave up. Why 20 s: twice the search deadline, and the 20 s a capture
/// upload is given, so the refusal still reaches the hub before its 30 s. A
/// bound per step would not: two slow steps would pass the hub's wait.
pub const HELPER_REQUEST_TIMEOUT_MS: u64 = 20_000;

/// One folder the user chose, as the daemon keeps it in its encrypted store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Folder {
    pub id: String,
    /// The folder's path when it was chosen, or when its bookmark last resolved.
    pub path: String,
    /// The macOS bookmark that finds the folder again, in base64.
    pub bookmark_base64: String,
}

/// Whether a folder can be read now, in three values: a folder nobody has
/// checked yet (the helper is starting) is neither readable nor lost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum FolderAccess {
    Accessible,
    Unavailable { cause: FolderCause },
    Unknown,
}

/// Why a folder's bookmark does not open it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FolderCause {
    /// macOS could not resolve the bookmark: the folder was deleted, or its
    /// disk is not connected. Foundation's error domain and code say which.
    BookmarkUnresolvable {
        domain: String,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        code: i64,
    },
    /// The bookmark leads to something that is no longer a folder.
    NotAFolder,
    /// The folder is there and macOS refused to open it (`errno`).
    AccessDenied { errno: i32 },
    /// Its path is longer than the system calls accept.
    PathTooLong {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        bytes: u64,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        max_bytes: u64,
    },
    /// The bookmark macOS renewed is larger than Orbit keeps.
    BookmarkTooLarge {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        bytes: u64,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        max_bytes: u64,
    },
}

/// One folder as Ajustes shows it: where it is and whether it opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct FolderView {
    pub id: String,
    pub path: String,
    pub access: FolderAccess,
}

/// The longest error domain kept from Foundation, in bytes.
///
/// Protects: the control topic and refusals, which carry it. Why 64: the
/// domains a bookmark resolution reports (`NSCocoaErrorDomain`,
/// `NSPOSIXErrorDomain`, `NSOSStatusErrorDomain`) are under 32; a longer one is
/// replaced by a fixed label, never cut into something that reads as another.
pub const MAX_ERROR_DOMAIN_BYTES: usize = 64;

impl FolderCause {
    /// The cause with its domain bounded (`MAX_ERROR_DOMAIN_BYTES`).
    pub fn bounded(self) -> Self {
        match self {
            Self::BookmarkUnresolvable { domain, code }
                if domain.len() > MAX_ERROR_DOMAIN_BYTES =>
            {
                Self::BookmarkUnresolvable {
                    domain: "error_domain_too_long".into(),
                    code,
                }
            }
            other => other,
        }
    }
}

/// Why a path inside a folder cannot be read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileCause {
    NotFound,
    /// A folder or something else was given where a file was asked.
    NotAFile,
    /// A file was given where a folder was asked.
    NotAFolder,
    /// Its bytes are not UTF-8 text, or carry a NUL.
    NotText,
    /// PDFKit could not open it, or it is locked with a password.
    PdfUnreadable,
    /// ImageIO could not decode it.
    ImageUnreadable,
    /// Even at its smallest size the image does not fit one capture.
    ImageDoesNotFit {
        smallest_png_bytes: u64,
        max_png_bytes: u64,
    },
    /// It changed between being resolved and being read (a link replaced a
    /// folder on the way, for instance); nothing was read.
    Changed,
    /// The system refused or failed the read (`errno`).
    ReadFailed {
        errno: i32,
    },
    /// The position asked for is past the end of the text.
    PastEnd {
        after_index: u64,
        size_bytes: u64,
    },
    /// The position asked for falls inside a character.
    NotCharacterBoundary {
        after_index: u64,
    },
    /// The page asked for is past the document's last page.
    PastLastPage {
        after_page: u32,
        page_count: u32,
    },
}

/// What a file is read as, decided by its extension. Anything that is not a
/// PDF or an image is attempted as text, and the helper refuses what is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    Text,
    Pdf,
    Image,
}

const IMAGE_EXTENSIONS: [&str; 11] = [
    "png", "jpg", "jpeg", "heic", "heif", "gif", "tif", "tiff", "bmp", "webp", "jp2",
];

pub fn content_kind(path: &str) -> ContentKind {
    let name = path.rsplit('/').next().unwrap_or(path);
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    if extension == "pdf" {
        ContentKind::Pdf
    } else if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
        ContentKind::Image
    } else {
        ContentKind::Text
    }
}

/// The heaviest file of a kind that is read at all. Text has none: it is read
/// one page at a time from where the last page ended.
pub fn max_bytes(kind: ContentKind) -> Option<u64> {
    match kind {
        ContentKind::Text => None,
        ContentKind::Pdf => Some(MAX_PDF_BYTES),
        ContentKind::Image => Some(MAX_IMAGE_BYTES),
    }
}

/// What a requested path must be before the helper is asked about it.
pub fn check_requested_path(path: &str) -> Result<(), Refusal> {
    let bytes = path.len();
    if !path.starts_with('/') || bytes > MAX_PATH_BYTES || path.contains('\0') {
        return Err(Refusal::InvalidParameter {
            parameter: Parameter::Request,
            expected: Expected::AbsolutePath {
                bytes: bytes as u64,
                max_bytes: MAX_PATH_BYTES as u64,
            },
        });
    }
    Ok(())
}

/// A path with `.` and `..` folded and repeated slashes removed, without the
/// disk: what the path says, not what it leads to.
pub fn lexical(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    format!("/{}", parts.join("/"))
}

/// Whether `path` is `root` or lies inside it, by whole components: a folder
/// `/a/b` does not contain `/a/bc`.
pub fn is_within(path: &str, root: &str) -> bool {
    let root = root.trim_end_matches('/');
    if root.is_empty() {
        return path.starts_with('/');
    }
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

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
    let wrong_kind = |cause| Refusal::FileUnavailable {
        path: requested_path.to_owned(),
        cause,
    };
    let content = match (asked, kind) {
        (Asked::List | Asked::Search, EntryKind::Folder) => None,
        (Asked::List | Asked::Search, EntryKind::File | EntryKind::Link | EntryKind::Other) => {
            return Err(wrong_kind(FileCause::NotAFolder));
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
            Some(content)
        }
        (Asked::Read, EntryKind::Folder | EntryKind::Link | EntryKind::Other) => {
            return Err(wrong_kind(FileCause::NotAFile));
        }
    };
    Ok(Admitted {
        folder: folder.to_owned(),
        path: path.clone(),
        size_bytes,
        modified_ms,
        content,
    })
}

/// The folders after the user chose `chosen` in the panel: `None` when it is
/// already one of them (choosing it twice changes nothing).
pub fn with_folder(saved: &[Folder], chosen: Folder) -> Result<Option<Vec<Folder>>, Refusal> {
    if saved
        .iter()
        .any(|folder| folder.path == chosen.path || folder.id == chosen.id)
    {
        return Ok(None);
    }
    check_folder(&chosen)?;
    if saved.len() >= MAX_FOLDERS {
        return Err(Refusal::TooMany {
            resource: crate::refusal::Resource::Folder,
            count: saved.len() as u64 + 1,
            max: MAX_FOLDERS as u64,
        });
    }
    let mut folders = saved.to_vec();
    folders.push(chosen);
    Ok(Some(folders))
}

/// Whether another folder can be chosen at all, asked before opening the panel.
pub fn room_for_another(saved: &[Folder]) -> Result<(), Refusal> {
    if saved.len() >= MAX_FOLDERS {
        return Err(Refusal::TooMany {
            resource: crate::refusal::Resource::Folder,
            count: saved.len() as u64 + 1,
            max: MAX_FOLDERS as u64,
        });
    }
    Ok(())
}

/// A folder the helper described is within the bounds Orbit keeps.
pub fn check_folder(folder: &Folder) -> Result<(), Refusal> {
    if !folder.path.starts_with('/') {
        return Err(Refusal::HelperReplyInvalid {
            field: "assistant_folder_pick.path".into(),
            expected: "absolute_path".into(),
            received_type: "relative_path".into(),
        });
    }
    if folder.path.len() > MAX_PATH_BYTES {
        return Err(Refusal::FolderUnavailable {
            path: folder.path.chars().take(MAX_PATH_BYTES).collect(),
            cause: FolderCause::PathTooLong {
                bytes: folder.path.len() as u64,
                max_bytes: MAX_PATH_BYTES as u64,
            },
        });
    }
    let decoded = bookmark_bytes(&folder.bookmark_base64);
    if decoded > MAX_BOOKMARK_BYTES {
        return Err(Refusal::FolderUnavailable {
            path: folder.path.clone(),
            cause: FolderCause::BookmarkTooLarge {
                bytes: decoded as u64,
                max_bytes: MAX_BOOKMARK_BYTES as u64,
            },
        });
    }
    Ok(())
}

/// The decoded size of a base64 bookmark, without decoding it.
fn bookmark_bytes(base64: &str) -> usize {
    let padding = base64
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'=')
        .count();
    (base64.len() / 4 * 3).saturating_sub(padding.min(2))
}

/// The folders without the one with `id`; `None` when it was not there.
pub fn without_folder(saved: &[Folder], id: &str) -> Option<Vec<Folder>> {
    saved.iter().any(|folder| folder.id == id).then(|| {
        saved
            .iter()
            .filter(|folder| folder.id != id)
            .cloned()
            .collect()
    })
}

/// The folders with what the helper just reported: a renewed bookmark
/// replaces the stale one, and a folder found at a new path keeps it. A
/// folder found where an earlier one already is goes: it is that folder,
/// chosen again through another path (`/tmp/x` is `/private/tmp/x` on
/// macOS), which the panel's path did not show. `None` when nothing changed.
pub fn refreshed(saved: &[Folder], reports: &[FolderReport]) -> Option<Vec<Folder>> {
    let mut changed = false;
    let mut folders: Vec<Folder> = Vec::with_capacity(saved.len());
    let renewed = saved
        .iter()
        .map(|folder| {
            let mut folder = folder.clone();
            for report in reports {
                if let FolderReport::Accessible {
                    id,
                    path,
                    renewed_bookmark_base64,
                } = report
                    && *id == folder.id
                {
                    if *path != folder.path && path.len() <= MAX_PATH_BYTES {
                        folder.path = path.clone();
                        changed = true;
                    }
                    if let Some(renewed) = renewed_bookmark_base64
                        && *renewed != folder.bookmark_base64
                        && bookmark_bytes(renewed) <= MAX_BOOKMARK_BYTES
                    {
                        folder.bookmark_base64 = renewed.clone();
                        changed = true;
                    }
                }
            }
            folder
        })
        .collect::<Vec<_>>();
    for folder in renewed {
        if folders.iter().any(|kept| kept.path == folder.path) {
            changed = true;
        } else {
            folders.push(folder);
        }
    }
    changed.then_some(folders)
}

/// One folder for the helper: which, and the bookmark that opens it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FolderBookmark {
    pub id: String,
    pub bookmark_base64: String,
}

impl From<&Folder> for FolderBookmark {
    fn from(folder: &Folder) -> Self {
        Self {
            id: folder.id.clone(),
            bookmark_base64: folder.bookmark_base64.clone(),
        }
    }
}

/// What the macOS panel returned: the folder chosen, or nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum PickReply {
    Chosen {
        path: String,
        bookmark_base64: String,
    },
    Cancelled,
}

/// One entry of a folder, as `lstat` describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_ms: Option<i64>,
    /// In the list of the folders themselves: why this one cannot be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<FolderCause>,
}

/// The helper's answer to `assistant_files_list`: entries sorted by name,
/// from `after_index`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListReply {
    pub entries: Vec<Entry>,
    pub total: u64,
    pub after_index: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_after_index: Option<u64>,
}

/// What a search may spend, sent with every search so the helper keeps no
/// copy of these numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchLimits {
    pub max_results: u32,
    pub max_visited: u32,
    pub max_files_read: u32,
    pub max_bytes_read: u64,
    pub max_file_bytes: u64,
    pub max_snippet_chars: u32,
    pub deadline_ms: u64,
}

impl SearchLimits {
    pub const ORBIT: Self = Self {
        max_results: MAX_SEARCH_RESULTS,
        max_visited: MAX_SEARCH_VISITED,
        max_files_read: MAX_SEARCH_FILES_READ,
        max_bytes_read: MAX_SEARCH_BYTES_READ,
        max_file_bytes: MAX_SEARCH_FILE_BYTES,
        max_snippet_chars: MAX_SNIPPET_CHARS,
        deadline_ms: SEARCH_DEADLINE_MS,
    };
}

/// One folder a search walks: its bookmark and where in it to start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchRoot {
    pub folder: FolderBookmark,
    pub path: String,
}

/// One match: a file or folder whose name matches, or a line of a text file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchHit {
    pub path: String,
    pub kind: EntryKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    /// The line of a content match, from 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

/// Which ceiling stopped a search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchLimit {
    Results,
    Visited,
    FilesRead,
    BytesRead,
    Deadline,
    /// The answer filled its page of the bridge.
    PageBytes,
}

/// A search that stopped before walking everything, with the ceiling's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchStop {
    pub cause: SearchLimit,
    pub limit: u64,
}

/// The helper's answer to `assistant_files_search`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchReply {
    pub results: Vec<SearchHit>,
    pub visited: u64,
    pub files_read: u64,
    pub bytes_read: u64,
    /// Files whose text was searched only up to `max_file_bytes`.
    pub partial_files: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped: Option<SearchStop>,
}

/// How the helper reads one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "as", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReadAs {
    /// UTF-8 text from byte `after_index`, at most `max_bytes`, ending at a
    /// character boundary.
    Text { after_index: u64, max_bytes: u32 },
    /// The text PDFKit extracts, from page `after_page` (from 0) and byte
    /// `after_index` of its text, at most `max_bytes` and `max_pages` pages.
    Pdf {
        after_page: u32,
        after_index: u64,
        max_bytes: u32,
        max_pages: u32,
    },
    /// The image fitted into `max_width` × `max_height` as a PNG of at most
    /// `max_png_bytes`.
    Image {
        max_png_bytes: u32,
        max_width: u32,
        max_height: u32,
    },
}

/// One page of a PDF's text; `page` counts from 1, as a reader does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PdfPage {
    pub page: u32,
    pub text: String,
}

/// The helper's answer to `assistant_files_read`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "content", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReadReply {
    Text {
        text: String,
        after_index: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next_after_index: Option<u64>,
        size_bytes: u64,
    },
    Pdf {
        pages: Vec<PdfPage>,
        page_count: u32,
        after_page: u32,
        after_index: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next_after_page: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next_after_index: Option<u64>,
    },
    Image {
        png_base64: String,
        width: u32,
        height: u32,
        source_width: u32,
        source_height: u32,
    },
}

/// The bytes one value costs inside a JSON array, its comma included.
fn element_cost(value: &impl Serialize) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len().saturating_add(1))
}

/// The entries that fit `budget`, in order, and where the listing goes on.
/// At least one entry while any remain, so a walk over pages ends.
pub fn fit_entries(
    entries: Vec<Entry>,
    after_index: u64,
    next_after_index: Option<u64>,
    budget: usize,
) -> (Vec<Entry>, Option<u64>) {
    let mut used = 0_usize;
    let mut kept = Vec::new();
    let total = entries.len();
    for entry in entries {
        let cost = element_cost(&entry);
        if !kept.is_empty() && used.saturating_add(cost) > budget {
            break;
        }
        used = used.saturating_add(cost);
        kept.push(entry);
    }
    if kept.len() < total {
        let next = after_index.saturating_add(kept.len() as u64);
        return (kept, Some(next));
    }
    (kept, next_after_index)
}

/// The matches that fit `budget`; a search cut here says so as its stop.
pub fn fit_hits(
    results: Vec<SearchHit>,
    stopped: Option<SearchStop>,
    budget: usize,
) -> (Vec<SearchHit>, Option<SearchStop>) {
    let mut used = 0_usize;
    let mut kept = Vec::new();
    let total = results.len();
    for hit in results {
        let cost = element_cost(&hit);
        if used.saturating_add(cost) > budget {
            break;
        }
        used = used.saturating_add(cost);
        kept.push(hit);
    }
    if kept.len() < total {
        return (
            kept,
            Some(SearchStop {
                cause: SearchLimit::PageBytes,
                limit: budget as u64,
            }),
        );
    }
    (kept, stopped)
}

/// The text that fits `budget` and where the file goes on.
pub fn fit_text(
    text: String,
    after_index: u64,
    next_after_index: Option<u64>,
    budget: usize,
) -> (String, Option<u64>) {
    let keep = fitting_prefix(&text, budget);
    if keep < text.len() {
        let next = after_index.saturating_add(keep as u64);
        let mut text = text;
        text.truncate(keep);
        return (text, Some(next));
    }
    (text, next_after_index)
}

/// What one page entry costs besides its text: `{"page":N,"text":},`.
const PAGE_ENVELOPE_BYTES: usize = 24;

/// The pages that fit `budget`, the last one cut at a character when it
/// does not fit whole, and where the document goes on: the page (from 0)
/// and the byte of its text.
pub fn fit_pages(
    pages: Vec<PdfPage>,
    after_page: u32,
    after_index: u64,
    next: Option<(u32, u64)>,
    budget: usize,
) -> (Vec<PdfPage>, Option<(u32, u64)>) {
    let mut used = 0_usize;
    let mut kept = Vec::new();
    for page in pages {
        let index = page.page.saturating_sub(1);
        let starts_at = if index == after_page { after_index } else { 0 };
        let cost = encoded_len(&page.text).saturating_add(PAGE_ENVELOPE_BYTES);
        if used.saturating_add(cost) <= budget {
            used = used.saturating_add(cost);
            kept.push(page);
            continue;
        }
        let room = budget
            .saturating_sub(used)
            .saturating_sub(PAGE_ENVELOPE_BYTES);
        let keep = fitting_prefix(&page.text, room);
        let resume = (index, starts_at.saturating_add(keep as u64));
        if keep > 0 {
            let mut page = page;
            page.text.truncate(keep);
            kept.push(page);
        }
        return (kept, Some(resume));
    }
    (kept, next)
}

/// What a page of an answer can spend on its results once the path it echoes
/// and its envelope are paid for.
pub fn page_budget(path: &str) -> usize {
    MAX_PAGE_BYTES
        .saturating_sub(encoded_len(path))
        .saturating_sub(1024)
}

/// The JSON bytes one character costs inside a string, as `serde_json` writes it.
fn escaped(character: char) -> usize {
    match character {
        '"' | '\\' | '\n' | '\r' | '\t' | '\u{08}' | '\u{0C}' => 2,
        control if u32::from(control) < 0x20 => 6,
        other => other.len_utf8(),
    }
}

/// The JSON bytes a string costs, quotes included.
pub fn encoded_len(text: &str) -> usize {
    text.chars().map(escaped).sum::<usize>().saturating_add(2)
}

/// How many bytes of `text` fit `budget` once encoded as a JSON string, cut
/// at a character boundary. Every page budget holds thousands of the
/// costliest character (six bytes), so a walk over pages always moves.
pub fn fitting_prefix(text: &str, budget: usize) -> usize {
    let mut used = 2_usize;
    for (index, character) in text.char_indices() {
        let cost = escaped(character);
        if used.saturating_add(cost) > budget {
            return index;
        }
        used = used.saturating_add(cost);
    }
    text.len()
}

#[cfg(test)]
mod tests;
