#!/usr/bin/env python3
"""The one command that changes Orbit's version, and the list of files carrying it.

The number used to be hand-written in several places at once. Raising it meant
remembering every copy, and forgetting one produced a build that named a package
the verifier then refused. Anything that can read the version at build time now
derives it; the files below cannot, because a manifest has to state a literal.

They are listed here rather than described, so the release script and the test
that forbids stray copies read the same list. server/uv.lock is among them: it
records the hub's own version, and every hub release is built with
`uv sync --locked`, which refuses a lock that disagrees. Cargo.lock is left to
cargo, as the last step. A step that fails fails the command, saying which.

Usage: python3 scripts/release/set-version.py 0.2.0
       python3 scripts/release/set-version.py --check
"""

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
NUMBER = r"\d+\.\d+\.\d+"

# path, pattern with the version as its second group, why the file needs a literal
DECLARATIONS = (
    (
        "Cargo.toml",
        r'(?m)^(version = ")(' + NUMBER + r')(")',
        "the source every derived copy reads",
    ),
    (
        "native/macos/Info.plist",
        r"(<key>CFBundleShortVersionString</key><string>)(" + NUMBER + r")(</string>)",
        "the packager overwrites this in the bundle, but a run from the tree reads it",
    ),
    (
        "server/pyproject.toml",
        r'(?m)^(version = ")(' + NUMBER + r')(")',
        "installed metadata, which is what the hub reports about itself",
    ),
    (
        "server/uv.lock",
        r'(?m)^(name = "orbit-hub"\nversion = ")(' + NUMBER + r')(")',
        "every hub release is built with uv sync --locked, which refuses a lock whose"
        " project version disagrees with pyproject.toml",
    ),
    (
        "apps/desktop/package.json",
        r'(?m)^(  "version": ")(' + NUMBER + r')(",)',
        "pnpm reads the package manifest; the app shell reports this number",
    ),
    (
        "apps/desktop/src-tauri/tauri.conf.json",
        r'(?m)^(  "version": ")(' + NUMBER + r')(",)',
        "the Tauri CLI stamps the executable's metadata from its own config",
    ),
)


def declared():
    """What each file currently states, in declaration order."""
    found = {}
    for name, pattern, _ in DECLARATIONS:
        text = (ROOT / name).read_text(encoding="utf-8")
        match = re.search(pattern, text)
        if match is None:
            raise SystemExit(f"{name} no longer declares a recognizable version")
        found[name] = match.group(2)
    return found


def current():
    """The version this repository is on."""
    return declared()[DECLARATIONS[0][0]]


def apply(version):
    """Write the version everywhere it is declared. Returns the files changed.

    Every file is read and matched before any is written, so a file that no
    longer declares a version stops the command with nothing half-changed.
    """
    pending = []
    for name, pattern, _ in DECLARATIONS:
        path = ROOT / name
        text = path.read_text(encoding="utf-8")
        if re.search(pattern, text) is None:
            raise SystemExit(f"{name} no longer declares a recognizable version; nothing was written")
        updated = re.sub(pattern, lambda m: m.group(1) + version + m.group(3), text, count=1)
        pending.append((name, path, text, updated))
    changed = []
    for name, path, text, updated in pending:
        if updated != text:
            path.write_text(updated, encoding="utf-8")
            changed.append(name)
    return changed


def update_cargo_lock(version):
    """Record the new workspace version in Cargo.lock; the build runs --locked.

    A failure here stops the command with the step, the output and what was
    already written, instead of a warning that scrolls past.
    """
    command = ["cargo", "update", "--workspace", "--offline"]
    try:
        lock = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
    except OSError as error:
        raise SystemExit(
            f"cargo could not be run ({error.strerror}): Cargo.lock still lacks {version}, "
            "although the files above already have it. Put cargo on the PATH and run "
            "cargo update --workspace --offline"
        ) from None
    if lock.returncode != 0:
        output = (lock.stdout + lock.stderr).strip()
        raise SystemExit(
            f"cargo update --workspace --offline exited with {lock.returncode}: "
            f"Cargo.lock still lacks {version}, although the files above already have it.\n{output}"
        )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version", nargs="?", help="the new version, as X.Y.Z")
    parser.add_argument(
        "--check", action="store_true", help="report disagreement without writing"
    )
    arguments = parser.parse_args()

    if arguments.check or arguments.version is None:
        found = declared()
        expected = current()
        for name, value in found.items():
            mark = "ok" if value == expected else "DIFFERS"
            print(f"{mark:9} {value:10} {name}")
        wrong = [name for name, value in found.items() if value != expected]
        return 1 if wrong else 0

    if not re.fullmatch(NUMBER, arguments.version):
        raise SystemExit("the version must have the form X.Y.Z")

    changed = apply(arguments.version)
    for name in changed:
        print(f"wrote {arguments.version} in {name}")
    if not changed:
        print(f"already at {arguments.version}")

    update_cargo_lock(arguments.version)
    print("Cargo.lock updated")
    return 0


if __name__ == "__main__":
    sys.exit(main())
