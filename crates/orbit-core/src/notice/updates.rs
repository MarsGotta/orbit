//! Updates: a search for a new Orbit, or its installation.

macro_rules! updates_notices {
    ([$next:ident $(, $rest:ident)*] ; $($entries:tt)*) => {
        $next! { [$($rest),*] ; $($entries)*
            /// A search for an update failed (`stage` `check`, `cause` its kind) or
            /// the last installation did not finish (`stage` `install`).
            UpdateFailed = 145 => Silent,
        }
    };
}
pub(super) use updates_notices;
