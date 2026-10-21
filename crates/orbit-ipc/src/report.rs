//! The local report log: one JSON line per notice or failure, with
//! no content, read over SSH.
//!
//! Each process writes its own file through one function, [`ReportLog::report`]:
//! the daemon its notices and failures (`report-daemon.log`), the app what the
//! interface sends it through the `report` order (`report-app.log`). Both live
//! in `<data directory>/logs/`, next to `daemon.log`, so a run with
//! `--data-dir` keeps its log inside that fixture.
//!
//! A line is a typed [`Report`] plus its time: the type admits only an op,
//! a code token, a class, a source and closed data, so no caller can write a
//! sentence, a transcript or an event title here.
use orbit_core::notice::Report;
use serde_json::json;
use std::io::Write;
use std::path::{Path, PathBuf};

/// What one generation may grow to before it rotates. It protects the Mac's
/// disk from a failure that repeats every second (86 400 lines, some 17 MB a
/// day unbounded). A line measures about 200 bytes and a normal day writes a
/// few hundred, so 1 MiB keeps several days; one previous generation is kept,
/// so the log never takes more than 2 MiB. It is not a limit on how many
/// notices are said: the oldest lines leave first.
pub const MAX_BYTES: u64 = 1_048_576;

pub struct ReportLog {
    file: LogFile,
}

impl ReportLog {
    /// The daemon's file.
    pub fn daemon(root: &Path) -> Self {
        Self::at(root.join("logs/report-daemon.log"), MAX_BYTES)
    }

    /// The app's file, for what the interface reports.
    pub fn app(root: &Path) -> Self {
        Self::at(root.join("logs/report-app.log"), MAX_BYTES)
    }

    /// A caller-chosen path and bound, so tests never touch a real log.
    pub fn at(path: PathBuf, max_bytes: u64) -> Self {
        Self {
            file: LogFile::new(path, max_bytes),
        }
    }

    pub fn path(&self) -> &Path {
        self.file.path()
    }

    /// Writes one line. Never panics and never fails the caller: a line that
    /// cannot be written is a lost diagnostic, not a reason to stop.
    pub fn report(&self, report: &Report) {
        let mut line = json!({
            "ts": timestamp(),
            "op": report.op,
            "code": report.code,
            "class": report.class,
            "source": report.source,
            "data": report.data,
        })
        .to_string();
        line.push('\n');
        let _ = self.file.append(line.as_bytes());
    }
}

/// The two local logs share private creation and one previous generation.
/// Returning the written file lets daemon.log sync its receipt to disk while
/// report logs retain their existing buffered write behavior.
pub struct LogFile {
    path: PathBuf,
    max_bytes: u64,
}

impl LogFile {
    pub fn new(path: PathBuf, max_bytes: u64) -> Self {
        Self { path, max_bytes }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, line: &[u8]) -> std::io::Result<std::fs::File> {
        use std::os::unix::fs::DirBuilderExt;
        if let Some(parent) = self.path.parent()
            && !parent.exists()
        {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)?;
        }
        self.rotate_if_needed(line.len() as u64);
        // NOFOLLOW: a symlink planted at this path must not redirect writes.
        let descriptor = rustix::fs::open(
            &self.path,
            rustix::fs::OFlags::WRONLY
                | rustix::fs::OFlags::APPEND
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::NOFOLLOW,
            rustix::fs::Mode::from_bits_truncate(0o600),
        )?;
        let mut file = std::fs::File::from(descriptor);
        file.write_all(line)?;
        Ok(file)
    }

    /// Keeps one previous generation (`<file>.1`).
    fn rotate_if_needed(&self, incoming: u64) {
        let Ok(metadata) = std::fs::metadata(&self.path) else {
            return;
        };
        if metadata.len() + incoming <= self.max_bytes {
            return;
        }
        let mut previous = self.path.clone().into_os_string();
        previous.push(".1");
        let _ = std::fs::rename(&self.path, PathBuf::from(previous));
    }
}

/// UTC time with milliseconds, `2026-10-16T09:30:00.123Z`.
pub fn timestamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use orbit_core::notice::{Notice, NoticeCode, NoticeKey, Op, ReportSource};

    fn report(index: u64) -> Report {
        Report::of_notice(
            &Notice::new(NoticeCode::CaptureCut)
                .with(NoticeKey::Fragments, index)
                .with_token(NoticeKey::Cause, "helper_lost"),
            Op::parse("0123abcd-ef").unwrap(),
            ReportSource::Daemon,
        )
    }

    #[test]
    fn each_report_is_one_line_with_the_six_fields_and_no_more() {
        let directory = tempfile::tempdir().unwrap();
        let log = ReportLog::daemon(directory.path());
        log.report(&report(3));
        let text = std::fs::read_to_string(log.path()).unwrap();
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 1);
        let line = lines[0].as_object().unwrap();
        let mut keys: Vec<&str> = line.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["class", "code", "data", "op", "source", "ts"]);
        assert_eq!(line["code"], "capture_cut");
        assert_eq!(line["class"], "failed");
        assert_eq!(line["source"], "daemon");
        assert_eq!(line["op"], "0123abcd-ef");
        assert_eq!(line["data"]["fragments"], 3);
        assert_eq!(line["data"]["cause"], "helper_lost");
        assert!(line["ts"].as_str().unwrap().ends_with('Z'));
    }

    #[test]
    fn the_log_keeps_its_bound_and_rotates_one_generation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logs/report-daemon.log");
        let max_bytes = 1_024;
        let log = ReportLog::at(path.clone(), max_bytes);
        for index in 0..200 {
            log.report(&report(index));
            let size = std::fs::metadata(&path).unwrap().len();
            assert!(size <= max_bytes, "line {index}: {size} > {max_bytes}");
        }
        let previous = directory.path().join("logs/report-daemon.log.1");
        assert!(previous.exists(), "the full generation is kept once");
        assert!(std::fs::metadata(&previous).unwrap().len() <= max_bytes);
        let mut names: Vec<String> = std::fs::read_dir(directory.path().join("logs"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["report-daemon.log", "report-daemon.log.1"]);
        // The newest line is in the current file.
        let current = std::fs::read_to_string(&path).unwrap();
        assert!(
            current
                .lines()
                .last()
                .unwrap()
                .contains("\"fragments\":199")
        );
    }
}
