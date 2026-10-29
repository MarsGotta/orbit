# Disposable-calendar write acceptance

Manual only; not run by the harness or the Linux gate. Obtain the calendar
owner's explicit yes before doing this procedure. macOS Calendar permission
is the product's consent; this procedure adds no product confirmation or switch.

Use Orbit Pruebas, an isolated hub/assistant profile and a new, empty, writable
calendar named **Cedar fixture**. The owner must select that calendar's identifier
from the permitted calendar read. Never use a default calendar implicitly.
Do not invite anyone or attach real addresses, links or documents. Use a future
local date and record its date, time zone and UTC offsets in the private result.

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
