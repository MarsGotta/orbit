//! Which turn held the daemon's main loop, and what inside it was slow.
//!
//! One `select!` serves the app's orders, the helper, the hub, the work
//! results, the note answers and the tick of every vertical. On 2026-10-22
//! it wrote nothing for 11.3 s during a meeting, and nothing said which turn
//! held it. Like Zed's `hang_telemetry` (each block tagged
//! `task:<file>:<line>` or `action:<name>`) and Home Assistant's
//! `block_async_io` (the culprit named by its call site), every turn is timed
//! with an `Instant` at its start, one per named step and one per store wait
//! or read, and a slow one is logged as `daemon_turn_slow` with:
//!
//! - `source`: the `select!` arm, with the kind of what it served
//!   (`command:meeting_finish`, `native:audio`, `hub:ready`, `work:upload`,
//!   `note_answer`, `tick`…);
//! - `step`: the slowest named step of the turn — each maintenance step of
//!   the tick, the topic publication — or `handler`, the arm's own code
//!   outside every named step;
//! - `wait`: the slowest wait on the store writer (`store@<file>:<line>`) or
//!   synchronous SQLite read (`read@<file>:<line>`), named by its call site.
//!
//! Codes and numbers only: never an order's, event's or note's content.
//! It measures; it stops, delays and refuses nothing.
use std::{
    fmt,
    panic::Location,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

/// How long a loop turn may take before the daemon logs `daemon_turn_slow`.
///
/// What it protects: a turn that holds the loop long enough to matter is seen
/// in `daemon.log` with its source, its step and its wait, instead of being
/// inferred from missing lines. Why 1 s: it is the loop's tick period, so a
/// longer turn is one that makes the capture's upkeep, a call's health and
/// the topic publication late; at the ~94 helper events/s of two recording
/// tracks it leaves about 94 of the native channel's 1000 slots queued, while
/// the drop that cuts a recording needs ~10.6 s. It is not Zed's 100 ms,
/// which serves a UI thread that must answer every frame, and it cannot be
/// 250 ms: every turn that awaits `StoreActor::snapshot` waits for the
/// writer's 250 ms coalescing interval (`SNAPSHOT_INTERVAL`) by design. On
/// Linux at load 33–35, six runs of the two-track capture lifecycle
/// (`tests/track_signal_lifecycle.rs`) logged turns of 250–258 ms (one
/// awaited snapshot: an audio start's answer, the tick's capture upkeep) and
/// 497–499 ms (a seal's turn, two snapshots) several times per run, which
/// at 250 ms would mean a flushed log line about every second of a meeting.
/// It stops, delays and bounds nothing.
pub(super) const SLOW_TURN_THRESHOLD: Duration = Duration::from_secs(1);

/// Longest code kept for a source's kind. Every order, helper event and hub
/// event name of the protocol fits; a longer one is not a protocol name and
/// is logged as `unrecognized`, so a peer cannot write text through it.
const CODE_BYTES: usize = 40;

/// A short ASCII code naming what a turn served, copied without allocating.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Code {
    bytes: [u8; CODE_BYTES],
    len: u8,
}

impl Code {
    const fn empty() -> Self {
        Self {
            bytes: [0; CODE_BYTES],
            len: 0,
        }
    }

    fn fixed(name: &'static str) -> Self {
        let mut code = Self::empty();
        for byte in name.bytes().take(CODE_BYTES) {
            code.push(byte);
        }
        code
    }

    fn push(&mut self, byte: u8) -> bool {
        let at = usize::from(self.len);
        if at >= CODE_BYTES {
            return false;
        }
        self.bytes[at] = byte;
        self.len += 1;
        true
    }

    /// A protocol name as it came on the wire (`audio`, `chat.delta`): kept
    /// only when it looks like one, else `unrecognized`.
    pub(super) fn wire(name: &str) -> Self {
        let valid = !name.is_empty()
            && name.len() <= CODE_BYTES
            && name.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'.')
            });
        if !valid {
            return Self::fixed("unrecognized");
        }
        let mut code = Self::empty();
        for byte in name.bytes() {
            code.push(byte);
        }
        code
    }

    /// The variant name of a value whose `Debug` is derived, in snake case:
    /// the wire name of a `serde(rename_all = "snake_case")` enum. Formatting
    /// stops at the first character after the name, so no field is ever
    /// formatted.
    pub(super) fn variant(value: &impl fmt::Debug) -> Self {
        let mut name = VariantName(Self::empty());
        // The writer refuses everything after the name: that error is the
        // way it stops, not a failure.
        let _ = fmt::write(&mut name, format_args!("{value:?}"));
        if name.0.len == 0 {
            return Self::fixed("unrecognized");
        }
        name.0
    }

    pub(super) fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or("unrecognized")
    }
}

