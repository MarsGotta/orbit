//! Control of the Mac: what the assistant asks of it (screen, selection, actions, calendar),
//! the engine that acts on the computer and the window sources it captures.

macro_rules! control_notices {
    ([$next:ident $(, $rest:ident)*] ; $($entries:tt)*) => {
        $next! { [$($rest),*] ; $($entries)*
            /// Deimos reads the cause in its reply and tells the user; only recorded here.
            /// `code` is the refusal's; `too_large` carries `bytes` and `max_bytes`.
            BridgeRefused = 98 => Silent,
            /// What Deimos asked of this Mac (`code`), said while it is served:
            /// Deimos never looks at this Mac quietly.
            BridgeServing = 99 => Done,
            BridgeReplySent = 100 => Silent,
            BridgeReplyUndelivered = 101 => Silent,
            EngineSwept = 103 => Silent,
            EngineInspected = 104 => Silent,
            /// The engine failed and Orbit starts it again: `cause` is the engine
            /// cause (`unhealthy`, `initialization_failed`), `code` the failure
            /// (`receipt_too_large` or `response_too_large`, both with `bytes` and
            /// `max_bytes`, or `engine_error`), `step` `helper_fallback` when the
            /// helper answered instead. The text is in daemon.log under the same op.
            /// An answer to what Deimos asked to see that is merely too large is not
            /// this: it is a `bridge_refused` or a helper fallback.
            EngineFailed = 105 => Silent,
            /// A process ended before Orbit stopped it: `cause` is engine/proxy,
            /// `code` exit/signal/wait_status, `status` its collected number.
            EngineExited = 106 => Fixable,
            SourceProgress = 129 => Silent,
            SourceFailed = 130 => Failed,
        }
    };
}
pub(super) use control_notices;
