# native-v1 fixtures

Generate these JSON files from the Rust transport types:

```sh
cargo run -p orbit-core --example native_fixtures --locked -- contracts/fixtures/native
```

There is one command and one result per command name, plus one file per helper
event and one for `helper_disconnected`, which the daemon synthesizes. Each result
fixture carries the command name because the actual response contains only an ID;
the daemon correlates that ID with the pending command. `native-frame-v1.json`
is retained to check the three compatibility frames already published.

The `events/notice.json` fixture exercises abandonment of “Oye Deimos” with its
closed-set cause and audio durations, without recognized words. Swift compares
its encoding with this Rust-generated JSON.

`calendar/` holds calendar-write variants that do not fit “one command and one
result per name”: `commands/calendar_save-change.json` (an occurrence
change; `commands/calendar_save.json` is creation) and `refusals/<code>.json`,
an `ok:false` response with its `detail` for each refusal. The same command
generates them, and Rust and Swift decode them.

`commands/audio_purpose.json` changes the consumer of a shared microphone without
reopening it. Its response only acknowledges the command; the daemon
does not read its body, just as with `audio_wake_phrase`.
