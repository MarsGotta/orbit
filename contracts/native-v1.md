# Orbit native protocol v1

Two private inherited pipes carry UTF-8 NDJSON; there is no network port.
The production daemon runs inside `Contents/MacOS/Orbit` and starts
`Contents/MacOS/OrbitNative --stdio --managed`. Helper stdin receives commands;
stdout emits events; stderr is reserved for content-free diagnostics.
`NativeLimits`/`NativeChannel` bound input/output. A valid header with malformed
body yields `invalid_native_command`; an unreadable header or saturated channel
stops capture without writing the privacy latch.

The managed helper does not outlive the daemon. EOF stops capture/playback
without writing the latch, releases shortcuts and exits within 2 seconds
(`NativeQuitPolicy.orphanExitGrace`), even if cleanup has not completed.
Orderly daemon shutdown sends `shutdown`, closes helper stdin, drains stdout,
then allows 3 seconds (`HELPER_EXIT_GRACE`) before killing it.
EOF revokes capabilities; it does not waive the daemon's durable shutdown.

The app alone owns an icon and stable identity. Managed mode retains shortcuts
and local privacy, creates no menu item/login item/instance lock, accepts local
daemon `rearm` and activates the app before TCC requests. Bare `--stdio` is
retired. `--stdio --test-state-directory <path>` is an isolated fixture mode,
without data acquisition. No-argument launch is refused. The helper never
hosts or launches a daemon.

## Envelope and connection

Command: `{"v":1,"id":"request-id","command":"status"}`. `id` is nonempty and at
most 128 bytes; `v` must be 1. Response shape:
[response fixture](fixtures/native/events/response.json). Failures carry
`ok:false` and `error:{"code":"privacy_stopped","message":"…"}`.
Spontaneous events include `v`/`event` without `id`.

The daemon sends `hello` on connection and `heartbeat` every 2 seconds, from
a thread of its own. Silence is counted only while the helper runs: a watchdog
tick more than 2 seconds after the previous one restarts it, so beats queued
behind a held main thread are read before anything is decided. After 6 seconds
without heartbeat the channel is unhealthy. With nothing being captured the
helper stops playback with `capture_stopped`, without latching. A recording or
a voice note in progress keeps capturing: up to five minutes of two-track audio
wait in the helper, in order, and what does not fit is one `gap`
(`daemon_backpressure`). Only the person, end of input, the privacy stop and
sleep stop a capture. Any later command from the daemon, a heartbeat or any
other, restores channel health; it never resumes a stopped activity.

**Only explicit user privacy stop writes the durable latch.** Latching reasons
are `local_hotkey` (`⌃⌥Esc`) and `daemon_requested` (the daemon relays its stop).
Swift `StopPolicy.latchingReasons` and Rust `LATCHING_HELPER_CODES` are compared
against [the shared fixture](fixtures/native-latching-stops.json).
Other causes—session inactivity, audio-route change, processing loss, app audio
loss, silent/disconnected daemon, shutdown/quit—stop capture/playback, invalidate
epoch-dependent activity and leave the latch unchanged. It persists until
explicit local rearm. `ready` reports version, launch mode and observed audio/
selection availability, not implementation negotiation: the daemon launches
the helper from its own bundle.

## Commands

Full shapes: [commands](fixtures/native/commands/) and
[results](fixtures/native/results/). Swift decodes a typed variant once; channel,
router, host and calendar bridge receive that value. Refusals keep request ID.

- `hello`/`heartbeat` renew channel health, as any command does; `status` reads without TCC.
- `permissions` asks only the system permission explicitly requested by the app.
- `audio_start` opens microphone/system audio; app capture includes its PID.
- `audio_stop` drains that source. `audio_wake_phrase` follows the live meeting
  microphone even if a voice note originally opened it. Its optional
  `assistant_names` lists every agent's name, and the helper listens for all
  of them; its optional `assistant_name` is the default agent's name, which a
  helper uses alone when `assistant_names` is absent or empty, defaulting to
  `Deimos` when both are omitted. A later order without either keeps the
  names the helper had.
- `playback`/`playback_clip` play; `playback_stop` cancels the playback generation.
- `screen_sources`/`screenshot`/`selected_text` use the one-use bindings below.
- `calendar_sources`/`calendar_snapshot` read; `calendar_save`/`calendar_delete`
  write; `calendar_disconnect` retires a pending reply without revoking TCC
  or pretending the physical SDK call finished.
- `privacy_stop` latches; `capture_stop` stops without latching.
- `rearm` arrives only on the local managed daemon's private pipe with
  `RearmReadiness`; isolated fixtures and remote traffic have no such authority.
- `shutdown` drains and exits before orderly parent EOF.

### Local calendars

Full macOS Calendar permission is the only consent. The helper reads every
calendar (`predicateForEvents(…, calendars:nil)`) without selection, copy or
calendar-count ceiling, and pages the results. The same permission permits
creating/moving/editing/deleting editable events from Orbit, without another
confirmation.

The backend is present when the bundled helper reports `ready`.
`state.calendar`/[disconnect](fixtures/native/results/calendar_disconnect.json)
describe authorization, physical pending read and optional failure.
Unknown authorization is neither granted nor denied.

`calendar_sources` returns a [page](fixtures/native/results/calendar_sources.json).
Each row also has `color` (`#RRGGBB`, sRGB from `cgColor`, absent if unavailable),
`can_edit` (`allowsContentModifications`) and `is_default`.
`next_after_index` exists until the last page. Enumeration order is deterministic;
an index outside the current enumeration yields `calendar_invalidated`.

