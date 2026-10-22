"""The `[[agents]]` keys of kind `acp`, read while the configuration loads.

Apart from the backend (`acp`) because that module imports the ACP SDK, a
quarter of a second or more: the administrator's commands (`pair-code`,
`devices`, `revoke`) load the configuration and must not pay it. Only
building an `acp` agent imports the backend (`agents.KINDS`).
"""

import os
import re
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Literal

AgentName = Literal["claude-code", "codex"]
AGENTS: tuple[AgentName, ...] = ("claude-code", "codex")

VARIABLE_NAME = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
# What Orbit itself sets in an agent's environment; `env` cannot replace it.
LOCK_DOWN_VARIABLES = ("CLAUDE_CONFIG_DIR", "CODEX_HOME", "CODEX_CONFIG", "INITIAL_AGENT_MODE")
# An entry's own keys: `id`, `name` and `kind` belong to every `[[agents]]`
# entry, and the registry checks them.
KEYS = {"agent", "command", "env", "orbit_mcp_config"}


@dataclass(frozen=True)
class AcpAgentSettings:
    """An `[[agents]]` entry's own keys for kind `acp`; the agent's `id` is the
    registry's (`agents.AgentConfiguration`)."""

    agent: AgentName
    # The adapter to start, absolute path first; the operator installs it.
    command: tuple[str, ...]
    # The translator's own TOML, given to orbit-mcp as `ORBIT_MCP_CONFIG`.
    orbit_mcp_config: Path
    # Names of variables passed from the hub's environment, such as an API key.
    env: tuple[str, ...] = ()


def parse_settings(raw: Mapping[str, Any]) -> AcpAgentSettings:
    """An `[[agents]]` entry's own keys for kind `acp` (all but `id`, `name`
    and `kind`), or a refusal naming the key and why. It reads no file: the
    command is checked by `build_backend` at startup."""
    if unknown := sorted(set(raw) - KEYS):
        raise ValueError(f"unknown keys for kind acp ({len(unknown)}): {', '.join(unknown)}")
    agent = raw.get("agent")
    if agent is None:
        raise ValueError('`agent` is missing; it must be "claude-code" or "codex"')
    if agent not in AGENTS:
        raise ValueError(f'`agent` must be "claude-code" or "codex"; got {agent!r}')
    command = raw.get("command")
    if not isinstance(command, list) or not command:
        raise ValueError("`command` must be a list of at least one string, the adapter's path first")  # fmt: skip
    if any(not isinstance(part, str) or not part for part in command):
        raise ValueError("`command` has an empty or non-string item")
    if not os.path.isabs(command[0]):
        raise ValueError(f"`command` must start with an absolute path; got {command[0]!r}")
    env = raw.get("env", [])
    if not isinstance(env, list) or any(not isinstance(name, str) for name in env):
        raise ValueError("`env` must be a list of variable names")
    for index, name in enumerate(env):
        if not VARIABLE_NAME.fullmatch(name):
            raise ValueError(f"`env` item {name!r} is not a variable name")
        if name in LOCK_DOWN_VARIABLES:
            raise ValueError(f"`env` cannot pass {name}; Orbit sets it for the agent")
        if name in env[:index]:
            raise ValueError(f"`env` names {name} twice")
    mcp_config = raw.get("orbit_mcp_config")
    if mcp_config is None:
        raise ValueError("`orbit_mcp_config` is missing: the translator's TOML path")
    if not isinstance(mcp_config, str) or not os.path.isabs(mcp_config):
        raise ValueError(f"`orbit_mcp_config` must be an absolute path; got {mcp_config!r}")
    return AcpAgentSettings(agent, tuple(command), Path(mcp_config), tuple(env))
