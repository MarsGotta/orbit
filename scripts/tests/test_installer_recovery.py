"""Installer recovery with synthetic macOS tools and an isolated HOME."""

import subprocess
from pathlib import Path

from test_install_publish import INSTALL, Server
from test_install_relaunch import RelaunchFixture


class RecoveryTests(RelaunchFixture):
    def setUp(self):
        super().setUp()
        self.synthetic_empty_listing()
        previous = self.applications / "Orbit.app/Contents/MacOS"
        previous.mkdir(parents=True)
        (previous / "Orbit").write_bytes(b"version-anterior")
        self.app = self.applications / "Orbit.app"
        self.log = self.home / "Library/Logs/Orbit/instalador.log"

    def shim(self, name, body):
        path = self.bin / name
        path.write_text("#!/bin/bash\n" + body)
        path.chmod(0o755)

    def origin(self):
        self.publish(self.build_package(b"version-nueva"))
        server = Server(self.work / "serve")
        self.addCleanup(server.close)
        return server.url

    def run_relaunch(self, origin, ready=False):
        env = self.installer_environment(origin)
        env["ORBIT_OPEN_LOG"] = str(self.open_log)
        if ready:
            env["ORBIT_INSTALL_READY_SIGNAL"] = "ORBIT_INSTALL_READY"
        return subprocess.run(
            ["bash", "--posix", str(INSTALL), "--relaunch"],
            capture_output=True, text=True, env=env, timeout=15,
        )

    def test_p1_unwritable_log_does_not_prevent_reopening_after_download_failure(self):
        origin = self.origin()
        self.shim("curl", """for arg in "$@"; do
  case "$arg" in */packages/*)
    rm "$HOME/Library/Logs/Orbit/instalador.log"
    mkdir "$HOME/Library/Logs/Orbit/instalador.log"
    exit 23;;
  esac
done
exec /usr/bin/curl "$@"
""")
        result = self.run_relaunch(origin)
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(self.open_log.exists(), result.stderr)
        self.assertEqual((self.app / "Contents/MacOS/Orbit").read_bytes(), b"version-anterior")

    def test_unwritable_log_from_start_does_not_skip_installation(self):
        origin = self.origin()
        self.log.mkdir(parents=True)
        self.shim("curl", 'touch "$HOME/curl-called"\nexec /usr/bin/curl "$@"\n')
        result = self.run_relaunch(origin)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.home / "curl-called").exists())
        self.assertEqual((self.app / "Contents/MacOS/Orbit").read_bytes(), b"version-nueva")

    def test_p2_unhandled_shasum_failure_records_command_and_reopens(self):
        origin = self.origin()
        self.shim("shasum", "exit 17\n")
        result = self.run_relaunch(origin)
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(self.open_log.exists(), result.stderr)
        self.assertIn("shasum", self.log.read_text())
        self.assertIn("(shasum: estado 17)", self.log.read_text())
        self.assertIn("|fallo|", self.log.read_text())


    def test_p4_unwritable_log_after_parking_never_leaves_app_absent(self):
        origin = self.origin()
        self.shim("mv", """/bin/mv "$@" || exit $?
case "$2" in */Orbit.app.anterior-*)
  rm "$HOME/Library/Logs/Orbit/instalador.log"
  mkdir "$HOME/Library/Logs/Orbit/instalador.log";;
esac
""")
        result = self.run_relaunch(origin)
        self.assertTrue(self.app.exists(), result.stderr)
        self.assertTrue(self.open_log.exists(), result.stderr)

    def test_p5_failure_after_ready_reopens_even_before_chain_closed(self):
        self.shim("ps", "exit 0\n")
        result = self.run_relaunch("http://127.0.0.1:1", ready=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ORBIT_INSTALL_READY", result.stdout)
        self.assertTrue(self.open_log.exists(), result.stderr)
        self.assertIn("|fallo|", self.log.read_text())

    def test_p6_invalid_version_cannot_forge_success_result(self):
        origin = self.origin()
        manifest = self.work / "serve/latest.txt"
        manifest.write_text(manifest.read_text().replace(
            "version=0.1.0", "version=9.9.9|2026-01-01T00:00:00Z|bien|instalado"
        ))
        result = self.run_relaunch(origin)
        self.assertNotEqual(result.returncode, 0)
        results = [line for line in self.log.read_text().splitlines() if line.startswith("resultado|")]
        self.assertTrue(results)
        self.assertTrue(all("|bien|" not in line for line in results), results)
        self.assertTrue(all(line.startswith("resultado|desconocida|") for line in results), results)

    def test_p7_failed_open_keeps_installed_app_and_matching_record(self):
        origin = self.origin()
        self.shim("open", 'printf "%s\\n" "$@" >> "$ORBIT_OPEN_LOG"\nexit 21\n')
        result = self.run_relaunch(origin)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((self.app / "Contents/MacOS/Orbit").read_bytes(), b"version-nueva")
        self.assertIn("version=0.1.0", (self.home / "Library/Application Support/com.MarsGotta.Orbit/.orbit-instalado").read_text())
        results = [line for line in self.log.read_text().splitlines() if line.startswith("resultado|")]
        self.assertIn("|fallo|", results[-1])
        self.assertIn("abrir", results[-1])
        self.assertIn("(open: estado 21)", results[-1])

    def test_p7b_failed_open_is_not_retried_by_exit_trap(self):
        origin = self.origin()
        self.shim("open", '''printf '%s\\n' "$@" >> "$ORBIT_OPEN_LOG"
if [ ! -e "$ORBIT_OPEN_LOG.once" ]; then
  touch "$ORBIT_OPEN_LOG.once"
  exit 1
fi
exit 0
''')
        result = self.run_relaunch(origin)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.open_log.read_text().count("\n"), 2)
        results = [line for line in self.log.read_text().splitlines() if line.startswith("resultado|")]
        self.assertIn("|fallo|", results[-1])
        self.assertIn("(open: estado 1)", results[-1])
        self.assertNotIn("Orbit vuelto a abrir tras el fallo", self.log.read_text())

    def test_t1_failed_restore_keeps_new_app_and_reports_both_locations(self):
        origin = self.origin()
        self.shim("unzip", '''/usr/bin/unzip "$@" || exit $?
for arg in "$@"; do last="$arg"; done
rm -f "$last/Orbit.app/Contents/MacOS/Orbit"
''')
        self.shim("mv", '''case "$1" in */Orbit.app.anterior-*) exit 5;; esac
exec /bin/mv "$@"
''')
        result = self.run_relaunch(origin)
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(self.open_log.exists(), result.stderr)
        parked = list((self.applications).glob("Orbit.app.anterior-*"))
        self.assertEqual(len(parked), 1)
        results = [line for line in self.log.read_text().splitlines() if line.startswith("resultado|")]
        self.assertIn("restauración falló (mv: estado 5)", results[-1])
        self.assertIn(f"anterior: {parked[0]}", results[-1])
        new_app = results[-1].split("nueva: ", 1)[1].split("; estado", 1)[0]
        self.assertTrue(Path(new_app).is_dir(), results[-1])
        self.assertIn(str(parked[0]), self.open_log.read_text())

    def test_t2_failed_placement_and_restore_name_the_new_app_where_it_is(self):
        origin = self.origin()
        self.shim("mv", '''case "$1" in
  */unpacked/Orbit.app) exit 9;;
  */Orbit.app.anterior-*) exit 5;;
esac
exec /bin/mv "$@"
''')
        result = self.run_relaunch(origin)
        self.assertNotEqual(result.returncode, 0)
        parked = list((self.applications).glob("Orbit.app.anterior-*"))
        self.assertEqual(len(parked), 1)
        results = [line for line in self.log.read_text().splitlines() if line.startswith("resultado|")]
        self.assertIn("restauración falló (mv: estado 5)", results[-1])
        self.assertIn(f"anterior: {parked[0]}", results[-1])
        new_app = results[-1].split("nueva: ", 1)[1].split("; estado", 1)[0]
        self.assertTrue(Path(new_app).is_dir(), results[-1])

    def test_origin_download_failure_carries_curl_status(self):
        self.shim("curl", "exit 22\n")
        result = self.run_relaunch("http://127.0.0.1:1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("(curl: estado 22)", self.log.read_text())

    def test_package_download_failure_carries_curl_status(self):
        origin = self.origin()
        self.shim("curl", """for arg in "$@"; do
  case "$arg" in */packages/*) exit 23;; esac
done
exec /usr/bin/curl "$@"
""")
        result = self.run_relaunch(origin)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("(curl: estado 23)", self.log.read_text())

    def test_unzip_failure_carries_status(self):
        origin = self.origin()
        self.shim("unzip", "exit 7\n")
        result = self.run_relaunch(origin)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("(unzip: estado 7)", self.log.read_text())

    def test_bundle_move_failure_carries_status(self):
        origin = self.origin()
        self.shim("mv", """case "$1" in */unpacked/Orbit.app) exit 9;; esac
exec /bin/mv "$@"
""")
        result = self.run_relaunch(origin)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("(mv: estado 9)", self.log.read_text())


if __name__ == "__main__":
    import unittest
    unittest.main()
