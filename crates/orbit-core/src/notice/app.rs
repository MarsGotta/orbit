//! Orbit itself: its shortcuts, its closing, the local store and its recovery, and the
//! failures it contains without cutting anything live. A notice about Orbit's own
//! processes and their supervision goes here.

macro_rules! app_notices {
    ([$next:ident $(, $rest:ident)*] ; $($entries:tt)*) => {
        $next! { [$($rest),*] ; $($entries)*
            /// A shortcut's order was refused (`code`).
            ShortcutRefused = 40 => Failed,
            OrbitClosing = 93 => Working,
            /// A step of a stop or a close failed (`step`); the error is in daemon.log.
            StopStepFailed = 94 => Failed,
            /// Work is still being saved: Orbit must stay open until it finishes.
            PendingWorkKeepOpen = 95 => Fixable,
            /// A failure was contained without cutting anything live (`cause`).
            ContainedFailure = 96 => Failed,
            /// The local store failed; the cause is in daemon.log.
            StoreProblem = 123 => Failed,
            /// Orbit closed with work still in memory: `audio_unsaved` when the store
            /// did not confirm audio, `reserve_exhausted` when the 64 MiB reserve ran
            /// out; the counts are in daemon.log.
            ClosedWithPendingWork = 124 => Failed,
            WorkFailed = 126 => Failed,
            StoreRecovered = 127 => Silent,
            /// Recovery found invalid, missing or incomplete files; they are kept.
            StoreRecoveryLostFiles = 128 => Failed,
            /// The interface could not connect to Orbit.
            ConnectFailed = 139 => Silent,
        }
    };
}
pub(super) use app_notices;
