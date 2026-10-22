# macOS helper

`OrbitNative` is Orbit's native helper: a windowless AppKit application that
owns permissions, capture, playback and global shortcuts. In production, the
Rust daemon launches it as a managed child (`OrbitNative --stdio --managed`)
and communicates using the [native protocol](../../contracts/native-v1.md).
When the daemon closes the pipe, whether through orderly shutdown or a crash,
the helper stops capture without changing the privacy latch and exits, releasing
the shortcuts. The helper no longer launches the daemon: it refuses to start
without arguments.

```sh
swift build --package-path native/macos
swift test --package-path native/macos
bash scripts/macos/bundle.sh
```

Requires macOS 15 and Xcode 16 or later. Protocol and ring tests do not record
audio or request permissions. `--stdio` mode without `--managed` is retired;
use the isolated fixture described below to test the protocol.

The explicit smoke test of the complete helper uses its own private temporary
directory, starts with privacy stopped and keeps stdin open until normal exit:

```sh
swift build --package-path native/macos --configuration release
orbit_native_bin_dir="$(swift build --package-path native/macos --configuration release --show-bin-path)"
python3 -B scripts/macos/smoke-native.py --binary "$orbit_native_bin_dir/OrbitNative"
```

Run it only against a trusted build of your own that includes
`--test-state-directory`; it does not execute or install downloaded ZIPs. It omits
the menu, shortcuts and session observers, and only accepts hello/heartbeat/status/
stop/shutdown. It does not change HOME, request permissions, open Keychain or
create an audio engine while idle. `status` permission queries are read-only
preflight checks. This is evidence of the protocol and shutdown without producers,
not of audio, TCC or installation. Verify those separately with consented Mac fixtures.

`WindowSelectionTests` validate ephemeral inventories, exact single-use tokens,
privacy, time and the slot that remains occupied after a timeout until capture
finishes. Run them separately with
`swift test --package-path native/macos --filter WindowSelectionTests`.
They do not query real windows or test TCC/focus. Capture requires selecting an
entry with an observed owner and instance; a window ID alone is insufficient.
There is no desktop fallback or fallback between source types. Identity checked
before/after does not establish an immutable WindowServer generation.
Metadata expires after 30 seconds and a new listing retires the previous one;
closing a window or changing its descriptor may require a new selection.

`selected_text` consumes the same single-use binding as `screenshot`. It requires
Accessibility to have been granted: listing sources or reading selection does not
request TCC. The AX worker is serial and performs no input, focus or clipboard
changes. It requires a unique match of owner, full title and public CG/AX bounds,
a TextField/TextArea with an explicit non-secure subrole, and a contiguous selection
stable across two reads. Missing classification or any ambiguity is rejected.
`selected_text_available` being true indicates permission/backend availability,
not a readable selection in that window or the absence of secrets
(`sensitivity:not_certified`).

Limits: 5 s including checks, 32 AX and PID windows (including hidden ones),
a CG query of up to 200 rows and SCK bounded before traversal,
256 AX access/acquisition calls with a 0.1 s timeout per object,
4096 UTF-16 units and 16384 UTF-8 bytes. NUL and malformed Unicode are rejected;
other control characters are retained as data, without truncation or normalization.
Stop/timeout revoke delivery and subsequent reads, but do not release the shared
slot until the framework returns. Internal CFString/array allocations Apple may
perform before returning them are not controlled.

`swift test --package-path native/macos --filter SelectedTextTests` uses pure
fixtures for ambiguity, prior classification, focus/range/text changes, Unicode
and budgets; it does not query apps. The two observations are not an atomic
snapshot, an immutable WindowServer identity or real Mac permission/focus acceptance.

From the repository root, the C regressions can also run without audio hardware
(Linux or macOS):

