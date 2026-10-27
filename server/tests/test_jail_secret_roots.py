"""Secret discovery through the public settings and jail argument builders."""

# ruff: noqa: F811

import asyncio
import json
import logging
from contextlib import contextmanager
from dataclasses import replace
from pathlib import Path

import pytest
from test_assistant_jail import fixture_paths, kernel_root  # noqa: F401

from orbit_hub.assistant.jail import arguments, launch
from orbit_hub.config import Settings


def configured(settings, *roots):
    return replace(settings, assistant_tools={"jail": {"secret_roots": list(map(str, roots))}})


def prepared_arguments(settings, agent, work):
    async def build():
        cache = settings.assistant_jail_cache
        try:
            await cache.wait_ready()
            return arguments(settings, agent, work)
        finally:
            await cache.close()

    return asyncio.run(build())


def masks(args):
    return [
        args[i + 2]
        for i, arg in enumerate(args[:-2])
        if arg == "--ro-bind" and args[i + 1] == "/dev/null"
    ]


def test_optional_jail_table_loads_with_official_tools(environment, tmp_path):
    settings, _, _ = environment
    path = tmp_path / "hub.toml"
    path.write_text(f'''data_dir = "{settings.data_dir}"
key_file = "{settings.key_file}"
hermes_url = "https://hermes.example.test"
hermes_credential_file = "{settings.hermes_credential_file}"
[meeting_transcription]
endpoint = "https://speech.example.test"
model = "fixture"
[assistant_tools.jail]
secret_roots = ["/opt/example/stacks"]
[assistant_tools.codex]
env = ["LANG"]
''')
    loaded = Settings.load(path)
    assert set(loaded.assistant_tool_settings()) == {"codex", "claude-code"}
    assert loaded.assistant_tool_settings()["codex"].env == ("LANG",)
    assert set(settings.assistant_tool_settings()) == {"codex", "claude-code"}


@pytest.mark.parametrize(
    "raw",
    [
        {"secret_roots": "relative"},
        {"secret_roots": ["relative"]},
        {"secret_roots": [1]},
        {"secret_roots": [""]},
        {"unknown": []},
    ],
)
def test_invalid_jail_settings_name_the_table(environment, raw):
    settings, _, _ = environment
    with pytest.raises(ValueError, match=r"\[assistant_tools.jail\].*(secret_roots|unknown)"):
        replace(settings, assistant_tools={"jail": raw})


def test_name_masks_follow_writable_binds_and_leave_neighbors_visible(
    environment, kernel_root, monkeypatch, caplog
):
    settings, agent, work, vendor, _, _, _ = fixture_paths(environment, kernel_root, monkeypatch)
    root = kernel_root / "stacks"
    root.mkdir()
    work = root / "project"
    work.mkdir()
    names = (
        ".env",
        ".env.production",
        "credentials",
        "credentials.json",
        ".npmrc",
        ".netrc",
        "service.pem",
        "service.key",
        "id_rsa",
        "id_ed25519",
        "id_ecdsa",
        "id_dsa",
        "id_ecdsa_sk",
        "id_ed25519_sk",
        "token",
        "config.toml",
        "auth.json",
        "settings.json",
        "settings.local.json",
    )
    for name in names:
        (root / name).write_text("invented secret")
    workspace_secret = work / ".env"
    workspace_secret.write_text("invented workspace secret")
    ordinary = root / "README.md"
    ordinary.write_text("public neighbor")
    (root / "id_rsa.pub").write_text("invented public key")
    for name in ("node_modules", ".git", ".venv", "venv", ".cache", "__pycache__"):
        (root / name).mkdir()
        (root / name / ".env").touch()
    (vendor / "auth.json").write_text("invented vendor credential")
    (root / "vendor-link").symlink_to(vendor, target_is_directory=True)
    settings = configured(settings, root, root, vendor, "/tmp")
    original_open = Path.open

    def forbid_secret_reads(path, *args, **kwargs):
        assert root not in path.parents, "discovery must not read file contents"
        return original_open(path, *args, **kwargs)

    monkeypatch.setattr(Path, "open", forbid_secret_reads)
    with caplog.at_level(logging.WARNING):
        args = prepared_arguments(settings, agent, work)
    found = masks(args)
    expected = {str(root / name) for name in names} | {str(workspace_secret)}
    assert expected <= set(found)
    assert len(found) == len(set(found))
    assert str(ordinary) not in found and str(root / "id_rsa.pub") not in found
    assert not any(
        f"/{name}/.env" in p
        for p in found
        for name in ("node_modules", ".git", ".venv", "venv", ".cache", "__pycache__")
    )
    assert str(vendor / "auth.json") not in found
    assert args.index(str(workspace_secret)) > args.index("--bind")
    assert args[-2:] == ["--chdir", str(work)]
    assert not caplog.records


