//! Orbit's own words, independent of meeting content and assistant replies.
use orbit_core::{silence::PresenceVocabulary, wire::native::SpeechLocale};

mod en;
mod es;

pub(super) struct SpokenCopy {
    pub(super) meeting_title: &'static str,
    pub(super) call_question: &'static str,
    pub(super) answers: PresenceVocabulary,
}

pub(super) fn copy(locale: SpeechLocale) -> &'static SpokenCopy {
    match locale {
        SpeechLocale::Spanish => &es::COPY,
        SpeechLocale::EnglishUS | SpeechLocale::EnglishGB => &en::COPY,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_core::silence::{PresenceReply, presence_reply};

    #[test]
    fn every_spanish_speech_entry_has_an_english_equivalent() {
        // Both tables must fill every typed field, including each reply intent.
        for table in [&es::COPY, &en::COPY] {
            assert!(!table.call_question.is_empty());
            for alternatives in table.answers.keep {
                assert!(!alternatives.is_empty());
                for text in alternatives {
                    assert!(!text.is_empty());
                    assert_eq!(presence_reply(text, &table.answers), PresenceReply::Keep);
                }
            }
            assert!(!table.answers.hang_up.is_empty());
            for text in table.answers.hang_up {
                assert_eq!(presence_reply(text, &table.answers), PresenceReply::HangUp);
            }
        }
    }
}
