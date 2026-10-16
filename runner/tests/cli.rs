//! End-to-end tests for the `current` binary against throwaway repos.
//!
//! Half validation (a passing check passes, a named check runs alone,
//! the results log gains a JSON line) and half adversarial (a failing
//! check fails the run, an unknown check name is a usage error, a
//! missing manifest is an error, a stub trigger refuses loudly).

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch_repo(tag: &str, manifest: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("current-cli-test-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch repo");
    std::fs::write(dir.join(".current.toml"), manifest).expect("manifest written");
    dir
}

fn run_current(repo: &Path, extra: &[&str]) -> std::process::Output {
    let mut args: Vec<String> = vec!["run".to_owned(), repo.display().to_string()];
    args.extend(extra.iter().map(|value| (*value).to_owned()));
    args.push("--results".to_owned());
    args.push(repo.join("results.jsonl").display().to_string());
    Command::new(env!("CARGO_BIN_EXE_current"))
        .args(&args)
        .output()
        .expect("current binary runs")
}

#[test]
fn passing_check_exits_zero_and_logs_json() {
    let repo = scratch_repo(
        "pass",
        "[[checks]]\nname = \"ok\"\nargv = [\"/bin/true\"]\n",
    );
    let output = run_current(&repo, &[]);
    assert!(output.status.success());
    let log = std::fs::read_to_string(repo.join("results.jsonl")).expect("results log");
    let parsed: serde_json::Value =
        serde_json::from_str(log.trim_end()).expect("results line is JSON");
    assert_eq!(parsed["check"], "ok");
    assert_eq!(parsed["passed"], true);
    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn failing_check_exits_one() {
    let repo = scratch_repo(
        "fail",
        "[[checks]]\nname = \"bad\"\nargv = [\"/bin/false\"]\n",
    );
    let output = run_current(&repo, &[]);
    assert_eq!(output.status.code(), Some(1));
    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn named_check_runs_alone() {
    let repo = scratch_repo(
        "named",
        "[[checks]]\nname = \"good\"\nargv = [\"/bin/true\"]\n\
         [[checks]]\nname = \"bad\"\nargv = [\"/bin/false\"]\n",
    );
    let output = run_current(&repo, &["good"]);
    assert!(output.status.success());
    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn unknown_check_name_is_a_usage_error() {
    let repo = scratch_repo(
        "unknown",
        "[[checks]]\nname = \"ok\"\nargv = [\"/bin/true\"]\n",
    );
    let output = run_current(&repo, &["nope"]);
    assert_eq!(output.status.code(), Some(2));
    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn missing_manifest_is_an_error() {
    let dir = std::env::temp_dir().join(format!("current-cli-test-empty-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let output = run_current(&dir, &[]);
    assert_eq!(output.status.code(), Some(2));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn poll_trigger_is_a_loud_stub() {
    let output = Command::new(env!("CARGO_BIN_EXE_current"))
        .args(["poll", "whatever"])
        .output()
        .expect("current binary runs");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("TODO"), "stub must say so: {stderr}");
}
