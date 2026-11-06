# Reviewed meeting distillation

The hub prepares proposals from the public transcript once a meeting note is
ready. It does **not** read private notes, the note's generated sections, or its
edits: those may contain private material. A ready note is the trigger, not a
source of text. The vault AI function selects the model independently from
meeting notes. An explicitly configured model overrides the operator's processing
model; the operator model is the default shown by AI settings. Without a vault
or a usable model no proposal is created. A legacy `maintenance_model` string
alone does not configure an endpoint or credential.

The notes worker scans durable ready notes after its queued note jobs. One
proposal per meeting/connection suppresses automatic repeats, including failures;
retry is explicit. Preparing on demand also works for older ready meetings.
Preparation is serialized, uses English instructions and asks for existing-note
text in that note's language and new notes in the meeting language. The request
contains SCHEMA.md, index.md, log.md, bounded vault notes and public transcript
turns. Instructions inside that material are untrusted data. No Hermes job or
real model is required by the tests. Preparation completion signals the meeting's
owning device through the existing recording-change notification.

## Device HTTP API

Every route requires the existing paired-device Bearer token and allowed hub
transport. Meetings belong to the personal hub; another paired Mac may review
or undo. Assistant vault routes and MCP remain read-only. JSON responses use
`Cache-Control: no-store`. Errors use the existing `{code, message, refusal, op}`
envelope with measured numbers; connector messages and provider content are
never copied into errors. Read the proposal after a write failure or conflict.

| Method and path | Request | Successful answer |
| --- | --- | --- |
| `POST /v1/recordings/{recording_id}/distillations` | No body | A durable queued `Distillation`; poll its GET route while the worker runs the model. Without vault/model: `{prepared:false, reason:"vault_or_model_not_configured"}` |
| `GET /v1/vault/distillations` | Optional `meeting_id`, `page=1`, `page_size=50` query parameters | `{entries: Distillation[], next_page: number \| null}`; at most 100 entries per page, within the 10,000-entry pagination budget |
| `GET /v1/vault/distillations/{id}` | No body | One `Distillation` |
| `PATCH /v1/vault/distillations/{id}` | `Review` below | Revised `Distillation`; stale revision is HTTP 409 |
| `POST /v1/vault/distillations/{id}/apply` | No body | Applied `Distillation`; undecided/stale/invalid state is HTTP 409 |
| `POST /v1/vault/distillations/{id}/discard` | No body | Discarded `Distillation`; **no vault write**, including no source |
| `POST /v1/vault/distillations/{id}/undo` | No body | Undone `Distillation`, with `skipped` paths and causes; also reconciles an interrupted/uncertain apply |
| `POST /v1/vault/distillations/{id}/prepare` | No body | A new queued `Distillation` with a new id; the worker reads fresh before hashes, or the disabled answer above |

`Distillation` fields are `id`, `source_kind:"meeting"`, `meeting_id`, `title`,
`created_at` (UTC ISO 8601), `by` (`orbit-hub/<installed-version>`), `revision`,
`status`, `changes`, `conflicts`, `quotes`, `people`, `new_source`, `error`,
`changed_paths`, `skipped`, `dropped`, and `can_prepare_again`. `summary` is
the source summary. A failed preparation may lack title/summary/source/author; it always
has its cause in `error.code`, plus provider/connector numeric details where
available. Statuses are `queued`, `preparing`, `proposed`, `failed`, `stale`, `applying`,
`applied`, `discarded`, `undoing`, `undone`, and `recovery_required`.

A note change has `id`, `path` (vault relative), `kind:create|update`, `title`,
`include`, `base_hash` (SHA-256 or null for creation), `properties`, `lines`, and
`warnings`. Each property has `key`, `before`, `after`, `quote_ids`. Each text
patch has `section`, `before` (empty to append), `after`, `quote_ids`. These are
section-scoped before/after fragments for the app's removed/added line display.
A quote is `{id, at_ms, speaker, text}` and must match a transcript turn's time,
speaker and exact text fragment. `people` maps named speakers to the unique
person note matched by title or aliases, or null when no match exists.

A contradiction has `id`, `path`, `property` (or section label), `current`,
`incoming`, `quote_ids`, `choice:null|incoming|current|both`. Every replacement
of an existing property or prose fragment conservatively becomes a contradiction;
the hub does not decide which old prose was written by a person. `both` retains
the old scalar and writes both alternatives under `## In discussion`, or keeps
both prose fragments with an `in discussion` marker. Every contradiction must
be decided, including those on excluded notes.

A `Review` example:

```json
{
  "revision": 0,
  "decisions": {"n1:property:0": "incoming", "n1:lines:0": "both"},
  "changes": {
    "n2": {"include": false},
    "n1": {"properties": {"kickoff": "2026-11-03"}, "lines": {"0": "The plan is ready."}}
  }
}
```

