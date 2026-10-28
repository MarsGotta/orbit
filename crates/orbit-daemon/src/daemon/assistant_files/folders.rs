//! The folders the user gave the assistant: kept across restarts, added with
//! the Mac's picker, removed, and checked again when the helper comes back.
use super::*;

/// What the screen says when a folder chosen in the panel is not kept: the
/// refusal's code with its cause and numbers, never the folder's path.
pub(super) fn not_added(refusal: &Refusal) -> Notice {
    let mut notice = Notice::new(NoticeCode::AssistantFolderNotAdded)
        .with_token(NoticeKey::Code, refusal.code().name());
    if let Refusal::TooMany { count, max, .. } = refusal {
        notice = notice
            .with(NoticeKey::Count, *count)
            .with(NoticeKey::Max, *max);
    }
    if let Refusal::FolderUnavailable { cause, .. } = refusal
        && let FolderCause::PathTooLong { bytes, max_bytes }
        | FolderCause::BookmarkTooLarge { bytes, max_bytes } = cause
    {
        let kind = if matches!(cause, FolderCause::PathTooLong { .. }) {
            "path_too_long"
        } else {
            "bookmark_too_large"
        };
        notice = notice
            .with_token(NoticeKey::Cause, kind)
            .with(NoticeKey::Bytes, *bytes)
            .with(NoticeKey::MaxBytes, *max_bytes);
    }
    notice
}

impl Runtime {
    /// Reads the folders at startup. A row that cannot be read is said and
    /// published with its cause, never as an empty list.
    pub(in crate::daemon) fn recover_assistant_folders(&mut self) {
        let loaded = self.reader.load_assistant_folders();
        if loaded.is_err() {
            self.log("assistant_folders_unreadable", &[]);
        }
        self.files.folders =
            Some(loaded.map_err(|error| format!("Could not read assistant folders ({error})")));
    }

    pub(super) fn saved_folders(&mut self) -> Result<Vec<Folder>, Refusal> {
        if self.files.folders.is_none() {
            self.recover_assistant_folders();
        }
        match &self.files.folders {
            Some(Ok(folders)) => Ok(folders.clone()),
            Some(Err(cause)) => Err(Refusal::StorageFailed {
                operation: Operation::Files,
                cause: Some(cause.clone().into()),
            }),
            None => Ok(Vec::new()),
        }
    }

    /// «Añadir carpeta»: the panel opens on the Mac; the choice arrives in
    /// `files_response`. Refused at once, by name, when no folder can be added.
    pub(in crate::daemon) fn add_assistant_folder(&mut self) -> Result<()> {
        if self.state.privacy_stopped() {
            return Err(Refusal::PrivacyStopped { since_ms: None }.into());
        }
        if !self.state.capabilities().native_helper() {
            return Err(Refusal::HelperNotRunning.into());
        }
        if self.files.pick.is_some() {
            return Err(Refusal::Busy {
                operation: Operation::Files,
                retry_after_ms: None,
            }
            .into());
        }
        let saved = self.saved_folders()?;
        files::room_for_another(&saved)?;
        let id = self
            .helper
            .send_files_command("assistant_folder_pick", json!({}))?;
        self.files.pick = Some(id);
        Ok(())
    }

    /// «Quitar»: forgotten at once, bookmark included, also by a request
    /// already under way: its next step neither reads in it nor answers
    /// with what was read there.
    pub(in crate::daemon) async fn remove_assistant_folder(&mut self, id: &str) -> Result<()> {
        let saved = self.saved_folders()?;
        if let Some(folders) = files::without_folder(&saved, id) {
            self.store_folders(folders).await?;
        }
        self.files.access.remove(id);
        if let Some(request) = self.files.request.as_mut() {
            request.saved.retain(|folder| folder.id != id);
        }
        Ok(())
    }

    pub(super) async fn store_folders(&mut self, folders: Vec<Folder>) -> Result<()> {
        let kept = folders.clone();
        self.store_actor
            .run(move |store| Ok(store.save_assistant_folders(&kept)?))
            .await
            .context("Could not save assistant folders")?;
        self.files.folders = Some(Ok(folders));
        Ok(())
    }

    /// Asks the helper whether every folder still opens, for Ajustes.
    pub(in crate::daemon) fn check_assistant_folders(&mut self) {
        let Ok(saved) = self.saved_folders() else {
            return;
        };
        if saved.is_empty()
            || self.files.check.is_some()
            || !self.state.capabilities().native_helper()
        {
            return;
        }
        let bookmarks: Vec<FolderBookmark> = saved.iter().map(FolderBookmark::from).collect();
        if let Ok(id) = self
            .helper
            .send_files_command("assistant_folders", json!({"folders":bookmarks}))
        {
            self.files.check = Some(id);
        }
    }

    pub(super) async fn on_folder_picked(&mut self, response: &NativeResponseFields) -> Result<()> {
        if !response.ok {
            let error = response.error.as_ref();
            let code = error.map_or("assistant_folder_pick_failed", |error| error.code.as_str());
            let detail = error.and_then(|error| error.detail.as_ref());
            let mut notice =
                Notice::new(NoticeCode::AssistantFolderNotAdded).with_token(NoticeKey::Code, code);
            if let Some(kind) = detail
                .and_then(|detail| detail.pointer("/cause/kind"))
                .and_then(Value::as_str)
            {
                notice = notice.with_token(NoticeKey::Cause, kind);
            }
            if let Some(status) = detail
                .and_then(|detail| detail.get("os_status"))
                .and_then(Value::as_i64)
            {
                notice = notice.with(NoticeKey::Status, status);
            }
            self.say(notice);
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
        let chosen = Folder {
            id: uuid::Uuid::new_v4().to_string(),
            path,
            bookmark_base64,
        };
        let added = match self.saved_folders() {
            Ok(saved) => files::with_folder(&saved, chosen),
            Err(refusal) => Err(refusal),
        };
        match added {
            Ok(Some(folders)) => {
                if self.store_folders(folders).await.is_err() {
                    self.log("assistant_folders_unsaved", &[]);
                    self.say(not_added(&Refusal::StorageFailed {
                        operation: Operation::Files,
                        cause: None,
                    }));
                    return Ok(());
                }
                self.check_assistant_folders();
            }
            Ok(None) => {}
            Err(refusal) => self.say(not_added(&refusal)),
        }
        Ok(())
    }

    /// What the helper just said of each folder still kept: shown in
    /// Ajustes, and a renewed bookmark or a moved folder stored. A report of
    /// a folder removed meanwhile changes nothing.
    pub(super) async fn take_reports(&mut self, reports: &[FolderReport]) {
        let Ok(saved) = self.saved_folders() else {
            return;
        };
        let reports: Vec<FolderReport> = reports
            .iter()
            .filter(|report| saved.iter().any(|folder| folder.id == report.id()))
            .cloned()
            .collect();
        for report in &reports {
            let access = match report {
                FolderReport::Accessible { .. } => FolderAccess::Accessible,
                FolderReport::Unavailable { cause, .. } => FolderAccess::Unavailable {
                    cause: cause.clone(),
                },
            };
            self.files.access.insert(report.id().to_owned(), access);
        }
        if let Some(folders) = files::refreshed(&saved, &reports) {
            let kept: Vec<String> = folders.iter().map(|folder| folder.id.clone()).collect();
            if self.store_folders(folders).await.is_err() {
                self.log("assistant_folders_refresh_unsaved", &[]);
                return;
            }
            self.files.access.retain(|id, _| kept.contains(id));
        }
    }
}
