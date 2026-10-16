//! Current: Qompass AI's local check runner.
//!
//! Current runs a repo's declared checks on this machine and records
//! the verdicts in a local JSONL log. It has no GitHub control plane,
//! no workflow YAML, no expression engine, and no telemetry: the only
//! network operation anywhere in the design is `git fetch`, and v1's
//! poll trigger is not wired yet (see `run_stub_trigger`).
//!
//! Usage:
//!   current run <repo> [check] [--results <path>]
//!   current poll <repo>     (stub in v1 skeleton: documented TODO)
//!   current sweep           (stub in v1 skeleton: documented TODO)
//!
//! `<repo>` is a filesystem path, or a bare name resolved under
//! `$CURRENT_REPO_ROOTS` (colon-separated), then `~/.GH/Qompass`,
//! then `~/workspace/repos`. Exit codes: 0 = every selected check
//! passed, 1 = a check failed or timed out, 2 = usage/manifest error.

#![forbid(unsafe_code)]

mod executor;
mod manifest;
mod results;

use manifest::Manifest;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match dispatch(&args) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("current: {message}");
            ExitCode::from(2)
        }
    }
}

fn dispatch(args: &[String]) -> Result<ExitCode, String> {
    let Some(command) = args.first() else {
        print_usage();
        return Ok(ExitCode::from(2));
    };
    match command.as_str() {
        "run" => run_command(&args[1..]),
        "poll" | "sweep" => Ok(run_stub_trigger(command)),
        "--help" | "-h" | "help" => {
            print_usage();
            Ok(ExitCode::SUCCESS)
        }
        "--version" | "version" => {
            println!("current {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
        other => Err(format!("unknown command '{other}' (try: current --help)")),
    }
}

fn print_usage() {
    println!(
        "current — local check runner\n\n\
         USAGE:\n  current run <repo> [check] [--results <path>]\n  \
         current poll <repo>   (stub: TODO, see runner/README.md)\n  \
         current sweep         (stub: TODO, see runner/README.md)\n  \
         current --version"
    );
}

/// The poll and sweep triggers are deliberate, documented stubs in
/// the v1 skeleton: they refuse with a distinct message instead of
/// pretending to work. The core `run` path is fully real.
fn run_stub_trigger(command: &str) -> ExitCode {
    eprintln!(
        "current: '{command}' is a documented TODO in the v1 skeleton; \
         only 'current run' is implemented. See runner/README.md."
    );
    ExitCode::from(2)
}

struct RunOptions {
    repo_arg: String,
    check_filter: Option<String>,
    results_path: PathBuf,
}

fn parse_run_args(args: &[String]) -> Result<RunOptions, String> {
    let mut positional: Vec<&String> = Vec::new();
    let mut results_path: Option<PathBuf> = None;
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--results" {
            let value = args.get(index + 1).ok_or("--results needs a path value")?;
            results_path = Some(PathBuf::from(value));
            index += 2;
        } else {
            positional.push(&args[index]);
            index += 1;
        }
    }
    if positional.is_empty() || positional.len() > 2 {
        return Err("usage: current run <repo> [check] [--results <path>]".to_owned());
    }
    Ok(RunOptions {
        repo_arg: positional[0].clone(),
        check_filter: positional.get(1).map(|name| (*name).clone()),
        results_path: results_path.unwrap_or_else(default_results_path),
    })
}

fn default_results_path() -> PathBuf {
    if let Ok(path) = std::env::var("CURRENT_RESULTS_LOG") {
        return PathBuf::from(path);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".local/state/current")
            .join("results.jsonl");
    }
    PathBuf::from("current-results.jsonl")
}

/// Resolve a repo argument to an existing directory.
///
/// A direct path wins only when it actually carries a manifest:
/// a bare name typed inside a repo must not be captured by a
/// same-named subdirectory of that repo (the Ghost repo contains a
/// `ghost/` module directory, and `current run ghost` from inside
/// it must still find `~/.GH/Qompass/ghost`).
fn resolve_repo(repo_arg: &str) -> Result<PathBuf, String> {
    let direct = PathBuf::from(repo_arg);
    if direct.join(".current.toml").is_file() {
        return Ok(direct);
    }
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(joined) = std::env::var("CURRENT_REPO_ROOTS") {
        roots.extend(
            joined
                .split(':')
                .filter(|path| !path.is_empty())
                .map(PathBuf::from),
        );
    }
    if let Ok(home) = std::env::var("HOME") {
        roots.push(PathBuf::from(&home).join(".GH/Qompass"));
        roots.push(PathBuf::from(&home).join("workspace/repos"));
    }
    for root in roots {
        let candidate = root.join(repo_arg);
        if candidate.is_dir() {
            return Ok(candidate);
        }
    }
    if direct.is_dir() {
        return Ok(direct);
    }
    Err(format!(
        "repo '{repo_arg}' not found as a path or under the search roots"
    ))
}

fn run_command(args: &[String]) -> Result<ExitCode, String> {
    let options = parse_run_args(args)?;
    let repo_root = resolve_repo(&options.repo_arg)?;
    let manifest_path = repo_root.join(".current.toml");
    let loaded = manifest::load_manifest(&manifest_path)
        .map_err(|error| format!("{}: {error}", manifest_path.display()))?;
    let repo_name = repo_root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| options.repo_arg.clone());
    let selected = select_checks(&loaded, options.check_filter.as_deref())?;
    let mut all_passed = true;
    // One job at a time, in manifest order: primo build politeness
    // is a design rule, not a tuning knob.
    for check in selected {
        let outcome = executor::run_check(&repo_root, check)
            .map_err(|error| format!("check '{}': {error}", check.name))?;
        report_check(check, &outcome);
        let record = results::ResultRecord::from_outcome(now_unix(), &repo_name, check, &outcome);
        if let Err(error) = results::append_record(&options.results_path, &record) {
            eprintln!("current: warning: cannot write results log: {error}");
        }
        all_passed &= outcome.passed();
    }
    Ok(if all_passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

fn select_checks<'a>(
    manifest: &'a Manifest,
    filter: Option<&str>,
) -> Result<Vec<&'a manifest::Check>, String> {
    match filter {
        None => Ok(manifest.checks.iter().collect()),
        Some(name) => {
            let found: Vec<&manifest::Check> =
                manifest.checks.iter().filter(|c| c.name == name).collect();
            if found.is_empty() {
                return Err(format!("no check named '{name}' in the manifest"));
            }
            Ok(found)
        }
    }
}

fn report_check(check: &manifest::Check, outcome: &executor::Outcome) {
    let verdict = if outcome.timed_out {
        "TIMEOUT"
    } else if outcome.passed() {
        "PASS"
    } else {
        "FAIL"
    };
    let code = outcome
        .exit_code
        .map(|c| c.to_string())
        .unwrap_or_else(|| "signal".to_owned());
    println!(
        "[{verdict}] {} (exit {code}, {} ms{}{})",
        check.name,
        outcome.elapsed_ms,
        if outcome.truncated {
            ", output truncated"
        } else {
            ""
        },
        if outcome.timed_out {
            ", killed at timeout"
        } else {
            ""
        },
    );
    let stderr_tail = results::tail_of(&outcome.stderr);
    if !outcome.passed() && !stderr_tail.is_empty() {
        eprintln!("  stderr tail: {stderr_tail}");
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
