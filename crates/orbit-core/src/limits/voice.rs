//! Voice: the assistant's spoken answers.
use super::Limit;

// The playback decoder accepts at most 4 MiB of compressed audio; measured
// base64 expansion is 5,592,408 bytes. This bounds decoded audio, not a line.
pub const MAX_SPEECH_CLIP_BYTES: usize = 4 * 1024 * 1024;

pub(super) fn limits() -> Vec<Limit> {
    vec![item!(
        MAX_SPEECH_CLIP_BYTES,
        "bytes",
        "speech playback clip"
    )]
}
