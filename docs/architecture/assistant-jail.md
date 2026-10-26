# Official assistants' host boundary

`assistant/jail.py::arguments` constructs one bubblewrap prefix from hub
settings, an agent and its validated working folder. Both conversation runners
and sign-in/out use it. Claude's SDK transport prefixes the CLI; Codex's RPC
transport prefixes `app-server`. Their children, including Orbit MCP, inherit
that namespace. A full-mount `/bin/true` probe precedes admission; missing bwrap
or failed namespace/mount creation refuses startup with actionable Spanish copy
and the existing `agent_adapter_missing` wire code.

For an example hub rooted at `/opt/orbit/data`, a Codex agent `aster` with
workspace `/home/example/project`, the argument groups are:

```sh
bwrap --ro-bind / / --dev /dev --proc /proc --tmpfs /tmp \
  --unshare-pid --die-with-parent --new-session --setenv TMPDIR /tmp \
  --tmpfs /opt/orbit/data \
  --dir /opt/orbit/data/agents/aster/codex-home \
  --dir /opt/orbit/data/agents/aster/bridge --remount-ro /opt/orbit/data \
  --ro-bind /dev/null /opt/orbit/secrets/storage.key \
  --ro-bind /dev/null /opt/orbit/secrets/hermes-token \
  --ro-bind /dev/null /opt/orbit/config/orbit.toml \
  --ro-bind /dev/null /opt/orbit/secrets/tls.key \
  --tmpfs /opt/orbit/data-workspaces/lyra --remount-ro /opt/orbit/data-workspaces/lyra \
  --tmpfs /run/user/1000 --remount-ro /run/user/1000 \
  --ro-bind /dev/null /run/docker.sock \
  --tmpfs /run/containerd --remount-ro /run/containerd \
  --tmpfs /var/snap/lxd/common/lxd --remount-ro /var/snap/lxd/common/lxd \
  --tmpfs /var/lib/lxd --remount-ro /var/lib/lxd \
  --tmpfs /home/example/.ssh --remount-ro /home/example/.ssh \
  --tmpfs /opt/orbit/data-workspaces --remount-ro /opt/orbit/data-workspaces \
  --ro-bind /opt/orbit/data/agents/aster/bridge /opt/orbit/data/agents/aster/bridge \
  --bind /home/example/project /home/example/project \
  --bind /opt/orbit/data/agents/aster/codex-home /opt/orbit/data/agents/aster/codex-home \
  --chdir /home/example/project -- /opt/orbit/app/bin/codex app-server
```

This example assumes the named secrets, sockets and directories exist. Actual
arguments also mask every other configured secret, workspace and existing
owner credential listed in the self-hosting guide, in deterministic order.
A directory is empty and read-only; a file or socket is covered by `/dev/null`.
Descendants of an already hidden directory need no separate mount. Mount points
for the agent's own homes are created before the data tree is sealed; bwrap
opens bind sources from the original root. If using the default workspace,
its mount point is similarly created before sealing the workspace root. Masking
that whole root also hides assistants created after launch.

The root mount limits writes; the masks limit reads and access to Unix control
sockets; the PID namespace hides the hub's processes and `/proc/*/environ`.
Only work, vendor home and private `/tmp` are writable. `/run/systemd/resolve`
remains visible for DNS. The network is shared. Session separation prevents
terminal injection; parent death terminates the jail.

## Tool classification

The bundled CLI identifies itself as Claude Code 2.1.310 (SDK 0.2.174).
The hook's local-read allowlist is explicit; every other name defaults to a
card. Glob/Grep are explicitly enabled because the Linux default preset omits
them. The classification follows the vendor's [tool reference](https://code.claude.com/docs/en/tools-reference)
and [SDK hook input contract](https://code.claude.com/docs/en/agent-sdk/python#pretoolusehookinput).
Availability of optional tools depends on CLI features and the model; the rule
applies even when a tool was not present at startup.

| Tool names (each listed name has the indicated class) | Orbit class |
| --- | --- |
| Read, Glob, Grep, LS, NotebookRead | Local read/search; no card |
| TaskOutput, TaskGet, TaskList, ListAgents, CronList, ToolSearch | Local result/metadata read; no card |
| Bash, PowerShell, Monitor, LSP | Executes code or starts a process; card |
| Edit, Write, NotebookEdit | Changes files; card |
| WebFetch, WebSearch, Artifact, SendFeedback, RemoteTrigger | Network effect; card |
| Task (legacy name), Agent | Starts a subagent, whose own tool calls follow this table; no card |
| TodoWrite, TaskCreate, TaskUpdate, TaskStop, KillShell | The session's own to-do list and tasks; no card |
| Skill | Loads executable instructions; card |
| EnterPlanMode, ExitPlanMode, EnterWorktree, ExitWorktree | Changes mode/worktree; card |
| CronCreate, CronDelete, TeamCreate, TeamDelete, SendMessage | Changes schedules/team state; card |
| ListMcpResourcesTool, ReadMcpResourceTool, ReadMcpResourceDirTool | Can contact an MCP server; card |
| Any other or future native tool | Card by default |
| AskUserQuestion | Existing clarification card |
| mcp__orbit__* | Existing Orbit access groups and system permissions |

PreToolUse applies to subagent calls too (`agent_id`/`agent_type` in the SDK
hook input); a subagent's Bash/Edit/WebFetch receives the same policy. The hook
never denies a textual path: the jail enforces the paths even after approval.

Codex 0.164.0 uses `untrusted` (`AskForApproval::UnlessTrusted`) with
`danger-full-access`. In
[`core/src/exec_policy.rs::render_decision_for_unmatched_command_for_platform`](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core/src/exec_policy.rs),
“Projects marked untrusted require approval for every command that is not
explicitly allowed by an exec policy rule.” There is no built-in safe-command
allowlist in that path. Explicit policy allow rules remain a vendor exception;
Orbit never returns reusable approvals or policy amendments.
[`core/src/safety.rs::assess_patch_safety`](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core/src/safety.rs)
returns `SafetyCheck::AskUser` for `UnlessTrusted` after rejecting empty patches.
Commands may use the shared network once approved.

## Translator identity and limits

Each assistant owns separate device/meetings HMAC keys and a translator TOML
under its read-only `bridge` mount. Verification resolves a live agent from the
key id and verifies its own key, then rejects a different claimed agent id.
An omitted id is filled from that identity. Operator-configured keys preserve
the original Hermes/default-agent behavior; they never enter a vendor jail.

This is a filesystem and process-visibility boundary, not a network firewall
or a replacement for vendor tool-approval behavior. Readable project secrets
outside the masks, an assistant's own vendor credentials and network services
remain accessible. Work executed later by the owner runs with the owner's
permissions outside the jail. Fake CLI tests establish Orbit's protocol;
real account, Mac and unsandboxed Linux acceptance remain necessary.
