# Orbit Pruebas test bridge

Only `orbit-desktop/pruebas` compiles this listener and its DOM code. That
feature also enables `orbit-core/pruebas`, so the app uses the test identity,
data directory and Keychain service. `ORBIT_PRUEBAS=1 scripts/macos/bundle.sh`
selects both features. The normal build has neither the listener nor the DOM
script, and requires no extra Tauri command, permission or capability. The
gate inspects the normal executable for the bridge's protocol and socket
markers, and checks their presence in the test executable as a positive control.

The app owns `orbit-test-bridge.sock`, beside `orbit.sock` in its own data
folder, with mode 0600. The daemon's instance lock protects startup. An existing
live bridge or non-socket path is refused. Orderly exit removes the socket;
startup removes a stale owner-local socket after a crash. Stopping an old
instance twice cannot remove the next instance's socket.

## Requests and replies

One UTF-8 JSON object and newline per connection, one reply and newline. The
request and reply share `MAX_IPC_LINE_BYTES` with the daemon (1 MiB including
the newline). No credentials or conversation data are logged. Examples use the
English interface; names must match the language currently visible in the app.

```json
{"command":"press","window":"main","role":"button","name":"Record meeting"}
{"command":"type","window":"main","role":"textbox","name":"Message","text":"Hello"}
{"command":"read","window":"panel"}
{"command":"menus"}
{"command":"windows"}
```

Only `main` and `panel` are accepted. DOM roles use web names (`button`,
`textbox`, `heading`, `link`, `radio`, etc.). Names come from ARIA labels,
labelled-by references, HTML field labels or visible text. Hidden and inert
subtrees are excluded. A press/type requires exactly one visible role/name
match; refusal returns `cause: "match_count"`, the role, name and count.
Disabled elements and non-writable fields are refused. Typing replaces an
input/textarea's value with the native prototype setter, then sends bubbling
input/change events so controlled React fields see it.

A press calls the matching element's `.click()` in the web view. Its existing
React handler issues the application's ordinary command. It never invokes
that command directly. A successful bridge reply acknowledges DOM dispatch;
it does not prove the daemon completed the action. Read the resulting UI or
state separately. A lost receipt is never retried automatically.

Rust queues work on the main thread and evaluates the included DOM function
with `eval_with_callback`. Its returned object comes through that callback,
a oneshot, and the socket reply. Reads return `elements` with `role`, `name`
and `text`, plus `tree_markdown` in the harness parser's AX notation. This is
a DOM projection, not macOS accessibility acceptance. Menus return `tray`
(`title`, `rows`, row names and enabled flags) and `application` (nested rows).
Snapshots follow successful native writes, including the one-second tray
clock. Application responder items reflect their state when Rust built them,
not later automatic AppKit validation as focus changes. No system Apple menu,
Recent Items or other application's content is read.

Windows return each app window's `label`, `window_id` (AppKit `windowNumber`,
usable by `screencapture -l`), `visible` and `frame`. Frames use AppKit screen
points with a bottom-left origin, explicitly identified in `coordinates`.
Non-macOS adapters refuse window-number requests with their platform cause.

The view budget is two seconds, including waiting for the main thread. A view
that does not answer returns `view_timeout`, its `bound_ms`, command, window
and `outcome: "unknown"`. Evaluations queued past their deadline refuse before
clicking or typing. Read and write budgets are four seconds each. Four
connections/evaluations may be in flight; additional peers get `bridge_busy`
with the counts. Each window retains at most one pending evaluation until
its callback returns or is dropped, even after a socket timeout; another
evaluation of that window returns `view_busy` with its counts. Oversized requests/replies refuse with observed and maximum
bytes. The Mac-side client imposes a twelve-second overall deadline and the
same byte ceiling. These are engineering budgets, not Mac latency measurements.

## Harness scope and reviewer acceptance

All web-view presses, typing and reads use the bridge through `interface.py`.
Each reading is saved under the run's `trees/` folder with its original query
and byte count. The semantic tree retains headings, text leaves, editable
values and descriptions, list hierarchy and transcript order. Queries retain
matching rows and their ancestors; a size refusal never becomes an empty view.
The original twenty checks keep their evidence and meaning. Check 16 still
reads the last native menu state through `mac.py bridge-menus`.