impl fmt::Debug for Code {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Takes a derived `Debug`'s leading variant name and refuses what follows.
struct VariantName(Code);

impl fmt::Write for VariantName {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for byte in text.bytes() {
            if !byte.is_ascii_alphanumeric() {
                return Err(fmt::Error);
            }
            // serde's snake case: an underscore before every capital but
            // the first, then the lowercase letter.
            if byte.is_ascii_uppercase() && self.0.len > 0 && !self.0.push(b'_') {
                return Err(fmt::Error);
            }
            if !self.0.push(byte.to_ascii_lowercase()) {
                return Err(fmt::Error);
            }
        }
        Ok(())
    }
}

/// The `select!` arm a turn came from, with the kind of what it served.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct TurnSource {
    arm: &'static str,
    kind: Option<Code>,
}

impl TurnSource {
    pub(super) const fn signal() -> Self {
        Self::bare("signal")
    }

    pub(super) const fn note_answer() -> Self {
        Self::bare("note_answer")
    }

    pub(super) const fn topics() -> Self {
        Self::bare("topics")
    }

    pub(super) const fn tick() -> Self {
        Self::bare("tick")
    }

    const fn bare(arm: &'static str) -> Self {
        Self { arm, kind: None }
    }

    /// An order from the app, by its wire name.
    pub(super) fn command(command: &orbit_core::protocol::Command) -> Self {
        Self {
            arm: "command",
            kind: Some(Code::variant(command)),
        }
    }

    /// A helper event, by its `event` name.
    pub(super) fn native(event: &serde_json::Value) -> Self {
        let kind = event
            .get("event")
            .and_then(serde_json::Value::as_str)
            .map_or_else(|| Code::fixed("unrecognized"), Code::wire);
        Self {
            arm: "native",
            kind: Some(kind),
        }
    }

    /// A hub event: the link's own (`connected`…) or a frame's event name.
    pub(super) fn hub(event: &crate::hub::HubEvent) -> Self {
        use crate::hub::HubEvent;
        let kind = match event {
            HubEvent::Connected => Code::fixed("connected"),
            HubEvent::Disconnected => Code::fixed("disconnected"),
            HubEvent::Frame(frame) => frame
                .get("event")
                .and_then(serde_json::Value::as_str)
                .map_or_else(|| Code::fixed("frame"), Code::wire),
            HubEvent::Decoded(frame) => Code::variant(&frame.kind),
            HubEvent::InvalidFrame { .. } => Code::fixed("invalid_frame"),
            HubEvent::Problem(_) => Code::fixed("problem"),
            HubEvent::Revoked => Code::fixed("revoked"),
        };
        Self {
            arm: "hub",
            kind: Some(kind),
        }
    }

    /// A work result, by the kind `WorkResult::kind` names.
    pub(super) fn work(kind: &'static str) -> Self {
        Self {
            arm: "work",
            kind: Some(Code::fixed(kind)),
        }
    }
}

impl fmt::Display for TurnSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            Some(kind) => write!(formatter, "{}:{}", self.arm, kind.as_str()),
            None => formatter.write_str(self.arm),
        }
    }
}

impl fmt::Debug for TurnSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

/// What a timed part of a turn was.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PartName {
    /// A named step: a maintenance step, the publication, the handler.
    Step(&'static str),
    /// A wait on the store writer, by its call site.
    Store(&'static Location<'static>),
    /// A synchronous SQLite read on the loop, by its call site.
    Read(&'static Location<'static>),
}

/// A call site as `<path under src>:<line>`: `daemon/work.rs:338`.
fn site(at: &Location<'_>) -> impl fmt::Display {
    let file = at.file();
    let file = file.rsplit_once("src/").map_or(file, |(_, tail)| tail);
    format!("{file}:{}", at.line())
}

impl fmt::Display for PartName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Step(name) => formatter.write_str(name),
            Self::Store(at) => write!(formatter, "store@{}", site(at)),
            Self::Read(at) => write!(formatter, "read@{}", site(at)),
        }
    }
}

/// One timed part of a turn.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Part {
    name: PartName,
    took: Duration,
}

impl Part {
    pub(super) fn name(&self) -> String {
        self.name.to_string()
    }

