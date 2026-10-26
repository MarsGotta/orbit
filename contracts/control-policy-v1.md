# Local control policy v1

There is no Orbit control switch. macOS Accessibility and Screen Recording
permissions are the user's authorization to see and use the computer, reported
by the helper as three-valued knowledge.
`orbit_core::computer_control::engine::decide` is the pure decision:

- With both granted, helper connected and privacy rearmed, start only the bundled
  engine. No command can select another executable or socket. Pairing and a hub
  are not required.
- Stop it, naming the cause, when privacy stops or either permission is withdrawn.
- Unknown permissions or an absent helper neither start nor stop it. Each has
  its code (`computer_permission_unknown`, `helper_not_connected`).

Authorization is not per application and does not expire. There is no shortlist,
frictionless mode or per-action confirmation.

Private `computer-control.json` retains the newest rows within both 256 entries
and 256 KiB: operation ID, requested tool, application, channel, optional
hub-authenticated agent (`id`, `name`), time and outcome (`performed`,
`cancelled`, `unknown` or `failed`). It contains no arguments, typed text,
conversations or credentials.
An outcome unknown to this version reads as `unknown`. Entries written before
agent attribution remain readable with no author; no default is inferred.
The display name is the one supplied by the hub when the request arrived, so
renaming or removing an assistant does not relabel earlier actions.

Permission withdrawal/privacy stop first removes live authority, aborts the
worker and clears pending preparations. Neither path repeats input with an
uncertain receipt. Rearming is local.