```sh
orbit_test_dir="$(mktemp -d)"
cc -std=c11 -Wall -Wextra -Werror -fsanitize=address,undefined -pthread \
  -I native/macos/Sources/OrbitAudioRing/include \
  native/macos/Sources/OrbitAudioRing/OrbitAudioRing.c \
  native/macos/Tests/ring_test.c -o "$orbit_test_dir/ring-test"
"$orbit_test_dir/ring-test"
cc -std=c11 -Wall -Wextra -Werror -fsanitize=address,undefined -pthread \
  -I native/macos/Sources/OrbitAudioRing/include \
  native/macos/Tests/stop_barrier_test.c -o "$orbit_test_dir/stop-barrier-test"
"$orbit_test_dir/stop-barrier-test"
```

The second test stops the real producer midway through a copy, delivers earlier
callbacks after the cutoff and checks time-based trimming. It does not record
or claim to have tested ScreenCaptureKit: that SDK delivery order requires a Mac.

Orbit is installed with `install.sh`, as described in [INSTALL.md](../../docs/INSTALL.md).
The bundle's main executable is the app (`Contents/MacOS/Orbit`): it hosts the
daemon on a thread, and the daemon launches this helper. The app manages login
startup through Settings; the helper no longer has its own login item.

Grant microphone and Screen Recording permissions only when enabling their features.
The process does not request Full Disk Access or install a virtual audio device.
Accessibility is requested separately and does not grant permission to other drivers.

The helper shows no menu of its own; the icon belongs to Orbit.app.
Control+Option+Escape stops capture/playback without depending on the daemon.
The stop persists at `~/Library/Application Support/Orbit/native-privacy-stop`.
Only local rearm from the app through the daemon's private pipe clears it;
remote messages cannot do so. EOF, an expired heartbeat, sleep or a device change
stop capture and leave the latch unchanged.
The immediate stop signal does not confirm draining: the ACK waits for producers,
callbacks and pending PCM. Delayed SCK callbacks retain their samples from before
the cutoff using the stream clock; later audio is not admitted.
If the boundary cannot be established, loss or uncertainty is reported and rearm
is blocked, without claiming a complete recording.
An app call requests an explicit session using Apple's VoiceProcessing backend.
Only valid readback of input/output, bypass and the shared TTS reference allows
VAD segmentation in the daemon. The microphone continues delivering silence and
voice while the response is processed/played. Control+Option+Space retains PTT.
An active raw meeting prevents mode changes; sources are not mixed and a live
meeting is not reconfigured.

Local development signing is not notarization. For Developer ID distribution,
provide `ORBIT_SIGN_IDENTITY` to the script and notarize the artifact with the
owner's credentials. Gatekeeper is not disabled and quarantine is not removed.

The manual **macOS development artifacts** workflow builds Apple Silicon (`arm64`)
and Intel (`x86_64`) separately, tests Rust/Swift and the concurrent buffer, and
uploads temporary Actions artifacts. It does not publish a release. Each artifact
includes a ZIP, a manifest with commit/architecture and a checksum verifiable with:

```sh
shasum -a 256 -c Orbit-0.1.0-macos-arm64.zip.sha256
```

For Intel, use the `x86_64` name. The development package uses ad hoc signing and
declares `notarized=false`; its CI tests do not establish permissions or hardware
behavior on the user's Mac. The exact artifact name includes the tested commit.

Current limitations: default microphone; system audio per app, not per tab;
VoiceProcessing available only after real readback, without acoustically validated
AEC; acoustic tests, setup latency and TCC acceptance remain pending on Mac.
Compiling the code does not establish that these capabilities work.

`AssistantFilesTests` cover the assistant's folders on a temporary folder with
invented names: resolution of links and `..` to canonical paths, the refusal to
read a path that is no longer canonical, listings and searches that never follow
a link, text pages cut at characters, PDF text through PDFKit, an image fitted
into a PNG capture, and bookmarks of a deleted, replaced or moved folder. Run
them with `swift test --package-path native/macos --filter AssistantFilesTests`.
They open no panel and touch no TCC-protected folder.
