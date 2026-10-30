"""One version for the repository, enforced rather than remembered.

The number was hand-written in five places. Raising it broke the build twice in
one afternoon: the packager produced 0.1.1 while the archive verifier and the CI
workflow both looked for a 0.1.0 that no longer existed. A release script alone
would not have caught that, because nothing stopped a sixth copy from appearing.

So the list of files allowed to state a version lives in
scripts/release/set-version.py, and this
refuses any other file that declares one.

Run: python3 -B -m unittest discover -s scripts/tests -v
"""

import importlib.util
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "set_version", ROOT / "scripts" / "release" / "set-version.py"
)
set_version = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(set_version)

# Files that state a version in a form a manifest reader understands.
MANIFESTS = {".toml", ".yaml", ".yml", ".plist", ".json"}
STATEMENT = re.compile(r"(?m)^\s*version\s*[:=]\s*[\"']?(\d+\.\d+\.\d+)")
PLIST_STATEMENT = re.compile(r"CFBundleShortVersionString</key><string>(\d+\.\d+\.\d+)")


class SingleSourceTests(unittest.TestCase):
    def test_every_declared_file_agrees_with_the_workspace_manifest(self):
        expected = set_version.current()
        for name, value in set_version.declared().items():
            self.assertEqual(value, expected, f"{name} declares {value}, not {expected}")

    def test_no_other_tracked_file_states_a_version_of_its_own(self):
        # A copy added later is the failure mode this exists for: it stays quiet
        # until a release, and then names a package nothing built.
        allowed = {name for name, _, _ in set_version.DECLARATIONS}
        listing = subprocess.run(
            ["git", "-C", str(ROOT), "ls-files"], capture_output=True, text=True, check=True
        )
        strays = []
        for name in listing.stdout.split():
            path = ROOT / name
            if name in allowed or path.suffix not in MANIFESTS or not path.is_file():
                continue
            if (
                name.endswith(("Cargo.lock", "pnpm-lock.yaml"))
                or name == "server/codex-release.toml"
            ):
                # Lockfiles state the resolved versions of dependencies, not
                # a version of Orbit; pnpm's is generated like Cargo's. The
                # Codex pin is a dependency too, guarded by its checksum.
                continue
            text = path.read_text(encoding="utf-8", errors="replace")
            for pattern in (STATEMENT, PLIST_STATEMENT):
                found = pattern.search(text)
                if found is not None:
                    strays.append(f"{name} declares {found.group(1)}")
        self.assertEqual(strays, [], "there are versions outside the list of set-version.py")

    def test_no_script_or_workflow_names_the_archive_by_hand(self):
        # Spelling out Orbit-X.Y.Z-macos is another copy of the version wearing a
        # different shape: it broke the build twice after the number was raised,
        # once in the workflow and once in a build script. Callers pass the
        # output directory and let verify-built-archive.sh derive the name.
        # Fixtures are exempt: one publishes a synthetic archive to prove the
        # installer accepts whatever name an origin serves, which is the opposite
        # of declaring this product's version. Documentation records what past
        # releases were actually called.
        runnable = {".sh", ".py", ".yml", ".yaml"}
        exempt = ("docs/", "scripts/tests/")
        listing = subprocess.run(
            ["git", "-C", str(ROOT), "ls-files"], capture_output=True, text=True, check=True
        )
        named = re.compile(r"Orbit-\d+\.\d+\.\d+-macos")
        strays = []
        for name in listing.stdout.split():
            path = ROOT / name
            if path.suffix not in runnable or name.startswith(exempt) or not path.is_file():
                continue
            if named.search(path.read_text(encoding="utf-8", errors="replace")):
                strays.append(name)
        self.assertEqual(strays, [], "the archive name is written by hand")


FAKE_CARGO = """#!/bin/sh
echo "cargo $*" >> "$(dirname "$0")/cargo.log"
if [ -n "$FAKE_CARGO_FAILS" ]; then
  echo "error: fake cargo without network" >&2
  exit 101
fi
exit 0
"""


def find_uv():
    found = shutil.which("uv")
    if found:
        return found
    fallback = Path.home() / ".local" / "bin" / "uv"
    return str(fallback) if fallback.is_file() else None