def test_discovery_reaches_own_workspace_but_not_hidden_sibling(
    environment, kernel_root, monkeypatch
):
    settings, agent, _, _, _, other, _ = fixture_paths(environment, kernel_root, monkeypatch)
    work = other.parent / agent.id
    work.mkdir()
    (work / ".env").touch()
    (other / ".env").touch()
    args = prepared_arguments(configured(settings, other.parent), agent, work)
    assert str(work / ".env") in masks(args)
    assert str(other / ".env") not in masks(args)


def test_file_aliases_mask_the_target_without_following_directory_links(
    environment, kernel_root, monkeypatch
):
    settings, agent, work, _, _, _, _ = fixture_paths(environment, kernel_root, monkeypatch)
    root = kernel_root / "stacks"
    root.mkdir()
    external = kernel_root / "external"
    external.mkdir()
    target = external / "service-data"
    target.touch()
    (external / "config.toml").touch()
    (root / ".env").symlink_to(target)
    (root / "directory-link").symlink_to(external, target_is_directory=True)
    args = prepared_arguments(configured(settings, root), agent, work)
    assert str(target) in masks(args)
    assert str(external / "config.toml") not in masks(args)


def test_a_looping_file_symlink_does_not_abort_other_masks(environment, kernel_root, monkeypatch):
    settings, agent, work, _, _, _, _ = fixture_paths(environment, kernel_root, monkeypatch)
    root = kernel_root / "stacks"
    root.mkdir()
    looping = root / ".env.loop"
    looping.symlink_to(looping)
    (root / "config.toml").touch()
    original_resolve = Path.resolve

    def resolve(path, *args, **kwargs):
        if path == looping:
            # Python 3.11 reports a symlink loop as RuntimeError, before stat.
            raise RuntimeError("fixture symlink loop")
        return original_resolve(path, *args, **kwargs)

    monkeypatch.setattr(Path, "resolve", resolve)
    args = prepared_arguments(configured(settings, root), agent, work)
    assert str(root / "config.toml") in masks(args)
    assert str(looping) not in masks(args)


@pytest.mark.parametrize(
    "bound,value,spanish",
    [
        ("MAX_SECRET_ENTRIES", 3, "entradas"),
        ("MAX_SECRET_DEPTH", 1, "profundidad"),
    ],
)
def test_scan_bounds_keep_partial_masks_and_report_once(
    environment, kernel_root, monkeypatch, capsys, bound, value, spanish
):
    from orbit_hub.assistant import jail_secrets

    settings, agent, work, _, _, _, _ = fixture_paths(environment, kernel_root, monkeypatch)
    root = kernel_root / "stacks"
    root.mkdir()
    deep = root / "service" / "nested"
    if bound == "MAX_SECRET_ENTRIES":
        for number in range(10):
            (root / f".env.{number}").touch()
    else:
        (root / ".env").touch()
        deep.mkdir(parents=True)
        (deep / "config.toml").touch()
    monkeypatch.setattr(jail_secrets, bound, value)
    original_scandir = jail_secrets.os.scandir
    visited = []

    @contextmanager
    def observed_scandir(folder):
        def observe(entries):
            for entry in entries:
                if not isinstance(folder, int) and Path(folder).is_relative_to(root):
                    visited.append(Path(entry.path))
                yield entry

        with original_scandir(folder) as entries:
            yield observe(entries)

    monkeypatch.setattr(jail_secrets.os, "scandir", observed_scandir)
    settings = configured(settings, root, root)

    async def admit():
        try:
            return await launch(settings, agent, work)
        finally:
            await settings.assistant_jail_cache.close()

    args = asyncio.run(admit())
    assert args[-1] == "--"  # Admission still succeeds after a bounded scan.
    found = [p for p in masks(args) if p.startswith(str(root))]
    if bound == "MAX_SECRET_ENTRIES":
        assert len(found) == value
        assert len(visited) == value
    else:
        assert str(root / ".env") in found
        assert not any(path.parent == deep for path in visited)
    assert str(deep / "config.toml") not in masks(args)
    lines = capsys.readouterr().err.splitlines()
    assert len(lines) == 1
    notice = json.loads(lines[0])["message"]
    assert str(root) in notice and str(value) in notice and spanish in notice
    assert "límite" in notice and "ocult" in notice
