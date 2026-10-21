"""Activating a release is one deliberate, atomic step, and so is going back.

`current` is a symlink the unit and the Deimos profile both run through. Moving
it has to be a single rename, so there is no instant with no hub to start; then
the hub, hermes-serve-orbit and the gateway restart in that order, and nothing
after the hub is touched until the hub answers /health as the version just
activated. When something fails the operator is told which service, with what
output, its status and its journal, and the one command that goes back.

Hermes runs one gateway per host, restarted with
`hermes -p default gateway restart`. Its PID did not change on 2026-10-15 and
its answer was "not confirmed … still pending" when it worked; those older
observations remain regression cases. A new orbit-mcp child of the gateway
whose interpreter is the release's proves it took that release.

Everything runs against hub_harness.py: a temporary base, a fake systemctl,
journalctl and hermes that only record, a synthetic process listing for ps, and
a fake hub on loopback. The releases here are synthetic (the shape activate.sh
checks); test_hub_publish.py builds real ones. Nothing here proves the real
services restart in this order on the server: that is an activation, which its
operator runs.
"""

import os
import sys
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))

from hub_harness import (  # noqa: E402
    DEIMOS_RESTART_OUTPUT,
    DEVELOPMENT_EXEC_START,
    DEVELOPMENT_WORKING_DIRECTORY,
    GATEWAY_PID,
    GATEWAY_RESTART,
    GATEWAY_RESTART_ARGV,
    GATEWAY_STATUS_ARGV,
    HOST_STATUS_OUTPUT,
    HUB_UNIT,
    PENDING_RESTART_OUTPUT,
    RESTART_ORDER,
    RETIRED_GATEWAY_UNIT,
    SERVE_PID,
    SERVE_UNIT,
    HubSandbox,
    linux_only,
    systemd_exec,
    translator_args,
)

STATUS_MARK = "fake status for the tests"
JOURNAL_MARK = "fake journal line"


