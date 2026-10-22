"""The assistant behind the hub, and its voice.

`backend` is the typed interface the hub's runtime drives: sessions, turns,
streamed updates, permission and clarification requests, cancellation and the
capabilities a backend declares. `speech` is the separate dictation and
read-aloud interface. `hermes` implements both over Hermes's gateway; `acp`
implements the assistant over the Agent Client Protocol, for Claude Code and Codex.
"""
