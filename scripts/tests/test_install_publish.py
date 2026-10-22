"""Exercise install.sh and publish-macos.sh by running them.

The installer is a macOS program, so the platform tools it leans on are stood in
for here and the packages come from a local server. That is enough to fix what
matters and cannot be checked by reading: that a package whose checksum does not
match is refused, that a previous install is kept rather than deleted, and that a
manifest arriving over the network cannot name anything but a plain file.

What this does NOT prove is anything about macOS itself: Gatekeeper, quarantine,
or whether the installed bundle runs. Those need the real machine.
"""

import functools
import hashlib
import http.server
import os
import shutil
import signal
import stat
import subprocess
import tempfile
import threading
import unittest
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
INSTALL = ROOT / "scripts" / "release" / "install.sh"
PUBLISH = ROOT / "scripts" / "release" / "publish-macos.sh"
EOL = chr(10)

UNAME = """#!/bin/bash
[ "$1" = "-m" ] && echo arm64 || echo Darwin
"""
SHASUM = """#!/bin/bash
sha256sum "${@: -1}"
"""
# The installer asks macOS which executable every process is running. Nothing is
# running in most of these tests, so the stand-in answers with the one line that
# is always true — this very shell — and no bundle executable at all.
PS = """#!/bin/bash
printf '%s %s %s\\n' "$$" "$PPID" "$0"
"""
# A `ps` that really looks at the machine, for the tests that start a synthetic
# bundle process. macOS prints the executable's path under `comm`; Linux prints
# a short name, so the path is read from /proc instead. A process started from a
# script reports its interpreter there, while on macOS the daemon and the helper
# are ordinary binaries that report themselves, so the interpreter's first
# argument — the script — stands in for the executable.
PS_FROM_PROC = """#!/bin/bash
exec python3 "$(dirname "$0")/ps_shim.py"
"""
PS_SHIM = '''import os

SHELLS = {"sh", "bash", "dash", "zsh"}


def is_interpreter(name):
    return name in SHELLS or name.startswith("python")


def parent_of(pid):
    with open("/proc/%s/status" % pid, encoding="utf-8") as handle:
        for line in handle:
            if line.startswith("PPid:"):
                return line.split()[1]
    return "0"


root = os.environ.get("ORBIT_PS_ROOT", "")
bundled = False
for entry in sorted(os.listdir("/proc")):
    if not entry.isdigit():
        continue
    try:
        executable = os.readlink("/proc/%s/exe" % entry)
        parent = parent_of(entry)
        arguments = open("/proc/%s/cmdline" % entry, "rb").read().decode(
            "utf-8", "replace"
        ).split("\\0")
    except OSError:
        continue
    if is_interpreter(os.path.basename(executable)) and len(arguments) > 1:
        if arguments[1].startswith("/"):
            executable = arguments[1]
    # A bundle belonging to somebody else on this machine — another copy of
    # these tests, or a Rust one — is none of this fixture's business.
    if "/Orbit.app/" in executable and not executable.startswith(root):
        executable = "/usr/bin/ajeno"
    bundled = bundled or "/Orbit.app/Contents/" in executable
    print(entry, parent, executable)

# Only when a bundle executable was actually there to be seen, so a test can
# tell that the installer looked while one was running.
seen = os.environ.get("ORBIT_PS_SEEN")
if seen and bundled:
    open(seen, "w", encoding="utf-8").close()
'''
UNZIP = """#!/bin/bash
exec python3 "$(dirname "$0")/unzip_shim.py" "$2" "$4"
"""
# Real unzip restores the stored mode bits and Python's extractall does not, so
# the stand-in has to as well: otherwise the installed command is not executable
# and the test would fail for a reason the installer is not responsible for.
UNZIP_SHIM = '''import os, sys, zipfile

with zipfile.ZipFile(sys.argv[1]) as bundle:
    for entry in bundle.infolist():
        written = bundle.extract(entry, sys.argv[2])
        mode = entry.external_attr >> 16
        if mode:
            os.chmod(written, mode)
'''
SHIMS = {"uname": UNAME, "shasum": SHASUM, "ps": PS, "unzip": UNZIP}


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


class Server:
    """Serve one directory on loopback for the duration of a test."""

    def __init__(self, directory):
        handler = functools.partial(
            http.server.SimpleHTTPRequestHandler, directory=str(directory)
        )
        handler.log_message = lambda *_args: None
        self.httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
        self.url = "http://127.0.0.1:%d" % self.httpd.server_address[1]
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()

    def close(self):
        self.httpd.shutdown()
        self.httpd.server_close()


