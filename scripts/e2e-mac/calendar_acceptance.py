#!/usr/bin/env python3
"""Run steps 1–4 of calendar-writes.md on the consented Cedar fixture.

Python 3.9+, standard library only. stdout is a privacy-filtered JSON report.
Never echo transport payloads, exception messages, notices or other calendars.
"""

import argparse
import json
import math
import socket
import sys
import time
import uuid
from contextlib import contextmanager
from datetime import date, datetime, timedelta
from datetime import time as day_time
from pathlib import Path
from zoneinfo import ZoneInfo

FIXTURE = "Cedar fixture"
TITLES = ("Cedar budget review", "Cedar recurring review")
NOTES = "Invented acceptance event"
# Protect the script's per-response allocation. Agenda reads are <=128 KiB;
# 256 KiB allows their envelope too. This does not bound event/calendar counts.
RESPONSE_BYTES = 256 * 1024
CODE_NAMES = (
    "calendar_event_not_found calendar_not_editable calendar_invalid_change "
    "calendar_permission_denied calendar_save_failed calendar_write_outcome_unknown "
    "calendar_not_written calendar_write_queue_full privacy_stopped calendar_busy "
    "calendar_permission_required calendar_read_expired calendar_invalidated "
    "calendar_selection_unavailable calendar_limit_exceeded calendar_read_failed "
    "calendar_clock_unavailable calendar_event_too_large permission_denied "
    "calendar_backend_unavailable invalid_parameter too_large "
    "calendar_read_limit helper_refused helper_reply_invalid helper_not_running read_interrupted "
    "permission_unknown revision_changed timeout busy protocol_version encode_failed"
)
CODES = frozenset(CODE_NAMES.split())
EVENT_FIELDS = (
    "calendar_item_id event_id original_occurrence_unix_ms title notes "
    "event_timezone interpreted_timezone start_unix_ms end_unix_ms "
    "is_all_day is_detached recurrence"
)
EXPECTED_ERRORS = (
    OSError,
    ValueError,
    KeyError,
    TypeError,
    AttributeError,
    IndexError,
    StopIteration,
    OverflowError,
    RecursionError,
)


class Failure(Exception):
    """Only locally chosen codes may reach stdout."""


def require(condition, code="unexpected_daemon_response"):
    if not condition:
        raise Failure(code)


def refusal_code(value):
    require(isinstance(value, dict))
    code = value.get("helper_code", value.get("code"))
    return code if code in CODES else "unrecognized_refusal"


def present(knowledge):
    require(isinstance(knowledge, dict))
    state = knowledge.get("state")
    if state == "unknown":
        return None
    if state == "absent":
        raise Failure(refusal_code(knowledge.get("refusal")))
    require(state == "present" and isinstance(knowledge.get("value"), dict))
    return knowledge["value"]


