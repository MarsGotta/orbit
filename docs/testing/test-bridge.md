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

This step changes check 16 only: during the first recording, the harness calls
`mac.py bridge-menus`. It requires exactly one enabled «Silenciar» row, one
enabled «Terminar sin guardar…» row, and a recording time in the tray title.
`dry-run-examples/good/bridge.json` passes; `bad-16/bridge.json` disables the
mute row and fails only check 16. All other presses, reads, pointer work,
shortcuts, relaunches and CuaDriver lifecycle remain on their existing paths.
The English harness migration is a later step.

Linux verifies the protocol with a fake desktop boundary, the injected DOM
against controlled React fixtures, build exclusion, and all dry-run checks.
It cannot prove real WebKit event delivery, AppKit menus or window numbers.
On a consented, idle Mac with the isolated Orbit Pruebas fixtures, the reviewer
must check:

1. Build the normal app and the test app. Only the test app creates the bridge
   socket; its mode is 0600, under the test data directory. Quit and reopen it:
   the old socket goes and the new process answers without a bridge restart.
2. Select Spanish and run the full real harness. Check 16 must pass, not skip;
   inspect the actual status-item menu for enabled «Silenciar» and
   «Terminar sin guardar…», and verify its time matches the recording and ticks.
   Compare `bridge-menus` with the displayed tray and application menus.
3. Through the socket, press one harmless named DOM button in the main window
   and one in the visible panel. Verify their actual UI transitions. Type into
   a named controlled field, then navigate away/back to verify React retained
   the value. A missing/duplicate name must refuse with 0/the duplicate count.
   Use synthetic content and read-only or reversible controls, not live accounts.
4. Compare `read` with the visible controls, their labels and text. Confirm
   hidden controls do not match. Compare `windows` to both app windows, then
   use each returned number with `screencapture -l` to identify the right window.
   Keep only necessary synthetic evidence outside the public repository.
5. Confirm an unresponsive/reloading view refuses within two seconds and a
   delayed action does not run after expiry. A timed-out action remains an
   uncertain outcome: inspect before taking any next action.
