"""The Orbit Pruebas entry point reads signing material only as private data."""

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class SigningFileTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory(prefix="orbit-signing-")
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        self.repo = self.root / "repo"
        for name in ("scripts/e2e-mac/build-test-app.sh", "scripts/release/lib.sh"):
            target = self.repo / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, target)
        self.home = self.root / "home"
        tools = self.home / ".cargo/bin"
        tools.mkdir(parents=True)
        uname = tools / "uname"
        uname.write_text("#!/bin/sh\necho Darwin\n")
        uname.chmod(0o755)
        # Stop at the build boundary, before any system tooling or installation.
        (self.repo / "scripts/macos").mkdir(parents=True)
        (self.repo / "scripts/macos/bundle.sh").write_text(
            '#!/bin/bash\nprintf "%s\\n" "$ORBIT_SIGN_IDENTITY" "${ORBIT_NOTARY_KEY-unset}" > "$ORBIT_FIXTURE_RECEIPT"\nexit 77\n'
        )
        self.receipt = self.root / "receipt"
        self.file = self.root / "signing.env"
        self.base = (
            "# private data\n\nORBIT_SIGN_IDENTITY=$(touch injected)\n"
            "ORBIT_SIGN_KEYCHAIN=synthetic\n"
            "ORBIT_SIGN_KEYCHAIN_PASSWORD_FILE=synthetic\n"
            "ORBIT_NOTARY_KEY=not-for-pruebas\n"
        )

    def run_build(self, contents=None, mode=0o600, file=None):
        self.file.write_text(self.base if contents is None else contents)
        self.file.chmod(mode)
        return subprocess.run(
            ["/bin/bash", str(self.repo / "scripts/e2e-mac/build-test-app.sh")],
            cwd=self.root,
            env={
                **os.environ,
                "HOME": str(self.home),
                "ORBIT_SIGNING_ENV": str(file or self.file),
                "ORBIT_FIXTURE_RECEIPT": str(self.receipt),
            },
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )

    def assert_refused(self, result, reason):
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn(reason, result.stderr)
        self.assertFalse(
            self.receipt.exists(), "the build ran with a refused file"
        )

    def test_unknown_key_is_refused_before_building(self):
        self.assert_refused(
            self.run_build(self.base + "UNRECOGNIZED=value\n"),
            "a key that is not accepted: UNRECOGNIZED",
        )

    def test_a_line_without_equals_is_refused_before_building(self):
        self.assert_refused(
            self.run_build(self.base + "BROKEN_LINE\n"), "without KEY=value"
        )

    def test_signing_values_are_data_and_notary_keys_do_not_reach_the_bundle(self):
        result = self.run_build()
        self.assertEqual(result.returncode, 77, result.stdout + result.stderr)
        self.assertEqual(
            self.receipt.read_text().splitlines(), ["$(touch injected)", "unset"]
        )
        self.assertFalse((self.root / "injected").exists())

    def test_group_readable_material_is_refused(self):
        self.assert_refused(self.run_build(mode=0o640), "mode 640")

    def test_symlink_and_material_inside_the_repository_are_refused(self):
        link = self.root / "link.env"
        link.symlink_to(self.file)
        self.assert_refused(self.run_build(file=link), "ORBIT_SIGNING_ENV")
        inside = self.repo / "signing.env"
        inside.write_text(self.base)
        inside.chmod(0o600)
        self.assert_refused(self.run_build(file=inside), "outside the repository")


if __name__ == "__main__":
    unittest.main()