`calendar_snapshot` returns:
`read_id`, `native_instance_id`, `privacy_generation`, exact requested `window`,
`clock_domain:mach_absolute_time`, `capture_started_monotonic_ns`,
`capture_completed_monotonic_ns`, `observed_at_unix_ms`, `system_timezone`,
`provider_sync_status:unknown`, `enumeration:bounded_local_read`, `events`,
`omitted:[{reason,count}]`, optional `oversized`, `after_index`, `total` and
`next_after_index` while events remain.
Whole events fill `CalendarLimits.pageEventBytes` (256 KiB), at least one per
nonempty page, never over `maximumPageBytes` (768 KiB), below the daemon's
1 MiB calendar response line. Events over `maximumEventBytes` (704 KiB) are
named in `oversized` and omitted without truncation; the rest arrives.
The daemon walks arbitrary ranges in 31-day windows (`READ_WINDOW_MS`) and
never combines pages from different enumerations.

Each event contains `observation_id` (new for that observation), exact `calendar`,
`calendar_item_id`, optional `event_id`/`original_occurrence_unix_ms`,
`start_unix_ms`/`end_unix_ms`, optional `event_timezone`, `interpreted_timezone`,
`is_floating`, `is_all_day`, `is_detached`, `title`, `status`,
optional `last_modified_unix_ms`/`invitees`/`event_url`,
`invitees_status` and `event_url_status`.
Additional fields:
`location?`, `notes?` (at most 32 KiB, `maximumNotesBytes`),
`notes_left_out?:{bytes,limit}` for longer notes without dropping the event,
`organizer?` (same participant shape as invitees),
`availability:busy|free|tentative|unavailable|not_supported`,
`recurrence?` (first rule:
`{frequency:daily|weekly|monthly|yearly,interval,
days_of_week?:[{day,week?}],end?:{until_ms}|{count},rules_count?}`;
`rules_count` only when more than one), and `can_edit` (calendar permits changes
and another person did not organize the event).

`alarms_minutes?` contains editable reminders relative to/before start, in minutes
before, bounded by `maximumAlarms`/`maximumAlarmMinutes`. Out-of-bound lists are
not supplied. `alarms_not_editable?` counts reminders Orbit cannot edit
(absolute/location-based/after-start/outside bounds), never 0.
Writing `alarms_minutes` replaces editable alarms while preserving the others.
Older helpers omit these fields: absence means unread, never empty/false.

Invitees/URL are always requested under macOS consent, never `not_requested`.
All EventKit invitees travel. EventKit IDs are local and may change after a full
sync; `observation_id` is not a person or permanent series ID. Floating events
report the observed system timezone. Valid year 1–9999 dates use milliseconds
truncated toward zero.

`status:none|confirmed|tentative|canceled|unknown` is informational except explicit
cancellation. A local read does not guarantee iCloud/Google/Exchange sync.
RSVP, room or invitation proves neither attendance nor voice identity.
`event_url` is never opened. Notes/location are bounded reads, never an automatic
search for conferencing links or contacts.

Field bounds exist on both sides: title, calendar/account/invitee names and
URL/invitee addresses ≤96 KiB (`maximumTitleBytes`/`maximumURLBytes`), fitting
a bridge page; IDs ≤1024 bytes (`maximumLabelBytes`), timezone ≤128 bytes
(`maximumTimezoneBytes`) validate shape. Some legacy copies are maintained
manually; register/guard shared bounds before changing them.
An oversized field/event does not stop the read: list it in
`oversized:[{calendar:{calendar_id,source_id},calendar_item_id?,
start_unix_ms,end_unix_ms,field,bytes,limit}]` and return the rest uncut.
Errors are explicit: `calendar_limit_exceeded`, `calendar_event_too_large`,
`calendar_busy`, `calendar_read_expired`, `calendar_invalidated`,
`calendar_permission_required`, `calendar_read_failed`,
`calendar_clock_unavailable` and `invalid_calendar_request`.

There is one physical EventKit slot, separate from audio. Reads expire 5 seconds
after admission (`readLifetimeNanoseconds`); permission dialogs after 120 seconds.
Timeout/stop does not release the slot before actual SDK return.
Spontaneous `calendar_invalidated` contains a code, no calendar content.
Isolated fixture mode accepts none of these commands or capabilities.

The daemon correlates request/host/generation and queues app-week, assistant and
calendar-list reads. It stores no calendar; projection is ephemeral IPC state.
Physical TCC/EventKit behavior requires Mac acceptance.

#### Write events

`calendar_save` creates/changes; `calendar_delete` deletes.
Identify an occurrence by `calendar_item_id` + `occurrence_start_ms` (original
`occurrenceDate`), never transient `observation_id` or optional `event_id` alone.
`span:this|future` maps to `.thisEvent`/`.futureEvents`; entire series means
`future` from the first occurrence, not a third value.

Fixtures: [create](fixtures/native/commands/calendar_save.json),
[change](fixtures/native/calendar/commands/calendar_save-change.json),
[delete](fixtures/native/commands/calendar_delete.json).

Without `calendar_id`, creation uses the default calendar. All `changes` are
optional; absence leaves a field unchanged. Empty location/notes/URL clears it.
`remove_recurrence:true` removes repetition; changing recurrence requires
`span:future`. `alarms_minutes` replaces editable reminders.
Save returns `{write_id,native_instance_id,privacy_generation,system_timezone,
occurrence}`, with the changed occurrence in read shape. Delete returns
`{write_id,native_instance_id,privacy_generation,calendar_item_id,
occurrence_start_ms,span}`.

Refusals use `ok:false` with `error:{code,message,detail}`:

