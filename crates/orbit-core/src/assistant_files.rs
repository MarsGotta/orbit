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
//!
//! The pieces: `admission` decides whether a requested path may be read,
//! `saved_folders` edits the list the user keeps, `helper_messages` is what
//! the helper is asked and answers, and `page_fit` cuts an answer to its page.
use crate::refusal::{Expected, Parameter, Refusal};
use serde::{Deserialize, Serialize};

mod admission;
mod helper_messages;
mod page_fit;
mod saved_folders;

pub use admission::{
    Admitted, Asked, EntryKind, FolderReport, FoldersReply, RequestedPath, admit, folder_paths,
};
pub use helper_messages::{
    Entry, ListReply, PdfPage, PickReply, ReadAs, ReadReply, SearchHit, SearchLimit, SearchLimits,
    SearchReply, SearchRoot, SearchStop,
};
pub use page_fit::{
    encoded_len, fit_entries, fit_hits, fit_pages, fit_text, fitting_prefix, page_budget,
};
pub use saved_folders::{
    FolderBookmark, check_folder, refreshed, room_for_another, with_folder, without_folder,
};

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

#[cfg(test)]
mod tests;
