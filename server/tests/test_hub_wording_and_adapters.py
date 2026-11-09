"""What is Linux's stays in its adapters, and the space refusals say "the hub".

The hub may run on a Linux server or on a Mac. Its common code (every module
of `orbit_hub` and `orbit_mcp`, comments included) never names `memfd`,
`/proc` or `bwrap`: only the two adapters do, the private audio file of the
final pass and the bubblewrap jail. The space refusals name the hub, which is
what the person knows, never "the server".
"""

import re
from pathlib import Path

import pytest

from orbit_hub.protocol import HubError
from orbit_hub.storage.space import STORAGE_FREE_FLOOR_BYTES, SpaceRepository

SERVER = Path(__file__).resolve().parents[1]
ADAPTERS = ("orbit_hub/private_audio.py", "orbit_hub/assistant/jail.py")
LINUX_ONLY = re.compile(r"memfd|/proc\b|\bbwrap\b")


def test_the_adapters_exist():
    assert [name for name in ADAPTERS if not (SERVER / name).is_file()] == []


def test_only_the_adapters_name_memfd_proc_or_bwrap():
    found = []
    for package in ("orbit_hub", "orbit_mcp"):
        for path in sorted((SERVER / package).rglob("*.py")):
            name = path.relative_to(SERVER).as_posix()
            if name in ADAPTERS:
                continue
            for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
                if LINUX_ONLY.search(line):
                    found.append(f"{name}:{number}: {line.strip()}")
    assert found == []


def says_the_hub(message):
    lowered = message.lower()
    assert "hub" in lowered, message
    assert "server" not in lowered and "servidor" not in lowered, message


@pytest.mark.parametrize("limit", ["quota", "disk"])
def test_a_write_over_the_limit_names_the_hub(tmp_path, limit):
    database = tmp_path / "hub.sqlite3"
    database.write_bytes(bytes(4096))
    if limit == "quota":
        space = SpaceRepository(database, 4096, lambda: 100 * 1024**3)
    else:
        space = SpaceRepository(database, None, lambda: STORAGE_FREE_FLOOR_BYTES)
    with pytest.raises(HubError) as refused:
        space.ensure_capacity(1024)
    assert refused.value.code == "quota_exceeded"
    assert refused.value.numbers["limit"] == limit
    says_the_hub(refused.value.message)


def test_a_full_disk_names_the_hub(tmp_path):
    space = SpaceRepository(tmp_path / "hub.sqlite3", None, lambda: 0)
    refusal = space.full()
    assert refusal.code == "storage_full"
    says_the_hub(refusal.message)
