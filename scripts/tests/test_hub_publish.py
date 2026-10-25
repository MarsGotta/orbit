"""A hub release is a folder built once, from a commit.

The production hub ran from the development checkout, so any restart loaded
whatever was in it, committed or not, and its environment was whichever one
`uv run --frozen` had last left behind. A release is the opposite: the server of
one commit, copied without .git, with its own environment built from uv.lock
with --locked, a file that says what it is, and no way to be written again.

These tests publish from a synthetic repository holding this tree's server/, with
the real uv and the real lock, offline, into a temporary base (hub_harness.py).
Publishing never talks to systemd. What they do not prove is that the production
base accepts the release: that is the operator's first publish, with its own
evidence.

Run: python3 -B -m unittest discover -s scripts/tests -p 'test_hub_*.py'
"""

import os
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))

from hub_harness import HubSandbox, find_uv, linux_only, lock_version, release_fields  # noqa: E402

DEV_ONLY = {"pytest", "pytest-asyncio", "ruff", "iniconfig", "pluggy"}


def normalized(name):
    return re.sub(r"[-_.]+", "-", name).lower()


def locked_packages(text):
    """Every package pinned in uv.lock, name to version."""
    return {
        normalized(name): version
        for name, version in re.findall(r'(?m)^name = "([^"]+)"\nversion = "([^"]+)"', text)
    }


def require_uv(test):
    if find_uv() is None:
        test.fail("uv not found (neither on the PATH nor in ~/.local/bin): publishing needs it")


