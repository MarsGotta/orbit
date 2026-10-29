"""Release decisions run only against synthetic trees and tools."""

import hashlib
import os
import re
import shlex
import shutil
import subprocess
import tempfile
import tomllib
import unittest
from pathlib import Path

from scripts.tests import test_verify_macos_archive as archive_fixture
from driver_fixture import verified_driver_version
import notice_fixture

ROOT = Path(__file__).resolve().parents[2]
RELEASE = ROOT / "scripts/release"


def committed_version(commit):
    manifest = subprocess.check_output(
        ["git", "show", f"{commit}:Cargo.toml"], cwd=ROOT, text=True, timeout=10
    )
    return tomllib.loads(manifest)["workspace"]["package"]["version"]


# What may never be written into a release script, with the reason beside each
# form. The list guards the scripts that build and publish.
FORBIDDEN_SECRET_FORMS = (
    (r"/Users/", "a Mac user path"),
    (r"\b(?:\d{1,3}\.){3}\d{1,3}\b", "an IP address"),
    (
        r"[A-Za-z0-9_./-]*\.p8\b",
        "a .p8 notarization key, with or without the AuthKey_ prefix",
    ),
    (r"Developer ID Application:", "a Developer ID identity"),
    (r"-----BEGIN .*PRIVATE KEY-----", "a private key in the clear"),
    (r"ssh-rsa ", "a public SSH key"),
    (r"\.ssh/(?:id_rsa|id_ed25519)", "a default SSH key"),
    (r"(?i:team[._?]?id)\s*[:=]\s*['\"]?[A-Z0-9]{10}\b", "an assigned Team ID"),
    (r"(?i:key[._?]?id)\s*[:=]\s*['\"]?[A-Z0-9]{10}\b", "a hand-written KEY_ID"),
    # A bare Team ID pasted on its own. The lookahead pins the run to exactly
    # ten characters and the digit keeps ordinary upper-case words out.
    (r"\b(?=[A-Z0-9]{10}\b)[A-Z0-9]*[0-9][A-Z0-9]*\b", "a loose Team ID"),
    # The Sentry address lives in a file outside the repository.
    (r"https://[0-9a-f]{32}@", "a Sentry address (DSN)"),
)

# A synthetic Sentry address: the shape of a real one, a host that resolves
# nowhere. Release tests never see the real one.
SYNTHETIC_DSN = "https://0123456789abcdef0123456789abcdef@o1.ingest.example.invalid/42"


