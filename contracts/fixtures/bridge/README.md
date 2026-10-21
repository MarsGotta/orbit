Synthetic bridge-v1 fixtures. Generate them from the types in
`crates/orbit-core/src/wire/bridge.rs` with:

```sh
cargo run -p orbit-core --example bridge_fixtures --locked -- contracts/fixtures/bridge
```

Each capability has one request and two responses: one with a result and one
with the typed REF-403 refusal. `crates/orbit-core/tests/bridge_fixtures.rs`
and `server/tests/test_bridge_fixtures.py` compare the inventory and consume all
frames. Python validates them using the orbit-mcp models in
`server/orbit_mcp/bridge.py`; adding a field in Rust without updating the consumer
fails the guard. The hub only validates the envelope.