class Fixture(unittest.TestCase):
    def setUp(self):
        self.work = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.work, ignore_errors=True)
        self.home = self.work / "home"
        self.home.mkdir()
        self.applications = self.work / "system-applications"
        self.bin = self.work / "bin"
        self.bin.mkdir()
        for name, body in SHIMS.items():
            shim = self.bin / name
            shim.write_text(body)
            shim.chmod(shim.stat().st_mode | stat.S_IEXEC)
        (self.bin / "unzip_shim.py").write_text(UNZIP_SHIM)

    def watch_real_processes(self):
        """Swap the empty `ps` for one that reports the machine's processes."""
        shim = self.bin / "ps"
        shim.write_text(PS_FROM_PROC)
        shim.chmod(shim.stat().st_mode | stat.S_IEXEC)
        (self.bin / "ps_shim.py").write_text(PS_SHIM)

    def bundle_process(self, relative, body="#!/bin/bash\nsleep 300\n", *arguments):
        """A synthetic process running from inside a bundle, killed on cleanup."""
        executable = self.work / "runtime" / relative
        executable.parent.mkdir(parents=True, exist_ok=True)
        executable.write_text(body)
        executable.chmod(executable.stat().st_mode | stat.S_IEXEC)
        process = subprocess.Popen(
            [str(executable), *arguments], start_new_session=True
        )

        def stop():
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
            process.wait()

        self.addCleanup(stop)
        return process

    def build_package(self, contents=b"orbit-binary"):
        # The bundle's one Rust executable is the app; the daemon runs
        # inside it and Contents/Resources/orbit is no longer shipped.
        staging = self.work / "staging"
        app = staging / "Orbit.app" / "Contents" / "MacOS"
        app.mkdir(parents=True, exist_ok=True)
        (app / "Orbit").write_bytes(contents)
        out = self.work / "build"
        out.mkdir(exist_ok=True)
        archive = out / "Orbit-0.1.0-macos-arm64.zip"
        with zipfile.ZipFile(archive, "w") as bundle:
            for path in sorted(staging.rglob("*")):
                if path.is_file():
                    info = zipfile.ZipInfo(str(path.relative_to(staging)))
                    info.external_attr = 0o755 << 16
                    bundle.writestr(info, path.read_bytes())
        checksum = out / (archive.name + ".sha256")
        checksum.write_text(digest(archive) + "  " + archive.name)
        (out / "build-manifest.txt").write_text(
            "version=0.1.0\narchitecture=arm64\nsource_commit=" + "a" * 40
            + "\nsource_dirty=false\nminimum_macos=15.0\nnotarized=true\n"
        )
        (out / "install.sh").write_bytes(INSTALL.read_bytes())
        (out / "install.sha256").write_text(
            digest(out / "install.sh") + "  install.sh\n"
        )
        return out

    def publish(self, source, version="0.1.0", serve=None):
        serve = serve or (self.work / "serve")
        serve.mkdir(exist_ok=True)
        return subprocess.run(
            ["bash", str(PUBLISH), version, str(source), str(serve)],
            capture_output=True,
            text=True,
        )

    def installer_environment(self, base, shimmed=True):
        env = dict(os.environ)
        env["HOME"] = str(self.home)
        env["ORBIT_APPLICATIONS_DIR"] = str(self.applications)
        env["ORBIT_INSTALL_BASE"] = base
        # This machine may be running another copy of these very tests, or a
        # Rust one with synthetic bundle processes of its own. Only the ones
        # this fixture started are this fixture's business.
        env["ORBIT_PS_ROOT"] = str(self.work)
        if shimmed:
            env["PATH"] = str(self.bin) + os.pathsep + env["PATH"]
        return env

    def install(self, base, shimmed=True):
        return subprocess.run(
            ["bash", str(INSTALL)],
            capture_output=True,
            text=True,
            env=self.installer_environment(base, shimmed),
        )