class ReleaseBuildTests(unittest.TestCase):
    def setUp(self):
        self.scratch = Path(tempfile.mkdtemp(prefix="orbit-build-"))
        self.addCleanup(shutil.rmtree, self.scratch)
        self.bin = self.scratch / "bin"
        self.bin.mkdir()
        self.log = self.scratch / "commands"
        for tool in (
            "ssh",
            "rsync",
            "codesign",
            "xcrun",
            "spctl",
            "security",
            "cc",
            "rustup",
        ):
            self.tool(
                tool,
                '#!/bin/sh\nprintf "%s %s\\n" "'
                + tool
                + '" "$*" >> "$ORBIT_TEST_LOG"\n',
            )
        self.tool(
            "uname", '#!/bin/sh\ncase "$1" in -s) echo Darwin;; -m) echo arm64;; esac\n'
        )
        self.env = {
            **os.environ,
            "PATH": f"{self.bin}:{os.environ['PATH']}",
            "ORBIT_RELEASE_UNDER_TEST": "1",
            "ORBIT_TEST_BIN": str(self.bin),
            "ORBIT_TEST_LOG": str(self.log),
            # No test reads the machine's real Sentry address file;
            # the tests that exercise the address give their own.
            "ORBIT_SENTRY_DSN_FILE": "",
        }

    def tool(self, name, body, directory=None):
        path = (directory or self.bin) / name
        path.write_text(body)
        path.chmod(0o755)

    def fake_home(self):
        """A HOME whose cargo/local bins are the only tool directories the
        script can reach: whatever it runs must resolve through the PATH it
        exports itself, the way the host build script does on the Mac."""
        home = self.scratch / "fakehome"
        cargo = home / ".cargo" / "bin"
        local = home / ".local" / "bin"
        cargo.mkdir(parents=True, exist_ok=True)
        local.mkdir(parents=True, exist_ok=True)
        return home, cargo, local

    def git_commit(self, fixture, message):
        subprocess.run(
            ["git", "init", "-q", "-b", "main", str(fixture)], check=True, timeout=10
        )
        subprocess.run(["git", "-C", str(fixture), "add", "."], check=True, timeout=10)
        subprocess.run(
            [
                "git",
                "-C",
                str(fixture),
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                message,
            ],
            check=True,
            timeout=10,
        )
        return subprocess.check_output(
            ["git", "-C", str(fixture), "rev-parse", "HEAD"], text=True, timeout=10
        ).strip()

    def run_script(self, name, *args, env=None, cwd=None):
        # This deadline only bounds a fork or I/O stall of the script under
        # test. It does not protect against a real external tool: that is
        # release_test_tools' refusal, proved by the marker test, and it is
        # not a performance budget. A fetch-build invocation against the real
        # repository was observed stalled past 20 s with one-minute load above
        # 100 (measured 2026-09-28); healthy invocations
        # stay under 1 s. 40 s is twice that observed lower bound.
        return subprocess.run(
            ["bash", str(RELEASE / name), *map(str, args)],
            cwd=cwd or ROOT,
            env={**self.env, **(env or {})},
            text=True,
            capture_output=True,
            timeout=40,
            check=False,
        )

    def mac_build_env(self, security_body=None):
        """An environment for running mac-build.sh.

        The script exports the host build's PATH, so every double must live
        where that PATH looks — under a fake $HOME — or it disappears the
        moment the script starts.
        """
        home, cargo, local = self.fake_home()
        for tool in (
            "ssh",
            "rsync",
            "codesign",
            "xcrun",
            "spctl",
            "cc",
            "rustup",
            "cargo",
            "shasum",
        ):
            self.tool(
                tool,
                f'#!/bin/sh\necho "{tool} $*" >> "$ORBIT_TEST_LOG"\n',
                directory=cargo,
            )
        self.tool(
            "security",
            security_body or '#!/bin/sh\necho "security $*" >> "$ORBIT_TEST_LOG"\n',
            directory=cargo,
        )
        self.tool(
            "uname",
            '#!/bin/sh\ncase "$1" in -s) echo Darwin;; -m) echo arm64;; esac\n',
            directory=cargo,
        )
        # The app's symbols (the daemon runs inside the app):
        # one UUID for the bundled binary and its dSYM, unless
        # ORBIT_TEST_DSYM_UUID says the dSYM's differs.
        self.tool(
            "dwarfdump",
            '#!/bin/sh\necho "dwarfdump $*" >> "$ORBIT_TEST_LOG"\n'
            'case "$2" in *.dSYM) uuid=${ORBIT_TEST_DSYM_UUID:-11111111-2222-3333-4444-555555555555};; '
            '*) uuid=11111111-2222-3333-4444-555555555555;; esac\n'
            'echo "UUID: $uuid (arm64) $2"\n',
            directory=cargo,
        )
        self.tool("ditto", '#!/bin/sh\necho "ditto $*" >> "$ORBIT_TEST_LOG"\n', directory=cargo)
        env = {
            "HOME": str(home),
            "PATH": "/usr/bin:/bin",
            "ORBIT_RELEASE_UNDER_TEST": "1",
            "ORBIT_TEST_BIN": str(cargo),
            "ORBIT_TEST_LOG": str(self.log),
        }
        return env, home, cargo, local

    def run_mac_build(self, env, cwd=None, script=None):
        return subprocess.run(
            ["bash", str(script or RELEASE / "mac-build.sh")],
            cwd=cwd or ROOT,
            env=env,
            text=True,
            capture_output=True,
            timeout=30,
            check=False,
        )

    def test_mac_build_is_versioned_and_rejects_a_reused_directory(self):
        script = RELEASE / "mac-build.sh"
        self.assertTrue(
            script.is_file(), "the Mac's script is missing from the repository"
        )
        output = self.scratch / "already-used"
        output.mkdir()
        env, _, _, _ = self.mac_build_env()
        result = self.run_mac_build({**env, "ORBIT_BUILDDIR": str(output)})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(str(output), result.stderr)
        # The refusal says the check it performs — an absolute, not-yet-existing
        # artifacts folder — and not the 16-09 story, which belongs to the guide.
        self.assertIn("absolute path", result.stderr)
        self.assertNotIn("orbit-core", result.stderr)


    def test_mac_build_runs_every_stage_of_the_host_script_in_order(self):
        """The whole host build, stage by stage, in its order.

        Every double lives under a fake $HOME and the process gets
        PATH=/usr/bin:/bin, so each stage command can only resolve through the
        PATH the script exports, exactly as over non-interactive SSH.
        """
        env, _, cargo, local = self.mac_build_env()
        self.tool(
            "cc",
            "#!/bin/sh\n"
            'echo "cc $*" >> "$ORBIT_TEST_LOG"\n'
            "out= prev=\n"
            "for arg do\n"
            '  [ "$prev" = "-o" ] && out="$arg"\n'
            "  prev=$arg\n"
            "done\n"
            'printf \'#!/bin/sh\\necho "run-test %s" >> "$ORBIT_TEST_LOG"\\n\' "$(basename "$out")" > "$out"\n'
            'chmod +x "$out"\n',
            directory=cargo,
        )
        self.tool(
            "swift",
            '#!/bin/sh\necho "swift $*" >> "$ORBIT_TEST_LOG"\n',
            directory=local,
        )
        self.tool(
            "pnpm",
            '#!/bin/sh\necho "pnpm $*" >> "$ORBIT_TEST_LOG"\nexit 0\n',
            directory=local,
        )
        self.tool(
            "bash",
            "#!/bin/sh\n"
            'echo "bash $*" >> "$ORBIT_TEST_LOG"\n'
            'if [ "$1" = "--version" ]; then printf "GNU bash, version 5.3.9(1)-release (double)\\n"; exit 0; fi\n'
            'exec /usr/bin/bash "$@"\n',
            directory=local,
        )
        fixture = self.scratch / "stage-repo"
        (fixture / "scripts/release").mkdir(parents=True)
        for name in ("lib.sh", "mac-build.sh"):
            shutil.copy2(RELEASE / name, fixture / "scripts/release" / name)
        (fixture / "scripts/macos").mkdir()
        (fixture / "scripts/macos/check-sdk.sh").write_text(
            '#!/bin/sh\necho "check-sdk.sh $1" >> "$ORBIT_TEST_LOG"\n'
        )
        (fixture / "scripts/release/verify-built-archive.sh").write_text(
            '#!/bin/sh\necho "verify-built-archive.sh $*" >> "$ORBIT_TEST_LOG"\n'
        )
        (fixture / "scripts/macos/bundle.sh").write_text(
            "#!/bin/sh\n"
            'echo "bundle" >> "$ORBIT_TEST_LOG"\n'
            'mkdir -p "$ORBIT_OUTPUT_DIR/Orbit.app/Contents/MacOS" target/release/orbit-desktop.dSYM\n'
            'printf binary > "$ORBIT_OUTPUT_DIR/Orbit.app/Contents/MacOS/Orbit"\n'
            'printf "synthetic zip" > "$ORBIT_OUTPUT_DIR/Orbit-9.8.7-macos-arm64.zip"\n'
        )
        commit = self.git_commit(fixture, "fixture")
        keychain = self.scratch / "keychain"
        password = self.scratch / "password"
        notary = self.scratch / "notary-key"
        for path in (keychain, password, notary):
            path.write_text("private")
        secret = self.scratch / "signing.env"
        secret.write_text(
            "ORBIT_SIGN_IDENTITY=$(touch SHOULD_NOT_EXIST)\n"
            f"ORBIT_SIGN_KEYCHAIN={keychain}\n"
            f"ORBIT_SIGN_KEYCHAIN_PASSWORD_FILE={password}\n"
            f"ORBIT_NOTARY_KEY={notary}\n"
            "ORBIT_NOTARY_KEY_ID=synthetic\n"
            "ORBIT_NOTARY_ISSUER=synthetic\n"
        )
        secret.chmod(0o600)
        builddir = self.scratch / "artifacts"
        result = self.run_mac_build(
            {
                **env,
                "ORBIT_SIGNING_ENV": str(secret),
                "ORBIT_BUILDDIR": str(builddir),
            },
            cwd=fixture,
            script=fixture / "scripts/release/mac-build.sh",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        out = builddir / "out"
        temp = builddir / "temp"
        cc_flags = (
            "cc -std=c11 -Wall -Wextra -Werror -fsanitize=address,undefined -pthread"
            " -I native/macos/Sources/OrbitAudioRing/include"
        )
        cc_common = f"{cc_flags} native/macos/Sources/OrbitAudioRing/OrbitAudioRing.c"
        self.assertEqual(
            self.log.read_text().splitlines(),
            [
                f"security find-certificate -c $(touch SHOULD_NOT_EXIST) {keychain}",
                "bash scripts/macos/check-sdk.sh sdk",
                "check-sdk.sh sdk",
                "rustup show",
                "cargo fmt --all -- --check",
                "cargo build -p orbit-daemon --bin orbit --locked",
                "cargo test --workspace --locked",
                "cargo clippy --workspace --all-targets --locked -- -D warnings",
                "bash scripts/macos/check-sdk.sh xctest",
                "check-sdk.sh xctest",
                "swift test --package-path native/macos",
                f"{cc_common} native/macos/Tests/ring_test.c -o {temp}/orbit-ring-test",
                "run-test orbit-ring-test",
                # The barrier test includes OrbitAudioRing.c: it is not linked twice.
                f"{cc_flags} native/macos/Tests/stop_barrier_test.c -o {temp}/orbit-stop-barrier-test",
                "run-test orbit-stop-barrier-test",
                "bash scripts/macos/bundle.sh",
                "bundle",
                f"bash scripts/release/verify-built-archive.sh {out} {commit} arm64",
                f"verify-built-archive.sh {out} {commit} arm64",
                f"dwarfdump --uuid {out}/Orbit.app/Contents/MacOS/Orbit",
                "dwarfdump --uuid target/release/orbit-desktop.dSYM",
                f"ditto -c -k --keepParent target/release/orbit-desktop.dSYM {out}/orbit-desktop.dSYM.zip",
                f"shasum -a 256 {out}/Orbit-9.8.7-macos-arm64.zip",
            ],
        )
        # The signing file is data: its $(...) payload never executes.
        self.assertFalse((fixture / "SHOULD_NOT_EXIST").exists())

    def test_the_host_path_resolves_pnpm_and_bash_5(self):
        """Over non-interactive SSH the Mac's PATH has no pnpm, node or bash 5.

        The probe resolves the shared RELEASE_HOST_PATH and asks the shell
        which tools it finds. The $HOME
        entries are proven with doubles; /opt/homebrew/bin, where the real Mac
        keeps pnpm and bash 5, cannot exist on this machine and is carried by
        the line verbatim.
        """
        home, _, local = self.fake_home()
        self.tool(
            "pnpm",
            '#!/bin/sh\necho "pnpm $*" >> "$ORBIT_TEST_LOG"\n[ "$1" = --version ] && echo 10.10.0\nexit 0\n',
            directory=local,
        )
        self.tool(
            "bash",
            "#!/bin/sh\n"
            'echo "bash $*" >> "$ORBIT_TEST_LOG"\n'
            'if [ "$1" = "--version" ]; then printf "GNU bash, version 5.3.9(1)-release (double)\\n"; exit 0; fi\n'
            'exec /usr/bin/bash "$@"\n',
            directory=local,
        )
        export = ('source "$1"\n'
                  'export PATH="${RELEASE_HOST_PATH//\\$HOME/$HOME}"')
        probe = subprocess.run(
            [
                "/usr/bin/bash",
                "-c",
                export
                + "\ncommand -v pnpm\ncommand -v bash\npnpm --version\nbash --version | sed -n 1p\n",
                "probe", str(RELEASE / "lib.sh"),
            ],
            env={
                "HOME": str(home),
                "PATH": "/usr/bin:/bin",
                "ORBIT_TEST_LOG": str(self.log),
            },
            text=True,
            capture_output=True,
            timeout=10,
            check=False,
        )
        self.assertEqual(probe.returncode, 0, probe.stderr)
        self.assertIn(str(local / "pnpm"), probe.stdout)
        self.assertIn(str(local / "bash"), probe.stdout)
        self.assertIn("10.10.0", probe.stdout)
        self.assertIn("5.3.9", probe.stdout)
        for name in ("scripts/release/mac-build.sh", "scripts/release/build-on-mac.sh",
                     "scripts/e2e-mac/build-test-app.sh"):
            self.assertIn("RELEASE_HOST_PATH", (ROOT / name).read_text())

    def test_signing_prechecks_stop_before_compiling(self):
        """A missing keychain or certificate refuses before any stage runs.

        codesign would otherwise fail half an hour into the build, after fmt,
        tests, clippy and the whole bundle.
        """
        password = self.scratch / "password"
        notary = self.scratch / "notary-key"
        for path in (password, notary):
            path.write_text("private")
        secret = self.scratch / "signing.env"
        secret.write_text(
            "ORBIT_SIGN_IDENTITY=Synthetic Identity\n"
            f"ORBIT_SIGN_KEYCHAIN={self.scratch}/absent.keychain\n"
            f"ORBIT_SIGN_KEYCHAIN_PASSWORD_FILE={password}\n"
            f"ORBIT_NOTARY_KEY={notary}\n"
            "ORBIT_NOTARY_KEY_ID=synthetic\n"
            "ORBIT_NOTARY_ISSUER=synthetic\n"
        )
        secret.chmod(0o600)
        env, _, _, _ = self.mac_build_env()
        overrides = {
            "ORBIT_SIGNING_ENV": str(secret),
            "ORBIT_BUILDDIR": str(self.scratch / "new"),
        }
        missing_keychain = self.run_mac_build({**env, **overrides})
        self.assertNotEqual(missing_keychain.returncode, 0)
        self.assertIn("keychain", missing_keychain.stderr)
        self.assertIn(f"{self.scratch}/absent.keychain", missing_keychain.stderr)
        if self.log.exists():
            self.assertEqual(self.log.read_text().splitlines(), [])

        keychain = self.scratch / "present.keychain"
        keychain.write_text("synthetic keychain")
        secret.write_text(
            "ORBIT_SIGN_IDENTITY=Synthetic Identity\n"
            f"ORBIT_SIGN_KEYCHAIN={keychain}\n"
            f"ORBIT_SIGN_KEYCHAIN_PASSWORD_FILE={password}\n"
            f"ORBIT_NOTARY_KEY={notary}\n"
            "ORBIT_NOTARY_KEY_ID=synthetic\n"
            "ORBIT_NOTARY_ISSUER=synthetic\n"
        )
        secret.chmod(0o600)
        env, _, _, _ = self.mac_build_env(
            security_body='#!/bin/sh\necho "security $*" >> "$ORBIT_TEST_LOG"\nexit 1\n'
        )
        missing_certificate = self.run_mac_build({**env, **overrides})
        self.assertNotEqual(missing_certificate.returncode, 0)
        self.assertIn("certificate", missing_certificate.stderr)
        self.assertIn("Synthetic Identity", missing_certificate.stderr)
        self.assertIn(str(keychain), missing_certificate.stderr)
        calls = self.log.read_text()
        self.assertIn("security find-certificate -c Synthetic Identity", calls)
        self.assertNotIn("cargo", calls)


    def test_missing_commit_and_version_report_both_values(self):
        self.assertTrue(
            (RELEASE / "build-on-mac.sh").is_file(), "the versioned launcher is missing"
        )
        commit = "f" * 40
        result = self.run_script("build-on-mac.sh", "9.8.7", commit)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("9.8.7", result.stderr)
        self.assertIn(commit, result.stderr)

    def test_wrong_version_and_failed_version_check_stop_before_transport(self):
        commit = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True, timeout=10
        ).strip()
        wrong = self.run_script("build-on-mac.sh", "9.8.7", commit)
        self.assertNotEqual(wrong.returncode, 0)
        self.assertIn("9.8.7", wrong.stderr)
        self.assertIn(committed_version(commit), wrong.stderr)
        self.assertIn(commit, wrong.stderr)

        fixture = self.scratch / "bad-version"
        (fixture / "scripts/release").mkdir(parents=True)
        for name in ("lib.sh", "build-on-mac.sh"):
            shutil.copy2(RELEASE / name, fixture / "scripts/release" / name)
        (fixture / "scripts/release/set-version.py").write_text(
            'raise SystemExit("versions disagree")\n'
        )
        (fixture / "Cargo.toml").write_text('[workspace.package]\nversion = "7.8.9"\n')
        subprocess.run(["git", "init", "-q", str(fixture)], check=True, timeout=10)
        subprocess.run(["git", "-C", str(fixture), "add", "."], check=True, timeout=10)
        subprocess.run(
            [
                "git",
                "-C",
                str(fixture),
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                "fixture",
            ],
            check=True,
            timeout=10,
        )
        failing_commit = subprocess.check_output(
            ["git", "-C", str(fixture), "rev-parse", "HEAD"], text=True, timeout=10
        ).strip()
        failed = subprocess.run(
            [
                "bash",
                str(fixture / "scripts/release/build-on-mac.sh"),
                "7.8.9",
                failing_commit,
            ],
            cwd=fixture,
            env=self.env,
            text=True,
            capture_output=True,
            timeout=30,
            check=False,
        )
        self.assertNotEqual(failed.returncode, 0)
        self.assertIn("set-version.py --check", failed.stderr)
        self.assertIn("7.8.9", failed.stderr)
        self.assertIn(failing_commit, failed.stderr)
        self.assertIn("versions disagree", failed.stderr)

    def test_sent_tree_is_exactly_the_commit_and_includes_git(self):
        fixture = self.scratch / "git-repo"
        (fixture / "scripts/release").mkdir(parents=True)
        for name in ("lib.sh", "build-on-mac.sh"):
            shutil.copy2(RELEASE / name, fixture / "scripts/release" / name)
        (fixture / "scripts/release/set-version.py").write_text("import sys\nsys.exit(0)\n")
        (fixture / "Cargo.toml").write_text('[workspace.package]\nversion = "7.8.9"\n')
        (fixture / ".gitignore").write_text("ignored/\n")
        subprocess.run(
            ["git", "init", "-q", "-b", "main", str(fixture)], check=True, timeout=10
        )
        subprocess.run(["git", "-C", str(fixture), "add", "."], check=True, timeout=10)
        subprocess.run(
            [
                "git",
                "-C",
                str(fixture),
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                "fixture",
            ],
            check=True,
            timeout=10,
        )
        commit = subprocess.check_output(
            ["git", "-C", str(fixture), "rev-parse", "HEAD"], text=True, timeout=10
        ).strip()
        (fixture / "ignored").mkdir()
        (fixture / "ignored/local.json").write_text("private")
        (fixture / "untracked.txt").write_text("untracked")
        sent = self.scratch / "sent"
        sent.mkdir()
        key = self.scratch / "ssh-key"
        key.write_text("fake")
        self.tool(
            "rsync",
            '#!/bin/sh\nprintf "rsync %s\\n" "$*" >> "$ORBIT_TEST_LOG"\nfor arg do if [ -d "$arg/.git" ]; then cp -a "$arg/." "$ORBIT_TEST_COPY_DIR"; fi; done\n',
        )
        result = subprocess.run(
            ["bash", str(fixture / "scripts/release/build-on-mac.sh"), "7.8.9", commit],
            env={
                **self.env,
                "ORBIT_MAC_HOST": "fixture@mac",
                "ORBIT_MAC_SSH_KEY": str(key),
                "ORBIT_CUA_DRIVER_BINARY": "/fixture/cua-driver",
                "ORBIT_SIGNING_ENV": "/fixture/signing.env",
                "ORBIT_TEST_COPY_DIR": str(sent),
                "ORBIT_UPDATE_BASE_URL": "https://updates.example.test/path?channel=beta&track=test",
                "VITE_ORBIT_HUB_URL": "https://hub.example",
            },
            cwd=fixture,
            text=True,
            capture_output=True,
            timeout=30,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        for line in self.log.read_text().splitlines():
            if line.startswith(("ssh ", "rsync ")):
                for option in (
                    "-o BatchMode=yes",
                    "-o ConnectTimeout=15",
                    "-o IdentitiesOnly=yes",
                ):
                    self.assertIn(option, line)
        launch = next(
            line for line in self.log.read_text().splitlines() if "mac-build.sh" in line
        )
        self.assertIn("$HOME/.cargo/bin:$HOME/.local/bin:/opt/homebrew/bin", launch)
        arguments = shlex.split(launch)
        self.assertIn("ORBIT_UPDATE_BASE_URL=https://updates.example.test/path?channel=beta&track=test", arguments)
        self.assertIn("VITE_ORBIT_HUB_URL=https://hub.example", arguments)
        expected = subprocess.check_output(
            ["git", "-C", str(fixture), "ls-tree", "-r", "--name-only", commit],
            text=True,
            timeout=10,
        ).splitlines()
        actual = sorted(
            str(path.relative_to(sent))
            for path in sent.rglob("*")
            if path.is_file() and ".git" not in path.relative_to(sent).parts
        )
        self.assertEqual(actual, sorted(expected))
        self.assertTrue((sent / ".git/refs/heads/main").is_file())
        self.assertEqual(
            subprocess.check_output(
                ["git", "-C", str(sent), "rev-parse", "HEAD"], text=True, timeout=10
            ).strip(),
            commit,
        )


    def bundle_fixture(self, swift_fails=False):
        fixture = self.scratch / "fixture"
        files = (
            "scripts/macos/bundle.sh",
            "scripts/macos/copy-localizations.sh",
            "Cargo.toml",
            "crates/orbit-control/src/cua.rs",
            "native/macos/Info.plist",
            "native/macos/Resources/en.lproj/InfoPlist.strings",
            "native/macos/Resources/es.lproj/InfoPlist.strings",
            "native/macos/Resources/en.lproj/Localizable.strings",
            "native/macos/Resources/es.lproj/Localizable.strings",
            "native/macos/Orbit.entitlements",
            "apps/desktop/src-tauri/icons/icon.icns",
            "third-party/cua-driver/LICENSE.md",
            "LICENSE",
            "scripts/release/third-party-notices.py",
        )
        for name in files:
            target = fixture / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, target)
        shutil.copytree(ROOT / "Icon", fixture / "Icon")
        bundle = fixture / "scripts/macos/bundle.sh"
        bundle.write_text(
            bundle.read_text().replace(
                "/usr/libexec/PlistBuddy", str(self.bin / "plistbuddy")
            )
        )
        if (ROOT / "scripts/release/lib.sh").is_file():
            target = fixture / "scripts/release/lib.sh"
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / "scripts/release/lib.sh", target)
        smoke = fixture / "scripts/macos/smoke-native.py"
        # The double refuses any --binary that is not the signed helper inside
        # the bundle: a smoke that ignores its arguments would stay green while
        # the script pointed it at the unsigned copy.
        smoke.write_text(
            "import os\n"
            "import sys\n"
            "args = sys.argv[1:]\n"
            "binary = args[args.index('--binary') + 1] if '--binary' in args else ''\n"
            "with open(os.environ['ORBIT_TEST_LOG'], 'a') as handle:\n"
            "    handle.write('smoke ' + ' '.join(args) + '\\n')\n"
            "end = 'Orbit.app/Contents/MacOS/OrbitNative'\n"
            "if not binary.endswith(end):\n"
            "    print(f'smoke: --binary must be the signed helper ({end}); received: {binary}', file=sys.stderr)\n"
            "    sys.exit(48)\n"
            "if not os.path.isfile(binary):\n"
            "    print(f'smoke: the signed helper does not exist: {binary}', file=sys.stderr)\n"
            "    sys.exit(49)\n"
            "print('smoke broken', file=sys.stderr)\n"
            "sys.exit(47)\n"
        )
        for name in ("orbit", "orbit-desktop"):
            target = fixture / "target/release" / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text("fixture")
        swift_bin = self.scratch / "swift-bin"
        swift_bin.mkdir()
        (swift_bin / "OrbitNative").write_text("fixture")
        self.tool(
            "swift",
            f'#!/bin/sh\necho "swift $*" >> "$ORBIT_TEST_LOG"\nif [ "$1" = test ] && [ "{int(swift_fails)}" = 1 ]; then echo "swift broken" >&2; exit 46; fi\ncase "$*" in *--show-bin-path*) echo "{swift_bin}";; esac\n',
        )
        pnpm, cargo = notice_fixture.listings(self.scratch / "listings")
        for tool in (
            "pnpm",
            "cargo",
            "codesign",
            "xcrun",
            "spctl",
            "security",
            "ditto",
        ):
            answer = notice_fixture.answering(tool, pnpm, cargo) if tool in ("pnpm", "cargo") else ""
            self.tool(tool, f'#!/bin/sh\necho "{tool} $*" >> "$ORBIT_TEST_LOG"\n{answer}')
        self.tool(
            "codesign",
            '#!/bin/sh\necho "codesign $*" >> "$ORBIT_TEST_LOG"\n'
            'if [ "$1" = -d ]; then echo "designated => certificate leaf[subject.OU] = SYNTHETIC" >&2; fi\n',
        )
        self.tool(
            "xcrun",
            '#!/bin/sh\necho "xcrun $*" >> "$ORBIT_TEST_LOG"\n'
            'if [ "$1" = actool ]; then\n'
            '  while [ "$1" != --compile ]; do shift; done\n'
            '  printf "compiled icon fixture" > "$2/Assets.car"\n'
            'fi\n'
            'case "$*" in *"notarytool submit"*) echo \'{"id":"synthetic"}\';;'
            ' *"notarytool info"*) echo \'{"status":"Accepted"}\';; esac\n',
        )
        self.tool(
            "ditto",
            '#!/bin/sh\necho "ditto $*" >> "$ORBIT_TEST_LOG"\nfor last do :; done\ncase "$*" in *-c*) touch "$last";; esac\n',
        )
        self.tool("lipo", "#!/bin/sh\necho arm64\n")
        self.tool("plistbuddy", "#!/bin/sh\nexit 0\n")
        driver = self.scratch / "cua-driver"
        version = verified_driver_version()
        driver.write_text(f'#!/bin/sh\necho "cua-driver {version}"\n')
        driver.chmod(0o755)
        return fixture, driver

    def test_swift_failure_stops_before_pnpm(self):
        fixture, driver = self.bundle_fixture(swift_fails=True)
        result = subprocess.run(
            ["bash", str(fixture / "scripts/macos/bundle.sh")],
            env={
                **self.env,
                "ORBIT_CUA_DRIVER_BINARY": str(driver),
                "ORBIT_OUTPUT_DIR": str(self.scratch / "output"),
            },
            cwd=fixture,
            text=True,
            capture_output=True,
            timeout=20,
            check=False,
        )
        self.assertEqual(result.returncode, 46, result.stderr)
        self.assertIn("swift broken", result.stderr)
        self.assertNotIn("pnpm", self.log.read_text())

    def test_smoke_failure_stops_before_notary(self):
        fixture, driver = self.bundle_fixture()
        notary = self.scratch / "notary-key"
        notary.write_text("synthetic")
        result = subprocess.run(
            ["bash", str(fixture / "scripts/macos/bundle.sh")],
            env={
                **self.env,
                "ORBIT_CUA_DRIVER_BINARY": str(driver),
                "ORBIT_OUTPUT_DIR": str(self.scratch / "output"),
                "ORBIT_SIGN_IDENTITY": "Synthetic Developer ID",
                "ORBIT_NOTARY_KEY": str(notary),
                "ORBIT_NOTARY_KEY_ID": "synthetic",
                "ORBIT_NOTARY_ISSUER": "synthetic",
            },
            cwd=fixture,
            text=True,
            capture_output=True,
            timeout=20,
            check=False,
        )
        self.assertEqual(result.returncode, 47, result.stderr)
        self.assertIn("smoke broken", result.stderr)
        calls = self.log.read_text()
        self.assertIn("codesign --verify", calls)
        self.assertNotIn("notarytool", calls)
        self.assertTrue(
            [
                line
                for line in calls.splitlines()
                if line.startswith("smoke --binary ")
                and line.endswith("/Orbit.app/Contents/MacOS/OrbitNative")
            ],
            "the smoke did not receive the bundle's signed helper",
        )


    def test_refuses_wrong_checksum_and_dirty_manifest_before_mac_verification(self):
        commit = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True, timeout=10
        ).strip()
        requested_version = committed_version(commit)
        archive = self.scratch / "remote" / f"Orbit-{requested_version}-macos-arm64.zip"
        archive.parent.mkdir()
        archive.write_bytes(b"synthetic zip")
        checksum = archive.with_name(archive.name + ".sha256")
        checksum.write_text("0" * 64 + f"  {archive.name}\n")
        manifest = archive.parent / "build-manifest.txt"
        manifest.write_text(
            f"version={requested_version}\nsource_commit={commit}\nsource_dirty=false\n"
        )
        key = self.scratch / "ssh-key"
        key.write_text("fake")
        self.tool(
            "rsync",
            '#!/bin/sh\nfor last do :; done\nfor arg do case "$arg" in *:~/*) name=${arg##*/}; cp "$ORBIT_TEST_REMOTE_DIR/$name" "$last"; break;; esac; done\n',
        )
        env = {
            "ORBIT_MAC_HOST": "fixture@mac",
            "ORBIT_MAC_SSH_KEY": str(key),
            "ORBIT_RELEASE_DIR": str(self.scratch / "releases"),
            "ORBIT_TEST_REMOTE_DIR": str(archive.parent),
        }
        wrong_hash = self.run_script("fetch-build.sh", requested_version, commit, env=env)
        self.assertNotEqual(wrong_hash.returncode, 0)
        self.assertIn("SHA-256 differs", wrong_hash.stderr)
        self.assertIn(commit, wrong_hash.stderr)
        self.assertNotIn("ssh ", self.log.read_text() if self.log.exists() else "")
        checksum.write_text(
            hashlib.sha256(archive.read_bytes()).hexdigest() + f"  {archive.name}\n"
        )
        manifest.write_text(
            f"version={requested_version}\nsource_commit={commit}\nsource_dirty=true\n"
        )
        dirty = self.run_script("fetch-build.sh", requested_version, commit, env=env)
        self.assertNotEqual(dirty.returncode, 0)
        self.assertIn("source_dirty", dirty.stderr)
        self.assertIn(commit, dirty.stderr)
        self.assertIn("manifest='true'", dirty.stderr)
        self.assertFalse((self.scratch / "releases" / requested_version).exists())
        nonexistent_version = "9.9.9"
        for wrong_version, wrong_commit in (
            (nonexistent_version, commit),
            (requested_version, "b" * 40),
        ):
            manifest.write_text(
                f"version={wrong_version}\nsource_commit={wrong_commit}\nsource_dirty=false\n"
            )
            refused = self.run_script("fetch-build.sh", requested_version, commit, env=env)
            self.assertNotEqual(refused.returncode, 0)
            self.assertIn("Manifest differs", refused.stderr)
            self.assertIn(wrong_version, refused.stderr)
            self.assertIn(wrong_commit, refused.stderr)
        manifest.write_text(
            f"version={requested_version}\nsource_commit={commit}\nsource_dirty=false\n"
        )
        invalid = self.run_script("fetch-build.sh", requested_version, commit, env=env)
        self.assertNotEqual(invalid.returncode, 0)
        self.assertIn("verify-macos-archive.py refused", invalid.stderr)
        self.assertIn(commit, invalid.stderr)

    def test_valid_synthetic_archive_is_fetched_and_mac_receipt_is_kept(self):
        # The archive and its verifier must come from the same source, even
        # while this worktree changes the bundle layout before integration.
        fixture = self.scratch / "source"
        for name in (
            "scripts/release/lib.sh", "scripts/release/fetch-build.sh",
            "scripts/release/verify-macos-archive.py", "scripts/release/strict_zip.py",
            "Cargo.toml",
            "crates/orbit-control/src/cua.rs", "third-party/cua-driver/LICENSE.md",
        ):
            target = fixture / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, target)
        commit = self.git_commit(fixture, "synthetic archive verifier")
        remote = self.scratch / "remote"
        remote.mkdir()
        manifest = archive_fixture.manifest().replace(
            archive_fixture.COMMIT.encode(), commit.encode()
        )
        version = dict(line.split("=", 1) for line in manifest.decode().splitlines())["version"]
        name = f"Orbit-{version}-macos-arm64.zip"
        entries = archive_fixture.changed(
            archive_fixture.entries(), archive_fixture.verify.MANIFEST, data=manifest
        )
        archive = remote / name
        archive.write_bytes(archive_fixture.zip_bytes(entries))
        (remote / "build-manifest.txt").write_bytes(manifest)
        (remote / f"{name}.sha256").write_text(
            hashlib.sha256(archive.read_bytes()).hexdigest() + f"  {name}\n"
        )
        (remote / "orbit-desktop.dSYM.zip").write_bytes(b"synthetic symbols")
        key = self.scratch / "ssh-key"
        key.write_text("fake")
        self.tool(
            "rsync",
            '#!/bin/sh\nfor last do :; done\nfor arg do case "$arg" in *:~/*) name=${arg##*/}; cp "$ORBIT_TEST_REMOTE_DIR/$name" "$last"; break;; esac; done\n',
        )
        self.tool(
            "ssh",
            '#!/bin/sh\nprintf "ssh %s\\n" "$*" >> "$ORBIT_TEST_LOG"\nprintf "codesign accepted\\nspctl accepted\\nstapler accepted\\n"\n',
        )
        result = subprocess.run(
            ["bash", str(fixture / "scripts/release/fetch-build.sh"), version, commit],
            env={
                **self.env,
                "ORBIT_MAC_HOST": "fixture@mac",
                "ORBIT_MAC_SSH_KEY": str(key),
                "ORBIT_RELEASE_DIR": str(self.scratch / "releases"),
                "ORBIT_TEST_REMOTE_DIR": str(remote),
            },
            cwd=fixture, text=True, capture_output=True, timeout=40, check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        saved = self.scratch / "releases" / version
        self.assertEqual((saved / name).read_bytes(), archive.read_bytes())
        self.assertEqual((saved / "build-manifest.txt").read_bytes(), manifest)
        # The app's symbols come with it, for upload-symbols.sh.
        self.assertEqual((saved / "orbit-desktop.dSYM.zip").read_bytes(), b"synthetic symbols")
        self.assertEqual(
            (saved / "verificacion.txt").read_text(),
            "codesign accepted\nspctl accepted\nstapler accepted\n",
        )
        for line in self.log.read_text().splitlines():
            if line.startswith(("ssh ", "rsync ")):
                for option in (
                    "-o BatchMode=yes",
                    "-o ConnectTimeout=15",
                    "-o IdentitiesOnly=yes",
                ):
                    self.assertIn(option, line)

    def test_fetch_uses_requested_commit_when_checkout_has_next_version(self):
        fixture = self.scratch / "git-repo"
        (fixture / "scripts/release").mkdir(parents=True)
        for name in ("lib.sh", "fetch-build.sh"):
            shutil.copy2(RELEASE / name, fixture / "scripts/release" / name)
        (fixture / "Cargo.toml").write_text('[workspace.package]\nversion = "7.8.9"\n')
        # The verifier reads a file the old fixed extraction list never carried
        # (LICENSE, not Cargo.toml): only the whole tree of the commit has it.
        (fixture / "LICENSE").write_text("licence of the commit\n")
        verifier = fixture / "scripts/release/verify-macos-archive.py"
        verifier.parent.mkdir(parents=True, exist_ok=True)
        verifier.write_text(
            "from pathlib import Path\nimport sys\n"
            "repo = Path(__file__).resolve().parents[2]\n"
            'assert "7.8.9" in (repo / "Cargo.toml").read_text()\n'
            'assert (repo / "LICENSE").read_text() == "licence of the commit\\n"\n'
            'assert Path(sys.argv[1]).name == "Orbit-7.8.9-macos-arm64.zip"\n'
        )
        license_file = fixture / "third-party/cua-driver/LICENSE.md"
        license_file.parent.mkdir(parents=True)
        license_file.write_text("synthetic license\n")
        subprocess.run(["git", "init", "-q", str(fixture)], check=True, timeout=10)
        subprocess.run(["git", "-C", str(fixture), "add", "."], check=True, timeout=10)
        subprocess.run(
            [
                "git",
                "-C",
                str(fixture),
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                "fixture",
            ],
            check=True,
            timeout=10,
        )
        commit = subprocess.check_output(
            ["git", "-C", str(fixture), "rev-parse", "HEAD"], text=True, timeout=10
        ).strip()
        (fixture / "Cargo.toml").write_text('[workspace.package]\nversion = "9.9.9"\n')
        (fixture / "LICENSE").write_text("licence of the working tree\n")
        remote = self.scratch / "remote"
        remote.mkdir()
        name = "Orbit-7.8.9-macos-arm64.zip"
        (remote / name).write_bytes(b"synthetic zip")
        (remote / f"{name}.sha256").write_text(
            hashlib.sha256(b"synthetic zip").hexdigest() + f"  {name}\n"
        )
        (remote / "build-manifest.txt").write_text(
            f"version=7.8.9\nsource_commit={commit}\nsource_dirty=false\n"
        )
        key = self.scratch / "ssh-key"
        key.write_text("fake")
        self.tool(
            "rsync",
            '#!/bin/sh\nprintf "rsync %s\\n" "$*" >> "$ORBIT_TEST_LOG"\nfor last do :; done\nfor arg do case "$arg" in *:~/*) name=${arg##*/}; cp "$ORBIT_TEST_REMOTE_DIR/$name" "$last"; break;; esac; done\n',
        )
        self.tool(
            "ssh",
            '#!/bin/sh\nprintf "ssh %s\\n" "$*" >> "$ORBIT_TEST_LOG"\nprintf "codesign accepted\\nspctl accepted\\nstapler accepted\\n"\n',
        )
        result = subprocess.run(
            ["bash", str(fixture / "scripts/release/fetch-build.sh"), "7.8.9", commit],
            env={
                **self.env,
                "ORBIT_MAC_HOST": "fixture@mac",
                "ORBIT_MAC_SSH_KEY": str(key),
                "ORBIT_RELEASE_DIR": str(self.scratch / "releases"),
                "ORBIT_TEST_REMOTE_DIR": str(remote),
            },
            cwd=fixture,
            text=True,
            capture_output=True,
            timeout=30,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.scratch / "releases/7.8.9/verificacion.txt").is_file())
        for line in self.log.read_text().splitlines():
            if line.startswith(("ssh ", "rsync ")):
                for option in (
                    "-o BatchMode=yes",
                    "-o ConnectTimeout=15",
                    "-o IdentitiesOnly=yes",
                ):
                    self.assertIn(option, line)

    def test_mac_verification_runs_all_three_checks_on_a_temporary_bundle(self):
        fixture = self.scratch / "mac-verify"
        (fixture / "scripts/release").mkdir(parents=True)
        for name in ("lib.sh", "mac-verify.sh"):
            shutil.copy2(RELEASE / name, fixture / "scripts/release" / name)
        archive = self.scratch / "synthetic.zip"
        archive.write_bytes(b"synthetic")
        self.tool(
            "ditto", '#!/bin/sh\nfor last do :; done\nmkdir -p "$last/Orbit.app"\n'
        )
        result = subprocess.run(
            ["bash", str(fixture / "scripts/release/mac-verify.sh"), str(archive)],
            env=self.env,
            cwd=fixture,
            text=True,
            capture_output=True,
            timeout=20,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Signature accepted", result.stdout)
        self.assertIn("Gatekeeper accepted", result.stdout)
        self.assertIn("Stapling accepted", result.stdout)
        calls = self.log.read_text()
        self.assertIn("codesign --verify --deep --strict", calls)
        self.assertIn("spctl --assess --type execute", calls)
        self.assertIn("xcrun stapler validate", calls)

    def test_mac_verification_stops_on_each_failed_check(self):
        fixture = self.scratch / "mac-verify"
        (fixture / "scripts/release").mkdir(parents=True)
        for name in ("lib.sh", "mac-verify.sh"):
            shutil.copy2(RELEASE / name, fixture / "scripts/release" / name)
        archive = self.scratch / "synthetic.zip"
        archive.write_bytes(b"synthetic")
        self.tool(
            "ditto", '#!/bin/sh\nfor last do :; done\nmkdir -p "$last/Orbit.app"\n'
        )
        for failing, later in (
            ("codesign", "spctl"),
            ("spctl", "xcrun"),
            ("xcrun", "Stapling accepted"),
        ):
            with self.subTest(failing=failing):
                for tool in ("codesign", "spctl", "xcrun"):
                    exit_code = 72 if tool == failing else 0
                    self.tool(
                        tool,
                        f'#!/bin/sh\necho "{tool} $*" >> "$ORBIT_TEST_LOG"\nexit {exit_code}\n',
                    )
                self.log.write_text("")
                result = subprocess.run(
                    [
                        "bash",
                        str(fixture / "scripts/release/mac-verify.sh"),
                        str(archive),
                    ],
                    env=self.env,
                    cwd=fixture,
                    text=True,
                    capture_output=True,
                    timeout=20,
                    check=False,
                )
                self.assertEqual(result.returncode, 72)
                self.assertNotIn(later, self.log.read_text() + result.stdout)

    def test_test_marker_refuses_a_real_external_tool(self):
        (self.bin / "ssh").unlink()
        for name in (
            "build-on-mac.sh",
            "fetch-build.sh",
            "mac-verify.sh",
        ):
            with self.subTest(name=name):
                result = self.run_script(name, archive_fixture.verify.VERSION, "f" * 40)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("ssh", result.stderr)
        # mac-build.sh exports the host PATH, so its doubles must live under
        # the fake $HOME even for this refusal.
        env, _, cargo, _ = self.mac_build_env()
        (cargo / "ssh").unlink()
        mac_build = self.run_mac_build(env)
        self.assertNotEqual(mac_build.returncode, 0)
        self.assertIn("ssh", mac_build.stderr)
        result = subprocess.run(
            ["bash", str(ROOT / "scripts/macos/bundle.sh")],
            env=self.env,
            cwd=ROOT,
            text=True,
            capture_output=True,
            timeout=20,
            check=False,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ssh", result.stderr)

    def test_scripts_contain_no_embedded_host_or_signing_identity(self):
        sources = list(RELEASE.glob("*.sh")) + [ROOT / "scripts/macos/bundle.sh"]
        for path in sources:
            text = path.read_text()
            with self.subTest(path=path):
                for forbidden, why in FORBIDDEN_SECRET_FORMS:
                    self.assertNotRegex(text, forbidden, why)


    # ── The Sentry address ──────────────────────────────────────────────────

    def dsn_file(self, name="orbit-app.dsn", mode=0o600, text=SYNTHETIC_DSN + "\n"):
        path = self.scratch / name
        path.write_text(text)
        path.chmod(mode)
        return path

    def staged_mac_build(self, extra_env):
        """mac-build.sh over a synthetic tree with every stage doubled, as in
        the stage-order test; the bundle double writes what ORBIT_SENTRY_DSN
        it received to `bundle-dsn`, and cargo logs whether it saw one."""
        env, _, cargo, local = self.mac_build_env()
        self.tool(
            "cargo",
            '#!/bin/sh\necho "cargo $* dsn=${ORBIT_SENTRY_DSN+set}" >> "$ORBIT_TEST_LOG"\n',
            directory=cargo,
        )
        self.tool(
            "cc",
            "#!/bin/sh\n"
            "out= prev=\n"
            "for arg do\n"
            '  [ "$prev" = "-o" ] && out="$arg"\n'
            "  prev=$arg\n"
            "done\n"
            "printf '#!/bin/sh\\n' > \"$out\"\n"
            'chmod +x "$out"\n',
            directory=cargo,
        )
        for name in ("swift", "pnpm"):
            self.tool(name, "#!/bin/sh\nexit 0\n", directory=local)
        fixture = self.scratch / "dsn-repo"
        (fixture / "scripts/release").mkdir(parents=True)
        for name in ("lib.sh", "mac-build.sh"):
            shutil.copy2(RELEASE / name, fixture / "scripts/release" / name)
        (fixture / "scripts/macos").mkdir()
        for name in ("macos/check-sdk.sh", "release/verify-built-archive.sh"):
            (fixture / "scripts" / name).write_text("#!/bin/sh\nexit 0\n")
        seen = self.scratch / "bundle-dsn"
        (fixture / "scripts/macos/bundle.sh").write_text(
            "#!/bin/sh\n"
            f'printf "%s" "${{ORBIT_SENTRY_DSN-unset}}" > "{seen}"\n'
            'mkdir -p "$ORBIT_OUTPUT_DIR/Orbit.app/Contents/MacOS" target/release/orbit-desktop.dSYM\n'
            'printf binary > "$ORBIT_OUTPUT_DIR/Orbit.app/Contents/MacOS/Orbit"\n'
            'printf "zip" > "$ORBIT_OUTPUT_DIR/Orbit-9.8.7-macos-arm64.zip"\n'
        )
        self.git_commit(fixture, "fixture")
        for path in ("keychain", "password", "notary-key"):
            (self.scratch / path).write_text("private")
        secret = self.scratch / "signing.env"
        secret.write_text(
            "ORBIT_SIGN_IDENTITY=Synthetic Identity\n"
            f"ORBIT_SIGN_KEYCHAIN={self.scratch}/keychain\n"
            f"ORBIT_SIGN_KEYCHAIN_PASSWORD_FILE={self.scratch}/password\n"
            f"ORBIT_NOTARY_KEY={self.scratch}/notary-key\n"
            "ORBIT_NOTARY_KEY_ID=synthetic\n"
            "ORBIT_NOTARY_ISSUER=synthetic\n"
        )
        secret.chmod(0o600)
        result = self.run_mac_build(
            {
                **env,
                "ORBIT_SIGNING_ENV": str(secret),
                "ORBIT_BUILDDIR": str(self.scratch / "dsn-artifacts"),
                **extra_env,
            },
            cwd=fixture,
            script=fixture / "scripts/release/mac-build.sh",
        )
        return result, fixture, seen

    def test_the_sentry_address_reaches_only_the_bundle(self):
        result, _, seen = self.staged_mac_build(
            {"ORBIT_SENTRY_DSN_FILE": str(self.dsn_file())}
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(seen.read_text(), SYNTHETIC_DSN)
        calls = self.log.read_text()
        # fmt, the debug build, the tests and clippy never see the address.
        cargo = [line for line in calls.splitlines() if line.startswith("cargo ")]
        self.assertEqual(len(cargo), 4, calls)
        for line in cargo:
            self.assertTrue(line.endswith("dsn="), line)
        for text in (calls, result.stdout, result.stderr):
            self.assertNotIn(SYNTHETIC_DSN, text)
            self.assertNotIn("0123456789abcdef0123456789abcdef", text)

    def test_without_the_address_the_bundle_builds_without_it(self):
        result, _, seen = self.staged_mac_build({"ORBIT_SENTRY_DSN_FILE": ""})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(seen.read_text(), "unset")

    def test_a_readable_misplaced_or_malformed_address_file_is_refused(self):
        env, _, _, _ = self.mac_build_env()
        readable = self.dsn_file("readable.dsn", mode=0o644)
        malformed = self.dsn_file("malformed.dsn", text="not an address\n")
        linked = self.scratch / "linked.dsn"
        linked.symlink_to(self.dsn_file("target.dsn"))
        inside = ROOT / "target" / "release-test-inside.dsn"
        cases = [
            (readable, "mode 644"),
            (malformed, "does not contain a Sentry address"),
            (linked, "is not a regular file"),
            (self.scratch / "absent.dsn", "is not a regular file"),
        ]
        inside.parent.mkdir(exist_ok=True)
        inside.write_text(SYNTHETIC_DSN + "\n")
        inside.chmod(0o600)
        self.addCleanup(inside.unlink)
        cases.append((inside, "outside the repository"))
        for path, why in cases:
            with self.subTest(path=path.name):
                result = self.run_mac_build(
                    {
                        **env,
                        "ORBIT_SENTRY_DSN_FILE": str(path),
                        "ORBIT_BUILDDIR": str(self.scratch / f"never-{path.name}"),
                    }
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(why, result.stderr)
                self.assertNotIn(SYNTHETIC_DSN, result.stderr)
        if self.log.exists():
            self.assertNotIn("cargo", self.log.read_text())

    def launcher_fixture(self):
        fixture = self.scratch / "launch-repo"
        (fixture / "scripts/release").mkdir(parents=True)
        for name in ("lib.sh", "build-on-mac.sh"):
            shutil.copy2(RELEASE / name, fixture / "scripts/release" / name)
        (fixture / "scripts/release/set-version.py").write_text("import sys\nsys.exit(0)\n")
        (fixture / "Cargo.toml").write_text('[workspace.package]\nversion = "7.8.9"\n')
        commit = self.git_commit(fixture, "fixture")
        key = self.scratch / "ssh-key"
        key.write_text("fake")
        return fixture, commit, {
            "ORBIT_MAC_HOST": "fixture@mac",
            "ORBIT_MAC_SSH_KEY": str(key),
            "ORBIT_CUA_DRIVER_BINARY": "/fixture/cua-driver",
            "ORBIT_SIGNING_ENV": "/fixture/signing.env",
        }

    def launch(self, fixture, commit, env):
        return subprocess.run(
            ["bash", str(fixture / "scripts/release/build-on-mac.sh"), "7.8.9", commit],
            env={**self.env, **env},
            cwd=fixture,
            text=True,
            capture_output=True,
            timeout=30,
            check=False,
        )

    def test_the_launcher_sends_the_address_as_a_private_file(self):
        fixture, commit, env = self.launcher_fixture()
        dsn = self.dsn_file()
        result = self.launch(fixture, commit, {**env, "ORBIT_SENTRY_DSN_FILE": str(dsn)})
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.log.read_text()
        sent = [line for line in calls.splitlines() if str(dsn) in line]
        self.assertEqual(len(sent), 1, calls)
        self.assertTrue(sent[0].startswith("rsync "), sent[0])
        self.assertIn("--chmod=F600", sent[0])
        self.assertIn("fixture@mac:~/orbit-rel-7.8.9/sentry.dsn", sent[0])
        launch = next(line for line in calls.splitlines() if "mac-build.sh" in line)
        self.assertIn('ORBIT_SENTRY_DSN_FILE="$HOME/orbit-rel-7.8.9/sentry.dsn"', launch)
        for text in (calls, result.stdout, result.stderr):
            self.assertNotIn(SYNTHETIC_DSN, text)

    def test_the_launcher_without_an_address_or_with_a_missing_one(self):
        fixture, commit, env = self.launcher_fixture()
        result = self.launch(fixture, commit, {**env, "ORBIT_SENTRY_DSN_FILE": ""})
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.log.read_text()
        self.assertNotIn("sentry.dsn", calls)
        launch = next(line for line in calls.splitlines() if "mac-build.sh" in line)
        self.assertIn('ORBIT_SENTRY_DSN_FILE=""', launch)

        self.log.unlink()
        missing = self.scratch / "missing.dsn"
        refused = self.launch(fixture, commit, {**env, "ORBIT_SENTRY_DSN_FILE": str(missing)})
        self.assertEqual(refused.returncode, 2, refused.stderr)
        self.assertIn(str(missing), refused.stderr)
        self.assertIn("Leave it empty", refused.stderr)
        if self.log.exists():
            self.assertNotRegex(self.log.read_text(), r"^(ssh|rsync) ")


    def test_a_dsym_of_another_binary_stops_the_release(self):
        """The symbols that go to Sentry must be the shipped app's (with the
        daemon inside): a dSYM whose UUID differs from the bundled binary's
        stops the build before anything is zipped for upload."""
        result, _, _ = self.staged_mac_build({"ORBIT_TEST_DSYM_UUID": "99999999-0000-0000-0000-000000000000"})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("is not its binary's", result.stderr)
        self.assertNotIn("ditto -c -k --keepParent target/release/orbit-desktop.dSYM", self.log.read_text())


if __name__ == "__main__":
    unittest.main()