    #[cfg(test)]
    pub(super) fn took(&self) -> Duration {
        self.took
    }
}

impl fmt::Debug for Part {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}={:?}", self.name, self.took)
    }
}

/// A finished turn: where it came from, how long it took and what in it
/// was slowest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SlowTurn {
    pub(super) source: TurnSource,
    pub(super) took: Duration,
    /// The slowest named step, or the handler when the arm's own code took
    /// longer than every named step.
    pub(super) step: Part,
    /// The slowest store wait or SQLite read, if the turn had one.
    pub(super) wait: Option<Part>,
}

impl SlowTurn {
    /// The `daemon_turn_slow` fields: codes and numbers only.
    pub(super) fn fields(&self, recording: bool) -> Vec<(&'static str, String)> {
        let mut fields = vec![
            ("source", self.source.to_string()),
            ("step", self.step.name()),
            ("took_ms", millis(self.took)),
            ("step_ms", millis(self.step.took)),
        ];
        if let Some(wait) = &self.wait {
            fields.push(("wait", wait.name()));
            fields.push(("wait_ms", millis(wait.took)));
        }
        fields.push(("recording", recording.to_string()));
        fields
    }

    /// The fields `daemon_loop_stalled` adds about the slowest turn of the
    /// period: the same codes, prefixed.
    pub(super) fn stall_fields(&self) -> Vec<(&'static str, String)> {
        let mut fields = vec![
            ("slowest_source", self.source.to_string()),
            ("slowest_step", self.step.name()),
            ("slowest_ms", millis(self.took)),
        ];
        if let Some(wait) = &self.wait {
            fields.push(("slowest_wait", wait.name()));
            fields.push(("slowest_wait_ms", millis(wait.took)));
        }
        fields
    }
}

fn millis(duration: Duration) -> String {
    duration.as_millis().to_string()
}

/// The turn in progress and the slowest turn since the last tick.
#[derive(Default)]
struct Current {
    turn: Option<(Instant, TurnSource)>,
    /// The named steps' total: what is left of the turn is the handler's.
    steps: Duration,
    step: Option<Part>,
    wait: Option<Part>,
    slowest_since_tick: Option<SlowTurn>,
}

fn slower(current: Option<Part>, candidate: Part) -> Option<Part> {
    match current {
        Some(part) if part.took >= candidate.took => Some(part),
        _ => Some(candidate),
    }
}

/// Times the loop's turns. Cloned into the store writer's handle and the
/// loop's reader, which report their waits into the same turn. The mutex is
/// only ever taken by the loop's own thread, a handful of times per turn.
#[derive(Clone, Default)]
pub(super) struct TurnWatch {
    current: Arc<Mutex<Current>>,
}

impl TurnWatch {
    fn lock(&self) -> MutexGuard<'_, Current> {
        // A panic while holding it can only have left a half-recorded turn,
        // which is still worth reading.
        self.current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A turn of `source` starts now; whatever an earlier one left is dropped.
    pub(super) fn begin(&self, source: TurnSource) {
        let mut current = self.lock();
        current.turn = Some((Instant::now(), source));
        current.steps = Duration::ZERO;
        current.step = None;
        current.wait = None;
    }

    /// A named step of the turn took `took`. Steps never nest: their total is
    /// what the handler did not take.
    pub(super) fn step(&self, name: &'static str, took: Duration) {
        let mut current = self.lock();
        current.steps = current.steps.saturating_add(took);
        current.step = slower(
            current.step,
            Part {
                name: PartName::Step(name),
                took,
            },
        );
    }

    /// Runs `future` as the named step `name`.
    pub(super) async fn time<F: std::future::Future>(
        &self,
        name: &'static str,
        future: F,
    ) -> F::Output {
        let started = Instant::now();
        let output = future.await;
        self.step(name, started.elapsed());
        output
    }

    /// Runs `step` as the named step `name`.
    pub(super) fn time_sync<T>(&self, name: &'static str, step: impl FnOnce() -> T) -> T {
        let started = Instant::now();
        let output = step();
        self.step(name, started.elapsed());
        output
    }

    fn waited(&self, name: PartName, took: Duration) {
        let mut current = self.lock();
        current.wait = slower(current.wait, Part { name, took });
    }

    /// The loop waited `took` for the store writer at `at`.
    pub(super) fn store_waited(&self, at: &'static Location<'static>, took: Duration) {
        self.waited(PartName::Store(at), took);
    }