| Code | Detail |
|---|---|
| `calendar_event_not_found` | `calendar_item_id`, `occurrence_start_ms` |
| `calendar_not_editable` | `reason:calendar_read_only\|organized_by_someone_else`, `calendar_id` |
| `calendar_invalid_change` | `field`, `problem` (`empty`, `too_large`, `too_many`, `end_before_start`, `invalid_date`, `invalid_identifier`, `invalid_value`, `unknown_calendar`, `no_default_calendar`, `unknown_time_zone`, `invalid_url`, `recurrence_needs_future_span`, `nothing_to_change`) and applicable `bytes`/`count`/`limit`, `value`, `start_ms`/`end_ms` |
| `calendar_permission_denied` | Actual EventKit `authorization` |
| `calendar_save_failed` | EventKit `error_domain`/`error_code`, never error text |
| `calendar_write_outcome_unknown` | `cause`: `write_deadline`, `calendar_invalidated` (withdrawal), `privacy_stopped`/`privacy_generation_changed` (receipt withheld after save), `calendar_permission_required`, `native_instance_changed`, `daemon_disconnected`, `result_not_encodable` or `saved_but_not_readable` |

Rust validates input; the helper decodes its typed shape and verifies actual
editability, organizer and dates before EventKit mutation. Refusal names the
cause without changing anything. Writes share the one EventKit slot and expire
after 5 seconds (`writeLifetimeNanoseconds`). Deadline/withdrawal/mid-write stop
yield `calendar_write_outcome_unknown`, never a read-expired/invalidated result,
because the write may have completed. The daemon never resends it; it rereads
and reports uncertainty.

`EKEventStoreChanged` does not withdraw its own in-flight write; it emits
`calendar_invalidated` to refresh the UI. `maximumNotesBytes`, `maximumAlarms`
(16), `maximumAlarmMinutes` (40320) and `writeLifetimeNanoseconds` have Rust
counterparts in `orbit_core::calendar`; legacy copies are maintained manually.
Failed `save`/`remove` or a partially rejected change calls `EKEventStore.reset()`
so unsaved in-memory mutations cannot leak into reads or a later commit.

Mac acceptance must check organizer `EKParticipant.isCurrentUser` with real
account aliases: Google/Exchange aliases may misclassify the user's own event.
Do not change this rule without measurement. Calendar fixtures include one
refusal per code and result shapes under `fixtures/native/calendar/`.

### Observed window inventory and screenshot

`screen_sources` has no desktop filters and never activates an app.
See [request](fixtures/native/commands/screen_sources.json). The inventory retains
one-use window bindings. Fixtures describe shape, not a performed capture.

UUIDs are canonical 36-character lowercase strings; uppercase is syntactically
valid, but received binding equality is exact and representation-sensitive.
`privacy_generation`/times are UInt64; bound `window_id` is positive UInt32,
`application_pid` positive Int32. Unidentified candidates may show PID 0 but
carry no authority. A lowercase SHA-256 hashes the internal descriptor: PID,
opaque app instance, exact observed `launchDate` bits, bundle, full title,
bounds and level. The daemon treats it as opaque, never rebuilds it from
truncated titles.

A live retained `NSRunningApplication`, numeric launch date and bundle matching
the SCK owner are required. Otherwise return metadata with `binding:null` and
`unavailable_reason:window_identity_unavailable`. Unknown/unrepresentable bundle
uses an empty string. PID/name/bundle alone never establish identity.
`application_instance_id` is a local observed-inventory UUID, neither WindowServer
ID nor date, and is not reused between inventories.

At most 200 rows/160 KiB result JSON; `omitted_count` counts all omissions.
UTF-8-safe metadata limits are application name 128 bytes, title/bundle 256.
Internal titles over 16384 bytes or nonfinite/uncapturable dimensions prevent
binding creation. Metadata is untrusted application content, not user instructions.

The host clock is read before SCK; TTL begins then and is never renewed on SDK
completion. Maximum 30 seconds, upper bound exclusive. Backwards/unrepresentable
clock invalidates the operation. A new inventory retires the previous one.
Only one physical SCK enumeration may be pending; timeout/stop keeps its slot
until SDK return (`screen_inventory_busy`). Stop/disconnect/logout invalidate
all selections immediately; rearm never restores them. `state` exposes current
instance/generation. `selected_text_available` reflects `AXIsProcessTrusted()`
without requesting permission or certifying non-sensitive content/selection support.

See [screenshot request](fixtures/native/commands/screenshot.json). The token is
shared with `selected_text` and permits only one operation. It is consumed before
suspension/read attempt; failure does not restore it or permit reuse as fallback.
The helper retains the original SCWindow/filter/app. Pre/post SCK/process checks
require the same owner, launch date, bundle and full descriptor; disappearance or
changed PID/instance/title/bounds/level discards bytes.
`SCContentFilter(desktopIndependentWindow:)` and
`SCScreenshotManager.captureImage` capture without focus/input/clipboard/desktop
fallback. Preflight never requests TCC; it cannot promise that the OS itself will
never display another dialog.

Results include exact requested `binding`, `clock_domain`,
`capture_started_monotonic_ns`/`capture_completed_monotonic_ns`,
`mime_type:image/png`, `width`, `height`, `data_base64` and `source:native_window`.
Times bound admission/checking/serialization, not exact pixel time or UTC;
both must fit inventory/operation validity. The daemon binds the receipt to its
request/operation ID, generation/deadline, validates PNG/binding and hashes bytes
before delivery.

