//! Files: the folders the user gave the assistant, and what it may read in them.

macro_rules! files_refusals {
    ([$next:ident $(, $rest:ident)*] ; $($entries:tt)*) => {
        $next! { [$($rest),*] ; $($entries)*
            // The assistant asked for a file and the user has given it no folder.
            NoFolders = 43 => "no_folders", retry Retry::AfterFix;
            // The path, as asked and as it resolves on the Mac (links and `..`
            // followed), is in none of the folders. Names them all so the caller can
            // correct its path.
            OutsideFolders = 44 => "outside_folders" {
                path: String,
                resolved: Option<String>,
                folders: Vec<String>,
            }, retry Retry::AfterFix;
            // The folder's bookmark does not open it, and why.
            FolderUnavailable = 45 => "folder_unavailable" {
                path: String,
                cause: crate::assistant_files::FolderCause,
            }, retry Retry::AfterFix;
            // A path inside a folder that cannot be read as asked, and why.
            FileUnavailable = 46 => "file_unavailable" {
                path: String,
                cause: crate::assistant_files::FileCause,
            }, retry Retry::AfterFix;
            // A PDF or an image over the size Orbit reads, with both sizes.
            FileTooLarge = 47 => "file_too_large" {
                path: String,
                content: crate::assistant_files::ContentKind,
                bytes: u64,
                max_bytes: u64,
            }, retry Retry::AfterFix;
        }
    };
}
pub(super) use files_refusals;
