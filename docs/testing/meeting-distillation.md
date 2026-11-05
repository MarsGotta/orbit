# Meeting distillation review acceptance

The hub owns proposals and all vault writes. The app sends typed device calls
through the vault bridge; provider credentials and vault contents are never
stored by the presentation layer. Upgrade the hub before installing this app.

## Screen states and hub facts

| Design state | Hub proposal and app projection |
| --- | --- |
| 53, decision required | `status: proposed`, at least one `conflicts[].choice: null`. The first contradiction opens. Apply names the missing decision and stays disabled, including contradictions on excluded notes. |
| 53a, selected change | The same proposal, selecting a `changes[].id`. `properties[].before/after` and section-scoped `lines[].before/after` render the diff. `quote_ids` resolve to exact transcript text, `at_ms` and speaker. |
| 53b, editing | A local draft of the proposed property and line slots. Done sends `PATCH` with the current `revision`; rejected edits remain visible. No vault write occurs. |
| 53c, decided | `choice: incoming`, `current` or `both`, persisted by `PATCH`. Apply becomes available only after every contradiction has a choice. |
| 53d, excluded change | `changes[].include: false`, persisted by `PATCH`. Counts sum property and section patches on included notes. The hub removes links to excluded new notes. |
| 53e, applying | An outstanding explicit apply request, or hub `status: applying`. All review controls are locked. A lost receipt causes a GET, never an automatic write retry. |
| 53f, applied | Hub `status: applied`. A status notice reports confirmed counts, offers Undo and links to the first included note and `log.md`. The pending entry disappears; the catalogue and source count refresh. |

`queued` and `preparing` show preparation progress without Apply. `failed` shows
its cause and numeric facts with an explicit retry. `stale` names changed notes
and offers a fresh preparation. `recovery_required` offers journal-aware Undo.
`discarded` states that neither notes nor a source were written. `undone` keeps
the source and explains each skipped note as `content_changed` or
`outcome_unknown`. Preparing again navigates to the hub's new proposal id.

Pending proposal lists retain one bounded transport page; the column provides
previous/more controls. The meeting pill reads that meeting's newest proposal:
amber for `proposed`, green for `applied`. The latter opens the ledger. Other
states retain their own names, including unconfirmed writes.

## Attribution

`SCHEMA.md` may specify `owner`; reviewed writes use `human:<owner>` exactly.
Without it, the hub retains the reviewing device owner-name/id fallback.
The app uses the schema owner, or the normalized current profile name, to show
“Reviewed by you”. Hub-generated authors use the assistant's display name and
local date/time. Sources use titles; file paths remain link targets. The raw
Markdown view continues to expose the actual document when explicitly opened.
Structured ledger rows use catalogue labels; handwritten ledger prose remains
unchanged.

The native observer reads the newest proposal page once at startup, when the
existing background Notes frame announces a newly ready meeting note, and after
either explicit preparation command settles. Only a hub page containing a
`queued` or `preparing` proposal keeps a ten-second timer. Between reasons it
waits without a timer or hub requests. A transport failure preserves an already
observed preparation; a refusal or privacy stop clears that active watch.
The observer works with the window closed, does not replay already-ready
proposals at startup, and retains only ids/statuses and the ready-note sequence.
It uses the existing notification delivery adapter. The notice contains no
meeting or note text; the system notification permission is the only decision.

## Reviewer checks on a Mac

Use Orbit Pruebas and an isolated invented hub/vault. These checks have not been
performed by Linux tests:

1. Compare 53 and 53a–53f in light/dark, Spanish/English, at the normal window
   size and a narrow window. Check scrolling, long titles, focus, checkboxes,
   radio arrow keys, diff signs and VoiceOver announcements.
2. From Summary, Transcript and Private notes, follow the amber pill. Exercise
   each contradiction choice, edit a property and a text section, and exclude
   a new person. Confirm the written notes, source transcript/citations,
   `index.md`, `log.md`, `generated` and owner verification in the isolated vault.
3. Change a note externally before Apply: no note should be written, and a fresh
   preparation must use a new proposal. Change another note after Apply: Undo
   must skip that note, explain its cause and retain the immutable source.
4. Verify the green pill opens a readable ledger; source titles open the named
   note or meeting. Verify counts and the pending list refresh after apply/undo.