PNG capture lasts at most 10 seconds from token consumption, or until earlier
inventory expiry. Timeout withdraws delivery authority, not the physical slot:
SDK cancellation is not established. One physical PNG/AX operation remains
pending until return/checking; others get `screenshot_busy` even after stop.
A missing callback can keep the backend busy until process restart.
Privacy/heartbeats remain independent.

Policy errors: `invalid_window_binding`, `screen_selection_unavailable`,
`screen_selection_expired`, `privacy_stopped`, `window_identity_changed`,
`screenshot_busy`, `screenshot_expired`. SDK/format/permissions can yield
`screenshot_sdk_failed`, `invalid_screenshot_dimensions`, `screenshot_too_large`,
`screen_permission_required` or a generic native error. EOF/timeout/unknown ID
does not mean no read occurred: discard the receipt, record cancellation/
uncertainty and never retry automatically.

`observed_owner_and_window` does not certify an immutable window generation.
Apple exposes no such SCWindow identity; replacement within the same PID and
identical ID/metadata between checks may be undetectable. AppKit properties
update through the run loop. Pure token/time tests do not establish visual identity,
TCC or absence of focus theft. CUA AXValue/AXSelected are not semantic selected
text; there is no copy/paste fallback.

Sources: [SCWindow](https://developer.apple.com/documentation/screencapturekit/scwindow),
[window filter](https://developer.apple.com/documentation/screencapturekit/sccontentfilter/init(desktopindependentwindow:)),
[screenshot](https://developer.apple.com/documentation/screencapturekit/scscreenshotmanager/captureimage(contentfilter:configuration:completionhandler:)),
[app instance](https://developer.apple.com/documentation/appkit/nsrunningapplication),
[launch date](https://developer.apple.com/documentation/appkit/nsrunningapplication/launchdate).

### Semantic selected text

`command:selected_text` uses the same full `window_binding`. Screen Recording
and Accessibility must be granted. No permission request, input, activation,
AX setter, clipboard read or `kAXValueAttribute` is used.
See [request](fixtures/native/commands/selected_text.json) and
[result](fixtures/native/results/selected_text.json); fixture text is synthetic.

Times bound admission/queries/validation, not an atomic selection instant or UTC.
Both precede inventory expiry and duration is strictly below 5 seconds, including
pre/post SCK, AX/CG and final privacy/instance checks. A serial worker executes
blocking calls off the main thread. Each acquired AX object gets a 0.1-second
timeout, without assuming propagation to equivalent objects. At most 256 AX
access/acquisition/messaging calls; deadline/revocation checked before/after each.
Local CF conversions and permission queries are not target-app calls.

Before reading content require:

1. Same retained SCK descriptor/owner/bundle/launch date. Collect the complete
   PID window set (1–32), including hidden/minimized windows, not a partial page.
   Reject more than 200 total SCK windows before traversal.
2. Public CG all-window query, excluding desktop, rejects over 200 rows and
   matches PID set, IDs and full titles. `kCGWindowBounds` uses the public
   top-left primary-display coordinate space also documented by AXPosition;
   never assume `SCWindow.frame` shares it.
3. `AXUIElementCreateApplication(pid)` count/copy/count of all AXWindows (1–32).
   Require exactly one matching PID/title/bounds in both CG and AX. Do not
   filter hidden peers, use private AXWindowNumber or take the first match.
4. That application's AXFocusedUIElement, not the system's, retains PID and
   AXWindow `CFEqual` reference without focus changes. Role is TextField/TextArea;
   subrole must be an explicit nonempty string other than SecureTextField.
   Missing/unsupported/error/incomplete classification refuses before reading
   content. This reduces compatibility and does not make an uncooperative app trusted.

First read AXSelectedTextRange: nonnegative location, length 1–4096, overflow-safe
sum ≤Int64.max within AXNumberOfCharacters. A supported plural range attribute
must contain exactly the same single range; missing/inconsistent/multiple ranges
refuse. Only `attributeUnsupported` permits the singular-only contract.
AXSelectedText must have exactly that UTF-16 length and equal AXStringForRange.
Reread AX set/identity, focus, window, classification, range and both text getters,
then repeat CG/SCK checks.

Reject empty text, NUL, isolated surrogate, repaired Unicode or over 16384 UTF-8
bytes. Compare exact UTF-16 units, without canonical normalization/truncation.
TAB/CR/ESC/bidi controls remain data, sanitized only by consumer presentation.
CFString/array receive bounds do not promise to control framework allocations
before return.

PNG and AX share the physical slot (`screenshot_busy`), even after consumer
timeout/stop. No accumulated workers or release on revocation. Late results
cannot revive tokens. Refreshing inventory revokes delivery and future getters
of the old worker; refusing a second `screen_inventory_busy` enumeration does
not invalidate the first. A lost AX return can require helper restart.

Errors: `accessibility_permission_required`, `selected_text_unavailable`,
`selected_text_empty`, `selected_text_not_exposed`,
`selected_text_classification_incomplete`, `selected_text_secure_field`,
`selected_text_window_ambiguous`, `selected_text_identity_changed`,
`selected_text_invalid_range`, `selected_text_invalid_text`,
`selected_text_changed`, `selected_text_limit`, `selected_text_expired` and
common token/privacy/clock errors.
`selected_text_empty` proves a valid zero-length range before reading content.
`selected_text_not_exposed` means a required attribute is unsupported/unimplemented,
not that selection is absent. `selected_text_unavailable` covers other failures,
including `noValue`, leaving selection unknown. Missing screen permission is
`screen_permission_required`, including during text reads.
`native.log` retains named check outcomes and numeric identities, never titles,
text or their hashes. EOF/unknown ID/timeout authorize neither retry nor fallback.

`unique_owner_title_bounds` is observable expiring evidence, not a public
AXWindow→CGWindowID link or proof of the same within-PID incarnation. Reads are
non-atomic; changes between identical observations may be undetectable.
`not_certified` means rejecting SecureTextField does not prove absence of secrets.
Pure tests do not establish physical app/TCC/focus compatibility.

Sources: [AXPosition](https://developer.apple.com/documentation/applicationservices/kaxpositionattribute),
[CG bounds](https://developer.apple.com/documentation/coregraphics/kcgwindowbounds),
[CG list](https://developer.apple.com/documentation/coregraphics/cgwindowlistcopywindowinfo(_:_:)),
[bounded AX read](https://developer.apple.com/documentation/applicationservices/1462060-axuielementcopyattributevalues),
[per-object timeout](https://developer.apple.com/documentation/applicationservices/1459345-axuielementsetmessagingtimeout),
[AXWindow](https://developer.apple.com/documentation/applicationservices/kaxwindowattribute),
[selected text](https://developer.apple.com/documentation/applicationservices/kaxselectedtextattribute),
[range](https://developer.apple.com/documentation/applicationservices/kaxselectedtextrangeattribute),
[substring](https://developer.apple.com/documentation/applicationservices/kaxstringforrangeparameterizedattribute),
[secure field](https://developer.apple.com/documentation/applicationservices/kaxsecuretextfieldsubrole).

### Assistant folders

These five commands never take the general command lane: a panel waits for a
person and a search walks the disk, and neither may delay a recording. The
daemon sends one of each at a time. The privacy latch refuses them.

- `assistant_folder_pick {}` opens `NSOpenPanel` for one folder and returns
  `{state:"chosen",path,bookmark_base64}` (a security-scoped, read-only
  bookmark; a plain one where macOS makes no scoped bookmark) or
  `{state:"cancelled"}`. A second panel while one is open is
  `assistant_folder_panel_busy`.
- `assistant_folders {folders:[{id,bookmark_base64}],path?}` opens each
  bookmark (starting its security-scoped access for this order only) and
  answers `{folders:[{state:"accessible",id,path,renewed_bookmark_base64?} |
  {state:"unavailable",id,cause}],requested?}`. `path` is canonical
  (`realpath`); a stale bookmark is renewed. `requested` is
  `{state:"found",path,kind,size_bytes,modified_ms?}`, `{state:"not_found"}`
  or `{state:"unavailable",cause}`. The helper does not decide containment:
  the daemon does, on these canonical paths.
- `assistant_files_list {folder,path,after_index,max_entries}`,
  `assistant_files_search {roots:[{folder,path}],query,content,limits}` and
  `assistant_files_read {folder,path,read}` run only on a path the daemon
  admitted. The helper first checks the path is still canonical
  (`realpath(path) == path`) and reads with `O_NOFOLLOW`; otherwise it refuses
  `changed`. Listings and searches use `lstat` and never follow a link. `read`
  is `{as:"text",after_index,max_bytes}`, `{as:"pdf",after_page,after_index,
  max_bytes,max_pages}` or `{as:"image",max_png_bytes,max_width,max_height}`;
  every bound travels in the command, so the helper keeps no copy of it.

Refusals use `ok:false` with `assistant_file_unavailable {cause}` or
`assistant_folder_unavailable {id,cause}`, the causes of `hub-v1.md`
(«Assistant folders»). Fixtures: `commands/assistant_*.json` and
`results/assistant_*.json`. No path or content is logged.

### Attachments: a window's text and a photo as JPEG

Two orders of the composer's «Adjuntar». Like the folders, neither takes the
general command lane, and the privacy latch refuses both.

- `window_text {application_pid,window_id,limits:{max_bytes,max_nodes,
  deadline_ms}}` reads the text Accessibility shows in the window «Adjuntar
  ventana» just captured, when the engine is not running (with the engine,
  the daemon asks its `get_window_state` for the tree instead). The window is
  found among the app's `AXWindows` by the frame CoreGraphics gives
  `window_id`; exactly one must match (`window_text_window_gone`,
  `window_text_window_not_found`, `window_text_window_ambiguous`). The walk is
  read-only and depth first, one line per element that shows a title, value
  or description; a secure field and its children are never read. It answers
  `{text,stopped?}`, where `stopped` is `bytes`, `nodes` or `deadline` when a
  bound cut it. Without Accessibility it refuses
  `accessibility_permission_required`. The text is never logged.
- `photo_jpeg {source_path,destination_path}` reads a photo no agent reads
  (an iPhone's HEIC) with ImageIO and writes it as JPEG at
  `destination_path`, a private folder of the daemon's: at its full size,
  turned as its EXIF says, without its metadata. It answers `{width,height}`
  or refuses `photo_unreadable` or `photo_jpeg_failed`.

Fixtures: `commands/window_text.json`, `commands/photo_jpeg.json` and
`results/window_text.json`.

## Stop barrier and preceding audio

`privacy_stop` or nonlatching `capture_stopped` is immediate and includes
`drain_complete:false`. It revokes daemon activity but **does not authorize
retiring IDs or sealing tracks**. ACKs for `privacy_stop`, `capture_stop`,
`audio_stop` and `shutdown` carry `drain_complete:true` only after producers stop,
callbacks finish and all preceding PCM/gaps enter the same FIFO writer before
the ACK. A failed prior write prevents a successful ACK. Concurrent stops share
one pending operation; removing a capturer reference does not complete a second stop.

The cutoff is fixed before waiting on SDK/disk. SCK uses
`CMClockGetTime(stream.synchronizationClock)` in PTS domain; the ring keeps samples
whose start precedes cutoff, even from late callbacks, and trims crossing blocks.
Admission remains open until stopCapture finishes and a FIFO marker crosses the
callback queue. Microphone uses host clock and removes its tap. Atomic counters
cover callbacks inspecting format/clock and copies already started; drain waits
for both to reach zero.

CMTime converts to integer nanoseconds by truncation; frame membership uses
sample start. Boundary quantization is under 1 ns, not measured physical
button/driver/output latency.

Invalid clock, unfinished queue, SDK error or a rejected callback that may contain
pre-stop audio emits an explicit gap/error, never successful ACK.
`capture_clock_unavailable`/`capture_callback_at_stop_boundary` carry
`pre_stop_possible:true,duration_unknown:true`. Capture remains stopped; uncertain
drain never resumes itself. Ordinary overflow is still a known gap:
`drain_complete` proves closure/PCM-gap ordering, not lossless recording.

Menu/protocol closure waits for the writer, then requests cooperative daemon
exit. Disconnect/forced death/broken pipe cannot promise full delivery. Explicit
privacy stop stays latched even if pending audio can no longer be delivered.

The microphone uses the macOS default device without changing global settings.
Sleep/logout stop capture (`capture_stopped`) without latching. A microphone
`audio_start` carries `purpose`, `meeting` or `voice_note`; absent means unknown.
`audio_purpose {stream_id,purpose}` updates it while both share the microphone:
`voice_note` while a voice note is active and, when it ends, `meeting` if the
meeting goes on. It does not depend on the wake phrase. It is an urgent order
that neither opens nor reconfigures devices.

Every configuration notification is logged by the helper as
`audio_configuration_changed`, with the previous and current input/output device
IDs (or `unknown`), the action, its cause and the purpose, never device names.
With the same IDs nothing restarts or stops. An unknown route never authorizes
resuming. Only an unprocessed meeting goes on when a device changes (detailed
below). A voice note alone stops once with `audio_device_changed_voice_note`;
a voice note dictated over a meeting (the microphone was opened or marked
`meeting`, or the system's audio records) restarts the meeting's microphone,
and the daemon cancels only the note on the route `gap` and says so
(`voice_note_cancelled_by_audio_route`). An unknown purpose stops with
`audio_device_changed_purpose_unknown`.

One track's failure never stops the other. When the system's audio stream
stops (`system_audio_stream_stopped`) while the microphone records, the helper
starts it again once under the same `stream_id`, within 10 seconds, after a
`gap` with `reason:"system_audio_restarted"` that tells the daemon the
sequence starts over. If it does not come back, or the microphone fails while
the system's audio records, that track drains and ends with a `gap`
`reason:"track_stopped"`, after an `error` naming the cause; the daemon keeps the
recording and says which track stopped (`track_stopped`). A later
`audio_stop` for that stream answers `drain_complete`. With nothing else
live, the whole capture stops with `capture_stopped` as before. While a
capture is in progress the helper holds a
`userInitiatedAllowingIdleSystemSleep` activity so macOS does not nap it. The privacy
stop and a close or drain in progress never authorize a restart.

`audio_start` reports effective format/scope. `application` uses SCK app filtering,
not a tab, and ACK includes PID/bundle. `system` uses
`SCContentFilter(display:excludingApplications:exceptingWindows:)` excluding
Orbit's bundle and reports no app identity. Both use
`excludesCurrentProcessAudio=true` to exclude helper audio too.
Microphone retains device rate; daemon STT resampling happens outside callbacks.
Playback accepts 16000, 24000, 32000, 44100 and 48000 Hz; rate cannot change with
pending playback frames. A persistent-phase linear resampler converts TTS outside
callbacks to a fixed mono 48000 Hz player without reconnecting the graph.
Only upsampling occurs, with under one source-sample interval deferred at the end;
tests do not establish acoustic quality. Retired IDs reject late packets;
the session restarts at 10000 generations.

## Processed voice session

One `SharedAudioSession` owns AVAudioEngine/input/player. Microphone uses processed
input; all TTS follows player→mixer→output in the same engine. With the engine
stopped, configure `setVoiceProcessingEnabled`, disable bypass/AGC and read input/
output state and graph connections before/after starting. No second capturer or
echo reference inferred from correlation/energy.

`ready`/`state` advertise `supported_audio_processing:["unprocessed","voice_processing"]`:
implementations to attempt, not verified hardware. `voice_processing_available`/
`playback_reference_available` begin false and become true only with running
engine and valid readback. `state.audio_processing` includes `mode`, `enabled`,
`bypassed`, `agc_enabled`, `playback_reference`, `pipeline_id`, `engine_running`,
`render_sample_rate:48000`. Status reads do not start audio/request the microphone.

An explicit call attempts `audio_start` with `processing:voice_processing`.
The daemon stays Starting until ACK validates these properties; the mode list
alone cannot authorize VAD. ACK and every `mic_local` block contain immutable
session provenance; see [audio ACK](fixtures/native/results/audio_start-mic.json).
This is a required shape, not hardware evidence.
Unprocessed mic uses `processing:unprocessed`, `processing_enabled:false`,
`processing_bypassed:true`, `playback_reference:none`. SCK remains unprocessed.
Consumers preserve provenance in recording/export: a call original is processed
PCM, not raw microphone audio. Silence is delivered; VAD segments STT, never
filters the producer/replaces the original.

Mode changes with active microphone/playback/remote app yield `audio_session_busy`.
Starting remote_app during VoiceProcessing also refuses: ducking/other-app
reference is unresolved. PTT over a raw meeting retains its stream, not call mode.

In processed voice, a real device change stops the session explicitly
(`audio_device_changed_voice_processing`), never restarting it under another
provenance. A one-second watchdog checks the reference, bypass and running
engine. A notification with the same devices has no effect, even if the format
or the engine state changed. This measures neither ERLE nor sample-level echo
cancellation.

## Events

See [audio fixture](fixtures/native/events/audio.json).
PCM is signed 16-bit little-endian mono. `frames` counts track samples;
`sequence` is per stream and may skip after overflow. The callback converts/copies
to a preallocated C SPSC ring (32 blocks, at most 4096 frames), without disk,
network or serialization. A worker emits NDJSON. `gap` carries `stream_id`,
`source`, lost `frames` and `reason`; gaps never imply complete audio.

`gap` with `reason:"audio_device_changed"` on `mic_local` precedes new-route PCM
after draining the old producer. Stream ID/meeting persist; only producer
sequence resets. The daemon seals the old tail to avoid joining across the gap.
Optional `duration_ns` measures old-last-frame-end to new-first-frame-start in
capture time; otherwise `duration_unknown:true`. `frames:0` does not mean zero
duration. New-device PCM converts outside callbacks to retain the stream rate.
Restart errors report `code`, optional `os_domain` (domain name) and
`os_status` (numeric status), never OS error text, then `capture_stopped` with cause. Pending playback cancels
with `playback_cancelled_audio_device_changed`. VoiceProcessing calls still stop
with `audio_device_changed_voice_processing`.

Mic frames add `playback_active` and optional `playback_id`, observed thread-safely
during ring drain (target 10 ms), not sample-exact alignment or proof of hearing.
The player is active after `play()` while dataPlayedBack buffers remain; stop
sets false immediately.

Daemon `recording_signal` derives ephemeral per-meeting/epoch/stream reception
from accepted PCM. It distinguishes sustained low level from missing samples,
without replacing native `no_capture_frames`; it is neither durable receipt
nor capture command.

Microphone timestamps convert AVAudioTime.hostTime through
`CMClockMakeHostTimeFromSystemUnits`. SCK PTS converts from stream synchronization
clock to host clock through `CMSyncConvertTime`; clocks are not assumed equal.
Only valid timestamps use `clock_domain:mach_absolute_time`.
Acoustic alignment/drift require Mac measurement; word offsets/speaker identity
are never invented.

`state` includes `privacy_stopped`, `connected`, `microphone`, `system_audio`,
`playing` and audio-session state. `privacy_stop`/`capture_stopped` include reason
and latch state. `privacy_stopped:true` there is latch evidence adopted by the
daemon. A refusal code `privacy_stopped` alone is not: it also covers cleanup.
A `daemon_requested` stop is a daemon-command echo; if the daemon was not stopped,
adopt an unattributed latch, not a user-authored stop. The daemon sends
`privacy_stop` on `ready` only for an active durable user stop, as that full stop.
Daemon tests cover this. `local_rearm` is the only native evidence allowing
daemon rearm. The daemon also persists its stop before admitting new actions.

`hotkey` carries `action` and `phase:pressed|released`:
Control+Option+Space PTT, +D call, +M meeting, +Escape privacy stop.
Carbon registers specific combinations rather than monitoring all keys.
PTT press immediately cuts playback; the daemon decides turns/safe closure.

`microphone_activity` lists `applications:[{application_pid,bundle_id,name}]`,
sorted by PID, at most 16, empty when none. The helper sends it after `ready`
and on changes, from CoreAudio process objects
(`kAudioHardwarePropertyProcessObjectList`/`kAudioProcessPropertyIsRunningInput`,
macOS 14+). No permission is needed; it says which app uses input, never what
it hears. Helpers/browser GPU/audio processes map to their responsible app;
processes without a visible app are omitted, as is Orbit itself.
Privacy latch stops observation/events; rearm sends a full list.
Core decides call classification/prompts/stops. Before the first observation,
absence means unknown, never “nobody uses the microphone”.

`assistant_question` carries the meeting microphone question:
`stream_id`, `text`, `started_ms`/`ended_ms` (Unix milliseconds at phrase start/
question end) and `assistant_name`, the name heard exactly as
`audio_wake_phrase` gave it, so the daemon sends the question to that agent;
a question without it goes to the open conversation. Phrases at sentence start
are “Oye <name>”, “Hola <name>” and “<name>,”, ignoring case and accents, for
every name. A name can contain multiple words; a longer name is tried before a
shorter one it starts with, so “Oye Atlas Azul, …” is Atlas Azul's.

```json
{"v":1,"id":"wake-1","command":"audio_wake_phrase","stream_id":"stream-1","enabled":true,
 "assistant_name":"Atlas","assistant_names":["Atlas","Vega"]}
{"v":1,"event":"assistant_question","stream_id":"stream-1","text":"¿Qué dijo Lucía?",
 "started_ms":1800000000000,"ended_ms":1800000004200,"assistant_name":"Vega"}
```

Only the name Deimos, whichever agent has it, accepts the
recognition spelling “Demos”, at sentence start followed by a pause/comma.
A listener restarted after an audio route change retains its configured names.
Questions end after 1.5 seconds silence
(`NativeLimits.assistantQuestionPauseMilliseconds`) or 30 seconds
(`maximumAssistantQuestionMilliseconds`), guarded copies of
`MEETING_QUESTION_PAUSE_MS`/`MAX_SPOKEN_QUESTION_MS`.
Local `SpeechAnalyzer`/`SpeechTranscriber` (macOS 26, `es_ES`) consumes the
recording microphone outside callbacks only while recording and rearmed.
Privacy/capture/audio stops end recognition; meeting pause/end disables
`audio_wake_phrase` even if a voice note keeps the stream. Only question text
leaves the helper. The daemon accepts it only from the active meeting's
`mic_local`, never `remote_app`, and submits a meeting-bound turn.

`voice_trigger.state` is `unavailable` (cause, e.g. `macos_version` with
`os_version` or `locale_unsupported`), `idle`, `downloading` (percent; Spanish
model downloads once via AssetInventory), `listening` or `failed` (cause:
`assets_download_failed`, `audio_format_unsupported`, `analyzer_failed`, plus
Apple `detail`, never heard text).
It is sent after ready/on change; before it arrives state is unknown.
Daemon log-only notices are `wake_phrase_listening`, `wake_phrase_off`,
`wake_phrase_downloading`, `wake_phrase_unavailable`, `wake_phrase_failed`,
with cause. Apple detail stays in `daemon.log`.

`notice` reports unsolicited content-free progress:
`wake_phrase_heard`, `wake_phrase_abandoned`, `wake_phrase_question_dropped`
(question not sent because privacy/stream changed). Unknown codes become
`helper_error` with the original code. Abandonment adds
`abandonment:{cause,elapsed_ms,silent_ms}`:
`phrase_revised`, `question_wait_expired` (no question words before silence wait),
`question_too_long` (maximum with no question words, even if voice continues).
When both bounds fire together, silence wait wins. Millisecond-rounded times
come from the audio timeline since phrase/last voice, from policy inputs.
Daemon report logs keep cause/counts, not words. Other notices have no abandonment;
older notices without it remain valid.
Command failures already carry `ok:false`/code in response and log as
`helper_command_failed` with command/code.

`playback_progress` contains playback ID, sequence and played frames confirmed
by `dataPlayedBack`. Cancellation is `state:cancelled,played_frames:null`:
no inference of what a person heard. Late cancelled-generation callbacks drop.
`state:started` means play requested, not acoustic delivery. Played events retain
source `frames` and add `rendered_frames`, `remaining_frames` (48000 Hz) and
`playback_active`, false after the last buffer. ACK `queued` also exposes
`rendered_frames`/`render_sample_rate:48000`.
Barge-in invalidates playback without stopping the call mic/changing its ring.

## Application identity

`application_sources {bundle_ids:["com.example.Editor"]}` is an explicit read
on a rearmed host. It does not launch apps, request TCC, activate windows or
authorize input. `bundle_ids` permits 1–8 distinct ASCII IDs, at most 256 bytes.
Reject over 512 processes before filtering, over 32 matches or over 196608 result
bytes. One physical identity/inventory job exists; privacy invalidates receipts
without releasing its slot before Security worker return. Results older than
15 seconds are rejected as stale.

See [inventory shape](fixtures/native/results/application_sources.json).
Canonical 36-character UUIDs are emitted lowercase; complete receipt equality
does not normalize. `application_instance_id` retains NSRunningApplication,
exact launch date, PID, bundle and path while matching.
`identity_token` identifies another temporary observation, not a new instance.
At most 256 receipts, valid strictly less than 15 seconds from read start.
Restart/stop/generation change removes them; rearm cannot restore them.

Results contain `native_instance_id`, `privacy_generation`, `clock_domain`,
`observed_monotonic_ns`, `completed_monotonic_ns`, `omitted_count:0` and
`applications`. Rows have `application_pid`, `application_name` (128 bytes), `bundle_id`,
binding/null and unavailable_reason/null. Unverified identity yields
`application_identity_unavailable` and null binding, never substituted name/PID.
Lists are not truncated to hide an exceeded bound.

Stable identity combines bundle ID, exact canonical POSIX path (4096 bytes) and
SHA-256 designated requirement. Public Code Signing APIs inspect running code
and validate the extracted requirement against that SecCode, not only a file.
`noNetworkAccess` validation proves neither notarization, reputation nor approved
publisher. Ad hoc signatures/requirement/path changes may need rule reapproval.
This evidence does not prove immutable process incarnation or eliminate changes
between observations.

`audio_start source:remote_app capture_scope:application` uses explicit app PID,
including the microphone-activity app for detected calls. No identity receipt
travels with startup; the old meeting rules requiring it are retired.
Capture is of the app, not a tab.

Daemon generation/clock differ from host privacy_generation/mach_absolute_time.
It binds replies to pending `command_id`, host and epoch, also bounds local read to
5 seconds, and never treats identity data as capture authorization.
A same-shaped JSON from a page/hub/user is not a host observation.

## Tests and limitations

`swift test --package-path native/macos` validates framing, limits, playback
generations and ring losses. Use a timeout. CI does not access personal screens/
microphones and is not native acceptance. `AudioDrainBarrierTests` covers stop
coalescing and ACK order after both queues. C `stop_barrier_test.c` pauses the
real producer mid-copy, checks boundary prefix, delayed pre-stop callbacks,
post-stop rejection, nonreopened cutoff and uncertainty counters.
macOS workflows run it with ASan/UBSan; it does not certify SCK/CoreAudio hardware.

VoiceProcessing is actually configured/read back at runtime; protocol tests do
not validate echo. Consent-based measurements must cover speakers/headphones,
barge-in, DAC tail, hotplug and setup duration. Synchronous AVAudioEngine control
latency must be measured; mode changes require inactive audio.
Wake recognition uses local macOS 26, not Porcupine. Its latency/CPU/permissions,
default microphone, per-app audio, TCC, shortcuts, Retina, sleep, echo, signed-bundle
reproducibility and audible cutoff need physical Mac acceptance.
