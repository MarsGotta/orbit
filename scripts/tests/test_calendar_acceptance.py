"""Disposable calendar acceptance against an actual, synthetic Unix socket."""

import copy
import json
import socket
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "e2e-mac/calendar_acceptance.py"
PRIVATE = "PRIVATE-MAPLE-CALENDAR"
DAY = 86_400_000


class CalendarDaemon:
    def __init__(self, path, mode="success"):
        self.mode = mode
        self.events = []
        self.writes = []
        self.commands = []
        self.window = None
        self.serial = 0
        self.read_id = None
        self.pending = False
        self.errors = []
        self.server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.server.bind(str(path))
        self.server.listen()
        self.thread = threading.Thread(target=self.serve, daemon=True)
        self.thread.start()

    def close(self):
        # Wake accept without a timed polling loop.
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as wake:
            wake.connect(self.server.getsockname())
            wake.sendall(b"{}\n")
        self.thread.join(5)
        self.server.close()

    def serve(self):
        while True:
            connection, _ = self.server.accept()
            with connection, connection.makefile("rb") as stream:
                request = json.loads(stream.readline())
                if not request:
                    return
                try:
                    rows = self.answer(request)
                    connection.sendall(b"".join((json.dumps(r) + "\n").encode() for r in rows))
                except Exception as error:
                    self.errors.append(repr(error))

    def answer(self, envelope):
        assert envelope["v"] == 1
        body = envelope["body"]
        self.commands.append(copy.deepcopy(body))
        reply = {"v": 1, "id": envelope["id"], "ok": True}
        command = body["command"]
        if self.mode == "malformed":
            return [{**reply, "answer": PRIVATE}]
        if command == "subscribe_topics":
            assert body["topics"] == ["agenda"]
            if self.mode == "bad_frame":
                return [reply, {"event": "topic.changed", "frame": {"data": []}}]
            outcomes = copy.deepcopy(self.writes)
            if self.pending and outcomes:
                outcomes[-1]["stage"] = "writing"
                outcomes[-1].pop("saved", None)
                self.pending = False
            value = {
                "invalidation": self.serial,
                "calendars": 2,
                "read_window": self.window,
                "read_id": self.read_id,
                "timezone": None if self.mode == "cold" and self.window is None else "UTC",
                "writes": outcomes,
            }
            return [
                reply,
                {
                    "event": "topic.changed",
                    "frame": {
                        "data": {
                            "topic": "agenda",
                            "knowledge": {"state": "present", "value": value},
                        }
                    },
                },
            ]
        if command == "calendar":
            request = body["request"]
            if request["action"] == "read":
                self.serial += 1
                self.read_id = str(self.serial)
                self.window = request["window"]
            else:
                self.apply(request)
                if self.mode == "lost_create" and len(self.writes) == 1:
                    return []
            return [reply]
        assert command == "read"
        read = body["read"]
        if self.mode == "read_refused":
            return [
                {
                    **reply,
                    "read": read["kind"],
                    "answer": {
                        "state": "absent",
                        "refusal": {
                            "code": "helper_refused",
                            "helper_code": "calendar_busy",
                            "notice": PRIVATE,
                        },
                    },
                }
            ]
        if read["kind"] == "agenda_calendars":
            fixture = {
                "title": "Cedar fixture",
                "reference": {"calendar_id": "cedar", "source_id": "fixture-source"},
                "can_edit": self.mode != "readonly",
            }
            if self.mode == "unknown_editability":
                fixture.pop("can_edit")
            entries = [
                {
                    "title": PRIVATE,
                    "reference": {"calendar_id": PRIVATE, "source_id": PRIVATE},
                    "can_edit": True,
                }
            ]
            if self.mode != "missing":
                entries.append(fixture)
                if self.mode == "duplicate":
                    entries.append({**fixture, "reference": {"calendar_id": "cedar-duplicate"}})
        else:
            assert read["kind"] == "agenda_range"
            assert read["start_ms"] == self.window["start_unix_ms"]
            assert read["end_ms"] == self.window["end_unix_ms"]
            entries = [
                {
                    "title": PRIVATE,
                    "calendar": {"calendar_id": PRIVATE},
                    "calendar_item_id": PRIVATE,
                    "observation_id": "private-observation",
                    "start_unix_ms": 1,
                }
            ] + copy.deepcopy(self.events)
            for index, entry in enumerate(entries):
                entry["observation_id"] = "observation-" + str(index)
        if self.mode == "nonempty" and read["kind"] == "agenda_range":
            entries.append(
                {
                    "calendar": {"calendar_id": "cedar"},
                    "title": PRIVATE,
                    "start_unix_ms": 2,
                    "observation_id": "existing",
                }
            )
        cursor = read.get("cursor")
        if read["kind"] == "agenda_calendars":
            entries.sort(key=lambda entry: entry["reference"]["calendar_id"])
            key = lambda entry: {"kind": "id", "id": entry["reference"]["calendar_id"]}
        else:
            entries.sort(key=lambda entry: (entry["start_unix_ms"], entry["observation_id"]))
            key = lambda entry: {
                "kind": "start_and_id",
                "start_ms": entry["start_unix_ms"],
                "id": entry["observation_id"],
            }
        offset = (
            0
            if cursor is None
            else next(i + 1 for i, entry in enumerate(entries) if key(entry) == cursor)
        )
        # One row per page exercises filtering on every page, including private-only pages.
        following = key(entries[offset]) if offset + 1 < len(entries) else None
        page = {
            "items": entries[offset : offset + 1],
            "next": following,
            "total": len(entries),
            "revision": self.serial,
        }
        return [
            {
                **reply,
                "read": read["kind"],
                "answer": {"state": "present", "value": page, "as_of_ms": self.serial},
            }
        ]

    def apply(self, request):
        outcome = {
            "request_id": request["request_id"],
            "operation": "delete",
            "stage": "deleted",
        }
        if self.mode == "refused" and not self.writes:
            self.writes.append(
                {
                    **outcome,
                    "operation": "create",
                    "stage": "refused",
                    "code": "calendar_not_editable",
                    "notice": PRIVATE,
                }
            )
            return
        if "save" in request:
            save = request["save"]
            if "create" in save:
                draft = save["create"]
                assert draft["calendar_id"] == "cedar"
                count = draft.get("recurrence", {}).get("end", {}).get("count", 1)
                item = "cedar-event-" + str(len(self.writes))
                for index in range(count):
                    start = draft["start_ms"] + index * DAY
                    self.events.append(
                        {
                            "calendar": {
                                "calendar_id": "cedar",
                                "source_id": "fixture-source",
                            },
                            "calendar_item_id": item,
                            "event_id": item + str(index),
                            "original_occurrence_unix_ms": start,
                            "start_unix_ms": start,
                            "end_unix_ms": draft["end_ms"] + index * DAY,
                            "title": draft["title"],
                            "notes": draft["notes"],
                            "location": draft["location"],
                            "event_timezone": draft["time_zone"],
                            "interpreted_timezone": draft["time_zone"],
                            "is_all_day": False,
                            "invitees": [],
                            "invitees_status": "provided",
                            "can_edit": True,
                            **({"recurrence": draft["recurrence"]} if count > 1 else {}),
                        }
                    )
                event = self.events[-count]
                outcome["operation"] = "create"
            else:
                change = save["change"]
                assert change["span"] == "this"
                assert change["changes"]["calendar_id"] == "cedar"
                event = self.target(change)
                event["start_unix_ms"] = change["changes"]["start_ms"]
                event["end_unix_ms"] = change["changes"]["end_ms"]
                event["is_detached"] = True
                outcome["operation"] = "update"
            outcome.update(
                stage="saved",
                saved={
                    "calendar_item_id": event["calendar_item_id"],
                    "occurrence_start_ms": event["original_occurrence_unix_ms"],
                    "start_ms": event["start_unix_ms"],
                    "end_ms": event["end_unix_ms"],
                },
            )
        else:
            target = request["delete"]
            event = self.target(target)
            if self.mode == "cleanup_refused":
                outcome.update(stage="refused", code="calendar_save_failed", notice=PRIVATE)
            elif target["span"] == "future":
                self.events = [
                    e
                    for e in self.events
                    if not (
                        e["calendar_item_id"] == event["calendar_item_id"]
                        and e["original_occurrence_unix_ms"] >= event["original_occurrence_unix_ms"]
                    )
                ]
            else:
                self.events.remove(event)
        if self.mode == "unknown" and outcome["operation"] == "create":
            outcome.update(stage="outcome_unknown", code="calendar_write_outcome_unknown")
            outcome.pop("saved", None)
        if self.mode == "wrong_title":
            for event in self.events:
                event["title"] = "Cedar altered review"
        if self.mode == "no_recurrence":
            for event in self.events:
                event.pop("recurrence", None)
        if self.mode == "nil_invitees":
            for event in self.events:
                event["invitees"] = None
                event["invitees_status"] = "not_provided"
        if self.mode == "badmove" and outcome["operation"] == "update":
            duplicate = copy.deepcopy(event)
            duplicate["start_unix_ms"] -= 3 * 3_600_000
            duplicate["end_unix_ms"] -= 3 * 3_600_000
            self.events.append(duplicate)
        self.writes.append(outcome)
        self.pending = True

    def target(self, target):
        return next(
            e
            for e in self.events
            if e["calendar_item_id"] == target["calendar_item_id"]
            and e["original_occurrence_unix_ms"] == target["occurrence_start_ms"]
        )


