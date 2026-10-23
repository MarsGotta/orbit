//! Privacy and permissions: the user's stop and the system permissions a capture needs.

macro_rules! privacy_notices {
    ([$next:ident $(, $rest:ident)*] ; $($entries:tt)*) => {
        $next! { [$($rest),*] ; $($entries)*
            MicrophonePermissionMissing = 18 => Fixable,
            ScreenPermissionMissing = 19 => Fixable,
            /// The user's privacy stop is in force: rearm to continue.
            PrivacyStopActive = 39 => Fixable,
            PrivacyStopped = 83 => Done,
            PrivacyStopKept = 84 => Fixable,
            PrivacyRearmed = 85 => Done,
            PrivacyStopConfirmed = 86 => Silent,
        }
    };
}
pub(super) use privacy_notices;
