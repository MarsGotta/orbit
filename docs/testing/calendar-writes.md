# Disposable-calendar write acceptance

EventKit acceptance runs on the test Mac; it is not run by the Linux gate.
Obtain the calendar owner's explicit yes before doing this procedure. macOS Calendar permission
is the product's consent; this procedure adds no product confirmation or switch.

Use Orbit Pruebas, an isolated hub/assistant profile and a new, empty, writable
calendar named **Cedar fixture**. For the manual pass, the owner selects that
calendar's identifier from the permitted calendar read; the script resolves
its exact title. Never use a default calendar implicitly.
Do not invite anyone or attach real addresses, links or documents. Use a future
local date and record its date, time zone and UTC offsets in the private result.

## Scripted steps 1–4

With Orbit Pruebas running and already granted full calendar access, run from
the repository on the test Mac (Python 3.9 or later, standard library only):

```sh
python3 -B scripts/e2e-mac/calendar_acceptance.py \
  --socket "$HOME/Library/Application Support/com.MarsGotta.OrbitPruebas/orbit.sock" \
  > "$HOME/cedar-calendar-acceptance.json"
```

The default date is seven days from today in the daemon's system time zone.
Use `--date YYYY-MM-DD --timezone America/Los_Angeles` to choose a future
local date and IANA time zone explicitly. The report includes that zone and
each event's start/end with UTC offsets. `--timeout 60` sets each wait budget;
this is not a latency acceptance threshold. Exit 0 means all four socket
checks and the final empty-fixture read passed; exit 1 means failure. The
script stops the sequence after a failed step and marks later steps `not_run`.

The script finds exactly one **Cedar fixture** with `can_edit: true` using
paged `read` requests of kind `agenda_calendars`. It refuses to write when
the fixture is missing, ambiguous, not writable, or has events in the test
window. The owner supplies the empty-calendar precondition outside that
window. Every save explicitly names the fixture calendar identifier.
The script sends `command: "calendar"` with read/save/delete requests, follows
the `agenda` topic's compact `writes` outcomes by its own `request_id`, and
uses paged `agenda_range` reads for actual event fields. A changed complete
`read_id`, the exact window, stable page revisions and an unchanged head
around pagination guard against accepting stale or mixed readbacks.

Only events in the fixture bearing this run's random synthetic location marker
can enter the report or become a change/delete target. Calendar inventory and
events from other calendars are filtered before reporting; daemon error text,
write notices and invitee details are never printed or saved. Refusal codes
are allowlisted. The fixture readbacks include identities, title, notes,
times, time zone, recurrence and invitee count/status. EventKit's
`not_provided` with absent invitees means no attendees; `not_requested` does
not establish absence.

Cleanup runs even after failure or a keyboard interrupt. It reads back before
deleting the run's remaining occurrences, including the first sibling left
by step 4, then reads again to require an empty fixture. A refused or unknown
write fails its step. Saves are never replayed; lost receipts are followed by
request identifier and unknown outcomes are resolved by a fresh read before
cleanup. If the daemon cannot confirm an outstanding operation or read the
fixture, the report fails cleanup and lists known remaining identities and
unresolved requests; those identities are not a confirmed current read.
Keep that report for manual resolution. The script never deletes the calendar.

This checks the daemon/EventKit path only. It does not exercise an assistant
turn or inspect Calendar.app. The reviewer still performs the Calendar.app
comparisons below (the script cleans up immediately, so use a separate manual
pass for those comparisons), records app/build versions and granted permission,
and has the owner remove the disposable calendar for step 5. Fake-socket
tests in `scripts/tests/test_calendar_acceptance.py` run in the Linux gate;
they do not count as EventKit acceptance.

## Manual procedure

1. Through the assistant's existing calendar tool, create **Cedar budget review**
   in that calendar, 10:00–10:30, with description **Invented acceptance event**.
   Read back the returned event identifier and the surrounding day using
   `orbit_calendar`. Confirm calendar, identifier, title, start/end, time zone,
   description and absence of invitees in both the tool result and Calendar.app.
2. Create **Cedar recurring review**, three daily occurrences, 11:00–11:30.
   Read all three occurrences and record their occurrence identifiers and dates.
   Move only the middle occurrence to 14:00–14:30 using the existing write tool
   with `span: "this"` and the exact occurrence identity returned by the read.
   Read the entire three-day range again. Require exactly three occurrences:
   first and last unchanged; middle at the new time, with no duplicate at 11:00.
   Confirm the same in Calendar.app after it refreshes.
3. Delete the moved middle occurrence with `span: "this"`. Read the three-day
   range again: only the first and last remain, at their original times. Verify
   the standalone budget event is still present.
4. Delete the last remaining occurrence with `span: "future"`. Read again:
   the earlier sibling remains. Then delete the standalone event by its own
   identifier with `span: "this"`; the earlier recurring sibling must remain.
5. Have the owner remove **Cedar fixture** in Calendar.app. Read the range again
   and confirm no fixture events remain. A refused or uncertain write is not
   retried blindly: read back first and resolve its actual scope before cleanup.

Save only synthetic identities and readback fields needed to compare before
and after, app/build versions, granted permission, operation/scope, timestamps,
actual result and any refusal code/numbers. Do not save other calendars or a
whole accessibility tree. Record each step as passed, failed or not run;
fake-backend and reader tests do not count as EventKit write acceptance.

Expected manual cost: about 10–15 minutes, excluding permission/setup and sync.
