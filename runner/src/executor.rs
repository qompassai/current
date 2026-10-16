//! Bounded subprocess execution for one check.
//!
//! Contract: argv is executed directly (no shell). The workdir must
//! resolve inside the repo root. Stdout and stderr are drained on
//! separate threads and capped at [`OUTPUT_BYTES_MAX`] retained bytes
//! each; draining continues past the cap so the child never blocks on
//! a full pipe, and the truncation is recorded. A hard timeout kills
//! and reaps the child. The child owns no cleanup beyond its pipes,
//! which close when it exits or is killed.

use crate::manifest::Check;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Maximum retained bytes per output stream.
pub const OUTPUT_BYTES_MAX: usize = 1_048_576;
/// Poll interval while waiting for a child, in milliseconds.
const WAIT_POLL_MS: u64 = 20;

/// The outcome of running one check.
#[derive(Debug)]
pub struct Outcome {
    /// Process exit code, or `None` when killed by a signal/timeout.
    pub exit_code: Option<i32>,
    /// True when the hard timeout fired and the child was killed.
    pub timed_out: bool,
    /// Wall-clock duration of the run, in milliseconds.
    pub elapsed_ms: u64,
    /// Retained stdout bytes (at most [`OUTPUT_BYTES_MAX`]).
    pub stdout: Vec<u8>,
    /// Retained stderr bytes (at most [`OUTPUT_BYTES_MAX`]).
    pub stderr: Vec<u8>,
    /// Total stdout bytes observed, including truncated-away bytes.
    pub stdout_bytes_total: u64,
    /// Total stderr bytes observed, including truncated-away bytes.
    pub stderr_bytes_total: u64,
    /// True when either stream exceeded the retention cap.
    pub truncated: bool,
}

impl Outcome {
    /// True when the check exited with code 0 and did not time out.
    pub fn passed(&self) -> bool {
        !self.timed_out && self.exit_code == Some(0)
    }
}

/// Why a check could not run at all (distinct from a failing check).
#[derive(Debug)]
pub enum RunError {
    /// The workdir does not exist or escapes the repo root.
    Workdir(String),
    /// The process could not be spawned.
    Spawn(std::io::Error),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Workdir(message) => write!(f, "workdir rejected: {message}"),
            Self::Spawn(error) => write!(f, "cannot spawn check command: {error}"),
        }
    }
}

impl std::error::Error for RunError {}

/// Resolve `workdir` against `repo_root`, requiring the result to be
/// an existing directory inside the canonical repo root.
///
/// # Errors
/// Returns [`RunError::Workdir`] for a missing directory or an escape
/// via `..` or a symlink.
pub fn resolve_workdir(repo_root: &Path, workdir: &str) -> Result<PathBuf, RunError> {
    let root = repo_root
        .canonicalize()
        .map_err(|error| RunError::Workdir(error.to_string()))?;
    let candidate = root
        .join(workdir)
        .canonicalize()
        .map_err(|error| RunError::Workdir(error.to_string()))?;
    if !candidate.starts_with(&root) {
        return Err(RunError::Workdir(format!(
            "{workdir} resolves outside the repo root"
        )));
    }
    if !candidate.is_dir() {
        return Err(RunError::Workdir(format!("{workdir} is not a directory")));
    }
    Ok(candidate)
}

struct Drained {
    retained: Vec<u8>,
    total_bytes: u64,
}

fn drain_stream(mut stream: impl Read + Send + 'static) -> std::thread::JoinHandle<Drained> {
    std::thread::spawn(move || {
        let mut retained: Vec<u8> = Vec::new();
        let mut total_bytes: u64 = 0;
        let mut buffer = [0_u8; 8192];
        loop {
            match stream.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    total_bytes = total_bytes.saturating_add(count as u64);
                    let room = OUTPUT_BYTES_MAX.saturating_sub(retained.len());
                    let keep = room.min(count);
                    retained.extend_from_slice(&buffer[..keep]);
                }
            }
        }
        Drained {
            retained,
            total_bytes,
        }
    })
}

