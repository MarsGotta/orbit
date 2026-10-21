"""Synthetic toolchain commands only; these do not validate a real macOS SDK."""

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "macos" / "check-sdk.sh"

# Every allowed command is read-only. Unexpected arguments, including selection
# or installation attempts, fail and remain visible in the fixture call log.
TOOL = r"""
import json, os, sys
from pathlib import Path
command = [Path(sys.argv[0]).name, *sys.argv[1:]]
with open(os.environ["ORBIT_FIXTURE_LOG"], "a", encoding="utf-8") as output:
    output.write(json.dumps(command) + "\n")
values = {
    ("xcode-select", "--print-path"): os.environ["ORBIT_FIXTURE_DEVELOPER"],
    ("swift", "--version"): "Apple Swift version 6.0 (synthetic fixture)",
    ("xcrun", "--sdk", "macosx", "--show-sdk-path"): os.environ["ORBIT_FIXTURE_SDK"],
    ("xcrun", "--sdk", "macosx", "--show-sdk-version"): "15.0",
    ("xcrun", "--sdk", "macosx", "--find", "swiftc"): "/synthetic/swiftc",
    ("xcrun", "--sdk", "macosx", "--find", "clang"): "/synthetic/clang",
    ("xcrun", "--sdk", "macosx", "--find", "xctest"): "/synthetic/xctest",
}
if tuple(command) not in values:
    sys.exit(91)
if command == json.loads(os.environ.get("ORBIT_FIXTURE_FAILURE", "null")):
    sys.exit(37)
print(values[tuple(command)])
"""


