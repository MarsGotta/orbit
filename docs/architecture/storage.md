# Encrypted local storage

The Mac keeps no copy of hub conversations, meetings or notes. It stores what it
produces and still needs to deliver, plus local voice-note audio. See the
[overview](README.md) for the hub's ownership.

`orbit-store` is an effects adapter. It receives a 32-byte key from the daemon;
it does not open Keychain, read clocks, decide permissions or use the network.
Only the daemon opens it: the storage actor has the sole write connection and
the loop a read-only connection (`daemon/startup.rs`, `daemon/store_actor.rs`).
The UI queries the daemon and never opens the database.

## Files

The data root depends on the app identity; see `orbit-ipc/src/paths.rs` and
`orbit-core/src/orbit_bundle.rs`. Acceptance and normal builds use separate roots.

```text
config.toml               settings, never tokens
orbit.sock                development/acceptance socket, mode 0600
daemon.lock               instance lock
store/
  orbit.sqlite3           SQLite database, plus WAL/SHM
  chunks/                 encrypted file per audio chunk
cua/                      engine socket and temporary desktop screenshot
computer-control.json     control history, at most 256 rows
logs/                     daemon.log, report-daemon.log, report-app.log, updates.log (1 MiB each)
```

Keychain holds the storage key and device token in accounts `storage` and `device`.
The service is identity-specific (`crates/orbit-daemon/src/config.rs`).
Explicit `--dev` mode uses `development-<account>.key` files in the data root;
non-macOS development requires that mode. There is no production fallback.

## Database contents

| Data | Location | Writer/reader | Retention |
|---|---|---|---|
| `DurableState`: privacy latch/requester, generation, connection, selected session ID, current turn, recording/call, pending approval and notices | `settings`, `snapshot` | Actor writes/coalesces; startup reads | Replaced on change |
| Durable queue and receipts | `jobs` | Daemon | Rows remain; terminal states do not revert |
| Audio | `chunks` receipts and `store/chunks/` | Actor seals/prunes; uploader and local playback read | Meeting chunks removed after authenticated durable hub ACK; unconfirmed audio retained; voice-note audio retained until the note is discarded or nothing was heard in it |
| Temporary capture receipts without audio | `records`, `uploaded_chunks:<recording>` | Actor writes on audio release; finalization/recovery read | Until hub confirms closure; not added after closure |
| Private notes awaiting hub | `records`, `recording_notes`, `note-private-<recording>` | Actor writes, hub upload reads | Until the hub confirms that text |
| Note and library orders awaiting hub (edits, ticked tasks, titles, speaker names, languages, rewrites, templates, series, dictionary orders) | `note_outbox`, one row per lane (`meeting:<recording>` or `library`) and target | Actor writes; the daemon keeps a copy to schedule | Until the hub answers; a newer order on the same target replaces the row |
| Call-detection preference | `settings`, `call_preference_v1` | Daemon | Until changed |
| Key check | `settings`, `key_check` | Creation/open | Permanent |

The current turn is the only conversation content in `DurableState.turn`.
Startup reports `AssistantTurnLostByRestart` if a restart interrupted it.
Conversation pages come from `GET /v1/sessions/{id}/messages`.

### Jobs

`orbit_store::QUEUE_KINDS` declares six kinds:

| Kind | Purpose | Producer |
|---|---|---|
| `chunk_upload` | `PUT /v1/chunks/{id}` | `seal_chunk` in the receipt transaction |
| `recording_finalize` | Hub closure | `daemon/finalize.rs` |
| `recording_manifest` | Meeting, `voice_call` or `voice_turn` manifest | Commands/audio/call adapters |
| `capture_epoch` | Recording capture epoch | Audio/call adapters |
| `audio_processing_receipt` | Native VoiceProcessing ACK | Call adapter |
| `call_recovery_boundary` | Call interrupted by restart | `daemon/call_recovery.rs` |

Only upload/finalize are pending work; the other four are written `succeeded`
and read by identity. Pending states are `queued`, `running`, `retry_wait` and
`unknown`; terminal states are `succeeded`, `blocked` and `cancelled`.
`unknown` requires reconciliation, never repetition of the effect.
`upload_allowed:false` (voice notes) cancels upload instead of sending audio.

### Ephemeral sources and migrations

Window captures, selected text and inventories stay in memory. Core validates
bytes; the daemon builds a receipt and delivers text or uploads the PNG.
Neither content, inventory, progress nor receipt enters the durable snapshot.
A late reply, privacy stop, changed session or generation does not repeat a read.
Legacy `max_source_intents`/`max_source_bytes` configuration is accepted but no
longer used and disappears on save.