/// Run one check inside `repo_root` and return its bounded outcome.
///
/// # Errors
/// Returns [`RunError`] when the workdir is rejected or the process
/// cannot be spawned. A nonzero exit or a timeout is an [`Outcome`],
/// not an error.
pub fn run_check(repo_root: &Path, check: &Check) -> Result<Outcome, RunError> {
    let workdir = resolve_workdir(repo_root, &check.workdir)?;
    assert!(
        !check.argv.is_empty(),
        "validated manifests always carry a non-empty argv"
    );
    let started = Instant::now();
    let mut child = Command::new(&check.argv[0])
        .args(&check.argv[1..])
        .current_dir(&workdir)
        .envs(&check.env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(RunError::Spawn)?;
    let stdout_thread = child.stdout.take().map(drain_stream);
    let stderr_thread = child.stderr.take().map(drain_stream);
    let timeout = Duration::from_secs(check.timeout_secs_effective());
    let (exit_code, timed_out) = wait_with_timeout(&mut child, started, timeout);
    let stdout = collect(stdout_thread);
    let stderr = collect(stderr_thread);
    let truncated = stdout.total_bytes > OUTPUT_BYTES_MAX as u64
        || stderr.total_bytes > OUTPUT_BYTES_MAX as u64;
    Ok(Outcome {
        exit_code,
        timed_out,
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        stdout: stdout.retained,
        stderr: stderr.retained,
        stdout_bytes_total: stdout.total_bytes,
        stderr_bytes_total: stderr.total_bytes,
        truncated,
    })
}

fn collect(thread: Option<std::thread::JoinHandle<Drained>>) -> Drained {
    match thread {
        Some(handle) => handle.join().unwrap_or(Drained {
            retained: Vec::new(),
            total_bytes: 0,
        }),
        None => Drained {
            retained: Vec::new(),
            total_bytes: 0,
        },
    }
}

fn wait_with_timeout(
    child: &mut Child,
    started: Instant,
    timeout: Duration,
) -> (Option<i32>, bool) {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return (status.code(), false),
            Ok(None) => {
                if started.elapsed() >= timeout {
                    // Kill and reap: dropping a Child does neither.
                    let _ = child.kill();
                    let status = child.wait().ok();
                    return (status.and_then(|s| s.code()), true);
                }
                std::thread::sleep(Duration::from_millis(WAIT_POLL_MS));
            }
            Err(_) => return (None, false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OUTPUT_BYTES_MAX, resolve_workdir, run_check};
    use crate::manifest::Check;
    use std::collections::BTreeMap;

    fn check(argv: Vec<&str>, timeout_secs: u64) -> Check {
        Check {
            name: "test".to_owned(),
            argv: argv.into_iter().map(str::to_owned).collect(),
            workdir: ".".to_owned(),
            timeout_secs: Some(timeout_secs),
            env: BTreeMap::new(),
        }
    }

    #[test]
    fn echo_exits_zero_and_captures_output() {
        let outcome = run_check(
            std::env::temp_dir().as_path(),
            &check(vec!["/bin/echo", "hello-current"], 10),
        )
        .expect("echo runs");
        assert!(outcome.passed());
        assert_eq!(outcome.stdout, b"hello-current\n");
    }

    #[test]
    fn nonzero_exit_propagates() {
        let outcome = run_check(
            std::env::temp_dir().as_path(),
            &check(vec!["/bin/false"], 10),
        )
        .expect("false runs");
        assert!(!outcome.passed());
        assert_eq!(outcome.exit_code, Some(1));
    }

    #[test]
    fn timeout_kills_long_running_check() {
        let outcome = run_check(
            std::env::temp_dir().as_path(),
            &check(vec!["/bin/sleep", "30"], 1),
        )
        .expect("sleep runs");
        assert!(outcome.timed_out);
        assert!(!outcome.passed());
        assert!(outcome.elapsed_ms < 15_000);
    }

    #[test]
    fn oversized_output_is_truncated_and_flagged() {
        let script = "dd if=/dev/zero bs=1024 count=3072 2>/dev/null";
        let outcome = run_check(
            std::env::temp_dir().as_path(),
            &check(vec!["/bin/sh", "-c", script], 30),
        )
        .expect("dd runs");
        assert!(outcome.truncated);
        assert_eq!(outcome.stdout.len(), OUTPUT_BYTES_MAX);
        assert_eq!(outcome.stdout_bytes_total, 3_145_728);
    }

    #[test]
    fn missing_workdir_is_an_error_not_a_panic() {
        let mut bad = check(vec!["/bin/true"], 10);
        bad.workdir = "no-such-dir-current-test".to_owned();
        assert!(run_check(std::env::temp_dir().as_path(), &bad).is_err());
    }

    #[test]
    fn workdir_escaping_repo_is_rejected() {
        assert!(resolve_workdir(std::env::temp_dir().as_path(), "..").is_err());
    }
}
