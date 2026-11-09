"""The assistant's tools of a plugin come from the plugin registry.

The access groups of `orbit_hub.assistant.access` are the hub's own five
(`screen`, `control`, `calendar`, `meetings`, `files`) followed by the group of
each registry entry that offers tools, in registry order. Notes (`vault`) is
the only such entry: its entry offers the group `notes` with its three tools,
and `access.py` names neither Notes nor its tools.

What each agent and the Mac see does not change: the same six groups in the
same order, the same tools, the same stored settings. An entry that offers
tools without the `assistant_tools` permission in its manifest is refused:
the hub enforces what the manifest declares.

`orbit-hub` reads the registry as soon as it loads its settings. A manifest
the hub refuses (one without that permission, or one that is not JSON) stops
every command, `serve` and the administrator's alike, with one line that names
the cause and exit code 1: never a traceback.
"""

import json
import os
import re
import shutil
import signal
import subprocess
import sys
from pathlib import Path

import pytest
from support_orbit_mcp import TOOL_NAMES, FakeHub, serving

from orbit_hub import plugins
from orbit_hub.assistant import access
from orbit_hub.vault import plugin as vault_plugin

SERVER = Path(__file__).resolve().parents[1]
EVERY_GROUP = dict.fromkeys(access.GROUPS, True)
NOTES_TOOLS = ("orbit_notes_search", "orbit_note_read", "orbit_notes_index")
TODAY_GROUPS = ("screen", "control", "calendar", "meetings", "files", "notes")
TODAY_TOOLS = {
    "screen": ("orbit_windows", "orbit_screenshot", "orbit_look", "orbit_selected_text"),
    "control": ("orbit_control_tools", "orbit_perform_action", "orbit_action_result"),
    "calendar": ("orbit_calendar",),
    "meetings": ("orbit_meetings", "orbit_transcript", "orbit_note"),
    "files": ("orbit_files_list", "orbit_files_search", "orbit_files_read"),
    "notes": NOTES_TOOLS,
}


def test_the_notes_entry_of_the_registry_offers_its_group():
    entries = {entry.id: entry for entry in plugins.load()}
    assert entries["vault"].assistant_tools == ("notes", NOTES_TOOLS)
    assert "assistant_tools" in entries["vault"].manifest["permissions"]


def test_access_names_no_plugin_nor_its_tools():
    source = (SERVER / "orbit_hub/assistant/access.py").read_text(encoding="utf-8")
    named = re.findall(r"(?i)\bnotes?\b|orbit_notes?_|vault", source)
    assert named == [], f"access.py still names Notes: {named}"


def test_groups_tools_and_their_order_do_not_change():
    assert access.GROUPS == TODAY_GROUPS
    assert list(access.TOOLS) == list(TODAY_GROUPS)
    assert {group: tuple(tools) for group, tools in access.TOOLS.items()} == TODAY_TOOLS
    assert dict(access.TOOL_GROUP) == {
        tool: group for group, tools in TODAY_TOOLS.items() for tool in tools
    }
    assert access.default_groups() == dict.fromkeys(TODAY_GROUPS, True)


def _access_built_from(offer) -> dict:
    """`access`'s tables in a fresh interpreter whose registry entry for Notes
    offers `offer` (a `(group, tools)` pair or None)."""
    script = f"""
import dataclasses, json
from orbit_hub import plugins
real = plugins.load
plugins.load = lambda: tuple(
    dataclasses.replace(entry, assistant_tools={offer!r}) for entry in real()
)
from orbit_hub.assistant import access
print(json.dumps({{
    "groups": list(access.GROUPS),
    "tools": {{group: list(tools) for group, tools in access.TOOLS.items()}},
    "tool_group": dict(access.TOOL_GROUP),
}}))
"""
    env = {**os.environ, "PYTHONPATH": str(SERVER)}
    done = subprocess.run(
        [sys.executable, "-c", script],
        cwd=SERVER,
        env=env,
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )
    assert done.returncode == 0, done.stderr
    return json.loads(done.stdout.strip().splitlines()[-1])


def test_the_group_of_a_plugin_follows_its_registry_entry():
    shorter = _access_built_from(("notes", ("orbit_notes_search",)))
    assert shorter["groups"] == list(TODAY_GROUPS)
    assert shorter["tools"]["notes"] == ["orbit_notes_search"]
    assert "orbit_note_read" not in shorter["tool_group"]

    none = _access_built_from(None)
    assert none["groups"] == list(TODAY_GROUPS[:-1])
    assert not set(NOTES_TOOLS) & set(none["tool_group"])


