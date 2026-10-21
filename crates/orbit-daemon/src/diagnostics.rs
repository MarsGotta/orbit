//! Durable diagnostic trail for the daemon.
//!
//! The macOS host writes `native.log`; this is its counterpart for the process
//! that actually talks to the hub. Without it, a refused connection or a dropped
//! socket leaves nothing on disk and the only symptom is a UI that says nothing.
//!
//! Two rules are structural, not stylistic:
//!   - No conversation content and no secrets. Callers pass an event name and
//!     short labelled fields, and every value is truncated, so a caller that
//!     hands over a transcript still cannot write one to disk.
//!   - Logging never fails the product: a line that cannot be written is a lost
//!     diagnostic, not a reason to drop a recording.

use serde_json::{Map, Value, json};
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

pub const MAXIMUM_FIELD_BYTES: usize = 200;
pub const MAXIMUM_FIELDS: usize = 12;
const MAXIMUM_BYTES: u64 = 1_048_576;

pub struct DiagnosticLog {
    file: orbit_ipc::report::LogFile,
}

impl DiagnosticLog {
    /// Lives inside the daemon's own data directory, never in a fixed place
    /// under the user's home. A run started with `--data-dir` therefore keeps
    /// its log inside that fixture: the previous version wrote to a single path
    /// for every process, so parallel test runs raced over one file and a test
    /// polluted the real profile.
    pub fn in_root(root: &Path) -> Self {
        Self {
            file: orbit_ipc::report::LogFile::new(root.join("logs/daemon.log"), MAXIMUM_BYTES),
        }
    }

    /// Testable location boundary. Production always resolves the user's own
    /// log directory; a caller-chosen path exists so tests never touch it.
    #[cfg(test)]
    pub fn at(path: PathBuf, maximum_bytes: u64) -> Self {
        Self {
            file: orbit_ipc::report::LogFile::new(path, maximum_bytes),
        }
    }

    pub fn path(&self) -> &Path {
        self.file.path()
    }

    /// Records one event. Never panics and never propagates an error.
    pub fn record(&self, event: &str, fields: &[(&str, &str)]) {
        let line = Self::encode(event, fields);
        // The last diagnostic before a crash must survive the process.
        let _ = self
            .file
            .append(line.as_bytes())
            .and_then(|file| file.sync_data());
    }

    fn encode(event: &str, fields: &[(&str, &str)]) -> String {
        let mut payload = Map::new();
        payload.insert("time".into(), json!(orbit_ipc::report::timestamp()));
        payload.insert("pid".into(), json!(std::process::id()));
        payload.insert("event".into(), json!(clamp(event)));
        if !fields.is_empty() {
            let mut entries: Vec<(&str, &str)> = fields.to_vec();
            // Sorted so a truncated set is the same on every run rather than
            // whichever fields the caller happened to list first.
            entries.sort_by(|left, right| left.0.cmp(right.0));
            let mut clamped = Map::new();
            for (key, value) in entries.into_iter().take(MAXIMUM_FIELDS) {
                clamped.insert(clamp(key), json!(clamp(value)));
            }
            payload.insert("fields".into(), Value::Object(clamped));
        }
        format!("{}\n", Value::Object(payload))
    }
}

/// Truncates on a UTF-8 boundary: cutting mid-character would emit bytes no
/// reader can decode, losing the whole line instead of a tail.
fn clamp(value: &str) -> String {
    if value.len() <= MAXIMUM_FIELD_BYTES {
        return value.to_owned();
    }
    let mut end = MAXIMUM_FIELD_BYTES;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &value[..end])
}

#[cfg(test)]
mod tests {
    //! Fixture setup only; production code above never panics.
    #![allow(clippy::expect_used)]
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("orbit-diagnostics-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        directory.join("daemon.log")
    }

    fn lines(path: &Path) -> Vec<Value> {
        let text = std::fs::read_to_string(path).expect("read log");
        text.lines()
            .map(|line| serde_json::from_str(line).expect("decodable line"))
            .collect()
    }

