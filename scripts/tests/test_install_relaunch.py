"""APP-09 A1: relaunch waits for the bundle, installs, and opens Orbit.

The macOS-only commands are replaced with the same fixtures as the installer
tests next door.  The synthetic bundle process is real: the ps stand-in reports
the executable it runs, and the test releases it only after the installer has
observed it.
"""

import stat
import subprocess
import time
import unittest

from test_install_publish import INSTALL, PS, Fixture, Server

OPEN = """#!/bin/bash
if [ -f "$HOME/Library/Logs/Orbit/instalador.log" ]; then
  cp "$HOME/Library/Logs/Orbit/instalador.log" "$ORBIT_OPEN_LOG.log-at-open"
fi
printf '%s\\n' "$@" >> "$ORBIT_OPEN_LOG"
"""

WAIT_FOR_MARKER = (
    "#!/usr/bin/env python3\n"
    "import pathlib, sys, time\n"
    "marker = pathlib.Path(sys.argv[1])\n"
    "while not marker.exists():\n"
    "    time.sleep(0.01)\n"
)


class RelaunchFixture(Fixture):
    def setUp(self):
        super().setUp()
        self.open_log = self.work / "open.log"
        self.ps_seen = self.work / "ps-seen"
        self.watch_real_processes()
        shim = self.bin / "open"
        shim.write_text(OPEN)
        shim.chmod(shim.stat().st_mode | stat.S_IEXEC)

    def install_command(self, base, *arguments):
        env = self.installer_environment(base)
        env["ORBIT_OPEN_LOG"] = str(self.open_log)
        env["ORBIT_PS_SEEN"] = str(self.ps_seen)
        return subprocess.Popen(
            ["bash", str(INSTALL), *arguments],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=env,
        )

    def synthetic_empty_listing(self):
        # These cases need no running bundle. Do not enumerate the host.
        shim = self.bin / "ps"
        shim.write_text(PS)
        shim.chmod(shim.stat().st_mode | stat.S_IEXEC)

    def start_bundle_process(self):
        release = self.work / "release-bundle-process"
        process = self.bundle_process(
            "Orbit.app/Contents/MacOS/Orbit",
            WAIT_FOR_MARKER,
            str(release),
        )
        return process, release

    def wait_until_installer_sees_process(self):
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if self.ps_seen.exists():
                return
            time.sleep(0.01)
        self.fail("el instalador no comprobó los procesos del bundle")


