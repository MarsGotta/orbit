"""Publish only a verified version, from isolated release fixtures."""

import hashlib
import os
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from scripts.tests.test_release_build import FORBIDDEN_SECRET_FORMS

ROOT = Path(__file__).resolve().parents[2]
PUBLISH = ROOT / "scripts/release/publish-macos.sh"
PREPARE = ROOT / "scripts/release/prepare-publish.sh"
COMMIT = "a" * 40
VERSION = "0.3.14"


def sha(data):
    return hashlib.sha256(data).hexdigest()


class PublishContract(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="orbit-publish-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.source = self.root / VERSION
        self.source.mkdir()
        self.serve = self.root / "served"
        self.serve.mkdir()
        self.zip = self.source / f"Orbit-{VERSION}-macos-arm64.zip"
        self.zip.write_bytes(b"synthetic archive")
        self.zip.with_name(self.zip.name + ".sha256").write_text(
            f"{sha(self.zip.read_bytes())}  {self.zip.name}\n"
        )
        self.installer = self.source / "install.sh"
        self.installer.write_bytes(b"#!/bin/sh\nexit 0\n")
        (self.source / "install.sha256").write_text(
            f"{sha(self.installer.read_bytes())}  install.sh\n"
        )
        self.write_manifest()

    def write_manifest(self, **changes):
        fields = {
            "version": VERSION,
            "architecture": "arm64",
            "source_commit": COMMIT,
            "source_dirty": "false",
            "minimum_macos": "15.0",
            "notarized": "true",
        }
        fields.update(changes)
        (self.source / "build-manifest.txt").write_text(
            "".join(f"{key}={value}\n" for key, value in fields.items())
        )

    def publish(self, env=None):
        # umask 007 is the publishing shell's (seen in a review): files it
        # creates are unreadable to the web server unless the publisher sets
        # 0644 itself.
        return subprocess.run(
            [
                "bash",
                "-c",
                'umask 007; exec bash "$@"',
                "publish",
                str(PUBLISH),
                VERSION,
                str(self.source),
                str(self.serve),
            ],
            capture_output=True,
            text=True,
            timeout=15,
            env={**os.environ, **(env or {})},
        )

    def refusal(self, field, expected, found):
        result = self.publish()
        self.assertNotEqual(result.returncode, 0, result.stdout)
        for value in (field, expected, found):
            self.assertIn(value, result.stderr)
        self.assertFalse((self.serve / "latest.txt").exists())

    def test_exact_package_and_manifest_fields(self):
        other = self.source / "Orbit-0.3.13-macos-arm64.zip"
        other.write_bytes(b"other")
        self.refusal(other.name, VERSION, "0.3.13")
        other.unlink()
        nested = self.source / "leftover"
        nested.mkdir()
        (nested / other.name).write_bytes(b"other")
        self.refusal(other.name, VERSION, "0.3.13")
        shutil.rmtree(nested)
        self.zip.rename(self.source / "Orbit-0.3.13-macos-arm64.zip")
        self.refusal("Orbit-0.3.13", VERSION, "0.3.13")

    def test_missing_manifest(self):
        (self.source / "build-manifest.txt").unlink()
        self.refusal("build-manifest.txt", VERSION, "absent")

    def test_nested_old_package_is_not_ignored(self):
        nested = self.source / "leftover"
        nested.mkdir()
        old = nested / "Orbit-0.3.13-macos-arm64.zip"
        old.write_bytes(b"other")
        self.refusal(old.name, VERSION, "0.3.13")

    def test_each_false_manifest_claim(self):
        for field, expected, found in (
            ("version", VERSION, "0.3.13"),
            ("architecture", "arm64", "x86_64"),
            ("source_commit", "40", "short"),
            ("source_dirty", "false", "true"),
            ("notarized", "true", "false"),
        ):
            with self.subTest(field=field):
                self.write_manifest(**{field: found})
                self.refusal(field, expected, found)
        self.write_manifest()

    def test_installer_digest_is_pinned_and_republication_restores_it(self):
        good = self.installer.read_bytes()
        self.installer.write_bytes(b"#!/bin/sh\nexit 9\n")
        self.refusal("install.sh", sha(good), sha(self.installer.read_bytes()))
        self.installer.write_bytes(good)
        self.assertEqual(self.publish().returncode, 0)
        served = self.serve / "install.sh"
        served.write_bytes(b"newer checkout's installer")
        result = self.publish()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(served.read_bytes(), good)

    def test_manifest_has_both_provenance_fields(self):
        result = self.publish()
        self.assertEqual(result.returncode, 0, result.stderr)
        fields = dict(
            line.split("=", 1) for line in (self.serve / "latest.txt").read_text().splitlines()
        )
        self.assertEqual(fields["source_commit"], COMMIT)
        self.assertEqual(fields["installer_sha256"], sha(self.installer.read_bytes()))
        self.assertEqual(fields["arm64_file"], self.zip.name)
        self.assertEqual(fields["arm64_sha256"], sha(self.zip.read_bytes()))
        self.assertIn("published", fields)
        fixture = ROOT / "contracts/fixtures/release-manifest/latest-with-provenance.txt"
        normalized = (
            "\n".join(
                "published=2026-09-29T13:00:00Z" if line.startswith("published=") else line
                for line in (self.serve / "latest.txt").read_text().splitlines()
            )
            + "\n"
        )
        self.assertEqual(normalized, fixture.read_text())

    def test_failed_manifest_move_restores_served_pair(self):
        self.assertEqual(self.publish().returncode, 0)
        old_manifest = (self.serve / "latest.txt").read_bytes()
        old_installer = (self.serve / "install.sh").read_bytes()
        self.installer.write_bytes(b"#!/bin/sh\nexit 7\n")
        (self.source / "install.sha256").write_text(
            f"{sha(self.installer.read_bytes())}  install.sh\n"
        )
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        move_log = self.root / "moves"
        move = bin_dir / "mv"
        move.write_text(
            '#!/bin/sh\nprintf "%s\\n" "$2" >> "$ORBIT_TEST_MOVE_LOG"\ncase "$2" in */latest.txt) exit 1;; esac\nexec /bin/mv "$@"\n'
        )
        move.chmod(0o755)
        result = self.publish(
            {"PATH": f"{bin_dir}:{os.environ['PATH']}", "ORBIT_TEST_MOVE_LOG": str(move_log)}
        )
        self.assertNotEqual(result.returncode, 0)
        moves = move_log.read_text().splitlines()
        self.assertEqual([Path(path).name for path in moves], ["install.sh", "latest.txt"])
        self.assertEqual((self.serve / "latest.txt").read_bytes(), old_manifest)
        self.assertEqual((self.serve / "install.sh").read_bytes(), old_installer)
        for served in (self.serve / "latest.txt", self.serve / "install.sh"):
            self.assertEqual(served.stat().st_mode & 0o777, 0o644, served.name)

    def test_served_files_are_world_readable_under_umask_007(self):
        self.assertEqual(self.publish().returncode, 0)
        served = [
            self.serve / "latest.txt",
            self.serve / "install.sh",
            *sorted((self.serve / "packages").glob("*.zip")),
        ]
        for path in served:
            self.assertEqual(path.stat().st_mode & 0o777, 0o644, path.name)

    def test_failed_manifest_copy_restores_served_pair(self):
        self.assertEqual(self.publish().returncode, 0)
        old_manifest = (self.serve / "latest.txt").read_bytes()
        old_installer = (self.serve / "install.sh").read_bytes()
        self.installer.write_bytes(b"#!/bin/sh\nexit 8\n")
        (self.source / "install.sha256").write_text(
            f"{sha(self.installer.read_bytes())}  install.sh\n"
        )
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        copy = bin_dir / "cp"
        copy.write_text(
            '#!/bin/sh\ncase "$2" in */latest.txt.partial) exit 1;; esac\nexec /bin/cp "$@"\n'
        )
        copy.chmod(0o755)
        result = self.publish({"PATH": f"{bin_dir}:{os.environ['PATH']}"})
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((self.serve / "latest.txt").read_bytes(), old_manifest)
        self.assertEqual((self.serve / "install.sh").read_bytes(), old_installer)

    def test_served_installer_is_measured_before_manifest_changes(self):
        self.assertEqual(self.publish().returncode, 0)
        old_manifest = (self.serve / "latest.txt").read_bytes()
        old_installer = (self.serve / "install.sh").read_bytes()
        self.installer.write_bytes(b"#!/bin/sh\nexit 6\n")
        (self.source / "install.sha256").write_text(
            f"{sha(self.installer.read_bytes())}  install.sh\n"
        )
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        copy = bin_dir / "cp"
        copy.write_text(
            '#!/bin/sh\ncase "$2" in */install.sh.partial) printf corrupted > "$2"; exit 0;; esac\nexec /bin/cp "$@"\n'
        )
        copy.chmod(0o755)
        result = self.publish({"PATH": f"{bin_dir}:{os.environ['PATH']}"})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("served=", result.stderr)
        self.assertEqual((self.serve / "latest.txt").read_bytes(), old_manifest)
        self.assertEqual((self.serve / "install.sh").read_bytes(), old_installer)

    def test_test_marker_refuses_srv(self):
        bin_dir = self.root / "no-host-writes"
        bin_dir.mkdir()
        attempted = self.root / "host-attempt"
        mkdir = bin_dir / "mkdir"
        mkdir.write_text('#!/bin/sh\nprintf "%s\\n" "$*" > "$ORBIT_TEST_HOST_ATTEMPT"\nexit 99\n')
        mkdir.chmod(0o755)
        result = subprocess.run(
            ["bash", str(PUBLISH), VERSION, str(self.source), "/srv/orbit-fixture"],
            capture_output=True,
            text=True,
            timeout=15,
            env={
                **os.environ,
                "ORBIT_RELEASE_UNDER_TEST": "1",
                "ORBIT_TEST_HOST_ATTEMPT": str(attempted),
                "PATH": f"{bin_dir}:{os.environ['PATH']}",
            },
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("/srv is forbidden in tests", result.stderr)
        self.assertFalse(
            attempted.exists(), "the command reached the double that blocks host writes"
        )

    def test_prepare_uses_built_commit_and_generated_script(self):
        repo = self.root / "repo"
        scripts = repo / "scripts"
        release = scripts / "release"
        release.mkdir(parents=True)
        shutil.copy2(PUBLISH, release / "publish-macos.sh")
        shutil.copy2(ROOT / "scripts/release/lib.sh", release / "lib.sh")
        if PREPARE.exists():
            shutil.copy2(PREPARE, release / "prepare-publish.sh")
        (release / "install.sh").write_bytes(self.installer.read_bytes())
        (release / "verify-macos-archive.py").write_text(
            "import os, pathlib, sys\n"
            "archive = pathlib.Path(sys.argv[1])\n"
            "assert archive.is_file() and archive.name == 'Orbit-0.3.14-macos-arm64.zip'\n"
            "manifest = pathlib.Path(sys.argv[sys.argv.index('--manifest') + 1])\n"
            "fields = dict(line.split('=', 1) for line in manifest.read_text().splitlines())\n"
            "assert sys.argv[sys.argv.index('--commit') + 1] == fields['source_commit']\n"
            "assert sys.argv[sys.argv.index('--architecture') + 1] == fields['architecture']\n"
            "checksum = pathlib.Path(sys.argv[sys.argv.index('--checksum-file') + 1])\n"
            "assert checksum.is_file()\n"
            "pathlib.Path(os.environ['ORBIT_TEST_VERIFIER_CALLED']).write_text('yes')\n"
        )
        subprocess.run(["git", "init", "-q", str(repo)], check=True, timeout=10)
        subprocess.run(["git", "-C", str(repo), "add", "."], check=True, timeout=10)
        subprocess.run(
            [
                "git",
                "-C",
                str(repo),
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "built",
            ],
            check=True,
            timeout=10,
        )
        commit = subprocess.check_output(
            ["git", "-C", str(repo), "rev-parse", "HEAD"], text=True, timeout=10
        ).strip()
        old_installer = (release / "install.sh").read_bytes()
        # The checkout moves ahead of the built commit twice: a later commit
        # (HEAD) and an uncommitted edit (the working tree), each with its own
        # installer. Only the built commit's installer may be published.
        (release / "install.sh").write_bytes(b"#!/bin/sh\nexit 98\n")
        subprocess.run(
            [
                "git",
                "-C",
                str(repo),
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qam",
                "moved ahead",
            ],
            check=True,
            timeout=10,
        )
        head = subprocess.check_output(
            ["git", "-C", str(repo), "rev-parse", "HEAD"], text=True, timeout=10
        ).strip()
        self.assertNotEqual(head, commit)
        (release / "install.sh").write_bytes(b"#!/bin/sh\nexit 99\n")
        self.write_manifest(source_commit=commit)
        bin_dir = self.root / "doubles"
        bin_dir.mkdir()
        verified = self.root / "verifier-called"
        for name in ("ssh", "rsync", "codesign", "xcrun", "spctl", "security", "cc", "rustup"):
            path = bin_dir / name
            path.write_text("#!/bin/sh\nexit 99\n")
            path.chmod(0o755)
        env = {
            **os.environ,
            "ORBIT_RELEASE_DIR": str(self.root),
            "ORBIT_RELEASE_UNDER_TEST": "1",
            "ORBIT_TEST_BIN": str(bin_dir),
            "ORBIT_TEST_VERIFIER_CALLED": str(verified),
            "PATH": f"{bin_dir}:{os.environ['PATH']}",
        }
        result = subprocess.run(
            [str(release / "prepare-publish.sh"), VERSION],
            capture_output=True,
            text=True,
            env=env,
            timeout=30,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(verified.read_text(), "yes")
        self.assertEqual(self.installer.read_bytes(), old_installer)
        self.assertEqual(
            (self.source / "install.sha256").read_text().split()[0], sha(old_installer)
        )
        self.assertEqual(
            (self.source / "publish-macos.sh").read_bytes(),
            (release / "publish-macos.sh").read_bytes(),
        )
        generated = self.source / f"publish-{VERSION}.sh"
        self.assertTrue(generated.is_file())
        text = generated.read_text()
        self.assertNotIn("/srv", text)
        self.assertNotIn("/home/example/projects/orbit", text)
        for pattern, reason in FORBIDDEN_SECRET_FORMS:
            self.assertIsNone(re.search(pattern, text), reason)
        result = subprocess.run(
            ["bash", str(generated)],
            capture_output=True,
            text=True,
            timeout=15,
            env={**env, "ORBIT_SERVE_DIR": str(self.serve)},
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.serve / "install.sh").read_bytes(), old_installer)
        attempted = self.root / "generated-host-attempt"
        mkdir = bin_dir / "mkdir"
        mkdir.write_text('#!/bin/sh\nprintf "%s\\n" "$*" > "$ORBIT_TEST_HOST_ATTEMPT"\nexit 99\n')
        mkdir.chmod(0o755)
        blocked = subprocess.run(
            ["bash", str(generated)],
            capture_output=True,
            text=True,
            timeout=15,
            env={
                **env,
                "ORBIT_SERVE_DIR": "/srv/orbit-fixture",
                "ORBIT_TEST_HOST_ATTEMPT": str(attempted),
            },
        )
        self.assertNotEqual(blocked.returncode, 0)
        self.assertIn("/srv is forbidden in tests", blocked.stderr)
        self.assertFalse(attempted.exists())


if __name__ == "__main__":
    unittest.main()