    #[test]
    fn each_record_is_one_decodable_line_in_a_private_file() {
        let path = scratch("basic");
        let log = DiagnosticLog::at(path.clone(), MAXIMUM_BYTES);
        log.record("hub_connected", &[("attempt", "1")]);
        log.record("hub_disconnected", &[("reason", "closed")]);
        let decoded = lines(&path);
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0]["event"], "hub_connected");
        assert_eq!(decoded[0]["fields"]["attempt"], "1");
        assert_eq!(decoded[1]["event"], "hub_disconnected");
        assert!(
            decoded[0]["time"]
                .as_str()
                .unwrap_or_default()
                .ends_with('Z')
        );
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(path.parent().unwrap_or(Path::new("/")));
    }

    #[test]
    fn a_conversation_sized_field_cannot_reach_disk_whole() {
        let path = scratch("clamp");
        let log = DiagnosticLog::at(path.clone(), MAXIMUM_BYTES);
        let secret = "contenido ".repeat(500);
        log.record("never_log_content", &[("leak", &secret)]);
        let stored = lines(&path)[0]["fields"]["leak"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert!(stored.len() <= MAXIMUM_FIELD_BYTES + 4);
        assert!(stored.ends_with('…'));
        assert!(stored.len() < secret.len());
        let _ = std::fs::remove_dir_all(path.parent().unwrap_or(Path::new("/")));
    }

    #[test]
    fn truncation_keeps_the_line_decodable_on_a_multibyte_boundary() {
        let path = scratch("multibyte");
        let log = DiagnosticLog::at(path.clone(), MAXIMUM_BYTES);
        // Three bytes per character, so the byte budget lands mid-character
        // unless the cut is moved back to a boundary.
        log.record("multibyte", &[("value", &"€".repeat(400))]);
        let decoded = lines(&path);
        assert_eq!(decoded.len(), 1, "a mid-character cut would break the line");
        let _ = std::fs::remove_dir_all(path.parent().unwrap_or(Path::new("/")));
    }

    #[test]
    fn field_count_is_bounded_and_selection_is_stable() {
        let path = scratch("fields");
        let log = DiagnosticLog::at(path.clone(), MAXIMUM_BYTES);
        let keys: Vec<String> = (0..40).map(|index| format!("k{index:02}")).collect();
        let fields: Vec<(&str, &str)> = keys.iter().map(|key| (key.as_str(), "v")).collect();
        log.record("many", &fields);
        let stored = lines(&path)[0]["fields"].clone();
        let object = stored.as_object().cloned().unwrap_or_default();
        assert_eq!(object.len(), MAXIMUM_FIELDS);
        assert!(object.contains_key("k00"));
        assert!(!object.contains_key("k39"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap_or(Path::new("/")));
    }

    #[test]
    fn the_log_stays_inside_the_root_it_was_given() {
        let root = std::env::temp_dir().join(format!("orbit-root-{}", std::process::id()));
        let log = DiagnosticLog::in_root(&root);
        // A fixed path under the user's home would make every parallel run share
        // one file and let a test write into the real profile.
        assert!(
            log.path().starts_with(&root),
            "log escaped its root: {}",
            log.path().display()
        );
        log.record("contained", &[]);
        assert!(log.path().exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod compatibility_tests {
    use super::*;
    use orbit_core::notice::{Notice, NoticeCode, NoticeKey, Op, Report, ReportSource};

    #[test]
    fn both_logs_keep_the_0_7_line_format() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let diagnostic = DiagnosticLog::in_root(root.path());
        let report = orbit_ipc::report::ReportLog::daemon(root.path());
        diagnostic.record("fixture_log", &[("attempt", "1")]);
        report.report(&Report::of_notice(
            &Notice::new(NoticeCode::CaptureCut)
                .with(NoticeKey::Fragments, 3u64)
                .with_token(NoticeKey::Cause, "helper_lost"),
            Op::parse("0123abcd-ef").ok_or_else(|| anyhow::anyhow!("invalid fixture op"))?,
            ReportSource::Daemon,
        ));
        let saved: Value = serde_json::from_str(include_str!("../tests/fixtures/logs-0.7.json"))?;
        for (path, kind, key) in [
            (diagnostic.path(), "diagnostic", "time"),
            (report.path(), "report", "ts"),
        ] {
            let text = std::fs::read_to_string(path)?;
            assert_eq!(text.lines().count(), 1);
            assert!(text.ends_with('\n'));
            let mut actual: Value = serde_json::from_str(&text)?;
            let timestamp = actual[key]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("timestamp missing"))?;
            let parsed = chrono::DateTime::parse_from_rfc3339(timestamp)?;
            assert_eq!(timestamp.len(), 24);
            assert_eq!(
                timestamp,
                parsed.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
            );
            actual[key] = saved[kind][key].clone();
            if kind == "diagnostic" {
                actual["pid"] = json!(0);
            }
            assert_eq!(actual, saved[kind]);
            eprintln!("0.7 {kind}: {actual}");
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(path)?.permissions().mode() & 0o777, 0o600);
        }
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(root.path().join("logs"))?
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        Ok(())
    }
}

#[cfg(test)]
mod symlink_tests {
    use super::*;

    #[test]
    fn neither_log_follows_a_planted_symlink() -> anyhow::Result<()> {
        use orbit_core::notice::{Notice, NoticeCode, Op, Report, ReportSource};
        let root = tempfile::tempdir()?;
        std::fs::create_dir(root.path().join("logs"))?;
        let target = root.path().join("untouched");
        std::fs::write(&target, "fixture sentinel")?;
        let diagnostic = DiagnosticLog::in_root(root.path());
        let report = orbit_ipc::report::ReportLog::daemon(root.path());
        for path in [diagnostic.path(), report.path()] {
            std::os::unix::fs::symlink(&target, path)?;
        }
        diagnostic.record("fixture", &[]);
        report.report(&Report::of_notice(
            &Notice::new(NoticeCode::CaptureCut),
            Op::parse("0123abcd-ef").ok_or_else(|| anyhow::anyhow!("invalid fixture op"))?,
            ReportSource::Daemon,
        ));
        assert_eq!(std::fs::read_to_string(&target)?, "fixture sentinel");
        for path in [diagnostic.path(), report.path()] {
            assert!(std::fs::symlink_metadata(path)?.file_type().is_symlink());
        }
        Ok(())
    }
}
