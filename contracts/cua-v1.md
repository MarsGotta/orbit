# CUA engine v1

Status: Rust implementation linked into the daemon, checked with fixtures.
This is not native acceptance against a real engine on a Mac.

The daemon starts bundled `cua-driver` when macOS grants Accessibility and Screen
Recording ([policy](control-policy-v1.md)). The assistant reads its declared
catalogue with `control_tools` and executes a declared tool with `perform_action`.
Orbit neither filters the catalogue nor reinterprets its vocabulary; it checks
tool membership and frame bounds.

## Transport and limits

`cua-driver` **0.28.2**, MCP **2025-06-18**, newline-delimited JSON-RPC 2.0.

`LocalCua::connect(CuaConfig { driver_path, socket_path, request_timeout })`
requires the bundled executable and a private socket directory under Orbit's
root. Paths are absolute without traversal. Socket and parent belong to the
current user with no group/other permissions. Authority is the private local
socket, not a credential.

Orbit starts the engine (`serve`) and its proxy:

```text
cua-driver mcp --embedded --socket /private/path/driver.sock
```

`--embedded` prevents the proxy from launching another engine app instance.
stderr is not retained; engine and proxy terminate with the connection.

A host already holding an authorized transport may use
`CuaClient::from_transport(reader, writer, timeout)` then `initialize()`.

Requests are serialized, bounded to 1 MiB per frame and a configurable nonzero
timeout of at most 30 seconds. Up to 64 progress/log notifications may arrive
while awaiting a response. EOF, oversize, RPC error, wrong response ID, server
request, changed catalogue or timeout invalidate the connection. No automatic
retry/reconnect. The unpaged catalogue permits at most `MAX_ENGINE_TOOLS`
entries and `MAX_ENGINE_CATALOGUE_BYTES` (`orbit_core::computer_control`).

## API

| Orbit method | MCP request |
|---|---|
| `initialize()` | `initialize` → `notifications/initialized` → `tools/list`; validate versions/catalogue and required `list_apps`/`list_windows` |
| `describe()` / `engine_tools()` | Return the declared catalogue unchanged, without execution |
| `apps()` | `tools/call`: `list_apps`, `{}` |
| `windows(pid)` | `list_windows`, `{on_screen_only:false,pid?:integer}` |
| `call_tool(name, arguments)` | Call a declared tool with object arguments |

Responses become `NativeObservation {structured,text,images}` without imposing
one cross-platform schema. A PNG is retained within remote-capture bounds;
unrequested resources are rejected. `isError:true` never becomes a successful
observation.

Every action leaves a `computer-control.json` history row (tool, application,
time, outcome). A lost response leaves `unknown` and is never retried.

## Evidence and sources

`cargo test --locked -p orbit-daemon --test cua_contract` uses synthetic fixtures
to cover catalogue, unsupported version, tool errors, EOF/protocol/timeout/size,
no retry and unsafe paths. Run with a timeout. Fixture success does not establish
live engine behavior on macOS.

- [General MCP tool schemas](https://github.com/trycua/cua/blob/main/docs/content/docs/reference/cua-driver/mcp-tools.mdx)
- [Handshake and serialization](https://github.com/trycua/cua/blob/e88e9d899ac5effaeae38619527ebaa46b26ce72/libs/cua-driver/rust/crates/cua-driver-core/src/protocol.rs)
- [Embedded proxy](https://github.com/trycua/cua/blob/e88e9d899ac5effaeae38619527ebaa46b26ce72/libs/cua-driver/rust/crates/cua-driver/src/cli.rs)
- [Host integration](https://github.com/trycua/cua/blob/e88e9d899ac5effaeae38619527ebaa46b26ce72/libs/cua-driver/rust/Skills/cua-driver/EMBEDDING.md)