class CalendarAcceptanceTests(unittest.TestCase):
    def run_case(self, mode="success"):
        with tempfile.TemporaryDirectory(prefix="cedar-") as directory:
            path = Path(directory) / "fixture.sock"
            daemon = CalendarDaemon(path, mode)
            try:
                result = subprocess.run(
                    [
                        sys.executable,
                        "-B",
                        str(SCRIPT),
                        "--socket",
                        str(path),
                        "--date",
                        "2040-06-12",
                        *([] if mode == "cold" else ["--timezone", "UTC"]),
                        "--timeout",
                        "2",
                    ],
                    capture_output=True,
                    text=True,
                    timeout=20,
                )
            finally:
                daemon.close()
            self.assertEqual(daemon.errors, [])
            self.assertNotIn(PRIVATE, result.stdout + result.stderr)
            self.assertEqual(result.stderr, "")
            return result, json.loads(result.stdout), daemon

    def test_steps_read_back_and_leave_fixture_empty(self):
        result, report, daemon = self.run_case()
        self.assertEqual(result.returncode, 0, report)
        self.assertEqual([s["status"] for s in report["steps"]], ["passed"] * 4)
        self.assertEqual(report["cleanup"]["status"], "passed")
        self.assertEqual(daemon.events, [])
        deletes = [
            c["request"]["delete"]
            for c in daemon.commands
            if c["command"] == "calendar" and c["request"]["action"] == "delete"
        ]
        self.assertEqual([d["span"] for d in deletes], ["this", "future", "this", "this"])
        self.assertIn("+00:00", result.stdout)

    def test_refusal_is_failure_with_code_and_no_retry(self):
        result, report, daemon = self.run_case("refused")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(report["steps"][0]["status"], "failed")
        self.assertIn("calendar_not_editable", result.stdout)
        self.assertEqual(len(daemon.writes), 1)
        self.assertEqual(daemon.events, [])

    def test_missing_or_readonly_calendar_never_writes(self):
        for mode in ("missing", "readonly", "duplicate", "nonempty"):
            with self.subTest(mode=mode):
                result, report, daemon = self.run_case(mode)
                self.assertEqual(result.returncode, 1)
                self.assertEqual(daemon.writes, [])
                self.assertEqual(report["status"], "failed")

    def test_unknown_create_is_read_before_cleanup_and_never_recreated(self):
        result, report, daemon = self.run_case("unknown")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(report["cleanup"]["status"], "passed")
        self.assertEqual(daemon.events, [])
        self.assertEqual([w["operation"] for w in daemon.writes], ["create", "delete"])

    def test_cleanup_failure_names_only_run_events(self):
        result, report, daemon = self.run_case("cleanup_refused")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(report["cleanup"]["status"], "failed")
        self.assertGreater(len(report["cleanup"]["remaining"]), 0)
        self.assertGreater(len(daemon.events), 0)

    def test_wrong_title_fails_but_run_marker_still_allows_cleanup(self):
        result, report, daemon = self.run_case("wrong_title")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(report["steps"][0]["status"], "failed")
        self.assertEqual(report["cleanup"]["status"], "passed")
        self.assertEqual(daemon.events, [])

    def test_unknown_editability_is_distinct_from_readonly(self):
        result, report, daemon = self.run_case("unknown_editability")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(report["steps"][0]["code"], "fixture_editability_unknown")
        self.assertEqual(daemon.writes, [])

    def test_read_refusal_preserves_specific_helper_code(self):
        result, report, daemon = self.run_case("read_refused")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(report["steps"][0]["code"], "calendar_busy")
        self.assertEqual(daemon.writes, [])

    def test_missing_recurrence_is_failed_readback_and_cleaned(self):
        result, report, daemon = self.run_case("no_recurrence")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(report["steps"][1]["status"], "failed")
        self.assertEqual(report["cleanup"]["status"], "passed")
        self.assertEqual(daemon.events, [])

    def test_default_timezone_bootstraps_a_cold_agenda(self):
        result, report, daemon = self.run_case("cold")
        self.assertEqual(result.returncode, 0, report)
        self.assertEqual(report["timezone"], "UTC")
        self.assertEqual(daemon.events, [])

    def test_nil_eventkit_attendees_means_not_provided(self):
        result, report, daemon = self.run_case("nil_invitees")
        self.assertEqual(result.returncode, 0, report)
        self.assertEqual(daemon.events, [])

    def test_lost_create_receipt_is_resolved_and_cleaned_without_replay(self):
        result, report, daemon = self.run_case("lost_create")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(report["cleanup"]["status"], "passed")
        self.assertEqual(daemon.events, [])
        self.assertEqual([w["operation"] for w in daemon.writes], ["create", "delete"])

    def test_failed_middle_move_cleans_every_created_occurrence(self):
        result, report, daemon = self.run_case("badmove")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(report["steps"][0]["status"], "passed")
        self.assertEqual(report["steps"][1]["status"], "failed")
        self.assertEqual(report["cleanup"]["status"], "passed")
        self.assertEqual(daemon.events, [])

    def test_wrong_frame_type_is_clean_failure(self):
        result, report, daemon = self.run_case("bad_frame")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(report["status"], "failed")
        self.assertEqual(daemon.writes, [])

    def test_python39_syntax(self):
        import ast

        ast.parse(SCRIPT.read_text(), feature_version=(3, 9))

    def test_unexpected_reply_is_clean_json_failure(self):
        result, report, daemon = self.run_case("malformed")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(report["status"], "failed")
        self.assertEqual(daemon.writes, [])


if __name__ == "__main__":
    unittest.main()
