"""Activating a release copies the database first; going back puts the copy back.

A new release migrates the hub's database when it starts. Before the links
move, activate.sh takes a consistent copy of the database and its key, as the
release it leaves has them, in <base>/backups/activation. Going back to that
release (by `anterior` or by its number) puts the copy back when the schema
has changed since, with the hub stopped, and moves the newer database aside;
with the same schema the database stays and keeps everything written since.

Everything runs against hub_harness.py: a temporary base, fake systemd and
hermes, a fake hub, and a synthetic SQLite database in a temporary data
directory. The copy itself is the real server/orbit_hub/backup.py.
"""

import os
import sys
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))

from hub_harness import HUB_UNIT, RESTART_ORDER, HubSandbox, linux_only


@linux_only
class ActivationCopyTests(unittest.TestCase):
    def setUp(self):
        self.sandbox = HubSandbox()
        self.addCleanup(self.sandbox.close)
        for version in ("1.0.0", "1.1.0"):
            self.sandbox.make_release(version)
        self.backups = self.sandbox.base / "backups"
        self.copy = self.backups / "activation"

    def activate(self, *arguments):
        return self.sandbox.run("activate.sh", *arguments, timeout=600)

    def assertActivated(self, result):
        self.assertEqual(result.returncode, 0, f"{result.stdout}\n{result.stderr}")

    def systemctl_verbs(self):
        return [
            entry["argv"][1] for entry in self.sandbox.events("systemctl") if len(entry["argv"]) > 1
        ]

    def test_without_a_database_yet_the_activation_goes_on(self):
        result = self.activate("1.0.0")
        self.assertActivated(result)
        self.assertIn("there is no database at", result.stdout)
        self.assertFalse(self.backups.exists())

    def test_an_activation_first_copies_the_database_and_its_key_as_the_old_release_left_them(self):
        self.sandbox.make_database()
        self.assertActivated(self.activate("1.0.0"))
        self.sandbox.clear_events()
        result = self.activate("1.1.0")
        self.assertActivated(result)
        self.assertIn("as 1.0.0 left them", result.stdout)
        self.assertEqual(
            sorted(path.name for path in self.copy.iterdir()),
            ["FROM", "hub.sqlite3", "storage.key"],
        )
        self.assertIn("version=1.0.0\n", (self.copy / "FROM").read_text(encoding="utf-8"))
        self.assertEqual(
            (self.copy / "storage.key").read_bytes(), self.sandbox.key_file.read_bytes()
        )
        self.assertEqual(
            self.sandbox.database_rows(self.copy / "hub.sqlite3"), ["Acta de la Cooperativa Brezo"]
        )
        for path in self.copy.iterdir():
            self.assertEqual(path.stat().st_mode & 0o077, 0, path)
        # Copied before anything restarted; the hub never stopped.
        self.assertEqual(self.sandbox.restarts(), list(RESTART_ORDER))
        self.assertNotIn("stop", self.systemctl_verbs())

    def test_going_back_after_a_migration_puts_the_copy_back_and_sets_the_newer_database_aside(
        self,
    ):
        self.sandbox.make_database()
        self.assertActivated(self.activate("1.0.0"))
        self.assertActivated(self.activate("1.1.0"))
        # 1.1.0 starts, migrates and writes.
        self.sandbox.migrate_database()
        self.sandbox.make_database(rows=("Escrito con 1.1.0",))
        self.sandbox.clear_events()
        result = self.activate("anterior")
        self.assertActivated(result)
        self.assertEqual(os.readlink(self.sandbox.base / "current"), "releases/1.0.0")
        self.assertEqual(self.sandbox.database_rows(), ["Acta de la Cooperativa Brezo"])
        aside = [path for path in self.backups.iterdir() if path.name.startswith("set-aside-")]
        self.assertEqual(len(aside), 1, result.stdout)
        self.assertIn(str(aside[0]), result.stdout)
        self.assertEqual(
            self.sandbox.database_rows(aside[0] / "hub.sqlite3"),
            ["Acta de la Cooperativa Brezo", "Escrito con 1.1.0"],
        )
        # The hub was stopped before the copy went back, and started by the restarts.
        verbs = self.systemctl_verbs()
        self.assertLess(verbs.index("stop"), verbs.index("restart"))
        self.assertEqual(self.sandbox.restarts(), list(RESTART_ORDER))

    def test_going_back_by_number_is_going_back(self):
        self.sandbox.make_database()
        self.assertActivated(self.activate("1.0.0"))
        self.assertActivated(self.activate("1.1.0"))
        self.sandbox.migrate_database()
        self.assertActivated(self.activate("1.0.0"))
        self.assertTrue(any(path.name.startswith("set-aside-") for path in self.backups.iterdir()))

    def test_going_back_with_the_same_schema_keeps_everything_written_since(self):
        self.sandbox.make_database()
        self.assertActivated(self.activate("1.0.0"))
        self.assertActivated(self.activate("1.1.0"))
        self.sandbox.make_database(rows=("Escrito con 1.1.0",))
        inode = (self.sandbox.data_dir / "hub.sqlite3").stat().st_ino
        self.sandbox.clear_events()
        result = self.activate("anterior")
        self.assertActivated(result)
        self.assertIn("keeps everything written since", result.stdout)
        self.assertEqual((self.sandbox.data_dir / "hub.sqlite3").stat().st_ino, inode)
        self.assertEqual(
            self.sandbox.database_rows(), ["Acta de la Cooperativa Brezo", "Escrito con 1.1.0"]
        )
        self.assertNotIn("stop", self.systemctl_verbs())

    def test_restarting_the_same_release_keeps_the_copy_of_the_one_before(self):
        self.sandbox.make_database()
        self.assertActivated(self.activate("1.0.0"))
        self.assertActivated(self.activate("1.1.0"))
        self.sandbox.migrate_database()
        self.assertActivated(self.activate("1.1.0"))
        self.assertIn("version=1.0.0\n", (self.copy / "FROM").read_text(encoding="utf-8"))

    def test_a_copy_that_cannot_be_taken_stops_the_activation_before_anything_moves(self):
        self.sandbox.make_database()
        self.assertActivated(self.activate("1.0.0"))
        self.sandbox.key_file.unlink()  # the copy takes the key with it
        self.sandbox.clear_events()
        result = self.activate("1.1.0")
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("could not copy the database before activating 1.1.0", result.stderr)
        self.assertIn("Nothing was touched", result.stderr)
        self.assertEqual(os.readlink(self.sandbox.base / "current"), "releases/1.0.0")
        self.assertEqual(self.sandbox.restarts(), [])
        self.assertFalse((self.backups / ".activation.new").exists())

    def test_a_unit_without_a_configuration_stops_the_activation(self):
        self.sandbox.make_database()
        self.sandbox.set_unit(
            HUB_UNIT,
            ExecStart=self.sandbox.default_units()[HUB_UNIT]["ExecStart"].replace(
                f"--config {self.sandbox.root / 'hub.toml'}", ""
            ),
        )
        result = self.activate("1.0.0")
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("names no --config", result.stderr)
        self.assertFalse((self.sandbox.base / "current").exists())
        self.assertEqual(self.sandbox.restarts(), [])


if __name__ == "__main__":
    unittest.main()