5. Complete a proposal with the window hidden; confirm one localized native
   notice, no repeated notice on polling, no startup replay and no content in
   Notification Center. Exercise granted and denied system notification states.
   Check privacy stop prevents the observer's hub requests. Verify one startup
   read and no further reads while idle, including without a vault/model. A new
   ready note and each preparation command must restart the watch; queued work
   keeps it active until preparation finishes. Save counts, not response content.
6. In Notes settings, follow the model link: AI settings must focus the vault
   function. Confirm no deferred automatic policy, private-note switch, query
   or vault-review action is offered by this step.

No real model, vault, Hermes service, SSH connection or Mac operation is needed
for the Linux fixtures and showcase captures.

## Real-test check 26

The real test distills its first meeting (`scripts/e2e-mac/vault_distillation.py`).
The test hub needs no new setting: its `vault` function uses the operator's
processing model (`server/orbit_hub/ai/defaults.py`), the test model. The
run writes an invented vault to `vault/` in its own directory on the test
server: a `SCHEMA.md` with `owner: maple` and the four roles, a person (Laura
Cedar, Clara in the English pass, with that first name as alias), a project
"Reading room works" whose `start` is 2026-11-17, `index.md` and `log.md`.
The meeting script already names Laura/Clara and starts the reading room's
works on 3 November, so no line was added to it and no other check's script
changed.

Once the first meeting's note is ready, and after every other check of that
meeting has its evidence, the check:

1. connects the folder as a server-folder vault with the app's own call
   (`orbit_vault` «configure», the body Settings › Notes sends), through
   Orbit Pruebas's daemon;
2. waits for the meeting's proposal to settle as `proposed`, reading the hub's
   list through the daemon every two seconds. The bound is what the hub takes
   to give up: the notes worker's 5 s idle tick, the model's 300 s and the
   connector's 120 s background budget, read from the hub's constants. A
   `failed` preparation ends it earlier, with its cause;
3. opens the meeting from «Reuniones» and screen 53 from its amber pill,
   through the test bridge, and decides every contradiction: a property takes
   the meeting's value («Usar …»), a text keeps both «in discussion»; after
   each choice it waits for the column to show it, because the review locks
   its controls while it saves;
4. presses «Aplicar N cambios», waits for `applied` (bound: the connector's
   120 s budget) and reads the vault from disk;
5. presses «Deshacer», waits for `undone` and reads the vault again;
6. disconnects the vault, also after a failure, so no later meeting of the
   run is distilled.

It passes when the proposal belongs to the first meeting and has a
contradiction on a property; the applied proposal carries the decisions; each
included note was written (created or updated as proposed) with
`generated.by` of the hub and `verified: human:maple`; one note is the
meeting's, with its `orbit://meeting/<id>` source; the source exists, its
`sha256` is its body's and the proposal's, and it is verified too; `index.md`
lists every written note and the source; `log.md` has exactly one new line
per written file (notes, source, `index.md`, `log.md`); each property decided
for the meeting has its value and each text kept «in discussion» has its
marker; and after undo nothing was skipped, every note is byte for byte as
before, created notes are gone and the source remains. It never checks what
the summary, people or tasks say: that depends on the model.

The dry run replays the proposals and the three vault states from
`dry-run-examples/good/distillation.json`. That example was produced once by
the real hub's preparation, apply and undo, with a recorded model answer, on
the vault the check writes. `bad-26` leaves the project with the meeting's
date after undo; in Spanish and English it fails check 26 only. The check is
not critical: a failure never stops the run.

On the Mac, the reviewer must:

1. Run the Spanish and English passes. Check 26 must pass, and checks 1–25
   keep their results.
2. In `report.json`, read the `distillation` and `distillation-write` waits
   (seconds against their bounds) and `distillation`: the decisions, the
   statuses and the three file sets. Open `vault/` in the run's directory:
   the meeting's note is gone, the project and person are as written, and the
   source and the undo lines in `log.md` remain.
3. Look at `trees/*distill-review*`, `*distill-decided*`, `*distill-applied*`
   and `*distill-undone*`: the pill opened screen 53, each choice was shown in
   the column, Apply named the count and «Deshacer» was pressed once.
4. If the test model proposes no contradiction on the project's `start`, the
   check fails saying so. Record it with the model's name: it is the model's
   behaviour, not the harness's.
