# Interface language

Settings › General offers System (the default), Spanish and English. The first
supported entry in `navigator.languages` chooses Spanish or English; if none is
supported, English wins. Explicit choices override that language. Spanish uses
`es-ES`. English uses `en-GB` when the first English system preference specifies
Britain, otherwise `en-US`. A Spanish Mac with no stored preference stays Spanish.

## Ownership and propagation

The daemon owns `Settings.language`. `Paths::load_settings` reads it from
`presentation.toml` beside `config.toml`; `save_settings` writes it atomically
with mode 0600. Keeping it separate lets older app versions continue loading
`config.toml`, whose parser rejects unknown keys. Existing data needs no
migration. This choice is not sent to the hub and never changes a meeting's
language, transcript, note or summary.

The `language_set` command stores the choice before acknowledging it. The
status topic carries the optional `language` field. App-core projects it into
`StatusSettingsView` and the composed settings snapshot; its tests generate
the TypeScript types and command vocabulary. Missing fields from an older
daemon mean System. The hub protocol does not change.

Both the main window and floating panel mount `CopyProvider` from their status
frames. The provider resolves language and locale and supplies the assistant's
name. Nested assistant views inherit language while naming their own assistant.
Settings applies an acknowledged choice without waiting for the next status
frame. Refused saves leave the choice unchanged. Later daemon choices replace
that temporary selection. `languagechange` updates System and English regions.

React code reads `useCopy()`. Non-component code reads `getCopy()` at the point
of use; it must not cache translated strings at module initialization. Locale
formatters are lazy for the same reason. Existing notices retain their original
codes in a weak association and reword without reporting or executing actions
again. CodeMirror updates copy through compartments and a facet, preserving
selection, drafts and undo history.

The pure Rust `LanguageChoice::resolve` accepts an ordered list of preferred
language tags, with the same supported-language policy. Future native consumers
can use it after obtaining macOS's preferences; this wave does not wire tray,
notifications, helper output or speech recognition to it.

## Catalogue contract

`es` remains the source of the catalogue's key structure. `Catalogue` widens
copy strings while preserving protocol values and function arguments. English
is composed by topic and checked by `defineCatalogue`, including extra keys
introduced through spreads. `catalogue.type-test.ts` exercises missing screens,
missing leaf keys and extra nested keys as expected compiler errors. No runtime
translation library is used. Plurals, values and the assistant's name remain
functions; language self-names are intentionally shared.

## Acceptance and remaining work

Resolver tests cover Spanish, British English, unsupported languages, preference
order and explicit overrides. A real daemon socket test changes the language,
observes the status topic and restarts the daemon to read the saved choice.
Vitest covers live Settings changes, failed saves, later daemon changes, system
language changes, retained notices, locale formatting and the five main screens.
The screen checks reject Spanish catalogue literals that differ from English;
meeting content and the remaining inline date labels are allowed.

The real-test harness writes `language = "es"` into the isolated test app's
`presentation.toml` after clearing its data and before launching it. Its dry-run
test checks the file and permissions. Only a consented real Mac run can confirm
WebKit's system preferences, both windows changing in the packaged app, and the
Spanish accessibility labels used by the native harness. Linux tests establish
neither native permissions nor audio or speech acceptance.

Inline UI strings, native menus and notifications, helper/wake recognition,
daemon/hub messages, and existing logs/comments remain later work. English
catalogue hints use “Hey {name}”; speech recognition itself is unchanged.

## Inline copy inventory

Run `node apps/desktop/scripts/count-inline-copy.mjs`. Counts are occurrences,
not unique messages, in production feature TypeScript/TSX. The AST excludes
comments, imports, type declarations, fixture/test files, property keys,
comparison/case tokens and structural JSX attributes. It counts readable JSX
text, copy-like properties/attributes and prose-like literals/templates.
These are review candidates: technical editor labels and internal prose can
still need classification. These counts use a narrower method than the earlier broad estimate and are
not directly comparable. The inventory does not migrate any inline copy.

| Feature folder | Inline copy candidates |
|---|---:|
| assistant | 7 |
| briefing | 2 |
| calendar | 2 |
| call-panel | 0 |
| editor | 17 |
| meetings | 0 |
| notes | 6 |
| onboarding | 4 |
| settings | 27 |
| vault | 164 |
| **Total** | **229** |
