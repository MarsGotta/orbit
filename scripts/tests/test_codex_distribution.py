"""Publishing verifies local fixture bytes before extraction; no network."""

import sys
import unittest
from pathlib import Path

from hub_harness import HubSandbox, linux_only

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "server"))


@linux_only
class CodexDistributionTests(unittest.TestCase):
    def test_checksum_mismatch_aborts_publish_and_removes_partial_release(self):
        sandbox = HubSandbox()
        try:
            sandbox.make_repo()
            manifest = sandbox.repo_file("codex-release.toml")
            manifest.write_text(manifest.read_text().replace(sandbox.codex_checksum, "0" * 64))
            sandbox.commit("synthetic checksum mismatch")
            result = sandbox.run("publish.sh", timeout=900)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("SHA-256 mismatch", result.stderr)
            self.assertIn("0" * 64, result.stderr)
            self.assertIn(sandbox.codex_checksum, result.stderr)
            self.assertFalse((sandbox.base / "releases" / sandbox.repo_version()).exists())
            self.assertEqual(sandbox.restarts(), [])
        finally:
            sandbox.close()

    def test_unsupported_platform_names_it_before_downloading(self):
        from orbit_hub.assistant.codex_distribution import install
        from unittest.mock import patch

        sandbox = HubSandbox()
        try:
            sandbox.make_repo()
            with patch("platform.machine", return_value="aarch64"), patch("urllib.request.urlopen") as download:
                with self.assertRaisesRegex(ValueError, "Linux/aarch64.*x86_64"):
                    install(sandbox.repo / "server")
                download.assert_not_called()
        finally:
            sandbox.close()
