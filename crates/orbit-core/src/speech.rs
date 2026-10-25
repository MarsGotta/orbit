//! Bounded text parts for sequential read-aloud. No provider or clock decisions.
use std::collections::VecDeque;

/// Keeps synthesis requests short enough to target roughly 20 seconds at normal
/// speaking pace, leaving room below the helper's independent 60-second decoded
/// audio ceiling. This is a text allocation/prosody bound, not a measurement of
/// a provider's duration; the decoder still enforces its audio bound.
pub const SPEECH_PART_CHARS: usize = 240;

/// Prefer complete sentences, then whitespace for an unusually long sentence.
/// Preserve every character, including punctuation and Unicode, exactly once.
pub fn parts(text: &str) -> VecDeque<String> {
    split(text, SPEECH_PART_CHARS)
}

/// A provider produced over 60 seconds even for one short part. Retry only
/// synthesis with smaller text; no device input action is ever repeated.
pub fn shorter_parts(text: &str) -> Option<VecDeque<String>> {
    let count = text.chars().count();
    (count > 1).then(|| split(text, count / 2))
}

fn split(text: &str, maximum: usize) -> VecDeque<String> {
    let mut parts = VecDeque::new();
    let mut rest = text;
    while !rest.is_empty() {
        let end = rest.char_indices().nth(maximum).map(|(i, _)| i);
        let Some(end) = end else {
            parts.push_back(rest.to_owned());
            break;
        };
        let prefix = &rest[..end];
        let sentence = prefix
            .char_indices()
            .rev()
            .find(|(_, c)| matches!(c, '.' | '!' | '?' | '\n'));
        let space = prefix.char_indices().rev().find(|(_, c)| c.is_whitespace());
        let cut = sentence.or(space).map_or(end, |(i, c)| i + c.len_utf8());
        parts.push_back(rest[..cut].to_owned());
        rest = &rest[cut..];
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentence_parts_preserve_words_punctuation_and_unicode() {
        for text in [
            "La sala Norte está lista. ¿Vamos? ".repeat(100),
            "á".repeat(900),
        ] {
            let pieces = parts(&text);
            assert!(pieces.len() > 1);
            assert!(
                pieces
                    .iter()
                    .all(|part| part.chars().count() <= SPEECH_PART_CHARS)
            );
            assert_eq!(pieces.into_iter().collect::<String>(), text);
        }
    }
}
