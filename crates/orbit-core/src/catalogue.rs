//! What the code catalogues share. Every domain declares its own refusal and
//! notice codes in its own file; the wire still sees one flat list of names
//! per kind, in the order it always had.
//!
//! Each code therefore carries its place in that list, written beside it as
//! its discriminant: the list is the codes sorted by place, whatever file
//! declares them. A new code takes the next free place (`ALL.len()` of its
//! kind before it is added, which the catalogue's own test checks), so the
//! codes already published keep theirs and the generated TypeScript only
//! grows at its end.

/// `ALL` for a fieldless code enum: its codes in their wire places.
macro_rules! wire_order {
    ($type:ident { $($variant:ident),* $(,)? }) => {
        impl $type {
            const DECLARED: [Self; [$(stringify!($variant)),*].len()] = [$(Self::$variant),*];

            /// Every code, in the order the wire lists them.
            pub const ALL: &'static [Self] = &Self::sorted(Self::DECLARED);

            /// Insertion sort by place; it runs once, at compile time.
            const fn sorted<const N: usize>(mut codes: [Self; N]) -> [Self; N] {
                let mut next = 1;
                while next < N {
                    let mut at = next;
                    while at > 0 && (codes[at - 1] as u32) > (codes[at] as u32) {
                        let earlier = codes[at - 1];
                        codes[at - 1] = codes[at];
                        codes[at] = earlier;
                        at -= 1;
                    }
                    next += 1;
                }
                codes
            }
        }
    };
}
pub(crate) use wire_order;

/// The TypeScript union of a code enum's wire names, in wire order, written
/// where ts-rs writes the other generated types. ts-rs would list the codes
/// in declaration order, which is now the order of the domains.
macro_rules! ts_union {
    ($type:ident, [$($doc:literal),* $(,)?]) => {
        #[cfg(feature = "ts")]
        impl ts_rs::TS for $type {
            type WithoutGenerics = Self;
            type OptionInnerType = Self;

            fn docs() -> Option<String> {
                let lines: &[&str] = &[$($doc),*];
                (!lines.is_empty()).then(|| {
                    let body: Vec<String> = lines.iter().map(|line| format!(" *{line}")).collect();
                    format!("/**\n{}\n */\n", body.join("\n"))
                })
            }

            fn name() -> String {
                stringify!($type).to_owned()
            }

            fn inline() -> String {
                Self::ALL
                    .iter()
                    .map(|code| format!("\"{}\"", code.name()))
                    .collect::<Vec<_>>()
                    .join(" | ")
            }

            fn inline_flattened() -> String {
                Self::inline()
            }

            fn decl() -> String {
                format!("type {} = {};", stringify!($type), Self::inline())
            }

            fn decl_concrete() -> String {
                Self::decl()
            }

            fn output_path() -> Option<std::path::PathBuf> {
                Some(std::path::PathBuf::from(concat!(
                    "../../apps/desktop/src/bridge/generated/",
                    stringify!($type),
                    ".ts"
                )))
            }
        }
    };
}
pub(crate) use ts_union;