A reading also returns an opaque `target` for each semantic element, alongside
its exact role and accessible name. The driver preserves existing explicit
first-match choices (for example, the sidebar before a duplicate breadcrumb),
then sends that chosen target with the full role/name guard. The view still
requires exactly one match. Targets use document nonces and weak node references,
so another node taking the same position cannot receive a stale press. Only
connected nodes are retained; reloads/relaunches invalidate old targets. A
request without a target still searches the whole view and refuses duplicates.

`mac.py bridge-call JSON` opens a fresh socket per request. After the Dock's
quit, `drafts_panel.py` waits for the relaunched app's bridge, refreshes the pid
and window number, and reads the recovered composer. The Mac-side draft step
still types, reads the value back from the named field and immediately sends
the Apple Event in a single remote call. It never starts/restarts CuaDriver.

Check 18 retains `pointer.swift` and the daemon's held/released facts. Its
panel readings now use the bridge. The Dock's quit, BlackHole, `say` and
`SwitchAudioSource` retain their original native paths. No new global shortcut
is pressed: the harness did not press those shortcuts before this migration.

Check 21 independently reads macOS accessibility through CuaDriver, with no
input tool granted in its manifest. The sidebar replaces «Grabar reunión» with
the recording card, so a single simultaneous snapshot cannot see both it and
«Terminar». The check takes exactly one AX sample before recording and one
during it, within one short CuaDriver session surrounding recording start.
Before each AX sample the bridge waits for the intended button to render;
there is no native AX polling. Both samples require the sidebar links «Hoy»,
«Calendario», «Reuniones», «Notas», «Asistente» and «Ajustes». The AX trees are
stripped of the system menu bar before being saved as `accessibility-*.txt`.
A `finally` block uses the strict stop that waits until no CuaDriver answers;
cleanup retries an interrupted stop only for the driver's recorded pid.

`dry_bridge.py` replays the app-view boundary from `screens/` and
`transitions.json`; the fake CuaDriver can read only the independent
`accessibility-*.md` fixtures and refuses every input tool. The good example
passes all 21 checks; `bad-21` removes an AX button name without changing DOM
behavior or any of the original checks. The original spoiled examples still
fail their own checks. Cross-language tests run the actual injected DOM code
and pass its trees to the existing list, transcript and recovered-draft checks.

Expected runtime is shorter because per-action CuaDriver subprocesses and AX
walks, the panel's WindowServer lookup, and the CuaDriver restart after quitting
are removed. One SSH round trip per bridge request and all audio, pointer-hold,
silence, hub and relaunch waits remain. The two new AX samples cost one scoped
CuaDriver session. No Mac timing improvement is claimed until the reviewer
measures complete runs under the same conditions.

Linux verifies the protocol, injected DOM, public driver and dry-run checks.
It cannot prove WebKit delivery, AppKit behavior or real AX visibility. On a
consented, idle Mac with the isolated Orbit Pruebas fixtures, the reviewer must:

1. Rebuild Orbit Pruebas with the current bridge and run the full Spanish
   harness. All 21 checks must pass, including 16, 17, 18 and the new 21.
2. Inspect saved bridge trees for the transcript's times/speakers/text, meeting
   titles and day groups, editable title values, muted spans and draft recovery.
   Confirm every web-view press/type is a DOM action. Repeated controls must
   target the intended sidebar/list/tab element, and stale targets must refuse.
3. Verify `accessibility-idle.txt` and `accessibility-recording.txt` contain
   the real macOS AX trees, the expected buttons and all sidebar names, without
   the system menu bar or Recent Items. Confirm CuaDriver has one short session,
   only two `get_window_state` calls and no click/type/key call. It must be gone
   before the script's first half and stay stopped through the Dock relaunch.
4. Confirm the panel is held by the real pointer and releases after it leaves;
   bridge reads must show both states. The Dock's quit must close the process;
   the next process must supply a fresh pid/window number, recover the exact
   typed draft and show «Recuperado…» without restarting CuaDriver.
5. Confirm audio input is restored and no CuaDriver remains after a failure or
   interruption, including one during check 21. Measure whole-run time against
   the previous run under comparable conditions; do not shorten audio or hold
   waits to claim a speedup. Keep only necessary synthetic evidence outside
   the public repository.