    /// A synchronous SQLite read at `at` held the loop for `took`.
    pub(super) fn read_waited(&self, at: &'static Location<'static>, took: Duration) {
        self.waited(PartName::Read(at), took);
    }

    /// The turn is over. Kept as the slowest of the period when it is, and
    /// returned when it took longer than [`SLOW_TURN_THRESHOLD`].
    pub(super) fn end(&self) -> Option<SlowTurn> {
        self.end_with_threshold(SLOW_TURN_THRESHOLD)
    }

    fn end_with_threshold(&self, threshold: Duration) -> Option<SlowTurn> {
        let mut current = self.lock();
        let (started, source) = current.turn.take()?;
        let took = started.elapsed();
        let handler = Part {
            name: PartName::Step("handler"),
            took: took.saturating_sub(current.steps),
        };
        let step = match current.step {
            Some(step) if step.took >= handler.took => step,
            _ => handler,
        };
        let finished = SlowTurn {
            source,
            took,
            step,
            wait: current.wait,
        };
        if current
            .slowest_since_tick
            .is_none_or(|slowest| slowest.took < took)
        {
            current.slowest_since_tick = Some(finished);
        }
        (took > threshold).then_some(finished)
    }

    /// The slowest turn since the last call: what held a late tick.
    pub(super) fn take_slowest_since_tick(&self) -> Option<SlowTurn> {
        self.lock().slowest_since_tick.take()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use orbit_core::protocol::Command;
    use serde_json::json;

    fn held(watch: &TurnWatch, source: TurnSource, hold: Duration) -> Option<SlowTurn> {
        watch.begin(source);
        std::thread::sleep(hold);
        watch.end_with_threshold(Duration::from_millis(5))
    }

    #[test]
    fn a_quick_turn_logs_nothing_but_is_still_the_slowest_of_its_period() {
        let watch = TurnWatch::default();
        watch.begin(TurnSource::tick());
        assert_eq!(watch.end(), None);
        let slowest = watch.take_slowest_since_tick().expect("one turn ran");
        assert_eq!(slowest.source.to_string(), "tick");
        assert_eq!(watch.take_slowest_since_tick(), None);
    }

    #[test]
    fn a_slow_turn_names_its_source_and_the_handler_when_no_step_was_named() {
        let watch = TurnWatch::default();
        let slow = held(
            &watch,
            TurnSource::native(&json!({"event": "audio", "pcm_s16le_base64": "AAAA"})),
            Duration::from_millis(20),
        )
        .expect("a 20 ms turn passes a 5 ms threshold");
        assert_eq!(slow.source.to_string(), "native:audio");
        assert_eq!(slow.step.name(), "handler");
        assert!(slow.took >= Duration::from_millis(20));
        assert!(slow.step.took() >= Duration::from_millis(20));
        assert_eq!(slow.wait, None);
    }

    #[test]
    fn the_slowest_named_step_wins_over_the_rest_of_the_handler() {
        let watch = TurnWatch::default();
        watch.begin(TurnSource::tick());
        watch.time_sync("maintain_verticals.calendar", || {
            std::thread::sleep(Duration::from_millis(2));
        });
        watch.time_sync("maintain_capture", || {
            std::thread::sleep(Duration::from_millis(30));
        });
        let slow = watch
            .end_with_threshold(Duration::from_millis(5))
            .expect("a 32 ms turn passes a 5 ms threshold");
        assert_eq!(slow.step.name(), "maintain_capture");
        assert!(slow.step.took() >= Duration::from_millis(30));
        let fields = slow.fields(true);
        let names: Vec<_> = fields.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, ["source", "step", "took_ms", "step_ms", "recording"]);
        assert_eq!(fields[0].1, "tick");
        assert_eq!(fields[4].1, "true");
    }

    #[tokio::test]
    async fn an_async_step_is_timed_across_its_awaits() {
        let watch = TurnWatch::default();
        watch.begin(TurnSource::tick());
        watch
            .time(
                "audio_timeout",
                tokio::time::sleep(Duration::from_millis(20)),
            )
            .await;
        let slow = watch
            .end_with_threshold(Duration::from_millis(5))
            .expect("slow");
        assert_eq!(slow.step.name(), "audio_timeout");
    }

