//! App-aware system preferences, never a shell's LANG environment variable.
use orbit_core::language::{LanguageChoice, ResolvedLanguage};
use std::sync::{Mutex, PoisonError};
use tauri::Manager;

pub struct NativeLanguage {
    resolved: Mutex<ResolvedLanguage>,
    applied: Mutex<Option<ResolvedLanguage>>,
    preferred: fn() -> Vec<String>,
}

impl NativeLanguage {
    pub fn new(choice: LanguageChoice, preferred: fn() -> Vec<String>) -> Self {
        Self {
            resolved: Mutex::new(choice.resolve(&preferred())),
            applied: Mutex::new(None),
            preferred,
        }
    }

    fn resolve(&self, choice: LanguageChoice) -> ResolvedLanguage {
        choice.resolve(&(self.preferred)())
    }
}

#[cfg(target_os = "macos")]
pub fn preferred_languages() -> Vec<String> {
    objc2_foundation::NSLocale::preferredLanguages()
        .iter()
        .map(|tag| tag.to_string())
        .collect()
}

#[cfg(not(target_os = "macos"))]
pub fn preferred_languages() -> Vec<String> {
    Vec::new()
}

pub fn current<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> ResolvedLanguage {
    app.try_state::<NativeLanguage>()
        .map_or(ResolvedLanguage::English, |state| {
            *state
                .resolved
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
        })
}

pub fn follow<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    choice: Option<LanguageChoice>,
) -> tauri::Result<()> {
    let before = current(app);
    let language = app.try_state::<NativeLanguage>().map_or_else(
        || choice.unwrap_or_default().resolve(&preferred_languages()),
        |state| state.resolve(choice.unwrap_or_default()),
    );
    if let Some(state) = app.try_state::<NativeLanguage>() {
        *state
            .resolved
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = language;
    }
    if before != language
        && let Some(client) = app.try_state::<orbit_app_core::DaemonClient>()
    {
        client.set_presentation_copy(crate::copy::presentation::renderer(language));
    }
    let applied = app
        .try_state::<NativeLanguage>()
        .and_then(|state| *state.applied.lock().unwrap_or_else(PoisonError::into_inner));
    if applied != Some(language) {
        crate::application_menu::install(app, crate::copy::for_language(language))?;
        if let Some(panel) = app.get_webview_window(crate::panel::PANEL_LABEL) {
            panel.set_title(crate::copy::for_language(language).panel_title)?;
        }
    }
    crate::tray::refresh_language(app, language)?;
    if let Some(state) = app.try_state::<NativeLanguage>() {
        *state.applied.lock().unwrap_or_else(PoisonError::into_inner) = Some(language);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_status_application_records_the_language_that_was_rendered()
    -> Result<(), Box<dyn std::error::Error>> {
        let app = tauri::test::mock_builder()
            .manage(NativeLanguage::new(LanguageChoice::English, || {
                vec!["en-GB".into()]
            }))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))?;
        for language in [LanguageChoice::Spanish, LanguageChoice::English] {
            follow(app.handle(), Some(language))?;
            let state = app.state::<NativeLanguage>();
            assert_eq!(
                *state.applied.lock().unwrap_or_else(PoisonError::into_inner),
                Some(language.resolve(&["en-GB"]))
            );
        }
        Ok(())
    }

    #[test]
    fn system_uses_the_first_supported_macos_preference_and_explicit_choices_override_it() {
        let state = NativeLanguage::new(LanguageChoice::System, || {
            vec!["fr-FR".into(), "es-MX".into(), "en-GB".into()]
        });
        assert_eq!(
            state.resolve(LanguageChoice::System),
            ResolvedLanguage::Spanish
        );
        assert_eq!(
            state.resolve(LanguageChoice::English),
            ResolvedLanguage::English
        );
        assert_eq!(
            state.resolve(LanguageChoice::Spanish),
            ResolvedLanguage::Spanish
        );
        let state = NativeLanguage::new(LanguageChoice::System, || vec!["fr-FR".into()]);
        assert_eq!(
            state.resolve(LanguageChoice::System),
            ResolvedLanguage::English
        );
    }
}
