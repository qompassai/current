//! The local JSONL results log.
//!
//! Contract: one JSON object per executed check, appended atomically
//! enough for a single-writer local log (one `write_all` of one line
//! under an append-mode handle). Records carry the verdict, timing,
//! byte totals, and a bounded tail of each stream. Nothing in a
//! record leaves this machine: Current has no telemetry path.

use crate::executor::Outcome;
use serde::Serialize;
use std::io::Write;
use std::path::Path;

/// Maximum characters kept from the tail of each captured stream.
pub const TAIL_CHARS_MAX: usize = 1_000;

/// One executed check, as written to the results log.
#[derive(Debug, Serialize)]
pub struct ResultRecord {
    /// Unix epoch seconds when the check finished.
    pub ts_unix: u64,
    /// Repo directory name the manifest was loaded from.
    pub repo: String,
    /// Check name from the manifest.
    pub check: String,
    /// The argv that ran, as declared in the manifest.
    pub argv: Vec<String>,
    /// Process exit code, or null when killed by a signal/timeout.
    pub exit_code: Option<i32>,
    /// True when the check passed (exit 0, no timeout).
    pub passed: bool,
    /// True when the hard timeout fired.
    pub timed_out: bool,
    /// Wall-clock duration, in milliseconds.
    pub elapsed_ms: u64,
    /// Total stdout bytes observed (before truncation).
    pub stdout_bytes_total: u64,
    /// Total stderr bytes observed (before truncation).
    pub stderr_bytes_total: u64,
    /// True when either stream exceeded the retention cap.
    pub truncated: bool,
    /// Last [`TAIL_CHARS_MAX`] characters of stdout, lossy UTF-8.
    pub stdout_tail: String,
    /// Last [`TAIL_CHARS_MAX`] characters of stderr, lossy UTF-8.
    pub stderr_tail: String,
}

/// Keep only the last [`TAIL_CHARS_MAX`] characters of `bytes`.
pub fn tail_of(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    if text.chars().count() <= TAIL_CHARS_MAX {
        return text.into_owned();
    }
    text.chars()
        .skip(text.chars().count() - TAIL_CHARS_MAX)
        .collect()
}

impl ResultRecord {
    /// Build a record from a finished check run.
    pub fn from_outcome(
        ts_unix: u64,
        repo: &str,
        check: &crate::manifest::Check,
        outcome: &Outcome,
    ) -> Self {
        Self {
            ts_unix,
            repo: repo.to_owned(),
            check: check.name.clone(),
            argv: check.argv.clone(),
            exit_code: outcome.exit_code,
            passed: outcome.passed(),
            timed_out: outcome.timed_out,
            elapsed_ms: outcome.elapsed_ms,
            stdout_bytes_total: outcome.stdout_bytes_total,
            stderr_bytes_total: outcome.stderr_bytes_total,
            truncated: outcome.truncated,
            stdout_tail: tail_of(&outcome.stdout),
            stderr_tail: tail_of(&outcome.stderr),
        }
    }
}

/// Append one record as one JSON line to the log at `path`,
/// creating parent directories and the file as needed.
///
/// # Errors
/// Returns the I/O or serialization error; the caller decides
/// whether a log failure fails the run (in `main` it does not:
/// the verdict is printed first, and the log error is reported
/// on stderr without changing the check's exit status).
pub fn append_record(path: &Path, record: &ResultRecord) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let line = serde_json::to_string(record)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")
}

#[cfg(test)]
mod tests {
    use super::{ResultRecord, append_record, tail_of};
    use crate::executor::Outcome;
    use crate::manifest::Check;
    use std::collections::BTreeMap;

    #[test]
    fn tail_is_bounded() {
        let bytes = vec![b'a'; 5_000];
        assert_eq!(tail_of(&bytes).chars().count(), 1_000);
    }

    #[test]
    fn appends_one_valid_json_line() {
        let dir = std::env::temp_dir().join(format!("current-results-test-{}", std::process::id()));
        let path = dir.join("results.jsonl");
        let check = Check {
            name: "demo".to_owned(),
            argv: vec!["/bin/true".to_owned()],
            workdir: ".".to_owned(),
            timeout_secs: Some(5),
            env: BTreeMap::new(),
        };
        let outcome = Outcome {
            exit_code: Some(0),
            timed_out: false,
            elapsed_ms: 3,
            stdout: b"ok\n".to_vec(),
            stderr: Vec::new(),
            stdout_bytes_total: 3,
            stderr_bytes_total: 0,
            truncated: false,
        };
        let record = ResultRecord::from_outcome(1_700_000_000, "demo-repo", &check, &outcome);
        append_record(&path, &record).expect("log appends");
        let text = std::fs::read_to_string(&path).expect("log readable");
        let parsed: serde_json::Value =
            serde_json::from_str(text.trim_end()).expect("line is valid JSON");
        assert_eq!(parsed["check"], "demo");
        assert_eq!(parsed["passed"], true);
        std::fs::remove_dir_all(&dir).ok();
    }
}
