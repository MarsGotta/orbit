# Meeting voice counts

The closing recording manifest already contains the linked calendar event's
`attendees` and `is_self`. EventKit's `is_current_user` becomes `is_self` in the
Mac's event context before upload. The hub reads that encrypted manifest, not
a live calendar snapshot. With at least one attendee, the remote pyannote call
uses `max_speakers=max(1, attendees excluding self)`. Without attendees it omits
the parameter. The microphone track is never clustered with the remote track.
An invitee is a ceiling assumption, not proof of attendance or identity.

No completed meeting is queued by an upgrade. The existing **Volver a
transcribir** action explicitly requests new voice separation. A language-only
change continues to reuse the existing voices. The durable `refresh_voices`
flag defaults to zero in older databases; a requested refresh survives worker
recovery. New text, voices and names replace the old view in one transaction
only after the pass succeeds.

Old words vote for the new voice interval that overlaps them most. When old
voices merge, a named voice takes precedence over an unnamed one; among named
voices the one contributing most words keeps its key and name. Equal counts
use the key as a deterministic tie. Other names remain encrypted aliases.
An old voice that splits lends its identity only to the part with most of its
words; fresh voices cannot accidentally inherit a reused numeric label. Turn
overrides and explicit user merges remain in force.

## Validation and remaining measurement

The gate uses fake pyannote dependencies and synthetic WAV fixtures only.
Boundary results are not an acoustic measurement:

| Synthetic boundary case | Before | After | What it establishes |
| --- | ---: | ---: | --- |
| Two labeled voices replaced by one | 2 | 1 | Named identity with two words wins over one word; alias survives |
| Calendar with owner and four invitees | — | — | `max_speakers=4` reaches the process and fake pipeline |
| Owner-only calendar invitation | — | — | `max_speakers=1` reaches the process and fake pipeline |
| No event or no invitees | — | — | No ceiling is passed |

The task's prohibition on model calls prevents running pyannote even on
synthetic audio. No acoustic threshold has been measured or guessed. Automatic
embedding merging for meetings without invitees is therefore **not implemented**;
the similar/distinct-voice acoustic test and before/after synthetic-audio table
remain pending. Before enabling it, measure community-1's own speaker
embeddings on known synthetic voices under varied text and audio conditions,
include held-out similar-but-distinct voices, and register the resulting
threshold and its measured purpose in `contracts/constants.json`. Embeddings
must stay inside the subprocess and must not be persisted or logged.

## Reviewer: count the 33-minute remote track

This is an explicit manual acceptance, never part of the gate. On the authorized
model host, use an existing private mono, signed 16-bit, 16 kHz **remote** WAV
fixture from the meeting. Do not include the owner's microphone. Set the
following variables privately; do not put paths or recordings in Git:

```sh
export ORBIT_DIARIZATION_PYTHON=/absolute/path/to/model-runtime/bin/python
export ORBIT_DIARIZATION_BUNDLE=/absolute/path/to/community-1
export ORBIT_DIARIZATION_GPU_LOCK=/absolute/path/to/shared-whisper.lock
export ORBIT_DIARIZATION_WAV=/absolute/path/to/private-remote-track.wav
export ORBIT_DIARIZATION_MAX_SPEAKERS=4
cd server
timeout 2400 .venv/bin/python tests/acceptance_diarization.py
```

The runner calls the same isolated separator as the final-pass worker and
prints only `{'voices': N}`. It does not modify a meeting or contact Whisper.
Run once without `ORBIT_DIARIZATION_MAX_SPEAKERS` for the baseline if needed.
To update the meeting itself after review, use **Volver a transcribir** in the
app with the upgraded hub and its recorded calendar invitees; upgrade the hub
first. This implementation needs no Mac wire-format or UI changes. Actual
voice quality, calendar capture and the reported eight-to-at-most-four result
still require that authorized real acceptance; Linux mocks cannot establish it.
