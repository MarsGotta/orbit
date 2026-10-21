"""Run scripts/release/install-on-mac.sh against a local origin.

It replaced the terminal's update for installing over SSH.
What is fixed here: an installer whose digest does not match the manifest is
never run and the app is not even asked to quit; a matching one runs with
--relaunch against the same origin, after the app was asked to quit.
`osascript` and `shasum` are stand-ins; nothing here proves macOS behaviour.
"""

import hashlib
import os
import shutil
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

from test_install_publish import Server

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "release" / "install-on-mac.sh"


class InstallOnMacTests(unittest.TestCase):
    def setUp(self):
        self.work = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.work, ignore_errors=True)
        self.serve = self.work / "serve"
        self.serve.mkdir()
        self.bin = self.work / "bin"
        self.bin.mkdir()
        self.calls = self.work / "calls"
        for name, body in {
            "osascript": f'#!/bin/bash\nprintf "osascript %s\\n" "$*" >> "{self.calls}"\n',
            "shasum": '#!/bin/bash\nsha256sum "${@: -1}"\n',
        }.items():
            shim = self.bin / name
            shim.write_text(body)
            shim.chmod(shim.stat().st_mode | stat.S_IEXEC)
        # The published installer, standing in: it records how it was run.
        installer = (
            "#!/bin/bash\n"
            f'printf "install.sh %s base=%s\\n" "$*" "$ORBIT_INSTALL_BASE" >> "{self.calls}"\n'
        )
        (self.serve / "install.sh").write_text(installer)
        self.digest = hashlib.sha256(installer.encode()).hexdigest()
        self.server = Server(self.serve)
        self.addCleanup(self.server.close)

    def publish_manifest(self, installer_sha256):
        (self.serve / "latest.txt").write_text(
            f"version=9.9.9\ninstaller_sha256={installer_sha256}\n"
        )

    def run_script(self):
        env = dict(os.environ)
        env["PATH"] = str(self.bin) + os.pathsep + env["PATH"]
        env["ORBIT_INSTALL_BASE"] = self.server.url
        return subprocess.run(
            ["bash", str(SCRIPT)], capture_output=True, text=True, env=env, timeout=60
        )

    def recorded(self):
        return self.calls.read_text().splitlines() if self.calls.exists() else []

    def test_a_matching_installer_runs_with_relaunch_after_the_app_is_asked_to_quit(self):
        self.publish_manifest(self.digest)
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.recorded()
        self.assertEqual(len(calls), 2, calls)
        self.assertIn('application id "app.orbit.companion"', calls[0])
        self.assertIn("quit", calls[0])
        self.assertEqual(calls[1], f"install.sh --relaunch base={self.server.url}")

    def test_an_installer_that_does_not_match_its_digest_is_never_run(self):
        self.publish_manifest("0" * 64)
        result = self.run_script()
        self.assertEqual(result.returncode, 1)
        self.assertIn("0" * 64, result.stderr)
        self.assertIn(self.digest, result.stderr)
        self.assertEqual(self.recorded(), [], "the app was quit or the installer ran")

    def test_a_manifest_without_an_installer_digest_is_refused(self):
        self.publish_manifest("")
        result = self.run_script()
        self.assertEqual(result.returncode, 1)
        self.assertIn("installer_sha256", result.stderr)
        self.assertEqual(self.recorded(), [])


if __name__ == "__main__":
    unittest.main()
