"""What the packaging script does, checked away from macOS.

scripts/macos/bundle.sh builds, signs, notarizes and archives, and only its first part
runs anywhere but a Mac. That first part is executed here: the refusals that
come before any build — the engine driver is required, executable and at the
version the daemon verifies, and notarization credentials come whole or not at
all — run against the real script with `uname` answering Darwin and every build
tool replaced by one that refuses, so a check that wrongly passed would stop at
the first tool without touching anything.

The two source/data guards retained here protect keychain password output and
permission entitlements. Signing, notarization and archive contents are checked
when the real package is built and verified on the Mac.

What the finished archive contains — the app (with the daemon inside it), the
helper, the driver, the icon and both licences, byte for byte — is
not checked here: the
archive verifier checks the real archive (`scripts/release/verify-macos-archive.py`,
tested by `test_verify_macos_archive.py`).
"""

import plistlib
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from driver_fixture import verified_driver_version

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "macos" / "bundle.sh"
INFO_PLIST = ROOT / "native" / "macos" / "Info.plist"
CUA_DRIVER_LICENSE = ROOT / "third-party" / "cua-driver" / "LICENSE.md"
EXPECTED_CUA_DRIVER_LICENSE = """MIT License

Copyright (c) 2025 Cua AI, Inc.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
"""


class BundleLayoutTests(unittest.TestCase):

    def test_the_cua_driver_license_is_the_agreed_mit_text(self):
        self.assertEqual(
            CUA_DRIVER_LICENSE.read_text(encoding="utf-8"),
            EXPECTED_CUA_DRIVER_LICENSE,
            "the versioned cua-driver licence is not the agreed MIT text",
        )

    def test_the_info_plist_describes_a_dock_app_that_explains_each_permission(self):
        info = plistlib.loads(INFO_PLIST.read_bytes())
        self.assertEqual(info.get("CFBundleExecutable"), "Orbit")
        self.assertEqual(info.get("CFBundleIdentifier"), "app.orbit.companion")
        self.assertNotIn("LSUIElement", info, "Orbit would stay hidden from the Dock")
        self.assertIs(info.get("NSHighResolutionCapable"), True)
        for key in (
            "NSCalendarsFullAccessUsageDescription",
            "NSMicrophoneUsageDescription",
            "NSAudioCaptureUsageDescription",
            "NSScreenCaptureUsageDescription",
        ):
            self.assertTrue(info.get(key), f"the usage description {key} is missing")

    def test_the_calendar_usage_text_says_orbit_reads_and_writes_events(self):
        # Orbit creates, moves, edits and deletes events when the user does
        # so from Orbit. The text macOS shows with the permission must say so,
        # and must not keep the promise it no longer makes.
        info = plistlib.loads(INFO_PLIST.read_bytes())
        text = info.get("NSCalendarsFullAccessUsageDescription", "").lower()
        for verb in ("leer", "crear", "mover", "editar", "borrar"):
            self.assertIn(verb, text, f"the Calendars permission text does not say «{verb}»")
        self.assertNotIn("no creará", text, "the text still promises that Orbit does not write events")
        self.assertNotIn("que elijas", text, "there are no calendars to choose")


class ArchiveCommandTests(unittest.TestCase):
    def setUp(self):
        self.text = SCRIPT.read_text(encoding="utf-8")


    def test_the_keychain_password_is_never_printed(self):
        # Unlocking over SSH needs the password on the command line, and a build
        # log is read and pasted around: the only safe shape is reading it from a
        # file and sending the command's own output to /dev/null.
        unlock = [line for line in self.text.splitlines() if "unlock-keychain" in line]
        self.assertTrue(unlock, "the script does not unlock the signing keychain")
        self.assertIn("cat --", unlock[0], "the password is not read from a file")
        self.assertIn("/dev/null", unlock[0], "the unlock may print the password")

    def test_every_protected_resource_asked_for_is_also_entitled(self):
        # Under the hardened runtime a resource the binary does not declare is
        # refused with no dialog at all, so an Info.plist that asks for calendar
        # access while the entitlements stay silent produces exactly the failure
        # once reported as "no dialog appears". The two files must agree.
        import plistlib

        native = Path(__file__).resolve().parents[2] / "native" / "macos"
        info = plistlib.loads((native / "Info.plist").read_bytes())
        granted = plistlib.loads((native / "Orbit.entitlements").read_bytes())
        # Only the resources the hardened runtime actually gates. Screen
        # recording and accessibility are not on this list on purpose: macOS
        # gates those without an entitlement.
        gated = {
            "NSMicrophoneUsageDescription": "com.apple.security.device.audio-input",
            "NSCameraUsageDescription": "com.apple.security.device.camera",
            "NSCalendarsFullAccessUsageDescription": "com.apple.security.personal-information.calendars",
            "NSCalendarsUsageDescription": "com.apple.security.personal-information.calendars",
            "NSContactsUsageDescription": "com.apple.security.personal-information.addressbook",
            "NSRemindersFullAccessUsageDescription": "com.apple.security.personal-information.calendars",
        }
        for key, entitlement in gated.items():
            if key in info:
                self.assertTrue(
                    granted.get(entitlement) is True,
                    f"{key} asks for a resource without declaring {entitlement}: "
                    "macOS will deny it without showing a dialog",
                )


