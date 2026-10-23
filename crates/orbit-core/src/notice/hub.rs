//! The hub link: pairing, the device's identity, the hub's answers and its storage.

macro_rules! hub_notices {
    ([$next:ident $(, $rest:ident)*] ; $($entries:tt)*) => {
        $next! { [$($rest),*] ; $($entries)*
            DeviceRevoked = 87 => Fixable,
            /// The hub link's problem; the link state is shown, the cause is logged.
            HubProblem = 88 => Silent,
            /// The hub refused or failed a request (`code`).
            HubError = 89 => Failed,
            /// The hub did not say which device it confirmed; the link was closed.
            HubDeviceIdentityMissing = 90 => Failed,
            /// The hub confirmed another device, or this Mac has no identity: pair again.
            DevicePairingMismatch = 91 => Fixable,
            /// The hub's storage is nearly full: `bytes` it uses of `max_bytes`,
            /// `remaining` bytes left, and `cause` what sets the limit, `disk` or
            /// `quota`. Someone must free room on the server before meetings stop
            /// reaching it. Nothing is deleted on its own. (Existing keys: every
            /// new key grows the notices frame, `notices_frame_fits_…`.)
            HubStorageNearlyFull = 92 => Fixable,
            /// Pairing failed: check the code and the hub's address.
            PairingFailed = 107 => Fixable,
            Paired = 108 => Done,
            HubSendFailed = 111 => Silent,
            /// The hub has no room for more audio (507, `code`), with what it said:
            /// `bytes` used, `max_bytes`, `remaining`. Retried every few minutes.
            HubStorageFull = 122 => Fixable,
            /// Handling a hub event, a worker's result or a note's answer failed;
            /// the error is in daemon.log under the same op.
            HubEventFailed = 125 => Failed,
        }
    };
}
pub(super) use hub_notices;