@linux_only
class ActivationTests(unittest.TestCase):
    def setUp(self):
        self.sandbox = HubSandbox()
        self.addCleanup(self.sandbox.close)
        self.sandbox.make_release("1.0.0")
        self.sandbox.make_release("1.1.0")
        self.current = self.sandbox.base / "current"
        self.previous = self.sandbox.base / "previous"

    def activate(self, *arguments, **env):
        return self.sandbox.run("activate.sh", *arguments, env=self.sandbox.env(**env), timeout=600)

    def assertActivated(self, result):
        self.assertEqual(result.returncode, 0, f"{result.stdout}\n{result.stderr}")

    def test_hermes_refuses_a_profile_restart_without_replacing_the_translator(self):
        before = self.sandbox.gateway_translators()
        refused = self.sandbox.source_lib('"$ORBIT_HUB_HERMES" -p deimos gateway restart')
        self.assertNotEqual(refused.returncode, 0)
        self.assertEqual(refused.stdout.strip(), DEIMOS_RESTART_OUTPUT)
        self.assertEqual(self.sandbox.gateway_translators(), before)
        self.assertActivated(self.activate("1.0.0"))
        self.assertIn(translator_args(self.sandbox.base, "1.0.0"), self.sandbox.gateway_translators())

    # Activating ----------------------------------------------------------

    def test_current_points_to_the_activated_release(self):
        self.assertActivated(self.activate("1.0.0"))
        self.assertTrue(self.current.is_symlink())
        self.assertEqual(os.readlink(self.current), "releases/1.0.0")
        leftovers = [path.name for path in self.sandbox.base.iterdir() if path.name.startswith(".current")]
        self.assertEqual(leftovers, [])

    def test_current_changes_by_one_rename_and_is_never_missing(self):
        # A rename(2) over the old link is what makes the switch atomic: `ln -sfn`
        # unlinks first and leaves an instant with no hub to start. The shell
        # functions below stand in for mv, rm, ln and unlink around the one call
        # that switches the link, and fail if current is removed, relinked in
        # place, or already different before the rename.
        (self.sandbox.base / "current").symlink_to("releases/1.0.0")
        switched = self.sandbox.source_lib(
            """
renames=0
link="$ORBIT_HUB_BASE/current"
mv() {
  [[ "$(readlink "$link")" == releases/1.0.0 ]] || { echo "current changed before the rename" >&2; exit 70; }
  renames=$((renames + 1))
  command mv "$@"
}
rm() { local a; for a in "$@"; do [[ "$a" == "$link" ]] && { echo "rm on current" >&2; exit 71; }; done; command rm "$@"; }
ln() { [[ "${*: -1}" == "$link" ]] && { echo "ln straight on current" >&2; exit 72; }; command ln "$@"; }
unlink() { echo "unlink" >&2; exit 73; }
hub_point_link current releases/1.1.0
[[ $renames -eq 1 ]] || { echo "renames=$renames" >&2; exit 74; }
readlink "$link"
"""
        )
        self.assertEqual(switched.returncode, 0, switched.stderr)
        self.assertEqual(switched.stdout.strip(), "releases/1.1.0")

    def test_the_three_restart_in_order_and_hermes_waits_for_the_hub(self):
        self.assertActivated(self.activate("1.0.0"))
        self.assertEqual(self.sandbox.restarts(), list(RESTART_ORDER))
        events = self.sandbox.events()

        def position(predicate, what):
            for index, entry in enumerate(events):
                if predicate(entry):
                    return index
            self.fail(f"did not happen: {what}")

        def restarted(unit):
            return lambda e: e["tool"] == "systemctl" and e["argv"][1:2] == ["restart"] and e["argv"][-1] == unit

        hub = position(restarted(HUB_UNIT), "hub restart")
        healthy = position(lambda e: e["tool"] == "hub" and e["path"] == "/health" and e["status"] == 200, "/health 200")
        version = position(lambda e: e["tool"] == "hub" and e["path"] == "/openapi.json" and e["status"] == 200, "version")
        serve = position(restarted(SERVE_UNIT), "hermes-serve-orbit restart")
        gateway = position(
            lambda e: e["tool"] == "hermes" and e["argv"] == GATEWAY_RESTART_ARGV, "hermes -p default gateway restart"
        )
        self.assertLess(hub, healthy)
        self.assertLess(healthy, serve)
        self.assertLess(version, serve)
        self.assertLess(serve, gateway)

    def test_systemctl_and_hermes_use_the_user_manager_bus(self):
        self.assertActivated(self.activate("1.0.0"))
        calls = self.sandbox.events("systemctl")
        self.assertTrue(calls)
        uid = os.getuid()
        for call in calls:
            self.assertEqual(call["argv"][0], "--user", call)
        hermes = self.sandbox.events("hermes")
        self.assertTrue(hermes)
        for call in calls + hermes:
            self.assertEqual(call["env"]["XDG_RUNTIME_DIR"], f"/run/user/{uid}", call)
            self.assertEqual(call["env"]["DBUS_SESSION_BUS_ADDRESS"], f"unix:path=/run/user/{uid}/bus", call)

    def test_a_hub_that_never_answers_is_named_with_its_status_and_journal(self):
        self.assertActivated(self.activate("1.0.0"))
        self.sandbox.clear_events()
        self.sandbox.hub.answering = False
        failed = self.activate("1.1.0", ORBIT_HUB_HEALTH_TIMEOUT="2")
        self.assertNotEqual(failed.returncode, 0)
        self.assertIn(HUB_UNIT, failed.stderr)
        self.assertIn("/health", failed.stderr)
        self.assertIn("2 s", failed.stderr)
        self.assertIn("503", failed.stderr)
        self.assertIn(STATUS_MARK, failed.stderr)
        self.assertIn(JOURNAL_MARK, failed.stderr)
        self.assertIn("activate.sh 1.0.0", failed.stderr)
        self.assertEqual(self.sandbox.restarts(), [HUB_UNIT], "Hermes was restarted with the hub down")

    def test_a_hub_that_dies_on_start_is_reported_without_waiting_out_the_deadline(self):
        # Restart=always puts a dead hub in activating/auto-restart; the wait
        # stops on that fact at the first poll rather than on the deadline. Only
        # that early path produces this message: without it the script waits
        # the full 120 s and reports a hub that did not answer instead, which
        # fails here on the message, well inside the harness's own 600 s.
        self.sandbox.set_unit(HUB_UNIT, after_restart={"ActiveState": "activating", "SubState": "auto-restart"})
        self.sandbox.hub.answering = False
        failed = self.activate("1.0.0", ORBIT_HUB_HEALTH_TIMEOUT="120")
        self.assertNotEqual(failed.returncode, 0)
        self.assertIn("went down while starting", failed.stderr)
        self.assertIn("auto-restart", failed.stderr)
        self.assertIn(STATUS_MARK, failed.stderr)
        self.assertIn(JOURNAL_MARK, failed.stderr)
        self.assertEqual(self.sandbox.restarts(), [HUB_UNIT])

    def test_a_failed_restart_names_the_service_and_its_output(self):
        self.sandbox.set_unit(SERVE_UNIT, restart_exit=1, restart_output="Job for hermes-serve-orbit.service failed (fake)")
        failed = self.activate("1.0.0")
        self.assertNotEqual(failed.returncode, 0)
        self.assertIn(SERVE_UNIT, failed.stderr)
        self.assertIn("Job for hermes-serve-orbit.service failed (fake)", failed.stderr)
        self.assertIn("status 1", failed.stderr)
        self.assertIn(STATUS_MARK, failed.stderr)
        self.assertIn(JOURNAL_MARK, failed.stderr)
        self.assertEqual(self.sandbox.restarts(), [HUB_UNIT, SERVE_UNIT], "it went on after the failure")

    def test_a_hermes_that_is_not_active_after_its_restart_is_named(self):
        # The gateway was the unit this used to check; it is no unit any more,
        # so hermes-serve-orbit, the one Hermes still under systemd, carries it.
        self.sandbox.set_unit(SERVE_UNIT, after_restart={"ActiveState": "failed", "SubState": "failed"})
        failed = self.activate("1.0.0")
        self.assertNotEqual(failed.returncode, 0)
        self.assertIn(SERVE_UNIT, failed.stderr)
        self.assertIn("failed", failed.stderr)
        self.assertIn(JOURNAL_MARK, failed.stderr)
        self.assertEqual(self.sandbox.restarts(), [HUB_UNIT, SERVE_UNIT], "the gateway was restarted after the failure")

    def test_the_running_hub_must_be_the_version_just_activated(self):
        self.sandbox.hub.version = "0.0.1"
        failed = self.activate("1.0.0")
        self.assertNotEqual(failed.returncode, 0)
        self.assertIn("0.0.1", failed.stderr)
        self.assertIn("1.0.0", failed.stderr)
        self.assertEqual(self.sandbox.restarts(), [HUB_UNIT])

    def test_a_version_that_cannot_be_read_is_not_claimed(self):
        # /health is the criterion; the version is a second check. When the
        # hub cannot say which it is, activation goes on but must not print
        # that the hub "says it is" the version: that would be an untrue name.
        self.sandbox.hub.describes_itself = False
        activated = self.activate("1.0.0")
        self.assertActivated(activated)
        self.assertNotIn("says it is", activated.stdout)
        self.assertIn("could not be checked", activated.stdout)
        self.assertIn("openapi.json", activated.stderr)
        self.assertEqual(self.sandbox.restarts(), list(RESTART_ORDER))

    def test_a_unit_that_does_not_run_from_current_is_refused_before_anything_moves(self):
        for properties in (
            {"ExecStart": systemd_exec(DEVELOPMENT_EXEC_START)},
            {"WorkingDirectory": DEVELOPMENT_WORKING_DIRECTORY},
        ):
            with self.subTest(properties=next(iter(properties))):
                self.sandbox.write_units(self.sandbox.default_units())
                self.sandbox.set_unit(HUB_UNIT, **properties)
                self.sandbox.clear_events()
                refused = self.activate("1.0.0")
                self.assertNotEqual(refused.returncode, 0)
                self.assertIn(next(iter(properties)), refused.stderr)
                self.assertIn("docs/self-hosting.md", refused.stderr)
                self.assertFalse(self.current.exists() or self.current.is_symlink())
                self.assertEqual(self.sandbox.restarts(), [])

    def test_a_unit_systemd_has_not_reloaded_is_refused(self):
        self.sandbox.set_unit(HUB_UNIT, NeedDaemonReload="yes")
        refused = self.activate("1.0.0")
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("daemon-reload", refused.stderr)
        self.assertFalse(self.current.is_symlink())
        self.assertEqual(self.sandbox.restarts(), [])

    def test_what_is_not_a_complete_release_is_refused_and_the_real_ones_listed(self):
        self.sandbox.make_release("1.2.0", complete=False)
        for version in ("2.0.0", "1.2.0", "anterior", "../1.0.0", ""):
            with self.subTest(version=version):
                refused = self.activate(version)
                self.assertNotEqual(refused.returncode, 0)
                self.assertFalse(self.current.is_symlink())
        refused = self.activate("2.0.0")
        self.assertIn("1.0.0", refused.stderr)
        self.assertIn("1.1.0", refused.stderr)
        self.assertEqual(self.sandbox.restarts(), [])

    def test_a_deimos_profile_that_runs_another_translator_is_named(self):
        self.sandbox.profile.write_text(self.sandbox.profile_text(current=False), encoding="utf-8")
        warned = self.activate("1.0.0")
        self.assertActivated(warned)
        self.assertIn("Warning", warned.stdout + warned.stderr)
        self.assertIn(f"command: {self.sandbox.base}/current/.venv/bin/orbit-mcp", warned.stdout + warned.stderr)
        self.sandbox.profile.write_text(self.sandbox.profile_text(current=True), encoding="utf-8")
        quiet = self.activate("1.1.0")
        self.assertActivated(quiet)
        self.assertNotIn("Warning", quiet.stdout + quiet.stderr)

    # The gateway, through Hermes ---------------------------------------

    def test_the_host_gateway_is_restarted_through_hermes_and_never_through_systemd(self):
        activated = self.activate("1.0.0")
        self.assertActivated(activated)
        self.assertEqual(self.sandbox.hermes_calls().count(GATEWAY_RESTART_ARGV), 1, self.sandbox.hermes_calls())
        for call in self.sandbox.events("systemctl"):
            self.assertNotIn(RETIRED_GATEWAY_UNIT, call["argv"], call)
        self.assertIn(translator_args(self.sandbox.base, "1.0.0"), self.sandbox.gateway_translators())
        self.assertIn("1.0.0", activated.stdout)
        self.assertIn(f"PID {GATEWAY_PID}", activated.stdout)

    def test_activation_waits_for_the_new_translator_rather_than_a_deadline(self):
        # The new orbit-mcp shows up a few listings after the restart returns,
        # as it did on the server; the wait ends on seeing it.
        self.sandbox.write_hermes({"respawn_after_calls": 3})
        self.assertActivated(self.activate("1.0.0", ORBIT_HUB_GATEWAY_TIMEOUT="600"))
        self.assertIn(translator_args(self.sandbox.base, "1.0.0"), self.sandbox.gateway_translators())

    def test_a_gateway_that_keeps_its_old_translator_is_named_with_what_is_seen(self):
        self.assertActivated(self.activate("1.0.0"))
        self.sandbox.write_hermes({"respawn": False})
        self.sandbox.clear_events()
        failed = self.activate("1.1.0", ORBIT_HUB_GATEWAY_TIMEOUT="2")
        self.assertNotEqual(failed.returncode, 0)
        self.assertIn("1.1.0", failed.stderr)
        self.assertIn("2 s", failed.stderr)
        self.assertIn(f"PID {GATEWAY_PID}", failed.stderr)
        self.assertIn("1.0.0", failed.stderr, "it does not say which version the gateway runs")
        self.assertIn(GATEWAY_RESTART, failed.stderr)
        self.assertIn("activate.sh 1.0.0", failed.stderr)
        # What Hermes itself says goes along, with its command and code.
        self.assertIn("hermes -p default gateway status", failed.stderr)
        self.assertIn(HOST_STATUS_OUTPUT, failed.stderr)
        self.assertIn(GATEWAY_STATUS_ARGV, self.sandbox.hermes_calls())

    def test_a_gateway_with_no_translator_at_all_is_named_as_such(self):
        self.sandbox.write_hermes({"respawn": False})
        self.sandbox.write_processes(
            [p for p in self.sandbox.default_processes() if "orbit-mcp" not in p["args"]]
        )
        failed = self.activate("1.0.0", ORBIT_HUB_GATEWAY_TIMEOUT="2")
        self.assertNotEqual(failed.returncode, 0)
        self.assertIn("no orbit-mcp", failed.stderr)
        self.assertIn(f"{GATEWAY_PID}", failed.stderr)

    def test_a_translator_respawned_from_another_release_is_named(self):
        self.sandbox.write_hermes({"respawn_version": "0.9.0"})
        failed = self.activate("1.0.0", ORBIT_HUB_GATEWAY_TIMEOUT="2")
        self.assertNotEqual(failed.returncode, 0)
        self.assertIn("0.9.0", failed.stderr)
        self.assertIn("1.0.0", failed.stderr)

    def test_the_translator_of_hermes_serve_orbit_does_not_count_for_the_gateway(self):
        # hermes-serve-orbit runs its own orbit-mcp from the same release; only
        # a child of the gateway proves the gateway took it.
        self.sandbox.add_process(900, SERVE_PID, translator_args(self.sandbox.base, "1.0.0"))
        self.sandbox.write_hermes({"respawn": False})
        failed = self.activate("1.0.0", ORBIT_HUB_GATEWAY_TIMEOUT="2")
        self.assertNotEqual(failed.returncode, 0)
        self.assertIn(GATEWAY_RESTART, failed.stderr)

    def test_restarting_the_same_release_needs_a_new_translator_not_the_old_one(self):
        self.assertActivated(self.activate("1.0.0"))
        self.sandbox.write_hermes({"respawn": False})
        failed = self.activate("1.0.0", ORBIT_HUB_GATEWAY_TIMEOUT="2")
        self.assertNotEqual(failed.returncode, 0, "it accepted the orbit-mcp from before the restart")

    def test_another_translator_left_on_an_old_release_is_warned_about(self):
        # A second gateway process with an orbit-mcp of another release: the
        # activation stands, since Deimos's gateway took it, but it is said.
        self.sandbox.add_process(250, 100, "/home/fake/hermes/python3 -I -c pass gateway run")
        self.sandbox.add_process(260, 250, translator_args(self.sandbox.base, "0.9.0"))
        activated = self.activate("1.0.0")
        self.assertActivated(activated)
        self.assertIn("Warning", activated.stderr)
        self.assertIn("PID 260", activated.stderr)
        self.assertIn("0.9.0", activated.stderr)

    def test_a_failed_gateway_restart_names_the_command_its_output_and_code(self):
        self.assertActivated(self.activate("1.0.0"))
        self.sandbox.write_hermes({"restart_exit": 1, "restart_output": "fake gateway failure", "respawn": False})
        self.sandbox.clear_events()
        failed = self.activate("1.1.0", ORBIT_HUB_GATEWAY_TIMEOUT="600")
        self.assertNotEqual(failed.returncode, 0)
        self.assertIn(GATEWAY_RESTART, failed.stderr)
        self.assertIn("fake gateway failure", failed.stderr)
        self.assertIn("status 1", failed.stderr)
        self.assertIn("activate.sh 1.0.0", failed.stderr)
        # It fails on Hermes's answer, without waiting out the 600 s.
        self.assertLess(len(self.sandbox.events("ps")), 5, "it waited for orbit-mcp after a failed restart")

    def test_a_restart_hermes_could_not_confirm_is_judged_by_the_translator(self):
        # Measured on 2026-10-15: the restart answered "not confirmed … still
        # pending" and had worked. That answer alone is no failure.
        for code in (0, 1):
            with self.subTest(code=code):
                self.sandbox.write_hermes({"restart_exit": code, "restart_output": PENDING_RESTART_OUTPUT})
                activated = self.activate("1.0.0")
                self.assertActivated(activated)
                self.assertIn(PENDING_RESTART_OUTPUT, activated.stderr)
        self.sandbox.write_hermes({"restart_exit": 1, "restart_output": PENDING_RESTART_OUTPUT, "respawn": False})
        failed = self.activate("1.1.0", ORBIT_HUB_GATEWAY_TIMEOUT="2")
        self.assertNotEqual(failed.returncode, 0)
        self.assertIn(PENDING_RESTART_OUTPUT, failed.stderr)
        self.assertIn("status 1", failed.stderr)

    def test_a_failing_gateway_status_is_named_with_its_output_and_code(self):
        self.sandbox.write_hermes({"respawn": False, "status_exit": 2, "status_output": "fake broken status"})
        failed = self.activate("1.0.0", ORBIT_HUB_GATEWAY_TIMEOUT="2")
        self.assertNotEqual(failed.returncode, 0)
        self.assertIn("hermes -p default gateway status", failed.stderr)
        self.assertIn("fake broken status", failed.stderr)
        self.assertIn("status 2", failed.stderr)

    def test_a_missing_hermes_is_named_with_its_path_before_anything_moves(self):
        missing = self.sandbox.root / "no-hermes-here"
        refused = self.activate("1.0.0", ORBIT_HUB_HERMES=str(missing))
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn(str(missing), refused.stderr)
        self.assertIn("ORBIT_HUB_HERMES", refused.stderr)
        self.assertFalse(self.current.is_symlink(), "it moved current without hermes")
        self.assertEqual(self.sandbox.restarts(), [])

    # Going back ----------------------------------------------------------

    def test_going_back_is_the_same_command(self):
        self.assertActivated(self.activate("1.0.0"))
        self.sandbox.clear_events()
        self.assertActivated(self.activate("1.1.0"))
        forward = [entry["argv"][1:] for entry in self.sandbox.events("systemctl")]
        self.sandbox.clear_events()
        back = self.activate("anterior")
        self.assertActivated(back)
        self.assertEqual(os.readlink(self.current), "releases/1.0.0")
        self.assertEqual(os.readlink(self.previous), "releases/1.1.0")
        self.assertEqual([entry["argv"][1:] for entry in self.sandbox.events("systemctl")], forward)
        self.assertEqual(self.sandbox.restarts(), list(RESTART_ORDER))
        for version in ("1.0.0", "1.1.0"):
            self.assertTrue((self.sandbox.base / "releases" / version / "RELEASE").is_file())
        # And by number, the same way.
        self.assertActivated(self.activate("1.1.0"))
        self.assertEqual(os.readlink(self.current), "releases/1.1.0")

    def test_every_activation_says_how_to_go_back(self):
        self.assertActivated(self.activate("1.0.0"))
        second = self.activate("1.1.0")
        self.assertActivated(second)
        self.assertIn("activate.sh 1.0.0", second.stdout)

    def test_activating_the_current_version_again_keeps_the_way_back(self):
        self.assertActivated(self.activate("1.0.0"))
        self.assertActivated(self.activate("1.1.0"))
        self.assertActivated(self.activate("1.1.0"))
        self.assertEqual(os.readlink(self.current), "releases/1.1.0")
        self.assertEqual(os.readlink(self.previous), "releases/1.0.0")

    def test_the_last_two_releases_are_kept_through_any_sequence(self):
        self.sandbox.make_release("1.2.0")
        for version in ("1.0.0", "1.1.0", "1.2.0", "anterior", "1.0.0"):
            self.assertActivated(self.activate(version))
        for version in ("1.0.0", "1.1.0", "1.2.0"):
            self.assertTrue((self.sandbox.base / "releases" / version / "RELEASE").is_file(), version)
        self.assertEqual(os.readlink(self.current), "releases/1.0.0")
        self.assertEqual(os.readlink(self.previous), "releases/1.1.0")