class Ipc:
    def __init__(self, path, timeout):
        self.path, self.timeout = path, timeout

    @contextmanager
    def request(self, body):
        identifier = "cedar-" + uuid.uuid4().hex
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
            connection.settimeout(self.timeout)
            connection.connect(str(self.path))
            line = {"v": 1, "id": identifier, "body": body}
            connection.sendall((json.dumps(line) + "\n").encode())
            with connection.makefile("rb") as stream:
                reply = self.line(stream)
                require(reply.get("v") == 1 and reply.get("id") == identifier)
                require(type(reply.get("ok")) is bool)
                if not reply["ok"]:
                    raise Failure(refusal_code(reply.get("refusal", {})))
                yield reply, stream

    @staticmethod
    def line(stream):
        line = stream.readline(RESPONSE_BYTES + 1)
        require(
            bool(line) and line.endswith(b"\n") and len(line) <= RESPONSE_BYTES,
            "invalid_response_line",
        )
        row = json.loads(line)
        require(isinstance(row, dict))
        return row

    def call(self, body):
        with self.request(body) as (reply, _):
            return reply

    def calendar_command(self, request):
        return self.call({"command": "calendar", "request": request})

    def head(self):
        body = {"command": "subscribe_topics", "topics": ["agenda"]}
        with self.request(body) as (_, stream):
            event = self.line(stream)
            require(event.get("event") == "topic.changed")
            data = event["frame"]["data"]
            require(data.get("topic") == "agenda")
            return present(data.get("knowledge"))

    def wait(self, observe, code):
        deadline = time.monotonic() + self.timeout
        while True:
            value = observe()
            if value is not None:
                return value
            require(time.monotonic() < deadline, code)
            # Polling is a wait budget, not an acceptance latency assertion.
            time.sleep(0.1)

    def pages(self, kind, select, **fields):
        cursor, seen, revision, count, total = None, set(), None, 0, None
        selected = []
        while True:
            reply = self.call(
                {"command": "read", "read": {"kind": kind, "cursor": cursor, **fields}}
            )
            require(reply.get("read") == kind)
            page = present(reply.get("answer"))
            if page is None:
                return None
            require(isinstance(page.get("items"), list))
            require(type(page.get("total")) is int and type(page.get("revision")) is int)
            if revision is None:
                revision, total = page["revision"], page["total"]
            require((revision, total) == (page["revision"], page["total"]), "read_changed")
            for item in page["items"]:
                require(isinstance(item, dict))
                chosen = select(item)
                if chosen is not None:
                    selected.append(chosen)
            count += len(page["items"])
            require(count <= total)
            cursor = page["next"]
            if cursor is None:
                require(count == total)
                return selected
            require(isinstance(cursor, dict) and bool(page["items"]))
            key = json.dumps(cursor, sort_keys=True)
            require(key not in seen, "repeated_cursor")
            seen.add(key)


