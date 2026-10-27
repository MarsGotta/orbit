# Read-only Markdown vault

The hub owns the encrypted working copy and connector configuration in its
existing encrypted SQLite store. The desktop uses a dedicated typed vault
call through app-core and the daemon. It never opens the server folder or
talks to Fast Note Sync. The vault is separate from meeting notes.

Available connectors are an absolute folder on the hub and Fast Note Sync.
The Mac-folder connector and conflict resolution are deferred. Configuration
is validated by an actual listing before replacement. An omitted token is
retained only for the same server URL and vault. Tokens never appear in a
configuration response. Disconnect removes the configuration and working copy.
Reads refresh the connector catalogue before using the working copy. Unchanged
notes reuse encrypted content only with matching fresh file metadata or sync
versions; changed notes are read again and deleted entries are pruned. An
upstream failure is reported instead of presenting a stale copy as current. No background model
job runs. The selected maintenance model and link style are stored for later.

Folder reads pin directories with descriptor-relative, no-follow opens.
Hidden files, system names, symlinks and non-Markdown files are skipped;
reads reject traversal and non-regular files. YAML aliases, anchors and tags
are refused. The reader preserves original Markdown, including unknown
properties; method validation is advisory, with a 400-word warning.

The index reads properties, internal Markdown/wikilinks, source citations,
task lines, task anchors, dependencies, query blocks, schema folder rules,
root OKF version and log records. Missing targets do not become invented
notes. The supplied design vault, including its missing references, is the
synthetic acceptance fixture. All connector tests use local peers.

Device routes are `GET/PUT/DELETE /v1/vault/config` and `POST /v1/vault/read`.
Read operations are `list`, `search`, `read`, `backlinks`, `outlinks`, `index`,
`catalog`, and `tasks`; pages have `page`, `page_size` and `next_page`.
Assistant reads use `/internal/v1/vault/read` with the existing signed
meetings-bridge credential, and independently require the `notes` group.
MCP exposes `orbit_notes_search`, `orbit_note_read`, `orbit_notes_index`.

Bounds protect distinct resources: 256 KiB per UTF-8 note, 100 entries per
page, 10,000 catalogue entries, 16 MiB scanned content, 16 KiB request
envelopes, a separate 4 MiB parsed/raw response, and a seven-second Fast
Note Sync read budget. These are registered in `contracts/constants.json`.
The word warning is not a legacy-read ceiling. Refusals carry their cause
and measured numbers; note text and tokens never enter reports.

Fast Note Sync was checked against its default branch, `master`:
[REST_API.md](https://github.com/haierkeys/fast-note-sync-service/blob/master/docs/REST_API.md)
shows `Authorization: {token}` in its introduction, but endpoint parameters
and [swagger.yaml](https://github.com/haierkeys/fast-note-sync-service/blob/master/docs/swagger.yaml)
specify `UserAuthToken: {in: header, name: token, type: apiKey}`. The current
middleware accepts the `token` header, which this connector uses. Responses
use `{code, status, message, data}`; lists use
`data: {list, pager: {page, pageSize, totalRows}}`. `GET /api/note` returns
`NoteWithFileLinksResponse` (including `path`, `content`, `fileLinks`), and
link endpoints return `NoteLinkItem[]` (`path`, `context`, `isEmbed`,
`linkText`). Only content/path or counts are used; upstream messages and
link contexts are not relayed.

[OKF 0.2](https://github.com/GoogleCloudPlatform/open-knowledge-format/blob/main/SPEC.md)
defines actor/time records and source provenance. The vault's `SCHEMA.md`
defines its own templates; `docs/vault/AGENTS.md` is a one-page method template.