@linux_only
class PublishedReleaseTests(unittest.TestCase):
    """One release, published once from the tree's committed server."""

    @classmethod
    def setUpClass(cls):
        cls.sandbox = HubSandbox()
        cls.commit = cls.sandbox.make_repo()
        cls.version = cls.sandbox.repo_version()
        cls.result = cls.sandbox.run("publish.sh", timeout=900)
        cls.release = cls.sandbox.base / "releases" / cls.version

    @classmethod
    def tearDownClass(cls):
        cls.sandbox.close()

    def setUp(self):
        require_uv(self)
        self.assertEqual(
            self.result.returncode,
            0,
            f"publishing failed:\n{self.result.stdout}\n{self.result.stderr}",
        )

    def python(self, *arguments, **options):
        return subprocess.run(
            [str(self.release / ".venv" / "bin" / "python"), "-I", *arguments],
            capture_output=True,
            text=True,
            timeout=120,
            cwd=self.sandbox.root,
            env={key: value for key, value in os.environ.items() if key != "PYTHONPATH"},
            **options,
        )

    # The release ---------------------------------------------------------

    def test_the_release_is_the_committed_server_and_nothing_else(self):
        # Compared against `git archive` of the same commit, not against the
        # working tree: the release must be what was committed.
        with tempfile.TemporaryDirectory() as scratch:
            archive = subprocess.run(
                ["git", "-C", str(self.sandbox.repo), "archive", self.commit, "server"],
                capture_output=True,
                check=True,
                env=self.sandbox.git_env(),
            ).stdout
            subprocess.run(
                ["tar", "-x", "-C", scratch, "--strip-components=1"], input=archive, check=True
            )
            expected = {
                path.relative_to(scratch): path.read_bytes()
                for path in Path(scratch).rglob("*")
                if path.is_file()
            }
        found = {}
        for path in self.release.rglob("*"):
            relative = path.relative_to(self.release)
            if ".git" in relative.parts:
                self.fail(f"the release carries {relative}")
            if relative.parts[0] in (".venv", "RELEASE", "bin") or "__pycache__" in relative.parts:
                continue
            if path.is_file():
                found[relative] = path.read_bytes()
        self.assertEqual(sorted(found), sorted(expected))
        for relative, content in expected.items():
            self.assertEqual(found[relative], content, f"{relative} differs from the commit")

    def test_the_environment_is_the_lock_without_development_tools(self):
        lock = locked_packages((self.release / "uv.lock").read_text(encoding="utf-8"))
        listing = self.python(
            "-c",
            "import importlib.metadata as m\n"
            "for d in m.distributions(): print(d.metadata['Name'], d.version)",
        )
        self.assertEqual(listing.returncode, 0, listing.stderr)
        installed = dict(line.split(" ", 1) for line in listing.stdout.splitlines())
        installed = {normalized(name): version for name, version in installed.items()}
        self.assertEqual(installed.get("orbit-hub"), self.version)
        for name, version in installed.items():
            self.assertEqual(lock.get(name), version, f"{name} {version} is not what uv.lock pins")
        self.assertEqual(sorted(DEV_ONLY & set(installed)), [], "it carries development tools")

    def test_codex_is_downloaded_from_the_pinned_asset_and_sealed(self):
        binary = self.release / "bin" / "codex"
        self.assertTrue(binary.is_file(), "publish must ship Codex beside the venv")
        result = subprocess.run([str(binary), "--version"], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.stdout.strip(), "codex-cli 0.164.0")
        fields = release_fields(self.release / "RELEASE")
        self.assertEqual(fields.get("codex"), "0.164.0")
        self.assertEqual(fields.get("codex_asset_sha256"), self.sandbox.codex_checksum)
        self.assertFalse(binary.stat().st_mode & 0o222)

    def test_the_release_says_its_version_commit_and_date(self):
        fields = release_fields(self.release / "RELEASE")
        self.assertEqual(fields.get("version"), self.version)
        self.assertEqual(fields.get("commit"), self.commit)
        self.assertRegex(fields.get("published_at", ""), r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$")
        self.assertRegex(fields.get("commit_date", ""), r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d")
        self.assertRegex(fields.get("python", ""), r"^3\.\d+\.\d+$")

    def test_nothing_in_the_release_can_be_written(self):
        writable = []
        for directory, subdirectories, files in os.walk(self.release):
            for name in [*subdirectories, *files, "."]:
                path = Path(directory) / name
                if not path.is_symlink() and path.stat().st_mode & 0o222:
                    writable.append(str(path.relative_to(self.release)))
        self.assertEqual(writable[:5], [], f"{len(writable)} entradas se pueden escribir")

    def test_the_environment_links_nothing_outside_itself_but_the_interpreter(self):
        # uv hardlinks from its cache by default, and a hardlinked file is the
        # same file as the cache's and every other venv's: making the release
        # read-only would change them too, and changing them would change it.
        shared = [
            str(path.relative_to(self.release))
            for path in self.release.rglob("*")
            if path.is_file() and not path.is_symlink() and path.stat().st_nlink > 1
        ]
        self.assertEqual(shared[:5], [], f"{len(shared)} files shared with another place")

    def test_publishing_the_same_commit_again_changes_nothing(self):
        before = (self.release / "RELEASE").read_text(encoding="utf-8")
        again = self.sandbox.run("publish.sh", self.commit, timeout=300)
        self.assertEqual(again.returncode, 0, again.stderr)
        self.assertIn("is already published", again.stdout)
        self.assertEqual((self.release / "RELEASE").read_text(encoding="utf-8"), before)

    # Its environment -----------------------------------------------------

    def test_both_entry_points_run_from_the_release_environment(self):
        interpreter = str(self.release / ".venv" / "bin" / "python")
        for name in ("orbit-hub", "orbit-mcp"):
            path = self.release / ".venv" / "bin" / name
            self.assertTrue(os.access(path, os.X_OK), f"{name} is not executable")
            head = "\n".join(path.read_text(encoding="utf-8").splitlines()[:3])
            self.assertIn(interpreter, head, f"{name} does not start with the release's Python")
        clean = {key: value for key, value in os.environ.items() if key != "PYTHONPATH"}
        hub = subprocess.run(
            [str(self.release / ".venv" / "bin" / "orbit-hub"), "--help"],
            capture_output=True, text=True, timeout=120, env=clean,
        )
        self.assertEqual(hub.returncode, 0, hub.stderr)
        self.assertIn("usage: orbit-hub", hub.stdout)
        clean["ORBIT_MCP_CONFIG"] = str(self.sandbox.root / "does-not-exist.toml")
        mcp = subprocess.run(
            [str(self.release / ".venv" / "bin" / "orbit-mcp")],
            capture_output=True, text=True, timeout=120, env=clean, stdin=subprocess.DEVNULL,
        )
        self.assertEqual(mcp.returncode, 1, mcp.stderr)
        # The same two wordings publish.sh accepts: Spanish up to 0.9.0, English after.
        self.assertRegex(mcp.stderr, "Orbit MCP (cannot start|no puede arrancar)")
        self.assertNotIn("Traceback", mcp.stderr)
        where = self.python("-c", "import orbit_hub, orbit_mcp; print(orbit_hub.__file__); print(orbit_mcp.__file__)")
        self.assertEqual(where.returncode, 0, where.stderr)
        for line in where.stdout.splitlines():
            self.assertTrue(
                Path(line).resolve().is_relative_to(self.release.resolve()),
                f"{line} is not inside the release",
            )

    # What the running hub says about itself -----------------------------

    def test_the_hub_reports_the_version_it_was_published_as(self):
        reported = self.python("-c", "from orbit_hub.app import hub_version; print(hub_version())")
        self.assertEqual(reported.returncode, 0, reported.stderr)
        lock = lock_version((self.release / "uv.lock").read_text(encoding="utf-8"))
        self.assertEqual(
            (reported.stdout.strip(), lock, release_fields(self.release / "RELEASE")["version"]),
            (self.version, self.version, self.version),
        )

    # On the copy that ships ---------------------------------------------

    def test_the_shipped_example_configuration_is_accepted(self):
        loaded = self.python(
            "-c",
            "import sys; from pathlib import Path; from orbit_hub.config import Settings;"
            " Settings.load(Path(sys.argv[1]))",
            str(self.release / "config.example.toml"),
        )
        self.assertEqual(loaded.returncode, 0, loaded.stderr)


@linux_only
class PublishRefusalTests(unittest.TestCase):
    def setUp(self):
        require_uv(self)
        self.sandbox = HubSandbox()
        self.addCleanup(self.sandbox.close)
        self.commit = self.sandbox.make_repo()
        self.version = self.sandbox.repo_version()
        self.release = self.sandbox.base / "releases" / self.version

    def publish(self, *arguments):
        return self.sandbox.run("publish.sh", *arguments, timeout=900)

    def test_uncommitted_changes_in_server_are_never_published(self):
        readme = self.sandbox.repo_file("README.md")
        readme.write_text(readme.read_text(encoding="utf-8") + "\nuncommitted\n", encoding="utf-8")
        refused = self.publish()
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("uncommitted", refused.stderr)
        self.assertIn("server/README.md", refused.stderr)
        self.assertFalse(self.release.exists())

    def test_an_untracked_file_in_server_is_an_uncommitted_change(self):
        (self.sandbox.repo_file("orbit_hub") / "half_done.py").write_text("x = 1\n", encoding="utf-8")
        refused = self.publish()
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("server/orbit_hub/half_done.py", refused.stderr)
        self.assertFalse(self.release.exists())

    def test_a_lock_that_disagrees_with_the_project_is_refused_with_both_numbers(self):
        self.sandbox.set_repo_version("9.9.9", lock=False)
        self.sandbox.commit("version without lock")
        refused = self.publish()
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("9.9.9", refused.stderr)
        self.assertIn(self.version, refused.stderr)
        self.assertIn("set-version.py", refused.stderr)
        self.assertFalse((self.sandbox.base / "releases" / "9.9.9").exists())

    def test_a_lock_uv_would_have_to_update_is_refused_and_nothing_is_left(self):
        # The same version on both sides, but a dependency the lock never saw:
        # only `uv sync --locked` can tell, and it must.
        pyproject = self.sandbox.repo_file("pyproject.toml")
        text = pyproject.read_text(encoding="utf-8")
        pyproject.write_text(text.replace('"cryptography", ', '"cryptography", "idna", ', 1), encoding="utf-8")
        self.sandbox.commit("dependency not locked")
        refused = self.publish()
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("--locked", refused.stderr)
        self.assertIn("uv sync", refused.stderr)
        self.assertFalse(self.release.exists(), "the failed publication left remains")

    def test_a_published_version_is_never_rebuilt_from_another_commit(self):
        first = self.publish()
        self.assertEqual(first.returncode, 0, first.stderr)
        readme = self.sandbox.repo_file("README.md")
        readme.write_text(readme.read_text(encoding="utf-8") + "\nanother commit\n", encoding="utf-8")
        second = self.sandbox.commit("same number, other code")
        refused = self.publish()
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn(self.commit[:12], refused.stderr)
        self.assertIn(second[:12], refused.stderr)
        self.assertIn("set-version.py", refused.stderr)
        self.assertEqual(release_fields(self.release / "RELEASE")["commit"], self.commit)
        self.assertNotIn("another commit", (self.release / "README.md").read_text(encoding="utf-8"))

    def test_a_leftover_of_an_interrupted_publish_is_named_and_left_alone(self):
        self.release.mkdir(parents=True)
        (self.release / "half-done").write_text("", encoding="utf-8")
        refused = self.publish()
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn(str(self.release), refused.stderr)
        self.assertIn("incomplete", refused.stderr)
        self.assertTrue((self.release / "half-done").exists())

    def test_an_unknown_commit_is_named(self):
        refused = self.publish("no-such-commit")
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("no-such-commit", refused.stderr)
        self.assertFalse((self.sandbox.base / "releases").exists() and any((self.sandbox.base / "releases").iterdir()))


if __name__ == "__main__":
    unittest.main()
