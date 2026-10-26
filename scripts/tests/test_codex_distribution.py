"""Publishing verifies local fixture bytes before extraction; no network."""

import sys
import hashlib
import tarfile
import unittest
from pathlib import Path

from hub_harness import HubSandbox, linux_only

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "server"))


@linux_only
class CodexDistributionTests(unittest.TestCase):
    def test_missing_code_mode_host_aborts_publish_even_with_matching_checksum(self):
        sandbox = HubSandbox()
        try:
            sandbox.make_repo()
            asset = sandbox.root / "codex-fixture.tar.gz"
            import tomllib
            import urllib.parse
            pin = tomllib.loads(sandbox.repo_file("codex-release.toml").read_text())
            asset = Path(urllib.parse.urlparse(pin["url"]).path)
            replacement = asset.with_name("incomplete.tar.gz")
            with tarfile.open(asset) as source, tarfile.open(replacement, "w:gz") as target:
                for member in source.getmembers():
                    if member.name != "bin/codex-code-mode-host":
                        target.addfile(member, source.extractfile(member))
            asset.write_bytes(replacement.read_bytes())
            digest = hashlib.sha256(asset.read_bytes()).hexdigest()
            manifest = sandbox.repo_file("codex-release.toml")
            manifest.write_text(manifest.read_text().replace(sandbox.codex_checksum, digest))
            sandbox.commit("synthetic incomplete package")
            result = sandbox.run("publish.sh", timeout=900)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("bin/codex-code-mode-host", result.stderr)
            self.assertFalse((sandbox.base / "releases" / sandbox.repo_version()).exists())
            self.assertEqual(sandbox.restarts(), [])
        finally:
            sandbox.close()

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
