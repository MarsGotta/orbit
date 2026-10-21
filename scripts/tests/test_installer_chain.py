"""The installer counts the chain, not everything inside the bundle.

Updating to 0.3.8 failed with Orbit closed. `pgrep -f "Orbit.app/Contents"`
matches any process whose command line merely contains that path, and the
engine ships inside Orbit.app — so the engine was counted as Orbit. Orbit
starts the engine itself, so this would have happened on every update made
with control enabled.

The chain is the app and its native helper, each identified by the executable
it runs. The platform tools are stood in for as in
test_install_publish: `ps` reports the executable of every process the way macOS
`ps -o comm=` does. What this does NOT prove is anything about macOS itself —
that its `ps` prints the path, that the real bundle runs these executables, and
that a real update goes through — which needs the machine.
"""

import subprocess
import unittest

from test_install_publish import Fixture, Server

APP = "Orbit.app/Contents/MacOS/Orbit"
HELPER = "Orbit.app/Contents/MacOS/OrbitNative"
ENGINE = "Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver"


class ChainFixture(Fixture):
    def setUp(self):
        super().setUp()
        self.watch_real_processes()

    def serve_a_package(self):
        self.publish(self.build_package(b"version-nueva"))
        server = Server(self.work / "serve")
        self.addCleanup(server.close)
        return server.url

    def assert_installed(self, result):
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(
            (self.applications / "Orbit.app" / "Contents" / "MacOS" / "Orbit").read_bytes(),
            b"version-nueva",
        )


class InstallerChainTests(ChainFixture):
    def test_the_engine_inside_the_bundle_does_not_stop_the_install(self):
        # The measured failure: Orbit closed, the engine alive, nothing else.
        engine = self.bundle_process(ENGINE)
        url = self.serve_a_package()

        result = self.install(url)

        self.assert_installed(result)
        # And the installer never touched it: stopping the engine is Orbit's
        # job, not the installer's.
        self.assertIsNone(engine.poll())

    def test_a_process_that_merely_mentions_the_bundle_is_not_orbit(self):
        # `tail` is not part of the chain, whatever file it is following.
        watched = self.work / "runtime" / "Orbit.app" / "Contents" / "Resources"
        watched.mkdir(parents=True)
        log = watched / "registro.txt"
        log.write_text("", encoding="utf-8")
        follower = subprocess.Popen(
            ["tail", "-f", str(log)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        self.addCleanup(follower.wait)
        self.addCleanup(follower.terminate)
        url = self.serve_a_package()

        self.assert_installed(self.install(url))

    def test_a_live_helper_still_refuses_and_the_message_names_it(self):
        self.bundle_process(HELPER)
        url = self.serve_a_package()

        result = self.install(url)

        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("Orbit está en marcha", result.stderr)
        self.assertIn("su ayudante nativo", result.stderr)
        # The refusal still names a way out when no icon is left to close.
        self.assertIn("Monitor de Actividad", result.stderr)
        self.assertFalse((self.applications / "Orbit.app").exists())

    def test_the_open_app_is_named_as_what_it_is(self):
        self.bundle_process(APP)
        url = self.serve_a_package()

        result = self.install(url)

        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("la aplicación", result.stderr)

    def test_a_ps_that_answers_nothing_does_not_install_blind(self):
        # Not knowing whether Orbit is running is not the same as knowing it is
        # closed, and replacing a bundle under a live app is what this prevents.
        silent = self.bin / "ps"
        silent.write_text("#!/bin/bash\nexit 0\n", encoding="utf-8")
        silent.chmod(0o755)
        url = self.serve_a_package()

        result = self.install(url)

        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("ps", result.stderr)
        self.assertFalse((self.applications / "Orbit.app").exists())


if __name__ == "__main__":
    unittest.main()
