//! «Silenciar»: the stretches of a recording whose microphone the person
//! muted. The helper sends silence for them (same frames, same timing), so
//! the recording lasts the same and its tracks still line up; these spans
//! say where that silence is, so the hub writes «Micrófono silenciado ·
//! 12:04 – 12:30» there and never reads it as lost words.
//!
//! Times are milliseconds from the recording's start (`started_at_ms`), on
//! the wall clock the hub times its transcript with. A span still open has
//! no end: the microphone is muted now.
use serde::{Deserialize, Serialize};

/// How many times one recording's microphone may be muted.
///
/// Protects: the recording's saved state and its closing manifest, which
/// carry every span (about 40 bytes each); the hub admits as many
/// (`protocol.MAX_MUTED_SPANS`) inside its 512 KiB manifest, which a
/// calendar event can already fill to just under 500 KiB. Why 256: a four
/// hour meeting muted about once a minute. Past it, muting is refused with
/// both numbers and «Pausar» still stops everything. It does not limit how
/// long the microphone stays muted, nor a call with the assistant.
pub const MAX_MUTED_SPANS: usize = 256;

/// One stretch of a muted microphone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutedSpan {
    pub start_ms: i64,
    /// `None` while the microphone is still muted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<i64>,
}

/// The recording already holds [`MAX_MUTED_SPANS`] spans.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "Ya se silenció el micrófono {spans} veces en esta grabación, el máximo es {max_spans}; puedes pausarla"
)]
pub struct TooManyMutedSpans {
    pub spans: usize,
    pub max_spans: usize,
}

/// Whether the microphone is muted now: the last span is still open.
pub fn is_muted(spans: &[MutedSpan]) -> bool {
    spans.last().is_some_and(|span| span.end_ms.is_none())
}

/// Whether `recording` is muted now: still open, its last span too.
pub fn recording_muted(recording: &crate::Recording) -> bool {
    recording.ended_at_ms.is_none() && is_muted(&recording.muted_spans)
}

/// Whether one more span fits.
pub fn may_mute(spans: &[MutedSpan]) -> Result<(), TooManyMutedSpans> {
    if is_muted(spans) || spans.len() < MAX_MUTED_SPANS {
        return Ok(());
    }
    Err(TooManyMutedSpans {
        spans: spans.len(),
        max_spans: MAX_MUTED_SPANS,
    })
}

/// Mutes at `at_ms`; already muted, nothing changes. Past the ceiling
/// nothing is added and the refusal says so.
pub fn mute(spans: &mut Vec<MutedSpan>, at_ms: i64) -> Result<(), TooManyMutedSpans> {
    may_mute(spans)?;
    if !is_muted(spans) {
        spans.push(MutedSpan {
            start_ms: at_ms.max(0),
            end_ms: None,
        });
    }
    Ok(())
}

/// Unmutes at `at_ms`; not muted, nothing changes.
pub fn unmute(spans: &mut [MutedSpan], at_ms: i64) {
    if let Some(span) = spans.last_mut().filter(|span| span.end_ms.is_none()) {
        span.end_ms = Some(at_ms.max(span.start_ms));
    }
}

/// The spans as the closing manifest carries them: each with its end, the
/// one still open ending with the recording at `end_ms`.
pub fn closed(spans: &[MutedSpan], end_ms: i64) -> Vec<MutedSpan> {
    spans
        .iter()
        .map(|span| MutedSpan {
            start_ms: span.start_ms,
            end_ms: Some(span.end_ms.unwrap_or(end_ms).max(span.start_ms)),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_span_opens_on_mute_and_closes_on_unmute_once() {
        let mut spans = Vec::new();
        assert!(!is_muted(&spans));
        assert_eq!(mute(&mut spans, 724_000), Ok(()));
        assert_eq!(mute(&mut spans, 730_000), Ok(()), "already muted");
        assert!(is_muted(&spans));
        unmute(&mut spans, 750_000);
        unmute(&mut spans, 760_000);
        assert_eq!(
            spans,
            [MutedSpan {
                start_ms: 724_000,
                end_ms: Some(750_000)
            }]
        );
        assert!(!is_muted(&spans));
    }

    #[test]
    fn the_open_span_ends_with_the_recording_in_the_manifest() {
        let mut spans = Vec::new();
        assert_eq!(mute(&mut spans, 1_000), Ok(()));
        unmute(&mut spans, 2_000);
        assert_eq!(mute(&mut spans, 5_000), Ok(()));
        assert_eq!(
            closed(&spans, 9_000),
            [
                MutedSpan {
                    start_ms: 1_000,
                    end_ms: Some(2_000)
                },
                MutedSpan {
                    start_ms: 5_000,
                    end_ms: Some(9_000)
                },
            ]
        );
    }

    #[test]
    fn past_the_ceiling_muting_is_refused_with_both_numbers() {
        let mut spans = Vec::new();
        for index in 0..MAX_MUTED_SPANS as i64 {
            assert_eq!(mute(&mut spans, index * 10), Ok(()));
            unmute(&mut spans, index * 10 + 5);
        }
        assert_eq!(
            mute(&mut spans, 1_000_000),
            Err(TooManyMutedSpans {
                spans: MAX_MUTED_SPANS,
                max_spans: MAX_MUTED_SPANS
            })
        );
        assert_eq!(spans.len(), MAX_MUTED_SPANS);
        assert!(!is_muted(&spans));
    }
}
