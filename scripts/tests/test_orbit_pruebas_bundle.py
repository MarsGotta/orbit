"""«Orbit Pruebas» is chosen when the bundle is built, and only then.

scripts/macos/bundle.sh runs here for real up to the copy of the first compiled binary:
`uname` answers Darwin, and swift, pnpm and cargo are replaced by doubles that
write down how they were called and succeed. What the script asked them to
build, and the bundle directory it created, are then what a Mac build would
get. Without ORBIT_PRUEBAS=1 the commands are the ones the published build
always ran; with it, the Cargo feature `orbit-core/pruebas` reaches both
compilations and the bundle has its own name. The identity inside the
binaries (bundle name, data directory, Keychain service) is fixed by
`orbit_bundle`'s own tests in crates/orbit-core.
"""

import plistlib
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from driver_fixture import verified_driver_version

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "macos" / "bundle.sh"
INFO_PLIST = ROOT / "native" / "macos" / "Info.plist"

OFFICIAL_TAURI = ["--dir", "apps/desktop", "tauri", "build", "--no-bundle"]
FEATURE = ["--features", "orbit-core/pruebas"]


class VariantBuildTests(unittest.TestCase):
    def setUp(self):
        self.scratch = Path(tempfile.mkdtemp(prefix="orbit-pruebas-bundle-"))
        self.addCleanup(shutil.rmtree, self.scratch)
        tools = self.scratch / "bin"
        tools.mkdir()
        self.calls = self.scratch / "calls"
        self._executable(
            tools / "uname",
            '#!/bin/sh\nif [ "$1" = "-m" ]; then echo arm64; else echo Darwin; fi\n',
        )
        for tool in ("swift", "pnpm", "cargo"):
            # One line per call: the tool, then each argument on its own field.
            self._executable(
                tools / tool,
                f'#!/bin/sh\nprintf "{tool}" >> "{self.calls}"\n'
                f'for a in "$@"; do printf "\\t%s" "$a" >> "{self.calls}"; done\n'
                f'printf "\\n" >> "{self.calls}"\n',
            )
        driver = self.scratch / "cua-driver"
        self._executable(driver, f'#!/bin/sh\necho "cua-driver {verified_driver_version()}"\n')
        self.environment = {
            "PATH": f"{tools}:/usr/bin:/bin",
            "HOME": str(self.scratch),
            "ORBIT_CUA_DRIVER_BINARY": str(driver),
        }

    @staticmethod
    def _executable(path, body):
        path.write_text(body, encoding="utf-8")
        path.chmod(0o755)

    def build(self, name, **extra):
        output = self.scratch / name
        environment = dict(self.environment, ORBIT_OUTPUT_DIR=str(output), **extra)
        self.calls.write_text("", encoding="utf-8")
        completed = subprocess.run(
            ["bash", str(SCRIPT)],
            cwd=self.scratch,
            env=environment,
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )
        # The first binary the doubles never produced stops the script: what
        # it asked for up to there is the whole build.
        self.assertNotEqual(completed.returncode, 0, completed.stderr)
        calls = [
            line.split("\t")
            for line in self.calls.read_text(encoding="utf-8").splitlines()
        ]
        bundles = sorted(path.name for path in output.glob("*.app"))
        return calls, bundles

    @staticmethod
    def compilation(calls, tool, start):
        found = [call[1:] for call in calls if call[0] == tool and call[1 : 1 + len(start)] == start]
        if len(found) != 1:
            raise AssertionError(f"{tool} {' '.join(start)}: {len(found)} calls in {calls}")
        return found[0]

    def test_the_official_build_runs_the_commands_it_always_ran(self):
        for name, extra in (
            ("no-variable", {}),
            ("empty-variable", {"ORBIT_PRUEBAS": ""}),
            ("zero-variable", {"ORBIT_PRUEBAS": "0"}),
        ):
            with self.subTest(name):
                calls, bundles = self.build(name, **extra)
                self.assertEqual(self.compilation(calls, "pnpm", OFFICIAL_TAURI[:3]), OFFICIAL_TAURI)
                # The daemon is inside the app; no build of its own.
                self.assertEqual([call for call in calls if call[0] == "cargo"], [])
                self.assertEqual(bundles, ["Orbit.app"])

    def test_orbit_pruebas_builds_the_app_with_the_feature_and_its_own_bundle(self):
        # The feature reaches the daemon through the app's own build: one
        # compilation, so the daemon inside cannot be the official one.
        calls, bundles = self.build("pruebas", ORBIT_PRUEBAS="1")
        self.assertEqual(self.compilation(calls, "pnpm", OFFICIAL_TAURI[:3]), OFFICIAL_TAURI + FEATURE)
        self.assertEqual([call for call in calls if call[0] == "cargo"], [])
        self.assertEqual(bundles, ["Orbit Pruebas.app"])

    def test_the_variant_identifier_is_its_own_and_keeps_orbits_prefix(self):
        official = plistlib.loads(INFO_PLIST.read_bytes())["CFBundleIdentifier"]
        self.assertEqual(official, "app.orbit.companion")
        found = re.findall(r"Set :CFBundleIdentifier ([A-Za-z0-9.]+)", SCRIPT.read_text(encoding="utf-8"))
        self.assertEqual(len(found), 1, found)
        variant = found[0]
        self.assertNotEqual(variant, official)
        # The helper's call detection ignores microphone users whose bundle
        # starts with «app.orbit» (MicrophoneActivityObserver.swift): the
        # variant must not see itself as a call.
        self.assertTrue(variant.startswith("app.orbit."), variant)


if __name__ == "__main__":
    unittest.main()