def test_an_entry_offering_tools_without_the_permission_is_refused():
    manifest = json.loads(vault_plugin.MANIFEST.read_text(encoding="utf-8"))
    manifest["permissions"] = [p for p in manifest["permissions"] if p != "assistant_tools"]
    with pytest.raises(ValueError, match="assistant_tools"):
        plugins.Plugin(manifest, vault_plugin.mount, (), assistant_tools=("notes", NOTES_TOOLS))


async def test_orbit_mcp_hides_the_notes_tools_when_the_group_is_off(tmp_path):
    hub = FakeHub(access={"default": {**EVERY_GROUP, "notes": False}})
    async with serving(tmp_path, hub) as (_client, server):
        listed = {tool.name for tool in await server.list_tools()}
        refused = await server.call_tool("orbit_note_read", {"path": "Ideas/ejemplo.md"})
    assert listed == set(TOOL_NAMES) - set(NOTES_TOOLS)
    assert refused.is_error


def _orbit_hub(root: Path, config: Path, *command: str) -> tuple[int | None, str]:
    """`orbit-hub --config <config> <command>` in a process of its own, run from the
    copy of the package in `root`: its exit code and all it printed. A command
    still running after a minute is killed, with its group, and has no code."""
    process = subprocess.Popen(
        [sys.executable, "-m", "orbit_hub.cli", "--config", str(config), *command],
        cwd=root,
        env={**os.environ, "PYTHONPATH": str(root)},
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        start_new_session=True,
    )
    try:
        printed, _ = process.communicate(timeout=60)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        printed, _ = process.communicate()
        return None, printed
    return process.returncode, printed


def _initialized_copy(tmp_path) -> tuple[Path, Path, Path]:
    """A copy of the package, an invented configuration and its initialized store.
    With the manifest as shipped, `devices` lists none and exits with 0."""
    root = tmp_path / "package"
    shutil.copytree(SERVER / "orbit_hub", root / "orbit_hub")
    config = tmp_path / "orbit.toml"
    config.write_text(
        "\n".join(
            (
                f"data_dir = {json.dumps(str(tmp_path / 'data'))}",
                f"key_file = {json.dumps(str(tmp_path / 'key'))}",
                'hermes_url = "https://hermes.example.test"',
                f"hermes_credential_file = {json.dumps(str(tmp_path / 'credential'))}",
                "allow_insecure_localhost = true",
                "[meeting_transcription]",
                'endpoint = "https://transcriber.example.test"',
                'model = "fixture-model"',
            )
        )
        + "\n",
        encoding="utf-8",
    )
    code, printed = _orbit_hub(root, config, "init")
    assert code == 0, printed
    code, printed = _orbit_hub(root, config, "devices")
    assert (code, json.loads(printed)) == (0, []), printed
    return root, config, root / vault_plugin.MANIFEST.relative_to(SERVER)


def _each_command_refuses(root: Path, config: Path, cause: str) -> list[str]:
    """What `serve` (plain HTTP on loopback, as the configuration allows) and
    `devices` did that is not one line naming `cause` with exit code 1."""
    wrong = []
    for command in (("serve", "--host", "127.0.0.1", "--port", "0"), ("devices",)):
        code, printed = _orbit_hub(root, config, *command)
        lines = printed.splitlines()
        if (
            code != 1
            or len(lines) != 1
            or not lines[0].startswith("Orbit Hub: ")
            or cause not in lines[0]
        ):
            wrong.append(f"{command[0]}: exit {code}, {len(lines)} lines: {printed[-600:]!r}")
    return wrong


def test_a_manifest_without_the_permission_stops_every_command_in_one_line(tmp_path):
    root, config, manifest_file = _initialized_copy(tmp_path)
    manifest = json.loads(manifest_file.read_text(encoding="utf-8"))
    manifest["permissions"] = [p for p in manifest["permissions"] if p != "assistant_tools"]
    manifest_file.write_text(json.dumps(manifest), encoding="utf-8")
    wrong = _each_command_refuses(root, config, "assistant_tools")
    assert not wrong, "\n".join(wrong)


def test_a_manifest_that_is_not_json_stops_every_command_in_one_line(tmp_path):
    root, config, manifest_file = _initialized_copy(tmp_path)
    manifest_file.write_text("{", encoding="utf-8")
    wrong = _each_command_refuses(root, config, manifest_file.name)
    assert not wrong, "\n".join(wrong)
