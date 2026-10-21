//! Error reports to Sentry, only when the person allowed them.
//!
//! One client for the whole Mac, here in the daemon: what the daemon reports
//! to its local log (`Runtime::reported`) and what the interface reports to
//! the app (the `orbit_report` order, forwarded as `Command::Report`) reach
//! [`ErrorReports::report`], and a panic of the daemon the hook
//! [`ErrorReports::report_panics`] installs. So one place decides whether
//! anything leaves: the switch «Enviar informes de errores anónimos», off by
//! default. Only failures leave ([`Report::is_failure`]); the rest of the
//! trail stays in the local log, so Sentry shows what broke and its quota is
//! not spent on what went well.
//!
//! Orbit handles meeting audio, transcripts and conversations, and nothing
//! of them may leave. The client collects as little as it can (no default
//! integrations, no breadcrumbs, no sessions, no tracing, no environment
//! read), and [`scrub`] then rebuilds every event from an allow-list, the
//! same criterion as the hub's (`server/orbit_hub/error_reports.py`): the
//! code as the message; the tags `op`, `code`, `class` and `source`; numbers
//! and booleans as data, and tokens only under a key the notice catalogue
//! marks as a closed enumeration ([`NoticeKey::closed`]); the release; the
//! name and version of macOS; and file, function, line and address of each
//! frame of a panic, with the loaded images' identifiers and addresses (no
//! paths) so Sentry names them from the release's dSYM. Everything else is dropped: the machine's name, the panic's text,
//! any other token, the SDK's own description.
//!
//! A build without `ORBIT_SENTRY_DSN` at compile time never creates a client:
//! the switch is not offered and nothing can leave.
use orbit_core::notice::{NoticeKey, Report, Token};
use sentry::protocol::{
    Context, DebugImage, DebugMeta, Event, Exception, Frame, Level, OsContext, Stacktrace,
    SymbolicDebugImage, Value,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

#[cfg(test)]
pub(crate) mod fake_sentry;

/// Where official builds send their reports: the Sentry project's address,
/// given to the compiler by `scripts/release` from a file outside the
/// repository. A build without it has no switch and no client.
pub const DSN: Option<&str> = option_env!("ORBIT_SENTRY_DSN");

/// How long one code with the same data is sent only once. It protects
/// Sentry's quota and the person's network from a failure that repeats every
/// second (a call-detection timer that fails each tick would send 3 600
/// events an hour); one minute keeps a repeated failure visible every
/// minute, with its count, while costing at most 60 events an hour per
/// failure. It is not a limit on the local log, which keeps every line.
pub const REPEAT_WINDOW: Duration = Duration::from_secs(60);

/// How long a process that is going away (a panic, or the daemon closing)
/// waits for its last reports to leave: Sentry's own default for closing.
/// Bounds the delay this adds to an exit, and only when something is still
/// queued; it is not a delivery guarantee.
pub const EXIT_FLUSH: Duration = Duration::from_secs(2);

/// The tags that may leave.
const TAGS: [&str; 4] = ["op", "code", "class", "source"];

/// The daemon's error-report client, or nothing when this build has no
/// Sentry address. Cheap to clone: every clone is the same client and the
/// same switch.
#[derive(Clone, Default)]
pub struct ErrorReports {
    inner: Option<Arc<Inner>>,
}

struct Inner {
    client: Arc<sentry::Client>,
    enabled: AtomicBool,
    repeats: Mutex<Repeats>,
}

impl ErrorReports {
    /// The client for `dsn`, switched on or off as the user chose. `None`, or an
    /// address that is not a Sentry DSN, gives no client at all.
    pub fn start(dsn: Option<&str>, enabled: bool) -> Self {
        let Some(dsn) = dsn.and_then(|raw| raw.parse::<sentry::types::Dsn>().ok()) else {
            return Self::default();
        };
        // The transport builds a rustls client with whichever provider the
        // process installed. The daemon already uses ring for the hub; a
        // provider installed before is kept.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let contexts = sentry::integrations::contexts::ContextIntegration::new()
            .add_rust(false)
            .add_device(false);
        // `ClientOptions` is non-exhaustive: every field not set here keeps
        // its default, and `sentry::init`'s defaults (environment variables,
        // default integrations) are never applied.
        let mut options = sentry::ClientOptions::default();
        options.dsn = Some(dsn);
        options.release = Some(format!("orbit@{}", env!("CARGO_PKG_VERSION")).into());
        options.send_default_pii = false;
        options.max_breadcrumbs = 0;
        options.attach_stacktrace = false;
        options.auto_session_tracking = false;
        options.default_integrations = false;
        // A panic carries the loaded images (identifier and load address),
        // so Sentry can name the frames of a stripped release binary from
        // the dSYM uploaded with that release; the scrub keeps nothing else
        // of them. Only panics: a reported code has no frames.
        let images = sentry::integrations::debug_images::DebugImagesIntegration::new()
            .filter(|event| !event.exception.is_empty());
        options.integrations = vec![Arc::new(contexts), Arc::new(images)];
        options.transport = Some(Arc::new(sentry::transports::DefaultTransportFactory));
        options.before_send = Some(Arc::new(scrub));
        Self {
            inner: Some(Arc::new(Inner {
                client: Arc::new(sentry::Client::with_options(options)),
                enabled: AtomicBool::new(enabled),
                repeats: Mutex::new(Repeats::default()),
            })),
        }
    }

    /// The user's choice, for the switch: `None` when this build cannot send, so
    /// the switch is not offered.
    pub fn choice(&self) -> Option<bool> {
        self.inner
            .as_ref()
            .map(|inner| inner.enabled.load(Ordering::SeqCst))
    }

    /// Turns sending on or off from now on. Off also forgets the repetitions
    /// counted so far: nothing said while the user allowed it leaves after
    /// they stopped allowing it. What already left cannot be withdrawn from here.
    pub fn set_enabled(&self, enabled: bool) {
        if let Some(inner) = &self.inner {
            inner.enabled.store(enabled, Ordering::SeqCst);
            if !enabled {
                inner.repeats().forget();
            }
        }
    }

    /// Sends one report line when it is a failure, unless the user has not
    /// allowed it or the same code with the same data was sent less than
    /// [`REPEAT_WINDOW`] ago; then it is only counted, and the count leaves
    /// with the next send of it.
    pub fn report(&self, report: &Report, now: Instant) {
        if !report.is_failure() {
            return;
        }
        let Some(inner) = self.allowed() else { return };
        let admitted = inner.repeats().admit(report, now);
        if let Some(repeated) = admitted {
            inner.client.capture_event(event(report, repeated), None);
        }
    }

    /// Sends the counts of repetitions whose minute ended, so a failure that
    /// stopped repeating still says how many times it happened. The daemon
    /// calls it once a second.
    pub fn send_due(&self, now: Instant) {
        let Some(inner) = self.allowed() else { return };
        let due = inner.repeats().due(now);
        for (report, repeated) in due {
            inner.client.capture_event(event(&report, repeated), None);
        }
    }

    /// Waits up to `timeout` for what was captured to leave. True when the
    /// queue emptied, or when there is no client.
    pub fn flush(&self, timeout: Duration) -> bool {
        self.inner
            .as_ref()
            .is_none_or(|inner| inner.client.flush(Some(timeout)))
    }

    /// Reports a panic of this process before it goes on: the Sentry SDK's
    /// panic event (its frames), sent and flushed for up to [`EXIT_FLUSH`]
    /// while the user allows it. Installed once per process, on top of the hook
    /// already there, which still runs.
    pub fn report_panics(&self) {
        static INSTALLED: std::sync::Once = std::sync::Once::new();
        let Some(inner) = self.inner.clone() else {
            return;
        };
        INSTALLED.call_once(move || {
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                if inner.enabled.load(Ordering::SeqCst) {
                    let mut event = sentry::integrations::panic::PanicIntegration::new()
                        .event_from_panic_info(info);
                    event.message = Some("panic".into());
                    event.tags.insert("code".into(), "panic".into());
                    event.tags.insert("class".into(), "failed".into());
                    event.tags.insert("source".into(), "daemon".into());
                    inner.client.capture_event(event, None);
                    inner.client.flush(Some(EXIT_FLUSH));
                }
                previous(info);
            }));
        });
    }

    fn allowed(&self) -> Option<&Arc<Inner>> {
        self.inner
            .as_ref()
            .filter(|inner| inner.enabled.load(Ordering::SeqCst))
    }
}

