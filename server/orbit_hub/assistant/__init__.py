"""The assistant behind the hub, and its voice.

`backend` is the typed interface the hub's runtime drives: sessions, turns,
streamed updates, permission and clarification requests, cancellation and the
capabilities a backend declares. `speech` is the separate dictation and
read-aloud interface. `hermes` implements both over Hermes's gateway;
`openai` runs the model loop in the hub against an OpenAI-compatible API or
Anthropic's, with orbit-mcp's tools; `official_tools` uses Claude's official Python Agent SDK and Codex's app-server. `agents` is the registry of
those kinds and the hub's configured agents.
"""
