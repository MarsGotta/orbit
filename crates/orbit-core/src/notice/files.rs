//! Files: the folders the user gives the assistant.

macro_rules! files_notices {
    ([$next:ident $(, $rest:ident)*] ; $($entries:tt)*) => {
        $next! { [$($rest),*] ; $($entries)*
            /// A folder chosen in the panel was not kept, or the panel failed:
            /// `code` is the refusal's or the helper's, with `cause`, `count`/`max`,
            /// `bytes`/`max_bytes` or the system's `status` when there are any.
            AssistantFolderNotAdded = 102 => Fixable,
        }
    };
}
pub(super) use files_notices;