class InstallRelaunchTests(RelaunchFixture):
    def test_ref502b_a3_log_rotates_before_a_full_installation(self):
        self.synthetic_empty_listing()
        self.publish(self.build_package())
        server = Server(self.work / "serve")
        self.addCleanup(server.close)
        log = self.home / "Library" / "Logs" / "Orbit" / "instalador.log"
        log.parent.mkdir(parents=True)
        log.write_bytes(b"x" * 70_000)
        install = self.install_command(server.url)
        stdout, stderr = install.communicate(timeout=10)
        self.assertEqual(install.returncode, 0, stdout + stderr)
        content = log.read_bytes()
        self.assertLess(len(content), 65_536)
        self.assertTrue(content.startswith(b"x" * 32_768))
        self.assertIn(b"resultado|0.1.0|", content)

    def test_ref502b_a3_detached_posix_installer_records_result(self):
        self.synthetic_empty_listing()
        self.publish(self.build_package(b"version-nueva"))
        server = Server(self.work / "serve")
        self.addCleanup(server.close)
        env = self.installer_environment(server.url)
        env["ORBIT_OPEN_LOG"] = str(self.open_log)
        launcher = subprocess.run(
            ["bash", "-c", 'bash --posix "$1" --relaunch </dev/null >/dev/null 2>&1 & echo $!', "--", str(INSTALL)],
            capture_output=True, text=True, env=env, timeout=5,
        )
        self.assertEqual(launcher.returncode, 0, launcher.stderr)
        self.assertTrue(launcher.stdout.strip().isdigit(), launcher.stdout)
        log = self.home / "Library" / "Logs" / "Orbit" / "instalador.log"
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if log.exists() and "resultado|0.1.0|" in log.read_text() and "|bien|" in log.read_text():
                break
            time.sleep(0.01)
        else:
            self.fail("el instalador sin padre y sin salida no registró el resultado")
        self.assertIn("1/4 · Buscando la versión publicada", log.read_text())
        self.assertLess(log.stat().st_size, 2048)
        print(f"REF502B_LOG_BYTES={log.stat().st_size}")

    def test_ref502b_a4_reopens_after_download_failure(self):
        self.synthetic_empty_listing()
        previous = self.applications / "Orbit.app" / "Contents" / "MacOS"
        previous.mkdir(parents=True)
        (previous / "Orbit").write_bytes(b"version-anterior")
        install = self.install_command("http://127.0.0.1:1", "--relaunch")
        stdout, stderr = install.communicate(timeout=10)
        self.assertNotEqual(install.returncode, 0, stdout + stderr)
        self.assertTrue(self.open_log.is_file(), "no se volvió a abrir Orbit tras fallar la descarga")
        self.assertIn(
            "|fallo|",
            (self.work / "open.log.log-at-open").read_text(),
            "Orbit se abrió antes de que constase el fallo",
        )

    def test_ref502b_a4_restores_and_reopens_after_move_failure(self):
        self.synthetic_empty_listing()
        previous = self.applications / "Orbit.app" / "Contents" / "MacOS"
        previous.mkdir(parents=True)
        (previous / "Orbit").write_bytes(b"version-anterior")
        self.publish(self.build_package(b"version-nueva"))
        server = Server(self.work / "serve")
        self.addCleanup(server.close)
        move = self.bin / "mv"
        move.write_text(
            '#!/bin/bash\nif [[ "$2" == */Orbit.app && "$1" == */unpacked/Orbit.app ]]; then exit 9; fi\nexec /bin/mv "$@"\n'
        )
        move.chmod(0o755)
        install = self.install_command(server.url, "--relaunch")
        stdout, stderr = install.communicate(timeout=10)
        self.assertNotEqual(install.returncode, 0, stdout + stderr)
        self.assertEqual((previous / "Orbit").read_bytes(), b"version-anterior")
        self.assertTrue(self.open_log.is_file(), "no se volvió a abrir la versión anterior")
        self.assertIn(
            "|fallo|",
            (self.work / "open.log.log-at-open").read_text(),
            "Orbit se abrió antes de que constase el fallo",
        )


    def test_a1_relaunch_waits_for_the_bundle_then_installs_and_opens_orbit(self):
        self.publish(self.build_package(b"version-nueva"))
        server = Server(self.work / "serve")
        self.addCleanup(server.close)
        process, release = self.start_bundle_process()

        install = self.install_command(server.url, "--relaunch")
        self.wait_until_installer_sees_process()
        self.assertIsNone(process.poll(), "el instalador mató el proceso del bundle")
        release.touch()
        process.wait(timeout=5)
        stdout, stderr = install.communicate(timeout=10)

        self.assertEqual(install.returncode, 0, stdout + stderr)
        installed = self.applications / "Orbit.app"
        self.assertTrue(installed.is_dir())
        self.assertTrue(self.open_log.is_file(), "el instalador no llamó a open")
        self.assertIn(
            "|bien|",
            (self.work / "open.log.log-at-open").read_text(),
            "Orbit se abrió antes de que constase la instalación",
        )
        invocation = self.open_log.read_text().splitlines()
        self.assertGreaterEqual(len(invocation), 2)
        self.assertEqual(invocation[0], "-a")
        self.assertIn(invocation[1], ("Orbit", str(installed)))

    def test_relaunch_checks_again_before_installing_over_a_live_app(self):
        self.publish(self.build_package(b"version-nueva"))
        server = Server(self.work / "serve")
        self.addCleanup(server.close)
        ready = self.work / "app-ready"
        release = self.work / "release-app"
        calls = self.work / "ps-calls"
        process = self.bundle_process(
            "Orbit.app/Contents/MacOS/Orbit",
            "#!/usr/bin/env python3\n"
            "import pathlib, sys, time\n"
            "ready, release = map(pathlib.Path, sys.argv[1:])\n"
            "ready.touch()\n"
            "while not release.exists():\n"
            "    time.sleep(0.01)\n",
            str(ready), str(release),
        )
        # Only a second process check lets the app exit. Skipping the wait
        # loop therefore completes the installation with the app still alive.
        observer = self.bin / "ps_shim.py"
        observer.write_text(
            "import pathlib, time\n"
            f"ready = pathlib.Path({str(ready)!r})\n"
            f"release = pathlib.Path({str(release)!r})\n"
            f"calls = pathlib.Path({str(calls)!r})\n"
            "while not ready.exists():\n"
            "    time.sleep(0.01)\n"
            "with calls.open('a') as handle:\n"
            "    handle.write('ps\\n')\n"
            "if len(calls.read_text().splitlines()) >= 2:\n"
            "    release.touch()\n"
            + observer.read_text()
        )

        install = self.install_command(server.url, "--relaunch")
        try:
            stdout, stderr = install.communicate(timeout=10)
        finally:
            if install.poll() is None:
                install.kill()
                install.communicate(timeout=5)

        self.assertEqual(install.returncode, 0, stdout + stderr)
        self.assertGreaterEqual(
            len(calls.read_text().splitlines()), 2,
            "el instalador reemplazó el paquete sin volver a comprobar la app viva",
        )
        self.assertEqual(process.wait(timeout=5), 0)
        installed = self.applications / "Orbit.app" / "Contents" / "MacOS" / "Orbit"
        self.assertEqual(installed.read_bytes(), b"version-nueva")

    def test_a1_install_without_relaunch_does_not_open_orbit(self):
        self.publish(self.build_package())
        server = Server(self.work / "serve")
        self.addCleanup(server.close)

        install = self.install_command(server.url)
        stdout, stderr = install.communicate(timeout=10)

        self.assertEqual(install.returncode, 0, stdout + stderr)
        self.assertFalse(self.open_log.exists(), "open se ejecutó sin --relaunch")


if __name__ == "__main__":
    unittest.main()
