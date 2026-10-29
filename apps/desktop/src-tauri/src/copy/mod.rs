//! Native presentation copy. Both catalogues must initialize every key.
use orbit_core::language::ResolvedLanguage;
mod en;
mod es;
pub mod presentation;

#[derive(Debug)]
#[cfg_attr(test, derive(serde::Serialize))]
pub struct Copy {
    pub update_recording_unobserved: &'static str,
    pub update_call_unobserved: &'static str,
    pub audio_transcription: &'static str,
    pub application_menu: &'static [(&'static str, &'static str)],
    pub read_failed: &'static str,
    pub read_failures: &'static [(&'static str, &'static str)],
    pub open: &'static str,
    pub record: &'static str,
    pub finish: &'static str,
    pub discard: &'static str,
    pub discard_question: &'static str,
    pub confirm_discard: &'static str,
    pub keep_recording: &'static str,
    pub mute: &'static str,
    pub unmute: &'static str,
    pub stop: &'static str,
    pub quit: &'static str,
    pub update: &'static str,
    pub approval_title: &'static str,
    pub approval_body: &'static str,
    pub question_title: &'static str,
    pub question_body: &'static str,
    pub answer_title: &'static str,
    pub answer_body: &'static str,
    pub call_silence: &'static str,
    pub recording_silence: &'static str,
    pub cut: &'static str,
    pub resumed: &'static str,
    pub closed: &'static str,
    pub unknown_duration: &'static str,
    pub helper_lost: &'static str,
    pub session_inactive: &'static str,
    pub restart: &'static str,
    pub heartbeat_expired: &'static str,
    pub privacy_stop: &'static str,
    pub finalize_timeout: &'static str,
    pub cause_not_saved: &'static str,
    pub capture_failed: &'static str,
    pub panel_title: &'static str,
    pub meeting_title: &'static str,
    pub ask_about: &'static str,
    pub update_recording_unknown: &'static str,
    pub update_call_unknown: &'static str,
    pub update_recording_stale: &'static str,
    pub update_call_stale: &'static str,
    pub update_recording_pending: &'static str,
    pub update_call_pending: &'static str,
    pub update_recording_refused: &'static str,
    pub update_call_refused: &'static str,
    pub update_recording_live: &'static str,
    pub update_call_live: &'static str,
    pub pending_unobserved: &'static str,
    pub pending_helper: &'static str,
    pub pending_reading: &'static str,
    pub pending_daemon: &'static str,
    pub pending_transcription: &'static str,
    pub yes: &'static str,
    pub no: &'static str,
    pub audio_unobserved: &'static str,
    pub audio_helper: &'static str,
    pub audio_reading: &'static str,
    pub audio_daemon: &'static str,
}

pub const fn spanish() -> &'static Copy {
    &es::COPY
}
pub const fn english() -> &'static Copy {
    &en::COPY
}
pub fn for_language(language: ResolvedLanguage) -> &'static Copy {
    match language {
        ResolvedLanguage::Spanish => spanish(),
        ResolvedLanguage::English => english(),
    }
}

pub fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut result = String::new();
    let mut rest = template;
    while let Some((before, tail)) = rest.split_once('{') {
        result.push_str(before);
        let Some((key, after)) = tail.split_once('}') else {
            result.push('{');
            rest = tail;
            break;
        };
        if let Some((_, value)) = values.iter().find(|(name, _)| *name == key) {
            result.push_str(value);
        } else {
            result.push('{');
            result.push_str(key);
            result.push('}');
        }
        rest = after;
    }
    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_spanish_key_has_english_copy_and_the_same_placeholders() {
        // Struct initialization makes a missing key a compile error. Check
        // nonempty values and placeholders too, including dynamic messages.
        fn entries(source: &str) -> std::collections::BTreeMap<&str, &str> {
            source
                .lines()
                .filter_map(|line| line.trim().split_once(": "))
                .filter(|(_, value)| value.starts_with('"'))
                .collect()
        }
        assert_eq!(
            super::spanish()
                .read_failures
                .iter()
                .map(|(key, _)| key)
                .collect::<Vec<_>>(),
            super::english()
                .read_failures
                .iter()
                .map(|(key, _)| key)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            super::spanish()
                .application_menu
                .iter()
                .map(|(key, _)| key)
                .collect::<Vec<_>>(),
            super::english()
                .application_menu
                .iter()
                .map(|(key, _)| key)
                .collect::<Vec<_>>()
        );
        let es = entries(include_str!("es.rs"));
        let en = entries(include_str!("en.rs"));
        assert_eq!(es.keys().collect::<Vec<_>>(), en.keys().collect::<Vec<_>>());
        for (key, spanish) in es {
            let english = en[key];
            assert_ne!(english, "\"\",");
            let placeholders = |value: &str| {
                value
                    .split('{')
                    .skip(1)
                    .filter_map(|tail| tail.split('}').next())
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            };
            assert_eq!(placeholders(spanish), placeholders(english), "{key}");
        }
    }
}

#[cfg(test)]
mod harness_export;
