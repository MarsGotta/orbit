//! The link with the Mac's helper (`OrbitNative`): it is not running, or an order to it
//! failed, was degraded or never confirmed.

macro_rules! helper_notices {
    ([$next:ident $(, $rest:ident)*] ; $($entries:tt)*) => {
        $next! { [$($rest),*] ; $($entries)*
            /// The Mac refused or degraded an order (`step` is the order, `code` why).
            MacCommandRefused = 32 => Failed,
            MacCommandDegraded = 33 => Failed,
            /// The helper reported an error code.
            HelperError = 37 => Failed,
            /// Orbit.app is not running: open it.
            HelperNotRunning = 38 => Fixable,
            HelperCommandUnconfirmed = 112 => Silent,
            /// The helper answered an order with an error (`step` is the order,
            /// `code` the helper's code). Every failed order, whatever else it says.
            HelperCommandFailed = 149 => Silent,
        }
    };
}
pub(super) use helper_notices;
