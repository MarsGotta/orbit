"""The installer decides from a synthetic ps listing; no host census runs."""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

INSTALL = Path(__file__).resolve().parents[1] / "install.sh"


class InstallerRoutes(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        source = INSTALL.read_text()
        self.assertTrue(source.endswith('main "$@"\n'))
        self.script = Path(self.directory.name) / "functions.sh"
        self.script.write_text(source.removesuffix('main "$@"\n'))
        home = Path(self.directory.name) / "home"
        (home / "Library" / "Logs" / "Orbit").mkdir(parents=True)
        self.environment = {**os.environ, "HOME": str(home), "ORBIT_INSTALL_BASE": "https://updates.example.test"}

    def run_decision(self, code, listing):
        return subprocess.run(
            ["bash", "-c", 'source "$1"; ' + code, "test", str(self.script)],
            input=listing,
            text=True,
            capture_output=True,
            timeout=5,
            check=False,
            env=self.environment,
        )

    def test_parked_chain_is_orbit_and_renamed_chain_is_not(self):
        rows = [
            ("/Applications/Orbit.app/Contents/MacOS/Orbit", "la aplicación"),
            ("/Applications/Orbit.app.anterior-20260928/Contents/MacOS/Orbit", "la aplicación"),
            ("/Applications/Orbit.app.anterior-20260928/Contents/MacOS/OrbitNative", "su ayudante nativo"),
            ("/Applications/Orbit 2.app/Contents/MacOS/Orbit", ""),
            ("/Applications/Orbit.app.anterior-/Contents/MacOS/Orbit", ""),
            ("/Applications/Orbit.app.anterior-20260928/nested/Contents/MacOS/Orbit", ""),
            ("/Applications/Editor.app/Contents/MacOS/Orbit", ""),
            ("/Users/sample/notes/Orbit.app/Contents/MacOS/OrbitNative.backup", ""),
        ]
        for path, expected in rows:
            with self.subTest(path=path):
                result = self.run_decision('chain_names', f"501 1 {path}\n")
                self.assertEqual(result.returncode, 0, result.stderr)
                bundle = path.split("/Contents/")[0]
                self.assertEqual(result.stdout, f"{expected} ({bundle})" if expected else "")

    def test_the_updating_parent_app_is_counted_until_it_exits(self):
        # The current app launches the installer before closing its daemon.
        # Supply its parent relationship without enumerating the host.
        code = (
            'ps() { printf "%s\\n" "$$ 501 /bin/bash" '
            '"501 1 /Applications/Orbit.app/Contents/MacOS/Orbit"; }; '
            'read_chain; printf "%s" "$CHAIN"'
        )
        result = self.run_decision(code, "")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "la aplicación (/Applications/Orbit.app)")



if __name__ == "__main__":
    unittest.main()