class SDKPreflightTests(unittest.TestCase):
    def run_fixture(self, args, *, developer="clt", fail=None, sdk="existing", search=None):
        with tempfile.TemporaryDirectory(prefix="orbit-sdk-fixture-") as temporary:
            root = Path(temporary)
            tools = root / "bin"
            tools.mkdir()
            sdk_path = root / "SDK with spaces"
            sdk_path.mkdir()
            log = root / "calls.jsonl"
            for name in ("xcode-select", "swift", "xcrun", "xcodebuild"):
                executable = tools / name
                executable.write_text(f"#!{sys.executable}\n{TOOL}", encoding="utf-8")
                executable.chmod(0o700)
            environment = dict(os.environ)
            environment.update(
                PATH=f"{tools}{os.pathsep}{environment.get('PATH', '')}",
                ORBIT_FIXTURE_LOG=str(log),
                ORBIT_FIXTURE_DEVELOPER=(
                    "/Library/Developer/CommandLineTools"
                    if developer == "clt"
                    else "/Applications/Xcode.app/Contents/Developer"
                ),
                ORBIT_FIXTURE_SDK=str(sdk_path) if sdk == "existing" else sdk,
                ORBIT_FIXTURE_FAILURE=json.dumps(fail),
                # An empty search root by default: a real Xcode on the machine
                # running these tests must not change what they observe.
                ORBIT_XCODE_SEARCH_PATHS=search if search is not None else str(root / "none"),
            )
            result = subprocess.run(
                ["/bin/bash", str(SCRIPT), *args],
                env=environment,
                text=True,
                capture_output=True,
                timeout=5,
                check=False,
            )
            calls = (
                [json.loads(line) for line in log.read_text().splitlines()]
                if log.exists()
                else []
            )
            return result, calls

    def test_clt_and_xcode_probe_sdk_without_requiring_xcodebuild(self):
        for developer in ("clt", "xcode"):
            with self.subTest(developer=developer):
                result, calls = self.run_fixture(["sdk"], developer=developer)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(
                    calls,
                    [
                        ["xcode-select", "--print-path"],
                        ["swift", "--version"],
                        ["xcrun", "--sdk", "macosx", "--show-sdk-path"],
                        ["xcrun", "--sdk", "macosx", "--show-sdk-version"],
                        ["xcrun", "--sdk", "macosx", "--find", "swiftc"],
                        ["xcrun", "--sdk", "macosx", "--find", "clang"],
                    ],
                )
                self.assertIn("SDK with spaces", result.stdout)

    def test_each_sdk_probe_failure_is_terminal_without_retry(self):
        commands = [
            ["xcode-select", "--print-path"],
            ["swift", "--version"],
            ["xcrun", "--sdk", "macosx", "--show-sdk-path"],
            ["xcrun", "--sdk", "macosx", "--show-sdk-version"],
            ["xcrun", "--sdk", "macosx", "--find", "swiftc"],
            ["xcrun", "--sdk", "macosx", "--find", "clang"],
        ]
        for index, command in enumerate(commands):
            with self.subTest(command=command):
                result, calls = self.run_fixture(["sdk"], fail=command)
                self.assertEqual(result.returncode, 37)
                self.assertEqual(calls, commands[: index + 1])

    def test_missing_relative_or_empty_sdk_path_is_not_success(self):
        for sdk in ("", "relative-sdk", "/nonexistent-orbit-sdk-fixture"):
            with self.subTest(sdk=sdk):
                result, calls = self.run_fixture(["sdk"], sdk=sdk)
                self.assertEqual(result.returncode, 1)
                self.assertIn("::error::", result.stderr)
                self.assertEqual(len(calls), 3)

    def test_available_xctest_only_locates_runner(self):
        result, calls = self.run_fixture(["xctest"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [["xcrun", "--sdk", "macosx", "--find", "xctest"]])

    def test_missing_xctest_fails_without_install_selection_or_skip(self):
        command = ["xcrun", "--sdk", "macosx", "--find", "xctest"]
        result, calls = self.run_fixture(["xctest"], fail=command)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(calls, [command])
        self.assertIn("XCTest is not available", result.stderr)
        self.assertIn("The tests are not skipped", result.stderr)
        self.assertNotIn("Xcode is not installed", result.stderr)

    def installed_xcode(self, root, *, with_runner=True):
        """An Xcode that is on disk, with or without the test runner in it."""
        developer = root / "Applications" / "Xcode.app" / "Contents" / "Developer"
        (developer / "usr" / "bin").mkdir(parents=True)
        if with_runner:
            runner = developer / "usr" / "bin" / "xctest"
            runner.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            runner.chmod(0o700)
        return developer


    def test_a_missing_runner_names_the_installed_xcode_that_has_one(self):
        with tempfile.TemporaryDirectory(prefix="orbit-xcode-") as temporary:
            root = Path(temporary)
            developer = self.installed_xcode(root)
            command = ["xcrun", "--sdk", "macosx", "--find", "xctest"]
            result, _ = self.run_fixture(
                ["xctest"],
                fail=command,
                search=str(root / "Applications"),
            )
            self.assertEqual(result.returncode, 1)
            self.assertIn(str(developer), result.stderr)
            self.assertIn("DEVELOPER_DIR", result.stderr)
            self.assertNotIn("The tests are not skipped", result.stderr)

    def test_a_missing_runner_without_any_xcode_states_the_exact_requirement(self):
        with tempfile.TemporaryDirectory(prefix="orbit-xcode-") as temporary:
            root = (Path(temporary) / "Applications")
            root.mkdir()
            command = ["xcrun", "--sdk", "macosx", "--find", "xctest"]
            result, _ = self.run_fixture(["xctest"], fail=command, search=str(root))
            self.assertEqual(result.returncode, 1)
            self.assertIn("Contents/Developer/usr/bin/xctest", result.stderr)
            self.assertIn("The tests are not skipped", result.stderr)

    def test_unknown_missing_or_extra_mode_runs_no_commands(self):
        for args in ([], ["install"], ["sdk", "xctest"]):
            with self.subTest(args=args):
                result, calls = self.run_fixture(args)
                self.assertEqual(result.returncode, 64)
                self.assertEqual(calls, [])


if __name__ == "__main__":
    unittest.main()
