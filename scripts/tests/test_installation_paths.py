"""Installation paths and migration use only temporary directories."""

import os
import subprocess

from test_install_publish import INSTALL, Fixture, Server


class SystemInstallationTests(Fixture):
    def setUp(self):
        super().setUp()
        self.system = self.work / "system-applications"
        self.record = self.home / "Library/Application Support/com.MarsGotta.Orbit/.orbit-instalado"

    def installer_environment(self, base, shimmed=True):
        return {
            **super().installer_environment(base, shimmed),
            "ORBIT_APPLICATIONS_DIR": str(self.system),
        }

    def origin(self):
        self.publish(self.build_package(b"new"))
        server = Server(self.work / "serve")
        self.addCleanup(server.close)
        return server.url

    def test_default_destination_is_applications_without_writing_to_it(self):
        functions = self.work / "functions.sh"
        source = INSTALL.read_text()
        self.assertTrue(source.endswith('main "$@"\n'))
        functions.write_text(source.removesuffix('main "$@"\n'))
        env = {**os.environ, "HOME": str(self.home), "ORBIT_INSTALL_BASE": "https://updates.example.test"}
        env.pop("ORBIT_APPLICATIONS_DIR", None)
        result = subprocess.run(
            ["bash", "-c", 'source "$1"; printf "%s" "$APP_DIR"', "test", str(functions)],
            env=env, text=True, capture_output=True, timeout=5, check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "/Applications")

    def test_installs_in_system_applications_and_records_in_data(self):
        result = self.install(self.origin())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.system / "Orbit.app/Contents/MacOS/Orbit").read_bytes(), b"new")
        self.assertIn("version=0.1.0", self.record.read_text())
        self.assertFalse((self.system / ".orbit-instalado").exists())
        self.assertFalse((self.home / "Applications/Orbit.app").exists())

    def test_unwritable_destination_refuses_without_falling_back(self):
        # Also deterministic as root: the system directory is a regular file.
        self.system.write_text("not a folder")
        result = self.install(self.origin())
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(str(self.system), result.stderr)
        self.assertIn("estado", result.stderr)
        self.assertFalse((self.home / "Applications/Orbit.app").exists())

    def test_first_move_preserves_rollback_and_unrelated_apps_until_startup(self):
        legacy = self.home / "Applications"
        for directory in (legacy, self.system):
            for name in ("Expo Orbit.app", "Orbit Pruebas.app", "Orbit.app.anterior-", "Orbit.app.anterior-vieja"):
                bundle = directory / name
                bundle.mkdir(parents=True)
                (bundle / "marker").write_text(name)
        old = legacy / "Orbit.app"
        old.mkdir()
        (old / "marker").write_text("anterior")
        result = self.install(self.origin())
        self.assertEqual(result.returncode, 0, result.stderr)
        # Installation alone is not proof that the new daemon opened the store.
        self.assertEqual((old / "marker").read_text(), "anterior")
        for directory in (legacy, self.system):
            for name in ("Expo Orbit.app", "Orbit Pruebas.app", "Orbit.app.anterior-", "Orbit.app.anterior-vieja"):
                self.assertEqual((directory / name / "marker").read_text(), name)
