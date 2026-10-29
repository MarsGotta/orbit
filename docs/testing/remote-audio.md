# Independent remote audio acceptance prerequisite

The current real harness supplies only **BlackHole 2ch**, selected as the
microphone and as `say`'s output. Selecting system audio on that same signal
would record duplicate speech, not a distinct remote participant. This work
installs no device or driver and does not claim remote-track acceptance.

A safe native trial needs two independently addressable virtual sinks (for
example an already installed second BlackHole device) or an existing app-specific
virtual route that isolates the remote source from the microphone. Both must
have no hardware output or monitoring branch. One source feeds the default
microphone; a different source app feeds only the remote virtual sink and is
selected by the existing application-audio selector. The source must remain
capturable by ScreenCaptureKit when its output is virtual. A multi-output device
including speakers/headphones is unsuitable. If a second virtual device/driver
would need installation, stop and ask the owner to arrange it separately.

Once that prerequisite is available on a consented, idle Mac, use the isolated
Orbit Pruebas/hub/profile and a short disposable meeting:

1. Route microphone speech **Cedar shelves cost seven hundred** to the microphone
   sink and remote speech **Maple windows open on Thursday** to the remote sink.
   Require different, nonempty stored `mic_local` and `remote_app` tracks and
   both phrases in the hub's stored transcript, attributed to the proper side.
2. Mute the microphone through the bridge. Say **Cedar umbrellas are purple**
   on the microphone sink and **Maple gardens need watering** on the remote
   sink. Unmute, finish normally and read back: the microphone-only muted
   phrase is absent; both remote phrases remain. Check the UI's muted span.
3. Inspect capture metadata, checksums, decoded audio and transcript from only
   this fixture. Never play the evidence on the owner's Mac. Restore routing
   even after a failure and save no unrelated device/app information.

Without that hardware configuration, Linux can verify the existing synthetic
mic/remote frame policies, mute isolation, upload contracts and stored hub turns
through the full gate. Check 13 verifies microphone muting in the current real
single-source run. Neither establishes native ScreenCaptureKit remote delivery.
The report therefore retains this explicit uncovered item; no duplicated track
or fabricated native pass substitutes for it.

Expected native trial cost after setup: 30–45 seconds of unique speech plus
transcription/closure time; not measured here. No calendar or remote audio trial
was run as part of the Linux changes.
