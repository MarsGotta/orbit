//! Recording and capture: a meeting still kept on this Mac blocks what would
//! lose it.

macro_rules! recording_refusals {
    ([$next:ident $(, $rest:ident)*] ; $($entries:tt)*) => {
        $next! { [$($rest),*] ; $($entries)*
            PendingRecording = 27 => "pending_recording" {
                id: String,
                title: String,
                phase: crate::CapturePhase,
                sealed_chunks: u64,
                gaps: u64,
                expires_at_ms: i64,
            }, retry Retry::AfterFix;
        }
    };
}
pub(super) use recording_refusals;