impl Inner {
    fn repeats(&self) -> std::sync::MutexGuard<'_, Repeats> {
        self.repeats.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The event a report line becomes, with everything it carries; [`scrub`]
/// decides what of it leaves.
fn event(report: &Report, repeated: u64) -> Event<'static> {
    let code = report.code.as_str().to_owned();
    // Only failures get here, silent ones included: all are errors.
    let mut event = Event {
        level: Level::Error,
        message: Some(code.clone()),
        ..Event::default()
    };
    for (key, value) in [
        ("op", Value::from(report.op.as_str())),
        ("code", Value::from(code)),
        (
            "class",
            serde_json::to_value(report.class).unwrap_or_default(),
        ),
        (
            "source",
            serde_json::to_value(report.source).unwrap_or_default(),
        ),
    ] {
        if let Some(text) = value.as_str() {
            event.tags.insert(key.into(), text.into());
        }
    }
    for (key, value) in &report.data {
        if let (Ok(Value::String(name)), Ok(value)) =
            (serde_json::to_value(key), serde_json::to_value(value))
        {
            event.extra.insert(name, value);
        }
    }
    if repeated > 0 {
        event.extra.insert("repeated".into(), Value::from(repeated));
    }
    event
}

/// The allow-list: every event is rebuilt from what may leave and the rest
/// is dropped. An event without a `code` tag was not made here and is not
/// sent at all.
pub fn scrub(event: Event<'static>) -> Option<Event<'static>> {
    let code = event.tags.get("code")?.clone();
    let mut clean = Event {
        event_id: event.event_id,
        timestamp: event.timestamp,
        level: event.level,
        platform: event.platform,
        release: event.release,
        message: Some(code),
        ..Event::default()
    };
    clean.tags = TAGS
        .iter()
        .filter_map(|key| Some(((*key).to_owned(), event.tags.get(*key)?.clone())))
        .collect();
    clean.extra = event
        .extra
        .into_iter()
        .filter(|(key, value)| match value {
            Value::Number(_) | Value::Bool(_) => true,
            Value::String(token) => closed_key(key) && Token::new(token).is_some(),
            Value::Null | Value::Array(_) | Value::Object(_) => false,
        })
        .collect();
    if let Some(Context::Os(os)) = event.contexts.get("os") {
        clean.contexts.insert(
            "os".into(),
            Context::Os(Box::new(OsContext {
                name: os.name.clone(),
                version: os.version.clone(),
                ..OsContext::default()
            })),
        );
    }
    clean.exception = event
        .exception
        .values
        .into_iter()
        .map(|exception| Exception {
            // The type, «panic», is a code word like the function names.
            ty: exception.ty,
            stacktrace: exception.stacktrace.map(|stacktrace| Stacktrace {
                frames: stacktrace
                    .frames
                    .into_iter()
                    .map(|frame| Frame {
                        function: frame.function,
                        filename: frame.filename,
                        lineno: frame.lineno,
                        // What Sentry names the frame by, with the image below.
                        instruction_addr: frame.instruction_addr,
                        ..Frame::default()
                    })
                    .collect(),
                ..Stacktrace::default()
            }),
            ..Exception::default()
        })
        .collect::<Vec<_>>()
        .into();
    // The images by identifier, address and size; the name reduced to the
    // file's own (the path carries the account's name), no debug file path.
    let images: Vec<DebugImage> = event
        .debug_meta
        .images
        .iter()
        .filter_map(|image| match image {
            DebugImage::Symbolic(image) => Some(DebugImage::Symbolic(SymbolicDebugImage {
                name: image.name.rsplit('/').next().unwrap_or_default().to_owned(),
                arch: image.arch.clone(),
                image_addr: image.image_addr,
                image_size: image.image_size,
                image_vmaddr: image.image_vmaddr,
                id: image.id,
                code_id: image.code_id.clone(),
                debug_file: None,
            })),
            DebugImage::Apple(_) | DebugImage::Proguard(_) | DebugImage::Wasm(_) => None,
        })
        .collect();
    if !images.is_empty() {
        clean.debug_meta = std::borrow::Cow::Owned(DebugMeta {
            sdk_info: None,
            images,
        });
    }
    Some(clean)
}

/// Whether a data key names a closed enumeration in the notice catalogue.
fn closed_key(name: &str) -> bool {
    serde_json::from_value::<NoticeKey>(Value::from(name)).is_ok_and(NoticeKey::closed)
}

/// What was sent in the last [`REPEAT_WINDOW`], by code and data, and how
/// many times it repeated since. Bounded by what one window sees: an entry
/// whose window ended is sent (if it repeated) or forgotten.
#[derive(Default)]
struct Repeats {
    seen: HashMap<String, Seen>,
}

struct Seen {
    since: Instant,
    repeated: u64,
    /// The latest repetition, whose op the count goes out with.
    last: Report,
}

impl Repeats {
    /// `Some(n)` when this report is sent now, `n` being the repetitions
    /// counted since the previous send of it; `None` when it is only counted.
    fn admit(&mut self, report: &Report, now: Instant) -> Option<u64> {
        let key = format!(
            "{}{}",
            report.code.as_str(),
            serde_json::to_string(&report.data).unwrap_or_default()
        );
        match self.seen.get_mut(&key) {
            Some(seen) if now.duration_since(seen.since) < REPEAT_WINDOW => {
                seen.repeated += 1;
                seen.last = report.clone();
                None
            }
            Some(seen) => {
                let repeated = std::mem::take(&mut seen.repeated);
                seen.since = now;
                seen.last = report.clone();
                Some(repeated)
            }
            None => {
                self.seen.insert(
                    key,
                    Seen {
                        since: now,
                        repeated: 0,
                        last: report.clone(),
                    },
                );
                Some(0)
            }
        }
    }

    /// The counts whose window ended, to send now; entries that did not
    /// repeat are forgotten.
    fn due(&mut self, now: Instant) -> Vec<(Report, u64)> {
        let mut due = Vec::new();
        self.seen.retain(|_, seen| {
            if now.duration_since(seen.since) < REPEAT_WINDOW {
                return true;
            }
            if seen.repeated == 0 {
                return false;
            }
            due.push((seen.last.clone(), std::mem::take(&mut seen.repeated)));
            seen.since = now;
            true
        });
        due
    }

    fn forget(&mut self) {
        self.seen.clear();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
