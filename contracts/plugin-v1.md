# Orbit plugin protocol v1

Status: proposed. The hub reads the manifests (`GET /v1/plugins`, `hub-v1.md`);
nothing else implements this contract yet.

## Manifest

A plugin declares itself in a file named `orbit-plugin.json`, with exactly
these seven fields:

| Field | Shape |
|---|---|
| `id` | text, `[a-z0-9_]{1,32}` |
| `version` | text; it belongs to the plugin, and this contract gives it no format |
| `name` | object with two texts, `es` and `en` |
| `orbit_plugin_api` | the integer `1` |
| `views` | object with `sidebar` (boolean), `settings` (boolean) and `slots` (list of texts); an absent field means no |
| `permissions` | list of permissions, from the vocabulary below |
| `logic` | the text `"none"`, or an object with one field, `command`: the list of texts that starts the plugin's MCP server |

An `id` is never `hub` or `plugin` and does not begin with `hub_` or `plugin_`:
a plugin's own refusal codes would otherwise be mistaken for this contract's.

The vocabulary of permissions is closed:

| Permission | What it allows |
|---|---|
| `data` | keeping the plugin's own data on the hub |
| `notices` | sending notices (see [Notices](#notices)) |
| `events:<event>` | receiving the named event (see [Events](#events)) |
| `read:meetings` | reading Orbit's meetings on the hub, under the assistant's rules |
| `read:notes` | reading Orbit's notes on the hub, under the assistant's rules |
| `assistant_tools` | offering tools to the assistant |
| `network:<domain>` | reaching that domain from the plugin's logic |

`network:<domain>` requires logic: a manifest with `logic` `"none"` does not
carry it.

Permissions are decided once, at install, and the hub enforces them. There are
no expiring reviews, activation switches or per-call confirmations on top.

A plugin never touches audio, the Keychain or the device's credentials, the
privacy stop, system permissions, the helper, the screen, computer control,
provider credentials or the data of other plugins. No permission exists for any
of them.

```json
{"id":"reading_list","version":"0.1.0","name":{"es":"Lecturas","en":"Reading list"},"orbit_plugin_api":1,"views":{"sidebar":true,"settings":false,"slots":[]},"permissions":["data"],"logic":"none"}
{"id":"reading_digest","version":"1.2.0","name":{"es":"Resumen de lecturas","en":"Reading digest"},"orbit_plugin_api":1,"views":{"sidebar":true,"settings":true,"slots":[]},"permissions":["data","notices","events:note.ready","read:notes","network:example.test"],"logic":{"command":["./bin/reading-digest","--stdio"]}}
```

## Call

`POST /v1/plugins/{id}/call`, with the device credential in
`Authorization: Bearer <device token>`.

The request body is a JSON object with exactly two fields: `name`, the text
naming one of the plugin's calls, and `input`, the object with its arguments
(`{}` when it has none).

The answer is `200` with a JSON object of one field, `value`, holding any JSON.

A refusal from the hub is a status of 400 or above with the body in
[Refusals](#refusals). A client looks only at that status and that body.

The hub checks that the plugin is installed, its `orbit_plugin_api` and its
permissions, and then passes the call to the plugin.

Before a call reaches the plugin, the hub refuses:

| Status | Code | When |
|---|---|---|
| 401 or 403 | the hub's own | a device credential it does not accept, as on its other `/v1/` routes (`hub-v1.md`) |
| 404 | `plugin_not_installed` | it has no plugin with that `id`, whatever the `id`'s shape as one path segment: for the hub, a malformed `id` is a plugin it does not have. An empty `id`, or one with `/` (also as `%2F`), is not a path of this route, and the hub answers `404` with no `code` |
| 413 | `plugin_call_too_large` | the body is larger than `PLUGIN_CALL_MAX_BYTES` |
| 400 | `invalid_request` | the body is not an object of exactly `name`, a text, and `input`, an object |
| 404 | `plugin_call_unknown` | the plugin has no call with that `name` |

`logic` `"none"` says the plugin has no process of its own: its calls, if it
has any, are code of the hub, as Notes' are.

A device does not interpret a call. It refuses one under the same conditions as
any other request to the hub (its privacy stop on, not paired, no connection);
beyond that it checks the shape of `id` and the two sizes in
[Limits](#limits), and nothing else.

A call is never retried, whoever says so: neither the device, the hub nor the
app sends it again by itself, whatever a refusal's `retry` says. `retry` tells a
person what to expect; if that person asks again, that is a new call.

```json
{"name":"add_link","input":{"url":"https://example.test/article"}}
{"value":{"link_id":"00000000-0000-4000-8000-000000000903","unread":3}}
```

## The device's local channel

The app asks its device for a call with one order on the device's local
channel, `plugin`, with the plugin's `id` and the call. The device answers it
with one line whose `outcome` is one of three:

- `answered`: the `value` of the hub's `200`;
- `refused`: the hub's status and body, as the hub wrote them (`op` included);
- `failed`: the device's own refusal, the `refusal` object alone
  ([Refusals](#refusals)).

```json
{"v":1,"id":"00000000-0000-4000-8000-000000000905","body":{"command":"plugin","plugin":"vault","call":{"name":"config","input":{}}}}
{"v":1,"id":"00000000-0000-4000-8000-000000000905","outcome":{"outcome":"answered","value":{"configured":false}}}
{"v":1,"id":"00000000-0000-4000-8000-000000000905","outcome":{"outcome":"refused","status":404,"body":{"code":"plugin_not_installed","message":"El plugin no está instalado.","op":"00000000-0000-4000-8000-000000000906","refusal":{"code":"plugin_not_installed","retry":{"kind":"after_fix"}}}}}
{"v":1,"id":"00000000-0000-4000-8000-000000000905","outcome":{"outcome":"failed","refusal":{"code":"not_paired","retry":{"kind":"after_fix"}}}}
```

`call` has exactly `name`, a text, and `input`, an object; the channel refuses
anything else as it refuses any malformed order. The device sends `call` as the
request body, as it writes it, once.

Before sending, the device refuses a call when its privacy stop is on, when it
is not paired, when it has no connection to the hub (also while connecting or
once the hub revoked it), when the `id` is not `[a-z0-9_]{1,32}` and when the
body is larger than `PLUGIN_CALL_MAX_BYTES`. It does not check the reserved
names: for those the hub answers `plugin_not_installed`.

The device reads the hub's whole body, a value or a refusal, up to
`PLUGIN_RESPONSE_MAX_BYTES`. It waits 8 s for the hub, sending and reading
included: longer than `PLUGIN_CALL_BUDGET_MS`, so a late plugin is the hub's
`plugin_timeout`, and shorter than the app's 10 s, so the device's own wait is
the cause the app sees.

One answer line is at most `PLUGIN_RESPONSE_MAX_BYTES` and 64 KiB for its
envelope, so an answer the device admitted reaches the caller whole; a line
that still does not fit is the channel's `too_large` (`ipc_response`), as for
any other line. An order line is at most 1 MiB with its envelope: through the
socket, a call near `PLUGIN_CALL_MAX_BYTES` is refused by the line first
(`too_large`, `ipc_request`), and the device's `plugin_call_too_large` is seen
by a caller inside the device's own process.

## Events

The hub has exactly two events for plugins:

| Event | When | Permission |
|---|---|---|
| `note.ready` | a meeting's note is ready | `events:note.ready` |
| `recording.deleted` | a meeting has been deleted | `events:recording.deleted` |

An event is a JSON object with `event`, its name, and `data`, an object with
one field, `recording_id`, the recording's canonical UUID, the same id as in
`recording.changed` (`hub-v1.md`).

No event carries the text of a note or a meeting. A plugin that needs it asks
for `read:notes` or `read:meetings`.

A plugin without an event's permission does not receive that event.

```json
{"event":"note.ready","data":{"recording_id":"00000000-0000-4000-8000-000000000903"}}
{"event":"recording.deleted","data":{"recording_id":"00000000-0000-4000-8000-000000000904"}}
```

## Notices

A notice needs the `notices` permission. The hub pushes it over the device's
WebSocket, in the shape of the hub's other events, with the event name
`plugin.notice`, `id` null and four fields in `data`:

| Field | Shape |
|---|---|
| `plugin` | the plugin's `id` |
| `title` | text, at most `PLUGIN_NOTICE_TITLE_MAX_CHARS` |
| `text` | text, at most `PLUGIN_NOTICE_TEXT_MAX_CHARS` |
| `view` | `main`, the plugin's main view, or `settings`, its section in Settings: where the notice leads |

Each device shows a notice in its own way. A device that does not know
`plugin.notice` ignores it, as it does any other event it does not know.

```json
{"v":1,"id":null,"event":"plugin.notice","data":{"plugin":"reading_digest","title":"Resumen de lecturas","text":"Hay tres enlaces sin leer.","view":"main"}}
{"v":1,"id":null,"event":"plugin.notice","data":{"plugin":"reading_digest","title":"Resumen de lecturas","text":"Falta la dirección del servicio.","view":"settings"}}
```

## Refusals

A refusal from the hub is a status of 400 or above and a JSON body with:

- `code`, a code from the table below, or one of the hub's refusals on any route
  ([not defined yet](#not-defined-yet));
- `message`, a text for a person, for a client with no text of its own for the
  refusal;
- optionally `message_code`, the key a client looks up its own text by, instead
  of `code`, and `message_fields`, an object of text to text, which holds
  whatever text the refusal needs;
- `refusal`, an object with `code` (equal to the outer `code`), the refusal's
  numbers and `retry`.

Numbers are integers, null only where the table says so, and travel inside
`refusal`. `retry` is an object whose `kind` is one of `never`, `after_fix`,
`check_outcome` or `after_ms`; `after_ms` also carries `ms`.

When the device is the one refusing there is no HTTP status and no body: the
refusal is the `refusal` object alone, with no outer `code` or `message`,
whether or not the device reached the hub. A code both can give has the form of
whoever gave it; the examples of `plugin_call_too_large` and
`plugin_response_too_large` show the hub's. The local channel carries that
object as the `failed` outcome ([The device's local
channel](#the-devices-local-channel)).

A plugin's own refusals are coded `<id>_<cause>`, with the plugin's `id` as
prefix, and the whole code matches `[a-z0-9_]{1,64}`.

| Code | Refused by | When | Numbers |
|---|---|---|---|
| `hub_unreachable` | `device` | The device cannot reach the hub; `since_ms` is null when it is not known | `since_ms` |
| `not_paired` | `device` | The device is not paired with a hub; nothing is sent | none: not being paired is the whole cause |
| `privacy_stopped` | `device` | The user's privacy stop is on; nothing is sent, and `since_ms` is null when it is not known | `since_ms` |
| `invalid_parameter` | `device` | The `id` is not `[a-z0-9_]{1,32}`; nothing is sent. `parameter` is `identifier` and `expected` is `{"kind":"wire_type","name":"plugin_id"}` | none: the shape is the whole cause |
| `timeout` | `device` | The hub did not answer within the device's wait, sending or reading; `operation` is `plugin_call` | `waited_ms`, `deadline_ms` |
| `upstream_failed` | `device` | The connection was cut before the hub's answer began; `service` is `hub` | `status`, null |
| `internal` | `device` | The device could not prepare the request; nothing is sent | `cause` |
| `hub_reply_invalid` | `device` | The device reached the hub and got no valid answer. `field` names what was wrong: `body`, the connection was cut while reading it; `value`, a `200` that is not exactly `{"value": …}`; `refusal`, a status of 400 or above whose body is not a refusal of this contract (`code` and `refusal.code` the same `[a-z0-9_]{1,64}`, and `message` a text); `status`, any other status (redirects are not followed) | `received_bytes`, what had arrived |
| `plugin_not_installed` | `hub` | The hub has no plugin with that `id` | none: the absent plugin is the whole cause |
| `plugin_call_unknown` | `hub` | The plugin has no call with that `name` | none: the absent call is the whole cause, and the refusal does not repeat the `name` |
| `plugin_api_unsupported` | `hub` | The plugin's `orbit_plugin_api`, `wanted`, is outside the range from `min` to `max` that the hub accepts | `wanted`, `min`, `max` |
| `plugin_permission_missing` | `hub` | The plugin asks for something its manifest does not declare | none: the permission is text and travels in `message_fields.permission` |
| `plugin_call_too_large` | `device and hub` | The request body is larger than `PLUGIN_CALL_MAX_BYTES`; `bytes` is what had arrived when that was seen, which can be less than the whole | `bytes`, `max_bytes` |
| `plugin_response_too_large` | `device and hub` | The answer's body is larger than `PLUGIN_RESPONSE_MAX_BYTES`; `bytes` is what had been read when that was seen, which can be less than the whole | `bytes`, `max_bytes` |
| `plugin_timeout` | `hub` | The plugin did not answer within `PLUGIN_CALL_BUDGET_MS` | `elapsed_ms`, `budget_ms` |
| `plugin_stopped` | `hub` | The plugin's process stopped; `exit_code` is null when it ended without one | `exit_code` |
| `plugin_invalid_answer` | `hub` | The plugin answered something outside this contract | none: a malformed answer has no quantity to report |
| `<id>_<cause>` | `hub` | The plugin refused by a rule of its own | none: this contract fixes no numbers here, each plugin names its own |

Which `retry` each refusal of the hub carries is [not defined
yet](#not-defined-yet). The device's are fixed: `after_fix` for `not_paired`,
`privacy_stopped`, `invalid_parameter`, `plugin_call_too_large` and
`plugin_response_too_large`; `check_outcome` for `hub_unreachable`, `timeout`,
`upstream_failed` and `hub_reply_invalid`; `never` for `internal`. The `retry` in the hub's examples
shows the shape only.

```json
{"code":"invalid_parameter","parameter":"identifier","expected":{"kind":"wire_type","name":"plugin_id"},"retry":{"kind":"after_fix"}}
{"code":"timeout","operation":"plugin_call","waited_ms":8001,"deadline_ms":8000,"retry":{"kind":"check_outcome"}}
{"code":"hub_reply_invalid","field":"body","received_bytes":11,"retry":{"kind":"check_outcome"}}
{"code":"plugin_response_too_large","bytes":4194305,"max_bytes":4194304,"retry":{"kind":"after_fix"}}
```

```json
{"code":"hub_unreachable","since_ms":null,"retry":{"kind":"check_outcome"}}
{"code":"plugin_not_installed","message":"El plugin no está instalado.","refusal":{"code":"plugin_not_installed","retry":{"kind":"after_fix"}}}
{"code":"plugin_call_unknown","message":"El plugin no tiene esa llamada.","refusal":{"code":"plugin_call_unknown","retry":{"kind":"never"}}}
{"code":"plugin_api_unsupported","message":"Este hub no admite la versión del plugin.","refusal":{"code":"plugin_api_unsupported","wanted":2,"min":1,"max":1,"retry":{"kind":"after_fix"}}}
{"code":"plugin_permission_missing","message":"El plugin no declara ese permiso.","message_fields":{"permission":"read:notes"},"refusal":{"code":"plugin_permission_missing","retry":{"kind":"after_fix"}}}
{"code":"plugin_call_too_large","message":"La llamada supera el límite.","refusal":{"code":"plugin_call_too_large","bytes":1048600,"max_bytes":1048576,"retry":{"kind":"after_fix"}}}
{"code":"plugin_response_too_large","message":"La respuesta supera el límite.","refusal":{"code":"plugin_response_too_large","bytes":4194400,"max_bytes":4194304,"retry":{"kind":"after_fix"}}}
{"code":"plugin_timeout","message":"El plugin no ha contestado a tiempo.","refusal":{"code":"plugin_timeout","elapsed_ms":7000,"budget_ms":7000,"retry":{"kind":"check_outcome"}}}
{"code":"plugin_stopped","message":"El proceso del plugin ha parado.","refusal":{"code":"plugin_stopped","exit_code":1,"retry":{"kind":"check_outcome"}}}
{"code":"plugin_invalid_answer","message":"El plugin ha contestado algo no válido.","refusal":{"code":"plugin_invalid_answer","retry":{"kind":"never"}}}
```

## Limits

| Bound | Value | Protects | Why this size | Does not limit |
|---|---|---|---|---|
| `PLUGIN_ID_MAX_CHARS` | 32 characters | The call route and every notice, where the `id` travels | The same measure as an agent's `id` (`hub-v1.md`, «Agents»); not measured | A plugin's `name` or the names of its calls |
| `PLUGIN_CALL_MAX_BYTES` | 1048576 bytes | The device's local channel and the hub's memory | A device admits at most 1 MiB for a local order today (`hub-v1.md`, «Local daemon IPC»); the hub admits 8 MiB for a vault revision, but through a device only what fits in that 1 MiB reaches it | The answer, or the data a plugin keeps |
| `PLUGIN_RESPONSE_MAX_BYTES` | 4194304 bytes | The memory that holds a whole answer while it is read | The largest answer of Orbit's vault feature today | The call, or the data a plugin keeps |
| `PLUGIN_CALL_BUDGET_MS` | 7000 ms | The device that waits and the call's place on it | The vault feature's whole read budget today is 7 s, its device waits 8 s and the app 10 s: each layer gives up before the one outside it, so the inner cause is the one seen | The size of a call or an answer, or a plugin's work outside a call |
| `PLUGIN_NOTICE_TITLE_MAX_CHARS` | 80 characters | A frame on the WebSocket and one line on screen | Not measured: the same measure as other names of one line, a note section's and an AI provider's | The notice's `text` |
| `PLUGIN_NOTICE_TEXT_MAX_CHARS` | 500 characters | A frame on the WebSocket and what a device shows of it | Not measured: the same measure as a `chat.item` `label` (`hub-v1.md`) | The notice's `title`, or what the plugin's views show |

## Not defined yet

- Logic in a separate process: the MCP version, the transport, how the process
  receives its credential, how a transport that can cut a stream fits with
  a call being never retried, how the hub holds the process to its
  `network:<domain>` permissions (and whether a domain covers its subdomains),
  and what keeps it from what this contract says a plugin never touches.
- How an event reaches a plugin's logic, and what happens when that logic is
  stopped.
- The routes for a plugin's own data and for its secret values.
- The language of a notice's `title` and `text`, which devices a notice goes
  to, and a notice for a device that is offline or whose privacy stop is on.
- What a plugin's logic receives when its notice is too long, lacks the
  `notices` permission or leads to a view its manifest does not declare.
- The HTTP status of the hub's `plugin_api_unsupported`,
  `plugin_permission_missing`, `plugin_timeout`, `plugin_stopped`,
  `plugin_invalid_answer` and `plugin_response_too_large`, and the `retry` of
  every refusal of the hub.
- The `views.slots` names: Orbit declares no slots yet, and `slots` is an empty
  list until it does.
- Which permission a call itself is checked against: the vocabulary has none
  for a call.
- Whether `notices`, `events:<event>` and `assistant_tools` need logic, as
  `network:<domain>` does.
- How a hub or an app says which `orbit_plugin_api` numbers it accepts.
