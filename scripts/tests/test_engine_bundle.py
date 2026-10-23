"""Execute the bundler with inert build/signing tools; no macOS or TCC claims."""

import json
import plistlib
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from notice_fixture import answering, listings

ROOT = Path(__file__).resolve().parents[2]
ENGINE = "Contents/Helpers/Orbit Engine.app"


class EngineBundleTests(unittest.TestCase):
    def test_engine_bundle_has_its_own_identity_and_is_signed_before_orbit(self):
        for pruebas in (False, True):
            with (
                self.subTest(pruebas=pruebas),
                tempfile.TemporaryDirectory() as temporary,
            ):
                root = Path(temporary)
                for name in (
                    "scripts/macos/bundle.sh",
                    "scripts/release/lib.sh",
                    "Cargo.toml",
                    "crates/orbit-daemon/src/cua.rs",
                    "native/macos/Info.plist",
                    "third-party/cua-driver/LICENSE.md",
                    "LICENSE",
                    "scripts/release/third-party-notices.py",
                ):
                    target = root / name
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(ROOT / name, target)
                for name in (
                    "target/release/orbit-desktop",
                    "swift/OrbitNative",
                    "apps/desktop/src-tauri/icons/icon.icns",
                ):
                    target = root / name
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_bytes(b"inert fixture")
                tools = root / "bin"
                tools.mkdir()

                def executable(name, text):
                    path = tools / name
                    path.write_text(text)
                    path.chmod(0o755)

                executable(
                    "uname",
                    '#!/bin/sh\nif [ "$1" = -m ]; then echo arm64; else echo Darwin; fi\n',
                )
                executable("swift", f'#!/bin/sh\necho "{root}/swift"\n')
                for name in ("ditto", "shasum"):
                    executable(name, "#!/bin/sh\nexit 0\n")
                pnpm, cargo = listings(root / "listings")
                calls = root / "calls"
                for name in ("pnpm", "cargo"):
                    executable(
                        name,
                        f'#!/bin/sh\necho "{name} $*" >> "{calls}"\n'
                        + answering(name, pnpm, cargo),
                    )
                executable("lipo", "#!/bin/sh\necho arm64\n")
                executable(
                    "git",
                    '#!/bin/sh\nif [ "$1" = rev-parse ]; then echo aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa; fi\n',
                )
                executable(
                    "codesign",
                    f'#!{sys.executable}\nimport json, os, sys\nwith open(os.environ["SIGN_LOG"], "a") as log: log.write(json.dumps(sys.argv[1:]) + "\\n")\nif "-d" in sys.argv: print("designated => identifier fixture and certificate leaf")\n',
                )
                executable(
                    "python3",
                    f'#!{sys.executable}\nimport os, sys\nif any("smoke-native.py" in arg for arg in sys.argv): sys.exit(0)\nos.execv({sys.executable!r}, [{sys.executable!r}, *sys.argv[1:]])\n',
                )
                # Bash can resolve an absolute command as a function. This double
                # implements PlistBuddy data edits, without changing the script.
                buddy = root / "buddy.py"
                buddy.write_text("""import plistlib, sys
command, path = sys.argv[2:]
parts = command.split(" ", 2)
with open(path, "rb") as source: info = plistlib.load(source)
if parts[0] == "Set": info[parts[1][1:]] = parts[2]
else: raise ValueError(command)
with open(path, "wb") as target: plistlib.dump(info, target)
""")
                bash_env = root / "bash-env"
                bash_env.write_text(
                    f'/usr/libexec/PlistBuddy() {{ "{sys.executable}" "{buddy}" "$@"; }}\n'
                )
                version = next(
                    line.split('"')[1]
                    for line in (ROOT / "crates/orbit-daemon/src/cua.rs")
                    .read_text()
                    .splitlines()
                    if line.startswith("pub const VERIFIED_DRIVER_VERSION")
                )
                driver = root / "driver"
                driver.write_text(f'#!/bin/sh\necho "{version}"\n')
                driver.chmod(0o755)
                log = root / "sign.jsonl"
                result = subprocess.run(
                    ["bash", str(root / "scripts/macos/bundle.sh")],
                    env={
                        "PATH": f"{tools}:/usr/bin:/bin",
                        "HOME": str(root),
                        "BASH_ENV": str(bash_env),
                        "SIGN_LOG": str(log),
                        "ORBIT_CUA_DRIVER_BINARY": str(driver),
                        "ORBIT_SIGN_IDENTITY": "fixture",
                        "ORBIT_PRUEBAS": "1" if pruebas else "0",
                    },
                    capture_output=True,
                    text=True,
                    timeout=30,
                )
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                name = "Orbit Pruebas" if pruebas else "Orbit"
                bundle = root / "dist" / f"{name}.app"
                engine = bundle / ENGINE
                self.assertTrue(
                    engine.is_dir(), "the engine is still in the outer bundle"
                )
                self.assertEqual(
                    (engine / "Contents/MacOS/cua-driver").read_bytes(),
                    driver.read_bytes(),
                )
                self.assertFalse((bundle / "Contents/MacOS/cua-driver").exists())
                info = plistlib.loads((engine / "Contents/Info.plist").read_bytes())
                parent = plistlib.loads((bundle / "Contents/Info.plist").read_bytes())
                self.assertEqual(
                    info["CFBundleIdentifier"],
                    parent["CFBundleIdentifier"] + ".cua-driver",
                )
                self.assertNotEqual(info["CFBundleName"], name)
                self.assertEqual(info["CFBundleExecutable"], "cua-driver")
                self.assertEqual(info["CFBundlePackageType"], "APPL")
                self.assertEqual(info["CFBundleShortVersionString"], version)
                self.assertIs(info["LSUIElement"], True)
                commands = [json.loads(line) for line in log.read_text().splitlines()]
                signed = [args[-1] for args in commands if "--sign" in args]
                self.assertLess(signed.index(str(engine)), signed.index(str(bundle)))
                checked = [args[-1] for args in commands if "-d" in args]
                self.assertIn(str(engine), checked)
                # The notice of what others wrote ships beside the licences,
                # from the listings of this build's own install, offline.
                notice = (bundle / "Contents/Resources/third-party/NOTICES.txt").read_text()
                self.assertTrue(notice.startswith("Third-party software in Orbit\n"))
                self.assertIn("ejemplo-ui 1.0.0 · MIT", notice)
                self.assertIn("ejemplo-crate 1.0.0 · MIT", notice)
                metadata = [
                    line for line in calls.read_text().splitlines() if line.startswith("cargo ")
                ]
                feature = " --features orbit-core/pruebas" if pruebas else ""
                self.assertEqual(
                    metadata,
                    [
                        "cargo metadata --format-version 1 --locked --offline "
                        f"--filter-platform aarch64-apple-darwin{feature}"
                    ],
                )
                self.assertIn(
                    "pnpm --dir apps/desktop licenses list --prod --json",
                    calls.read_text().splitlines(),
                )
                self.assertIn(str(engine / "Contents/MacOS/cua-driver"), checked)