Change keys are note-change ids; line keys are zero-based decimal patch indices.
Only existing proposed property/line slots may be edited. Edits are revalidated
before saving and again before applying. Unchecking a new note converts links
to it in included new text to their plain labels, so excluding an optional
person does not create broken links. Existing notes remain valid link targets
when their proposed updates are excluded. Long fiches have a 400-word warning.

`dropped` lists the notes the model proposed that did not validate, as
`{path, code}` with the validation code; they are not in `changes` and are
never written. `new_source` is `{path, summary, sha256}`. `changed_paths`
names stale input notes; the proposal becomes `stale` and applying writes nothing.
`skipped` entries are `{path, code:"content_changed"|"outcome_unknown"}`.
Repeat apply/discard/undo after success is idempotent. Re-preparing retains the
older proposal for inspection. Deleting/discarding a meeting removes its stored
proposals and refuses in-flight preparation persistence after deletion.

## Validation and effects

The recorded model returns a strict JSON object: summary, exact transcript
quotes, and bounded property/text patches. No missing fields, arbitrary source
writes, managed provenance edits, unknown schema types, role-folder mismatches,
duplicate people, overlapping replacements, invalid dates, broken new links or
new wikilinks are written.

The answer's shape and its quotes are checked whole: a malformed answer or an
invented quote fails the proposal. The notes are then checked one by one
(`validation.build`). Before a text patch is checked, two slips of small models
are repaired without inventing anything. A wikilink `[[Note]]` or
`[[Note|label]]` that the vault's own resolver maps to one note of the vault,
or to a note the same answer creates, becomes `[label](/vault/path.md)`, as
Orbit writes links; an unresolved or ambiguous one becomes its plain words.
A Tasks date (`📅`, `⏳`, `🛫`, `✅`) whose value is not an ISO date is removed,
from its marker to the next Tasks marker or the end of the line, and the task
keeps its text. A note path whose file name has a character the vault refuses
(`:`, `\` or a control character, as in a title with its time) gets `-` for
each, unless that would name an existing note; the answer's links to the
written path follow it. The folder is left as written, so absolute, hidden and
`..` paths are still refused. Wikilinks and dates in a reviewer's own edit are still refused,
not repaired. A note that still fails any check is left out of the proposal,
with its contradictions, and named in `dropped` with its code. Links in the
remaining notes to a new note that was left out become plain words, as when a
reviewer excludes one; a note whose new link names no note at all is left out
too. The meeting note anchors the proposal: without exactly one valid meeting
note the proposal fails, with the first dropped note's cause when there is
one.

Existing YAML must parse safely without duplicate keys, tags, anchors or aliases. Invalid SCHEMA.md fails preparation instead of
writing against fallback roles. OKF 0.2 provenance and lifecycle are checked;
project fields use the schema's aliases. Arbitrary schema prose informs the
model; it is not an executable validation language. Language and factual
adequacy still require the human review: structural validation is not a proof
that the model understood a sentence.

All input notes' content hashes and remote versions are checked before the first
write, including excluded notes and read-only context. Unchanged remote context can be
validated by its fresh catalogue version rather than downloading it again; every
touched file receives a direct content check. Missing planned paths
must still be absent. Apply writes included notes, an immutable source with the
full public transcript and summary plus SHA-256 of its UTF-8 body, root index,
and newest-first log. A changed transcript or summary gets a new content-addressed
source path; an identical existing source is retained byte for byte. The meeting fiche also links the original `orbit://meeting/{recording_id}` in
`sources`. New text has source citations and Markdown footnote definitions; namespace-prefixed citation
ids avoid collisions with existing notes' source ids. Generated and verification
events preserve prior records. The source uses the schema's source role/folder;
meeting patches use its meeting role/folder and the model sees existing naming
patterns. Log actions are `create`, `update`, and `delete` on undo, with an
English `Undo meeting distillation` detail. Logs retain older free prose.

The optional `owner` id in `SCHEMA.md` takes precedence for `verified.by`,
using `human:<owner>` exactly. It is validated alongside `types` and `fields`;
invalid owner metadata prevents preparation. Without `owner`, use the reviewing
Mac's encrypted device profile `owner_name`. Normalize it using the vault filename normalization
(lowercase, transliterated, hyphen separated) for `human:<owner>`. Thus the
invented owner “Owen Reed” becomes `human:owen-reed`, without a hard-coded
identity. If the reviewing
Mac has no usable name, use `human:<authenticated-device-id>` rather than
inventing a person. Apply reads the reviewer's current profile, not the model's
claims or the recorder's local microphone label. `generated.by` comes from
installed hub metadata, and timestamps have explicit UTC offsets.

Encrypted `vault_distillations` rows keep originals, hashes, remote versions,
selected connector identity, and the effect journal. They use the existing
hub storage key and capacity checks. A changed connector cannot receive an old
proposal's writes. Sources are kept on successful undo. Other notes restore
exact previous bytes (created notes are removed), unless current hashes or sync
versions differ; those are skipped with their causes. The index removes reverted
links while retaining the source and skipped notes; a later index edit is skipped.
The log is an append ledger and receives undo entries while preserving later prose.
Interrupted preparations become failed with `preparation_interrupted` at restart;
interrupted applies become `recovery_required`. Undo reconciles that journal by
reading current content, never blindly replaying an uncertain write.

## Connector guarantees and limitation

Folder writes pin directories with no-follow descriptors, write and fsync a
private temporary file in the destination directory, recheck the old content,
then atomically rename and fsync the directory. Unrelated frontmatter fields,
comments, line endings and body fragments remain byte for byte. A group of files
is preflighted and journaled, with compensating rollback on a failure. Filesystem
rename is atomic **per file**, not a transaction visible across multiple files;
external editors do not participate in the hub's process lock.

Fast Note Sync uses its real `GET/POST/DELETE /api/note` API with the `token`
header. A read must provide `version` and opaque `contentHash` before writing.
The hub rechecks both content and version, sends the read hash as `baseHash`
and `createOnly` for new paths, then verifies content and the advanced version.
The service's own history preserves prior versions. Transport failures record
uncertainty; the hub reads back and compensates only notes still matching its
planned content. No original update is retried merely because its receipt was
lost. Unresolved compensation remains `recovery_required` and names paths.

**Upstream constraint:** the current REST `CreateOrUpdate` handler does not
enforce `baseHash`, and its request DTO has no expected-version condition. The
service has no REST transaction across notes. Consequently the requested strict
remote transaction cannot be guaranteed against concurrent editors or partial
network failure. Tests deliberately implement this actual behavior, not an
invented batch/CAS API. Strict remote all-or-nothing requires upstream support;
read preflight plus compensation is the implemented guarantee. Upstream sources:
[request DTO](https://github.com/haierkeys/fast-note-sync-service/blob/master/internal/dto/note_dto.go),
[REST handler](https://github.com/haierkeys/fast-note-sync-service/blob/master/internal/routers/api_router/handler_note.go),
[write service](https://github.com/haierkeys/fast-note-sync-service/blob/master/internal/service/note_service.go).

Distinct bounds: 24 touched notes, an 8 MiB encrypted proposal/review envelope,
existing 256 KiB per Markdown file and 16 MiB vault scan, a separate 120-second
background connector budget, and the bounded model I/O budget. Ordinary app reads
retain their seven-second budget. These protect memory, review work and
recovery; they do not limit vault storage or silently truncate a transcript.
The existing 4 MiB public vault response bound also applies to proposal replies.
Upgrade the hub first. The app forwards typed proposal calls through the existing
vault bridge. React polls active reviews and lists; a native observer posts a
localized notice when a queued or new proposal becomes ready, even with the
window closed. The observer wakes at startup, on the existing ready-note signal
and after explicit preparation; only queued/preparing proposals retain a timer.
Query/lint jobs, auto-apply policies, private-note inclusion and
document ingestion remain deferred.

## Acceptance evidence

The feature was developed from authenticated HTTP acceptance tests. The first
run reached `POST /v1/recordings/{id}/distillations` and failed with HTTP 404
before its route existed. Subsequent red cases caught stale input context,
interrupted preparation, a source exceeding the note byte ceiling before any
write, later index edits during undo, missing prose citations and removal of
independent YAML comments. The resulting focused suite has 36 passing cases:

- [proposal lifecycle](../../server/tests/test_distillation.py): all three
  contradiction choices, hashes, invalid/incomplete output, source hashes,
  discard and deletion;
- [versioned HTTP connector](../../server/tests/test_distillation_sync.py): a
  real loopback fake REST server, content/version conflicts, compensation and
  lost receipts and an independent background I/O budget, without inventing
  upstream transaction semantics;
- [repairs and drops](../../server/tests/test_distillation_repair.py):
  recorded answers with Obsidian links (Spanish) and unreadable task dates
  (English), which failed whole before the repair, a note left out with its
  cause while the rest is applied, and an answer with nothing valid left;
- [jobs and review](../../server/tests/test_distillation_jobs.py): ready-note
  worker integration, durable on-demand queueing, default/selected models,
  paired-device authorization, restarts, edits and exclusions, source citations,
  400-word warnings, CRLF/comments and custom provenance extensions.

The invented English Garden/Cedar vault has two known speakers (one matched
by an alias), a new named speaker and a project with schema-defined field names.
Both private notes and generated meeting-note text deliberately contain an
excluded marker, checked absent from the model request, proposal and source.
Every model answer is recorded; no test uses a real provider or real vault.
