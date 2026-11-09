"""The older Hermes whose single question keeps a shape of its own.

Hermes 0.21.4 (revision 7b3c7aef31) asks through the same requests from server
to client as the installed Hermes, with one difference the hub must read: when
the model calls `clarify` with a lone top-level `question` (the older call its
tool still accepts), it asks `{question, choices?, multi_select?}` without
`questions`, and reads the answer as `{"answer": …}` in the request's result,
which `request.answer` carries. `clarify.lock` cannot answer it: that Hermes
locks only questions that came in `questions`. Its documented call (`questions`,
one or more) is asked as the installed Hermes asks it.

`contracts/hermes-server-requests-0.21.4-7b3c7aef31.json` is that Hermes's
contract, generated from its own models exactly as the installed Hermes's is
(`support_hermes_requests.GENERATE`), with its version and revision. The hub's
tests build its frames from that file. Regenerating needs that Hermes's
interpreter, named by ORBIT_HERMES_LEGACY_PYTHON (nothing is written beside it:
`-I -B` and a temporary `HERMES_HOME`, as for the installed Hermes):

    ORBIT_HERMES_LEGACY_PYTHON=… server/.venv/bin/python server/tests/support_hermes_legacy.py --write
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

from support_hermes_requests import ROOT, generate, run_in_hermes

VERSION, REVISION = "0.21.4", "7b3c7aef31"
LEGACY_PATH = ROOT / "contracts" / f"hermes-server-requests-{VERSION}-{REVISION}.json"
LEGACY = json.loads(LEGACY_PATH.read_text(encoding="utf-8")) if LEGACY_PATH.exists() else {}

# Where that interpreter imports Hermes from, and which release it is.
FIND_CHECKOUT = """
import importlib.metadata, json, pathlib
import tui_gateway
print(json.dumps({
    "checkout": str(pathlib.Path(tui_gateway.__file__).resolve().parents[1]),
    "version": importlib.metadata.version("hermes_agent"),
}))
"""


def legacy_hermes() -> tuple[Path | None, Path | None, str]:
    """That Hermes's interpreter and checkout, or Nones and why not."""
    named = os.environ.get("ORBIT_HERMES_LEGACY_PYTHON")
    if not named:
        return None, None, f"ORBIT_HERMES_LEGACY_PYTHON names no interpreter of Hermes {VERSION}"
    python = Path(named)
    if not python.is_file():
        return None, None, f"ORBIT_HERMES_LEGACY_PYTHON is not a file: {python}"
    found = json.loads(run_in_hermes(python, FIND_CHECKOUT))
    checkout = Path(found["checkout"])
    revision = subprocess.run(
        ["git", "-C", str(checkout), "rev-parse", "--short=10", "HEAD"],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    ).stdout.strip()
    if (found["version"], revision) != (VERSION, REVISION):
        return (
            None,
            None,
            (
                f"ORBIT_HERMES_LEGACY_PYTHON runs Hermes {found['version']} at {revision or '?'}, "
                f"not {VERSION} at {REVISION}"
            ),
        )
    return python, checkout, ""


def generate_legacy(python: Path, checkout: Path) -> str:
    """The older contract's text, from that Hermes's own models."""
    return generate(python, checkout, hermes={"version": VERSION, "revision": REVISION})


if __name__ == "__main__":
    python, checkout, why = legacy_hermes()
    if python is None:
        raise SystemExit(why)
    text = generate_legacy(python, checkout)
    if sys.argv[1:] == ["--write"]:
        LEGACY_PATH.write_text(text, encoding="utf-8")
        print(f"wrote {LEGACY_PATH}")
    else:
        sys.stdout.write(text)