@linux_only
class RestartAndStatusTests(unittest.TestCase):
    def setUp(self):
        self.sandbox = HubSandbox()
        self.addCleanup(self.sandbox.close)
        self.sandbox.make_release("1.0.0")
        self.sandbox.make_release("1.1.0")

    def test_restart_restarts_the_three_in_order_without_moving_versions(self):
        self.assertEqual(self.sandbox.run("activate.sh", "1.0.0").returncode, 0)
        self.assertEqual(self.sandbox.run("activate.sh", "1.1.0").returncode, 0)
        self.sandbox.clear_events()
        restarted = self.sandbox.run("restart.sh")
        self.assertEqual(restarted.returncode, 0, restarted.stderr)
        self.assertEqual(self.sandbox.restarts(), list(RESTART_ORDER))
        self.assertEqual(os.readlink(self.sandbox.base / "current"), "releases/1.1.0")
        self.assertEqual(os.readlink(self.sandbox.base / "previous"), "releases/1.0.0")

    def test_restart_without_an_active_release_is_refused(self):
        refused = self.sandbox.run("restart.sh")
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("activate.sh", refused.stderr)
        self.assertEqual(self.sandbox.restarts(), [])

    def test_restart_needs_hermes_and_waits_for_the_new_translator(self):
        self.assertEqual(self.sandbox.run("activate.sh", "1.0.0").returncode, 0)
        missing = self.sandbox.root / "no-hermes-here"
        refused = self.sandbox.run("restart.sh", env=self.sandbox.env(ORBIT_HUB_HERMES=str(missing)))
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn(str(missing), refused.stderr)
        self.sandbox.write_hermes({"respawn": False})
        self.sandbox.clear_events()
        failed = self.sandbox.run("restart.sh", env=self.sandbox.env(ORBIT_HUB_GATEWAY_TIMEOUT="2"))
        self.assertNotEqual(failed.returncode, 0, "it took the orbit-mcp from before the restart as good")
        self.assertIn(GATEWAY_RESTART, failed.stderr)

    def test_status_no_longer_asks_for_the_retired_unit_and_shows_the_gateway(self):
        self.assertEqual(self.sandbox.run("activate.sh", "1.0.0").returncode, 0)
        self.sandbox.clear_events()
        status = self.sandbox.run("status.sh")
        self.assertEqual(status.returncode, 0, status.stderr)
        self.assertNotIn(RETIRED_GATEWAY_UNIT, status.stdout + status.stderr)
        for call in self.sandbox.events("systemctl"):
            self.assertNotIn(RETIRED_GATEWAY_UNIT, call["argv"], call)
        self.assertEqual(self.sandbox.hermes_calls(), [GATEWAY_STATUS_ARGV], "status.sh asked for more than status")
        self.assertIn(HOST_STATUS_OUTPUT, status.stdout)
        translator = [line for line in status.stdout.splitlines() if "orbit-mcp" in line and "gateway" in line]
        self.assertTrue(translator, status.stdout)
        self.assertTrue(any("1.0.0" in line and "active" in line for line in translator), status.stdout)
        self.assertEqual(self.sandbox.restarts(), [])

    def test_status_says_when_the_gateway_runs_another_release(self):
        self.assertEqual(self.sandbox.run("activate.sh", "1.0.0").returncode, 0)
        self.assertEqual(self.sandbox.run("activate.sh", "1.1.0", env=self.sandbox.env()).returncode, 0)
        table = [p for p in self.sandbox.processes() if not ("orbit-mcp" in p["args"] and p["ppid"] == GATEWAY_PID)]
        table.append({"pid": 777, "ppid": GATEWAY_PID, "args": translator_args(self.sandbox.base, "1.0.0")})
        self.sandbox.write_processes(table)
        status = self.sandbox.run("status.sh")
        self.assertIn("1.0.0", status.stdout)
        self.assertTrue(
            any("777" in line and "1.0.0" in line and "1.1.0" in line for line in status.stdout.splitlines()),
            status.stdout,
        )

    def test_status_names_a_failing_gateway_status_and_a_missing_hermes(self):
        self.assertEqual(self.sandbox.run("activate.sh", "1.0.0").returncode, 0)
        self.sandbox.write_hermes({"status_exit": 2, "status_output": "fake broken status"})
        status = self.sandbox.run("status.sh")
        self.assertNotEqual(status.returncode, 0)
        shown = status.stdout + status.stderr
        self.assertIn("hermes -p default gateway status", shown)
        self.assertIn("status 2", shown)
        self.assertIn("fake broken status", shown)
        missing = self.sandbox.root / "no-hermes-here"
        status = self.sandbox.run("status.sh", env=self.sandbox.env(ORBIT_HUB_HERMES=str(missing)))
        self.assertNotEqual(status.returncode, 0)
        self.assertIn(str(missing), status.stdout + status.stderr)
        self.assertIn("Active:", status.stdout, "it stopped reporting the rest because hermes was missing")

    def test_status_names_current_previous_and_every_release(self):
        self.sandbox.make_release("1.2.0", complete=False)
        self.assertEqual(self.sandbox.run("activate.sh", "1.0.0").returncode, 0)
        self.assertEqual(self.sandbox.run("activate.sh", "1.1.0").returncode, 0)
        self.sandbox.clear_events()
        status = self.sandbox.run("status.sh")
        self.assertEqual(status.returncode, 0, status.stderr)
        lines = status.stdout.splitlines()
        self.assertTrue(any("1.1.0" in line and "current" in line for line in lines), status.stdout)
        self.assertTrue(any("1.0.0" in line and "previous" in line for line in lines), status.stdout)
        self.assertTrue(any("1.2.0" in line and "incomplete" in line for line in lines), status.stdout)
        self.assertEqual(self.sandbox.restarts(), [], "status.sh restarted something")


if __name__ == "__main__":
    unittest.main()