    #[test]
    fn a_wait_is_named_by_its_call_site() {
        let watch = TurnWatch::default();
        watch.begin(TurnSource::work("upload"));
        let at = Location::caller();
        watch.read_waited(at, Duration::from_millis(40));
        watch.store_waited(at, Duration::from_millis(10));
        let slow = held(&watch, TurnSource::work("upload"), Duration::ZERO);
        // `held` began a new turn: the waits of the first one are gone.
        assert_eq!(slow.and_then(|slow| slow.wait), None);
        watch.begin(TurnSource::work("upload"));
        watch.read_waited(at, Duration::from_millis(40));
        watch.store_waited(at, Duration::from_millis(10));
        std::thread::sleep(Duration::from_millis(6));
        let slow = watch
            .end_with_threshold(Duration::from_millis(5))
            .expect("slow");
        let wait = slow.wait.expect("the read was recorded");
        assert_eq!(
            wait.name(),
            format!("read@daemon/turn_watch.rs:{}", at.line())
        );
        assert_eq!(wait.took(), Duration::from_millis(40));
        let fields = slow.fields(false);
        assert!(fields.contains(&("wait", wait.name())));
        assert!(fields.contains(&("wait_ms", "40".to_owned())));
    }

    #[test]
    fn the_slowest_turn_of_the_period_is_kept_until_the_tick_takes_it() {
        let watch = TurnWatch::default();
        held(&watch, TurnSource::topics(), Duration::from_millis(1));
        held(
            &watch,
            TurnSource::command(&Command::MeetingFinish),
            Duration::from_millis(25),
        );
        held(&watch, TurnSource::note_answer(), Duration::from_millis(1));
        let slowest = watch.take_slowest_since_tick().expect("three turns ran");
        assert_eq!(slowest.source.to_string(), "command:meeting_finish");
        let fields = slowest.stall_fields();
        assert_eq!(
            fields[0],
            ("slowest_source", "command:meeting_finish".into())
        );
        assert_eq!(fields[1], ("slowest_step", "handler".into()));
        assert_eq!(watch.take_slowest_since_tick(), None);
    }

    #[test]
    fn a_turn_never_begun_is_not_reported() {
        let watch = TurnWatch::default();
        watch.step("maintain_state", Duration::from_secs(3));
        assert_eq!(watch.end(), None);
        assert_eq!(watch.take_slowest_since_tick(), None);
    }

    #[test]
    fn an_order_is_named_by_its_wire_name_and_never_by_its_content() {
        let chat = Command::Chat {
            attachments: Vec::new(),
            primary: false,
            text: "SECRET_FIXTURE_CONVERSATION".into(),
        };
        for command in [
            chat,
            Command::Status,
            Command::MeetingFinish,
            Command::PrivacyStop,
        ] {
            let wire = serde_json::to_value(&command).expect("serializes");
            let source = TurnSource::command(&command).to_string();
            assert_eq!(
                source,
                format!("command:{}", wire["command"].as_str().expect("tagged")),
            );
            assert!(!source.contains("SECRET"));
        }
    }

    #[test]
    fn a_peer_cannot_write_text_through_an_event_name() {
        for name in [
            "Hola, esto es una frase",
            "",
            "audio\nforged",
            &"a".repeat(CODE_BYTES + 1),
        ] {
            assert_eq!(
                TurnSource::native(&json!({ "event": name })).to_string(),
                "native:unrecognized"
            );
        }
        assert_eq!(
            TurnSource::native(&json!({ "frames": 1 })).to_string(),
            "native:unrecognized"
        );
        assert_eq!(
            TurnSource::native(&json!({ "event": "calendar_invalidated" })).to_string(),
            "native:calendar_invalidated"
        );
        assert_eq!(
            TurnSource::hub(&crate::hub::HubEvent::Frame(
                json!({ "event": "chat.delta" })
            ))
            .to_string(),
            "hub:chat.delta"
        );
        assert_eq!(
            TurnSource::hub(&crate::hub::HubEvent::Problem("texto libre".into())).to_string(),
            "hub:problem"
        );
    }

    #[test]
    fn a_variant_name_is_cut_before_any_field() {
        #[derive(Debug)]
        #[allow(dead_code, reason = "only its Debug is read")]
        enum Sample {
            UploadSettled { text: String },
            Tuple(String),
            Unit,
        }
        let secret = "SECRET_FIXTURE".to_owned();
        assert_eq!(
            Code::variant(&Sample::UploadSettled {
                text: secret.clone()
            })
            .as_str(),
            "upload_settled"
        );
        assert_eq!(Code::variant(&Sample::Tuple(secret)).as_str(), "tuple");
        assert_eq!(Code::variant(&Sample::Unit).as_str(), "unit");
        assert_eq!(Code::variant(&"quoted").as_str(), "unrecognized");
    }
}