Every open adds the `note_outbox` table when it is missing, without raising
the schema version: an older release still opens the store and leaves the
table alone. The obsolete `messages` table is removed on open. Hub meeting/transcript/note
copies are retired; pending private notes remain. Schema 3→4 removes ten unused
vertical namespaces and their known keys/triggers, `action_journal`,
`calendar_copies`, `source_intents` and `sources`. Unknown keys/namespaces are
preserved. Audio/private notes are unchanged. `store/sources/` is deleted after
commit; every open retries interrupted cleanup, rejects a symlink there and
does not recreate the directory.

## Encryption

RustCrypto XChaCha20-Poly1305 uses a random 192-bit nonce and associated data
including record kind and ID. The versioned envelope contains magic, nonce and
ciphertext. Orbit uses the library's cryptography. The key is not in the store;
losing it makes the data unreadable.

SQLite leaves opaque IDs, kinds, roles, sequence/timing/count/state indexes
plaintext. Titles, text, recipients and payloads are encrypted. Indexed state is
checked against authenticated state before returning a job. Logs do not serialize
messages, snapshots or payloads. Production uses `keyring` with `apple-native`,
not a dummy credential backend.

## Durability and recovery

- WAL, `synchronous=FULL`, `foreign_keys=ON`, `fullfsync=ON`,
  `checkpoint_fullfsync=ON` and `trusted_schema=OFF` are configured.
  `Store::open` verifies effective values for the first five and rejects SQLite
  older than 3.51.3.
- A chunk is written to an exclusive temporary file, synced, published without
  replacing an existing file, and its directory synced. Receipt and
  `upload-<chunk_id>` job commit together. A pre-commit crash may leave a valid
  orphan whose header reconstructs both records with the same identity.
  Conflicts do not delete either file.
- `begin_recovery`/`recover_next` traverse chunks in batches to the end,
  reconcile orphans, detect missing originals and retain invalid ones.
  `recover_upload_jobs` makes only immutable interrupted uploads retryable;
  the hub deduplicates by ID and bytes.
- The daemon validates remote identity, hash and `durable`, then records
  `Succeeded` before releasing meeting audio. Unlink/directory sync precede
  the transaction removing `chunks` and keeping closure/recovery facts.
  A crash after unlink may leave a confirmed receipt without a file; recovery
  authenticates that upload receipt and does not count it as lost or upload
  again. Without confirmation, a missing file still counts as lost.
  Cleanup failures are logged/retried without changing upload success.
  Old confirmed chunks are cleaned one at a time when the worker has no
  eligible transfers, including while disconnected.
- Directories are 0700, database/temporaries 0600. Direct destination symlinks
  are refused. This does not protect against root or a same-user process
  manipulating parent directories.
- Linux tests do not establish power-loss safety on physical APFS. Do not copy
  only an open `.sqlite3` file: its WAL is part of the state.

## Schema and bounds

`PRAGMA user_version` is 4. Open authenticates the key before migrating schema 3;
SQL retirement and version change commit in one transaction. Reopening clean
schema 4 does not write. Earlier-than-3 and future versions are rejected with
store name, version and supported range. `open_existing`/`open_reader` require
the current schema and never migrate.

`crates/orbit-store/src/lib.rs` bounds JSON payloads to 1 MiB, chunks to 8 MiB,
and job pages to 500 rows/8 MiB.

## Storage actor

`daemon/store_actor.rs`:

- The loop admits writes through a bounded 8192-operation channel
  (`STORE_OP_CAPACITY`). The actor's sole write connection seals PCM, saves
  snapshots, mutates jobs/receipts (`StoreOp::Run`) and returns typed results.
- Admission order is preserved. No mutation overtakes a previously admitted
  seal, retry or snapshot. Failed sealing is reported before the next operation
  and parked for retry.
- Admitted PCM counts against a 64 MiB reserve (`daemon/sealing.rs`) until its
  `Sealed` receipt; only then does `sealed_chunks` increase. Exhaustion reports
  loss immediately.
- Snapshots coalesce at a minimum 250 ms interval. The fingerprint compares
  `recording`, `privacy_stopped`, `privacy_stop_by`, `generation`, `connection`,
  `session_id`, `pending_approval` and `call`, never ciphertext. Unchanged
  snapshots do not create new ciphertext.
- Loop-side reads are bounded: identities, job pages, receipts, authenticated
  chunks, closure counts and pending private notes. They never sync or mutate;
  resulting decisions return through the actor.
