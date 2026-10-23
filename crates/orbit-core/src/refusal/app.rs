//! Orbit itself: the daemon still recovering what it kept when it closed.

macro_rules! app_refusals {
    ([$next:ident $(, $rest:ident)*] ; $($entries:tt)*) => {
        $next! { [$($rest),*] ; $($entries)*
            Recovering = 10 => "recovering" {
                step: crate::RecoveryStep,
                elapsed_ms: u64,
            }, retry Retry::Never;
        }
    };
}
pub(super) use app_refusals;

/// The recovery phase in the messages emitted by startup and typed refusals.
pub fn recovery_step_name(step: crate::RecoveryStep) -> &'static str {
    match step {
        crate::RecoveryStep::Chunks => "chunks",
        crate::RecoveryStep::UploadJobs => "upload_jobs",
        crate::RecoveryStep::Snapshot => "snapshot",
        crate::RecoveryStep::Environment => "environment",
        crate::RecoveryStep::Operations => "operations",
        crate::RecoveryStep::CallPreference => "call_preference",
    }
}
