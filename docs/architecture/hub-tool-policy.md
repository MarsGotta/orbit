# Official assistant tool policy on the hub

The authoritative feature-by-feature decisions, with one reason for every
flag left on or forced off, are in
[`codex_policy.py`](../../server/orbit_hub/assistant/codex_policy.py).
There are 34 explicit true values and 158 explicit false values, including the pinned legacy aliases.
No second runtime feature list exists. The hub sends the same policy as CLI
`-c` overrides before app-server startup, and as `thread/start` and
`thread/resume` config. Startup matters because plugin synchronization can
precede the first thread.

## Evidence and upgrade review

The default snapshot in
[`codex-0.164.0-features.txt`](../../server/tests/fixtures/codex-0.164.0-features.txt)
was captured from `codex-cli 0.164.0 features list` with an empty temporary
`HOME` and `CODEX_HOME`, without sign-in or a model call. It was compared with
[`codex-rs/features/src/lib.rs` at rust-v0.164.0](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/features/src/lib.rs).
The public-session regression requires a decision for every true default.
It selects the snapshot by the manifest's version: upgrading the pin requires
a new snapshot and a decision for each new enabled feature. Tests never fetch
upstream or use the operator's account.

An isolated run of the pinned CLI with the hub overrides confirmed the
effective feature values, even with stored legacy aliases that enable apps and
hooks. Whole feature tables preserve literal dotted compatibility keys instead
of passing them through the CLI's dot-splitting override parser. Four true compatibility flags ignore false values
in upstream `Features::apply_map`: `worktrees`, `realtime_conversation`,
`tui_app_server`, and `tool_search_always_defer_mcp_tools`. They are retained
and explained in the true map rather than falsely reported as disabled.
`tool_search` itself is also a no-op compatibility flag: intrinsic MCP
discovery is always available, confined here to Orbit's MCP server. Repository
worktree APIs, TUI voice and TUI selection are not invoked by hub turns.
Other removed false flags are held false but have no active switch to disable.

The [plugin manager](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core-plugins/src/manager.rs)
returns an empty plugin load outcome before reading cached plugin bundles
when `plugins_enabled` is false. Public hub tests seed a cached fake plugin
and an old trusted parent/project configuration and verify that neither is
loaded, both at startup and at thread start/resume.

The [config loader](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/config/src/loader/mod.rs)
checks explicit directory trust before repository fallback and disables
untrusted project layers, hooks and exec policies. Invocation overrides mark
the working directory and every canonical ancestor untrusted, superseding
old trusted parent entries without rewriting the account's configuration.
An offline differential `config/read` probe against the pinned binary used
an invented git repository and a trusted parent with `.codex/config.toml`:
the control loaded `invented-project-model`, while the hub policy returned no
project model and marked that project layer disabled. No model request was made.
The `hooks` feature is false and the separate legacy `notify` command is empty.

## Official package inspection

The asset names, sizes and published SHA-256 values came from GitHub's
[expanded release asset listing](https://github.com/openai/codex/releases/expanded_assets/rust-v0.164.0).
Each candidate archive was streamed to an isolated build target folder,
SHA-256 checked, and inspected only through `tarfile.getmembers()` (names,
types and sizes). No binary or payload was extracted or executed. Both
archive downloads were then deleted.

| Archive | Relevant layout | SHA-256 |
|---|---|---|
| `codex-app-server-package-x86_64-unknown-linux-musl.tar.gz` | `bin/codex-app-server`, `bin/codex-code-mode-host`, `codex-package.json`, `codex-path/rg`, `codex-resources/bwrap` | `366ae46e0a22b62d4dadea055b3786652e0d69221e309f2b7394757c65ef6820` |
| `codex-package-x86_64-unknown-linux-musl.tar.gz` | `bin/codex`, the same host/metadata/rg/bwrap layout, plus the complete voice runtime and its licenses | `3ec55053109608d5038f7e50750f7d25f768dfd740fef071aa710e2559b63aac` |

Orbit uses the **complete CLI package**: the standalone server's
[`main.rs`](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/app-server/src/main.rs)
has no `login` subcommand, while Orbit needs both device sign-in and
`codex app-server`. The exact 43 regular-file members are pinned in
[`codex-release.toml`](../../server/codex-release.toml). Publication installs
all of them at their official relative paths beside the release's `.venv`,
including voice files that hub text turns do not use. Directories are created
as needed; archive links, duplicate names, missing or extra files are refused.
The checksum is checked before archive parsing, and the full inventory is
validated before writing an executable. Failed publication removes its
incomplete release and never restarts a service.

Upstream [install-context](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/install-context/src/lib.rs)
locates `codex-package.json`, `codex-path`, resources and the sibling
`bin/codex-code-mode-host` relative to the running `bin/codex`. The existing
jail exposes the entire sealed installation through its read-only root.
No jail exception or writable mount is added. Local synthetic archives exercise
publish success, checksum failure and a missing code-mode host, without network.

## Claude isolation

The pinned SDK emits empty `--setting-sources=` and `--strict-mcp-config`.
[Setting sources](https://code.claude.com/docs/en/agent-sdk/python#settingsource)
control user, project and local settings (including hooks and plugin enablement).
Strict MCP config excludes implicit user/project MCP servers; it does not by
itself express the account connector decision. The hub supplies only Orbit's
MCP and [no explicit plugin paths](https://code.claude.com/docs/en/agent-sdk/plugins).
Ambient environment values are blanked before applying the allowlist.

The bundled Claude Code 2.1.310 connector-loading branch was inspected: it calls
its false-value parser on `ENABLE_CLAUDEAI_MCP_SERVERS` and returns an empty
connector map for `false`, `0`, `no` or `off`. It also returns early when
`disableClaudeAiConnectors` is true. The hub forces both controls on each turn,
even if an operator environment entry tries to enable connectors. The public
fake-CLI test observes the final environment, settings, MCP config and SDK hook.
The built-in CLI help confirms `--bare` would skip OAuth and keychain auth, so
that flag is deliberately unused. Only Orbit's SDK permission hook is supplied;
user/project hook definitions do not load. Managed policy is still an
administrator boundary, requiring real-hub review.

## Provider retries and validation limits

`unbounded_connection_retries=false` restores the pinned finite retry defaults:
four HTTP attempts and five stream reconnections. The upstream
[provider implementation](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/model-provider-info/src/lib.rs)
contains those defaults. Codex and Claude turns have no hub inactivity or
total-duration deadline: a reasoning model can remain silent for minutes.
Startup and protocol request acknowledgements keep their separate timeouts.
Terminal provider errors preserve their structured reason and available HTTP
status, never raw diagnostics.

The first regression run failed on undecided defaults and Claude's empty
connector variable. The old publisher accepted a valid-checksum archive lacking
`codex-code-mode-host`; the new regression rejected that success. The
connection-error test first lost its structured cause. These cases were then
verified green through the public hub session or publication entry points,
using invented data. A public-session regression gates a fake provider's reply,
advances the hub clock by 61 seconds while the provider emits no frames, then
releases the answer. It completes normally for both vendors without a real
30-second sleep.

Only the real hub/account can confirm actual subscription sign-in, connected
account inventory, the installed package's provider/tool execution, Orbit MCP
reachability from the real jail, Mac approval cards, and host-managed policy.
The real kernel namespace test is skipped where the container forbids user
namespaces; a skip does not establish a working production jail. No production
service, account, Mac or Hermes installation was used for development.