class SetVersionRunTests(unittest.TestCase):
    """set-version.py run whole, on a copy of the tree, with a stand-in cargo.

    The copy holds every declared file, the whole server/ (so uv can build it)
    and the script itself, whose ROOT is two directories above it. Cargo is a
    stand-in because the workspace is not copied; uv is the real one, offline.
    """

    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="orbit-set-version-"))
        self.addCleanup(shutil.rmtree, self.root, ignore_errors=True)
        shutil.copytree(
            ROOT / "server",
            self.root / "server",
            ignore=shutil.ignore_patterns(".venv", "__pycache__", ".pytest_cache", ".ruff_cache"),
        )
        names = {name for name, _, _ in set_version.DECLARATIONS}
        for name in names | {"server/uv.lock", "scripts/release/set-version.py"}:
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, target)
        self.bin = self.root / "fake-bin"
        self.bin.mkdir()
        (self.bin / "cargo").write_text(FAKE_CARGO, encoding="utf-8")
        (self.bin / "cargo").chmod(0o755)

    def set_version(self, *arguments, **env):
        environment = dict(os.environ, PATH=f"{self.bin}{os.pathsep}{os.environ.get('PATH', '')}")
        environment.update(env)
        return subprocess.run(
            [
                sys.executable,
                "-B",
                str(self.root / "scripts" / "release" / "set-version.py"),
                *arguments,
            ],
            cwd=self.root,
            env=environment,
            capture_output=True,
            text=True,
            timeout=120,
        )

    def test_the_lock_it_writes_builds_and_the_hub_reports_the_number(self):
        uv = find_uv()
        if uv is None:
            if sys.platform.startswith("linux"):
                self.fail("uv not found: the server needs it to publish the hub")
            self.skipTest("no uv on this machine")
        written = self.set_version("7.8.9")
        self.assertEqual(written.returncode, 0, written.stdout + written.stderr)
        self.assertEqual(self.set_version("--check").returncode, 0)
        env = {
            key: value
            for key, value in os.environ.items()
            if key not in ("VIRTUAL_ENV", "UV_PROJECT_ENVIRONMENT", "UV_FROZEN")
        }
        env["UV_OFFLINE"] = "1"
        built = subprocess.run(
            [uv, "sync", "--locked", "--no-dev", "--link-mode", "copy"],
            cwd=self.root / "server",
            env=env,
            capture_output=True,
            text=True,
            timeout=600,
        )
        self.assertEqual(built.returncode, 0, built.stderr)
        reported = subprocess.run(
            [
                str(self.root / "server" / ".venv" / "bin" / "python"),
                "-I",
                "-c",
                "from orbit_hub.app import hub_version; print(hub_version())",
            ],
            capture_output=True,
            text=True,
            timeout=120,
        )
        self.assertEqual(reported.returncode, 0, reported.stderr)
        self.assertEqual(reported.stdout.strip(), "7.8.9")

    def test_a_failing_cargo_step_fails_the_command_and_says_so(self):
        failed = self.set_version("7.8.9", FAKE_CARGO_FAILS="1")
        self.assertNotEqual(failed.returncode, 0)
        output = failed.stdout + failed.stderr
        self.assertIn("Cargo.lock", output)
        self.assertIn("fake cargo without network", output)

    def test_a_missing_cargo_fails_the_command_and_says_so(self):
        empty = self.root / "empty-bin"
        empty.mkdir()
        failed = self.set_version("7.8.9", PATH=str(empty))
        self.assertNotEqual(failed.returncode, 0)
        output = failed.stdout + failed.stderr
        self.assertIn("cargo", output)
        self.assertNotIn("Traceback", output)

    def test_a_file_that_no_longer_declares_its_version_fails_the_command(self):
        lock = self.root / "server" / "uv.lock"
        text = lock.read_text(encoding="utf-8")
        lock.write_text(
            text.replace('name = "orbit-hub"\nversion = ', 'name = "orbit-hub"\nrelease = ', 1),
            encoding="utf-8",
        )
        failed = self.set_version("7.8.9")
        self.assertNotEqual(failed.returncode, 0)
        self.assertIn("server/uv.lock", failed.stdout + failed.stderr)


if __name__ == "__main__":
    unittest.main()
