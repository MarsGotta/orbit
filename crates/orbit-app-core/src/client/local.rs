use super::*;

impl DaemonClient {
    /// Reprojects retained facts so already visible native text changes at once.
    pub fn set_presentation_copy(&self, copy: crate::PresentationCopy) {
        self.inner
            .frames
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set_copy(copy);
    }

    pub(super) fn presentation_copy(&self) -> crate::PresentationCopy {
        self.inner
            .frames
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .copy
    }

    pub fn set_login_item(&self, state: LoginItemState) -> AppSnapshot {
        self.update_local(|snapshot| snapshot.settings.login_item = state)
    }

    pub fn set_update(&self, mut update: UpdateSnapshot) -> AppSnapshot {
        self.update_local(|snapshot| {
            if update.last_installation.is_none() {
                update.last_installation = snapshot.settings.update.last_installation.clone();
            }
            snapshot.settings.update = update;
        })
    }

    pub async fn update_installation_blocker(&self) -> Option<String> {
        let topics = self.inner.topics.read().await;
        crate::UpdateService::installation_blocker(&topics)
    }

    pub fn set_update_result(&self, state: UpdateState) -> AppSnapshot {
        if self.local_snapshot().settings.update.state == crate::SettingsUpdateState::Installing {
            return self.composed_snapshot();
        }
        self.set_update(project_update(state))
    }

    pub fn complete_onboarding(&self) -> AppSnapshot {
        self.update_local(|snapshot| snapshot.settings.onboarding_done = true)
    }

    pub fn set_profile_name(&self, name: Option<String>) -> Result<AppSnapshot, AppError> {
        let name = normalize_profile_name(name);
        persist_profile_name(&self.inner.paths, name.as_deref())?;
        Ok(self.update_local(|snapshot| apply_profile_name(snapshot, name)))
    }

    /// Ask the daemon to refresh a list a view depends on, and keep quiet when
    /// it cannot. Opening Briefing or the start dialog is not an order
    /// the person gave, and the snapshot already says why each list is empty —
    /// no hub credential for activity, no helper for the audio inventory — so
    /// a failure here must not shout over the view that was just opened.
    pub(super) fn update_local(&self, update: impl FnOnce(&mut AppSnapshot)) -> AppSnapshot {
        let mut updated = self.local_snapshot();
        self.inner.snapshots.send_modify(|snapshot| {
            snapshot.revision = snapshot.revision.saturating_add(1);
            update(snapshot);
            updated.clone_from(snapshot);
        });
        let mut publisher = self
            .inner
            .frames
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        publisher.observe(&updated);
        updated
    }
}

pub(super) fn normalize_profile_name(name: Option<String>) -> Option<String> {
    name.map(|value| {
        value
            .trim()
            .chars()
            .take(MAX_PROFILE_NAME_CHARS)
            .collect::<String>()
    })
    .filter(|value| !value.is_empty())
}

pub(super) fn profile_name_path(paths: &Paths) -> std::path::PathBuf {
    paths.root.join(PROFILE_NAME_FILE)
}

pub(super) fn load_profile_name(paths: &Paths) -> Option<String> {
    let bytes = fs::read(profile_name_path(paths)).ok()?;
    if bytes.len() > MAX_PROFILE_NAME_BYTES {
        return None;
    }
    normalize_profile_name(String::from_utf8(bytes).ok())
}

pub(super) fn persist_profile_name(paths: &Paths, name: Option<&str>) -> Result<(), AppError> {
    paths
        .initialize()
        .and_then(|()| {
            orbit_ipc::atomic_private_write(
                &profile_name_path(paths),
                name.unwrap_or_default().as_bytes(),
            )
        })
        .map_err(|error| AppError::settings(error.to_string()))
}

pub(super) fn apply_profile_name(snapshot: &mut AppSnapshot, name: Option<String>) {
    snapshot.settings.profile = Some(crate::ProfileSettingsSnapshot { name });
}
