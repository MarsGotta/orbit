# Independent remote audio: real-test check 25

Check 25 records a separate, short meeting in the isolated Orbit Pruebas/hub
fixtures. It requires already installed **BlackHole 2ch** and **BlackHole 16ch**
input/output devices. It installs no driver. The early preflight fails check 25
with the missing device's name before any speech; the other checks can still run.

## Capture source and speech

Choose **Una app concreta / Application audio**, then **Orbit Pruebas Remote**.
The harness launches that small fixture app from
`~/Applications/Orbit Pruebas Remote.app`. `build-fixture-apps.sh` builds it with
every `build-test-app.sh` and signs it (`app.orbit.pruebas.remote`) with Orbit
Pruebas's stable identity; nothing is compiled or signed during a run. It has a
visible window and a stable process for the whole meeting. Only `orbit-core/pruebas` adds that identifier to the
source picker's existing inventory. The shipping build never offers it.

`crates/orbit-daemon/src/daemon/audio/meeting.rs` starts both `mic_local` and
`remote_app` for `MeetingAudio::Application`. The native helper's
`SystemAudioCapture.swift` uses a ScreenCaptureKit `SCContentFilter` including
only the selected application's PID, with `capturesAudio = true` and
`captureMicrophone = false`. Check 25 confirms that PID in the recording topic.
All-system capture would also hear the microphone fixture's output and would
not prove independent sides.

The microphone phrases use `say -a 'BlackHole 2ch'`, as before. The remote
phrases are synthesized **to files**, then played by the fixture app's AVPlayer
with the explicit `BlackHole16ch_UID` output. Running `say` as the remote app's
child would not establish which process ScreenCaptureKit attributes its speech
to. AVPlayer is owned directly by the selected app. Apple's
[audioOutputDeviceUniqueID documentation](https://developer.apple.com/documentation/avfoundation/avplayer/audiooutputdeviceuniqueid)
describes the explicit CoreAudio device selection; the
[BlackHole source](https://github.com/ExistentialAudio/BlackHole/blob/master/BlackHole/BlackHole.c)
defines the devices' UIDs and virtual transport.

| Phase | English pass | Spanish pass | Stored side |
|---|---|---|---|
| Microphone | Cedar shelves cost seven hundred | Los estantes de Cedro cuestan setecientos | `mic_local` only |
| Remote | Maple windows open on Thursday | Las ventanas de Arce abren el jueves | `remote_app` only |
| Muted microphone | Cedar umbrellas are purple | Los paraguas de Cedro son morados | Absent everywhere |
| Remote during mute | Maple gardens need watering | Los jardines de Arce necesitan riego | `remote_app` only |

The bridge presses «Silenciar» / Mute. Speech starts only after the daemon
confirms mute and the bridge reads its visible muted indicator. Unmute is also
confirmed. The harness finishes normally, waits for the note and reads this
fixture's stored transcript directly from the isolated hub. Phrases must occur
on their proper side and be absent from the opposite side. Even partial muted
microphone content words fail. Matching ignores case, punctuation and accents;
the number seven hundred/setecientos may also be written as 700. It does not
relax missing phrases.

The hub readback validates each chunk's checksum, WAV format, decoded frame
count, sequence and manifest totals. Both tracks must contain nonzero PCM and
have distinct hashes of their decoded PCM. No evidence is played on the Mac.

## Silence and restoration

`remote-route.swift` inspects CoreAudio before synthesis/launch and before every
spoken phase. Both exact device names must resolve to the expected distinct
UIDs, virtual transport, live devices, and input and output streams. Hardware,
renamed substitutes and aggregate/multi-output sinks fail. Any aggregate whose
active subdevices contain a fixture sink also fails, even if not selected.
Any other process reading a fixture device fails conservatively: a separate
monitor could forward it to hardware. Only Orbit Pruebas and its helper,
identified by exact executable paths, may read them; a reader of another device
(Siri on the built-in microphone) cannot hear them. An unreadable
CoreAudio property fails closed; an object that went away since it was listed
(a `say` that ended) is skipped. No device or unrelated process inventory is
saved. Close monitoring/routing software before running, and keep the Mac idle
with Bluetooth audio disabled as required by CONTRIBUTING.md.

The player rechecks the route and its explicit output UID every 100 ms and
stops on a failure. Playback has a finite receipt budget; an abandoned player
exits after two idle minutes. These are provisional harness budgets, not measured
Mac latency claims. External route changes during a phrase invalidate the trial.

Check 25 changes no system output, alert output, device volume, mute or sample
rate settings. The existing harness selects BlackHole 2ch as the default input
and durably records/restores the owner's previous input, including on failure.
Check 25 verifies that input before speaking. Its `finally` finishes an unfinished
fixture recording and stops the player, without replaying a finish whose receipt
was lost. Whole-run cleanup also stops it by its exact executable path, waits
for exit, and restores the input. A failed stop fails cleanup; a stale PID file
never authorizes signalling another process. A failed run retains only its
invented fixture evidence, following the harness's existing policy.

## Linux evidence and Mac review

`dry-run-examples/good/remote-audio.json` supplies the successful stored sides;
`bad-25/remote-audio.json` leaks the muted microphone phrase into the remote
transcript. Both Spanish and English replays run the same orchestrator and check.
The spoiled example fails only check 25; checks 1–24 retain their results.
Routing, missing-device, cleanup and attribution mutations use synthetic data
and never call host audio commands. Linux success proves harness logic, not
CoreAudio routing or ScreenCaptureKit delivery.

On a consented, idle Mac with isolated fixtures, the reviewer must:

1. Build Orbit Pruebas with `pruebas` and run the Spanish and English harness.
   Confirm the fixture app compiles/opens, is listed in the application source
   picker, and its PID matches the selected recording. Screen Recording remains
   the existing macOS permission; no new permission/activation switch is added.
2. Inspect `report.json`'s `remote_audio`: safe route metadata, distinct nonempty
   track hashes/frame counts, stored phrases and sides, mute/unmute confirmations,
   and `restored: true`. Inspect `trees/remote-muted.txt` for the muted indicator.
   Verify actual silence at speakers/headphones and that no monitoring app is open.
3. Exercise a missing BlackHole 16ch fixture listing and a failure during speech
   without uninstalling anything. Confirm no speech starts with a failed
   preflight, the player exits, and the original default input and unchanged
   output settings are preserved. Check both ordinary failure and interruption.
4. Confirm actual remote audio is captured while AVPlayer targets the virtual
   device. This native behavior cannot be inferred from a Linux replay. Record
   timings and any capture/speech failures; do not count skipped as passed.

Expected added cost: roughly **20–40 seconds plus hub transcription/note closure**,
with about 10–15 seconds of unique speech. These are estimates, not Mac
measurements. No Mac, SSH, production hub or Hermes trial was run for this change.