class Acceptance:
    def __init__(self, ipc, chosen_date, timezone):
        self.ipc, self.chosen_date, self.timezone = ipc, chosen_date, timezone
        self.calendar_id = None
        self.marker = "cedar-acceptance-" + uuid.uuid4().hex
        self.sent = []
        self.last_events = []
        self.report = {
            "fixture": FIXTURE,
            "status": "failed",
            "steps": [
                {"step": n, "status": "not_run", "reads": [], "writes": []} for n in range(1, 5)
            ],
            "cleanup": {"status": "not_run", "remaining": []},
        }
        self.current = self.report["steps"][0]

    def calendar(self, entry):
        if entry.get("title") != FIXTURE:
            return None
        editable = entry.get("can_edit")
        code = "fixture_not_writable" if editable is False else "fixture_editability_unknown"
        require(editable is True, code)
        identifier = entry["reference"]["calendar_id"]
        require(isinstance(identifier, str) and bool(identifier))
        return identifier

    def fixture_event(self, event):
        if event.get("calendar", {}).get("calendar_id") != self.calendar_id:
            return None
        return event

    def owned(self, event):
        return event.get("location") == self.marker

    def setup(self):
        head = self.ipc.wait(self.ipc.head, "agenda_not_observed")
        if not self.timezone and not head.get("timezone"):
            # A cold agenda has no zone until its first complete read.
            start = int(time.time() * 1000)
            self.window = {"start_unix_ms": start, "end_unix_ms": start + 3_600_000}
            head = self.refresh()
        self.zone = ZoneInfo(self.timezone or head["timezone"])
        today = datetime.now(self.zone).date()
        self.day = (
            date.fromisoformat(self.chosen_date) if self.chosen_date else today + timedelta(days=7)
        )
        require(self.day > today, "date_must_be_future")
        self.window = {"start_unix_ms": self.instant(0, 0), "end_unix_ms": self.instant(3, 0)}
        self.report.update(
            date=self.day.isoformat(),
            timezone=self.zone.key,
            started_at=datetime.now(self.zone).isoformat(),
        )
        self.refresh()
        calendars = self.ipc.wait(
            lambda: self.ipc.pages("agenda_calendars", self.calendar),
            "calendar_listing_not_observed",
        )
        require(len(calendars) == 1, "fixture_missing_or_ambiguous")
        self.calendar_id = calendars[0]
        events = self.read_events()
        require(not events, "fixture_window_not_empty")

    def instant(self, day, hour, minute=0):
        moment = datetime.combine(self.day + timedelta(days=day), day_time(hour, minute), self.zone)
        # Refuse a nonexistent wall time rather than silently normalize it.
        stamp = int(moment.timestamp() * 1000)
        require(
            datetime.fromtimestamp(stamp / 1000, self.zone) == moment,
            "nonexistent_local_time",
        )
        return stamp

    def refresh(self):
        old = self.ipc.head()
        old_id = old.get("read_id") if old else None
        self.ipc.calendar_command({"action": "read", "window": self.window})

        def complete():
            head = self.ipc.head()
            if (
                head
                and head.get("read_id")
                and head["read_id"] != old_id
                and head.get("read_window") == self.window
            ):
                return head
            return None

        return self.ipc.wait(complete, "calendar_read_timeout")

    def read_events(self):
        head = self.refresh()
        events = self.ipc.wait(
            lambda: self.ipc.pages(
                "agenda_range",
                self.fixture_event,
                start_ms=self.window["start_unix_ms"],
                end_ms=self.window["end_unix_ms"],
            ),
            "events_not_observed",
        )
        after = self.ipc.head()
        require(after and after.get("read_id") == head["read_id"], "read_changed")
        self.last_events = [event for event in events if self.owned(event)]
        return events

    def fields(self, event):
        # Only our synthetic fixture events reach this projection. No invitee
        # contents, organizer, source titles, notices or other calendars escape.
        require(self.owned(event) and self.fixture_event(event) is not None, "foreign_event")
        fields = {key: event[key] for key in EVENT_FIELDS.split() if key in event}
        fields["calendar_id"] = self.calendar_id
        invitees = event.get("invitees")
        fields["invitees_count"] = len(invitees) if isinstance(invitees, list) else None
        if invitees is None and event.get("invitees_status") == "not_provided":
            fields["invitees_count"] = 0
        fields["invitees_status"] = event.get("invitees_status")
        for key in ("start_unix_ms", "end_unix_ms"):
            fields[key.replace("_unix_ms", "")] = datetime.fromtimestamp(
                event[key] / 1000, self.zone
            ).isoformat()
        return fields

    def snapshot(self, label):
        events = self.read_events()
        self.current["reads"].append(
            {
                "label": label,
                "fixture_count": len(events),
                "events": [self.fields(event) for event in events if self.owned(event)],
            }
        )
        require(all(self.owned(event) for event in events), "fixture_changed_externally")
        return events

    def outcome(self, identifier):
        head = self.ipc.head()
        if head is None:
            return None
        writes = head.get("writes", [])
        require(isinstance(writes, list))
        for write in writes:
            require(isinstance(write, dict))
            if write.get("request_id") != identifier:
                continue
            stage = write.get("stage")
            stages = {"queued", "writing", "saved", "deleted", "refused", "outcome_unknown"}
            require(stage in stages)
            if stage not in ("queued", "writing"):
                return write
        return None

    def write(self, action, **payload):
        identifier = "cedar-" + uuid.uuid4().hex
        operation = (
            "delete"
            if action == "delete"
            else ("create" if "create" in payload["save"] else "update")
        )
        record = {"request_id": identifier, "operation": operation, "stage": "unconfirmed"}
        self.sent.append(record)
        self.current["writes"].append(record)
        self.ipc.calendar_command({"action": action, "request_id": identifier, **payload})
        result = self.ipc.wait(lambda: self.outcome(identifier), "write_outcome_timeout")
        require(result.get("operation") == operation)
        record["stage"] = result["stage"]
        if result.get("code") is not None:
            record["code"] = refusal_code(result)
        return result

    def verify_write(self, result, events):
        expected = "deleted" if result["operation"] == "delete" else "saved"
        require(result["stage"] == expected, "write_did_not_complete")
        if expected == "saved":
            saved = result["saved"]
            require(
                any(
                    self.target(event)
                    == {key: saved[key] for key in ("calendar_item_id", "occurrence_start_ms")}
                    for event in events
                ),
                "saved_identity_not_read",
            )

    def target(self, event):
        require(
            self.owned(event) and self.fixture_event(event) is not None,
            "foreign_delete_target",
        )
        item = event["calendar_item_id"]
        require(isinstance(item, str) and bool(item))
        occurrence = event.get("original_occurrence_unix_ms") or event["start_unix_ms"]
        require(type(occurrence) is int)
        return {"calendar_item_id": item, "occurrence_start_ms": occurrence}

    def create(self, series=False):
        draft = {
            "calendar_id": self.calendar_id,
            "title": TITLES[int(series)],
            "start_ms": self.instant(0, 11 if series else 10),
            "end_ms": self.instant(0, 11 if series else 10, 30),
            "all_day": False,
            "time_zone": self.zone.key,
            "notes": NOTES,
            "location": self.marker,
            "alarms_minutes": [],
        }
        if series:
            draft["recurrence"] = {"frequency": "daily", "interval": 1, "end": {"count": 3}}
        return self.write("save", save={"create": draft})

    def check(self, events, expected):
        actual = sorted(
            (event["title"], event["start_unix_ms"], event["end_unix_ms"]) for event in events
        )
        require(actual == sorted(expected), "event_readback_mismatch")
        for event in events:
            require(
                event.get("notes") == NOTES and not event.get("notes_left_out"),
                "notes_readback_mismatch",
            )
            require(
                event.get("event_timezone") == self.zone.key,
                "timezone_readback_mismatch",
            )
            require(event.get("is_all_day") is False, "all_day_readback_mismatch")
            # EventKit's nil attendees are explicitly `not_provided`; an old
            # or incomplete read's `not_requested` does not prove absence.
            require(
                (event.get("invitees") == [] and event.get("invitees_status") == "provided")
                or (
                    event.get("invitees") is None and event.get("invitees_status") == "not_provided"
                ),
                "invitees_not_confirmed_empty",
            )

    def expected(self, series_days=(), moved=False, budget=True):
        rows = [(TITLES[0], self.instant(0, 10), self.instant(0, 10, 30))] if budget else []
        for day in series_days:
            hour = 14 if moved and day == 1 else 11
            rows.append((TITLES[1], self.instant(day, hour), self.instant(day, hour, 30)))
        return rows

    def remove(self, event, span):
        return self.write("delete", delete={**self.target(event), "span": span})

    def readback(self, result, label, days=(), moved=False, budget=True):
        events = self.snapshot(label)
        self.verify_write(result, events)
        self.check(events, self.expected(days, moved, budget))
        return events

    def resolve(self, record):
        if record["stage"] == "unconfirmed":
            identifier = record["request_id"]
            result = self.ipc.wait(
                lambda: self.outcome(identifier), "cleanup_write_still_unresolved"
            )
            record["stage"] = result["stage"]

    def steps(self):
        result = self.create()
        self.readback(result, "standalone_created")
        self.current["status"] = "passed"
        self.current = self.report["steps"][1]
        result = self.create(series=True)
        events = self.readback(result, "series_created", (0, 1, 2))
        siblings = sorted(
            (e for e in events if e["title"] == TITLES[1]),
            key=lambda e: e["start_unix_ms"],
        )
        rule = {"frequency": "daily", "interval": 1, "end": {"count": 3}}
        require(all(e.get("recurrence") == rule for e in siblings), "recurrence_readback_mismatch")
        middle = siblings[1]
        changes = {
            "calendar_id": self.calendar_id,
            "start_ms": self.instant(1, 14),
            "end_ms": self.instant(1, 14, 30),
        }
        change = {**self.target(middle), "span": "this", "changes": changes}
        result = self.write("save", save={"change": change})
        events = self.readback(result, "middle_moved", (0, 1, 2), moved=True)
        for sibling in (siblings[0], siblings[2]):
            require(
                any(self.target(e) == self.target(sibling) for e in events),
                "sibling_identity_changed",
            )
        self.current["status"] = "passed"
        self.current = self.report["steps"][2]
        middle = next(e for e in events if e["start_unix_ms"] == self.instant(1, 14))
        result = self.remove(middle, "this")
        events = self.readback(result, "middle_deleted", (0, 2))
        self.current["status"] = "passed"
        self.current = self.report["steps"][3]
        last = next(e for e in events if e["start_unix_ms"] == self.instant(2, 11))
        result = self.remove(last, "future")
        events = self.readback(result, "last_and_future_deleted", (0,))
        budget = next(e for e in events if e["title"] == TITLES[0])
        result = self.remove(budget, "this")
        self.readback(result, "standalone_deleted", (0,), budget=False)
        self.current["status"] = "passed"

    def cleanup(self):
        cleanup = self.report["cleanup"]
        if not self.sent:
            cleanup["status"] = "not_needed"
            return
        self.current = cleanup
        cleanup["writes"] = []
        cleanup["status"] = "failed"
        for record in list(self.sent):
            self.resolve(record)
        # A fresh complete read resolves unknown outcomes. Never replay a save.
        events = self.read_events()
        cleanup["errors"] = []
        for event in list(events):
            if self.owned(event):
                try:
                    self.remove(event, "this")
                except (Failure, *EXPECTED_ERRORS) as error:
                    cleanup["errors"].append(safe_error(error))
                    # Resolve a lost delete receipt before issuing another
                    # effect. Still attempt cleanup of the other siblings.
                    self.resolve(self.sent[-1])
        events = self.read_events()
        cleanup["remaining"] = [self.fields(e) for e in events if self.owned(e)]
        cleanup["fixture_count"] = len(events)
        require(not events, "cleanup_fixture_not_empty")
        cleanup["status"] = "passed"

    def run(self):
        try:
            self.setup()
            self.steps()
        except (Failure, *EXPECTED_ERRORS) as error:
            self.current.update(status="failed", code=safe_error(error))
        except KeyboardInterrupt:
            self.current.update(status="failed", code="interrupted")
        finally:
            try:
                self.cleanup()
            except (Failure, *EXPECTED_ERRORS) as error:
                self.report["cleanup"].update(
                    status="failed",
                    code=safe_error(error),
                    remaining=[
                        {
                            "calendar_item_id": e.get("calendar_item_id"),
                            "occurrence_start_ms": e.get("original_occurrence_unix_ms"),
                        }
                        for e in self.last_events
                        if self.owned(e)
                    ],
                    remaining_read_confirmed=False,
                    unresolved_requests=[
                        r["request_id"] for r in self.sent if r["stage"] == "unconfirmed"
                    ],
                )
        passed = all(step["status"] == "passed" for step in self.report["steps"])
        self.report["status"] = (
            "passed" if passed and self.report["cleanup"]["status"] == "passed" else "failed"
        )
        return self.report


def safe_error(error):
    if isinstance(error, Failure):
        return str(error)
    if isinstance(error, OSError):
        return "socket_timeout" if isinstance(error, TimeoutError) else "socket_io_failed"
    return "unexpected_daemon_response_or_invalid_date_zone"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--socket",
        type=Path,
        default=Path.home() / ("Library/Application Support/com.MarsGotta.OrbitPruebas/orbit.sock"),
    )
    parser.add_argument("--date", help="Future local YYYY-MM-DD; default: seven days from today")
    parser.add_argument("--timezone", help="IANA zone; default: daemon agenda's system timezone")
    parser.add_argument("--timeout", type=float, default=60, help="Seconds per wait (default: 60)")
    args = parser.parse_args()
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        report = {"status": "failed", "code": "invalid_timeout"}
    else:
        report = Acceptance(
            Ipc(args.socket.expanduser(), args.timeout), args.date, args.timezone
        ).run()
    print(json.dumps(report, indent=2, ensure_ascii=True))
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())
