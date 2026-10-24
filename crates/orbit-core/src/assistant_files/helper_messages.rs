//! What the helper is asked and answers when it lists, searches and reads
//! inside the folders.
use super::{EntryKind, FolderBookmark, FolderCause};
use super::{
    MAX_SEARCH_BYTES_READ, MAX_SEARCH_FILE_BYTES, MAX_SEARCH_FILES_READ, MAX_SEARCH_RESULTS,
    MAX_SEARCH_VISITED, MAX_SNIPPET_CHARS, SEARCH_DEADLINE_MS,
};
use serde::{Deserialize, Serialize};

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
