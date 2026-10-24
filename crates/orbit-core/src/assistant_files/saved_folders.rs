//! The list of folders the user keeps: choosing one, removing one, and what
//! the helper's last report changes in it.
use super::Folder;
use super::{FolderCause, FolderReport, MAX_BOOKMARK_BYTES, MAX_FOLDERS, MAX_PATH_BYTES};
use crate::refusal::Refusal;
use serde::{Deserialize, Serialize};

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
