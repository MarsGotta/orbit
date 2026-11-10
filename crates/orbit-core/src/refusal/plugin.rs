//! Plugins: the device's refusals of a call by size, the codes the contract
//! gives both the device and the hub (`contracts/plugin-v1.md`, «Refusals»).

macro_rules! plugin_refusals {
    ([$next:ident $(, $rest:ident)*] ; $($entries:tt)*) => {
        $next! { [$($rest),*] ; $($entries)*
            // The call's body, as the device would send it, is over
            // `PLUGIN_CALL_MAX_BYTES`; it never left the device.
            PluginCallTooLarge = 49 => "plugin_call_too_large" {
                bytes: u64,
                max_bytes: u64,
            }, retry Retry::AfterFix;
            // The hub's answer is over `PLUGIN_RESPONSE_MAX_BYTES`: `bytes`
            // is what had been read when that was seen, or its announced length.
            PluginResponseTooLarge = 50 => "plugin_response_too_large" {
                bytes: u64,
                max_bytes: u64,
            }, retry Retry::AfterFix;
        }
    };
}
pub(super) use plugin_refusals;
