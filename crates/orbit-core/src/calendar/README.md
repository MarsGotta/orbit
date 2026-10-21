# Pure calendar contract

Mirrors the DTOs in `OrbitNativeProtocol/CalendarModels.swift`, `CalendarEvents.swift`
and `OrbitNativeCalendar/CalendarBackend.swift`; the wire format is in
`contracts/native-v1.md`, under “Local calendars”.
Contains no effects, persisted selection, clock, SDK or recording state.

## Admitting a response

1. The actor must correlate command/response, capture the local start **before
   sending**, retain current host facts and reject responses from another
   operation. Decoders do not perform this authentication.
2. `decode_status`, `decode_inventory`, `decode_scope`, `decode_snapshot` bound
   bytes before deserialization and validate structure. `decode_scope` accepts
   the actual `{scope:…}` result. Absent optional fields are omitted; `null`,
   unknown fields and duplicate fields are rejected. There are no defaults
   granting consent for invitees or URLs.
3. `CalendarInventory::validate_current(facts)` admits only a recent read from
   the current host/generation, with full permission and no privacy stop.
   `validate_selection(request, elapsed)` requires the exact IDs and the
   inventory's exclusive expiry, without selecting by title, sorting or truncating.
4. `CalendarScope::validate_confirmation` compares the result with the complete
   pending inventory and selection, including order and both consents.
   The actor consumes that inventory once; pure validation neither issues nor
   reactivates a scope.
5. `CalendarSnapshot::validate_response(scope, query, facts)` requires **current
   values retained by the actor**, not scope/query values read from the JSON itself.
   The window does not shift. A valid empty read is still only a bounded local
   read; it does not prove synchronization, cancellation or attendance.

`CalendarReadFacts` is not deserialized. Its time is a local duration since the
correlated send, `None` if the clock is unavailable or went backwards. The response
limit is <5 s. Selection also supplies elapsed time since the inventory request
was sent and compares it with `expires-observed` (<30 s, or its shorter TTL).
Absolute Rust Instant, UTC and `mach_absolute_time` values are not compared.
This conservative bound does not renew the read on receipt.

The actor must retire the scope on `calendar_invalidated`, a new inventory,
disconnection, EOF/ready, or a permission or privacy change. The native controller's
internal invalidation identity is not a new wire field: even if calendars, host UUID
or privacy generation match, a retired scope is not readopted. DTOs alone do not
prove this lifecycle.

## Data and limits

Up to 128 calendars, 16 selected, 200 occurrences, 64 invitees per event,
an 8-day window and 256 KiB per complete result. Excess is rejected without
truncation. UTF-8 bytes, original/current dates and civil duration are preserved;
a day is not forced to 24 hours and a time zone is not reinterpreted. Names and
titles are untrusted content: presentation must escape control characters.

Optional states distinguish not requested, not provided and provided. Without
consent for a category, both data and a claim that the category was queried are
rejected. `provided` requires a present field; an empty invitee list differs from
an omitted field. An email, if present, must match a recipient's mailto candidate;
if absent, none is fabricated. No RSVP, URI or name confirms a person's identity,
voice, presence or meeting link.

## Evidence for this unit

A previous local check passed 25 Rust tests with
`cargo test -p orbit-core calendar --offline --locked`. These cover synthetic wire
data compatible with Swift fields, clocks with different origins, generations,
expiry, previous scopes, optional fields, limits and preservation. That check did
not execute the Swift encoder or EventKit: cross-platform execution equivalence
and physical TCC/calendar acceptance remained pending. This unit does not connect
the daemon/TUI or send calendars to Hermes, and does not authorize recording or
publishing anything.
