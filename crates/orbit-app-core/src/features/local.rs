use super::FeatureInput;
use crate::{AppSnapshot, LoginItemState, ProfileSettingsSnapshot, UpdateSnapshot};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../apps/desktop/src/bridge/generated/")]
pub struct LocalView {
    pub profile: Option<ProfileSettingsSnapshot>,
    pub login_item: LoginItemState,
    pub update: UpdateSnapshot,
    pub onboarding_done: bool,
}

pub type View = LocalView;

pub fn project(input: &FeatureInput<'_>) -> View {
    LocalView {
        profile: (*input.local.profile).clone(),
        login_item: (*input.local.login_item).clone(),
        update: UpdateSnapshot {
            install_blocked: crate::UpdateService::installation_blocker_reason(input.topics).map(
                |reason| {
                    input
                        .copy
                        .text(crate::PresentationText::UpdateBlocked(reason))
                },
            ),
            ..(*input.local.update).clone()
        },
        onboarding_done: input.local.onboarding_done,
    }
}

pub fn apply(view: &View, snapshot: &mut AppSnapshot) {
    snapshot.settings.profile = view.profile.clone();
    snapshot.settings.login_item = view.login_item.clone();
    snapshot.settings.update = view.update.clone();
    snapshot.settings.onboarding_done = view.onboarding_done;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "command", rename_all = "snake_case")]
#[ts(export, export_to = "../../apps/desktop/src/bridge/generated/")]
pub enum LocalCommand {
    LoginItem { on: bool },
    UpdateCheck,
    UpdateInstall,
    OnboardingDone,
    ProfileSet { name: Option<String> },
}
