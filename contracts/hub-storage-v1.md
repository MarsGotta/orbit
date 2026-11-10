# Orbit Hub storage v1

The hub keeps everything in one SQLite database, sealed value by value with one
key. This document is what another program needs to open, read and write that
database as the hub of this repository does, without reading its Python. What
the hub serves is in [the protocol](hub-v1.md).

**How it is written.** Three parts have a fixed form, which
`server/tests/test_contract_storage.py` reads and holds to the code: the
[Values](#values) table, the [Derived values](#derived-values) table, and one
block per table under [Tables](#tables). A block is headed by the table's name
in code (`` `devices` ``), has no heading inside, and holds two tables:

- one row per column, in the order SQLite reports them: its name, its declared
  type, whether it admits NULL (`yes`; a `TEXT` primary key admits NULL in
  SQLite), its place in the primary key, its default as SQLite reports it, and,
  when it is sealed, the kind of its plaintext and its context (`retired` in
  front when only earlier hubs wrote it);
- one row per unique key, reference, check or index, with a partial index's
  `WHERE`.

Everything else is prose. The files named in “Written by” are under
`server/orbit_hub/`.

## Files

| file | where | required | created |
|---|---|---|---|
| data directory | `data_dir` in the configuration | owned by the user the hub runs as, no permission for group or others | 0700 by `init` |
| database | `hub.sqlite3` in the data directory, with SQLite's `-wal` and `-shm` beside it | not a symbolic link; an existing file's mode is not checked | 0600 when missing |
| lock | `hub.lock` in the data directory | not a symbolic link; an existing file's mode is not checked | 0600 when missing |
| key | `key_file` in the configuration, a file of its own | a regular file owned by the user, not a link, no permission for group or others, exactly 32 bytes | 0600 by `init` |

- **The data directory** must belong to the user the hub runs as and give no
  permission to group or others: the hub refuses to open it otherwise. `init`
  creates it 0700.
- **The database** is in WAL mode. The hub refuses a database that is a
  symbolic link. Every time it opens the store it creates the file (0600,
  without following a link) if it is missing; it does not check the mode of a
  file that exists.
- **Other files.** The data directory also holds files that are not this
  database, each agent's folder under `agents/` and `secret-scan.json` among
  them. This contract does not cover them.
- **The lock.** A serving hub holds an exclusive, non-blocking `flock` on
  `hub.lock` from the start of its lifespan until it shuts down; a second
  `serve` that cannot take it fails to start. The file is created if missing
  and never removed. The lock is taken *after* the database is opened and
  migrated: building the application opens the store, then the lifespan takes
  the lock. So a second `serve` on the same data directory migrates the
  database before it stops.
- **The key** is 32 random bytes, the AES-256 key of every sealed value. It
  lives in a file of its own, never in the database and never in a nightly
  copy. The hub reads it only from a regular file owned by its user, not a
  link, with no permission for group or others, and refuses any length but 32.
  `init` writes it once (`O_EXCL`, so an existing key is never replaced) and
  syncs it and its directory to disk. The copy kept before an activation keeps
  it beside the database (below). Without it no sealed value can be read.

**The administration commands** (`orbit-hub --config … <command>`) open the
database without the lock, also while the hub serves:

- `init` refuses a data directory that already holds `hub.sqlite3` (or a link
  by that name) and a key file that already exists, writes the key, opens the
  store once (which creates the schema's tables, the key check and the hub's
  id) and closes it.
- `pair-code`, `revoke` and `devices` open the store, with its schema step,
  write a pairing code or a revocation or read the devices, and close it.
- `backup <directory>` copies the database with SQLite's online backup API
  from a query-only connection of its own, without the key. A copy is a single
  file in rollback-journal mode (`journal_mode=DELETE`), 0600.
- `restore <copy>` puts a copy into a data directory that has no database,
  then opens the store with the configured key, which checks the key and
  migrates the copy. If that fails it removes what it put there.

Between the hub and these commands only SQLite's own locking stands: WAL for
readers, `BEGIN IMMEDIATE` for writers, and the default busy timeout of
Python's `sqlite3` (5 seconds). A revocation written by `revoke` reaches the
serving hub through the database.

## Connections and transactions

- **The writer.** The hub opens one connection in autocommit mode
  (`isolation_level=None`) and runs `PRAGMA journal_mode=WAL`,
  `PRAGMA synchronous=FULL` and `PRAGMA foreign_keys=ON` on it. Every write of
  a domain runs on it inside `BEGIN IMMEDIATE` … `COMMIT`, under one lock of
  the process; a failure rolls back.
- **The separate reader.** Reads that must not wait for the writer open their
  own connection: read-only (`mode=ro` URI), `PRAGMA query_only=ON`, then
  `BEGIN`, so they see one snapshot of the WAL. Other reads use the writer's
  connection under its lock.
- **References** have no `ON DELETE` action. With `foreign_keys` on, whoever
  deletes removes the children first (see
  [Deleting a recording](#deleting-a-recording)).
- **Text** is UTF-8. The hub's clock is Unix time in seconds, kept in `REAL`
  columns. A column whose name ends in `_ms` holds milliseconds: a Unix time,
  except `messages.duration_ms`, a voice note's length.

## Values

| name | value |
|---|---|
| `database` | `hub.sqlite3` |
| `lock` | `hub.lock` |
| `journal_mode` | `wal` |
| `user_version` | `10` |
| `cipher` | `AES-256-GCM` |
| `key_bytes` | `32` |
| `nonce_bytes` | `12` |
| `tag_bytes` | `16` |
| `mark` | `OHK1` |
| `key_id_label` | `orbit.storage.key-id.v1` |
| `key_id_bytes` | `8` |
| `key_check_plaintext` | `orbit.storage.key-check.v1` |
| `fingerprint_label` | `orbit.operation.fingerprint.v1` |
| `reminder_label` | `orbit.reminder.key.v1` |

`database` and `lock` are file names in the data directory; `journal_mode` and
`user_version` are what SQLite reports for the database; the labels are ASCII
strings used as HMAC messages, below.

## Sealed values

Every `BLOB` column is sealed: AES-256-GCM with the key, a 12-byte nonce and
a 16-byte tag, and the value's **context** bound in as associated data. There
is no plaintext fallback. A sealed value has one of two forms:

- **today's form:** nonce (12) | ciphertext and tag. The associated data is
  the context, as UTF-8.
- **the marked form:** `mark` (the 4 ASCII bytes `OHK1`) | key id (8) |
  nonce (12) | ciphertext and tag. The key id is the first `key_id_bytes`
  bytes of HMAC-SHA256(key, `key_id_label`): derived from the key, never
  stored, so a copy restored with its key matches, and it says nothing of the
  key. The associated data is the 12-byte header (mark and key id) followed by
  the context as UTF-8, so neither can be swapped.

The two forms do not separate their associated data, so **no context may
begin with `OHK1`**.

**Reading.** A value that begins with this key's header and is at least 40
bytes long (header, nonce and tag: an empty marked value) is tried as marked
first. Whatever does not open that way is read as today's form, because a
random nonce of today's form may begin with the header. A mark of another key
is today's form to this hub, and fails to open.

**Writing.** This hub writes only today's form, which every earlier hub reads,
with a fresh random nonce for every value. So must any program that writes
while an earlier hub may still open the database.

**The context** of each sealed column is in its block, in code. `{column}`
stands for that column's value in the same row, as stored (every column a
context names is `TEXT`); the rest is literal. So `manifest:{device_id}:{recording}`
of a row with `device_id` `d1` and `recording` `r1` is `manifest:d1:r1`. A
context binds a value to its row only as far as it names the row's columns:
the [`messages`](#messages) contexts do not name the message.

**The plaintext** is one of:

- `text`: UTF-8 text;
- `json`: a UTF-8 JSON document. Some writers escape non-ASCII characters and
  some do not; a reader takes both. What is inside each document belongs to
  its domain, not to this contract;
- `bytes`: the bytes as they came, audio or a file.

## The key check and the hub's identity

**The key check** is the one row of [`storage_key_check`](#storage_key_check):
`key_check_plaintext` sealed under the context `storage:key-check`. Before the
schema step, opening a database checks the key:

1. a database with that row must open it to exactly `key_check_plaintext`;
2. a database without it (written before it existed) must open the `name` of
   its first device by `rowid`, with the context `device:{id}`;
3. a database with neither, a new one among them, takes the key it is opened
   with.

Any other result refuses the database with the wrong-key error, before the
schema step touches it. Otherwise the schema step writes the row if it is missing (`INSERT OR
IGNORE`): a database never ends up with values sealed by two keys.

**The hub's identity** is the one row of [`hub_identity`](#hub_identity): a
random UUID, written the first time a hub opens the database and never
changed. A restored copy keeps it, because it is the same hub. `GET /health`
and `ready` report it as `hub_id`.

## Derived values

| column | how |
|---|---|
| `devices.token_hash` | SHA-256 of the device's bearer token |
| `pair_codes.hash` | SHA-256 of the pairing code |
| `chunks.sha256` | SHA-256 of the chunk's audio |
| `chunks.size` | length of the chunk's audio |
| `attachments.sha256` | SHA-256 of the file |
| `attachments.size` | length of the file |
| `operations.fingerprint` | HMAC-SHA256 with the fingerprint key, of the request's canonical JSON |
| `safety_operations.fingerprint` | the same, of the control's request |
| `safety_operations.target` | SHA-256 of the control's target as Python's default JSON |
| `personal_reminders.key` | HMAC-SHA256 with the reminder key, of the meeting's title digest |
| `vault_working_copy.namespace` | SHA-256 of the vault's identity as Python's default JSON |
| `vault_working_copy.id` | SHA-256 of the note's path |
| `vault_distillations.namespace` | the same as `vault_working_copy.namespace` |

Every hash and HMAC is stored as 64 lowercase hexadecimal characters, and
every string is hashed as UTF-8. Sizes are in bytes, of the plaintext.

- **Tokens and codes.** A bearer token is 32 random bytes and a pairing code
  24, each in URL-safe Base64 without padding; only their hashes are kept.
- **Audio and files.** `sha256` and `size` are those of the plaintext, before
  sealing. The Mac sends the chunk's hash, and the hub refuses a chunk whose
  bytes do not match it.
- **The fingerprint of an operation.** The fingerprint key is the 32 raw
  bytes of HMAC-SHA256(key, `fingerprint_label`). The canonical JSON is the
  request with sorted keys, no spaces (`,` and `:`), and non-ASCII characters
  escaped as `\uXXXX`. The request is the WebSocket frame as the hub's model
  of it dumps it, every optional field filled in, without `op`
  (`ws_reader.py`); a device-bridge request is the payload `bridge_route.py`
  builds. A program that fills the fields differently computes another
  fingerprint, and the hub answers `operation_conflict` to a request sent
  again across that change.
- **The target of a control** is SHA-256 of the JSON array `[method,
  session_id, turn_id, request_id]` as Python's `json.dumps` writes it by
  default: `, ` between items, non-ASCII escaped. `method` is the frame's
  method, `chat.interrupt` or `approval.respond`; `turn_id` is the turn the
  control is for; the fourth item is the `request_id` a denial answers, and
  `""` for an interrupt. An interrupt hashes, for example,
  `["chat.interrupt", "s-1", "t-1", ""]`; a denial,
  `["approval.respond", "s-1", "t-1", "r-1"]`.
- **The key of a reminder.** The reminder key is the 32 raw bytes of
  HMAC-SHA256(key, `reminder_label`); the row's key is HMAC-SHA256(reminder
  key, the `{key}` of `/v1/reminders/{key}`): the meeting's title digest, 64
  lowercase hexadecimal characters, as ASCII. So a copy of the database alone
  cannot test titles against it.
- **The namespace of a vault** is SHA-256 of the JSON array `[kind, path,
  url, vault]` of its configuration, as Python's `json.dumps` writes it by
  default; a field that is not set is `""`, never `null`. A folder vault
  hashes, for example, `["folder", "/srv/vault", "", ""]`. A working-copy
  note's `id` is SHA-256 of its path in the vault.

## The schema and how it changes

**`user_version` is not a version.** Every open writes `PRAGMA
user_version=10`, whatever the database held. Which schema a database has is
told by its columns, not by that number.

**The schema step** runs every time the store opens (`storage/schema.py`),
after the key check and outside a transaction, each statement committing on
its own. It creates the tables and indexes that are missing
(`CREATE … IF NOT EXISTS`) and adds the columns an older database lacks, by
`ALTER TABLE … ADD COLUMN`, in this order:

1. `devices.last_seen` (then set to `created` in every row), `devices.kind`,
   `devices.capabilities`, `devices.installation_id`;
2. `sessions.agent_id`;
3. `recording_manifests.title`;
4. `messages.created_at_ms`, `messages.kind`, `messages.duration_ms`,
   `messages.client_id`, `messages.work`;
5. `transcription_groups.language`;
6. `recording_final_passes.progress`, `recording_final_passes.kept_draft`,
   `recording_final_passes.refresh_voices`;
7. `transcription_groups.model`, `recording_final_passes.model`.

Seventeen columns. Some of them are added even to a new database, whose
`CREATE TABLE` lacks them; an older database may hold some of them in another
position than a new one (`devices.last_seen` after `revoked`, for one). Read
and write those tables by column name.

**`recording_dictionaries` is rebuilt** once, in its own transaction, when
its primary key lacks `dictionary_id` (one dictionary per meeting, before
lists): a new table with the key `(device_id, recording, dictionary_id)`
takes every row as it is, nothing read or decrypted, the old one is dropped
and the new one renamed.

**Six tables are created when the hub starts**, not by the schema step:
`recording_notes`, `note_templates` and `note_template_links` (the notes),
`vault_configuration` and `vault_working_copy` (the vault) and
`vault_distillations` (the distillation), each by its domain when the hub
builds its application. A database opened only by `init` or the other
administration commands has the other 40. Deleting a recording deletes from
`vault_distillations`, a table of the Notes plugin, without asking whether it
exists: the hub creates it when it builds the application that serves the
deletion.

**Fifteen tables are inserted by position** (`INSERT INTO t VALUES (…)`):
`ai_settings`, `chunk_receipts`, `chunks`, `group_transcripts`,
`meeting_dictionaries`, `recording_dictionaries`,
`recording_final_transcripts`, `recording_speakers`, `recording_voices`,
`safety_operations`, `transcription_group_chunks`,
`transcription_preferences`, `vault_configuration`, `vault_distillations`
and `vault_working_copy`. A column added to any of them breaks this hub's
writes to it.

**Going back.** Before an activation moves to another release, it keeps a
consistent copy of the database, with the key it was taken with, labelled
with the release it was taken from (`scripts/hub/activate.sh`,
`server/orbit_hub/backup.py`).
Activating that release again compares the database's schema with the
copy's: `user_version` and every row of `sqlite_master` (type, name, table and
SQL text). The same schema keeps the database. Any difference, a column, an
index or a single character of SQL, stops the hub and puts the copy back; the
database it replaces is moved aside, with everything written since, never
deleted. A program that changes the schema in any way makes going back lose,
from the live database, what was written after the copy.

**Old tables stay.** A database written by an earlier hub may hold tables
this hub neither creates nor reads. Nothing drops them; a reader ignores what
it does not know. The same goes for the retired columns marked in the blocks:
the schema still creates them, nobody writes or reads them, and deleting a
recording empties its rows of `transcripts`.

## Tables

Forty-six tables, by domain.

### The store itself

#### `storage_key_check`

One row, `id` 1: `key_check_plaintext` sealed under `storage:key-check`,
written the first time a hub opens the database and never changed (see [The key
check and the hub's identity](#the-key-check-and-the-hubs-identity)).

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `id` | `INTEGER` | yes | 1 |  |  |
| `body` | `BLOB` | no |  |  | text, `storage:key-check` |

| kind | columns | detail |
|---|---|---|
| check | `id = 1` | |

Written by: `storage/schema.py`, with `INSERT OR IGNORE`.

#### `hub_identity`

One row: the hub's id, a random UUID written the first time a hub opens the
database (`INSERT … WHERE NOT EXISTS`) and never changed. The schema does not
hold it to one row; the code does.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `id` | `TEXT` | yes | 1 |  |  |

Written by: `storage/schema.py`.

### Devices

#### `devices`

One row per paired device, never deleted: a revocation sets `revoked` to 1.
`id` is a random UUID; `name` is the name the device gave when it paired;
`token_hash` is the hash of its bearer token, which is not stored. `created`
and `last_seen` are Unix seconds; `last_seen` only grows. `kind` is `mac` or
`phone`; `capabilities` is the JSON array of device groups it handles, NULL for
every group; `installation_id` is its installation's UUID, or NULL when
unknown.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `id` | `TEXT` | yes | 1 |  |  |
| `name` | `BLOB` | no |  |  | text, `device:{id}` |
| `token_hash` | `TEXT` | no |  |  |  |
| `created` | `REAL` | no |  |  |  |
| `last_seen` | `REAL` | no |  | `0` |  |
| `revoked` | `INTEGER` | no |  | `0` |  |
| `kind` | `TEXT` | no |  | `'mac'` |  |
| `capabilities` | `TEXT` | yes |  |  |  |
| `installation_id` | `TEXT` | yes |  |  |  |

| kind | columns | detail |
|---|---|---|
| unique | `token_hash` | |

Written by: `storage/devices.py` (pairing, revocation, `last_seen`);
`storage/schema.py` sets `last_seen` to `created` when it adds that column.

#### `pair_codes`

One row per pairing code: its hash, when it expires (Unix seconds) and whether
it was used. Creating a code deletes the expired and used ones. The code itself
is shown once and not stored.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `hash` | `TEXT` | yes | 1 |  |  |
| `expires` | `REAL` | no |  |  |  |
| `used` | `INTEGER` | no |  | `0` |  |

Written by: `storage/devices.py`.

#### `device_profiles`

Per device, the owner's name as that device set it: `{"owner_name": …}`.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `device_id` | `TEXT` | yes | 1 |  |  |
| `body` | `BLOB` | no |  |  | json, `device-profile:{device_id}` |
| `updated` | `REAL` | no |  |  |  |

| kind | columns | detail |
|---|---|---|
| references | `device_id` | `devices(id)` |

Written by: `meetings/repository.py`.

### Conversations

#### `sessions`

One row per conversation. `id` is the hub's own (a random UUID); `runtime_id`,
`stored_id` and `epoch` are how the agent's runtime knows it; `created` is Unix
seconds; `agent_id` is NULL in conversations stored before agents, which belong
to the default agent. When the hub starts, a conversation whose `status` is
`submitting`, `streaming` or `interrupting` becomes `outcome_unknown`.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `id` | `TEXT` | yes | 1 |  |  |
| `device_id` | `TEXT` | no |  |  |  |
| `runtime_id` | `TEXT` | no |  |  |  |
| `stored_id` | `TEXT` | no |  |  |  |
| `epoch` | `TEXT` | no |  |  |  |
| `status` | `TEXT` | no |  |  |  |
| `created` | `REAL` | no |  |  |  |
| `agent_id` | `TEXT` | yes |  |  |  |

| kind | columns | detail |
|---|---|---|
| references | `device_id` | `devices(id)` |

Written by: `storage/chat.py`; `storage/database.py` at start.

#### `messages`

One row per message, in `id` order. `role` is `user` or `assistant`;
`created_at_ms` is Unix milliseconds, NULL in rows from before it; `kind` is
`text` or `voice`, with `duration_ms` for a voice note; `client_id` is the
Mac's id for the message; `work` is an answer's summary of its turn. The
context of `body` names the conversation and the role, not the message: two
messages of the same role in the same conversation can be swapped without the
cipher noticing. `work` names only the conversation.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `id` | `INTEGER` | yes | 1 |  |  |
| `session_id` | `TEXT` | no |  |  |  |
| `role` | `TEXT` | no |  |  |  |
| `body` | `BLOB` | no |  |  | text, `message:{session_id}:{role}` |
| `created_at_ms` | `INTEGER` | yes |  |  |  |
| `kind` | `TEXT` | no |  | `'text'` |  |
| `duration_ms` | `INTEGER` | yes |  |  |  |
| `client_id` | `TEXT` | yes |  |  |  |
| `work` | `BLOB` | yes |  |  | json, `message-work:{session_id}` |

| kind | columns | detail |
|---|---|---|
| index `messages_session` | `session_id, id` |  |
| references | `session_id` | `sessions(id)` |

Written by: `storage/chat.py`.

#### `attachments`

Files sent with a message. A file is stored unbound (`message_id` and
`session_id` NULL) and bound when its message is stored; one left unbound
longer than `UNBOUND_TTL_MS` (in `attachments.py`) is deleted. `size` and
`sha256` are the file's (see [Derived values](#derived-values)); `pages` is a
PDF's page count; `extract` is a PDF's page texts as a JSON array; `created_ms`
is Unix milliseconds.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `id` | `TEXT` | yes | 1 |  |  |
| `device_id` | `TEXT` | no |  |  |  |
| `session_id` | `TEXT` | yes |  |  |  |
| `message_id` | `INTEGER` | yes |  |  |  |
| `kind` | `TEXT` | no |  |  |  |
| `mime` | `TEXT` | no |  |  |  |
| `size` | `INTEGER` | no |  |  |  |
| `sha256` | `TEXT` | no |  |  |  |
| `pages` | `INTEGER` | yes |  |  |  |
| `name` | `BLOB` | no |  |  | text, `attachment:{id}:name` |
| `body` | `BLOB` | no |  |  | bytes, `attachment:{id}:body` |
| `extract` | `BLOB` | yes |  |  | json, `attachment:{id}:extract` |
| `created_ms` | `INTEGER` | no |  |  |  |

| kind | columns | detail |
|---|---|---|
| index `attachments_message` | `message_id` |  |
| index `attachments_session` | `session_id` |  |
| index `attachments_unbound` | `device_id, message_id` |  |
| index `attachments_unbound_age` | `created_ms` | `message_id IS NULL` |
| references | `device_id` | `devices(id)` |
| references | `message_id` | `messages(id)` |
| references | `session_id` | `sessions(id)` |

Written by: `attachments.py`.

#### `session_handovers`

Which conversation, and of which agent, a conversation was handed over from.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `session_id` | `TEXT` | yes | 1 |  |  |
| `from_session_id` | `TEXT` | no |  |  |  |
| `from_agent_id` | `TEXT` | no |  |  |  |

Written by: `assistant/handover.py`.

### Operation receipts

A device's request that must not be applied twice reserves its id before
anything happens and keeps its answer once known; the same id sent again is
answered from the receipt.

#### `operations`

One row per operation id of a device: its fingerprint (see [Derived
values](#derived-values)), `status` `pending` until its answer is kept in
`response`, then `completed`. Only a `pending` reservation is ever deleted,
when the operation was refused before it did anything.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `id` | `TEXT` | no | 2 |  |  |
| `device_id` | `TEXT` | no | 1 |  |  |
| `fingerprint` | `TEXT` | no |  |  |  |
| `status` | `TEXT` | no |  |  |  |
| `response` | `BLOB` | yes |  |  | json, `operation:{device_id}:{id}` |

| kind | columns | detail |
|---|---|---|
| references | `device_id` | `devices(id)` |

Written by: `storage/operations.py`, for the WebSocket (`ws_reader.py`,
`ws_dispatch.py`) and the device bridge (`bridge_route.py`).

#### `safety_operations`

The receipts of controls on a turn (an interrupt, a denial), one per `target`
of a device, at most `MAX_SAFETY_TARGETS` (in `safety.py`) per device. `response` is
sealed with the target, not the operation id. Reserving a control deletes the
receipts of targets that are no longer live, and the receipt of its own target
when that answer was an error. An operation id of a device is either here or in
`operations`, never both. Inserted by position.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `device_id` | `TEXT` | no | 1 |  |  |
| `target` | `TEXT` | no | 2 |  |  |
| `session_id` | `TEXT` | no |  |  |  |
| `turn_id` | `TEXT` | no |  |  |  |
| `operation_id` | `TEXT` | no |  |  |  |
| `fingerprint` | `TEXT` | no |  |  |  |
| `response` | `BLOB` | yes |  |  | json, `safety:{device_id}:{target}` |

| kind | columns | detail |
|---|---|---|
| references | `device_id` | `devices(id)` |
| unique | `device_id, operation_id` | |

Written by: `safety.py`, called from `ws_reader.py` and `ws_dispatch.py`.

### Recordings

A Mac uploads a recording chunk by chunk, then closes it with its manifest.

#### `chunks`

One row per audio chunk. `id` is the Mac's; `recording`, `track` and `sequence`
place it; `sha256` and `size` are those of the audio before it is sealed (the
hub refuses a chunk whose bytes do not match the hash the Mac sent). Inserted
by position.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `id` | `TEXT` | no | 2 |  |  |
| `device_id` | `TEXT` | no | 1 |  |  |
| `recording` | `TEXT` | no |  |  |  |
| `track` | `TEXT` | no |  |  |  |
| `sequence` | `INTEGER` | no |  |  |  |
| `sha256` | `TEXT` | no |  |  |  |
| `body` | `BLOB` | no |  |  | bytes, `chunk:{device_id}:{id}` |
| `size` | `INTEGER` | no |  |  |  |

| kind | columns | detail |
|---|---|---|
| references | `device_id` | `devices(id)` |
| unique | `device_id, recording, track, sequence` | |

Written by: `storage/recordings.py`; deleted with its recording.

#### `transcription_jobs`

One row per chunk that came with capture metadata: the metadata (JSON with
sorted keys) and the chunk's place in the hub's work. When the hub starts, a
job whose `state` is `running` goes back to `pending`.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `device_id` | `TEXT` | no | 1 |  |  |
| `chunk_id` | `TEXT` | no | 2 |  |  |
| `metadata` | `BLOB` | no |  |  | json, `metadata:{device_id}:{chunk_id}` |
| `state` | `TEXT` | no |  | `'pending'` |  |
| `attempts` | `INTEGER` | no |  | `0` |  |
| `next_attempt` | `REAL` | no |  | `0` |  |
| `error_code` | `TEXT` | yes |  |  |  |

| kind | columns | detail |
|---|---|---|
| references | `device_id, chunk_id` | `chunks(device_id, id)` |

Written by: `storage/recordings.py`, `group_storage.py`; `storage/database.py`
at start.

#### `chunk_receipts`

When the hub received each chunk that came with metadata. Inserted by position.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `device_id` | `TEXT` | no | 1 |  |  |
| `chunk_id` | `TEXT` | no | 2 |  |  |
| `received` | `REAL` | no |  |  |  |

| kind | columns | detail |
|---|---|---|
| references | `device_id, chunk_id` | `chunks(device_id, id)` |

Written by: `storage/recordings.py`.

#### `transcripts`

Retired. Per-chunk transcripts of earlier hubs. This hub creates the table,
never writes or reads it, and deleting a recording empties its rows.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `device_id` | `TEXT` | no | 1 |  |  |
| `chunk_id` | `TEXT` | no | 2 |  |  |
| `body` | `BLOB` | no |  |  | retired: json, `transcript:{device_id}:{chunk_id}` |

| kind | columns | detail |
|---|---|---|
| references | `device_id, chunk_id` | `chunks(device_id, id)` |

Written by: nobody; emptied by `meetings/deletion.py`.

#### `recording_manifests`

The manifest that closes a recording (JSON with sorted keys). Once it exists
the recording takes no more chunks. `title` is retired: earlier hubs sealed the
meeting's title there; this hub never writes or reads it, and leaves it NULL.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `device_id` | `TEXT` | no | 1 |  |  |
| `recording` | `TEXT` | no | 2 |  |  |
| `body` | `BLOB` | no |  |  | json, `manifest:{device_id}:{recording}` |
| `title` | `BLOB` | yes |  |  | retired: text, `manifest-title:{device_id}:{recording}` |

| kind | columns | detail |
|---|---|---|
| references | `device_id` | `devices(id)` |

Written by: `storage/recordings.py`.

#### `deleted_recordings`

The recordings each device deleted: identities and a time, no content. A late
chunk, manifest or vault proposal for one of them is refused, so a recording
cannot come back half.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `device_id` | `TEXT` | no | 1 |  |  |
| `recording` | `TEXT` | no | 2 |  |  |
| `deleted` | `REAL` | no |  |  |  |

| kind | columns | detail |
|---|---|---|
| references | `device_id` | `devices(id)` |

Written by: `meetings/deletion.py`.

### Transcription

#### `transcription_groups`

Consecutive chunks of a track transcribed together. `manifest` describes the
group; `state`, `attempts`, `next_attempt` and `error_code` are its place in
the hub's work; `language` and `model` are what it was transcribed with, NULL
in groups stored before them. When the hub starts, a group whose `state` is
`running` goes back to `pending`.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `device_id` | `TEXT` | no | 1 |  |  |
| `id` | `TEXT` | no | 2 |  |  |
| `recording` | `TEXT` | no |  |  |  |
| `track` | `TEXT` | no |  |  |  |
| `first_sequence` | `INTEGER` | no |  |  |  |
| `last_sequence` | `INTEGER` | no |  |  |  |
| `manifest` | `BLOB` | no |  |  | json, `group:{device_id}:{id}` |
| `state` | `TEXT` | no |  |  |  |
| `attempts` | `INTEGER` | no |  | `0` |  |
| `next_attempt` | `REAL` | no |  | `0` |  |
| `error_code` | `TEXT` | yes |  |  |  |
| `created` | `REAL` | no |  |  |  |
| `language` | `TEXT` | yes |  |  |  |
| `model` | `TEXT` | yes |  |  |  |

| kind | columns | detail |
|---|---|---|
| index `groups_pending` | `state, next_attempt, created` |  |
| index `groups_recording` | `device_id, recording, last_sequence DESC` |  |

Written by: `group_storage.py`, `transcription.py`, `meetings/repository.py` (a
change of language); `storage/database.py` at start.

#### `transcription_group_chunks`

The group each chunk belongs to. Inserted by position.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `device_id` | `TEXT` | no | 1 |  |  |
| `chunk_id` | `TEXT` | no | 2 |  |  |
| `group_id` | `TEXT` | no |  |  |  |

| kind | columns | detail |
|---|---|---|
| index `group_members` | `device_id, group_id` |  |
| references | `device_id, chunk_id` | `chunks(device_id, id)` |
| references | `device_id, group_id` | `transcription_groups(device_id, id)` |

Written by: `group_storage.py`.

#### `group_transcripts`

A group's text, `{"text": …}`, kept even when empty. Inserted by position.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `device_id` | `TEXT` | no | 1 |  |  |
| `group_id` | `TEXT` | no | 2 |  |  |
| `body` | `BLOB` | no |  |  | json, `group-transcript:{device_id}:{group_id}` |

| kind | columns | detail |
|---|---|---|
| references | `device_id, group_id` | `transcription_groups(device_id, id)` |

Written by: `group_storage.py`.

#### `recording_languages`

The language chosen for a recording, and how many of its groups that choice
sent to be transcribed again (`repeated`).

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `device_id` | `TEXT` | no | 1 |  |  |
| `recording` | `TEXT` | no | 2 |  |  |
| `language` | `TEXT` | no |  |  |  |
| `repeated` | `INTEGER` | no |  |  |  |
| `updated` | `REAL` | no |  |  |  |

| kind | columns | detail |
|---|---|---|
| references | `device_id` | `devices(id)` |

Written by: `meetings/repository.py`.

#### `recording_final_passes`

A recording's final pass: its `state`, `error` and `kept_draft` (JSON text),
`progress`, whether it refreshes the voices, and its `model`. When the hub
starts, a pass whose `state` is `running` goes back to `queued`.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `device_id` | `TEXT` | no | 1 |  |  |
| `recording` | `TEXT` | no | 2 |  |  |
| `state` | `TEXT` | no |  |  |  |
| `error` | `TEXT` | yes |  |  |  |
| `progress` | `TEXT` | yes |  |  |  |
| `kept_draft` | `TEXT` | yes |  |  |  |
| `refresh_voices` | `INTEGER` | no |  | `0` |  |
| `model` | `TEXT` | yes |  |  |  |

| kind | columns | detail |
|---|---|---|
| references | `device_id` | `devices(id)` |

Written by: `storage/recordings.py` (when a recording is closed),
`meeting_final_pass.py`, `meetings/repository.py`; `storage/database.py` at
start.

#### `recording_final_transcripts`

The final pass's words: `words`, `audio_ms` and `language`. Inserted by
position.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `device_id` | `TEXT` | no | 1 |  |  |
| `recording` | `TEXT` | no | 2 |  |  |
| `body` | `BLOB` | no |  |  | json, `recording-final-transcript:{device_id}:{recording}` |

| kind | columns | detail |
|---|---|---|
| references | `device_id` | `devices(id)` |

Written by: `meeting_voice_identity.py`.

#### `recording_voices`

The final pass's voice intervals: `{"spans": …}`. Inserted by position.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `device_id` | `TEXT` | no | 1 |  |  |
| `recording` | `TEXT` | no | 2 |  |  |
| `body` | `BLOB` | no |  |  | json, `recording-voices:{device_id}:{recording}` |

| kind | columns | detail |
|---|---|---|
| references | `device_id` | `devices(id)` |

Written by: `meeting_voice_identity.py`.

### What the owner adds to a meeting

#### `recording_titles`

The title the owner gave a recording: `{"title": …}`.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `device_id` | `TEXT` | no | 1 |  |  |
| `recording` | `TEXT` | no | 2 |  |  |
| `body` | `BLOB` | no |  |  | json, `recording-title:{device_id}:{recording}` |
| `updated` | `REAL` | no |  |  |  |

| kind | columns | detail |
|---|---|---|
| references | `device_id` | `devices(id)` |

Written by: `meetings/repository.py`.

#### `recording_speakers`

The names given to a recording's voices. Rows of earlier hubs are a flat map of
key to name, rewritten in today's shape on the next write. A final pass that
refreshes the voices of a recording with no row yet inserts it by position,
with `updated` 0.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `device_id` | `TEXT` | no | 1 |  |  |
| `recording` | `TEXT` | no | 2 |  |  |
| `body` | `BLOB` | no |  |  | json, `recording-speakers:{device_id}:{recording}` |
| `updated` | `REAL` | no |  |  |  |

| kind | columns | detail |
|---|---|---|
| references | `device_id` | `devices(id)` |

Written by: `meetings/repository.py`, `meeting_voice_identity.py`.

#### `recording_private_notes`

The owner's private notes on a recording, `{"text": …}`, with a `revision` that
grows on each save.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `device_id` | `TEXT` | no | 1 |  |  |
| `recording` | `TEXT` | no | 2 |  |  |
| `body` | `BLOB` | no |  |  | json, `recording-private-notes:{device_id}:{recording}` |
| `revision` | `INTEGER` | no |  |  |  |
| `updated` | `REAL` | no |  |  |  |

| kind | columns | detail |
|---|---|---|
| references | `device_id` | `devices(id)` |

Written by: `meetings/repository.py`.

#### `personal_reminders`

The owner's reminder for a meeting, shared by every Mac. The row is keyed by an
HMAC of the meeting's title digest (see [Derived values](#derived-values)),
never by its title; `body` is the text. An edit whose `edited_at_ms` is not
later than the kept one's is ignored: on a tie, what is stored wins.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `key` | `TEXT` | yes | 1 |  |  |
| `body` | `BLOB` | no |  |  | text, `reminder:{key}` |
| `edited_at_ms` | `INTEGER` | no |  |  |  |

Written by: `storage/reminders.py`.

#### `meeting_dictionaries`

The owner's dictionaries, one JSON document each, shared by every Mac. A row
keeps the `device_id` it was first saved under, whichever Mac saves it later,
and its context uses that one. Inserted by position.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `device_id` | `TEXT` | no | 1 |  |  |
| `id` | `TEXT` | no | 2 |  |  |
| `body` | `BLOB` | no |  |  | json, `dictionary:{device_id}:{id}` |

Written by: `meetings/dictionaries.py`.

#### `recording_dictionaries`

The dictionaries a meeting uses, one row each. Inserted by position.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `device_id` | `TEXT` | no | 1 |  |  |
| `recording` | `TEXT` | no | 2 |  |  |
| `dictionary_id` | `TEXT` | no | 3 |  |  |

Written by: `meetings/dictionaries.py`.

### Notes

#### `recording_notes`

A recording's note: `state` and `revision` (it grows on every change, so an
edit based on an older one is refused), its work columns, and four sealed
columns whose context is `note-` followed by the column's name: what the note
is written from, the note, the owner's edits, and the edits set aside when it
was written again.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `device_id` | `TEXT` | no | 1 |  |  |
| `recording` | `TEXT` | no | 2 |  |  |
| `state` | `TEXT` | no |  |  |  |
| `revision` | `INTEGER` | no |  | `0` |  |
| `attempts` | `INTEGER` | no |  | `0` |  |
| `next_attempt` | `REAL` | no |  | `0` |  |
| `error` | `TEXT` | yes |  |  |  |
| `inputs` | `BLOB` | no |  |  | json, `note-inputs:{device_id}:{recording}` |
| `note` | `BLOB` | yes |  |  | json, `note-note:{device_id}:{recording}` |
| `edits` | `BLOB` | yes |  |  | json, `note-edits:{device_id}:{recording}` |
| `superseded` | `BLOB` | yes |  |  | json, `note-superseded:{device_id}:{recording}` |
| `requested` | `REAL` | no |  |  |  |
| `updated` | `REAL` | no |  |  |  |

| kind | columns | detail |
|---|---|---|
| index `recording_notes_work` | `state, next_attempt` |  |

Written by: `notes/repository.py`.

#### `note_templates`

The owner's own note templates, one JSON document each. Like a dictionary, a
row keeps the `device_id` it was first saved under, and its context uses that
one.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `device_id` | `TEXT` | no | 1 |  |  |
| `id` | `TEXT` | no | 2 |  |  |
| `body` | `BLOB` | no |  |  | json, `note-template:{device_id}:{id}` |
| `created` | `REAL` | no |  |  |  |
| `updated` | `REAL` | no |  |  |  |

Written by: `notes/custom_templates.py`.

#### `note_template_links`

Which template writes the notes of a calendar series, keyed by the series.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `device_id` | `TEXT` | no | 1 |  |  |
| `series` | `TEXT` | no | 2 |  |  |
| `template_id` | `TEXT` | no |  |  |  |
| `updated` | `REAL` | no |  |  |  |

| kind | columns | detail |
|---|---|---|
| index `note_template_links_template` | `device_id, template_id` |  |

Written by: `notes/custom_templates.py`.

### Agents

#### `agent_profiles`

Each agent's name, description and color.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `agent_id` | `TEXT` | yes | 1 |  |  |
| `name` | `TEXT` | no |  |  |  |
| `description` | `TEXT` | no |  |  |  |
| `color` | `TEXT` | no |  |  |  |

Written by: `assistant/roster.py`.

#### `agent_primary`

One row, `slot` 1: the primary agent.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `slot` | `INTEGER` | yes | 1 |  |  |
| `agent_id` | `TEXT` | no |  |  |  |

| kind | columns | detail |
|---|---|---|
| check | `slot = 1` | |

Written by: `assistant/roster.py`.

#### `added_agents`

The agents added from Orbit, with their service and when (Unix milliseconds).

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `agent_id` | `TEXT` | yes | 1 |  |  |
| `service` | `TEXT` | no |  |  |  |
| `added_at_ms` | `INTEGER` | no |  |  |  |

Written by: `assistant/roster.py`.

#### `agent_accounts`

The account state of an agent the hub can sign in. `email` and `plan` are not
sealed.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `agent_id` | `TEXT` | yes | 1 |  |  |
| `state` | `TEXT` | no |  |  |  |
| `limited` | `INTEGER` | no |  |  |  |
| `limited_until_ms` | `INTEGER` | yes |  |  |  |
| `email` | `TEXT` | yes |  |  |  |
| `plan` | `TEXT` | yes |  |  |  |

Written by: `assistant/roster.py`.

#### `agent_personalities`

An agent's personality, as text.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `agent_id` | `TEXT` | yes | 1 |  |  |
| `body` | `BLOB` | no |  |  | text, `agent-personality:{agent_id}` |

Written by: `assistant/agent_config.py`; deleted with the agent by
`assistant/roster.py`.

#### `agent_skills`

An agent's skills, one row each: `{"description", "content"}`, and whether it
is enabled.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `agent_id` | `TEXT` | no | 1 |  |  |
| `name` | `TEXT` | no | 2 |  |  |
| `enabled` | `INTEGER` | no |  |  |  |
| `body` | `BLOB` | no |  |  | json, `agent-skill:{agent_id}:{name}` |

Written by: `assistant/agent_config.py`; deleted with the agent by
`assistant/roster.py`.

#### `agent_workspaces`

An agent's working folder, as a path.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `agent_id` | `TEXT` | yes | 1 |  |  |
| `body` | `BLOB` | no |  |  | text, `agent-workspace:{agent_id}` |

Written by: `assistant/agent_config.py`; deleted with the agent by
`assistant/roster.py`.

#### `agent_access`

Which groups of tools an agent may use: a JSON object of group to boolean, not
sealed.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `agent_id` | `TEXT` | yes | 1 |  |  |
| `groups` | `TEXT` | no |  |  |  |

Written by: `assistant/agent_config.py`; deleted with the agent by
`assistant/roster.py`.

### Settings

#### `ai_settings`

One row, `id` 1: the provider registry, `{"providers", "functions"}`, API keys
included. Inserted by position.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `id` | `INTEGER` | yes | 1 |  |  |
| `body` | `BLOB` | no |  |  | json, `hub-ai-settings` |

| kind | columns | detail |
|---|---|---|
| check | `id = 1` | |

Written by: `ai/registry.py`.

#### `transcription_preferences`

One row, `id` 1: the older transcription preference, `{"model", "api_key"}`.
Saving it also writes it into `ai_settings`; it is read only while
`ai_settings` has no provider and no function, and when the hub loads with no
`ai_settings` row, which it then copies there. Inserted by position.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `id` | `INTEGER` | yes | 1 |  |  |
| `body` | `BLOB` | no |  |  | json, `hub-transcription-preference` |

| kind | columns | detail |
|---|---|---|
| check | `id = 1` | |

Written by: `transcription_preferences.py`.

### Vault

#### `vault_configuration`

One row, `id` 1: the vault connector's configuration, its token included.
Inserted by position.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `id` | `INTEGER` | yes | 1 |  |  |
| `body` | `BLOB` | no |  |  | json, `vault-configuration:v1` |

| kind | columns | detail |
|---|---|---|
| check | `id = 1` | |

Written by: `vault/service.py`.

#### `vault_working_copy`

The hub's copy of each vault note it read, `{"path", "raw", "stamp"}`, per
vault (`namespace`) and note (`id`); see [Derived values](#derived-values).
Connecting another vault or disconnecting empties it, and a note gone from the
vault is pruned. Inserted by position.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `namespace` | `TEXT` | no | 1 |  |  |
| `id` | `TEXT` | no | 2 |  |  |
| `body` | `BLOB` | no |  |  | json, `vault-copy:{namespace}:{id}` |

Written by: `vault/working_copy.py`, emptied through it by `vault/service.py`.

#### `vault_distillations`

The proposals that distill a meeting into the vault, one JSON document each;
`id` is a random UUID, `namespace` the vault's, `state` the proposal's status.
Inserted by position.

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `id` | `TEXT` | yes | 1 |  |  |
| `device_id` | `TEXT` | no |  |  |  |
| `recording` | `TEXT` | no |  |  |  |
| `namespace` | `TEXT` | no |  |  |  |
| `state` | `TEXT` | no |  |  |  |
| `body` | `BLOB` | no |  |  | json, `vault-distillation:{id}` |

| kind | columns | detail |
|---|---|---|
| index `vault_distillations_meeting` | `device_id, recording` |  |
| index `vault_distillations_work` | `namespace, state` |  |

Written by: `distillation/repository.py`.

## Deleting a recording

Deleting a recording (`meetings/deletion.py`) removes, in one transaction,
the rows of one device's recording from 17 tables of the hub, most of them
other domains', children before their parents, and then from its plugins'
tables:

1. by its chunks: `transcripts`, `transcription_jobs`, `chunk_receipts`,
   `transcription_group_chunks`;
2. by its groups: `group_transcripts`;
3. by the recording: `transcription_groups`, `chunks`, `recording_manifests`,
   `recording_final_passes`, `recording_final_transcripts`,
   `recording_voices`, `recording_speakers`, `recording_private_notes`,
   `recording_titles`, `recording_languages`, `recording_notes` and
   `recording_dictionaries`;
4. the tables keyed by device and recording that each plugin of the hub
   names (`plugins.py`): today `vault_distillations`, of Notes, which the hub
   creates when it builds its application.

Then it adds the recording to `deleted_recordings` (`INSERT OR IGNORE`), so
nothing of it is taken again. The tables are named in variables, so a search
for a table's name in the code does not find this deletion; a new table with
rows of a recording has to join the list. The dictionaries themselves,
the templates and their links, and the reminders stay. The nightly copies
keep the recording for as long as they are kept.

## Removing an agent

Removing an agent added from Orbit (`assistant/roster.py`) deletes, in one
transaction, its rows from 8 tables: `added_agents`, `agent_profiles`,
`agent_accounts`, `agent_access`, `agent_workspaces`, `agent_personalities`,
`agent_skills` and `agent_primary`. Its conversations stay, with its id. The
route that removes it also deletes its folder under `agents/` in the data
directory. As with a recording, the tables are named in a variable: a new
table with rows of an agent has to join the list.
