# Assistant sign-in refusals

Checked against Codex 0.164.0 and Claude Code 2.1.310, without signing in,
running a model or using a real account. The hub keeps the vendor's
subscription login commands and the existing attempt response. `cause` is an
optional string, so older Macs can still decode a new `plan_required` cause;
their existing unknown-cause copy remains the fallback.

## Vendor evidence

- Codex's [0.164.0 error definition](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/protocol/src/error.rs)
  defines `UsageNotIncluded`: “To use Codex with your ChatGPT plan, upgrade to
  Plus”. This is a usage error, not a guaranteed login refusal. The
  [versioned README](https://github.com/openai/codex/blob/rust-v0.164.0/README.md)
  recommends ChatGPT Plus, Pro, Business, Edu or Enterprise for subscription
  access; Orbit's Spanish recovery copy names those plans.
- Codex's [device login](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/login/src/device_code_auth.rs)
  drops unsuccessful response bodies and reports HTTP status. While polling,
  403 and 404 mean keep waiting. The
  [account processor](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/app-server/src/request_processors/account_processor.rs)
  forwards the login error string in `account/login/completed`; `account/read`
  reports account and plan facts, not a separate plan-required login code.
  A free plan label or a generic 403 therefore does not prove a plan refusal.
- The official [Claude Code 2.1.310 npm package](https://registry.npmjs.org/@anthropic-ai/claude-code/-/claude-code-2.1.310.tgz)
  points to the platform packages. Inspection of the
  [Linux binary](https://registry.npmjs.org/@anthropic-ai/claude-code-linux-x64/-/claude-code-linux-x64-2.1.310.tgz)
  finds the subscription choice “Pro, Max, Team, or Enterprise” and the
  explicit requirement “Claude subscription required.” The latter belongs to
  setup-token guidance, not a dedicated `auth login` refusal branch.
  `auth login --claudeai` prints `Login failed:` followed by the OAuth error.
  [Authentication documentation](https://code.claude.com/docs/en/authentication)
  confirms these subscription account types. Console/API billing is a separate
  login method, which Orbit does not select for subscription sign-in.
- The repository's locked Python SDK 0.2.174 bundles the same Claude binary:
  both files have SHA-256
  `b73676430af12f97316a798b8371f81d640f90a58bdcb8f3ce56a835bb47df56`.

Neither pinned client establishes a universal server-side OAuth rejection
message for an ineligible account. Orbit recognizes only explicit requirement
phrases, without reclassifying disabled login, expired/rejected codes, quota
exhaustion or transport failures. The synthetic CLI tests inject those phrases
through the real HTTP sign-in routes; they establish classification and cleanup,
not the response a live account receives. Confirm the hosted OAuth behavior
with isolated consented accounts before claiming real sign-in acceptance.