class InstallTests(Fixture):
    def test_a_platform_that_is_not_macos_is_refused_before_downloading(self):
        result = self.install("http://127.0.0.1:1", shimmed=False)
        self.assertEqual(result.returncode, 1)
        self.assertIn("macOS", result.stderr)

    def test_a_verified_package_lands(self):
        app = self.applications / "Orbit.app"
        self.publish(self.build_package())
        server = Server(self.work / "serve")
        self.addCleanup(server.close)
        result = self.install(server.url)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((app / "Contents" / "MacOS" / "Orbit").is_file())
        self.assertFalse((app / "Contents" / "Resources" / "orbit").exists())


    def test_what_was_installed_is_recorded_with_data_not_inside_the_app(self):
        self.publish(self.build_package())
        server = Server(self.work / "serve")
        self.addCleanup(server.close)
        self.assertEqual(self.install(server.url).returncode, 0)
        record = self.home / "Library/Application Support/com.MarsGotta.Orbit/.orbit-instalado"
        self.assertTrue(record.is_file())
        package = next((self.work / "serve" / "packages").glob("*.zip"))
        fields = dict(
            line.split("=", 1)
            for line in record.read_text().splitlines()
            if "=" in line
        )
        self.assertEqual(fields["sha256"], digest(package))
        self.assertEqual(fields["version"], "0.1.0")
        # Writing inside the bundle would break its signature.
        app = self.applications / "Orbit.app"
        self.assertEqual(list(app.rglob(".orbit-instalado")), [])

    def test_a_package_that_does_not_match_its_checksum_is_never_installed(self):
        self.publish(self.build_package())
        serve = self.work / "serve"
        package = next((serve / "packages").glob("*.zip"))
        expected = digest(package)
        package.write_bytes(package.read_bytes() + b"tampered")
        actual = digest(package)
        server = Server(serve)
        self.addCleanup(server.close)
        result = self.install(server.url)
        self.assertEqual(result.returncode, 1)
        self.assertIn("checksum", result.stderr)
        self.assertIn(expected, result.stderr)
        self.assertIn(actual, result.stderr)
        self.assertFalse((self.applications / "Orbit.app").exists())


    def test_previous_copies_wait_for_the_new_app_to_open_the_store(self):
        # A user had three stale bundles in ~/Applications after three updates,
        # 25 MB each, and macOS lists every one of them under Privacy as its own
        # app: a permission granted to a copy that no longer runs reads as a
        # permission that is missing.
        applications = self.applications
        stale = applications / "Orbit.app.anterior-20260101000000" / "Contents"
        stale.mkdir(parents=True)
        (stale / "marker").write_bytes(b"stale")
        previous = applications / "Orbit.app" / "Contents" / "Resources"
        previous.mkdir(parents=True)
        (previous / "orbit").write_bytes(b"previous-version")
        self.publish(self.build_package(b"new-version"))
        server = Server(self.work / "serve")
        self.addCleanup(server.close)

        self.assertEqual(self.install(server.url).returncode, 0)

        kept = list(applications.glob("Orbit.app.anterior-*"))
        self.assertEqual(len(kept), 2)
        self.assertEqual((stale / "marker").read_bytes(), b"stale")
        latest = [bundle for bundle in kept if bundle != stale.parent]
        self.assertEqual(len(latest), 1)
        self.assertEqual(
            (latest[0] / "Contents" / "Resources" / "orbit").read_bytes(),
            b"previous-version",
        )

    def test_the_installer_is_published_with_the_packages_it_installs(self):
        # The served installer belongs to the same source as the package.
        self.publish(self.build_package())
        served = self.work / "serve" / "install.sh"
        self.assertTrue(served.is_file(), "the installer is not published")
        source = INSTALL
        self.assertEqual(served.read_bytes(), source.read_bytes())


    def test_a_manifest_naming_a_path_is_refused_instead_of_followed(self):
        self.publish(self.build_package())
        serve = self.work / "serve"
        lines = (serve / "latest.txt").read_text().splitlines()
        rewritten = [
            "arm64_file=../../../../etc/passwd"
            if line.startswith("arm64_file=")
            else line
            for line in lines
        ]
        (serve / "latest.txt").write_text(EOL.join(rewritten) + EOL)
        server = Server(serve)
        self.addCleanup(server.close)
        result = self.install(server.url)
        self.assertEqual(result.returncode, 1)
        self.assertIn("paquete utilizable", result.stderr)

    def test_an_unreachable_origin_says_so_and_installs_nothing(self):
        result = self.install("http://127.0.0.1:1")
        self.assertEqual(result.returncode, 1)
        self.assertFalse((self.applications).exists())


class PublishTests(Fixture):
    def test_a_published_version_cannot_be_replaced_with_other_contents(self):
        self.assertEqual(self.publish(self.build_package(b"first")).returncode, 0)
        result = self.publish(self.build_package(b"second"))
        self.assertEqual(result.returncode, 1)
        self.assertIn("raise the version", result.stderr)

    def test_republishing_the_identical_package_is_allowed(self):
        source = self.build_package()
        self.assertEqual(self.publish(source).returncode, 0)
        self.assertEqual(self.publish(source).returncode, 0)

    def test_a_declared_checksum_that_does_not_match_stops_the_publication(self):
        source = self.build_package()
        next(source.glob("*.zip.sha256")).write_text("0" * 64 + "  package")
        result = self.publish(source)
        self.assertEqual(result.returncode, 1)
        self.assertIn("sha256 expected='" + "0" * 64, result.stderr)
        self.assertIn("found='" + digest(next(source.glob("*.zip"))), result.stderr)


    def test_a_version_that_is_not_a_plain_name_is_refused(self):
        source = self.build_package()
        for bad in ["../escape", "with space", ""]:
            self.assertEqual(self.publish(source, version=bad).returncode, 1, bad)

    def test_a_source_without_packages_publishes_nothing(self):
        empty = self.work / "empty"
        empty.mkdir()
        result = self.publish(empty)
        self.assertEqual(result.returncode, 1)
        self.assertFalse((self.work / "serve" / "latest.txt").exists())


if __name__ == "__main__":
    unittest.main()