# Every tool the script calls once its refusals are behind it. Each stand-in
# refuses loudly, so a refusal that wrongly passed stops at the first build step.
BUILD_TOOLS = ("pnpm", "cargo", "swift", "codesign", "ditto", "lipo", "xcrun", "security", "spctl", "shasum")
BUILD_STARTED = 97


class PreludeTests(unittest.TestCase):
    """The script's refusals, run for real before any build could start."""

    def setUp(self):
        self.scratch = Path(tempfile.mkdtemp(prefix="orbit-bundle-"))
        self.addCleanup(shutil.rmtree, self.scratch)
        tools = self.scratch / "bin"
        tools.mkdir()
        self._executable(
            tools / "uname",
            '#!/bin/sh\nif [ "$1" = "-m" ]; then echo arm64; else echo Darwin; fi\n',
        )
        for tool in BUILD_TOOLS:
            self._executable(
                tools / tool,
                f'#!/bin/sh\necho "build tool invoked: {tool}" >&2\nexit {BUILD_STARTED}\n',
            )
        self.path = f"{tools}:/usr/bin:/bin"

    @staticmethod
    def _executable(path, body):
        path.write_text(body, encoding="utf-8")
        path.chmod(0o755)

    def driver(self, reports):
        path = self.scratch / "cua-driver"
        self._executable(path, f'#!/bin/sh\necho "{reports}"\n')
        return path

    def package(self, **environment):
        env = {
            "PATH": self.path,
            "HOME": str(self.scratch),
            "ORBIT_OUTPUT_DIR": str(self.scratch / "dist"),
        }
        env.update({key: str(value) for key, value in environment.items()})
        completed = subprocess.run(
            ["bash", str(SCRIPT)],
            cwd=self.scratch,
            env=env,
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )
        return completed.returncode, completed.stderr

    def assert_refused(self, outcome, reason):
        code, stderr = outcome
        self.assertEqual(code, 1, stderr)
        self.assertIn(reason, stderr)
        self.assertNotIn("build tool invoked", stderr)

    def test_the_engine_driver_is_required_before_anything_is_built(self):
        self.assert_refused(self.package(), "ORBIT_CUA_DRIVER_BINARY is missing")
        self.assert_refused(
            self.package(ORBIT_CUA_DRIVER_BINARY="relative/cua-driver"),
            "must be an absolute path",
        )
        idle = self.scratch / "not-executable"
        idle.write_text("", encoding="utf-8")
        self.assert_refused(
            self.package(ORBIT_CUA_DRIVER_BINARY=idle), "does not exist or is not executable"
        )

    def test_a_driver_at_another_version_is_refused_naming_both(self):
        expected = verified_driver_version()
        code, stderr = self.package(ORBIT_CUA_DRIVER_BINARY=self.driver("cua-driver 0.0.1"))
        self.assert_refused((code, stderr), f"expected '{expected}'")
        self.assertIn("cua-driver 0.0.1", stderr)


    def test_notarization_credentials_are_demanded_whole_or_not_at_all(self):
        # Half a credential set fails on Apple's side after the entire build
        # and upload, which is the most expensive possible moment to find out.
        driver = self.driver(f"cua-driver {verified_driver_version()}")
        for present in ("ORBIT_NOTARY_KEY", "ORBIT_NOTARY_KEY_ID", "ORBIT_NOTARY_ISSUER"):
            with self.subTest(present=present):
                self.assert_refused(
                    self.package(ORBIT_CUA_DRIVER_BINARY=driver, **{present: "synthetic"}),
                    "Partial notarization settings",
                )

    def test_an_ad_hoc_signature_is_never_sent_to_be_notarized(self):
        driver = self.driver(f"cua-driver {verified_driver_version()}")
        key = self.scratch / "AuthKey_synthetic.p8"
        key.write_text("synthetic key", encoding="utf-8")
        self.assert_refused(
            self.package(
                ORBIT_CUA_DRIVER_BINARY=driver,
                ORBIT_NOTARY_KEY=key,
                ORBIT_NOTARY_KEY_ID="synthetic-key",
                ORBIT_NOTARY_ISSUER="synthetic-issuer",
            ),
            "An ad-hoc signature cannot be notarized",
        )


if __name__ == "__main__":
    unittest.main()
